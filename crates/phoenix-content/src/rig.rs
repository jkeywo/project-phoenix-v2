//! Rig sidecar preparation, without ECS or rendering.
pub use phoenix_model::rig::*;
pub fn parse_model_rig(text: &str) -> Result<ModelRig, toml::de::Error> {
    toml::from_str(text)
}

/// The reserved default variant name used when an entity's `[mesh]` does not
/// specify a `variant`.
pub const DEFAULT_VARIANT: &str = "model";

/// Pure path helper: produce the sidecar path for a model.
///
/// `assets/models/<stem>.<variant-or-"model">.toml`. The model path's
/// directory and `assets/` prefix are preserved; only the final `.glb`
/// extension is replaced with `.<variant>.toml`. A `variant` of `Some("model")`
/// is treated the same as the default.
///
/// # Examples
/// * `("assets/models/dynasty_destroyer.glb", None)`
///   → `assets/models/dynasty_destroyer.model.toml`
/// * `("assets/models/dynasty_destroyer.glb", Some("weathered"))`
///   → `assets/models/dynasty_destroyer.weathered.toml`
pub fn sidecar_path(model_path: &str, variant: Option<&str>) -> String {
    let variant = variant.unwrap_or(DEFAULT_VARIANT);
    // Strip a trailing ".glb" (case-insensitive) so the stem is clean; if the
    // path has some other / no extension, just append.
    let stem = match model_path
        .to_ascii_lowercase()
        .strip_suffix(".glb")
        .map(|_| &model_path[..model_path.len() - 4])
    {
        Some(s) => s,
        None => model_path,
    };
    format!("{stem}.{variant}.toml")
}

/// Pure path helper: the inverse of [`sidecar_path`] — which variant a sidecar
/// path names.
///
/// `assets/models/asteroid_common_1.large.toml` → `Some("large")`;
/// `assets/models/dynasty_destroyer.model.toml` → `Some("model")`. `None` when
/// the path is not a `<stem>.<variant>.toml` sidecar at all.
///
/// Used when a sidecar's own `[[lod]]` level omits `variant`: the level then
/// inherits the variant of the sidecar it was declared in, which is exactly the
/// variant the entity's `[mesh]` used to reach that sidecar — so the preload
/// walk and the renderer's `MeshConfig::variant` fallback agree by construction.
pub fn sidecar_variant(sidecar: &str) -> Option<&str> {
    let file = sidecar.rsplit(['/', '\\']).next()?;
    let stem = file.strip_suffix(".toml")?;
    let (base, variant) = stem.rsplit_once('.')?;
    if base.is_empty() || variant.is_empty() {
        return None;
    }
    Some(variant)
}
