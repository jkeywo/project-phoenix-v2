use super::*;

const BEAM: &str = r#"
[[weapon]]
id = "phaser-fore"
strategy_id = "fore"
impact = { damage_type = "laser", penetration = 0 }
marker = "beam_mount"
[weapon.ai]
idle = true
[weapon.mount]
facing_deg = -90.0
fire_arc_deg = 180.0
range = 40.0
[weapon.delivery]
kind = "beam"
auto_arc_deg = 120.0
damage_per_sec = 5.0
duration_secs = 6.0
cooldown_secs = 6.0
cycle_jitter = 0.33
color = [0.5, 0.2, 0.0, 1.0]
shield_pierce = 0.1
"#;

const BOLT: &str = r#"
[[weapon]]
id = "blaster-fore"
strategy_id = "fore"
impact = { damage_type = "physical", penetration = 0 }
barrels = ["port", "starboard"]
pattern = [{barrels=[0],offset_secs=0.0}, {barrels=[1],offset_secs=0.2}]
[weapon.ai]
idle = true
[weapon.mount]
facing_deg = 0.0
fire_arc_deg = 90.0
range = 35.0
[weapon.delivery]
kind = "bolt"
volley_count = 0
volley_interval_secs = 0.15
cooldown_secs = 3.0
projectile_speed = 40.0
collision_radius = 1.5
visual_scale = 1.0
damage = 20
shield_pierce = 0.25
"#;

const GUIDED: &str = r#"
[torpedoes]
count = 12
speed = 15.0
lifespan = 20.0
[torpedoes.ai]
idle = true
[[weapon]]
id = "torpedo-tube-fore"
strategy_id = "fore"
impact = { damage_type = "physical", penetration = 0 }
barrels = ["port", "starboard"]
pattern = [{barrels=[0,1],offset_secs=0.0}]
[weapon.ai]
idle = true
[weapon.mount]
facing_deg = 0.0
fire_arc_deg = 90.0
range = 300.0
[weapon.delivery]
kind = "guided"
volley_max = 2
ai_target_count = 1
load_time = 5.0
"#;

fn parse(source: &str) -> Result<EntityConfig, toml::de::Error> {
    EntityConfig::from_toml_in_mode(source, AiDeclarationMode::Lenient)
}

#[test]
fn generic_beam_compiles_to_the_same_existing_strategy_and_round_trips() {
    let generic = parse(BEAM).unwrap();
    let shorthand = parse(
        r#"
[[weapons_console.phaser_banks]]
id="fore"
impact={damage_type="laser",penetration=0}
facing_deg=-90.0
fire_arc_deg=180.0
auto_arc_deg=120.0
beam_range=40.0
beam_damage_per_sec=5.0
beam_duration_secs=6.0
cooldown_secs=6.0
cycle_jitter=0.33
beam_color=[0.5,0.2,0.0,1.0]
shield_pierce=0.1
marker="beam_mount"
[weapons_console.phaser_banks.ai]
idle=true
"#,
    )
    .unwrap();
    assert_eq!(generic.weapons_console, shorthand.weapons_console);
    let emitted = toml::to_string(&generic.to_toml_value().unwrap()).unwrap();
    assert_eq!(
        parse(&emitted).unwrap().weapons_console,
        generic.weapons_console
    );
    assert!(!emitted.contains("[[weapon]]"));
}

#[test]
fn generic_bolt_reuses_the_existing_multi_barrel_cycle() {
    let config = parse(BOLT).unwrap();
    let bank = &config.weapons_console.unwrap().blaster_banks[0];
    assert_eq!(bank.id, "fore");
    assert_eq!(
        bank.to_runtime().firing_schedule(),
        vec![(vec![0], 0.0), (vec![1], 0.2)]
    );
    assert_eq!(bank.range / bank.projectile_speed, 35.0 / 40.0);
    assert_eq!(bank.shield_pierce, 0.25);
}

