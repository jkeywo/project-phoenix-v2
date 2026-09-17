//! Live inspection of instantiated entities and their AI configuration.
//!
//! This is a READING, never a write path. Every field here is classified
//! `Derived` or `RecreateRequired` except the one that already has a named
//! action — authored NPC doctrine, which the existing checked transaction owns
//! and which this panel links to rather than duplicating. There is no generic
//! field setter, and adding one would defeat the classification.
//!
//! Two deliberate limits on what a reading may claim:
//!
//! * Composition provenance is resolved during Authoring and dropped before
//!   spawn — `config_cache` keeps the parsed `EntityConfig`, not the
//!   `Provenance` that produced it. So a Live reading cannot name the include
//!   layer a field won in, and it says so (`origin.document`/`line`/`layer`
//!   stay `None`) rather than inferring one.
//! * "Layer" in a Live descriptor means the authored sub-world layer, never the
//!   include chain. The runtime retains that layer for an authored DEFINITION
//!   (`NpcDoctrinePaletteEntry.origin_layer`) but not for a spawned entity, so
//!   entity descriptors report it unavailable too rather than guessing from the
//!   world the entity happens to be standing in.
use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which part of the entity/AI surface a field belongs to, so the panel can
/// group readings without hard-coding the schema in JavaScript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntityFieldGroup {
    Identity,
    Placement,
    Faction,
    Tags,
    Behaviour,
    Ai,
    Target,
    Derived,
}

/// One known schema field: what it is, where it came from, and what Live may
/// do with it. Published once for the whole domain rather than per entity —
/// the descriptor is a property of the schema, not of an instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityInspectorField {
    /// Stable key, equal to `descriptor.origin.schema_path`. Readings are keyed
    /// by this, never by the field's label.
    pub id: String,
    /// String-table id. Player-visible text is never composed here.
    pub label: String,
    pub group: EntityFieldGroup,
    #[serde(flatten)]
    pub descriptor: FieldDescriptor,
}

/// Every reading for one entity, by field id. An absent field is absent from
/// the map: the schema says a table may be unauthored, and an empty string
/// would claim it was authored empty.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityInspection {
    pub values: BTreeMap<String, String>,
    /// Field id -> the entity id that field REFERS to, where the reading names
    /// another live object. This is what makes reference navigation possible
    /// without the panel guessing: a hop follows an id, never the displayed
    /// name, so it cannot land on a different entity that happens to share one.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub references: BTreeMap<String, String>,
}

/// The whole domain: one descriptor table plus one reading per live entity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityInspectorProjection {
    pub fields: Vec<EntityInspectorField>,
    /// Keyed by `entity_id`. A despawned entity simply stops appearing; the
    /// panel keeps its final reading and marks it gone, because the runtime has
    /// nothing left to say about it and a replacement sharing its authored name
    /// is a different entity.
    pub readings: BTreeMap<String, EntityInspection>,
}

fn field(
    id: &str,
    label: &str,
    group: EntityFieldGroup,
    kind: &str,
    mutability: LiveMutability,
) -> EntityInspectorField {
    EntityInspectorField {
        id: id.into(),
        label: label.into(),
        group,
        descriptor: FieldDescriptor {
            kind: kind.into(),
            default_source: None,
            live_mutability: mutability,
            origin: FieldOrigin {
                schema_path: id.into(),
                // See the module header: resolution provenance does not survive
                // spawn, so a Live reading reports its location unavailable.
                document: None,
                line: None,
                layer: None,
            },
            validation: Vec::new(),
        },
    }
}

