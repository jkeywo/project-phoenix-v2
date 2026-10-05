pub use phoenix_model::entity::visual::*;

use super::*;
pub(super) fn reject_relocated_mesh_lod(value: &toml::Value) -> Result<(), toml::de::Error> {
    let Some(mesh) = value.get("mesh") else {
        return Ok(());
    };
    if mesh.get("lod").is_none() {
        return Ok(());
    }
    let sidecar = mesh
        .get("model")
        .and_then(|m| m.as_str())
        .map(|model| {
            phoenix_content::rig::sidecar_path(model, mesh.get("variant").and_then(|v| v.as_str()))
        })
        .unwrap_or_else(|| "assets/models/<model>.<variant>.toml".to_string());
    Err(SerdeError::custom(format!(
        "[[mesh.lod]] has moved to the model rig sidecar (issue #914): author the \
         chain as [[lod]] blocks in {sidecar} and delete it from the entity TOML. \
         The entity's [mesh] keeps the model reference and the flat fallback fields \
         that sidecar levels fall back to."
    )))
}
