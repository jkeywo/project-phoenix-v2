//! The offline browser adapter. Initialising the host WASM module does not
//! initialise a host App; these exports have no route to its live caches.
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn wasm_workshop_ship_schema() -> Result<String, JsValue> {
    serde_json::to_string(&super::ship_schema())
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_asset_dependencies(bytes: &[u8]) -> Vec<String> {
    let Ok(members) = crate::world::mod_pack::read_store_zip_bytes(bytes) else {
        return Vec::new();
    };
    members
        .iter()
        .filter_map(|(path, bytes)| {
            crate::world::pack_asset_validation::required_assets(path, bytes).ok()
        })
        .flatten()
        .filter(|path| !members.contains_key(path))
        .chain(
            members
                .keys()
                .filter(|path| path.ends_with(".bin"))
                .cloned(),
        )
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[wasm_bindgen]
pub fn wasm_workshop_test_catalog(files: &str) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::to_json(&super::test_source::catalog(files))
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_test_check_selection(files: &str, launch: &str) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let launch: super::test_protocol::Launch =
        crate::core::codec::from_json_bytes(launch.as_bytes())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::to_json(&super::test_source::validate_selection(
        files,
        &launch.selection,
    ))
    .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_validate_pack(bytes: &[u8], dependencies: &str) -> Result<String, JsValue> {
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let result = super::validate_pack(bytes, &dependencies);
    crate::core::codec::to_json(&result).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_fields(source: &str, document_path: &str) -> Result<String, JsValue> {
    let fields = super::document::fields(source, document_path)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::to_json(&fields).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_patch(source: &str, patch: &str) -> Result<String, JsValue> {
    let patch = crate::core::codec::from_json(patch)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    super::document::patch(source, &patch).map_err(|error| JsValue::from_str(&error.to_string()))
}

/// The faction and complexity catalog over the draft's text members plus the
/// same explicit dependency bundle validation takes (issue #1474).
#[wasm_bindgen]
pub fn wasm_workshop_definitions(files: &str, dependencies: &str) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::to_json(&super::definitions::catalog(&files, &dependencies))
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_edit(source: &str, edit: &str) -> Result<String, JsValue> {
    let edit = crate::core::codec::from_json(edit)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    super::document::edit(source, &edit).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_new_faction(name: &str, uuid: &str) -> Result<String, JsValue> {
    super::definitions::new_faction_source(name, uuid)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// The composition catalog over the draft's text members plus the same
/// explicit dependency bundle validation takes (issue #1475).
#[wasm_bindgen]
pub fn wasm_workshop_composition(files: &str, dependencies: &str) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::to_json(&super::composition::catalog(&files, &dependencies))
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// One composition edit group, refused with the source untouched when it
/// introduces a missing, cyclic, duplicate or disallowed reference.
#[wasm_bindgen]
pub fn wasm_workshop_compose(
    files: &str,
    dependencies: &str,
    request: &str,
) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let request = crate::core::codec::from_json(request)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    super::composition::compose(&files, &dependencies, &request)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_new_world(title: &str) -> Result<String, JsValue> {
    super::composition::new_world_source(title)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// One entity template's composition over the draft's text members plus the
/// same explicit dependency bundle validation takes: its includes, the merge
/// order, who owns each effective field, and what may still be added
/// (issue #1476).
#[wasm_bindgen]
pub fn wasm_workshop_entity(
    files: &str,
    dependencies: &str,
    path: &str,
) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::to_json(&super::entity::catalog(&files, &dependencies, path))
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// One entity edit group, refused with the source untouched when it introduces
/// a missing, cyclic, self or disallowed include, a component the runtime does
/// not know, or a composed document that no longer parses.
#[wasm_bindgen]
pub fn wasm_workshop_entity_edit(
    files: &str,
    dependencies: &str,
    request: &str,
) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let request = crate::core::codec::from_json(request)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    super::entity::compose(&files, &dependencies, &request)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// Write one inherited value into the local document as an exact-source
/// override — the only place a runtime value is serialised into source, and it
/// writes new local text only.
#[wasm_bindgen]
pub fn wasm_workshop_entity_materialise(
    files: &str,
    dependencies: &str,
    path: &str,
    address: &str,
) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    super::entity::materialise(&files, &dependencies, path, address)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// One world's GM role presets and typed widgets over the draft's text members
/// plus the same explicit dependency bundle validation takes: every authored
/// value with its line, which references resolve, and the vocabularies the
/// RUNTIME owns (issue #1477). The panel-id and quick-action vocabularies stay
/// in the browser — see `presets`' module header.
#[wasm_bindgen]
pub fn wasm_workshop_presets(
    files: &str,
    dependencies: &str,
    path: &str,
) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::to_json(&super::presets::catalog(&files, &dependencies, path))
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// One preset edit group, refused with the source untouched when it introduces
/// a preset or widget rule the world did not already break.
#[wasm_bindgen]
pub fn wasm_workshop_presets_edit(
    files: &str,
    dependencies: &str,
    request: &str,
) -> Result<String, JsValue> {
    let files = crate::core::codec::from_json(files)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let dependencies = crate::core::codec::from_json(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let request = crate::core::codec::from_json(request)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    super::presets::compose(&files, &dependencies, &request)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

/// The `[[gm_role_preset]]` block a new preset is appended as, asserted by the
/// runtime's own world reader before it is returned.
#[wasm_bindgen]
pub fn wasm_workshop_new_preset(id: &str, label: &str) -> Result<String, JsValue> {
    super::presets::new_preset_source(id, label)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}
