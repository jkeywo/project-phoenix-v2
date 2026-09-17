//! One reading of a captured Workshop draft, shared by every disposable
//! surface that boots from exact bytes.
//!
//! The rules here are the isolation guarantee, not convenience: a path outside
//! the authored namespace, a traversal, a case-folded duplicate or a value that
//! is neither text nor bytes is refused rather than repaired, and the caps are
//! what stop one draft from being handed a whole filesystem. A surface that
//! wrote its own copy of this loop would be a second, quietly different
//! definition of "captured" — which is exactly how an uncaptured project file
//! gets read.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use wasm_bindgen::prelude::*;

/// Most captured members one draft may carry.
const MAX_MEMBERS: usize = 16384;
/// Most captured bytes one draft may carry.
const MAX_BYTES: usize = 512 * 1024 * 1024;

pub(crate) struct CapturedSource {
    /// Every captured member, by its authored path.
    pub assets: BTreeMap<String, Arc<[u8]>>,
    /// The textual members only, for the validators that read source rather
    /// than bytes.
    pub text: BTreeMap<String, String>,
}

/// Read one `{path: string | Uint8Array}` object into exact bytes.
///
/// `long` names the surface in the whole-source messages and `short` in the
/// per-member ones, so each caller keeps the wording it already had.
pub(crate) fn capture(source: JsValue, long: &str, short: &str) -> Result<CapturedSource, String> {
    if !source.is_object() || js_sys::Array::is_array(&source) {
        return Err(format!("Invalid {long} source"));
    }
    let entries = js_sys::Object::entries(&source.unchecked_into());
    if entries.length() as usize > MAX_MEMBERS {
        return Err(format!("{long} source is too large"));
    }
    let mut assets = BTreeMap::new();
    let mut text = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut length = 0usize;
    for entry in entries.iter() {
        let pair = js_sys::Array::from(&entry);
        let path = pair
            .get(0)
            .as_string()
            .ok_or_else(|| format!("Invalid {short} source path"))?;
        if !(path == "scenarios.toml" || path.starts_with("assets/"))
            || path.contains(['\\', ':'])
            || path.chars().any(char::is_control)
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || !folded.insert(path.to_ascii_lowercase())
        {
            return Err(format!("Invalid {short} source path"));
        }
        let value = pair.get(1);
        let bytes = if let Some(source) = value.as_string() {
            if path.ends_with(".toml") || path.ends_with(".rhai") {
                text.insert(path.clone(), source.clone());
            }
            source.into_bytes()
        } else if value.is_instance_of::<js_sys::Uint8Array>() {
            let bytes = value.unchecked_into::<js_sys::Uint8Array>();
            if bytes.length() as usize > MAX_BYTES.saturating_sub(length) {
                return Err(format!("{long} source is too large"));
            }
            bytes.to_vec()
        } else {
            return Err(format!("Invalid {short} source bytes"));
        };
        length = length.saturating_add(bytes.len());
        if length > MAX_BYTES {
            return Err(format!("{long} source is too large"));
        }
        assets.insert(path, Arc::from(bytes));
    }
    Ok(CapturedSource { assets, text })
}
