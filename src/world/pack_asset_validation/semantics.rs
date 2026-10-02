//! Metadata contracts for the glTF readers used by Bevy. The caller validates
//! raw indices, graph structure and decoded size before this pass. Byte ranges
//! are checked separately before readers run. No buffer is opened here,
//! including for sparse accessors.
use gltf::{
    accessor::{DataType, Dimensions},
    animation::{Interpolation, Property},
    Accessor, Document, Semantic,
};
use std::collections::BTreeSet;

// Bevy 0.18.1 bevy_pbr::MAX_JOINTS and bevy_mesh::morph::MAX_MORPH_WEIGHTS.
// The parent integration tests pin these compatibility bounds to the renderer;
// this metadata-only module can be exercised without linking a renderer.
pub(super) const MAX_JOINTS: usize = 256;
pub(super) const MAX_MORPH_WEIGHTS: usize = 256;

pub(super) fn validate(model: &Document) -> Result<(), String> {
    validate_meshes(model)?;
    validate_skins(model)?;
    validate_animations(model)
}

fn plain(accessor: &Accessor<'_>, kind: DataType, shape: Dimensions) -> bool {
    accessor.data_type() == kind
        && accessor.dimensions() == shape
        && !accessor.normalized()
        && accessor.count() > 0
}

fn unsigned_or_float(accessor: &Accessor<'_>) -> bool {
    match accessor.data_type() {
        DataType::F32 => !accessor.normalized(),
        DataType::U8 | DataType::U16 => accessor.normalized(),
        _ => false,
    }
}

fn animation_value(accessor: &Accessor<'_>) -> bool {
    match accessor.data_type() {
        DataType::F32 => !accessor.normalized(),
        DataType::I8 | DataType::U8 | DataType::I16 | DataType::U16 => accessor.normalized(),
        _ => false,
    }
}

fn vertex_format(semantic: &Semantic, accessor: &Accessor<'_>) -> bool {
    use DataType::{F32, U16, U8};
    use Dimensions::{Vec2, Vec3, Vec4};
    match semantic {
        Semantic::Positions | Semantic::Normals => plain(accessor, F32, Vec3),
        Semantic::Tangents => plain(accessor, F32, Vec4),
        Semantic::Colors(_) => {
            matches!(accessor.dimensions(), Vec3 | Vec4) && unsigned_or_float(accessor)
        }
        Semantic::TexCoords(_) => accessor.dimensions() == Vec2 && unsigned_or_float(accessor),
        Semantic::Joints(_) => {
            accessor.dimensions() == Vec4
                && matches!(accessor.data_type(), U8 | U16)
                && !accessor.normalized()
        }
        Semantic::Weights(_) => accessor.dimensions() == Vec4 && unsigned_or_float(accessor),
        // Custom attributes are ignored unless the renderer explicitly declares
        // them. This does not turn an unknown attribute into a standard one.
        Semantic::Extras(_) => true,
    }
}

fn mesh_target_count(mesh: &gltf::Mesh<'_>) -> usize {
    mesh.primitives()
        .next()
        .map_or(0, |primitive| primitive.morph_targets().len())
}

fn validate_weights(weights: &[f32], count: usize) -> Result<(), String> {
    if count == 0 || weights.len() != count || weights.iter().any(|value| !value.is_finite()) {
        return Err("GLB morph weights must match the mesh's target count and be finite".into());
    }
    Ok(())
}

fn validate_meshes(model: &Document) -> Result<(), String> {
    for mesh in model.meshes() {
        let targets = mesh_target_count(&mesh);
        if targets > MAX_MORPH_WEIGHTS {
            return Err("GLB mesh exceeds the renderer's morph target limit".into());
        }
        if let Some(weights) = mesh.weights() {
            validate_weights(weights, targets)?;
        }
        for primitive in mesh.primitives() {
            let count = primitive
                .get(&Semantic::Positions)
                .ok_or("GLB primitive has no positions")?
                .count();
            for (semantic, accessor) in primitive.attributes() {
                if accessor.count() != count
                    || accessor.count() == 0
                    || !vertex_format(&semantic, &accessor)
                {
                    return Err(format!(
                        "GLB vertex attribute {semantic:?} has an unsupported type, shape or count"
                    ));
                }
            }
            if let Some(indices) = primitive.indices() {
                if !matches!(
                    indices.data_type(),
                    DataType::U8 | DataType::U16 | DataType::U32
                ) || indices.dimensions() != Dimensions::Scalar
                    || indices.normalized()
                    || indices.count() == 0
                {
                    return Err("GLB primitive indices must be unsigned scalar integers".into());
                }
            }
            if primitive.morph_targets().len() != targets {
                return Err("GLB mesh primitives must have the same morph target count".into());
            }
            for target in primitive.morph_targets() {
                let attributes = [target.positions(), target.normals(), target.tangents()];
                if attributes.iter().all(Option::is_none) {
                    return Err("GLB morph target has no vertex attributes".into());
                }
                for accessor in attributes.into_iter().flatten() {
                    if !plain(&accessor, DataType::F32, Dimensions::Vec3)
                        || accessor.count() != count
                    {
                        return Err(
                            "GLB morph attributes must be float VEC3 with the primitive's vertex count"
                                .into(),
                        );
                    }
                }
            }
        }
    }
    for node in model.nodes() {
        if let Some(weights) = node.weights() {
            let mesh = node.mesh().ok_or("GLB node morph weights require a mesh")?;
            validate_weights(weights, mesh_target_count(&mesh))?;
        }
    }
    Ok(())
}

