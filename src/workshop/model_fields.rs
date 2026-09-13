//! Scalar metadata for rig sidecars, drawn from the runtime field types.
//! This only describes existing source spans. The ordinary ModelRig parser
//! still owns document validation; no model is reconstructed or serialized.
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::document::{ScalarField, Segment};
use crate::entities::config::{LodCapture, LodGeneration, LodLevel, MeshShape, TierRig};
use crate::entities::model_rig::{BaseBuild, BaseTransform, Extents, Marker, TargetPoint};

struct Scalar {
    kind: &'static str,
    default: Option<String>,
    accepts: fn(&str) -> bool,
}

#[derive(Deserialize)]
struct ScalarInput<T> {
    value: T,
}

fn accepts<T: DeserializeOwned>(source: &str) -> bool {
    // Use the same serde implementation as the runtime field, including its
    // unsigned integer range and enum spelling. Other malformed document
    // fields do not prevent the author repairing this one scalar.
    toml::from_str::<ScalarInput<T>>(&format!("value = {source}"))
        .map(|input| input.value)
        .is_ok()
}

fn scalar<T: ScalarField + DeserializeOwned>(default: Option<String>) -> Scalar {
    Scalar {
        kind: T::KIND,
        default,
        accepts: accepts::<T>,
    }
}

fn defaulted<T: ScalarField + DeserializeOwned>(value: &T) -> Scalar {
    scalar::<T>(value.default_source())
}

/// A field accessor ties the descriptor to its real Rust field without
/// constructing an invented default instance of a required table.
fn required<S, T: ScalarField + DeserializeOwned>(_: fn(&S) -> &T) -> Scalar {
    scalar::<T>(None)
}

trait Components {
    type Scalar: ScalarField + DeserializeOwned;
    fn contains(index: usize) -> bool;
    fn default_component(&self, index: usize) -> Option<String>;
}

impl<T: ScalarField + DeserializeOwned, const N: usize> Components for [T; N] {
    type Scalar = T;
    fn contains(index: usize) -> bool {
        index < N
    }
    fn default_component(&self, index: usize) -> Option<String> {
        self.get(index).and_then(ScalarField::default_source)
    }
}

impl<T: ScalarField + DeserializeOwned> Components for Vec<T> {
    type Scalar = T;
    fn contains(_: usize) -> bool {
        true
    }
    fn default_component(&self, index: usize) -> Option<String> {
        self.get(index).and_then(ScalarField::default_source)
    }
}

impl<T: Components> Components for Option<T> {
    type Scalar = T::Scalar;
    fn contains(index: usize) -> bool {
        T::contains(index)
    }
    fn default_component(&self, index: usize) -> Option<String> {
        self.as_ref()
            .and_then(|value| value.default_component(index))
    }
}

fn component<T: Components>(value: &T, index: usize) -> Option<Scalar> {
    T::contains(index).then(|| scalar::<T::Scalar>(value.default_component(index)))
}

fn required_component<S, T: Components>(_: fn(&S) -> &T, index: usize) -> Option<Scalar> {
    T::contains(index).then(|| scalar::<T::Scalar>(None))
}

macro_rules! enum_field {
    ($ty:ty) => {
        impl ScalarField for $ty {
            const KIND: &'static str = "string";
            fn default_source(&self) -> Option<String> {
                // Encoded names come from the runtime enum's serde contract.
                Some(toml::Value::try_from(self).ok()?.to_string())
            }
        }
    };
}
enum_field!(MeshShape);
enum_field!(TierRig);

fn is_sidecar(path: &str) -> bool {
    path.starts_with("assets/models/")
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && !part.ends_with(['.', ' ']))
        && crate::entities::model_rig::sidecar_variant(path).is_some()
}

fn lookup(document_path: &str, path: &[Segment]) -> Option<Scalar> {
    use Segment::{Index, Key};
    if !is_sidecar(document_path) {
        return None;
    }
    match path {
        [Key(table), Key(key), Index(axis)] if table == "base" => {
            let base = BaseTransform::default();
            match key.as_str() {
                "offset" => component(&base.offset, *axis),
                "rotation" => component(&base.rotation, *axis),
                "scale" => component(&base.scale, *axis),
                _ => None,
            }
        }
        [Key(table), Key(key), Index(axis)] if table == "extents" => match key.as_str() {
            "min" => required_component(|value: &Extents| &value.min, *axis),
            "max" => required_component(|value: &Extents| &value.max, *axis),
            "size" => required_component(|value: &Extents| &value.size, *axis),
            _ => None,
        },
        [Key(table), Key(_name), Key(key), Index(axis)] if table == "markers" => {
            match key.as_str() {
                "position" => required_component(|value: &Marker| &value.position, *axis),
                "direction" => required_component(|value: &Marker| &value.direction, *axis),
                _ => None,
            }
        }
        [Key(table), Index(_point), Key(key), Index(axis)]
            if table == "target_points" && key == "position" =>
        {
            required_component(|value: &TargetPoint| &value.position, *axis)
        }
        [Key(table), Key(key)] if table == "base_build" && key == "budget_mb" => {
            Some(required(|value: &BaseBuild| &value.budget_mb))
        }
        [Key(table), Index(_level), Key(key)] if table == "lod" => {
            let level = LodLevel::default();
            Some(match key.as_str() {
                "max_distance" => defaulted(&level.max_distance),
                "model" => defaulted(&level.model),
                "variant" => defaulted(&level.variant),
                "billboard" => defaulted(&level.billboard),
                "shape" => defaulted(&level.shape),
                "radius" => defaulted(&level.radius),
                "minor_radius" => defaulted(&level.minor_radius),
                "emissive" => defaulted(&level.emissive),
                "tier_rig" => defaulted(&level.tier_rig),
                _ => return None,
            })
        }
        [Key(table), Index(_level), Key(key), Index(axis)] if table == "lod" => {
            let level = LodLevel::default();
            match key.as_str() {
                "colour" => component(&level.colour, *axis),
                "size" => component(&level.size, *axis),
                "rotation" => component(&level.rotation, *axis),
                "scale" => component(&level.scale, *axis),
                _ => None,
            }
        }
        [Key(table), Index(_level), Key(section), Key(key)] if table == "lod" => {
            Some(match section.as_str() {
                "generate" => {
                    let generation = LodGeneration::default();
                    match key.as_str() {
                        "source" => defaulted(&generation.source),
                        "ratio" => defaulted(&generation.ratio),
                        "error" => defaulted(&generation.error),
                        "texture_size" => defaulted(&generation.texture_size),
                        "remesh_voxel_size" => defaulted(&generation.remesh_voxel_size),
                        _ => return None,
                    }
                }
                "capture" => {
                    let capture = LodCapture::default();
                    match key.as_str() {
                        "source" => defaulted(&capture.source),
                        "yaw_views" => defaulted(&capture.yaw_views),
                        "resolution" => defaulted(&capture.resolution),
                        "pitch" => defaulted(&capture.pitch),
                        _ => return None,
                    }
                }
                _ => return None,
            })
        }
        _ => None,
    }
}

pub(super) fn descriptor(
    document_path: &str,
    path: &[Segment],
) -> Option<(&'static str, Option<String>)> {
    lookup(document_path, path).map(|field| (field.kind, field.default))
}

pub(super) fn validate(document_path: &str, path: &[Segment], source: &str) -> Result<(), String> {
    if lookup(document_path, path).is_some_and(|field| !(field.accepts)(source)) {
        return Err("The value is not accepted by this runtime field type.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