/// `[behaviour]` tuning scalars. Numeric gameplay tuning is not live-editable.
const BEHAVIOUR_SCALARS: &[(&str, &str)] = &[
    (
        "waypoint_arrival_radius",
        "inspector.entity.waypoint_arrival_radius",
    ),
    ("avoidance_buffer", "inspector.entity.avoidance_buffer"),
    (
        "avoidance_look_ahead_secs",
        "inspector.entity.avoidance_look_ahead_secs",
    ),
    ("nav_handoff_speed", "inspector.entity.nav_handoff_speed"),
    (
        "docking_engage_distance",
        "inspector.entity.docking_engage_distance",
    ),
    (
        "docking_approach_speed",
        "inspector.entity.docking_approach_speed",
    ),
    (
        "hazard_ignore_size_ratio",
        "inspector.entity.hazard_ignore_size_ratio",
    ),
    (
        "hazard_threat_exponent",
        "inspector.entity.hazard_threat_exponent",
    ),
    (
        "low_lod_avoidance_deviation_rad",
        "inspector.entity.low_lod_avoidance_deviation_rad",
    ),
    (
        "lateral_hazard_sensitivity",
        "inspector.entity.lateral_hazard_sensitivity",
    ),
    (
        "vertical_hazard_sensitivity",
        "inspector.entity.vertical_hazard_sensitivity",
    ),
    (
        "imminent_collision_facing_threshold",
        "inspector.entity.imminent_collision_facing_threshold",
    ),
];

/// `[[behaviour.doctrine]]` element schema, addressed the way
/// `include_resolve`'s provenance addresses a key-addressed array member.
const DOCTRINE_OBJECTIVE_FIELDS: &[(&str, &str, &str)] = &[
    ("id", "inspector.entity.doctrine_id", "string"),
    ("text", "inspector.entity.doctrine_text", "string"),
    ("mandatory", "inspector.entity.doctrine_mandatory", "bool"),
    (
        "directive_kind",
        "inspector.entity.doctrine_directive_kind",
        "string",
    ),
    (
        "directive_anchors",
        "inspector.entity.doctrine_directive_anchors",
        "list",
    ),
    (
        "directive_loop",
        "inspector.entity.doctrine_directive_loop",
        "bool",
    ),
    (
        "directive_target",
        "inspector.entity.doctrine_directive_target",
        "string",
    ),
    (
        "directive_anchor",
        "inspector.entity.doctrine_directive_anchor",
        "string",
    ),
    (
        "directive_dock_target",
        "inspector.entity.doctrine_directive_dock_target",
        "string",
    ),
    (
        "directive_hail_target",
        "inspector.entity.doctrine_directive_hail_target",
        "string",
    ),
    (
        "directive_scan_target",
        "inspector.entity.doctrine_directive_scan_target",
        "string",
    ),
    (
        "directive_operate_target",
        "inspector.entity.doctrine_directive_operate_target",
        "string",
    ),
    (
        "directive_order_target",
        "inspector.entity.doctrine_directive_order_target",
        "string",
    ),
    (
        "directive_order_route",
        "inspector.entity.doctrine_directive_order_route",
        "list",
    ),
    (
        "base_priority",
        "inspector.entity.doctrine_base_priority",
        "float",
    ),
    ("zero_gates", "inspector.entity.doctrine_zero_gates", "list"),
    ("modifiers", "inspector.entity.doctrine_modifiers", "list"),
    (
        "target_speed",
        "inspector.entity.doctrine_target_speed",
        "float",
    ),
    (
        "maintain_range",
        "inspector.entity.doctrine_maintain_range",
        "float",
    ),
    (
        "use_impulse",
        "inspector.entity.doctrine_use_impulse",
        "bool",
    ),
];

/// `[ai_profile]` scalars.
const AI_PROFILE_SCALARS: &[(&str, &str)] = &[
    ("aggression", "inspector.entity.aggression"),
    ("sensor_range", "inspector.entity.sensor_range"),
    (
        "low_lod_cruise_fraction",
        "inspector.entity.low_lod_cruise_fraction",
    ),
    (
        "low_lod_speed_decay_per_sec",
        "inspector.entity.low_lod_speed_decay_per_sec",
    ),
    (
        "low_lod_turn_rate_fraction",
        "inspector.entity.low_lod_turn_rate_fraction",
    ),
];

/// `[target]` — what this entity looks like to another ship's Tactical.
const TARGET_FIELDS: &[(&str, &str, &str)] = &[
    ("tags", "inspector.entity.target_tags", "list"),
    (
        "threat_level",
        "inspector.entity.target_threat_level",
        "string",
    ),
    (
        "description",
        "inspector.entity.target_description",
        "string",
    ),
];