#[test]
fn generic_guided_preserves_shared_payload_and_tube_count_authority() {
    let config = parse(GUIDED).unwrap().torpedoes.unwrap();
    assert_eq!(config.count, 12);
    assert_eq!(
        config.to_runtime().damage_hull,
        TorpedoesConfig::default().damage_hull
    );
    let tube = &config.tubes[0];
    assert_eq!(tube.id, "fore");
    assert_eq!(tube.volley_max, 2);
    assert_eq!(tube.ai_target_count, Some(1));
    assert_eq!(tube.load_time, Some(5.0));
    assert_eq!(tube.pattern[0].barrels, vec![0, 1]);
}

#[test]
fn generic_identity_is_unique_but_strategy_identity_keeps_family_namespaces() {
    let source = format!("{GUIDED}\n{BEAM}");
    let config = parse(&source).unwrap();
    assert_eq!(config.torpedoes.unwrap().tubes[0].id, "fore");
    assert_eq!(config.weapons_console.unwrap().phaser_banks[0].id, "fore");
    let error = parse(&format!("{BEAM}\n{BEAM}")).unwrap_err().to_string();
    assert!(error.contains("duplicate generic weapon id"), "{error}");
    let error = parse(&format!(
        "{BEAM}\n{}",
        BEAM.replace("phaser-fore", "second-beam")
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("keep one definition"), "{error}");
}

#[test]
fn generic_and_shorthand_cannot_create_two_controllers_for_one_bank() {
    let source = format!("{BEAM}\n[[weapons_console.phaser_banks]]\nid='fore'\nimpact={damage_type='laser',penetration=0}\nfacing_deg=0.0\nfire_arc_deg=90.0\nauto_arc_deg=90.0\n");
    assert!(parse(&source)
        .unwrap_err()
        .to_string()
        .contains("keep one definition"));
}

#[test]
fn generic_definitions_refuse_unsupported_or_invalid_composition() {
    for (source, expected) in [
        (
            BEAM.replace("range = 40.0", "range = 0.0"),
            "positive finite range",
        ),
        (
            BEAM.replace("facing_deg = -90.0", "facing_deg = nan"),
            "finite facing",
        ),
        (
            BEAM.replace("damage_per_sec = 5.0", "damage_per_sec = -1.0"),
            "damage_per_sec",
        ),
        (
            BEAM.replace("cycle_jitter = 0.33", "cycle_jitter = 1.0"),
            "cycle_jitter",
        ),
        (
            BEAM.replace("shield_pierce = 0.1", "shield_pierce = nan"),
            "shield_pierce",
        ),
        (
            BEAM.replace("marker = \"beam_mount\"", "barrels = ['first']"),
            "beam delivery",
        ),
        (
            BEAM.replace("kind = \"beam\"", "kind = 'pdc'"),
            "unknown variant",
        ),
        (
            BEAM.replace("color = [0.5, 0.2, 0.0, 1.0]", "color = [1.0]"),
            "RGBA",
        ),
        (
            BOLT.replace("projectile_speed = 40.0", "projectile_speed = 0.0"),
            "projectile_speed",
        ),
        (BOLT.replace("barrels=[1]", "barrels=[2]"), "barrel index"),
        (
            BOLT.replace("offset_secs=0.2", "offset_secs=0.2, unsupported=true"),
            "unknown field",
        ),
        (
            BOLT.replace("damage = 20", "damage = 20\nunsupported = true"),
            "unknown field",
        ),
        (
            GUIDED.replace("range = 300.0", "range = 30.0"),
            "must equal",
        ),
        (
            GUIDED.replace("ai_target_count = 1", "ai_target_count = 3"),
            "ai_target_count",
        ),
    ] {
        let error = parse(&source).unwrap_err().to_string();
        assert!(error.contains(expected), "expected {expected}: {error}");
    }
}

#[test]
fn generic_ai_is_validated_by_the_existing_strategy_admission_contract() {
    let source = BEAM.replace(
        "idle = true",
        "[[weapon.ai.rule]]\nchannel='unsupported'\nverb='fire_phaser'",
    );
    assert!(parse(&source).is_err());
    let parsed = EntityConfig::from_toml(&format!("{BEAM}\n[weapons_console.ai]\nidle=true\n"));
    assert!(parsed.is_ok(), "{parsed:?}");
}

#[test]
fn generic_guided_requires_actual_payload_tuning() {
    let source = GUIDED.split("[[weapon]]").nth(1).unwrap();
    let error = parse(&format!("[[weapon]]{source}"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("requires [torpedoes]"), "{error}");
}

#[test]
fn simultaneous_bolt_volley_retains_zero_cadence() {
    let source = BOLT.replace("volley_interval_secs = 0.15", "volley_interval_secs = 0.0");
    let config = parse(&source).unwrap();
    assert_eq!(
        config.weapons_console.unwrap().blaster_banks[0].volley_interval_secs,
        0.0
    );
}

#[test]
fn sustained_operation_is_explicit_and_finite_rounds_require_actual_local_owner() {
    use phoenix_model::weapon_operation::BeamOperation;
    let fixed = parse(BEAM).unwrap();
    assert_eq!(
        fixed.weapons_console.unwrap().phaser_banks[0].operation,
        BeamOperation::Fixed
    );
    let sustained = BEAM
        .replace(
            r#"kind = "beam""#,
            "kind = \"beam\"\noperation=\"sustained\"",
        )
        .replace("cycle_jitter = 0.33", "cycle_jitter=0");
    assert_eq!(
        parse(&sustained)
            .unwrap()
            .weapons_console
            .unwrap()
            .phaser_banks[0]
            .operation,
        BeamOperation::Sustained
    );
    assert!(parse(&sustained.replace("cycle_jitter=0", "cycle_jitter=0.1")).is_err());
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/broadside_cruiser.toml").unwrap();
    validate_round_feeds(&config).unwrap();
    let bolt = &config.weapons_console.as_ref().unwrap().blaster_banks[0];
    assert_eq!(bolt.rounds.as_ref().unwrap().store, "local-kinetic-rounds");
    assert_eq!(bolt.to_runtime().rounds, bolt.rounds);
    for bad in 0..5 {
        let mut invalid = config.clone();
        let store = invalid
            .logistics
            .as_mut()
            .unwrap()
            .stores
            .iter_mut()
            .find(|s| s.id == "local-kinetic-rounds")
            .unwrap();
        match bad {
            0 => store.role = phoenix_model::logistics::StoreRole::Main,
            1 => store.system = Some(phoenix_model::identity::SystemId("phaser-aft".into())),
            2 => store.compartment = "support".into(),
            3 => store.endpoint = phoenix_model::logistics::ResourceEndpoint::TorpedoMagazine {},
            _ => {
                invalid.weapons_console.as_mut().unwrap().blaster_banks[0]
                    .rounds
                    .as_mut()
                    .unwrap()
                    .amount = 9
            }
        }
        assert!(
            validate_round_feeds(&invalid).is_err(),
            "invalid feed variant {bad}"
        );
    }
}

#[test]
fn target_policy_compiles_all_families_and_preserves_explicit_empty() {
    use phoenix_model::tags::EntityTag::Missile;
    for (source, family) in [(BEAM, 0), (BOLT, 1), (GUIDED, 2)] {
        for (authored, expected) in [
            ("targets=[\"missile\"]", Some(vec![Missile])),
            ("targets=[]", Some(vec![])),
            ("targets=false", None),
        ] {
            let value = source.replace("[weapon.ai]", &format!("{authored}\n[weapon.ai]"));
            let parsed = parse(&value).unwrap();
            let actual = match family {
                0 => parsed.weapons_console.as_ref().unwrap().phaser_banks[0]
                    .targets
                    .clone(),
                1 => parsed.weapons_console.as_ref().unwrap().blaster_banks[0]
                    .targets
                    .clone(),
                _ => parsed.torpedoes.as_ref().unwrap().tubes[0].targets.clone(),
            };
            assert_eq!(actual, expected);
            let saved = serde_json::to_string(&parsed).unwrap();
            let restored: EntityConfig = serde_json::from_str(&saved).unwrap();
            assert_eq!(parsed, restored);
        }
        for bad in [
            "targets=true",
            "targets=[\"unknown-target-tag\"]",
            "targets=[\"missile\",\"missile\"]",
        ] {
            assert!(parse(&source.replace("[weapon.ai]", &format!("{bad}\n[weapon.ai]"))).is_err());
        }
    }
}
