//! Asset admission uses the same glTF/image/audio decoders as presentation.
use base64::Engine;
use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
use std::{collections::BTreeSet, sync::Arc};

mod metadata;
mod semantics;
use metadata::{parse_model, validate_expanded_budget, validate_node_hierarchy};
#[cfg(test)]
use metadata::{MAX_EXPANDED_MODEL_BYTES, MAX_NODE_DEPTH};

pub type AssetResolver<'a> = dyn Fn(&str) -> Option<Arc<[u8]>> + 'a;

/// Compressed descriptor sources have a guaranteed, decoded native fallback.
/// Their container is validated here; the optional browser transcoder may still
/// select that fallback, as it does for shipped planet textures.
pub fn descriptor_sources<'a>(
    members: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> BTreeSet<String> {
    members
        .into_iter()
        .filter(|(path, _)| path.ends_with(".ptex"))
        .filter_map(|(_, bytes)| crate::core::codec::decode_planet_texture_source(bytes).ok())
        .filter_map(|source| local_reference(&source.source, "assets/descriptor").ok())
        .collect()
}

fn ktx_container(bytes: &[u8]) -> Result<(), String> {
    let texture =
        ktx2::Reader::new(bytes).map_err(|error| format!("Invalid KTX2 container: {error:?}"))?;
    if texture.levels().any(|level| level.data.is_empty()) {
        return Err("KTX2 contains an empty mip level".into());
    }
    Ok(())
}

pub fn validate_member(
    path: &str,
    bytes: &[u8],
    resolve: &AssetResolver<'_>,
    sources: &BTreeSet<String>,
) -> Result<(), String> {
    if path.ends_with(".ktx2") && sources.contains(path) {
        ktx_container(bytes)
    } else {
        validate(path, bytes, resolve)
    }
}

