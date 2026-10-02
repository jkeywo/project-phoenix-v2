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
