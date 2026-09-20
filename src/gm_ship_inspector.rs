//! Read-only Live Inspector projection for hulls, Stations and Systems.
//!
//! The projection deliberately serializes the effective EntityConfig ship
//! sections, `ShipConfig`, and each live `SystemBlackboard` instead of
//! maintaining a second hand-written copy of
//! those schemas. Every leaf that exists on an active hull therefore receives
//! a descriptor and a reading, including kind-specific System config and newly
//! added blackboard fields. Authored values require recreation; live values are
//! derived. The only actionable rows link to the existing checked owners.

use crate::core::messages::{SystemBlackboard, SystemId};
use crate::entities::config::EntityConfig;
use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};
use crate::ship::components::ActiveStationRatings;
use crate::ship::config::ShipConfig;
use crate::ship::control_source::ControlSourceResolver;
use crate::ship::damage::SystemHull;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShipFieldGroup {
    Hull,
    Station,
    System,
    Power,
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipInspectorField {
    pub id: String,
    pub label: String,
    pub group: ShipFieldGroup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_panel: Option<String>,
    #[serde(flatten)]
    pub descriptor: FieldDescriptor,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShipInspection {
    pub label: String,
    pub destroyed: bool,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShipInspectorProjection {
    pub fields: Vec<ShipInspectorField>,
    pub readings: BTreeMap<String, ShipInspection>,
}

impl ShipInspectorProjection {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.readings.is_empty()
    }
}

fn group_for(path: &str) -> ShipFieldGroup {
    if path.starts_with("station[") {
        ShipFieldGroup::Station
    } else if path.starts_with("system[") {
        ShipFieldGroup::System
    } else if path.starts_with("power_groups[") {
        ShipFieldGroup::Power
    } else if path.starts_with("runtime.") {
        ShipFieldGroup::Runtime
    } else {
        ShipFieldGroup::Hull
    }
}

fn field(
    id: String,
    kind: &str,
    mutability: LiveMutability,
    action_panel: Option<&str>,
) -> ShipInspectorField {
    ShipInspectorField {
        label: "inspector.ship.field".into(),
        group: group_for(&id),
        action_panel: action_panel.map(str::to_owned),
        descriptor: FieldDescriptor {
            kind: kind.into(),
            default_source: None,
            live_mutability: mutability,
            origin: FieldOrigin {
                schema_path: id.clone(),
                document: None,
                line: None,
                layer: None,
            },
            validation: Vec::new(),
        },
        id,
    }
}

fn scalar(value: &Value) -> Option<(String, &'static str)> {
    match value {
        Value::Null => Some(("not-authored".into(), "optional")),
        Value::Bool(v) => Some((v.to_string(), "bool")),
        Value::Number(v) => Some((
            v.to_string(),
            if v.is_i64() || v.is_u64() {
                "integer"
            } else {
                "float"
            },
        )),
        Value::String(v) => Some((v.clone(), "string")),
        _ => None,
    }
}

fn keyed_segment(value: &Value, index: usize) -> String {
    value
        .as_object()
        .and_then(|v| {
            v.get("id")
                .or_else(|| v.get("system_id"))
                .or_else(|| v.get("name"))
        })
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| index.to_string())
}

fn flatten(
    value: &Value,
    path: &str,
    values: &mut BTreeMap<String, String>,
    fields: &mut BTreeMap<String, ShipInspectorField>,
    mutability: LiveMutability,
) {
    if let Some((display, kind)) = scalar(value) {
        values.insert(path.into(), display);
        fields
            .entry(path.into())
            .or_insert_with(|| field(path.into(), kind, mutability, None));
        return;
    }
    match value {
        Value::Array(rows) => {
            if rows.is_empty() {
                values.insert(path.into(), "[]".into());
                fields
                    .entry(path.into())
                    .or_insert_with(|| field(path.into(), "list", mutability, None));
            } else {
                for (index, row) in rows.iter().enumerate() {
                    let next = format!("{path}[{}]", keyed_segment(row, index));
                    flatten(row, &next, values, fields, mutability);
                }
            }
        }
        Value::Object(map) => {
            if map.is_empty() && !path.is_empty() {
                values.insert(path.into(), "{}".into());
                fields
                    .entry(path.into())
                    .or_insert_with(|| field(path.into(), "table", mutability, None));
            } else {
                for (key, child) in map {
                    let next = if path.is_empty() {
                        key.clone()
                    } else if path == "power_groups" {
                        format!("{path}[{key}]")
                    } else {
                        format!("{path}.{key}")
                    };
                    flatten(child, &next, values, fields, mutability);
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn insert(
    values: &mut BTreeMap<String, String>,
    fields: &mut BTreeMap<String, ShipInspectorField>,
    id: String,
    value: impl ToString,
    kind: &str,
    action: Option<&str>,
) {
    values.insert(id.clone(), value.to_string());
    fields.entry(id.clone()).or_insert_with(|| {
        field(
            id,
            kind,
            if action.is_some() {
                LiveMutability::NamedAction
            } else {
                LiveMutability::Derived
            },
            action,
        )
    });
}

fn ensure_authored(
    values: &mut BTreeMap<String, String>,
    fields: &mut BTreeMap<String, ShipInspectorField>,
    id: String,
    kind: &str,
) {
    values
        .entry(id.clone())
        .or_insert_with(|| "not-authored".into());
    fields
        .entry(id.clone())
        .or_insert_with(|| field(id, kind, LiveMutability::RecreateRequired, None));
}

fn ensure_known_ship_fields(
    config: &ShipConfig,
    values: &mut BTreeMap<String, String>,
    fields: &mut BTreeMap<String, ShipInspectorField>,
) {
    for station in &config.stations {
        let prefix = format!("station[{}]", station.id.0);
        for (name, kind) in [
            ("manual_overview", "optional"),
            ("tutorial", "list"),
            ("host_order", "list"),
            ("visiting_rating", "optional"),
            ("command_target", "optional"),
            ("stance", "list"),
        ] {
            ensure_authored(values, fields, format!("{prefix}.{name}"), kind);
        }
        for rating in &station.ratings {
            ensure_authored(
                values,
                fields,
                format!("{prefix}.rating[{}].ai_tuning", rating.name),
                "optional",
            );
        }
    }
    for system in &config.systems {
        let prefix = format!("system[{}]", system.id.0);
        for (name, kind) in [
            ("station", "optional"),
            ("seek_order", "list"),
            ("power_group", "optional"),
            ("marker", "optional"),
            ("config", "optional"),
        ] {
            ensure_authored(values, fields, format!("{prefix}.{name}"), kind);
        }
    }
}

/// Build the complete active-hull inventory and its current readings.
pub fn projection<'a>(
    ships: impl IntoIterator<Item = ShipInspectorInputs<'a>>,
) -> ShipInspectorProjection {
    let mut fields = BTreeMap::new();
    let mut readings = BTreeMap::new();
    for ship in ships {
        let mut values = BTreeMap::new();
        if let Some(authored) = ship.authored {
            let document = serde_json::to_value(authored).expect("EntityConfig serializes");
            if let Value::Object(sections) = document {
                // The domain boundary is explicit: presentation/AI/world entity
                // sections are inspected by their own panels. These are every
                // known hull and ship-operation section on EntityConfig.
                for name in [
                    "hull",
                    "helm_console",
                    "helm_capability",
                    "weapons_console",
                    "engineering_console",
                    "captain_console",
                    "comms_console",
                    "power",
                    "sensors_console",
                    "navigation_console",
                    "shields_console",
                    "torpedoes",
                    "repair",
                    "scan",
                    "tractor",
                    "dock",
                    "umbilical",
                    "security",
                    "transporter",
                    "shield_arc",
                ] {
                    if let Some(section) = sections.get(name) {
                        flatten(
                            section,
                            name,
                            &mut values,
                            &mut fields,
                            LiveMutability::RecreateRequired,
                        );
                    }
                }
            }
            // EntityConfig intentionally skips these parsed authoring blocks
            // during ordinary serde serialization. They are still first-class
            // hull schema and must stay inside the Live coverage ratchet.
            flatten(
                &serde_json::to_value(&authored.shield_arcs).expect("shield arcs serialize"),
                "shield_arc",
                &mut values,
                &mut fields,
                LiveMutability::RecreateRequired,
            );
            for arc in &authored.shield_arcs {
                let prefix = format!("shield_arc[{}]", arc.id);
                for (name, kind) in [
                    ("max_hp", "optional"),
                    ("regen_per_sec", "optional"),
                    ("offline_duration", "optional"),
                ] {
                    ensure_authored(&mut values, &mut fields, format!("{prefix}.{name}"), kind);
                }
            }
        }
        if let Some(document) = ship.authored_document {
            insert(
                &mut values,
                &mut fields,
                "provenance.document".into(),
                document,
                "path",
                None,
            );
        }
        flatten(
            &serde_json::to_value(ship.config).expect("ShipConfig serializes"),
            "",
            &mut values,
            &mut fields,
            LiveMutability::RecreateRequired,
        );
        ensure_known_ship_fields(ship.config, &mut values, &mut fields);
        insert(
            &mut values,
            &mut fields,
            "runtime.hull.current_hp".into(),
            ship.hull.total_current(),
            "float",
            None,
        );
        insert(
            &mut values,
            &mut fields,
            "runtime.hull.max_hp".into(),
            ship.hull.total_max(),
            "float",
            None,
        );
        insert(
            &mut values,
            &mut fields,
            "runtime.hull.destroyed".into(),
            ship.hull.is_destroyed(),
            "bool",
            None,
        );
        insert(
            &mut values,
            &mut fields,
            "runtime.hull.effect_action".into(),
            "open",
            "action",
            Some("effect"),
        );
        for station in &ship.config.stations {
            let sid = &station.id.0;
            if let Some(rating) = ship.ratings.0.get(&station.id) {
                insert(
                    &mut values,
                    &mut fields,
                    format!("runtime.station[{sid}].active_rating"),
                    rating,
                    "string",
                    None,
                );
            }
            insert(
                &mut values,
                &mut fields,
                format!("runtime.station[{sid}].puppet"),
                "open",
                "action",
                Some("station"),
            );
        }
        for system in &ship.config.systems {
            let id = &system.id;
            let prefix = format!("runtime.system[{}]", id.0);
            insert(
                &mut values,
                &mut fields,
                format!("{prefix}.control_source"),
                format!("{:?}", ship.controls.source_for(id)).to_lowercase(),
                "string",
                None,
            );
            insert(
                &mut values,
                &mut fields,
                format!("{prefix}.gm_disabled"),
                ship.controls.is_gm_disabled(id),
                "bool",
                None,
            );
            insert(
                &mut values,
                &mut fields,
                format!("{prefix}.available"),
                !ship.controls.is_offline(id),
                "bool",
                None,
            );
            insert(
                &mut values,
                &mut fields,
                format!("{prefix}.availability_action"),
                "open",
                "action",
                Some("system"),
            );
            if let Some(entry) = ship.hull.get(id) {
                insert(
                    &mut values,
                    &mut fields,
                    format!("{prefix}.health.current_hp"),
                    entry.current,
                    "float",
                    None,
                );
                insert(
                    &mut values,
                    &mut fields,
                    format!("{prefix}.health.max_hp"),
                    entry.max,
                    "float",
                    None,
                );
                insert(
                    &mut values,
                    &mut fields,
                    format!("{prefix}.health.tier"),
                    format!("{:?}", ship.hull.tier_for(id)).to_lowercase(),
                    "string",
                    None,
                );
                insert(
                    &mut values,
                    &mut fields,
                    format!("{prefix}.effect_action"),
                    "open",
                    "action",
                    Some("effect"),
                );
            }
            if let Some(blackboard) = ship.blackboards.and_then(|rows| rows.get(id)) {
                flatten(
                    &serde_json::to_value(blackboard).expect("SystemBlackboard serializes"),
                    &format!("{prefix}.state"),
                    &mut values,
                    &mut fields,
                    LiveMutability::Derived,
                );
            }
        }
        // Reserved derived channels (dossiers, scan and demolition) do not
        // necessarily appear in `ShipConfig.systems`, but are still live ship
        // state and must not disappear from the inspector.
        if let Some(blackboards) = ship.blackboards {
            for (id, blackboard) in blackboards {
                if ship.config.systems.iter().any(|system| &system.id == id) {
                    continue;
                }
                flatten(
                    &serde_json::to_value(blackboard).expect("SystemBlackboard serializes"),
                    &format!("runtime.channel[{}].state", id.0),
                    &mut values,
                    &mut fields,
                    LiveMutability::Derived,
                );
            }
        }
        readings.insert(
            ship.id.to_owned(),
            ShipInspection {
                label: ship.label.to_owned(),
                destroyed: ship.hull.is_destroyed(),
                values,
            },
        );
    }
    ShipInspectorProjection {
        fields: fields.into_values().collect(),
        readings,
    }
}

pub struct ShipInspectorInputs<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub config: &'a ShipConfig,
    pub authored: Option<&'a EntityConfig>,
    pub authored_document: Option<&'a str>,
    pub ratings: &'a ActiveStationRatings,
    pub controls: &'a ControlSourceResolver,
    pub hull: &'a SystemHull,
    pub blackboards: Option<&'a HashMap<SystemId, SystemBlackboard>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{PowerGroupId, StationId};
    use crate::ship::config::{
        PowerGroupConfig, StationConfig, StationRatingConfig, SystemInstanceConfig,
    };

    #[test]
    fn active_schema_and_runtime_rows_are_classified_without_generic_mutation() {
        let station = StationConfig {
            id: StationId("helm".into()),
            name: "Helm".into(),
            description: "Fly".into(),
            rank: "Officer".into(),
            short_code: "H".into(),
            ratings: vec![StationRatingConfig {
                name: "Full".into(),
                automated_systems: vec![],
                ai_tuning: None,
            }],
            console: None,
            manual_overview: None,
            tutorials: vec![],
            human_seeking: false,
            host_order: vec![],
            visiting_rating: None,
            auxiliary: false,
            command_target: None,
            stances: vec![],
        };
        let system = SystemInstanceConfig {
            id: SystemId("engine".into()),
            kind: "helm-engine".into(),
            station: Some(station.id.clone()),
            ai_only: false,
            human_seeking: false,
            seek_order: vec![],
            power_group: Some(PowerGroupId("engines".into())),
            marker: None,
            config: Some(toml::Value::Table(toml::Table::from_iter([(
                "cooldown_secs".into(),
                toml::Value::Float(3.0),
            )]))),
        };
        let config = ShipConfig {
            stations: vec![station],
            systems: vec![system],
            power_groups: HashMap::from([(
                PowerGroupId("engines".into()),
                PowerGroupConfig {
                    label: "Engines".into(),
                    default_level: 2,
                    min_level: 1,
                    max_level: 4,
                },
            )]),
            coordination_lag_secs: 2.0,
        };
        let ratings =
            ActiveStationRatings(HashMap::from([(StationId("helm".into()), "Full".into())]));
        let controls = ControlSourceResolver::default();
        let hull = SystemHull::from_config(&[(SystemId("engine".into()), 100.0)]);
        let authored = EntityConfig {
            hull: Some(crate::entities::config::HullConfig {
                hull_integrity: 0.0,
                system_hull: vec![crate::entities::config::SystemHullEntry {
                    system_id: SystemId("engine".into()),
                    display_name: Some("Engine".into()),
                    max_hp: 100.0,
                    damaged_threshold_pct: 0.75,
                    disabled_threshold_pct: 0.25,
                    debuff_magnitude: 0.15,
                }],
            }),
            repair: Some(crate::entities::config::RepairConfig {
                repair_team_count: 2,
                ..Default::default()
            }),
            captain_console: Some(Default::default()),
            comms_console: Some(Default::default()),
            shield_arcs: vec![crate::entities::config::ShieldArcConfig {
                id: "fore".into(),
                label: "Fore".into(),
                center_deg: 0.0,
                width_deg: 180.0,
                max_hp: None,
                regen_per_sec: None,
                offline_duration: None,
                hull_max_hp: 50.0,
                hull_damaged_threshold_pct: 0.75,
                hull_disabled_threshold_pct: 0.25,
                hull_debuff_magnitude: 0.15,
                priority: 1,
                frequency: 0.5,
            }],
            ..Default::default()
        };
        let projection = projection([ShipInspectorInputs {
            id: "ship",
            label: "Ship",
            config: &config,
            authored: Some(&authored),
            authored_document: Some("assets/entities/ship.toml"),
            ratings: &ratings,
            controls: &controls,
            hull: &hull,
            blackboards: None,
        }]);
        let paths: BTreeMap<_, _> = projection
            .fields
            .iter()
            .map(|f| {
                (
                    f.id.as_str(),
                    (&f.descriptor.live_mutability, f.action_panel.as_deref()),
                )
            })
            .collect();
        assert_eq!(
            paths["system[engine].config.cooldown_secs"].0,
            &LiveMutability::RecreateRequired
        );
        assert_eq!(
            paths["runtime.system[engine].availability_action"].1,
            Some("system")
        );
        assert_eq!(
            paths["runtime.system[engine].health.current_hp"].0,
            &LiveMutability::Derived
        );
        assert_eq!(
            paths["runtime.system[engine].effect_action"].1,
            Some("effect")
        );
        assert!(projection.readings["ship"]
            .values
            .contains_key("station[helm].rating[Full].name"));
        assert_eq!(
            projection.readings["ship"].values["hull.system_hull[engine].max_hp"],
            "100.0"
        );
        assert_eq!(
            projection.readings["ship"].values["repair.repair_team_count"],
            "2"
        );
        assert_eq!(
            projection.readings["ship"].values["captain_console.ai"],
            "not-authored"
        );
        assert_eq!(
            projection.readings["ship"].values["comms_console.selector"],
            "not-authored"
        );
        assert_eq!(
            projection.readings["ship"].values["shield_arc[fore].max_hp"],
            "not-authored"
        );
        assert_eq!(
            paths["station[helm].manual_overview"].0,
            &LiveMutability::RecreateRequired
        );
        assert_eq!(
            paths["system[engine].marker"].0,
            &LiveMutability::RecreateRequired
        );
        assert_eq!(
            projection.readings["ship"].values["provenance.document"],
            "assets/entities/ship.toml"
        );
    }
}
