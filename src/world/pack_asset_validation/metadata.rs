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
mod tests {
    use super::*;

    #[test]
    fn unsafe_upstream_references_are_refused_without_entering_their_hooks() {
        for source in [
            r#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":99}}]}]}"#,
            r#"{"asset":{"version":"2.0"},"nodes":[{}],"animations":[{"channels":[{"sampler":0,"target":{"node":99,"path":"translation"}}],"samplers":[]}]}"#,
            r#"{"asset":{"version":"2.0"},"nodes":[{}],"animations":[{"channels":[{"sampler":0,"target":{"node":0,"path":"unknown"}}],"samplers":[]}]}"#,
        ] {
            assert!(parse_model(source.as_bytes()).is_err());
        }
    }

    #[test]
    fn hierarchy_checks_cover_unreachable_cycles_root_aliasing_and_depth() {
        for (nodes, scenes, accepted) in [
            (
                r#"[{},{"children":[2]},{"children":[1]}]"#,
                r#"[{"nodes":[0]}]"#,
                false,
            ),
            (r#"[{"children":[1,1]},{}]"#, "[]", false),
            (r#"[{"children":[2]},{"children":[2]},{}]"#, "[]", false),
            (r#"[{"children":[1]},{}]"#, r#"[{"nodes":[0,1]}]"#, false),
            ("[{}]", r#"[{"nodes":[0,0]}]"#, false),
            (
                r#"[{"children":[1]},{}]"#,
                r#"[{"nodes":[0]},{"nodes":[0]}]"#,
                true,
            ),
        ] {
            let source =
                format!(r#"{{"asset":{{"version":"2.0"}},"nodes":{nodes},"scenes":{scenes}}}"#);
            let model = parse_model(source.as_bytes()).unwrap();
            assert_eq!(validate_node_hierarchy(&model.document).is_ok(), accepted);
        }
        for count in [MAX_NODE_DEPTH, MAX_NODE_DEPTH + 1] {
            let nodes = (0..count)
                .map(|node| {
                    if node + 1 == count {
                        "{}".to_owned()
                    } else {
                        format!(r#"{{"children":[{}]}}"#, node + 1)
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            let source = format!(r#"{{"asset":{{"version":"2.0"}},"nodes":[{nodes}]}}"#);
            let model = parse_model(source.as_bytes()).unwrap();
            assert_eq!(
                validate_node_hierarchy(&model.document).is_ok(),
                count == MAX_NODE_DEPTH
            );
        }
    }

    #[test]
    fn expansion_is_bounded_without_allocating_the_advertised_samples() {
        for (count, tail, accepted) in [
            (MAX_EXPANDED_MODEL_BYTES / 4, "", true),
            (
                MAX_EXPANDED_MODEL_BYTES / 4,
                r#",{"bufferView":0,"componentType":5126,"count":1,"type":"SCALAR"}"#,
                false,
            ),
            (1_000_000_000, "", false),
        ] {
            let source = format!(
                r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"uri":"mesh.bin","byteLength":4}}],"bufferViews":[{{"buffer":0,"byteLength":4}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":{count},"type":"SCALAR"}}{tail}]}}"#
            );
            let model = parse_model(source.as_bytes()).unwrap();
            assert_eq!(validate_expanded_budget(&model.document).is_ok(), accepted);
        }
    }

    #[test]
    fn raw_lengths_and_counts_cannot_truncate_in_browser_loaders() {
        for member in [
            r#""buffers":[{"byteLength":4294967299}]"#,
            r#""buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteLength":4294967299}]"#,
            r#""buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteLength":4,"byteOffset":4294967299}]"#,
            r#""accessors":[{"componentType":5126,"count":4294967299,"type":"SCALAR"}]"#,
            r#""accessors":[{"componentType":5126,"count":1,"byteOffset":4294967299,"type":"SCALAR"}]"#,
            r#""accessors":[{"componentType":5126,"count":1,"type":"SCALAR","sparse":{"count":4294967299,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":0}}}]"#,
            r#""accessors":[{"componentType":5126,"count":1,"type":"SCALAR","sparse":{"count":1,"indices":{"bufferView":0,"componentType":5121,"byteOffset":4294967299},"values":{"bufferView":0}}}]"#,
            r#""accessors":[{"componentType":5126,"count":1,"type":"SCALAR","sparse":{"count":1,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":0,"byteOffset":4294967299}}}]"#,
        ] {
            let source = format!(r#"{{"asset":{{"version":"2.0"}},{member}}}"#);
            assert!(parse_model(source.as_bytes())
                .unwrap_err()
                .contains("32-bit"));
        }
    }

    #[test]
    fn reused_sparse_morph_targets_are_charged_for_their_generated_image() {
        let targets = std::iter::repeat_n(r#"{"POSITION":0}"#, 256)
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"uri":"mesh.bin","byteLength":16}}],"bufferViews":[{{"buffer":0,"byteLength":1}},{{"buffer":0,"byteOffset":4,"byteLength":12}}],"accessors":[{{"componentType":5126,"count":100000,"type":"VEC3","min":[0,0,0],"max":[1,1,1],"sparse":{{"count":1,"indices":{{"bufferView":0,"componentType":5121}},"values":{{"bufferView":1}}}}}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"targets":[{targets}]}}]}}]}}"#
        );
        let model = parse_model(source.as_bytes()).unwrap();
        assert!(validate_expanded_budget(&model.document)
            .unwrap_err()
            .contains("model budget"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shipped_models_remain_compatible_with_metadata_admission() {
        let mut pending = vec![std::path::PathBuf::from("assets/models")];
        let mut checked = 0;
        while let Some(path) = pending.pop() {
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.extend(
                    std::fs::read_dir(path)
                        .unwrap()
                        .map(|entry| entry.unwrap().path()),
                );
            } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == "glb") {
                let result = (|| {
                    let model = parse_model(&std::fs::read(&path).unwrap())?;
                    validate_node_hierarchy(&model.document)?;
                    validate_expanded_budget(&model.document)?;
                    super::super::semantics::validate(&model.document)
                })();
                assert!(result.is_ok(), "{}: {result:?}", path.display());
                checked += 1;
            }
        }
        assert!(checked > 0, "the shipped model corpus must be exercised");
    }
}
