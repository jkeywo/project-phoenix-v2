//! The offline browser adapter. Initialising the host WASM module does not
//! initialise a host App; these exports have no route to its live caches.
use wasm_bindgen::prelude::*;

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
