use super::*;
const PATH: &str = "assets/models/ship/probe.glb";
const PNG: &[u8] = include_bytes!("../../../assets/viewscreen/cap-top.png");

#[test]
fn unsafe_index_reader_types_are_refused_before_fetching_buffers() {
    let bytes = glb(
        r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":40}],"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":4}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5126,"count":1,"type":"SCALAR"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"mode":0}]}]}"#,
    );
    let error = validate(PATH, &bytes, &|_| panic!("refuse before buffer access")).unwrap_err();
    assert!(error.contains("unsigned scalar"), "{error}");
}

#[test]
fn malformed_reader_references_are_refused_by_validation_and_dependency_preflight() {
    for json in [
        r#"{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":99}}]}]}"#,
        r#"{"asset":{"version":"2.0"},"nodes":[{}],"animations":[{"channels":[{"sampler":0,"target":{"node":99,"path":"translation"}}],"samplers":[]}]}"#,
        r#"{"asset":{"version":"2.0"},"nodes":[{}],"animations":[{"channels":[{"sampler":0,"target":{"node":0,"path":"unknown"}}],"samplers":[]}]}"#,
    ] {
        let bytes = glb(json);
        assert!(validate(PATH, &bytes, &|_| None).is_err());
        assert!(required_assets(PATH, &bytes).is_err());
    }
}

#[test]
fn tiny_sparse_models_cannot_request_unbounded_expansion() {
    let bytes = glb(
        r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":16}],"bufferViews":[{"buffer":0,"byteLength":1},{"buffer":0,"byteOffset":4,"byteLength":12}],"accessors":[{"componentType":5126,"count":1000000000,"type":"VEC3","min":[0,0,0],"max":[1,1,1],"sparse":{"count":1,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}]}"#,
    );
    let error = validate(PATH, &bytes, &|_| {
        panic!("expansion must be bounded before resolving or decoding buffers")
    })
    .unwrap_err();
    assert!(error.contains("model budget"), "{error}");
}

#[test]
fn expanded_budget_is_aggregate_and_accepts_its_exact_boundary() {
    let count = MAX_EXPANDED_MODEL_BYTES / 4;
    for second_count in [0, 1] {
        let extra = if second_count == 0 {
            String::new()
        } else {
            r#",{"bufferView":0,"componentType":5126,"count":1,"type":"SCALAR"}"#.to_owned()
        };
        let bytes = glb(&format!(
            r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"uri":"mesh.bin","byteLength":4}}],"bufferViews":[{{"buffer":0,"byteLength":4}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":{count},"type":"SCALAR"}}{extra}]}}"#
        ));
        let model = parse_model(&bytes).unwrap();
        assert_eq!(
            validate_expanded_budget(&model.document).is_ok(),
            second_count == 0
        );
    }
}

