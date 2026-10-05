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
    assert_eq!(
        fields
            .iter()
            .find(|field| field.id == "behaviour.doctrine")
            .and_then(|field| field.action_panel.as_deref()),
        Some("npc")
    );
    assert!(fields.iter().all(|field| {
        field.descriptor.live_mutability == LiveMutability::NamedAction
            || field.action_panel.is_none()
    }));
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
