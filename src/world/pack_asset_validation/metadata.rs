//! Pure model metadata admission before renderer or buffer access.
use std::collections::BTreeSet;

/// Guard the raw references whose upstream validation hooks index directly or
/// omit validation, then retain the library's complete normal validation pass.
pub(super) fn parse_model(bytes: &[u8]) -> Result<gltf::Gltf, String> {
    let model =
        gltf::Gltf::from_slice_without_validation(bytes).map_err(|error| error.to_string())?;
    let raw = model.document.into_json();
    // glTF's USize64 accessors cast to usize. Refuse nonportable values before
    // those accessors can silently truncate them in a 32-bit browser build.
    let portable = |value| {
        u32::try_from(value).map(|_| ()).map_err(|_| {
            "GLB lengths, counts and offsets must fit the browser's 32-bit address space".to_owned()
        })
    };
    for buffer in &raw.buffers {
        portable(buffer.byte_length.0)?;
    }
    for view in &raw.buffer_views {
        portable(view.byte_length.0)?;
        portable(view.byte_offset.map_or(0, |offset| offset.0))?;
    }
    for accessor in &raw.accessors {
        portable(accessor.count.0)?;
        portable(accessor.byte_offset.map_or(0, |offset| offset.0))?;
        if let Some(sparse) = &accessor.sparse {
            portable(sparse.count.0)?;
            portable(sparse.indices.byte_offset.0)?;
            portable(sparse.values.byte_offset.0)?;
        }
    }
    for mesh in &raw.meshes {
        for primitive in &mesh.primitives {
            if primitive
                .attributes
                .values()
                .any(|index| index.value() >= raw.accessors.len())
            {
                return Err("GLB primitive references an absent accessor".into());
            }
        }
    }
    for animation in &raw.animations {
        for channel in &animation.channels {
            if channel.target.node.value() >= raw.nodes.len()
                || !matches!(
                    channel.target.path,
                    gltf::json::validation::Checked::Valid(_)
                )
            {
                return Err("GLB animation has an invalid target node or property".into());
            }
        }
    }
    let document = gltf::Document::from_json(raw).map_err(|error| error.to_string())?;
    Ok(gltf::Gltf {
        document,
        blob: model.blob,
    })
}

// Sparse accessors can describe billions of zero-filled values in a tiny file.
// Bound their expanded representation before any reader can iterate or allocate
// it. This is a portable content limit, not a claim about total renderer memory.
pub(super) const MAX_EXPANDED_MODEL_BYTES: usize = 256 * 1024 * 1024;

pub(super) fn validate_expanded_budget(model: &gltf::Document) -> Result<(), String> {
    let mut expanded = 0usize;
    let mut charge = |bytes: Option<usize>| {
        expanded = bytes
            .and_then(|bytes| expanded.checked_add(bytes))
            .filter(|bytes| *bytes <= MAX_EXPANDED_MODEL_BYTES)
            .ok_or("GLB expanded data exceed the 256 MiB model budget")?;
        Ok::<_, String>(())
    };
    for accessor in model.accessors() {
        let components = accessor.size() / accessor.data_type().size();
        charge(
            accessor
                .count()
                .checked_mul(components)
                .and_then(|count| count.checked_mul(4)),
        )?;
    }
    // Primitive conversion and generated morph images allocate per use, even
    // when every primitive or target references the same small sparse accessor.
    for mesh in model.meshes() {
        for primitive in mesh.primitives() {
            let count = primitive
                .get(&gltf::Semantic::Positions)
                .map_or(0, |a| a.count());
            let indices = primitive.indices().map_or(0, |a| a.count());
            let vertices = count.max(indices); // Flat-normal generation can duplicate indexed vertices.
            for (_, accessor) in primitive.attributes() {
                let components = accessor.size() / accessor.data_type().size();
                charge(
                    vertices
                        .checked_mul(components)
                        .and_then(|n| n.checked_mul(4)),
                )?;
            }
            charge(indices.checked_mul(4))?;
            // Reserve generated normal/tangent vectors, whether or not this
            // material ultimately requires them on a particular renderer.
            charge(vertices.checked_mul(7 * 4))?;
            charge(
                count
                    .checked_mul(primitive.morph_targets().len())
                    .and_then(|n| n.checked_mul(9 * 4)),
            )?;
        }
    }
    Ok(())
}

// The renderer traverses node hierarchies recursively. Keep admission iterative
// and bound accepted depth before either native or browser loading begins.
pub(super) const MAX_NODE_DEPTH: usize = 128;

pub(super) fn validate_node_hierarchy(model: &gltf::Document) -> Result<(), String> {
    let children: Vec<Vec<usize>> = model
        .nodes()
        .map(|node| node.children().map(|child| child.index()).collect())
        .collect();
    let mut parents = vec![None; children.len()];
    for (parent, nodes) in children.iter().enumerate() {
        for &child in nodes {
            if parents[child].replace(parent).is_some() {
                return Err("GLB node has multiple parents or repeated child references".into());
            }
        }
    }
    let mut pending: Vec<_> = parents
        .iter()
        .enumerate()
        .filter_map(|(node, parent)| parent.is_none().then_some((node, 1)))
        .collect();
    let mut visited = 0;
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_NODE_DEPTH {
            return Err(format!(
                "GLB node hierarchy exceeds {MAX_NODE_DEPTH} levels"
            ));
        }
        visited += 1;
        pending.extend(children[node].iter().map(|&child| (child, depth + 1)));
    }
    if visited != children.len() {
        return Err("GLB node hierarchy contains a cycle".into());
    }
    for scene in model.scenes() {
        let mut roots = BTreeSet::new();
        for node in scene.nodes() {
            if parents[node.index()].is_some() || !roots.insert(node.index()) {
                return Err("GLB scene roots must be distinct parentless nodes".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
