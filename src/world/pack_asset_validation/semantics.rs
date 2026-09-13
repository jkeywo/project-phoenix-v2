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
mod tests {
    use super::*;

    // These fixtures exercise metadata only. The parent admission pass owns
    // actual bytes, raw index validation, graph shape and resource limits.
    fn accessor(kind: &str, component: u32, count: usize, normalized: bool) -> String {
        format!(
            r#"{{"bufferView":0,"componentType":{component},"count":{count},"type":"{kind}","normalized":{normalized},"min":[0,0,0],"max":[1,1,1]}}"#
        )
    }

    fn model(accessors: &[String], extra: &str) -> Document {
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":4096}}],"bufferViews":[{{"buffer":0,"byteLength":4096}}],"accessors":[{}]{extra}}}"#,
            accessors.join(",")
        );
        gltf::Gltf::from_slice(json.as_bytes()).unwrap().document
    }

    #[test]
    fn primitive_indices_are_checked_before_the_reader_can_panic() {
        for (kind, component, normalized, accepted) in [
            ("SCALAR", 5121, false, true),
            ("SCALAR", 5123, false, true),
            ("SCALAR", 5125, false, true),
            ("SCALAR", 5126, false, false),
            ("VEC2", 5123, false, false),
            ("SCALAR", 5123, true, false),
        ] {
            let doc = model(
                &[
                    accessor("VEC3", 5126, 3, false),
                    accessor(kind, component, 3, normalized),
                ],
                r#", "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}]"#,
            );
            assert_eq!(
                validate(&doc).is_ok(),
                accepted,
                "{kind} {component} {normalized}"
            );
        }
    }

    #[test]
    fn standard_vertex_formats_preserve_normalized_integer_attributes() {
        for (semantic, kind, component, normalized, accepted) in [
            ("POSITION", "VEC3", 5126, false, true),
            ("POSITION", "VEC2", 5126, false, false),
            ("POSITION", "VEC3", 5125, false, false),
            ("NORMAL", "VEC3", 5126, false, true),
            ("TANGENT", "VEC4", 5126, false, true),
            ("COLOR_0", "VEC3", 5121, true, true),
            ("COLOR_0", "VEC4", 5123, true, true),
            ("TEXCOORD_0", "VEC2", 5123, true, true),
            ("JOINTS_0", "VEC4", 5121, false, true),
            ("WEIGHTS_0", "VEC4", 5123, true, true),
            ("WEIGHTS_0", "VEC4", 5123, false, false),
        ] {
            let attributes = if semantic == "POSITION" {
                r#""POSITION":1"#.to_owned()
            } else {
                format!(r#""POSITION":0,"{semantic}":1"#)
            };
            let doc = model(
                &[
                    accessor("VEC3", 5126, 3, false),
                    accessor(kind, component, 3, normalized),
                ],
                &format!(r#", "meshes":[{{"primitives":[{{"attributes":{{{attributes}}}}}]}}]"#),
            );
            assert_eq!(
                validate(&doc).is_ok(),
                accepted,
                "{semantic} {kind} {component}"
            );
        }
    }

    #[test]
    fn skins_validate_matrix_layout_and_scene_local_joint_reachability() {
        for (kind, matrices, scene, accepted) in [
            ("MAT4", 1, "0,1", true),
            ("MAT4", 2, "0,1", true),
            ("VEC4", 1, "0,1", false),
            ("MAT4", 1, "0", false),
        ] {
            let doc = model(
                &[
                    accessor("VEC3", 5126, 3, false),
                    accessor(kind, 5126, matrices, false),
                ],
                &format!(
                    r#", "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"nodes":[{{"mesh":0,"skin":0}},{{}}],"skins":[{{"joints":[1],"inverseBindMatrices":1}}],"scenes":[{{"nodes":[{scene}]}}]"#
                ),
            );
            assert_eq!(
                validate(&doc).is_ok(),
                accepted,
                "{kind} {matrices} {scene}"
            );
        }
        let no_matrices = model(
            &[accessor("VEC3", 5126, 3, false)],
            r#", "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0,"skin":0,"children":[1]},{}],"skins":[{"joints":[1]}],"scenes":[{"nodes":[0]},{"nodes":[0]}]"#,
        );
        assert!(
            validate(&no_matrices).is_ok(),
            "identity bind matrices and scene reuse are supported"
        );
    }

    #[test]
    fn skin_joint_counts_are_distinct_and_have_enough_bind_matrices() {
        for (joints, accepted) in [("1", true), ("1,1", false), ("1,2", false), ("", false)] {
            let doc = model(
                &[accessor("MAT4", 5126, 1, false)],
                &format!(
                    r#", "nodes":[{{}},{{}},{{}}],"skins":[{{"joints":[{joints}],"inverseBindMatrices":0}}]"#
                ),
            );
            assert_eq!(validate(&doc).is_ok(), accepted, "{joints}");
        }
        let scene_reuse = model(
            &[accessor("VEC3", 5126, 3, false)],
            r#", "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0,"skin":0,"children":[1]},{},{"mesh":0,"skin":0}],"skins":[{"joints":[1]}],"scenes":[{"nodes":[0]},{"nodes":[2]}]"#,
        );
        assert!(validate(&scene_reuse).unwrap_err().contains("every scene"));
        for count in [MAX_JOINTS, MAX_JOINTS + 1] {
            let nodes = vec!["{}"; count].join(",");
            let joints = (0..count)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let doc = model(
                &[],
                &format!(r#", "nodes":[{nodes}],"skins":[{{"joints":[{joints}]}}]"#),
            );
            assert_eq!(validate(&doc).is_ok(), count == MAX_JOINTS);
        }
    }

    fn morph_model(kind: &str, count: usize, targets: usize, weights: Option<usize>) -> Document {
        let targets = std::iter::repeat_n(r#"{"POSITION":1}"#, targets)
            .collect::<Vec<_>>()
            .join(",");
        let weights = weights.map_or(String::new(), |count| {
            format!(r#", "weights":[{}]"#, vec!["0"; count].join(","))
        });
        model(
            &[
                accessor("VEC3", 5126, 3, false),
                accessor(kind, 5126, count, false),
            ],
            &format!(
                r#", "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"targets":[{targets}]}}]{weights}}}]"#
            ),
        )
    }

    #[test]
    fn morph_layout_count_and_weights_match_the_renderer_contract() {
        for (kind, count, targets, weights, accepted) in [
            ("VEC3", 3, 1, None, true),
            ("VEC3", 3, 1, Some(1), true),
            ("VEC3", 2, 1, None, false),
            ("VEC2", 3, 1, None, false),
            ("VEC3", 3, 1, Some(257), false),
            ("VEC3", 3, 256, Some(256), true),
            ("VEC3", 3, 257, None, false),
        ] {
            let result = validate(&morph_model(kind, count, targets, weights));
            assert_eq!(
                result.is_ok(),
                accepted,
                "{kind} {count} {targets} {weights:?}: {result:?}"
            );
        }
        let unequal = model(
            &[accessor("VEC3", 5126, 3, false)],
            r#", "meshes":[{"primitives":[{"attributes":{"POSITION":0},"targets":[{"POSITION":0}]},{"attributes":{"POSITION":0}}]}]"#,
        );
        assert!(validate(&unequal)
            .unwrap_err()
            .contains("same morph target count"));
        let node_weights = model(
            &[accessor("VEC3", 5126, 3, false)],
            r#", "meshes":[{"primitives":[{"attributes":{"POSITION":0},"targets":[{"POSITION":0}]}]}],"nodes":[{"mesh":0,"weights":[0,0]}]"#,
        );
        assert!(validate(&node_weights)
            .unwrap_err()
            .contains("morph weights"));
    }

    fn animation_model(
        property: &str,
        input: String,
        output: String,
        interpolation: &str,
    ) -> Document {
        model(
            &[input, output],
            &format!(
                r#", "nodes":[{{}}],"animations":[{{"samplers":[{{"input":0,"output":1,"interpolation":"{interpolation}"}}],"channels":[{{"sampler":0,"target":{{"node":0,"path":"{property}"}}}}]}}]"#
            ),
        )
    }

    #[test]
    fn animation_layout_and_interpolation_cardinality_precede_typed_readers() {
        for (property, kind, component, normalized, inputs, outputs, interpolation, accepted) in [
            ("translation", "VEC3", 5126, false, 1, 1, "LINEAR", true),
            ("translation", "VEC3", 5126, false, 2, 2, "STEP", true),
            ("scale", "VEC3", 5126, false, 2, 6, "CUBICSPLINE", true),
            ("rotation", "VEC4", 5126, false, 2, 2, "LINEAR", true),
            ("rotation", "VEC4", 5123, true, 2, 2, "LINEAR", true),
            ("rotation", "VEC4", 5125, false, 2, 2, "LINEAR", false),
            ("rotation", "VEC3", 5126, false, 2, 2, "LINEAR", false),
            (
                "translation",
                "VEC3",
                5126,
                false,
                2,
                2,
                "CUBICSPLINE",
                false,
            ),
            ("scale", "VEC3", 5126, false, 1, 3, "CUBICSPLINE", false),
            ("translation", "VEC3", 5126, false, 2, 1, "LINEAR", false),
        ] {
            let doc = animation_model(
                property,
                accessor("SCALAR", 5126, inputs, false),
                accessor(kind, component, outputs, normalized),
                interpolation,
            );
            let result = validate(&doc);
            assert_eq!(
                result.is_ok(),
                accepted,
                "{property} {kind} {component} {interpolation}: {result:?}"
            );
        }
        let doc = animation_model(
            "scale",
            accessor("VEC2", 5126, 2, false),
            accessor("VEC3", 5126, 2, false),
            "LINEAR",
        );
        assert!(validate(&doc).unwrap_err().contains("animation input"));
        let sparse_input = r#"{"componentType":5126,"count":2,"type":"SCALAR","sparse":{"count":1,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":0}}}"#.to_owned();
        let doc = animation_model(
            "scale",
            sparse_input,
            accessor("VEC3", 5126, 2, false),
            "LINEAR",
        );
        assert!(validate(&doc).unwrap_err().contains("non-sparse"));
        let duplicate = model(
            &[
                accessor("SCALAR", 5126, 2, false),
                accessor("VEC3", 5126, 2, false),
            ],
            r#", "nodes":[{}],"animations":[{"samplers":[{"input":0,"output":1}],"channels":[{"sampler":0,"target":{"node":0,"path":"scale"}},{"sampler":0,"target":{"node":0,"path":"scale"}}]}]"#,
        );
        assert!(validate(&duplicate).unwrap_err().contains("distinct"));
    }

    #[test]
    fn morph_weight_animation_multiplies_samples_by_target_count() {
        for (outputs, interpolation, accepted) in [
            (4, "LINEAR", true),
            (12, "CUBICSPLINE", true),
            (2, "LINEAR", false),
        ] {
            let doc = model(
                &[
                    accessor("VEC3", 5126, 3, false),
                    accessor("SCALAR", 5126, 2, false),
                    accessor("SCALAR", 5126, outputs, false),
                ],
                &format!(
                    r#", "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"targets":[{{"POSITION":0}},{{"POSITION":0}}]}}]}}],"nodes":[{{"mesh":0}}],"animations":[{{"samplers":[{{"input":1,"output":2,"interpolation":"{interpolation}"}}],"channels":[{{"sampler":0,"target":{{"node":0,"path":"weights"}}}}]}}]"#
                ),
            );
            assert_eq!(
                validate(&doc).is_ok(),
                accepted,
                "{outputs} {interpolation}"
            );
        }
        let no_morph = animation_model(
            "weights",
            accessor("SCALAR", 5126, 2, false),
            accessor("SCALAR", 5126, 2, false),
            "LINEAR",
        );
        assert!(validate(&no_morph).unwrap_err().contains("morph targets"));
    }
}
