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
        .filter_map(|(_, bytes)| {
            crate::core::codec::from_json_bytes::<crate::entities::planet_texture::TextureSource>(
                bytes,
            )
            .ok()
        })
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
        let source: crate::entities::planet_texture::TextureSource =
            crate::core::codec::from_json_bytes(bytes).map_err(|e| e.to_string())?;
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
#[path = "pack_asset_validation_sound_catalog_tests.rs"]
mod sound_catalog_tests;

pub fn validate(path: &str, bytes: &[u8], resolve: &AssetResolver<'_>) -> Result<(), String> {
    let extension = path.rsplit('.').next().unwrap_or_default();
    match extension {
        "png" | "jpg" | "jpeg" | "ktx2" => image(bytes, ImageType::Extension(extension)),
        "mp3" | "ogg" | "wav" => crate::audio_decode::decode(bytes.to_vec(), extension).map(|_| ()),
        "bin" if !bytes.is_empty() => Ok(()), // Opaque glTF buffer; referencing models validate its ranges.
        "ptex" => {
            let source: crate::entities::planet_texture::TextureSource =
                crate::core::codec::from_json_bytes(bytes).map_err(|e| e.to_string())?;
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
#[path = "pack_asset_validation_tests.rs"]
mod tests;