/// Runtime context, shown only where it explains an authored field.
const DERIVED_FIELDS: &[(&str, &str)] = &[
    (
        "scored_objectives.chosen",
        "inspector.entity.derived_intent",
    ),
    ("current_target", "inspector.entity.derived_target"),
    ("control_source", "inspector.entity.derived_control_source"),
    ("modifiers.effective", "inspector.entity.derived_modifiers"),
];

/// Every known entity/AI schema field this domain covers.
///
/// The list is the contract #1494's ratchet checks against, so a field added to
/// the authored schema without a row here is meant to fail that test rather
/// than quietly disappear from the Inspector.
pub fn fields() -> Vec<EntityInspectorField> {
    use EntityFieldGroup as G;
    use LiveMutability::{Derived, RecreateRequired};
    let mut fields = vec![
        field(
            "name",
            "inspector.entity.name",
            G::Identity,
            "string",
            RecreateRequired,
        ),
        field(
            "id",
            "inspector.entity.id",
            G::Identity,
            "string",
            RecreateRequired,
        ),
        field(
            "mass",
            "inspector.entity.mass",
            G::Identity,
            "float",
            RecreateRequired,
        ),
        // An AI ranking input, not decoration: authored selectors read it as
        // `self_fact(power_rating)`. The runtime carries it on the selector
        // components, so it is present exactly when a selector is authored.
        field(
            "power_rating",
            "inspector.entity.power_rating",
            G::Ai,
            "float",
            RecreateRequired,
        ),
        field(
            "transform.translation",
            "inspector.entity.translation",
            G::Placement,
            "vec3",
            RecreateRequired,
        ),
        field(
            "transform.rotation",
            "inspector.entity.rotation",
            G::Placement,
            "quat",
            RecreateRequired,
        ),
        field(
            "transform.scale",
            "inspector.entity.scale",
            G::Placement,
            "vec3",
            RecreateRequired,
        ),
        field(
            "faction",
            "inspector.entity.faction",
            G::Faction,
            "uuid",
            RecreateRequired,
        ),
        field(
            "tags",
            "inspector.entity.tags",
            G::Tags,
            "list",
            RecreateRequired,
        ),
    ];
    // Authored standing doctrine is the one entity/AI field with a named
    // action. The panel links to the existing checked transaction rather than
    // offering a second form for it.
    let mut doctrine = field(
        "behaviour.doctrine",
        "inspector.entity.doctrine",
        G::Behaviour,
        "list",
        LiveMutability::NamedAction,
    );
    doctrine.descriptor.validation = vec![
        "inspector.validation.authored_choice".into(),
        "inspector.validation.npc_compatible".into(),
    ];
    fields.push(doctrine);
    for (id, label) in BEHAVIOUR_SCALARS {
        fields.push(field(
            &format!("behaviour.{id}"),
            label,
            G::Behaviour,
            "float",
            RecreateRequired,
        ));
    }
    for (id, label, kind) in DOCTRINE_OBJECTIVE_FIELDS {
        fields.push(field(
            &format!("behaviour.doctrine[].{id}"),
            label,
            G::Behaviour,
            kind,
            RecreateRequired,
        ));
    }
    for (id, label) in AI_PROFILE_SCALARS {
        fields.push(field(
            &format!("ai_profile.{id}"),
            label,
            G::Ai,
            "float",
            RecreateRequired,
        ));
    }
    fields.push(field(
        "lod_bubble.radius",
        "inspector.entity.lod_bubble_radius",
        G::Ai,
        "float",
        RecreateRequired,
    ));
    for (id, label, kind) in TARGET_FIELDS {
        fields.push(field(
            &format!("target.{id}"),
            label,
            G::Target,
            kind,
            RecreateRequired,
        ));
    }
    // Derived context. Each one explains an authored field beside it and is
    // never editable: an intent explains the doctrine that scored it, a chosen
    // target explains the target section, a control source explains who is
    // acting on the authored behaviour, and the modifier count explains why an
    // effective value differs from the authored one.
    for (id, label) in DERIVED_FIELDS {
        fields.push(field(id, label, G::Derived, "string", Derived));
    }
    fields
}