#[test]
fn node_hierarchies_reject_cycles_repeated_parents_and_invalid_scene_roots() {
    for (nodes, scenes, reason) in [
        (r#"[{"children":[0]}]"#, r#"[{"nodes":[0]}]"#, "cycle"),
        (
            r#"[{},{"children":[2]},{"children":[1]}]"#,
            r#"[{"nodes":[0]}]"#,
            "cycle",
        ),
        (
            r#"[{"children":[2]},{"children":[2]},{}]"#,
            "[]",
            "multiple parents",
        ),
        (r#"[{"children":[1,1]},{}]"#, "[]", "repeated child"),
        (
            r#"[{"children":[1]},{}]"#,
            r#"[{"nodes":[0,1]}]"#,
            "scene roots",
        ),
        ("[{}]", r#"[{"nodes":[0,0]}]"#, "scene roots"),
    ] {
        let bytes = glb(&format!(
            r#"{{"asset":{{"version":"2.0"}},"nodes":{nodes},"scenes":{scenes}}}"#
        ));
        assert!(validate(PATH, &bytes, &|_| None)
            .unwrap_err()
            .contains(reason));
    }
    // Sharing a root between different scenes is valid; only its parentage
    // within the node graph and repetition inside one scene are constrained.
    let shared = glb(
        r#"{"asset":{"version":"2.0"},"nodes":[{"children":[1]},{}],"scenes":[{"nodes":[0]},{"nodes":[0]}]}"#,
    );
    assert!(validate(PATH, &shared, &|_| None).is_ok());
}

#[test]
fn node_hierarchy_depth_is_bounded_before_recursive_renderer_loading() {
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
        let bytes = glb(&format!(
            r#"{{"asset":{{"version":"2.0"}},"nodes":[{nodes}],"scenes":[{{"nodes":[0]}}]}}"#
        ));
        let result = validate(PATH, &bytes, &|_| None);
        assert_eq!(result.is_ok(), count == MAX_NODE_DEPTH, "{result:?}");
    }
}

#[test]
fn decoded_primitive_indices_and_topology_must_be_renderable() {
    let model = |mode, normals| {
        glb(&format!(
            r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"uri":"mesh.bin","byteLength":68}}],"bufferViews":[{{"buffer":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}},{{"buffer":0,"byteOffset":44,"byteLength":24}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}},{{"bufferView":2,"componentType":5126,"count":2,"type":"VEC3"}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0{normals}}},"indices":1,"mode":{mode}}}]}}]}}"#,
        ))
    };
    let mut buffer = vec![0u8; 68];
    buffer[36..42].copy_from_slice(&[0, 0, 1, 0, 2, 0]);
    assert!(validate(PATH, &model(4, ""), &|_| Some(Arc::from(buffer.as_slice()))).is_ok());
    buffer[40] = 99;
    assert!(
        validate(PATH, &model(4, ""), &|_| Some(Arc::from(buffer.as_slice())))
            .unwrap_err()
            .contains("primitive index")
    );
    buffer[40] = 2;
    for mode in [2, 6] {
        assert!(validate(PATH, &model(mode, ""), &|_| Some(Arc::from(
            buffer.as_slice()
        )))
        .unwrap_err()
        .contains("topology"));
    }
    assert!(
        validate(PATH, &model(4, ",\"NORMAL\":2"), &|_| Some(Arc::from(
            buffer.as_slice()
        )))
        .unwrap_err()
        .contains("count")
    );
}

#[test]
fn sparse_indices_and_padded_matrix_ranges_cannot_escape_validated_buffers() {
    let sparse = glb(
        r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":16}],"bufferViews":[{"buffer":0,"byteLength":1},{"buffer":0,"byteOffset":4,"byteLength":12}],"accessors":[{"componentType":5126,"count":2,"type":"VEC3","sparse":{"count":1,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}]}"#,
    );
    let mut bytes = vec![0; 16];
    bytes[0] = 1;
    assert!(validate(PATH, &sparse, &|_| Some(Arc::from(bytes.as_slice()))).is_ok());
    bytes[0] = 2;
    assert!(
        validate(PATH, &sparse, &|_| Some(Arc::from(bytes.as_slice())))
            .unwrap_err()
            .contains("sparse indices")
    );
    let matrix = |length| {
        glb(&format!(
            r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"uri":"mesh.bin","byteLength":{length}}}],"bufferViews":[{{"buffer":0,"byteLength":{length}}}],"accessors":[{{"bufferView":0,"componentType":5121,"count":1,"type":"MAT3"}}]}}"#
        ))
    };
    assert!(validate(PATH, &matrix(9), &|_| Some(Arc::from([0u8; 9]))).is_err());
    assert!(validate(PATH, &matrix(11), &|_| Some(Arc::from([0u8; 11]))).is_ok());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn planet_descriptors_require_complete_source_containers_and_decoded_fallbacks() {
    let source = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/planets/gas_giant/surface_colour.uastc.ktx2"
    ))
    .unwrap();
    let fallback = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/planets/gas_giant/surface_colour.ktx2"
    ))
    .unwrap();
    let descriptor = br#"{"source":"textures/source.ktx2","fallback":"textures/fallback.ktx2"}"#;
    let resolve = |path: &str| match path {
        "assets/textures/source.ktx2" => Some(Arc::from(source.as_slice())),
        "assets/textures/fallback.ktx2" => Some(Arc::from(fallback.as_slice())),
        _ => None,
    };
    let sources = descriptor_sources([("assets/textures/colour.ptex", descriptor.as_slice())]);
    assert!(validate_member("assets/textures/source.ktx2", &source, &resolve, &sources).is_ok());
    assert!(validate("assets/textures/colour.ptex", descriptor, &resolve).is_ok());
    assert!(validate_member(
        "assets/textures/source.ktx2",
        &source[..80],
        &resolve,
        &sources
    )
    .is_err());
    assert!(
        validate("assets/textures/colour.ptex", descriptor, &|path| {
            if path.ends_with("fallback.ktx2") {
                Some(Arc::from(&b"broken"[..]))
            } else {
                resolve(path)
            }
        })
        .is_err()
    );
}

