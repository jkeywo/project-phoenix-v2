//! Read-only Live inspection of the loaded world and scenario state.
//!
//! The projection deliberately owns no mutation route. Named actions are links
//! to the mission, Objective and session panels; Flags and every other value
//! are either derived runtime context or require recreating the simulation.
use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldInspectorField {
    pub id: String,
    pub label: String,
    pub group: String,
    #[serde(flatten)]
    pub descriptor: FieldDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_panel: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldInspection {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_layer: Option<String>,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldInspectorProjection {
    pub fields: Vec<WorldInspectorField>,
    pub readings: BTreeMap<String, WorldInspection>,
}

impl WorldInspectorProjection {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.readings.is_empty()
    }
}

fn field(
    id: &str,
    group: &str,
    kind: &str,
    mutability: LiveMutability,
    panel: Option<&str>,
) -> WorldInspectorField {
    WorldInspectorField {
        id: id.into(),
        label: format!("inspector.world.{}", id.replace(['.', '[', ']'], "_")),
        group: group.into(),
        descriptor: FieldDescriptor {
            kind: kind.into(),
            default_source: None,
            live_mutability: mutability,
            origin: FieldOrigin {
                schema_path: id.into(),
                document: None,
                line: None,
                layer: None,
            },
            validation: Vec::new(),
        },
        action_panel: panel.map(str::to_owned),
    }
}

fn flatten(prefix: &str, value: &toml::Value, out: &mut BTreeMap<String, String>) {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                flatten(&format!("{prefix}{key}."), value, out);
            }
        }
        toml::Value::Array(values) => {
            out.insert(
                prefix.trim_end_matches('.').into(),
                values
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        value => {
            out.insert(
                prefix.trim_end_matches('.').into(),
                value.to_string().trim_matches('"').into(),
            );
        }
    }
}

fn kind(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::Boolean(_) => "bool",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Array(_) => "list",
        toml::Value::Table(_) => "table",
        _ => "string",
    }
}

fn descriptor_tree(prefix: &str, value: &toml::Value, fields: &mut Vec<WorldInspectorField>) {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                descriptor_tree(&format!("{prefix}{key}."), value, fields);
            }
        }
        value => fields.push(field(
            prefix.trim_end_matches('.'),
            "global",
            kind(value),
            LiveMutability::RecreateRequired,
            None,
        )),
    }
}

fn record_key(value: &str) -> String {
    value.replace(['[', ']'], "_")
}

fn put(
    reading: &mut WorldInspection,
    collection: &str,
    key: &str,
    field: &str,
    value: impl ToString,
) {
    reading.values.insert(
        format!("{collection}[{}].{field}", record_key(key)),
        value.to_string(),
    );
}

fn put_optional(
    reading: &mut WorldInspection,
    collection: &str,
    key: &str,
    field: &str,
    value: Option<impl ToString>,
) {
    if let Some(value) = value {
        put(reading, collection, key, field, value);
    }
}

fn definition(reading: &mut WorldInspection, config: &crate::world::config::WorldConfig) {
    if let Ok(global) = toml::Value::try_from(&config.global) {
        flatten("global.", &global, &mut reading.values);
    }
    reading.values.insert(
        "definition.entity_count".into(),
        config.entities.len().to_string(),
    );
    reading.values.insert(
        "definition.anchor_count".into(),
        config.anchors.len().to_string(),
    );
    reading.values.insert(
        "definition.extra_worlds".into(),
        config.extra_worlds.join(", "),
    );
    reading.values.insert(
        "definition.script_units".into(),
        config.script_sources.len().to_string(),
    );
    for deadline in &config.deadlines {
        for (field, value) in [
            ("id", deadline.id.clone()),
            ("label", deadline.label.clone()),
            ("due_secs", deadline.due_secs.to_string()),
            ("visible", deadline.visible.to_string()),
        ] {
            put(reading, "deadline", &deadline.id, field, value);
        }
    }
}

