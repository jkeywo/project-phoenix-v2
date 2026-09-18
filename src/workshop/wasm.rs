//! The offline browser adapter. Initialising the host WASM module does not
//! initialise a host App; these exports have no route to its live caches.
use wasm_bindgen::prelude::*;

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
    let files = crate::core::codec::decode_workshop_test_source(files)
        .map_err(|error| JsValue::from_str(&error))?;
    crate::core::codec::encode_workshop_test_catalog(&super::test_source::catalog(files))
        .map_err(|error| JsValue::from_str(&error))
}

#[wasm_bindgen]
pub fn wasm_workshop_test_check_selection(files: &str, launch: &str) -> Result<String, JsValue> {
    let files = crate::core::codec::decode_workshop_test_source(files)
        .map_err(|error| JsValue::from_str(&error))?;
    let launch = crate::core::codec::decode_workshop_test_launch(launch.as_bytes())
        .map_err(|error| JsValue::from_str(&error))?;
    crate::core::codec::encode_workshop_validation(&super::test_source::validate_selection(
        files,
        &launch.selection,
    ))
    .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_validate_pack(bytes: &[u8], dependencies: &str) -> Result<String, JsValue> {
    let dependencies = crate::core::codec::decode_workshop_dependencies(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let result = super::validate_pack(bytes, &dependencies);
    crate::core::codec::encode_workshop_validation(&result)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_fields(source: &str, document_path: &str) -> Result<String, JsValue> {
    let fields = super::document::fields(source, document_path)
        .map_err(|error| JsValue::from_str(&error))?;
    crate::core::codec::encode_workshop_fields(&fields)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn wasm_workshop_patch(source: &str, patch: &str) -> Result<String, JsValue> {
    let patch = crate::core::codec::decode_workshop_patch(patch)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    super::document::patch(source, &patch).map_err(|error| JsValue::from_str(&error))
}

/// The faction and complexity catalog over the draft's text members plus the
/// same explicit dependency bundle validation takes (issue #1474).
#[wasm_bindgen]
pub fn wasm_workshop_definitions(files: &str, dependencies: &str) -> Result<String, JsValue> {
    let files = crate::core::codec::decode_workshop_definition_files(files)
        .map_err(|error| JsValue::from_str(&error))?;
    let dependencies = crate::core::codec::decode_workshop_dependencies(dependencies)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    crate::core::codec::encode_workshop_definition_catalog(&super::definitions::catalog(
        &files,
        &dependencies,
    ))
    .map_err(|error| JsValue::from_str(&error))
}

#[wasm_bindgen]
pub fn wasm_workshop_edit(source: &str, edit: &str) -> Result<String, JsValue> {
    let edit = crate::core::codec::decode_workshop_edit(edit)
        .map_err(|error| JsValue::from_str(&error))?;
    super::document::edit(source, &edit).map_err(|error| JsValue::from_str(&error))
}

#[wasm_bindgen]
pub fn wasm_workshop_new_faction(name: &str, uuid: &str) -> Result<String, JsValue> {
    super::definitions::new_faction_source(name, uuid).map_err(|error| JsValue::from_str(&error))
}