fn glb(json: &str) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bytes = b"glTF".to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(((20 + json.len()) as u32).to_le_bytes());
    bytes.extend((json.len() as u32).to_le_bytes());
    bytes.extend(b"JSON");
    bytes.extend(json);
    bytes
}

#[test]
fn external_images_resolve_exact_snapshot_bytes_and_decode_them() {
    let model = glb(r#"{"asset":{"version":"2.0"},"images":[{"uri":"../texture.png"}]}"#);
    assert_eq!(
        required_assets(PATH, &model).unwrap(),
        ["assets/models/texture.png".to_owned()].into()
    );
    assert!(
        validate(PATH, &model, &|path| (path == "assets/models/texture.png")
            .then(|| Arc::from(PNG)))
        .is_ok()
    );
    assert!(validate(PATH, &model, &|_| None)
        .unwrap_err()
        .contains("Missing immutable"));
    assert!(validate(PATH, &model, &|_| Some(Arc::from(&b"not an image"[..]))).is_err());
}

#[test]
fn external_buffers_and_accessor_ranges_are_checked_against_actual_bytes() {
    let model = glb(
        r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}]}"#,
    );
    assert!(validate(PATH, &model, &|_| Some(Arc::from([0u8; 12]))).is_ok());
    assert!(validate(PATH, &model, &|_| Some(Arc::from([0u8; 8])))
        .unwrap_err()
        .contains("truncated"));
    let bad_accessor = glb(
        r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"VEC3"}]}"#,
    );
    assert!(validate(PATH, &bad_accessor, &|_| Some(Arc::from([0u8; 12]))).is_err());
}

#[test]
fn embedded_images_and_buffers_are_decoded_instead_of_trusting_data_uri_headers() {
    let encoded = base64::engine::general_purpose::STANDARD.encode(PNG);
    let image = glb(&format!(
        r#"{{"asset":{{"version":"2.0"}},"images":[{{"uri":"data:image/png;base64,{encoded}"}}]}}"#
    ));
    assert!(validate(PATH, &image, &|_| None).is_ok());
    for uri in [
        "data:image/png;base64,AAAA",
        "data:image/png;base64,%%",
        "data:image/png",
    ] {
        let model = glb(&format!(
            r#"{{"asset":{{"version":"2.0"}},"images":[{{"uri":"{uri}"}}]}}"#
        ));
        assert!(validate(PATH, &model, &|_| None).is_err(), "{uri}");
    }
    let truncated = glb(
        r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"data:application/gltf-buffer;base64,AAAA","byteLength":12}]}"#,
    );
    assert!(validate(PATH, &truncated, &|_| None).is_err());
}

#[test]
fn model_dependencies_cannot_leave_content_or_name_a_remote_url() {
    for uri in [
        "../../../escape.png",
        "https://example.invalid/image.png",
        "/outside.png",
        "%2e%2e/escape.png",
    ] {
        let model = glb(&format!(
            r#"{{"asset":{{"version":"2.0"}},"images":[{{"uri":"{uri}"}}]}}"#
        ));
        assert!(required_assets(PATH, &model).is_err(), "{uri}");
        assert!(
            validate(PATH, &model, &|_| Some(Arc::from(PNG))).is_err(),
            "{uri}"
        );
    }
}
