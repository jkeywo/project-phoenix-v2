use super::*;
use crate::world::load::MemoryTemplateLoader;

fn loader() -> MemoryTemplateLoader {
    // `mass` is set explicitly to what a real `from_toml` parse of an
    // unauthored-mass template would produce (issue #1154): the bare
    // `#[derive(Default)]` on `EntityConfig` gives `mass` its type
    // default (`0.0`), not `default_mass()`'s `DEFAULT_ENTITY_MASS` —
    // only serde deserialisation runs the field-level
    // `#[serde(default = ...)]`. Left at `0.0`, this stand-in template
    // would fail `validate_mass` the moment `resolve()` round-trips it
    // through `apply_overrides`, unlike any template a real loader would
    // ever hand back.
    MemoryTemplateLoader::new([(
        "harrow.toml",
        EntityConfig {
            name: Some("Harrow Destroyer".to_string()),
            tags: vec!["npc".to_string()],
            mass: crate::entities::config::DEFAULT_ENTITY_MASS,
            ..Default::default()
        },
    )])
}

fn origin() -> SpawnOrigin {
    SpawnOrigin {
        template_path: "harrow.toml".to_string(),
        name: "wave_1".to_string(),
        position: [10.0, 0.0, -4.0],
        ..Default::default()
    }
}

#[test]
fn a_resolved_origin_carries_the_scenarios_name_not_the_templates() {
    let mut warnings = Vec::new();
    let config = origin()
        .resolve(&loader(), &mut warnings)
        .expect("the template resolves");
    assert_eq!(config.name.as_deref(), Some("wave_1"));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn overrides_are_merged_over_the_template() {
    let mut origin = origin();
    origin.overrides =
        Some(toml::from_str::<toml::Value>("tags = [\"hostile\"]").expect("the override parses"));
    let mut warnings = Vec::new();
    let config = origin
        .resolve(&loader(), &mut warnings)
        .expect("the template resolves");
    assert_eq!(config.tags, vec!["hostile".to_string()]);
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A missing template is a gap, not a hull with invented dimensions.
#[test]
fn a_template_that_does_not_resolve_is_reported_and_spawns_nothing() {
    let mut origin = origin();
    origin.template_path = "absent.toml".to_string();
    let mut warnings = Vec::new();
    assert!(origin.resolve(&loader(), &mut warnings).is_none());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("absent.toml"), "{warnings:?}");
}

/// A rejected override keeps the template and says so — a partial hull
/// beats none, which is the rule the spawn path itself follows.
#[test]
fn a_rejected_override_warns_and_keeps_the_template() {
    let mut origin = origin();
    origin.overrides =
        Some(toml::from_str::<toml::Value>("tags = { _remove = true }").expect("parses"));
    let mut warnings = Vec::new();
    let config = origin
        .resolve(&loader(), &mut warnings)
        .expect("the template still resolves");
    assert_eq!(config.tags, vec!["npc".to_string()]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

/// The record travels through the same serde the payload uses, dynamic
/// override document and all.
#[test]
fn a_record_round_trips_through_ron() {
    let mut origin = origin();
    origin.rotation = Some([0.0, 1.5, 0.0]);
    origin.scale = Some([2.0, 2.0, 2.0]);
    origin.layer_path = Some("assets/worlds/layer.toml".to_string());
    origin.overrides = Some(
        toml::from_str::<toml::Value>(
            "faction = \"raider\"\nspeed = 12\n[behaviour]\npool = \"raid\"\n",
        )
        .expect("parses"),
    );
    let text = ron::ser::to_string(&origin).expect("serialises");
    let back: SpawnOrigin = ron::de::from_str(&text).expect("parses back");
    assert_eq!(back, origin);
}