/// Everything one reading is built from, gathered by the Bevy adapter in
/// `gm_projection` so this stays a pure function over plain values.
///
/// Every field is optional because absence is meaningful: a hull that authored
/// no `[ai_profile]` has no `ai_profile.*` reading at all, which is a different
/// statement from authoring its defaults explicitly.
#[derive(Default)]
pub struct EntityReadingInputs<'a> {
    pub name: Option<&'a str>,
    pub id: Option<&'a str>,
    pub mass: Option<f32>,
    pub power_rating: Option<f32>,
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    pub scale: Option<[f32; 3]>,
    pub faction: Option<&'a str>,
    pub tags: Option<&'a [String]>,
    pub behaviour: Option<&'a crate::entities::config::BehaviourConfig>,
    pub ai_profile: Option<&'a crate::ai::server::AiProfile>,
    pub lod_bubble_radius: Option<f32>,
    pub target: Option<&'a crate::entities::target::TargetSection>,
    /// Derived context, already resolved by the adapter.
    pub intent: Option<&'a str>,
    pub current_target: Option<&'a str>,
    /// The entity id behind `current_target`, when the target is a live object.
    pub current_target_id: Option<&'a str>,
    pub control_source: Option<&'a str>,
    pub modifiers: Option<String>,
}

fn vec3(value: [f32; 3]) -> String {
    format!("{:.3}, {:.3}, {:.3}", value[0], value[1], value[2])
}