fn image(bytes: &[u8], kind: ImageType<'_>) -> Result<(), String> {
    Image::from_buffer(
        bytes,
        kind,
        CompressedImageFormats::all(),
        true,
        ImageSampler::Default,
        Default::default(),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn local_reference(uri: &str, source: &str) -> Result<String, String> {
    if uri.is_empty()
        || uri.contains(['\\', ':', '%', '?', '#'])
        || uri
            .split('/')
            .any(|part| part.is_empty() || part.chars().any(char::is_control))
    {
        return Err("Asset references must stay within local content paths".into());
    }
    let mut parts: Vec<_> = source.split('/').collect();
    parts.pop();
    for part in uri.split('/') {
        match part {
            "." => {}
            ".." if parts.len() > 1 => {
                parts.pop();
            }
            ".." => return Err("Asset reference leaves the content root".into()),
            part => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

fn data_uri(uri: &str) -> Result<Option<(&str, Arc<[u8]>)>, String> {
    let Some(data) = uri.strip_prefix("data:") else {
        return Ok(None);
    };
    let (header, data) = data.split_once(',').ok_or("Malformed embedded asset URI")?;
    let (mime, bytes) = if let Some(mime) = header.strip_suffix(";base64") {
        (
            mime,
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|error| error.to_string())?,
        )
    } else {
        // Matches Bevy's embedded-data decoder. URI escapes are disallowed at
        // this admission boundary rather than interpreted differently by two readers.
        if data.contains('%') {
            return Err("Escaped embedded data must use base64".into());
        }
        (header, data.as_bytes().to_vec())
    };
    Ok(Some((mime, Arc::from(bytes))))
}

fn referenced_bytes(
    uri: &str,
    path: &str,
    resolve: &AssetResolver<'_>,
) -> Result<Arc<[u8]>, String> {
    let target = local_reference(uri, path)?;
    resolve(&target).ok_or_else(|| format!("Missing immutable asset dependency {target:?}"))
}

fn element_layout(accessor: &gltf::Accessor<'_>) -> (usize, usize) {
    let rows = match accessor.dimensions() {
        gltf::accessor::Dimensions::Mat2 => 2,
        gltf::accessor::Dimensions::Mat3 => 3,
        gltf::accessor::Dimensions::Mat4 => 4,
        _ => return (accessor.size(), accessor.size()),
    };
    let column = rows * accessor.data_type().size();
    let padded_column = column.div_ceil(4) * 4;
    (rows * padded_column, (rows - 1) * padded_column + column)
}

fn check_span(
    view: &gltf::buffer::View<'_>,
    offset: usize,
    count: usize,
    stride: usize,
    last_size: usize,
) -> Result<(), String> {
    let end = count
        .checked_sub(1)
        .and_then(|count| count.checked_mul(stride))
        .and_then(|size| size.checked_add(last_size))
        .and_then(|size| size.checked_add(offset));
    if stride < last_size || end.is_none_or(|end| end > view.length()) {
        Err("GLB accessor extends beyond its buffer view".into())
    } else {
        Ok(())
    }
}

fn validate_accessor(accessor: &gltf::Accessor<'_>, buffers: &[Arc<[u8]>]) -> Result<(), String> {
    let (stride, last_size) = element_layout(accessor);
    if let Some(view) = accessor.view() {
        check_span(
            &view,
            accessor.offset(),
            accessor.count(),
            view.stride().unwrap_or(stride),
            last_size,
        )?;
    }
    if let Some(sparse) = accessor.sparse() {
        if sparse.count() > accessor.count() {
            return Err("GLB sparse count exceeds its accessor".into());
        }
        let indices = sparse.indices();
        let index_view = indices.view();
        let index_size = indices.index_type().size();
        check_span(
            &index_view,
            indices.offset(),
            sparse.count(),
            index_size,
            index_size,
        )?;
        let values = sparse.values();
        check_span(
            &values.view(),
            values.offset(),
            sparse.count(),
            stride,
            last_size,
        )?;
        if index_view.stride().is_some() || values.view().stride().is_some() {
            return Err("GLB sparse buffers must be tightly packed".into());
        }
        let start = index_view.offset() + indices.offset();
        let data =
            &buffers[index_view.buffer().index()][start..start + sparse.count() * index_size];
        let mut previous = None;
        for bytes in data.chunks_exact(index_size) {
            let index = match indices.index_type() {
                gltf::accessor::sparse::IndexType::U8 => u32::from(bytes[0]),
                gltf::accessor::sparse::IndexType::U16 => u32::from(u16::from_le_bytes(
                    bytes.try_into().expect("checked index width"),
                )),
                gltf::accessor::sparse::IndexType::U32 => {
                    u32::from_le_bytes(bytes.try_into().expect("checked index width"))
                }
            } as usize;
            if index >= accessor.count() || previous.is_some_and(|last| index <= last) {
                return Err("GLB sparse indices must increase within the accessor".into());
            }
            previous = Some(index);
        }
    }
    Ok(())
}

/// Resolve dependency names before fetching any bytes. This is shared by the
/// native snapshot provider and browser preflight; it never reads a live cache.
pub fn required_assets(path: &str, bytes: &[u8]) -> Result<BTreeSet<String>, String> {
    let mut required = BTreeSet::new();
    if path == crate::sound_cues::PATH {
        let source = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
        let catalog = crate::sound_cues::parse_source(source)?;
        required.extend(catalog.cues.into_iter().map(|cue| cue.file));
    } else if path.ends_with(".glb") {
        let model = parse_model(bytes)?;
        for buffer in model.buffers() {
            if let gltf::buffer::Source::Uri(uri) = buffer.source() {
                if !uri.starts_with("data:") {
                    required.insert(local_reference(uri, path)?);
                }
            }
        }
        for texture in model.images() {
            if let gltf::image::Source::Uri { uri, .. } = texture.source() {
                if !uri.starts_with("data:") {
                    required.insert(local_reference(uri, path)?);
                }
            }
        }
    } else if path.ends_with(".ptex") {
        let source = crate::core::codec::decode_planet_texture_source(bytes)?;
        // PlanetTextureLoader resolves these against the asset root, not the descriptor's directory.
        required.insert(local_reference(&source.source, "assets/descriptor")?);
        required.insert(local_reference(&source.fallback, "assets/descriptor")?);
    }
    Ok(required)
}

/// Catalog acceptance resolves the same immutable bytes as playback. A known
/// shipped filename is not proof that this selected content has those bytes.
pub fn validate_sound_catalog(
    source: &str,
    resolve: &AssetResolver<'_>,
) -> Result<crate::sound_cues::Catalog, String> {
    let catalog = crate::sound_cues::parse_source(source)?;
    let files: BTreeSet<_> = catalog.cues.iter().map(|cue| cue.file.as_str()).collect();
    for path in files {
        let bytes =
            resolve(path).ok_or_else(|| format!("Missing immutable sound dependency {path:?}"))?;
        validate(path, &bytes, resolve)
            .map_err(|error| format!("Invalid sound dependency {path:?}: {error}"))?;
    }
    Ok(catalog)
}

#[cfg(test)]
mod sound_catalog_tests {
    use super::*;

    #[test]
    fn sound_catalog_dependencies_are_validated_local_paths_and_decoded_once() {
        const SOURCE: &str = include_str!("../../tests/fixtures/sound-cue-pack.toml");
        const SOUND: &str = "assets/sounds/custom/sonar ping.ogg";
        assert_eq!(
            required_assets(crate::sound_cues::PATH, SOURCE.as_bytes()).unwrap(),
            [SOUND.to_owned()].into()
        );
        let reads = std::cell::Cell::new(0);
        let catalog = validate_sound_catalog(SOURCE, &|path| {
            assert_eq!(path, SOUND);
            reads.set(reads.get() + 1);
            Some(Arc::from(
                include_bytes!("../../assets/sounds/ui_click.ogg").as_slice(),
            ))
        })
        .unwrap();
        assert_eq!(catalog.cues.len(), 2);
        assert_eq!(
            reads.get(),
            1,
            "two definitions sharing one file decode once"
        );
        for path in [
            "https://example.invalid/tone.ogg",
            "assets/sounds/../secret.ogg",
        ] {
            assert!(required_assets(
                crate::sound_cues::PATH,
                SOURCE.replace(SOUND, path).as_bytes()
            )
            .is_err());
        }
        assert!(required_assets(crate::sound_cues::PATH, b"not [valid").is_err());
        assert!(required_assets(crate::sound_cues::PATH, &[255]).is_err());
    }
}

pub fn validate(path: &str, bytes: &[u8], resolve: &AssetResolver<'_>) -> Result<(), String> {
    let extension = path.rsplit('.').next().unwrap_or_default();
    match extension {
        "png" | "jpg" | "jpeg" | "ktx2" => image(bytes, ImageType::Extension(extension)),
        "mp3" | "ogg" | "wav" => crate::audio_decode::decode(bytes.to_vec(), extension).map(|_| ()),
        "bin" if !bytes.is_empty() => Ok(()), // Opaque glTF buffer; referencing models validate its ranges.
        "ptex" => {
            let source = crate::core::codec::decode_planet_texture_source(bytes)?;
            let compressed = referenced_bytes(&source.source, "assets/descriptor", resolve)?;
            ktx_container(&compressed)?;
            let fallback = referenced_bytes(&source.fallback, "assets/descriptor", resolve)?;
            // The runtime falls back to this actual decoded image if browser
            // UASTC transcoding is unavailable or fails. Native always uses it.
            image(&fallback, ImageType::Extension("ktx2"))
        }
        "glb" => {
            let model = parse_model(bytes)?;
            validate_node_hierarchy(&model.document)?;
            validate_expanded_budget(&model.document)?;
            semantics::validate(&model.document)?;
            let mut buffers = Vec::new();
            for buffer in model.buffers() {
                let data: Arc<[u8]> = match buffer.source() {
                    gltf::buffer::Source::Bin => {
                        Arc::from(model.blob.as_deref().ok_or("Missing GLB buffer")?)
                    }
                    gltf::buffer::Source::Uri(uri) => match data_uri(uri)? {
                        Some(("application/octet-stream" | "application/gltf-buffer", bytes)) => {
                            bytes
                        }
                        Some(_) => return Err("Unsupported embedded glTF buffer format".into()),
                        None => referenced_bytes(uri, path, resolve)?,
                    },
                };
                if data.len() < buffer.length() {
                    return Err("GLB buffer is truncated".into());
                }
                buffers.push(data);
            }
            for view in model.views() {
                if view
                    .offset()
                    .checked_add(view.length())
                    .is_none_or(|end| end > view.buffer().length())
                {
                    return Err("GLB view extends beyond its buffer".into());
                }
            }
            for accessor in model.accessors() {
                validate_accessor(&accessor, &buffers)?;
            }
            for mesh in model.meshes() {
                for primitive in mesh.primitives() {
                    if matches!(
                        primitive.mode(),
                        gltf::mesh::Mode::LineLoop | gltf::mesh::Mode::TriangleFan
                    ) {
                        return Err(
                            "GLB primitive topology is not supported by the renderer".into()
                        );
                    }
                    let count = primitive
                        .get(&gltf::Semantic::Positions)
                        .ok_or("GLB primitive has no positions")?
                        .count();
                    // The renderer may duplicate vertices to generate normals.
                    // A byte-valid accessor containing an out-of-range vertex
                    // index would otherwise panic that real loader path.
                    let reader = primitive.reader(|buffer| Some(buffers[buffer.index()].as_ref()));
                    if reader.read_indices().is_some_and(|indices| {
                        indices.into_u32().any(|index| index as usize >= count)
                    }) {
                        return Err("GLB primitive index exceeds its vertex attributes".into());
                    }
                }
            }
            for texture in model.images() {
                match texture.source() {
                    gltf::image::Source::Uri { uri, mime_type } => match data_uri(uri)? {
                        Some((mime, bytes)) => {
                            image(&bytes, ImageType::MimeType(mime_type.unwrap_or(mime)))?
                        }
                        None => {
                            let bytes = referenced_bytes(uri, path, resolve)?;
                            let kind = mime_type.map(ImageType::MimeType).unwrap_or_else(|| {
                                ImageType::Extension(uri.rsplit('.').next().unwrap_or_default())
                            });
                            image(&bytes, kind)?;
                        }
                    },
                    gltf::image::Source::View { view, mime_type } => {
                        let data = buffers[view.buffer().index()]
                            .get(view.offset()..view.offset() + view.length())
                            .ok_or("GLB image is truncated")?;
                        image(data, ImageType::MimeType(mime_type))?;
                    }
                }
            }
            Ok(())
        }
        _ => Err("Unsupported runtime asset format".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PATH: &str = "assets/models/ship/probe.glb";
    const PNG: &[u8] = include_bytes!("../../assets/viewscreen/cap-top.png");

    #[test]
    fn model_reader_limits_match_the_installed_renderer() {
        assert_eq!(semantics::MAX_JOINTS, bevy::pbr::MAX_JOINTS);
        assert_eq!(
            semantics::MAX_MORPH_WEIGHTS,
            bevy::mesh::morph::MAX_MORPH_WEIGHTS
        );
    }

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
        let source = std::fs::read("assets/planets/gas_giant/surface_colour.uastc.ktx2").unwrap();
        let fallback = std::fs::read("assets/planets/gas_giant/surface_colour.ktx2").unwrap();
        let descriptor =
            br#"{"source":"textures/source.ktx2","fallback":"textures/fallback.ktx2"}"#;
        let resolve = |path: &str| match path {
            "assets/textures/source.ktx2" => Some(Arc::from(source.as_slice())),
            "assets/textures/fallback.ktx2" => Some(Arc::from(fallback.as_slice())),
            _ => None,
        };
        let sources = descriptor_sources([("assets/textures/colour.ptex", descriptor.as_slice())]);
        assert!(
            validate_member("assets/textures/source.ktx2", &source, &resolve, &sources).is_ok()
        );
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
}