fn validate_skins(model: &Document) -> Result<(), String> {
    for skin in model.skins() {
        let joints: Vec<_> = skin.joints().map(|node| node.index()).collect();
        if joints.is_empty()
            || joints.len() > MAX_JOINTS
            || joints.iter().copied().collect::<BTreeSet<_>>().len() != joints.len()
        {
            return Err(
                "GLB skin must have distinct joints within the renderer's joint limit".into(),
            );
        }
        if let Some(matrices) = skin.inverse_bind_matrices() {
            if !plain(&matrices, DataType::F32, Dimensions::Mat4) || matrices.count() < joints.len()
            {
                return Err(
                    "GLB inverse bind matrices must be float MAT4 with one entry per joint".into(),
                );
            }
        }
    }
    let nodes: Vec<_> = model.nodes().collect();
    for scene in model.scenes() {
        let mut reached = vec![false; nodes.len()];
        let mut pending: Vec<_> = scene.nodes().map(|node| node.index()).collect();
        while let Some(index) = pending.pop() {
            if !std::mem::replace(&mut reached[index], true) {
                pending.extend(nodes[index].children().map(|node| node.index()));
            }
        }
        for node in nodes.iter().filter(|node| reached[node.index()]) {
            if let Some(skin) = node.skin() {
                if node.mesh().is_none() {
                    return Err("GLB skinned node requires a mesh".into());
                }
                // Bevy resolves these through this scene's entity map, not the
                // global node table. A valid global index alone is insufficient.
                if skin.joints().any(|joint| !reached[joint.index()]) {
                    return Err(
                        "GLB skin joints must be reachable in every scene using that skin".into(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_animations(model: &Document) -> Result<(), String> {
    for animation in model.animations() {
        let mut targets = BTreeSet::new();
        for channel in animation.channels() {
            let target = channel.target();
            let property = target.property();
            if !targets.insert((target.node().index(), property as u8)) {
                return Err(
                    "GLB animation channels must have distinct node/property targets".into(),
                );
            }
            let sampler = channel.sampler();
            let input = sampler.input();
            if !plain(&input, DataType::F32, Dimensions::Scalar) || input.sparse().is_some() {
                return Err(
                    "GLB animation input must be a non-sparse float SCALAR accessor".into(),
                );
            }
            let output = sampler.output();
            let (shape, width, compatible) = match property {
                Property::Translation | Property::Scale => (
                    Dimensions::Vec3,
                    1,
                    plain(&output, DataType::F32, Dimensions::Vec3),
                ),
                Property::Rotation => (Dimensions::Vec4, 1, animation_value(&output)),
                Property::MorphTargetWeights => {
                    let mesh = target
                        .node()
                        .mesh()
                        .ok_or("GLB weight animation requires a node with morph targets")?;
                    let width = mesh_target_count(&mesh);
                    if width == 0 {
                        return Err(
                            "GLB weight animation requires a node with morph targets".into()
                        );
                    }
                    (Dimensions::Scalar, width, animation_value(&output))
                }
            };
            let multiplier = match sampler.interpolation() {
                Interpolation::CubicSpline if input.count() < 2 => {
                    return Err("GLB cubic animation requires at least two input samples".into());
                }
                Interpolation::CubicSpline => 3,
                _ => 1,
            };
            let expected = input
                .count()
                .checked_mul(width)
                .and_then(|n| n.checked_mul(multiplier));
            if !compatible || output.dimensions() != shape || expected != Some(output.count()) {
                return Err(
                    "GLB animation output type, shape or count does not match its channel".into(),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "semantics_tests.rs"]
mod tests;