/// Build one descriptor table and one reading for each active authored layer.
/// Lightweight world nodes remain available while the detail tool is closed.
pub fn topology(
    world: Option<&crate::world::config::WorldConfig>,
    layers: Option<&crate::world::server::WorldLayerMap>,
) -> WorldInspectorProjection {
    let mut result = WorldInspectorProjection::default();
    let Some(world) = world else {
        return result;
    };
    result.readings.insert(
        "root".into(),
        WorldInspection {
            label: world.global.title.clone().unwrap_or_else(|| "root".into()),
            origin_layer: Some("root".into()),
            values: BTreeMap::new(),
        },
    );
    if let Some(layers) = layers {
        for (path, layer) in &layers.0 {
            if layer.is_active {
                result.readings.insert(
                    path.clone(),
                    WorldInspection {
                        label: path.clone(),
                        origin_layer: Some(path.clone()),
                        values: BTreeMap::new(),
                    },
                );
            }
        }
    }
    result
}

/// Build one descriptor table and one reading for each active authored layer.
/// The root is always `root`; supporting-layer keys are their retained paths.
pub fn projection(
    world: Option<&crate::world::config::WorldConfig>,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    objectives: Option<&crate::objectives::ObjectiveManager>,
    layers: Option<&crate::world::server::WorldLayerMap>,
    paused: Option<bool>,
) -> WorldInspectorProjection {
    use LiveMutability::{Derived, NamedAction, RecreateRequired};
    let mut fields = Vec::new();
    let mut readings = BTreeMap::new();
    let Some(world) = world else {
        return WorldInspectorProjection::default();
    };
    if let Ok(global) = toml::Value::try_from(&world.global) {
        descriptor_tree("global.", &global, &mut fields);
    }
    // TOML has no null value, so serde omits absent Options. Keep those known
    // schema fields discoverable even when this world did not author them.
    for (id, kind) in [
        ("global.seed", "integer"),
        ("global.title", "string"),
        ("global.description", "string"),
    ] {
        fields.push(field(id, "global", kind, RecreateRequired, None));
    }
    for (id, group, kind, mutability, panel) in [
        (
            "definition.entity_count",
            "definition",
            "integer",
            RecreateRequired,
            None,
        ),
        (
            "definition.anchor_count",
            "definition",
            "integer",
            RecreateRequired,
            None,
        ),
        (
            "definition.extra_worlds",
            "definition",
            "list",
            RecreateRequired,
            None,
        ),
        (
            "definition.script_units",
            "definition",
            "integer",
            RecreateRequired,
            None,
        ),
        ("event[].id", "events", "string", RecreateRequired, None),
        ("event[].label", "events", "string", RecreateRequired, None),
        (
            "event[].condition",
            "events",
            "string",
            RecreateRequired,
            None,
        ),
        ("event[].when", "events", "string", RecreateRequired, None),
        (
            "event[].repeatable",
            "events",
            "bool",
            RecreateRequired,
            None,
        ),
        (
            "event[].cooldown_secs",
            "events",
            "float",
            RecreateRequired,
            None,
        ),
        (
            "event[].attention_band",
            "events",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "event[].fire",
            "events",
            "bool",
            NamedAction,
            Some("mission"),
        ),
        (
            "event[].pause",
            "events",
            "bool",
            NamedAction,
            Some("mission"),
        ),
        (
            "event[].skip",
            "events",
            "bool",
            NamedAction,
            Some("mission"),
        ),
        ("event[].paused", "events", "bool", Derived, None),
        ("event[].spent", "events", "bool", Derived, None),
        ("event[].armed", "events", "bool", Derived, None),
        ("event[].skip_armed", "events", "bool", Derived, None),
        (
            "objective[].id",
            "objectives",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "objective[].status",
            "objectives",
            "string",
            NamedAction,
            Some("objective"),
        ),
        (
            "objective[].mandatory",
            "objectives",
            "bool",
            RecreateRequired,
            None,
        ),
        (
            "objective[].base_priority",
            "objectives",
            "float",
            RecreateRequired,
            None,
        ),
        (
            "objective[].directive",
            "objectives",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "deadline[].id",
            "deadlines",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "deadline[].label",
            "deadlines",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "deadline[].due_secs",
            "deadlines",
            "integer",
            RecreateRequired,
            None,
        ),
        (
            "deadline[].visible",
            "deadlines",
            "bool",
            RecreateRequired,
            None,
        ),
        ("deadline[].due_tick", "deadlines", "integer", Derived, None),
        ("deadline[].state", "deadlines", "string", Derived, None),
        (
            "scenario.schema_version",
            "scenario",
            "integer",
            Derived,
            None,
        ),
        ("flag[].name", "scenario", "string", Derived, None),
        ("flag[].value", "scenario", "integer", Derived, None),
        ("trigger[].id", "scenario", "string", RecreateRequired, None),
        (
            "trigger[].condition",
            "scenario",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "trigger[].when",
            "scenario",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "trigger[].repeat",
            "scenario",
            "bool",
            RecreateRequired,
            None,
        ),
        (
            "trigger[].cooldown_secs",
            "scenario",
            "float",
            RecreateRequired,
            None,
        ),
        ("trigger[].fired", "scenario", "bool", Derived, None),
        ("trigger[].pending", "scenario", "bool", Derived, None),
        ("trigger[].when_holds", "scenario", "bool", Derived, None),
        (
            "trigger[].last_fired_secs",
            "scenario",
            "float",
            Derived,
            None,
        ),
        (
            "trigger[].fire_history[].fired_secs",
            "scenario",
            "float",
            Derived,
            None,
        ),
        (
            "trigger[].fire_history[].predicate_values",
            "scenario",
            "list",
            Derived,
            None,
        ),
        (
            "delayed_action[].action",
            "scenario",
            "string",
            Derived,
            None,
        ),
        (
            "delayed_action[].entity",
            "scenario",
            "string",
            Derived,
            None,
        ),
        (
            "delayed_action[].fire_at_secs",
            "scenario",
            "float",
            Derived,
            None,
        ),
        ("commitment[].id", "scenario", "string", Derived, None),
        ("commitment[].made_to", "scenario", "string", Derived, None),
        ("commitment[].terms", "scenario", "string", Derived, None),
        (
            "commitment[].resolves_when",
            "scenario",
            "string",
            Derived,
            None,
        ),
        ("commitment[].state", "scenario", "string", Derived, None),
        (
            "commitment[].made_at_tick",
            "scenario",
            "integer",
            Derived,
            None,
        ),
        (
            "commitment[].resolved_at_tick",
            "scenario",
            "integer",
            Derived,
            None,
        ),
        (
            "dossier[].subject_uuid",
            "scenario",
            "string",
            Derived,
            None,
        ),
        ("dossier[].text", "scenario", "string", Derived, None),
        ("dossier[].provenance", "scenario", "string", Derived, None),
        (
            "dossier[].gathered_at_tick",
            "scenario",
            "integer",
            Derived,
            None,
        ),
        (
            "session.paused",
            "session",
            "bool",
            NamedAction,
            Some("session"),
        ),
    ] {
        fields.push(field(id, group, kind, mutability, panel));
    }
    let mut root = WorldInspection {
        label: world.global.title.clone().unwrap_or_else(|| "root".into()),
        origin_layer: Some("root".into()),
        values: BTreeMap::new(),
    };
    definition(&mut root, world);
    if let Some(paused) = paused {
        root.values
            .insert("session.paused".into(), paused.to_string());
    }
    readings.insert("root".into(), root);
    if let Some(layers) = layers {
        for (path, layer) in &layers.0 {
            if layer.is_active {
                let mut reading = WorldInspection {
                    label: path.clone(),
                    origin_layer: Some(path.clone()),
                    values: BTreeMap::new(),
                };
                if let Some(config) = layer.inspection_config.as_deref() {
                    definition(&mut reading, config);
                }
                let mut flags = layer.flags.iter().collect::<Vec<_>>();
                flags.sort_by(|left, right| left.0.cmp(right.0));
                for (name, value) in flags {
                    put(&mut reading, "flag", name, "name", name);
                    put(&mut reading, "flag", name, "value", value);
                }
                readings.insert(path.clone(), reading);
            }
        }
    }
    if let Some(runtime) = runtime {
        let empty_objectives = crate::objectives::ObjectiveManager::default();
        let scenario = crate::debug::scenario::collect_scenario_state(
            runtime,
            objectives.unwrap_or(&empty_objectives),
        );
        for reading in readings.values_mut() {
            reading.values.insert(
                "scenario.schema_version".into(),
                scenario.schema_version.to_string(),
            );
        }
        let root = readings.get_mut("root").expect("root reading");
        for row in &scenario.flags {
            put(root, "flag", &row.name, "name", &row.name);
            put(root, "flag", &row.name, "value", row.value);
        }
        let events = crate::gm_event::controllable_events(
            &runtime.triggers,
            &runtime.pending_gm_event_fires,
            &runtime.paused_gm_events,
            &runtime.pending_gm_event_skips,
        );
        for (index, state) in runtime.triggers.iter().enumerate() {
            let origin = state.origin_layer.as_deref().unwrap_or("root");
            let Some(reading) = readings.get_mut(origin) else {
                continue;
            };
            let key = index.to_string();
            let debug = &scenario.triggers[index];
            put_optional(reading, "trigger", &key, "id", debug.id.as_deref());
            put(reading, "trigger", &key, "condition", &debug.condition);
            put_optional(reading, "trigger", &key, "when", debug.when.as_deref());
            put(reading, "trigger", &key, "repeat", debug.repeat);
            put_optional(
                reading,
                "trigger",
                &key,
                "cooldown_secs",
                state.trigger.cooldown_secs,
            );
            put(reading, "trigger", &key, "fired", debug.fired);
            put(reading, "trigger", &key, "pending", debug.pending);
            put(reading, "trigger", &key, "when_holds", debug.when_holds);
            put_optional(
                reading,
                "trigger",
                &key,
                "last_fired_secs",
                debug.last_fired_secs,
            );
            for (fire_index, fire) in debug.fire_history.iter().enumerate() {
                let prefix = format!("trigger[{key}].fire_history[{fire_index}]");
                reading
                    .values
                    .insert(format!("{prefix}.fired_secs"), fire.fired_secs.to_string());
                reading.values.insert(
                    format!("{prefix}.predicate_values"),
                    fire.predicate_values
                        .iter()
                        .map(|value| format!("{}={}", value.atom, value.value))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            let Some(controls) = state.trigger.gm_controls.as_ref() else {
                continue;
            };
            let event_id =
                crate::gm_event::qualified_event_id(state.origin_layer.as_deref(), &controls.id);
            let Some(event) = events.iter().find(|event| event.id == event_id) else {
                continue;
            };
            for (key, value) in [
                ("id", event.id.clone()),
                ("label", event.label.clone()),
                ("condition", debug.condition.clone()),
                ("when", debug.when.clone().unwrap_or_default()),
                ("fire", event.fire.to_string()),
                ("pause", event.pause.to_string()),
                ("skip", event.skip.to_string()),
                ("repeatable", event.repeatable.to_string()),
                (
                    "cooldown_secs",
                    state
                        .trigger
                        .cooldown_secs
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                (
                    "attention_band",
                    controls
                        .attention_band
                        .clone()
                        .unwrap_or_else(|| "attention".into()),
                ),
                ("paused", event.paused.to_string()),
                ("spent", event.spent.to_string()),
                ("armed", event.armed.to_string()),
                ("skip_armed", event.skip_armed.to_string()),
            ] {
                put(reading, "event", &event.id, key, value);
            }
        }
        for (index, row) in scenario.delayed_actions.iter().enumerate() {
            let origin = runtime.pending_delayed_actions[index]
                .origin_layer
                .as_deref()
                .unwrap_or("root");
            let Some(reading) = readings.get_mut(origin) else {
                continue;
            };
            let key = index.to_string();
            put(reading, "delayed_action", &key, "action", &row.action);
            put_optional(
                reading,
                "delayed_action",
                &key,
                "entity",
                row.entity.as_deref(),
            );
            put(
                reading,
                "delayed_action",
                &key,
                "fire_at_secs",
                row.fire_at_secs,
            );
        }
        for record in &runtime.deadlines.records {
            let origin = record.origin_layer.as_deref().unwrap_or("root");
            let Some(reading) = readings.get_mut(origin) else {
                continue;
            };
            let id = record.id.clone();
            for (key, value) in [
                ("id", record.id.clone()),
                ("label", record.label.clone()),
                ("visible", record.visible.to_string()),
                ("due_tick", record.due_tick.to_string()),
                ("state", record.state.as_str().to_string()),
            ] {
                put(reading, "deadline", &id, key, value);
            }
        }
        for objective in &scenario.objectives {
            let origin = layers
                .and_then(|layers| {
                    layers.0.iter().find_map(|(path, layer)| {
                        layer
                            .owned_objective_ids
                            .contains(&objective.id)
                            .then_some(path.as_str())
                    })
                })
                .unwrap_or("root");
            let Some(reading) = readings.get_mut(origin) else {
                continue;
            };
            for (key, value) in [
                ("id", objective.id.clone()),
                ("status", format!("{:?}", objective.status)),
                ("mandatory", objective.mandatory.to_string()),
                ("base_priority", objective.base_priority.to_string()),
                ("directive", format!("{:?}", objective.directive)),
            ] {
                put(reading, "objective", &objective.id, key, value);
            }
        }
        let root = readings.get_mut("root").expect("root reading");
        for row in &scenario.commitments {
            for (field, value) in [
                ("id", row.id.clone()),
                ("made_to", row.made_to.clone()),
                ("terms", row.terms.clone()),
                ("resolves_when", row.resolves_when.clone()),
                ("state", row.state.clone()),
                ("made_at_tick", row.made_at_tick.to_string()),
            ] {
                put(root, "commitment", &row.id, field, value);
            }
            put_optional(
                root,
                "commitment",
                &row.id,
                "resolved_at_tick",
                row.resolved_at_tick,
            );
        }
        for (index, row) in scenario.dossier.iter().enumerate() {
            let key = index.to_string();
            for (field, value) in [
                ("subject_uuid", row.subject_uuid.clone()),
                ("text", row.text.clone()),
                ("provenance", row.provenance.clone()),
                ("gathered_at_tick", row.gathered_at_tick.to_string()),
            ] {
                put(root, "dossier", &key, field, value);
            }
        }
    }
    fields.sort_by(|a, b| a.id.cmp(&b.id));
    fields.dedup_by(|a, b| a.id == b.id);
    WorldInspectorProjection { fields, readings }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_has_no_generic_mutation_and_flags_are_derived() {
        let world = crate::world::config::parse_world(
            "[global]\ntitle='Probe'\n[[deadline]]\nid='end'\ndue_secs=10\n",
        )
        .unwrap();
        let projection = projection(Some(&world), None, None, None, Some(false));
        assert!(projection
            .fields
            .iter()
            .any(|f| f.id == "global.sim_tick_hz"));
        assert!(projection
            .fields
            .iter()
            .any(|f| f.id == "global.description"));
        let flags = projection
            .fields
            .iter()
            .find(|f| f.id == "flag[].value")
            .unwrap();
        assert_eq!(flags.descriptor.live_mutability, LiveMutability::Derived);
        assert!(projection
            .fields
            .iter()
            .filter(|f| f.descriptor.live_mutability == LiveMutability::NamedAction)
            .all(|f| f.action_panel.is_some()));
    }

    #[test]
    fn active_supporting_layer_keeps_its_path_provenance_and_derived_flags() {
        let world = crate::world::config::parse_world("[global]\ntitle='Probe'\n").unwrap();
        let layer_config = crate::world::config::parse_world(
            "[global]\ntitle='Reinforcements'\nsim_tick_hz=120\nai_tick_hz=20\nai_snapshot_hz=10\n",
        )
        .unwrap();
        let mut layer = crate::world::server::WorldRuntime {
            inspection_config: Some(Box::new(layer_config)),
            is_active: true,
            ..Default::default()
        };
        layer.flags.set_flag_value("wave", 3);
        let layers = crate::world::server::WorldLayerMap(std::collections::HashMap::from([(
            "assets/worlds/reinforcements.toml".to_owned(),
            layer,
        )]));

        let projection = projection(Some(&world), None, None, Some(&layers), None);
        let reading = projection
            .readings
            .get("assets/worlds/reinforcements.toml")
            .unwrap();
        assert_eq!(
            reading.origin_layer.as_deref(),
            Some("assets/worlds/reinforcements.toml")
        );
        assert_eq!(reading.values["flag[wave].value"], "3");
        assert_eq!(reading.values["global.title"], "Reinforcements");
        assert_eq!(reading.values["global.sim_tick_hz"], "120.0");
    }
}