/// One reading per authored field the entity actually carries.
///
/// A doctrine element field is reported per objective, keyed by the objective's
/// own authored id — the same key-addressed vocabulary `include_resolve` uses
/// for arrays — so a hull with three objectives reads three values under one
/// descriptor rather than needing three descriptors.
pub fn reading(inputs: &EntityReadingInputs) -> EntityInspection {
    let mut values = BTreeMap::new();
    let mut references = BTreeMap::new();
    let mut put = |key: &str, value: String| {
        values.insert(key.to_string(), value);
    };
    if let Some(name) = inputs.name {
        put("name", name.to_string());
    }
    if let Some(id) = inputs.id {
        put("id", id.to_string());
    }
    if let Some(mass) = inputs.mass {
        put("mass", format!("{mass:.3}"));
    }
    if let Some(rating) = inputs.power_rating {
        put("power_rating", format!("{rating:.3}"));
    }
    if let Some(translation) = inputs.translation {
        put("transform.translation", vec3(translation));
    }
    if let Some(r) = inputs.rotation {
        put(
            "transform.rotation",
            format!("{:.3}, {:.3}, {:.3}, {:.3}", r[0], r[1], r[2], r[3]),
        );
    }
    if let Some(scale) = inputs.scale {
        put("transform.scale", vec3(scale));
    }
    if let Some(faction) = inputs.faction {
        put("faction", faction.to_string());
    }
    if let Some(tags) = inputs.tags {
        put("tags", tags.join(", "));
    }
    if let Some(behaviour) = inputs.behaviour {
        put(
            "behaviour.doctrine",
            behaviour
                .doctrine
                .iter()
                .map(|objective| objective.id.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        for (id, value) in [
            ("waypoint_arrival_radius", behaviour.waypoint_arrival_radius),
            ("avoidance_buffer", behaviour.avoidance_buffer),
            (
                "avoidance_look_ahead_secs",
                behaviour.avoidance_look_ahead_secs,
            ),
            ("nav_handoff_speed", behaviour.nav_handoff_speed),
            ("docking_engage_distance", behaviour.docking_engage_distance),
            ("docking_approach_speed", behaviour.docking_approach_speed),
            (
                "hazard_ignore_size_ratio",
                behaviour.hazard_ignore_size_ratio,
            ),
            ("hazard_threat_exponent", behaviour.hazard_threat_exponent),
            (
                "low_lod_avoidance_deviation_rad",
                behaviour.low_lod_avoidance_deviation_rad,
            ),
            (
                "lateral_hazard_sensitivity",
                behaviour.lateral_hazard_sensitivity,
            ),
            (
                "vertical_hazard_sensitivity",
                behaviour.vertical_hazard_sensitivity,
            ),
            (
                "imminent_collision_facing_threshold",
                behaviour.imminent_collision_facing_threshold,
            ),
        ] {
            put(&format!("behaviour.{id}"), format!("{value:.3}"));
        }
        for (id, render) in doctrine_readings(&behaviour.doctrine) {
            put(&format!("behaviour.doctrine[].{id}"), render);
        }
    }
    if let Some(profile) = inputs.ai_profile {
        for (id, value) in [
            ("aggression", profile.aggression),
            ("sensor_range", profile.sensor_range),
            ("low_lod_cruise_fraction", profile.low_lod_cruise_fraction),
            (
                "low_lod_speed_decay_per_sec",
                profile.low_lod_speed_decay_per_sec,
            ),
            (
                "low_lod_turn_rate_fraction",
                profile.low_lod_turn_rate_fraction,
            ),
        ] {
            put(&format!("ai_profile.{id}"), format!("{value:.3}"));
        }
    }
    if let Some(radius) = inputs.lod_bubble_radius {
        put("lod_bubble.radius", format!("{radius:.3}"));
    }
    if let Some(target) = inputs.target {
        put("target.tags", target.tags.join(", "));
        put(
            "target.threat_level",
            target.threat_level.as_str().to_string(),
        );
        if let Some(description) = target.description.as_deref() {
            put("target.description", description.to_string());
        }
    }
    if let Some(intent) = inputs.intent {
        put("scored_objectives.chosen", intent.to_string());
    }
    if let Some(target) = inputs.current_target {
        put("current_target", target.to_string());
    }
    if let Some(id) = inputs.current_target_id {
        references.insert("current_target".to_string(), id.to_string());
    }
    if let Some(source) = inputs.control_source {
        put("control_source", source.to_string());
    }
    if let Some(modifiers) = inputs.modifiers.as_deref() {
        put("modifiers.effective", modifiers.to_string());
    }
    EntityInspection { values, references }
}

/// Render each `[[behaviour.doctrine]]` element field across every authored
/// objective, as `id=<objective>: <value>` joined in authored order.
fn doctrine_readings(
    doctrine: &[crate::entities::config::DoctrineObjective],
) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (id, _, _) in DOCTRINE_OBJECTIVE_FIELDS {
        let mut parts = Vec::new();
        for objective in doctrine {
            if let Some(value) = doctrine_field(objective, id) {
                parts.push(format!("id={}: {value}", objective.id));
            }
        }
        if !parts.is_empty() {
            out.push((*id, parts.join("; ")));
        }
    }
    out
}

fn doctrine_field(
    objective: &crate::entities::config::DoctrineObjective,
    field: &str,
) -> Option<String> {
    let list = |values: &[String]| {
        if values.is_empty() {
            None
        } else {
            Some(values.join(", "))
        }
    };
    match field {
        "id" => Some(objective.id.clone()),
        "text" => Some(objective.text.clone()),
        "mandatory" => Some(objective.mandatory.to_string()),
        "directive_kind" => objective.directive_kind.clone(),
        "directive_anchors" => list(&objective.directive_anchors),
        "directive_loop" => Some(objective.directive_loop.to_string()),
        "directive_target" => objective.directive_target.clone(),
        "directive_anchor" => objective.directive_anchor.clone(),
        "directive_dock_target" => objective.directive_dock_target.clone(),
        "directive_hail_target" => objective.directive_hail_target.clone(),
        "directive_scan_target" => objective.directive_scan_target.clone(),
        "directive_operate_target" => objective.directive_operate_target.clone(),
        "directive_order_target" => objective.directive_order_target.clone(),
        "directive_order_route" => objective.directive_order_route.clone(),
        "base_priority" => Some(format!("{:.3}", objective.base_priority)),
        "zero_gates" => (!objective.zero_gates.is_empty()).then(|| {
            objective
                .zero_gates
                .iter()
                .map(|gate| match gate.threshold {
                    Some(threshold) => format!("{} {threshold:.3}", gate.condition),
                    None => gate.condition.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        }),
        "modifiers" => (!objective.modifiers.is_empty()).then(|| {
            objective
                .modifiers
                .iter()
                .map(|modifier| match modifier.threshold {
                    Some(threshold) => format!(
                        "{} {threshold:.3} {:+.3}",
                        modifier.condition, modifier.weight
                    ),
                    None => format!("{} {:+.3}", modifier.condition, modifier.weight),
                })
                .collect::<Vec<_>>()
                .join(", ")
        }),
        "target_speed" => Some(format!("{:.3}", objective.target_speed)),
        "maintain_range" => Some(format!("{:.3}", objective.maintain_range)),
        "use_impulse" => objective.use_impulse.map(|value| value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::config::{BehaviourConfig, DoctrineObjective};

    fn objective(id: &str) -> DoctrineObjective {
        DoctrineObjective {
            id: id.to_string(),
            text: format!("{id} text"),
            ..Default::default()
        }
    }

    #[test]
    fn live_inspector_classifies_every_known_entity_field() {
        let fields = fields();
        // Exactly one named action in this domain: authored NPC doctrine, which
        // an existing checked transaction owns. Any second one would be a write
        // path the classification is supposed to forbid.
        let named: Vec<&str> = fields
            .iter()
            .filter(|field| field.descriptor.live_mutability == LiveMutability::NamedAction)
            .map(|field| field.id.as_str())
            .collect();
        assert_eq!(named, vec!["behaviour.doctrine"]);
        // Every field is classified, keyed by its own schema path, and unique.
        let mut seen = std::collections::BTreeSet::new();
        for field in &fields {
            assert_eq!(
                field.id, field.descriptor.origin.schema_path,
                "{}",
                field.id
            );
            assert!(seen.insert(field.id.clone()), "duplicate {}", field.id);
            assert!(!field.label.is_empty(), "{}", field.id);
        }
        // Resolution provenance does not survive spawn, so a Live reading says
        // its location is unavailable instead of inferring one.
        for field in &fields {
            assert!(field.descriptor.origin.document.is_none(), "{}", field.id);
            assert!(field.descriptor.origin.line.is_none(), "{}", field.id);
        }
    }

    /// Every key the authored schema actually serialises, so the ratchet reads
    /// the struct rather than the list the descriptor table was built from.
    ///
    /// A test that walks the same consts `fields()` walks cannot fail — it only
    /// restates the table to itself. Serialising a real `Default` value asks
    /// the *type* what its fields are, so adding a field to `BehaviourConfig`
    /// and forgetting a descriptor fails here, which is the whole point.
    fn authored_keys<T: serde::Serialize>(value: &T) -> Vec<String> {
        let toml = toml::to_string(value).expect("authored schema serialises to TOML");
        let table: toml::Table = toml.parse().expect("authored schema parses back");
        table.keys().cloned().collect()
    }

    #[test]
    fn live_inspector_covers_every_authored_behaviour_and_ai_scalar() {
        let fields = fields();
        let ids: std::collections::BTreeSet<&str> =
            fields.iter().map(|field| field.id.as_str()).collect();

        // `[behaviour]`. `doctrine` is the array whose ELEMENT schema is
        // covered separately below, so it is checked under its own id.
        for key in authored_keys(&BehaviourConfig::default()) {
            let id = format!("behaviour.{key}");
            assert!(
                ids.contains(id.as_str()),
                "authored [behaviour] field has no Live descriptor: {id}"
            );
        }
        // `[[behaviour.doctrine]]` elements.
        for key in authored_keys(&DoctrineObjective::default()) {
            let id = format!("behaviour.doctrine[].{key}");
            assert!(
                ids.contains(id.as_str()),
                "authored doctrine field has no Live descriptor: {id}"
            );
        }
        // `[ai_profile]`.
        for key in authored_keys(&crate::entities::config::AiProfileConfig {
            aggression: 0.0,
            sensor_range: 0.0,
            low_lod_cruise_fraction: 0.0,
            low_lod_speed_decay_per_sec: 0.0,
            low_lod_turn_rate_fraction: 0.0,
        }) {
            let id = format!("ai_profile.{key}");
            assert!(
                ids.contains(id.as_str()),
                "authored [ai_profile] field has no Live descriptor: {id}"
            );
        }
        // `[target]`.
        let target = crate::entities::target::TargetSection {
            tags: Vec::new(),
            threat_level: Default::default(),
            description: Some(String::new()),
        };
        for key in authored_keys(&target) {
            let id = format!("target.{key}");
            assert!(
                ids.contains(id.as_str()),
                "authored [target] field has no Live descriptor: {id}"
            );
        }
        // `[lod_bubble]`.
        for key in authored_keys(&crate::entities::config::LodBubbleConfig { radius: 0.0 }) {
            let id = format!("lod_bubble.{key}");
            assert!(
                ids.contains(id.as_str()),
                "authored [lod_bubble] field has no Live descriptor: {id}"
            );
        }
        // Derived context is this domain's own vocabulary rather than an
        // authored table, so it is checked against the list that defines it.
        for (id, _) in DERIVED_FIELDS {
            assert!(ids.contains(id), "{id}");
        }
    }

    /// The identity and AI scalars that live on `EntityConfig` itself rather
    /// than in a table. Spelled out because `EntityConfig` also carries the
    /// hull, Region and presentation surfaces that other domains own.
    #[test]
    fn live_inspector_covers_the_entity_level_identity_and_ai_fields() {
        let ids: std::collections::BTreeSet<String> =
            fields().into_iter().map(|field| field.id).collect();
        for id in [
            "name",
            "id",
            "mass",
            // An AI ranking input read by authored selectors as
            // `self_fact(power_rating)`, so it belongs to this domain.
            "power_rating",
            "tags",
            "faction",
            "transform.translation",
            "transform.rotation",
            "transform.scale",
            "behaviour.doctrine",
        ] {
            assert!(
                ids.contains(id),
                "entity-level field has no descriptor: {id}"
            );
        }
    }

    #[test]
    fn live_inspector_reads_only_what_the_entity_authored() {
        // A hull with no [behaviour], [ai_profile], [lod_bubble] or [target].
        let bare = reading(&EntityReadingInputs {
            name: Some("Courier"),
            translation: Some([1.0, 2.0, 3.0]),
            ..Default::default()
        });
        assert_eq!(bare.values.get("name").map(String::as_str), Some("Courier"));
        assert_eq!(
            bare.values.get("transform.translation").map(String::as_str),
            Some("1.000, 2.000, 3.000")
        );
        // Absent is absent. An empty string here would claim the table was
        // authored empty, which is a different fact from unauthored.
        assert!(!bare.values.contains_key("ai_profile.aggression"));
        assert!(!bare.values.contains_key("behaviour.doctrine"));
        assert!(!bare.values.contains_key("lod_bubble.radius"));
    }

    #[test]
    fn live_inspector_reads_doctrine_elements_keyed_by_authored_id() {
        let behaviour = BehaviourConfig {
            doctrine: vec![objective("hold-station"), objective("destroy-hostiles")],
            ..Default::default()
        };
        let values = reading(&EntityReadingInputs {
            behaviour: Some(&behaviour),
            ..Default::default()
        })
        .values;
        assert_eq!(
            values.get("behaviour.doctrine").map(String::as_str),
            Some("hold-station, destroy-hostiles")
        );
        // Each element field reads once per objective, addressed by the
        // objective's own authored id rather than by its position.
        let ids = values
            .get("behaviour.doctrine[].id")
            .expect("doctrine ids read");
        assert!(ids.contains("id=hold-station"), "{ids}");
        assert!(ids.contains("id=destroy-hostiles"), "{ids}");
    }

    #[test]
    fn live_inspector_reports_derived_context_without_making_it_authored() {
        let values = reading(&EntityReadingInputs {
            intent: Some("Destroy the courier"),
            current_target: Some("Courier"),
            control_source: Some("human 1, ai 4, offline 0"),
            modifiers: Some("2 float, 0 int, 1 flags".to_string()),
            ..Default::default()
        })
        .values;
        assert_eq!(
            values.get("scored_objectives.chosen").map(String::as_str),
            Some("Destroy the courier")
        );
        assert_eq!(
            values.get("current_target").map(String::as_str),
            Some("Courier")
        );
        // Every one of them is classified Derived, so nothing renders it as an
        // editable control.
        let fields = fields();
        for (id, _) in DERIVED_FIELDS {
            let field = fields.iter().find(|field| field.id == *id).expect(id);
            assert_eq!(
                field.descriptor.live_mutability,
                LiveMutability::Derived,
                "{id}"
            );
        }
    }
}
