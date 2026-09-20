//! Read-only Live Inspector projection for active Regions.
//!
//! Region shape, transform, effects and presentation are authored topology:
//! changing any of them requires recreating the simulation. Runtime occupancy
//! is derived from `RegionMembership`, and consequences are derived directly
//! from the Region's public effect components. The projection deliberately
//! never reads a ship's modifier cache or crew/session state.

use crate::entities::spawner::{
    EntityId, EntityName, EntityTagsSection, EntityTemplatePath, EntityUuid,
    RadarAppearanceSection, RegionEffectsSection, RegionShapeSection,
};
use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};
use crate::regions::effects::RegionEffectKind;
use crate::world::server::EntityOriginLayer;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegionFieldGroup {
    Identity,
    Transform,
    Shape,
    Effect,
    Presentation,
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionInspectorField {
    pub id: String,
    pub label: String,
    pub group: RegionFieldGroup,
    #[serde(flatten)]
    pub descriptor: FieldDescriptor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionConsequence {
    pub kind: String,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionOccupant {
    pub entity_id: String,
    pub label: String,
    pub consequences: Vec<RegionConsequence>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionInspection {
    pub label: String,
    pub values: BTreeMap<String, String>,
    pub occupants: Vec<RegionOccupant>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionInspectorProjection {
    pub fields: Vec<RegionInspectorField>,
    pub readings: BTreeMap<String, RegionInspection>,
}

impl RegionInspectorProjection {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.readings.is_empty()
    }
}

fn group(path: &str) -> RegionFieldGroup {
    if path.starts_with("identity.") || path.starts_with("provenance.") {
        RegionFieldGroup::Identity
    } else if path.starts_with("transform.") {
        RegionFieldGroup::Transform
    } else if path.starts_with("shape.") {
        RegionFieldGroup::Shape
    } else if path.starts_with("effects.") {
        RegionFieldGroup::Effect
    } else if path.starts_with("presentation.") {
        RegionFieldGroup::Presentation
    } else {
        RegionFieldGroup::Runtime
    }
}

fn descriptor(path: &str, kind: &str, mutability: LiveMutability) -> RegionInspectorField {
    RegionInspectorField {
        id: path.into(),
        label: "inspector.region.field".into(),
        group: group(path),
        descriptor: FieldDescriptor {
            kind: kind.into(),
            default_source: None,
            live_mutability: mutability,
            origin: FieldOrigin {
                schema_path: path.into(),
                document: None,
                line: None,
                layer: None,
            },
            validation: if mutability == LiveMutability::RecreateRequired {
                vec!["inspector.region.recreate_explanation".into()]
            } else {
                Vec::new()
            },
        },
    }
}

/// Complete Region field inventory. Adding a known shape/effect/presentation
/// field requires extending this explicit ratchet and its test.
pub fn fields() -> Vec<RegionInspectorField> {
    use LiveMutability::{Derived, RecreateRequired};
    [
        ("identity.uuid", "string", Derived),
        ("identity.kind", "enum", RecreateRequired),
        ("identity.name", "optional", RecreateRequired),
        ("identity.display_name", "optional", RecreateRequired),
        ("identity.id", "optional", RecreateRequired),
        ("identity.tags", "list", RecreateRequired),
        ("provenance.document", "optional", Derived),
        ("provenance.layer", "optional", Derived),
        ("transform.translation.x", "float", RecreateRequired),
        ("transform.translation.y", "float", RecreateRequired),
        ("transform.translation.z", "float", RecreateRequired),
        ("transform.rotation.x", "float", RecreateRequired),
        ("transform.rotation.y", "float", RecreateRequired),
        ("transform.rotation.z", "float", RecreateRequired),
        ("transform.rotation.w", "float", RecreateRequired),
        ("transform.scale.x", "float", RecreateRequired),
        ("transform.scale.y", "float", RecreateRequired),
        ("transform.scale.z", "float", RecreateRequired),
        ("shape.type", "enum", RecreateRequired),
        ("shape.sphere.radius", "float", RecreateRequired),
        ("shape.box.half_extents.x", "float", RecreateRequired),
        ("shape.box.half_extents.y", "float", RecreateRequired),
        ("shape.box.half_extents.z", "float", RecreateRequired),
        ("shape.box.yaw", "float", RecreateRequired),
        ("shape.torus.inner_radius", "float", RecreateRequired),
        ("shape.torus.outer_radius", "float", RecreateRequired),
        ("effects.damage_zone.present", "bool", RecreateRequired),
        (
            "effects.damage_zone.damage_per_second",
            "float",
            RecreateRequired,
        ),
        (
            "effects.damage_zone.shield_pierce",
            "float",
            RecreateRequired,
        ),
        ("effects.slow_zone.present", "bool", RecreateRequired),
        (
            "effects.slow_zone.thrust_modifier",
            "optional",
            RecreateRequired,
        ),
        (
            "effects.slow_zone.yaw_rate_modifier",
            "optional",
            RecreateRequired,
        ),
        ("effects.blocks_impulse.present", "bool", RecreateRequired),
        ("effects.radar_dampening.present", "bool", RecreateRequired),
        (
            "effects.radar_dampening.range_modifier",
            "float",
            RecreateRequired,
        ),
        ("effects.comms_jammed.present", "bool", RecreateRequired),
        ("effects.sensor_blind.present", "bool", RecreateRequired),
        ("effects.nebula_fog.present", "bool", RecreateRequired),
        ("effects.nebula_fog.color.r", "float", RecreateRequired),
        ("effects.nebula_fog.color.g", "float", RecreateRequired),
        ("effects.nebula_fog.color.b", "float", RecreateRequired),
        ("effects.nebula_fog.density", "float", RecreateRequired),
        ("presentation.radar.icon", "optional", RecreateRequired),
        ("presentation.radar.colour.r", "optional", RecreateRequired),
        ("presentation.radar.colour.g", "optional", RecreateRequired),
        ("presentation.radar.colour.b", "optional", RecreateRequired),
        ("presentation.radar.size", "optional", RecreateRequired),
        (
            "presentation.radar.region_colour.r",
            "optional",
            RecreateRequired,
        ),
        (
            "presentation.radar.region_colour.g",
            "optional",
            RecreateRequired,
        ),
        (
            "presentation.radar.region_colour.b",
            "optional",
            RecreateRequired,
        ),
        ("runtime.occupant_count", "integer", Derived),
    ]
    .into_iter()
    .map(|(path, kind, mutability)| descriptor(path, kind, mutability))
    .collect()
}

fn put(values: &mut BTreeMap<String, String>, path: &str, value: impl ToString) {
    values.insert(path.into(), value.to_string());
}

fn put_optional(values: &mut BTreeMap<String, String>, path: &str, value: Option<impl ToString>) {
    put(
        values,
        path,
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "not-authored".into()),
    );
}

fn modifier_multiplier(bonus: f32) -> f32 {
    if bonus >= 0.0 {
        1.0 + bonus
    } else {
        1.0 / (1.0 + bonus.abs())
    }
}

fn consequences(effects: &[RegionEffectKind]) -> Vec<RegionConsequence> {
    effects
        .iter()
        .map(|effect| {
            let mut values = BTreeMap::new();
            let kind = match effect {
                RegionEffectKind::DamageZone { dps, shield_pierce } => {
                    put(&mut values, "damage_per_second", dps);
                    put(&mut values, "shield_pierce", shield_pierce);
                    "damage-zone"
                }
                RegionEffectKind::SlowZone {
                    thrust_modifier,
                    yaw_rate_modifier,
                } => {
                    put_optional(
                        &mut values,
                        "thrust_multiplier",
                        thrust_modifier.map(modifier_multiplier),
                    );
                    put_optional(
                        &mut values,
                        "yaw_rate_multiplier",
                        yaw_rate_modifier.map(modifier_multiplier),
                    );
                    "slow-zone"
                }
                RegionEffectKind::BlocksImpulse => "blocks-impulse",
                RegionEffectKind::RadarDampening { multiplier } => {
                    put(
                        &mut values,
                        "radar_range_multiplier",
                        modifier_multiplier(*multiplier),
                    );
                    "radar-dampening"
                }
                RegionEffectKind::CommsJam => "comms-jam",
                RegionEffectKind::SensorBlind => "sensor-blind",
                RegionEffectKind::NebulaFog { color, density } => {
                    put(&mut values, "color.r", color[0]);
                    put(&mut values, "color.g", color[1]);
                    put(&mut values, "color.b", color[2]);
                    put(&mut values, "density", density);
                    "nebula-fog"
                }
            };
            RegionConsequence {
                kind: kind.into(),
                values,
            }
        })
        .collect()
}

pub struct RegionReadingInputs<'a> {
    pub uuid: &'a EntityUuid,
    pub name: Option<&'a EntityName>,
    pub display_name: Option<&'a str>,
    pub id: Option<&'a EntityId>,
    pub template: Option<&'a EntityTemplatePath>,
    pub layer: Option<&'a EntityOriginLayer>,
    pub tags: Option<&'a EntityTagsSection>,
    pub transform: &'a Transform,
    pub shape: &'a RegionShapeSection,
    pub effects: Option<&'a RegionEffectsSection>,
    pub radar: Option<&'a RadarAppearanceSection>,
    pub occupants: Vec<(&'a EntityUuid, String)>,
}

pub fn reading(input: RegionReadingInputs<'_>) -> RegionInspection {
    let mut values = fields()
        .into_iter()
        .map(|field| (field.id, "not-applicable".into()))
        .collect::<BTreeMap<_, _>>();
    put(&mut values, "identity.uuid", &input.uuid.0);
    put(
        &mut values,
        "identity.kind",
        if input.effects.is_some_and(|effects| !effects.0.is_empty()) {
            "hazard"
        } else {
            "region"
        },
    );
    put_optional(
        &mut values,
        "identity.name",
        input.name.map(|value| &value.0),
    );
    put_optional(&mut values, "identity.display_name", input.display_name);
    put_optional(&mut values, "identity.id", input.id.map(|value| &value.0));
    put(
        &mut values,
        "identity.tags",
        input.tags.map(|tags| tags.0.join(", ")).unwrap_or_default(),
    );
    put_optional(
        &mut values,
        "provenance.document",
        input.template.map(|value| &value.0),
    );
    put_optional(
        &mut values,
        "provenance.layer",
        input.layer.map(|value| &value.0),
    );
    for (axis, value) in ["x", "y", "z"]
        .into_iter()
        .zip(input.transform.translation.to_array())
    {
        put(&mut values, &format!("transform.translation.{axis}"), value);
    }
    for (axis, value) in ["x", "y", "z", "w"]
        .into_iter()
        .zip(input.transform.rotation.to_array())
    {
        put(&mut values, &format!("transform.rotation.{axis}"), value);
    }
    for (axis, value) in ["x", "y", "z"]
        .into_iter()
        .zip(input.transform.scale.to_array())
    {
        put(&mut values, &format!("transform.scale.{axis}"), value);
    }
    match &input.shape.0 {
        crate::regions::shape::RegionShape::Sphere { radius } => {
            put(&mut values, "shape.type", "sphere");
            put(&mut values, "shape.sphere.radius", radius);
        }
        crate::regions::shape::RegionShape::Box { half_extents, yaw } => {
            put(&mut values, "shape.type", "box");
            for (axis, value) in ["x", "y", "z"].into_iter().zip(half_extents) {
                put(
                    &mut values,
                    &format!("shape.box.half_extents.{axis}"),
                    value,
                );
            }
            put(&mut values, "shape.box.yaw", yaw);
        }
        crate::regions::shape::RegionShape::Torus {
            inner_radius,
            outer_radius,
        } => {
            put(&mut values, "shape.type", "torus");
            put(&mut values, "shape.torus.inner_radius", inner_radius);
            put(&mut values, "shape.torus.outer_radius", outer_radius);
        }
    }
    for path in [
        "effects.damage_zone.present",
        "effects.slow_zone.present",
        "effects.blocks_impulse.present",
        "effects.radar_dampening.present",
        "effects.comms_jammed.present",
        "effects.sensor_blind.present",
        "effects.nebula_fog.present",
    ] {
        put(&mut values, path, false);
    }
    for effect in input
        .effects
        .map(|effects| effects.0.as_slice())
        .unwrap_or_default()
    {
        match effect {
            RegionEffectKind::DamageZone { dps, shield_pierce } => {
                put(&mut values, "effects.damage_zone.present", true);
                put(&mut values, "effects.damage_zone.damage_per_second", dps);
                put(
                    &mut values,
                    "effects.damage_zone.shield_pierce",
                    shield_pierce,
                );
            }
            RegionEffectKind::SlowZone {
                thrust_modifier,
                yaw_rate_modifier,
            } => {
                put(&mut values, "effects.slow_zone.present", true);
                put_optional(
                    &mut values,
                    "effects.slow_zone.thrust_modifier",
                    *thrust_modifier,
                );
                put_optional(
                    &mut values,
                    "effects.slow_zone.yaw_rate_modifier",
                    *yaw_rate_modifier,
                );
            }
            RegionEffectKind::BlocksImpulse => {
                put(&mut values, "effects.blocks_impulse.present", true)
            }
            RegionEffectKind::RadarDampening { multiplier } => {
                put(&mut values, "effects.radar_dampening.present", true);
                put(
                    &mut values,
                    "effects.radar_dampening.range_modifier",
                    multiplier,
                );
            }
            RegionEffectKind::CommsJam => put(&mut values, "effects.comms_jammed.present", true),
            RegionEffectKind::SensorBlind => put(&mut values, "effects.sensor_blind.present", true),
            RegionEffectKind::NebulaFog { color, density } => {
                put(&mut values, "effects.nebula_fog.present", true);
                for (axis, value) in ["r", "g", "b"].into_iter().zip(color) {
                    put(
                        &mut values,
                        &format!("effects.nebula_fog.color.{axis}"),
                        value,
                    );
                }
                put(&mut values, "effects.nebula_fog.density", density);
            }
        }
    }
    let radar = input.radar.map(|radar| &radar.0);
    put_optional(
        &mut values,
        "presentation.radar.icon",
        radar.and_then(|radar| radar.icon.as_ref()),
    );
    put_optional(
        &mut values,
        "presentation.radar.size",
        radar.and_then(|radar| radar.size),
    );
    for (name, colour) in [
        ("colour", radar.and_then(|radar| radar.colour.as_ref())),
        (
            "region_colour",
            radar.and_then(|radar| radar.region_colour.as_ref()),
        ),
    ] {
        for (index, axis) in ["r", "g", "b"].into_iter().enumerate() {
            put_optional(
                &mut values,
                &format!("presentation.radar.{name}.{axis}"),
                colour.and_then(|colour| colour.get(index)).copied(),
            );
        }
    }
    put(&mut values, "runtime.occupant_count", input.occupants.len());
    let effect_consequences = consequences(
        input
            .effects
            .map(|effects| effects.0.as_slice())
            .unwrap_or_default(),
    );
    let occupants = input
        .occupants
        .into_iter()
        .map(|(uuid, label)| RegionOccupant {
            entity_id: uuid.0.clone(),
            label,
            consequences: effect_consequences.clone(),
        })
        .collect();
    RegionInspection {
        label: input
            .display_name
            .map(str::to_owned)
            .or_else(|| input.name.map(|name| name.0.clone()))
            .or_else(|| input.id.map(|id| id.0.clone()))
            .unwrap_or_else(|| input.uuid.0.clone()),
        values,
        occupants,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_is_complete_classified_and_occupancy_uses_public_effects_only() {
        let inventory = fields();
        let by_id: BTreeMap<_, _> = inventory
            .iter()
            .map(|field| (field.id.as_str(), field))
            .collect();
        for required in [
            "identity.uuid",
            "identity.kind",
            "transform.rotation.w",
            "shape.sphere.radius",
            "shape.box.yaw",
            "shape.torus.outer_radius",
            "effects.damage_zone.shield_pierce",
            "effects.slow_zone.yaw_rate_modifier",
            "effects.blocks_impulse.present",
            "effects.radar_dampening.range_modifier",
            "effects.comms_jammed.present",
            "effects.sensor_blind.present",
            "effects.nebula_fog.density",
            "presentation.radar.region_colour.b",
            "runtime.occupant_count",
        ] {
            assert!(
                by_id.contains_key(required),
                "missing descriptor {required}"
            );
        }
        assert!(inventory
            .iter()
            .all(|field| field.descriptor.live_mutability != LiveMutability::NamedAction));
        assert!(inventory
            .iter()
            .filter(|field| field.descriptor.live_mutability == LiveMutability::RecreateRequired)
            .all(|field| field.descriptor.validation == ["inspector.region.recreate_explanation"]));

        let effects = RegionEffectsSection(vec![
            RegionEffectKind::SlowZone {
                thrust_modifier: Some(-1.0),
                yaw_rate_modifier: None,
            },
            RegionEffectKind::RadarDampening { multiplier: -1.0 },
            RegionEffectKind::CommsJam,
        ]);
        let region_uuid = EntityUuid("region".into());
        let ship_uuid = EntityUuid("ship".into());
        let shape = RegionShapeSection(crate::regions::shape::RegionShape::Sphere { radius: 10.0 });
        let transform = Transform::from_xyz(1.0, 2.0, 3.0);
        let reading = reading(RegionReadingInputs {
            uuid: &region_uuid,
            name: None,
            display_name: Some("Storm front"),
            id: None,
            template: None,
            layer: None,
            tags: None,
            transform: &transform,
            shape: &shape,
            effects: Some(&effects),
            radar: None,
            occupants: vec![(&ship_uuid, "Scout".into())],
        });
        assert_eq!(reading.values["shape.sphere.radius"], "10");
        assert_eq!(reading.values["identity.display_name"], "Storm front");
        assert_eq!(reading.label, "Storm front");
        assert_eq!(reading.values["shape.box.yaw"], "not-applicable");
        assert_eq!(reading.values["effects.damage_zone.present"], "false");
        assert_eq!(reading.values["effects.slow_zone.present"], "true");
        assert_eq!(reading.occupants[0].entity_id, "ship");
        assert_eq!(
            reading.occupants[0].consequences[0].values["thrust_multiplier"],
            "0.5"
        );
        assert_eq!(
            reading.occupants[0].consequences[1].values["radar_range_multiplier"],
            "0.5"
        );
    }
}
