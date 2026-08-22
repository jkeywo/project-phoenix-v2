use super::*;
use crate::world::config::WorldEntitySpawnOn;

/// Parse a document of `[[action]]` tables into their [`TriggerAction`]s.
///
/// The tables below were `[[trigger.action]]` arrays until issue #985 deleted
/// the `[[trigger]]` container. Their SHAPE outlived it: [`RawActionEntry`] is
/// what the Rhai effect host populates from a `#{ ... }` script map before
/// running the shared [`parse_action_entry`], so every rule these tests pin —
/// the directive field ownership, the anchor/position XOR, the required
/// fields, the unknown-type refusal — is still live. It is reached from a
/// script now instead of from a trigger's action array, which is why the
/// container is what went and the table is what stayed.
fn actions(tables: &str) -> Result<Vec<TriggerAction>, String> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        action: Vec<RawActionEntry>,
    }
    let doc: Doc = toml::from_str(tables).map_err(|e| e.to_string())?;
    doc.action.iter().map(parse_action_entry).collect()
}

#[test]
fn parse_world_empty_string_returns_empty_config() {
    let cfg = parse_world("").expect("empty TOML should parse");
    assert!(cfg.anchors.is_empty());
    assert!(cfg.entities.is_empty());
    assert_eq!(cfg.global.seed, None, "an unauthored seed stays unset");
}

/// `[global] ai_tick_hz` (issues #803, #889): serde default when omitted,
/// authored value when present. The default is the one hardcoded value the
/// TOML-parse rule allows, and it matches the old `AiLateralThrustTimer`
/// period so worlds that don't author the key see no cadence change.
#[test]
fn parse_world_reads_ai_tick_hz() {
    let defaulted = parse_world("[global]\nseed = 7\n").expect("TOML should parse");
    assert_eq!(
        defaulted.global.ai_tick_hz, 30.0,
        "omitted ai_tick_hz must default to 30 Hz"
    );
    assert_eq!(
        defaulted.global.ai_snapshot_hz, 10.0,
        "omitted ai_snapshot_hz must default to the 10 Hz the retired \
         hardcoded AiSnapshotTimer ran at"
    );

    let authored = parse_world("[global]\nai_tick_hz = 20.0\n").expect("TOML should parse");
    assert_eq!(
        authored.global.ai_tick_hz, 20.0,
        "an authored ai_tick_hz must be read verbatim"
    );
}

/// `[global] intent_break_off_hull_fraction` (issue #879): the hull
/// fraction at which a seat's intent narration announces that the ship is
/// breaking off. Authored data, not a Rust literal (AGENTS.md #11) — the
/// serde default is the only sanctioned hardcoded copy of it.
#[test]
fn parse_world_reads_the_intent_break_off_hull_fraction() {
    let defaulted = parse_world("[global]\nseed = 7\n").expect("TOML should parse");
    assert_eq!(
        defaulted.global.intent_break_off_hull_fraction, 0.5,
        "an omitted intent_break_off_hull_fraction falls back to half hull"
    );

    let authored = parse_world("[global]\nintent_break_off_hull_fraction = 0.25\n")
        .expect("TOML should parse");
    assert_eq!(
        authored.global.intent_break_off_hull_fraction, 0.25,
        "an authored break-off fraction must be read verbatim — a designer \
         retunes when the crew is told the ship is pulling out without \
         touching Rust"
    );
}

/// `[global] attacked_memory_secs` (issue #1010): how long a hit keeps a
/// hull's doctrine `attacked` condition true. Authored data, not a Rust
/// literal (AGENTS.md #11) — the serde default is the only sanctioned
/// hardcoded copy of it, and it IS the shipped tuning: no world TOML
/// authors the key, exactly as with `intent_break_off_hull_fraction`.
#[test]
fn parse_world_reads_the_attacked_memory_secs() {
    let defaulted = parse_world("[global]\nseed = 7\n").expect("TOML should parse");
    assert_eq!(
        defaulted.global.attacked_memory_secs, 8.0,
        "an omitted attacked_memory_secs falls back to an 8 s reprieve"
    );

    let authored =
        parse_world("[global]\nattacked_memory_secs = 2.5\n").expect("TOML should parse");
    assert_eq!(
        authored.global.attacked_memory_secs, 2.5,
        "an authored window must be read verbatim — a designer retunes how \
         long a raid stays broken off after a hit without touching Rust"
    );
}

/// `[global] station_activity_bucket_secs` (issue #1145): how long one
/// station-activity debug bucket spans. Authored data, not a Rust literal
/// (AGENTS.md #11) — the serde default is the only sanctioned hardcoded copy,
/// and it IS the shipped tuning: no world TOML authors the key.
#[test]
fn parse_world_reads_the_station_activity_bucket_secs() {
    let defaulted = parse_world("[global]\nseed = 7\n").expect("TOML should parse");
    assert_eq!(
        defaulted.global.station_activity_bucket_secs, 15.0,
        "an omitted station_activity_bucket_secs falls back to fifteen seconds"
    );

    let authored =
        parse_world("[global]\nstation_activity_bucket_secs = 5.0\n").expect("TOML should parse");
    assert_eq!(
        authored.global.station_activity_bucket_secs, 5.0,
        "an authored bucket length must be read verbatim — a crew-control \
         designer retunes the activity chart's resolution without touching Rust"
    );
}

/// `[global] trigger_fire_history_depth` (issue #1151): how many recent fires
/// the trigger-fire-history debug recorder keeps per trigger. Authored data,
/// not a Rust literal (AGENTS.md #11) — the serde default is the only
/// sanctioned hardcoded copy, and it IS the shipped tuning.
#[test]
fn parse_world_reads_the_trigger_fire_history_depth() {
    let defaulted = parse_world("[global]\nseed = 7\n").expect("TOML should parse");
    assert_eq!(
        defaulted.global.trigger_fire_history_depth, 16,
        "an omitted trigger_fire_history_depth falls back to sixteen fires"
    );

    let authored =
        parse_world("[global]\ntrigger_fire_history_depth = 4\n").expect("TOML should parse");
    assert_eq!(
        authored.global.trigger_fire_history_depth, 4,
        "an authored ring depth must be read verbatim — a scenario author sets \
         how deep the fire history goes without touching Rust"
    );
}

/// `attacked_memory_secs` feeds straight into `now - last <
/// attacked_memory_secs` in `objectives::attacked_recently` with no
/// runtime clamp. TOML admits `nan`/`inf` as float literals, and IEEE 754
/// makes every comparison against a non-finite value false — so a NaN (or
/// infinite) window would make `attacked_recently` silently and
/// permanently read `false`, the doctrine `not_attacked` gate would never
/// close, and a raid under active fire would never break off for
/// self-defence. Non-finite is therefore a load-time rejection, the same
/// treatment `sim_tick_hz` gets above.
#[test]
fn parse_world_rejects_a_non_finite_attacked_memory_secs() {
    let err = parse_world("[global]\nattacked_memory_secs = nan\n").expect_err(
        "a NaN window must be rejected at load, not silently read as an \
                      always-open `not_attacked` gate",
    );
    assert!(
        err.contains("attacked_memory_secs") && err.contains("NaN"),
        "the load error must name the field and the authored value so the author \
         can fix it; got: {err}"
    );

    let err = parse_world("[global]\nattacked_memory_secs = inf\n")
        .expect_err("an infinite window must be rejected too");
    assert!(
        err.contains("attacked_memory_secs"),
        "the load error must name the field; got: {err}"
    );
}

/// Non-positive is a deliberate, documented authored intent —
/// `GlobalConfig::attacked_memory_secs`'s docs and
/// `objectives::attacked_recently`'s own doc comment both read a
/// non-positive window as "never counts as attacked" — not a mistake, so
/// the finite check above must not sweep it up too.
#[test]
fn parse_world_still_accepts_a_non_positive_attacked_memory_secs() {
    let zero = parse_world("[global]\nattacked_memory_secs = 0.0\n").expect(
        "a zero window is the documented \"never attacked\" authoring, not \
                 a load error",
    );
    assert_eq!(zero.global.attacked_memory_secs, 0.0);

    let negative = parse_world("[global]\nattacked_memory_secs = -1.0\n").expect(
        "a negative window is likewise the documented \"never attacked\" \
                 authoring, not a load error",
    );
    assert_eq!(negative.global.attacked_memory_secs, -1.0);
}

#[test]
fn parse_world_preserves_hull_agnostic_scenario_detail_floor_vocabulary() {
    let parsed = parse_world(
        r#"
scenario_detail_floor = ["navigation", "sensors"]
[global]
title = "Floor fixture"
"#,
    )
    .expect("scenario detail-floor vocabulary is valid world TOML");
    assert_eq!(
        parsed.scenario_detail_floor,
        vec!["navigation".to_string(), "sensors".to_string()]
    );
}

/// Every shipped world TOML authors the pre-#889 key. Promoting the field
/// to `ai_tick_hz` must not silently drop those authored rates back to the
/// serde default — the old key stays a serde alias.
#[test]
fn parse_world_still_reads_the_legacy_ai_helm_tick_hz_key() {
    let legacy = parse_world("[global]\nai_helm_tick_hz = 60.0\n").expect("TOML should parse");
    assert_eq!(
        legacy.global.ai_tick_hz, 60.0,
        "the pre-#889 `ai_helm_tick_hz` key must still set the shared AI \
         tick rate — every shipped world authors it"
    );
}

/// Issue #889: the slower AI cadence is DERIVED from the base tick as a
/// whole number of base ticks, so an authored pair that does not divide is
/// a content error. Before this, `ai_helm_tick_hz = 25` against the
/// hardcoded 10 Hz snapshot timer gave 2.5 snapshot ticks per helm tick
/// with nothing complaining.
#[test]
fn parse_world_rejects_a_non_integer_cadence_relationship() {
    let err = parse_world("[global]\nai_tick_hz = 25.0\n")
        .expect_err("25 Hz base against the 10 Hz default is 2.5 base ticks per snapshot");
    assert!(
        err.contains("ai_tick_hz") && err.contains("ai_snapshot_hz"),
        "the load error must name both authored rates so the author can fix \
         the pair; got: {err}"
    );

    // `sim_tick_hz = 50` keeps the OTHER commensurability contract (the
    // #895 sim/ai one below) satisfied: 50 / 25 = 2 sim ticks per AI tick.
    parse_world("[global]\nsim_tick_hz = 50.0\nai_tick_hz = 25.0\nai_snapshot_hz = 5.0\n")
        .expect("25 Hz base against 5 Hz snapshot divides exactly (5 base ticks)");
}

/// Issue #895: the AI decision tick is derived from the LOGICAL SIM tick
/// by counting, so `sim_tick_hz / ai_tick_hz` must be a positive integer —
/// the same contract, one level up. The default 60 Hz sim tick against the
/// default 30 Hz AI tick is 2:1; an authored pair that does not divide is
/// a content error at world load, not something to round at runtime.
#[test]
fn parse_world_rejects_a_sim_tick_the_ai_tick_does_not_divide() {
    let defaulted = parse_world("[global]\nseed = 7\n").expect("TOML should parse");
    assert_eq!(
        defaulted.global.sim_tick_hz, 60.0,
        "omitted sim_tick_hz must default to the 60 Hz the frame-driven \
         browser host effectively ran at"
    );
    assert_eq!(
        defaulted.global.sim_ticks_per_ai_tick(),
        2,
        "60 Hz sim against 30 Hz AI is two sim ticks per decision"
    );

    let err = parse_world("[global]\nsim_tick_hz = 50.0\n")
        .expect_err("50 Hz sim against the 30 Hz default AI tick is 1.67 ticks");
    assert!(
        err.contains("sim_tick_hz") && err.contains("ai_tick_hz"),
        "the load error must name both authored rates so the author can fix \
         the pair; got: {err}"
    );

    let authored = parse_world("[global]\nsim_tick_hz = 120.0\nai_tick_hz = 30.0\n")
        .expect("120 Hz sim against 30 Hz AI divides exactly (4 sim ticks)");
    assert_eq!(authored.global.sim_tick_hz, 120.0);
    assert_eq!(authored.global.sim_ticks_per_ai_tick(), 4);
}

/// Issue #895: a logical tick slower than the helm integrator's
/// `HELM_AI_MAX_DT_SECS` cap would be silently shortened by it — the sim
/// under-integrates and two hosts on different rates diverge. So the floor
/// is a load-time rejection, not a runtime clamp, and 30 Hz itself (the
/// cap's own rate) is still legal.
#[test]
fn parse_world_rejects_a_sim_tick_below_the_integrator_floor() {
    // Rates chosen so BOTH commensurability contracts hold (20/10 = 2
    // snapshot ticks, 20/20 = 1 sim tick per AI tick): the only thing
    // wrong with this world is that it is too slow.
    let err = parse_world("[global]\nsim_tick_hz = 20.0\nai_tick_hz = 20.0\n")
        .expect_err("20 Hz is below the 30 Hz integrator floor");
    assert!(
        err.contains("sim_tick_hz") && err.contains("20"),
        "the load error must name the authored rate so the author can fix \
         it; got: {err}"
    );

    // Commensurate with the default 30 Hz AI tick AND exactly on the
    // floor: the boundary is inclusive, or the shipped `HELM_AI_MAX_DT_SECS`
    // rate would itself be unauthorable.
    let at_the_floor = parse_world("[global]\nsim_tick_hz = 30.0\n")
        .expect("30 Hz sits exactly on the floor and must be accepted");
    assert_eq!(at_the_floor.global.sim_tick_hz, 30.0);
    assert_eq!(at_the_floor.global.sim_ticks_per_ai_tick(), 1);
}

/// Re-review of issue #895: the floor above had no matching ceiling, so
/// `sim_tick_hz = 100000` loaded and wedged the host — `max_delta /
/// timestep` `FixedUpdate` steps trying to run inside a single frame
/// (~25 000 of them at that rate under the 250 ms default `max_delta`).
/// So the ceiling is a load-time rejection too, and 240 Hz itself is
/// still legal.
#[test]
fn parse_world_rejects_a_sim_tick_above_the_ceiling() {
    // ai_tick_hz left at its 30 Hz default: 480 / 30 = 16, a whole
    // number, so the only thing wrong with this world is that the sim
    // tick itself is too fast.
    let err = parse_world("[global]\nsim_tick_hz = 480.0\n")
        .expect_err("480 Hz is above the 240 Hz ceiling");
    assert!(
        err.contains("sim_tick_hz") && err.contains("480"),
        "the load error must name the authored rate so the author can fix \
         it; got: {err}"
    );

    // Commensurate with the default 30 Hz AI tick AND exactly on the
    // ceiling: the boundary is inclusive.
    let at_the_ceiling = parse_world("[global]\nsim_tick_hz = 240.0\n")
        .expect("240 Hz sits exactly on the ceiling and must be accepted");
    assert_eq!(at_the_ceiling.global.sim_tick_hz, 240.0);
    assert_eq!(at_the_ceiling.global.sim_ticks_per_ai_tick(), 8);
}

#[test]
fn parse_world_reads_dust_layers_and_warp() {
    let toml = r#"
[dust]
enabled = true
speed_curve_exponent = 2.0
low_speed_tint = [0.55, 0.65, 0.75]
high_speed_tint = [0.95, 0.98, 1.0]
turbulence = 0.05

[[dust.layer]]
name = "near"
texture = "pfx/space_mote_streak_head.png"
max_motes = 24
spawn_rate = [0.0, 12.0]
length = [1.0, 20.0]
additive = true

[[dust.layer]]
name = "far"
texture = "pfx/space_mote_compact_core.png"
max_motes = 200
spawn_rate = [10.0, 250.0]
additive = false

[dust.warp]
enabled = true
texture = "pfx/space_mote_streak_soft.png"
exit_secs = 0.6
"#;
    let cfg = parse_world(toml).expect("must parse");
    let dust = cfg.dust.expect("[dust] must be present");

    assert_eq!(dust.enabled, Some(true));
    assert_eq!(dust.speed_curve_exponent, Some(2.0));
    assert_eq!(dust.low_speed_tint, Some([0.55, 0.65, 0.75]));
    assert_eq!(dust.turbulence, Some(0.05));

    // `[[dust.layer]]` maps onto the renamed `layers` field, in file order.
    assert_eq!(dust.layers.len(), 2);
    assert_eq!(dust.layers[0].name.as_deref(), Some("near"));
    assert_eq!(dust.layers[0].spawn_rate, Some([0.0, 12.0]));
    assert_eq!(dust.layers[0].length, Some([1.0, 20.0]));
    assert_eq!(dust.layers[0].additive, Some(true));
    assert_eq!(dust.layers[1].name.as_deref(), Some("far"));
    assert_eq!(dust.layers[1].additive, Some(false));

    // Unset fields stay None so the renderer's own defaults win.
    assert_eq!(dust.layers[1].length, None);
    assert_eq!(dust.centre_fade_inner, None);

    let warp = dust.warp.expect("[dust.warp] must be present");
    assert_eq!(warp.enabled, Some(true));
    assert_eq!(warp.exit_secs, Some(0.6));
    assert_eq!(warp.enter_secs, None);
}

#[test]
fn parse_world_dust_absent_is_none() {
    let cfg = parse_world("").expect("empty TOML should parse");
    assert!(cfg.dust.is_none());
}

/// The shipped worlds are what actually reach the renderer, so parse the
/// real files rather than a fixture — a stale `[dust]` key here would
/// otherwise fail silently at runtime rather than in CI.
#[test]
fn shipped_default_world_dust_parses() {
    let cfg = parse_world(include_str!("../../assets/worlds/default.toml"))
        .expect("default.toml must parse");
    let dust = cfg.dust.expect("default.toml declares [dust]");
    assert_eq!(dust.enabled, Some(true));
    assert!(
        dust.layers.is_empty(),
        "default world should ride the built-in layers"
    );
    assert_eq!(
        dust.warp
            .expect("default.toml declares [dust.warp]")
            .enabled,
        Some(true)
    );
}

#[test]
fn shipped_combat_test_world_dust_parses_with_tuned_layers() {
    let cfg = parse_world(include_str!("../../assets/worlds/combat_test.toml"))
        .expect("combat_test.toml must parse");
    let dust = cfg.dust.expect("combat_test.toml declares [dust]");
    assert_eq!(dust.layers.len(), 3, "near/mid/far must all be declared");
    assert_eq!(dust.layers[0].name.as_deref(), Some("near"));
    assert_eq!(dust.layers[0].max_motes, Some(32));
    assert_eq!(dust.layers[2].name.as_deref(), Some("far"));
    // Layers are matched to built-in defaults by position, so ordering is
    // load-bearing: near must come first and far last.
    let near_depth = dust.layers[0].depth_band.expect("near sets depth_band");
    let far_depth = dust.layers[2].depth_band.expect("far sets depth_band");
    assert!(
        near_depth[0] < far_depth[0],
        "layers must be authored near→far, got {near_depth:?} then {far_depth:?}"
    );
}

#[test]
fn parse_world_reads_anchors_as_three_element_arrays() {
    let toml = r#"
[anchors]
alpha = [10.0, 0.0, 20.0]
beta  = [-5.0, 1.5, 30.0]
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.anchors.len(), 2);
    assert_eq!(cfg.anchors.get("alpha"), Some(&[10.0, 0.0, 20.0]));
    assert_eq!(cfg.anchors.get("beta"), Some(&[-5.0, 1.5, 30.0]));
}

#[test]
fn parse_world_widens_two_element_anchor_to_three() {
    // Historic AI code widened 2-element anchors by inserting 0.0 at Y.
    let toml = r#"
[anchors]
flat = [100.0, 200.0]
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.anchors.get("flat"), Some(&[100.0, 0.0, 200.0]));
}

#[test]
fn parse_world_rejects_one_element_anchor() {
    let toml = r#"
[anchors]
busted = [1.0]
"#;
    let err = parse_world(toml).expect_err("one-element anchor must error");
    assert!(
        err.contains("busted"),
        "error must mention anchor name: {err}"
    );
}

#[test]
fn parse_world_reads_entity_blocks_with_template_path_and_position() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
transform = { position = [100.0, 0.0, -200.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.entities.len(), 2);
    assert_eq!(
        cfg.entities[0].template_path,
        "assets/entities/star_sun.toml"
    );
    assert_eq!(
        cfg.entities[1].template_path,
        "assets/entities/asteroid_field_main.toml"
    );
    assert_eq!(
        cfg.entities[1].transform.as_ref().and_then(|t| t.position),
        Some([100.0, 0.0, -200.0])
    );
}

#[test]
fn parse_world_entity_spawn_on_defaults_to_immediate() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.entities[0].spawn_on, WorldEntitySpawnOn::Immediate);
}

#[test]
fn world_config_default_has_empty_name_to_uuid() {
    // PRD #337/#339 slice 2: the unified WorldConfig owns the
    // `name ? uuid` map that `spawn_world_entities` populates and
    // trigger/comms lookup reads. Starts empty.
    let cfg = WorldConfig::default();
    assert!(cfg.name_to_uuid.is_empty());
    assert_eq!(cfg.name_to_uuid.len(), 0);
}

#[test]
fn assign_named_entity_uuids_collects_named_only_with_stable_uuids() {
    // PRD #337/#339 slice 2: a pure helper builds the `name ? uuid`
    // map from a slice of WorldEntity. Anonymous entries are skipped;
    // a caller-supplied generator yields the UUIDs (so tests can be
    // deterministic without dragging real RNG in).
    let entities = vec![
        WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            ..Default::default()
        },
        WorldEntity {
            template_path: "assets/entities/star_sun.toml".into(),
            name: None,
            ..Default::default()
        },
        WorldEntity {
            template_path: "assets/entities/planet_earth.toml".into(),
            name: Some("earth".into()),
            ..Default::default()
        },
    ];
    let mut counter = 0u32;
    let map = assign_named_entity_uuids(&entities, || {
        counter += 1;
        format!("uuid-{counter}")
    });
    assert_eq!(map.len(), 2, "only named entities get a uuid");
    assert_eq!(
        map.get("starbase_alpha").map(String::as_str),
        Some("uuid-1")
    );
    assert_eq!(map.get("earth").map(String::as_str), Some("uuid-2"));
}

#[test]
fn is_owned_by_unified_pipeline_routes_asteroid_fields_and_named_entries() {
    // The complementary `setup_world` path in `server_app.rs` must skip
    // both asteroid-field templates AND any entry carrying a `name` field
    // (owned by `spawn_world_entities`).
    let asteroid = WorldEntity {
        template_path: "assets/entities/asteroid_field_dense.toml".into(),
        ..Default::default()
    };
    let named = WorldEntity {
        template_path: "assets/entities/station_outpost.toml".into(),
        name: Some("starbase_alpha".into()),
        ..Default::default()
    };
    let anon = WorldEntity {
        template_path: "assets/entities/star_sun.toml".into(),
        ..Default::default()
    };

    let is_field = |p: &str| p.contains("asteroid_field");
    assert!(is_owned_by_unified_pipeline(&asteroid, is_field));
    assert!(is_owned_by_unified_pipeline(&named, is_field));
    assert!(
        !is_owned_by_unified_pipeline(&anon, is_field),
        "anonymous non-asteroid entries stay on the legacy path"
    );
}

#[test]
fn partition_immediate_entities_three_buckets_separates_fields_named_anonymous() {
    // PRD #339 slice 2: named non-asteroid entries are owned by the
    // unified pipeline (and must be spawned by it). The partition
    // helper now produces three buckets so `spawn_world_entities` can
    // iterate both fields AND named entries while the complementary
    // `setup_world` in `server_app.rs` keeps anonymous ones.
    let mut cfg = WorldConfig::default();
    cfg.entities.push(WorldEntity {
        template_path: "assets/entities/asteroid_field_main.toml".into(),
        ..Default::default()
    });
    cfg.entities.push(WorldEntity {
        template_path: "assets/entities/station_outpost.toml".into(),
        name: Some("starbase_alpha".into()),
        ..Default::default()
    });
    cfg.entities.push(WorldEntity {
        template_path: "assets/entities/star_sun.toml".into(),
        ..Default::default()
    });
    // game_start entries are in no bucket
    cfg.entities.push(WorldEntity {
        template_path: "assets/entities/alliance_cruiser.toml".into(),
        spawn_on: crate::world::config::WorldEntitySpawnOn::GameStart,
        ..Default::default()
    });

    let is_field = |p: &str| p.contains("asteroid_field");
    let (fields, named, anon) = partition_immediate_entities_three_way(&cfg, is_field);

    assert_eq!(fields.len(), 1);
    assert_eq!(named.len(), 1);
    assert_eq!(named[0].name.as_deref(), Some("starbase_alpha"));
    assert_eq!(anon.len(), 1);
    assert_eq!(anon[0].template_path, "assets/entities/star_sun.toml");
}

#[test]
fn parse_world_entity_accepts_optional_name_field() {
    // PRD #337/#339 slice 2: named [[entity]] blocks become the unified
    // replacement for [[spawn]] — they get a UUID at spawn time and
    // become eligible for trigger / comms lookups.
    let toml = r#"
[[entity]]
template_path = "assets/entities/station_outpost.toml"
name = "Starbase Alpha"
transform = { position = [500.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.entities.len(), 2);
    assert_eq!(cfg.entities[0].name.as_deref(), Some("Starbase Alpha"));
    assert_eq!(
        cfg.entities[1].name, None,
        "entity without a name field must deserialize as None"
    );
}

#[test]
fn parse_world_entity_accepts_anchor_field() {
    // PRD #337 slice 3: `[[entity]]` now supports `anchor = "..."` so NPC
    // patrols (formerly `[[spawn]]`) can be migrated into the unified
    // pipeline without inlining anchor coordinates.
    let toml = r#"
[anchors]
patrol_alpha = [300.0, 0.0, -300.0]

[[entity]]
template_path = "assets/entities/ship_harrow_destroyer.toml"
name = "raider_alpha"
transform = { anchor = "patrol_alpha" }
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.entities.len(), 1);
    assert_eq!(
        cfg.entities[0]
            .transform
            .as_ref()
            .and_then(|t| t.anchor.as_deref()),
        Some("patrol_alpha")
    );
    assert!(
        cfg.entities[0]
            .transform
            .as_ref()
            .and_then(|t| t.position)
            .is_none(),
        "no inline position when anchor is supplied"
    );
}

// -- available_ships (issue #623) ---------------------------------------

#[test]
fn parse_world_reads_available_ships() {
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/ship_scout.toml"
label = "Scout"

[[available_ships]]
template_path = "assets/entities/ship_cruiser.toml"
label = "Cruiser"
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.available_ships.len(), 2);
    assert_eq!(
        cfg.available_ships[0].template_path,
        "assets/entities/ship_scout.toml"
    );
    assert_eq!(cfg.available_ships[0].label.as_deref(), Some("Scout"));
    assert_eq!(
        cfg.available_ships[1].template_path,
        "assets/entities/ship_cruiser.toml"
    );
    assert_eq!(cfg.available_ships[1].label.as_deref(), Some("Cruiser"));
}

#[test]
fn parse_world_available_ships_defaults_to_empty() {
    let cfg = parse_world("").expect("must parse");
    assert!(cfg.available_ships.is_empty());
}

#[test]
fn parse_world_available_ships_optional_label() {
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/ship_scout.toml"
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.available_ships.len(), 1);
    assert!(cfg.available_ships[0].label.is_none());
}

// -- player_spawn (issue #623) -------------------------------------------

#[test]
fn parse_world_reads_player_spawn_position() {
    let toml = r#"
[player_spawn]
position = [100.0, 0.0, 200.0]
"#;
    let cfg = parse_world(toml).expect("must parse");
    let spawn = cfg.player_spawn.expect("player_spawn must be Some");
    assert_eq!(spawn.position, Some([100.0, 0.0, 200.0]));
    assert!(spawn.anchor.is_none());
    assert!(spawn.rotation.is_none());
}

#[test]
fn parse_world_reads_player_spawn_anchor() {
    let toml = r#"
[anchors]
spawn_point = [50.0, 0.0, 0.0]

[player_spawn]
anchor = "spawn_point"
"#;
    let cfg = parse_world(toml).expect("must parse");
    let spawn = cfg.player_spawn.expect("player_spawn must be Some");
    assert_eq!(spawn.anchor.as_deref(), Some("spawn_point"));
    assert!(spawn.position.is_none());
}

#[test]
fn parse_world_player_spawn_defaults_to_none() {
    let cfg = parse_world("").expect("must parse");
    assert!(cfg.player_spawn.is_none());
}

#[test]
fn parse_world_no_available_ships_yields_empty() {
    // World without [[available_ships]] should remain empty.
    let toml = r#"
[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert!(cfg.available_ships.is_empty());
}

#[test]
fn parse_world_explicit_available_ships_works() {
    // World with explicit [[available_ships]].
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/ship_scout.toml"
label = "Scout"

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
spawn_on = "game_start"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.available_ships.len(), 1);
    assert_eq!(
        cfg.available_ships[0].template_path,
        "assets/entities/ship_scout.toml"
    );
}

// -- entity_template_paths + available_ships (issue #623) ----------------

#[test]
fn entity_template_paths_includes_available_ship_templates() {
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/ship_scout.toml"
label = "Scout"

[[available_ships]]
template_path = "assets/entities/ship_cruiser.toml"
label = "Cruiser"

[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let paths = entity_template_paths(&cfg, &[]);
    assert!(paths.contains(&"assets/entities/ship_scout.toml".to_string()));
    assert!(paths.contains(&"assets/entities/ship_cruiser.toml".to_string()));
    assert!(paths.contains(&"assets/entities/star_sun.toml".to_string()));
}

#[test]
fn entity_template_paths_dedups_available_ships_with_entity_list() {
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"
label = "Alliance Cruiser"

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
spawn_on = "game_start"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let paths = entity_template_paths(&cfg, &[]);
    let count = paths
        .iter()
        .filter(|p| *p == "assets/entities/alliance_cruiser.toml")
        .count();
    assert_eq!(
        count, 1,
        "duplicate ship path must be collapsed to one entry"
    );
}

#[test]
fn entity_template_paths_filters_available_ships_to_curated_allowlist() {
    // Issue #917: a non-empty curated allowlist restricts which
    // [[available_ships]] hulls get queued for preload — the BLOCKER this
    // fixes is that world preload used to ignore curation entirely and
    // fetch every hull regardless.
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/ship_scout.toml"
label = "Scout"

[[available_ships]]
template_path = "assets/entities/ship_cruiser.toml"
label = "Cruiser"
"#;
    let cfg = parse_world(toml).expect("must parse");
    let curated = vec!["assets/entities/ship_cruiser.toml".to_string()];
    let paths = entity_template_paths(&cfg, &curated);
    assert_eq!(paths, vec!["assets/entities/ship_cruiser.toml".to_string()]);
}

#[test]
fn entity_template_paths_empty_curated_list_is_unrestricted() {
    // Empty curation (the default, and what the `?scenario=<path>` dev
    // bypass always passes since it never resolves a catalog entry) must
    // preload every hull the world offers — byte-identical to the
    // pre-#917 caller with no allowlist parameter at all.
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/ship_scout.toml"
label = "Scout"

[[available_ships]]
template_path = "assets/entities/ship_cruiser.toml"
label = "Cruiser"
"#;
    let cfg = parse_world(toml).expect("must parse");
    let paths = entity_template_paths(&cfg, &[]);
    assert_eq!(
        paths,
        vec![
            "assets/entities/ship_scout.toml".to_string(),
            "assets/entities/ship_cruiser.toml".to_string(),
        ]
    );
}

#[test]
fn entity_template_paths_curation_only_restricts_available_ships() {
    // A curated allowlist scopes [[available_ships]] only — static
    // [[entity]] instances (scenery, NPCs) are unaffected and always
    // preload, regardless of which hull the player may fly.
    let toml = r#"
[[available_ships]]
template_path = "assets/entities/ship_scout.toml"

[[available_ships]]
template_path = "assets/entities/ship_cruiser.toml"

[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let curated = vec!["assets/entities/ship_scout.toml".to_string()];
    let paths = entity_template_paths(&cfg, &curated);
    assert!(paths.contains(&"assets/entities/ship_scout.toml".to_string()));
    assert!(!paths.contains(&"assets/entities/ship_cruiser.toml".to_string()));
    assert!(paths.contains(&"assets/entities/star_sun.toml".to_string()));
}

// -- resolve_entity_position (PRD #337 slice 3) ------------------------

fn anchor_table(entries: &[(&str, [f32; 3])]) -> HashMap<String, [f32; 3]> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), *v))
        .collect()
}

fn entity_with(template_path: &str, xf: TransformConfig) -> WorldEntity {
    WorldEntity {
        template_path: template_path.into(),
        transform: Some(xf),
        ..Default::default()
    }
}

#[test]
fn resolve_entity_position_uses_anchor_when_set() {
    let entity = entity_with(
        "assets/entities/ship_harrow_destroyer.toml",
        TransformConfig {
            anchor: Some("patrol_alpha".into()),
            ..Default::default()
        },
    );
    let anchors = anchor_table(&[("patrol_alpha", [300.0, 0.0, -300.0])]);
    let pos = resolve_entity_position(&entity, &anchors).unwrap();
    assert_eq!(pos, [300.0, 0.0, -300.0]);
}

#[test]
fn resolve_entity_position_falls_back_to_inline_position() {
    let entity = entity_with(
        "assets/entities/star_sun.toml",
        TransformConfig {
            position: Some([10.0, 0.0, 20.0]),
            ..Default::default()
        },
    );
    let pos = resolve_entity_position(&entity, &HashMap::new()).unwrap();
    assert_eq!(pos, [10.0, 0.0, 20.0]);
}

#[test]
fn resolve_entity_position_errors_on_unknown_anchor() {
    let entity = entity_with(
        "assets/entities/ship_harrow_destroyer.toml",
        TransformConfig {
            anchor: Some("ghost".into()),
            ..Default::default()
        },
    );
    let err = resolve_entity_position(&entity, &HashMap::new()).unwrap_err();
    assert!(
        err.contains("ghost"),
        "error must mention missing anchor: {err}"
    );
}

#[test]
fn resolve_entity_position_anchor_wins_over_inline_position() {
    let entity = entity_with(
        "x.toml",
        TransformConfig {
            anchor: Some("a".into()),
            position: Some([999.0, 999.0, 999.0]),
            ..Default::default()
        },
    );
    let anchors = anchor_table(&[("a", [1.0, 2.0, 3.0])]);
    let pos = resolve_entity_position(&entity, &anchors).unwrap();
    assert_eq!(pos, [1.0, 2.0, 3.0]);
}

// -- relative_to + offset (PRD #337 — closing AC) ----------------------

fn resolved_table(entries: &[(&str, [f32; 3])]) -> HashMap<String, [f32; 3]> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), *v))
        .collect()
}

#[test]
fn resolve_entity_position_relative_to_adds_offset_to_referenced_entity() {
    let entity = entity_with(
        "assets/entities/ship_harrow_destroyer.toml",
        TransformConfig {
            relative_to: Some("starbase_alpha".into()),
            offset: Some([10.0, 0.0, -5.0]),
            ..Default::default()
        },
    );
    let resolved = resolved_table(&[("starbase_alpha", [100.0, 0.0, 200.0])]);
    let pos = resolve_entity_position_with(&entity, &HashMap::new(), &resolved).unwrap();
    assert_eq!(pos, [110.0, 0.0, 195.0]);
}

#[test]
fn resolve_entity_position_relative_to_with_missing_offset_uses_zero() {
    let entity = entity_with(
        "x.toml",
        TransformConfig {
            relative_to: Some("origin".into()),
            ..Default::default()
        },
    );
    let resolved = resolved_table(&[("origin", [5.0, 6.0, 7.0])]);
    let pos = resolve_entity_position_with(&entity, &HashMap::new(), &resolved).unwrap();
    assert_eq!(pos, [5.0, 6.0, 7.0]);
}

#[test]
fn resolve_entity_position_relative_to_errors_on_unknown_reference() {
    let entity = entity_with(
        "x.toml",
        TransformConfig {
            relative_to: Some("ghost".into()),
            ..Default::default()
        },
    );
    let err = resolve_entity_position_with(&entity, &HashMap::new(), &HashMap::new()).unwrap_err();
    assert!(
        err.contains("ghost") && err.contains("relative_to"),
        "error must mention missing reference and relative_to: {err}"
    );
}

#[test]
fn resolve_entity_position_relative_to_wins_over_anchor_and_position() {
    let entity = entity_with(
        "x.toml",
        TransformConfig {
            anchor: Some("a".into()),
            position: Some([999.0, 999.0, 999.0]),
            relative_to: Some("base".into()),
            offset: Some([1.0, 1.0, 1.0]),
            ..Default::default()
        },
    );
    let anchors = anchor_table(&[("a", [50.0, 50.0, 50.0])]);
    let resolved = resolved_table(&[("base", [10.0, 0.0, 0.0])]);
    let pos = resolve_entity_position_with(&entity, &anchors, &resolved).unwrap();
    assert_eq!(pos, [11.0, 1.0, 1.0]);
}

#[test]
fn parse_world_accepts_relative_to_and_offset_on_entity() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/starbase_alpha.toml"
name          = "starbase_alpha"
transform     = { position = [100.0, 0.0, 200.0] }

[[entity]]
template_path = "assets/entities/ship_harrow_destroyer.toml"
transform     = { relative_to = "starbase_alpha", offset = [10.0, 0.0, -5.0] }
"#;
    let world = parse_world(toml).expect("parse");
    assert_eq!(world.entities.len(), 2);
    let raider = &world.entities[1];
    let xf = raider.transform.as_ref().expect("transform present");
    assert_eq!(xf.relative_to.as_deref(), Some("starbase_alpha"));
    assert_eq!(xf.offset, Some([10.0, 0.0, -5.0]));
}

// -- relative_to resolves against `id` as well as `name` (issue #969) ----

/// The defect proper. `combat_test.toml` authors its ice moon
/// `relative_to = "gas-giant"` — the gas giant's `id`, since the
/// localisation pass moved every landmark `name` to a strings.csv key. When
/// only `name` keyed the lookup table the reference matched nothing, the
/// spawn loop logged and `continue`d, and the moon was simply never in the
/// scenario. Asserts the shipped world, not a fixture shaped like it.
#[test]
fn combat_test_ice_moon_resolves_against_the_gas_giants_authored_id() {
    let world = parse_world(include_str!("../../assets/worlds/combat_test.toml"))
        .expect("shipped world parses");
    let table = build_named_entity_positions(&world);
    let moon = world
        .entities
        .iter()
        .find(|e| e.id.as_deref() == Some("ice-moon"))
        .expect("combat_test authors an ice moon");
    let pos = resolve_entity_position_with(moon, &world.anchors, &table)
        .expect("the ice moon must resolve against the gas giant");
    // gas-giant [-1200, 0, 300] + offset [125, 0, 40]
    assert_eq!(pos, [-1075.0, 0.0, 340.0]);
}

/// The same defect in the other shipped world: `default.toml`'s luna is
/// `relative_to = "earth"`, and earth's `name` is
/// `world.entity.earth.name`.
#[test]
fn default_world_luna_resolves_against_earths_authored_id() {
    let world = parse_world(include_str!("../../assets/worlds/default.toml"))
        .expect("shipped world parses");
    let table = build_named_entity_positions(&world);
    let luna = world
        .entities
        .iter()
        .find(|e| e.id.as_deref() == Some("luna"))
        .expect("default authors luna");
    let pos = resolve_entity_position_with(luna, &world.anchors, &table)
        .expect("luna must resolve against earth");
    // earth [400, 0, 400] + offset [60, 0, 30]
    assert_eq!(pos, [460.0, 0.0, 430.0]);
}

/// `name` still resolves — the documented reference id keeps working — and
/// wins over an `id` of the same spelling on a *different* entity, since
/// `name` is the reference id proper and `id` only an accepted alias.
///
/// The `name`-bearing entity is declared **first** on purpose. Precedence
/// has to come from the two keying passes, not from file order: a single
/// interleaved pass gives the spelling to whichever block is last, which
/// would pass this assertion with the blocks the other way round and fail
/// it as written.
#[test]
fn build_named_entity_positions_keys_both_id_and_name_with_name_winning() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "earth"
name = "decoy"
transform = { position = [2.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/station_axiom.toml"
id = "decoy"
transform = { position = [1.0, 0.0, 0.0] }
"#;
    let world = parse_world(toml).expect("parse");
    let table = build_named_entity_positions(&world);
    assert_eq!(table.get("earth"), Some(&[2.0, 0.0, 0.0]));
    assert_eq!(
        table.get("decoy"),
        Some(&[2.0, 0.0, 0.0]),
        "a `name` must win a collision with another entity's `id` even when \
         the `id` is declared later"
    );
}

/// The mirror image, so neither ordering is left to chance: the `id`-only
/// entity first, the `name`-bearing one second. Both orderings must land on
/// the `name` holder.
#[test]
fn a_name_outranks_an_earlier_entitys_id_of_the_same_spelling() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/station_axiom.toml"
id = "decoy"
transform = { position = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "earth"
name = "decoy"
transform = { position = [2.0, 0.0, 0.0] }
"#;
    let world = parse_world(toml).expect("parse");
    let table = build_named_entity_positions(&world);
    assert_eq!(
        table.get("decoy"),
        Some(&[2.0, 0.0, 0.0]),
        "a `name` must win a collision with another entity's `id` when the \
         `id` is declared earlier too"
    );
}

/// Both directions. The lookup table is built over every `[[entity]]`
/// before anything is positioned, so a reference resolves whether its
/// target sits above it or below it in the file — the single-pass trap the
/// old error message ("previously-declared") implied but the code never
/// actually had.
#[test]
fn relative_to_resolves_a_target_declared_earlier_or_later_in_the_file() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "forward-moon"
transform = { relative_to = "planet", offset = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "planet"
transform = { position = [100.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_ice.toml"
id = "backward-moon"
transform = { relative_to = "planet", offset = [0.0, 0.0, 5.0] }
"#;
    let world = parse_world(toml).expect("parse");
    let table = build_named_entity_positions(&world);
    let at = |id: &str| {
        let e = world
            .entities
            .iter()
            .find(|e| e.id.as_deref() == Some(id))
            .expect("entity present");
        resolve_entity_position_with(e, &world.anchors, &table).expect("resolves")
    };
    assert_eq!(at("forward-moon"), [101.0, 0.0, 0.0], "declared later");
    assert_eq!(at("backward-moon"), [100.0, 0.0, 5.0], "declared earlier");
}

/// A `relative_to`-positioned entity is still not a valid base — chains
/// stay unsupported, and the table must not gain one via the `id` key.
#[test]
fn build_named_entity_positions_excludes_relative_to_positioned_entities() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "planet"
transform = { position = [100.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "moon"
transform = { relative_to = "planet", offset = [1.0, 0.0, 0.0] }
"#;
    let world = parse_world(toml).expect("parse");
    let table = build_named_entity_positions(&world);
    assert!(table.contains_key("planet"));
    assert!(
        !table.contains_key("moon"),
        "a relative_to-positioned entity is not a base for further lookups"
    );
}

#[test]
fn parse_world_entity_spawn_on_game_start_recognised() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.entities[0].spawn_on, WorldEntitySpawnOn::GameStart);
    assert_eq!(cfg.entities[0].id.as_deref(), Some("player-ship"));
}

// -- Triggers & comms (PRD #341) ---------------------------------------

// ── on_all_destroyed parser ────────────────────────────────────────────

#[test]
fn add_objective_reads_targets() {
    let toml = r#"
[[action]]
type      = "add_objective"
id        = "obj-hail-briefing"
text      = "Hail Axiom Station or Research Outpost."
targets   = ["Axiom Station", "Research Outpost"]
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::AddObjective { targets, .. } => {
            assert_eq!(
                targets,
                &vec!["Axiom Station".to_string(), "Research Outpost".to_string()]
            );
        }
        other => panic!("expected AddObjective, got {other:?}"),
    }
}

#[test]
fn add_objective_targets_default_empty() {
    let toml = r#"
[[action]]
type = "add_objective"
id   = "obj-x"
text = "Do the thing."
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::AddObjective { targets, .. } => assert!(targets.is_empty()),
        other => panic!("expected AddObjective, got {other:?}"),
    }
}

/// `Retreat` is ordinary authored doctrine since #702 (hull TOMLs accept
/// `directive_kind = "Retreat"`), so a mission objective must be able to
/// author it too — same shape as `Reach`: a required `directive_anchor`.
#[test]
fn add_objective_reads_a_retreat_directive() {
    let toml = r#"
[[action]]
type             = "add_objective"
id               = "obj-retreat"
text             = "Fall back to the haven."
directive_kind   = "Retreat"
directive_anchor = "pirate_haven"
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::AddObjective { directive, .. } => {
            assert_eq!(
                *directive,
                crate::core::messages::AiDirective::Retreat {
                    anchor: "pirate_haven".into(),
                }
            );
        }
        other => panic!("expected AddObjective, got {other:?}"),
    }
}

#[test]
fn retreat_directive_requires_directive_anchor() {
    let toml = r#"
[[action]]
type           = "add_objective"
id             = "obj-retreat"
text           = "Fall back."
directive_kind = "Retreat"
"#;
    let err = actions(toml).expect_err("Retreat without an anchor must be rejected");
    assert!(
        err.contains("Retreat") && err.contains("directive_anchor"),
        "error must name the directive and the missing field, got: {err}"
    );
}

// ── add_objective: a field belonging to another directive kind ─────────

/// The Requiem Courier bug, authored on the mission side: `Reach` reads the
/// singular `directive_anchor`, so the plural Patrol field does nothing at
/// all and says nothing about it. Rejected at parse rather than activating
/// an objective that can never fire.
#[test]
fn patrol_anchors_on_a_reach_objective_are_rejected() {
    let toml = r#"
[[action]]
type              = "add_objective"
id                = "obj-reach"
text              = "Make the rendezvous."
directive_kind    = "Reach"
directive_anchors = ["rendezvous"]
"#;
    let err = actions(toml).expect_err("the Patrol field on a Reach must be rejected");
    assert!(
        err.contains("directive_anchors")
            && err.contains("Patrol")
            && err.contains("directive_anchor`"),
        "error must name the misplaced field, its owner, and what Reach does read, \
         got: {err}"
    );
}

/// The reverse direction, and the shared `target` field: a Patrol reads
/// neither `directive_anchor` nor `target`.
#[test]
fn reach_and_destroy_fields_on_a_patrol_objective_are_rejected() {
    let anchor_on_patrol = r#"
[[action]]
type             = "add_objective"
id               = "obj-patrol"
text             = "Walk the line."
directive_kind   = "Patrol"
directive_anchor = "somewhere"
"#;
    let err = actions(anchor_on_patrol).expect_err("Reach's field on a Patrol");
    assert!(
        err.contains("directive_anchor") && err.contains("Reach"),
        "error must name the misplaced field and its owner, got: {err}"
    );

    let target_on_patrol = r#"
[[action]]
type           = "add_objective"
id             = "obj-patrol"
text           = "Walk the line."
directive_kind = "Patrol"
target         = "wave_1"
"#;
    let err = actions(target_on_patrol).expect_err("Destroy's field on a Patrol");
    assert!(
        err.contains("`target`") && err.contains("Destroy"),
        "error must name the misplaced field and its owner, got: {err}"
    );
}

/// A directive field with no `directive_kind` at all reads as nothing too,
/// and gets its own message rather than the "which kind reads what" one.
#[test]
fn a_directive_field_with_no_directive_kind_is_rejected() {
    let toml = r#"
[[action]]
type             = "add_objective"
id               = "obj-plain"
text             = "Do the thing."
directive_anchor = "somewhere"
"#;
    let err = actions(toml).expect_err("a directive field with no kind must be rejected");
    assert!(
        err.contains("directive_anchor") && err.contains("no directive_kind"),
        "error must say nothing reads the field, got: {err}"
    );
}

/// `targets` (plural) is the nav-radar marker list, not a directive field:
/// it is legitimate beside any kind and must not be swept up by the check.
/// Neither must a defaulted `directive_loop = false` on a non-Patrol, which
/// carries no authorial intent — the same absent-vs-default limit the entity
/// side documents.
#[test]
fn targets_and_defaulted_fields_are_allowed_beside_any_directive() {
    let toml = r#"
[[action]]
type             = "add_objective"
id               = "obj-reach"
text             = "Make the rendezvous."
targets          = ["Rendezvous Beacon"]
directive_kind   = "Reach"
directive_anchor = "rendezvous"
directive_loop   = false
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::AddObjective {
            directive, targets, ..
        } => {
            assert_eq!(
                *directive,
                crate::core::messages::AiDirective::Reach {
                    anchor: "rendezvous".into(),
                }
            );
            assert_eq!(targets, &vec!["Rendezvous Beacon".to_string()]);
        }
        other => panic!("expected AddObjective, got {other:?}"),
    }
}

// -- Objective command-stance contribution (issue #1110) ----------------

#[test]
fn add_objective_parses_a_command_stance_contribution() {
    let parsed = actions(
        r#"
[[action]]
type = "add_objective"
id   = "obj-escort"
text = "world.obj.text"
command_stance = { station = "tactical", id = "objective-escort", kind = "standard", high_alert = true, persist_behind_human = true, label = "stance.escort" }
"#,
    )
    .expect("must parse");
    match &parsed[0] {
        TriggerAction::AddObjective { command_stance, .. } => {
            let (station, stance) = command_stance
                .as_ref()
                .expect("the contribution is present");
            assert_eq!(station.0, "tactical");
            assert_eq!(stance.id, "objective-escort");
            assert_eq!(stance.kind, crate::ship::config::StanceKind::Standard);
            assert!(stance.high_alert);
            assert!(stance.persist_behind_human);
            assert_eq!(stance.label, "stance.escort");
        }
        other => panic!("expected AddObjective, got {other:?}"),
    }
}

#[test]
fn add_objective_without_a_command_stance_carries_none() {
    let parsed = actions(
        "[[action]]\n\
         type = \"add_objective\"\n\
         id = \"obj-plain\"\n\
         text = \"world.obj.text\"\n",
    )
    .expect("must parse");
    match &parsed[0] {
        TriggerAction::AddObjective { command_stance, .. } => {
            assert!(command_stance.is_none(), "no contribution is authored");
        }
        other => panic!("expected AddObjective, got {other:?}"),
    }
}

#[test]
fn add_objective_rejects_a_command_stance_with_a_blank_station() {
    let err = actions(
        "[[action]]\n\
         type = \"add_objective\"\n\
         id = \"obj-escort\"\n\
         text = \"world.obj.text\"\n\
         command_stance = { station = \"\", id = \"objective-escort\", kind = \"standard\" }\n",
    )
    .expect_err("a blank target station must be refused");
    assert!(err.contains("station"), "{err}");
}

// -- Flag-system triggers (issue #412) ---------------------------------

/// Issue #890: the bounded-window operator belongs to an AI fine system's
/// policy, which has a host to advance it once per shared AI tick. World
/// expressions evaluate against flags alone, so an atom authored here would
/// load and then read false for the whole scenario — the trap #779/#891
/// closed on two other surfaces, refused on this one before it opens.
///
/// It covered a `[[trigger]]`'s own `when` and a `[[trigger.action]]`'s
/// per-action `when` too; issue #985 deleted both, leaving the `[[entity]]`
/// spawn gate as the one world expression a TOML author can still write. A
/// SCRIPT's guard is ordinary Rhai and never reaches `parse_predicate`, so
/// this rule now guards exactly one surface — and `reject_world_history` is
/// still the shared refusal behind it.
#[test]
fn a_history_atom_is_rejected_in_an_entity_when_predicate() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/ship_harrow_destroyer.toml"
when          = "history(net_change, hull_pct, 5) < 0"
"#;
    let err = parse_world(toml).expect_err("a history atom must fail the load");
    assert!(
        err.contains("history(") && err.contains("AI fine system"),
        "the error must quote the atom and say where windows live; got: {err}"
    );
}

// -- Trigger action vocabulary ------------------------------------------
//
// The condition half of these sections (`on_world_loaded`, the region pair,
// `on_waypoint_reached`, the flag conditions, the `[[trigger]] script = "fn"`
// front-end) went with `parse_trigger_condition_from_string` in issue #985.
// The scripted registrations that replaced them are covered in
// `world::script::triggers`.

#[test]
fn flag_mutation_actions_parse() {
    let toml = r#"
[[action]]
type = "set_flag"
name = "raider_down"

[[action]]
type = "clear_flag"
name = "danger"

[[action]]
type = "increment_flag"
name = "kills"
by   = 2

[[action]]
type  = "set_flag_value"
name  = "wave"
value = 3
"#;
    let parsed = actions(toml).expect("must parse");
    assert_eq!(parsed.len(), 4);
    assert_eq!(
        parsed[0],
        TriggerAction::SetWorldFlag {
            name: "raider_down".into()
        }
    );
    assert_eq!(
        parsed[1],
        TriggerAction::ClearWorldFlag {
            name: "danger".into()
        }
    );
    assert_eq!(
        parsed[2],
        TriggerAction::IncrementWorldFlag {
            name: "kills".into(),
            by: 2
        }
    );
    assert_eq!(
        parsed[3],
        TriggerAction::SetWorldFlagValue {
            name: "wave".into(),
            value: 3
        }
    );
}

#[test]
fn set_flag_requires_name() {
    let toml = r#"
[[action]]
type = "set_flag"
"#;
    let err = actions(toml).expect_err("set_flag without name must error");
    assert!(err.contains("name"), "error must mention name: {err}");
}

#[test]
fn increment_flag_requires_by() {
    let toml = r#"
[[action]]
type = "increment_flag"
name = "kills"
"#;
    let err = actions(toml).expect_err("increment_flag without by must error");
    assert!(err.contains("by"), "error must mention by: {err}");
}

#[test]
fn parse_world_default_toml_is_script_authored_with_no_declarative_content() {
    let toml = include_str!("../../assets/worlds/default.toml");
    // (#984) default.toml is the first world whose COMMS converted to
    // `[script]`, not just its triggers: its two `[[trigger]]` blocks and
    // its three `[[comms]]` templates are all Rhai now. Since issue #985
    // `parse_world` REFUSES either block by name, so parsing clean is
    // itself the "no declarative content" half of this test — there is no
    // list left to read empty.
    //
    // The `[script]` half still earns its place: it is the check that this
    // world's scenario logic is REACHABLE, i.e. that it did not lose its
    // block in an edit. The scripted behaviour is pinned by the
    // dialogue-tree parity test in `comms::scripted` and by the
    // conversion's digest parity.
    parse_world(toml).expect("default.toml must parse");
    assert!(
        toml.contains("[script]"),
        "default.toml must carry its [script] block"
    );
}

#[test]
fn parse_world_patrol_toml_loads_triggers_with_no_comms() {
    let toml = include_str!("../../assets/worlds/patrol.toml");
    // patrol.toml is [script]-authored (issue #984): its on_world_loaded +
    // on_destroyed triggers live in the Rhai block, registered at activation
    // by compile_world_scripts/merge_script_triggers. Since issue #985 a
    // `[[trigger]]` block is refused by name, so a clean parse is the proof
    // that none survived the conversion. The scripted behaviour is pinned by
    // the world::server scripted-trigger tests and the conversion's digest
    // parity.
    parse_world(toml).expect("patrol.toml must parse");
    assert!(
        toml.contains("[script]"),
        "patrol.toml must carry its [script] block"
    );
}

/// (#475, rewritten for #892, re-authored for #960 + #936, re-homed onto the
/// script front-end for #984.)
///
/// The combat-test scenario is a TIMED eight-wave defence. This test pins
/// the structure the runtime then leans on:
///
///   - EIGHT `on_timer` registrations, one per wave, at 45-second intervals;
///     no spawn hangs off a death any more
///   - the eight-wave table: singles alternating cruiser/destroyer through
///     wave 4, pairs through wave 7, closing on a patrol cruiser
///   - every hostile spawn registers into `hostiles` (victory) and into its
///     own `wave_N` group (its objective), tier bonuses included
///   - NO standing pickets and no `pickets` group
///   - every spawn — not just the cruisers (#936) — carries the
///     `assault-starbase` Destroy override naming the starbase's STRING ID,
///     the `close-on-starbase` Reach run-in, and the 200-unit acquisition
///     band (#960)
///   - ONE victory registration over the dynamic `hostiles` group, guarded
///     by `counter(waves_spawned) >= 8`, whose `game_over` is the only
///     deferred effect in the world
///   - 1 on_destroyed Starbase Alpha defeat registration
///   - 8 wave objectives, each completed on its OWN group being cleared
///     alongside a single `mission_threat_remaining` decrement (#943)
///   - 1 on_world_loaded objective registration, and an `on_world_loaded`
///     that spawns nothing
///   - 12 comms announcements (1 on world-load + 8 on the clock + 3 hull
///     bands), all from Starbase Alpha
///
/// # Why this reads the script rather than a parsed trigger list
///
/// Issue #984 moved all of it into `[script]` and issue #985 deleted the
/// declarative parser behind it, so there is no `WorldConfig` trigger list
/// left to read. The facts above did not become
/// unobservable, they moved: a registration's condition is on the
/// `ScriptTrigger` the front-end built (byte-identical to its TOML twin), and
/// its effects are what running its handler buffers. Both are read here
/// through the production compiler and the production runtime host, so this
/// still pins the shipped world rather than a description of it.
///
/// It also got STRONGER in one place. The power-tier bonuses used to be
/// action `when` predicates, and the old test could only count that eight of
/// them carried *a* predicate. They are `if` guards inside the handler now,
/// so the guard is exercised instead of counted: the same handlers are run at
/// three `ship_power` tiers and the spawn set is asserted at each.
#[test]
fn parse_world_combat_test_toml_is_script_authored_with_8_timed_waves() {
    use crate::world::dispatch::{ActionCmd, FlagMutation};
    use crate::world::flags::FlagStore;
    use crate::world::script::effects::BufferedEffect;
    use crate::world::script::fixture::ScriptedWorld;

    const WORLD: &str = "assets/worlds/combat_test.toml";
    let toml = include_str!("../../assets/worlds/combat_test.toml");
    // A clean parse is the "fully converted" half (#985 refuses a surviving
    // `[[trigger]]` / `[[comms]]` block by name); the script read below is
    // the "and this is what it does instead" half.
    let cfg = parse_world(toml).expect("combat_test.toml must parse");
    let world = ScriptedWorld::compile(WORLD, toml);

    /// The player tier a run is read at. The DEMO tier (destroyer,
    /// power_rating 70) is under both bonus gates.
    fn tier(ship_power: i64) -> FlagStore {
        let mut flags = FlagStore::new();
        flags.set_flag_value("ship_power", ship_power);
        flags
    }
    let top = tier(120);

    // Every registration's effects at the top tier, so the bonus spawns are
    // included, paired with the condition that releases them.
    let fired: Vec<(
        TriggerCondition,
        crate::world::script::schedule::CallEffects,
    )> = world
        .triggers
        .iter()
        .map(|t| (t.trigger.condition.clone(), world.call(&t.handler, &top)))
        .collect();
    let actions = |effects: &crate::world::script::schedule::CallEffects| -> Vec<TriggerAction> {
        crate::world::script::fixture::buffered_actions(effects.commands.clone())
    };
    let all_actions: Vec<TriggerAction> = fired.iter().flat_map(|(_, e)| actions(e)).collect();
    let all_commands: Vec<ActionCmd> = fired
        .iter()
        .flat_map(|(_, e)| e.commands.iter())
        .filter_map(|e| match e {
            BufferedEffect::Cmd(c) => Some(c.clone()),
            BufferedEffect::Action(_) => None,
        })
        .collect();

    // ── The clock ────────────────────────────────────────────────────────
    // Eight timer registrations at the authored cadence, then eight more for
    // the comms calls that ride the same clock. `on_timer(n, …)` IS the
    // schedule: a reader of the world file can see when each wave lands
    // without simulating anything, which is the point of the conversion.
    let timer_starts: Vec<f32> = fired
        .iter()
        .filter(|(_, e)| e.comms_opens.is_empty())
        .filter_map(|(c, _)| match c {
            TriggerCondition::OnTimer { after_secs } => Some(*after_secs),
            _ => None,
        })
        .collect();
    assert_eq!(
        timer_starts,
        vec![0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0],
        "all eight waves must be on the clock, at the authored cadence"
    );

    // Nothing spawns off a death. This is the assertion the conversion is
    // FOR: an `on_all_destroyed` that spawns is a death-gate by another
    // name, and re-introducing one would restore the pacing #960 removed.
    for (condition, effects) in &fired {
        if !matches!(condition, TriggerCondition::OnAllDestroyed { .. }) {
            continue;
        }
        assert!(
            !actions(effects)
                .iter()
                .any(|a| matches!(a, TriggerAction::SpawnEntity { .. })),
            "no wave may be released by a death — {condition:?} spawns"
        );
    }
    // …and the game-over window is the only deferred effect left, so a
    // `schedule.in_seconds` cannot quietly become a second, hidden schedule.
    let delayed: Vec<&TriggerAction> = fired
        .iter()
        .flat_map(|(_, e)| e.delayed.iter())
        .map(|d| &d.action)
        .collect();
    assert_eq!(
        delayed.len(),
        1,
        "expected one delayed effect, got {delayed:?}"
    );
    assert!(
        matches!(delayed[0], TriggerAction::GameOver { .. }),
        "the only delayed effect may be the game-over window, got {:?}",
        delayed[0]
    );
    assert!(
        fired.iter().all(|(_, e)| e.callbacks.is_empty()),
        "no handler defers a CALLBACK — the world's only deferral is the \
         game-over window, and a callback would be a second scheduler"
    );

    // Collect every spawn in the world, keyed by spawned entity name.
    #[allow(clippy::type_complexity)]
    let spawns: HashMap<String, (String, Vec<String>, Option<toml::Value>)> = all_actions
        .iter()
        .filter_map(|a| match a {
            TriggerAction::SpawnEntity {
                name,
                template_path,
                groups,
                overrides,
                ..
            } => Some((
                name.clone(),
                (template_path.clone(), groups.clone(), overrides.clone()),
            )),
            _ => None,
        })
        .collect();

    const CRUISER: &str = "assets/entities/ship_harrow_cruiser.toml";
    const DESTROYER: &str = "assets/entities/ship_harrow_destroyer.toml";
    const PATROL: &str = "assets/entities/ship_harrow_patrol.toml";

    // The eight-wave table (#892). #960 changed WHEN each wave arrives, not
    // what is in it.
    let table: &[(&str, &str)] = &[
        ("wave_1", CRUISER),
        ("wave_2", DESTROYER),
        ("wave_3", CRUISER),
        ("wave_4", DESTROYER),
        ("wave_5", CRUISER),
        ("wave_5_second", CRUISER),
        ("wave_6", DESTROYER),
        ("wave_6_second", DESTROYER),
        ("wave_7", CRUISER),
        ("wave_7_second", DESTROYER),
        ("wave_8", PATROL),
    ];
    for (name, template) in table {
        let (path, groups, _) = spawns
            .get(*name)
            .unwrap_or_else(|| panic!("combat_test must spawn {name}"));
        assert_eq!(path, template, "{name} must fly {template}");
        let wave_group = name.trim_end_matches("_second");
        assert!(
            groups.contains(&"hostiles".to_string()) && groups.contains(&wave_group.to_string()),
            "{name} must join both 'hostiles' and '{wave_group}', got {groups:?}"
        );
    }
    // Waves 1-4 and 8 are singles; only 5, 6 and 7 field a second ship.
    for wave in [1, 2, 3, 4, 8] {
        assert!(
            !spawns.contains_key(&format!("wave_{wave}_second")),
            "wave {wave} is a single ship in the corrected table"
        );
    }

    // Tier bonuses ride the wave groups, so a wave's objective waits for
    // them too.
    for wave in 1..=8 {
        let name = format!("wave_{wave}_bonus");
        let (path, groups, _) = spawns
            .get(&name)
            .unwrap_or_else(|| panic!("combat_test must author {name}"));
        let expected = if wave % 2 == 1 { DESTROYER } else { CRUISER };
        assert_eq!(
            path, expected,
            "{name} is the odd/even tier bonus and must fly {expected}"
        );
        assert!(
            groups.contains(&"hostiles".to_string()) && groups.contains(&format!("wave_{wave}")),
            "{name} must gate both victory and its wave, got {groups:?}"
        );
    }

    // ── The bonus gates, EXERCISED (#984) ────────────────────────────────
    // The declarative form was an action `when` predicate and could only be
    // counted; the scripted form is an `if` on the same counter, so run the
    // same handlers at each tier and read what they spawn.
    let bonuses_at = |ship_power: i64| -> Vec<String> {
        let flags = tier(ship_power);
        let mut names: Vec<String> = world
            .triggers
            .iter()
            .flat_map(|t| world.actions(&t.handler, &flags))
            .filter_map(|a| match a {
                TriggerAction::SpawnEntity { name, .. } if name.ends_with("_bonus") => Some(name),
                _ => None,
            })
            .collect();
        names.sort();
        names
    };
    assert!(
        bonuses_at(70).is_empty(),
        "the DEMO tier (destroyer, power_rating 70) is under both gates and \
         must see exactly the authored eight-wave table"
    );
    assert_eq!(
        bonuses_at(90),
        vec![
            "wave_1_bonus".to_string(),
            "wave_3_bonus".to_string(),
            "wave_5_bonus".to_string(),
            "wave_7_bonus".to_string()
        ],
        "a cruiser or battleship (>= 90) adds a destroyer to each ODD wave"
    );
    assert_eq!(
        bonuses_at(120).len(),
        8,
        "a battleship (>= 100) adds a cruiser to each EVEN wave as well"
    );

    // ── No standing presence (#960) ──────────────────────────────────────
    for picket in ["picket_north", "picket_south"] {
        assert!(
            !spawns.contains_key(picket),
            "the picket line is retired — {picket} must not spawn"
        );
    }
    for (name, (_, groups, _)) in &spawns {
        assert!(
            !groups.contains(&"pickets".to_string()),
            "{name} joins the retired 'pickets' group"
        );
        assert!(
            groups.iter().any(|g| g.starts_with("wave_")),
            "{name} stands outside the wave schedule — every ship in this \
             world belongs to a wave now, got {groups:?}"
        );
    }
    // The `on_world_loaded` handlers flip factions, publish the threat count
    // and add the standing mission; none may put anything on station.
    for (condition, effects) in &fired {
        if !matches!(condition, TriggerCondition::OnWorldLoaded) {
            continue;
        }
        assert!(
            !actions(effects)
                .iter()
                .any(|a| matches!(a, TriggerAction::SpawnEntity { .. })),
            "world load must spawn nothing — the raid is the whole roster"
        );
    }
    // Both directions of the Federation <-> Harrow rivalry are armed there.
    let enemies: Vec<(&str, &str)> = all_actions
        .iter()
        .filter_map(|a| match a {
            TriggerAction::AddFactionEnemy { faction, enemy } => {
                Some((faction.as_str(), enemy.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        enemies,
        vec![("Harrow", "Federation"), ("Federation", "Harrow")],
        "the rivalry is asymmetric, so both directions must be authored"
    );

    // ── Every wave commits to the assault (#936) ─────────────────────────
    // Asserted per spawn rather than counted: #936 exists precisely because
    // the override was authored on some spawns and not others, and a count
    // cannot tell the difference between "all nineteen" and "eleven of
    // nineteen". The conversion factors the override into `raid_overrides()`
    // so there is one place to author it — but this still reads the value
    // each spawn ACTUALLY carries, because a factored helper is only as good
    // as every call site using it.
    for (name, (_, _, overrides)) in &spawns {
        let doctrine = overrides
            .as_ref()
            .and_then(|o| o.get("behaviour"))
            .and_then(|b| b.get("doctrine"))
            .and_then(|d| d.as_array())
            .unwrap_or_else(|| panic!("{name} must override behaviour.doctrine"));
        let entry = |id: &str| {
            doctrine
                .iter()
                .find(|e| e.get("id").and_then(|i| i.as_str()) == Some(id))
        };
        let assault = entry("assault-starbase")
            .unwrap_or_else(|| panic!("{name} must carry the assault-starbase override"));
        assert_eq!(
            assault.get("directive_target").and_then(|t| t.as_str()),
            Some("world.entity.starbase_alpha.name"),
            "{name}'s assault must name the starbase's STRING ID — the \
             display text 'Starbase Alpha' matches no entity name, which is \
             how every wave's assault silently resolved to nothing"
        );
        // The fractional leaves survive the `no_float` boundary as the same
        // f64 the declarative `0.9` parsed to: `flt("0.9")` carries the value
        // as opaque data rather than as arithmetic (#984).
        assert_eq!(
            assault.get("target_speed").and_then(|s| s.as_float()),
            Some(0.9),
            "{name}'s assault must keep its fractional cruise speed"
        );
        assert!(
            entry("close-on-starbase").is_some(),
            "{name} must carry the close-on-starbase run-in, or it cannot \
             reach a target outside its own acquisition band"
        );
        // The 200-unit engagement band (#960), authored per spawn because
        // the hull templates declare no radar at all — which the host reads
        // as an UNBOUNDED horizon.
        let range = overrides
            .as_ref()
            .and_then(|o| o.get("weapons_console"))
            .and_then(|w| w.get("radar"))
            .and_then(|r| r.get("range"))
            .and_then(|r| r.as_float());
        assert_eq!(
            range,
            Some(200.0),
            "{name} must author the 200-unit acquisition band"
        );
    }

    // Victory: ONE registration over the dynamic `hostiles` group, guarded by
    // the wave counter. The three per-tier name-list variants are gone — an
    // unregistered name in such a list permanently blocks victory, which is
    // how this world shipped unwinnable.
    let victories: Vec<&(
        TriggerCondition,
        crate::world::script::schedule::CallEffects,
    )> = fired
        .iter()
        .filter(|(_, e)| {
            e.delayed.iter().any(|d| {
                matches!(&d.action, TriggerAction::GameOver { outcome, .. }
                        if *outcome == Some(crate::core::balance::Outcome::Victory))
            })
        })
        .collect();
    assert_eq!(
        victories.len(),
        1,
        "combat_test must have ONE victory registration"
    );
    match &victories[0].0 {
        TriggerCondition::OnAllDestroyed { group, .. } => {
            assert_eq!(group, "hostiles", "victory covers every hostile spawned");
        }
        other => panic!("victory must be on_all_destroyed, got {other:?}"),
    }
    let victory_trigger = world
        .triggers
        .iter()
        .find(|t| t.trigger.condition == victories[0].0)
        .expect("the victory registration");
    assert!(
        victory_trigger.trigger.when.is_some(),
        "victory must be guarded by the waves_spawned counter, and by a \
         trigger-level `.when(…)` rather than an `if` in the handler. Under a \
         CLOCK this matters MORE than it did under the death-gated chain: a \
         good player can clear every ship on the field with four waves still \
         unspawned, and without the guard that window reads as victory. An \
         in-handler guard would not do — the condition would already have \
         fired and SPENT the one-shot trigger, so victory could never fire \
         again; a `when` that reads false leaves it armed"
    );
    // The guard has to be satisfiable: eight increments are authored.
    let increments: i64 = all_commands
        .iter()
        .filter_map(|c| match c {
            ActionCmd::MutateFlag {
                name,
                mutation: FlagMutation::Increment(by),
                ..
            } if name == "waves_spawned" => Some(*by),
            _ => None,
        })
        .sum();
    assert_eq!(
        increments, 8,
        "waves_spawned must be able to reach the victory guard's threshold"
    );

    // Starbase-destroyed defeat registration.
    let defeat = fired.iter().any(|(c, _)| {
        matches!(
            c,
            TriggerCondition::OnDestroyed { entity_name } if entity_name == "world.entity.starbase_alpha.name"
        )
    });
    assert!(
        defeat,
        "must have on_destroyed Starbase Alpha defeat registration"
    );

    for anchor in [
        "starbase_patrol_north",
        "starbase_patrol_east",
        "starbase_patrol_south",
        "starbase_patrol_west",
    ] {
        assert!(
            cfg.anchors.contains_key(anchor),
            "combat_test must define patrol anchor {anchor}"
        );
    }
    // The run-in point every wave's Reach entry names.
    assert!(
        cfg.anchors.contains_key("harrow_assault_point"),
        "combat_test must define the Harrow run-in anchor"
    );
    // The picket's own stations are GONE, not merely unflown. `combat_test`
    // retired the picket line; a world that still declares the route it no
    // longer flies is a world whose anchor table lies about its geography.
    for anchor in ["ironveil_patrol_a", "ironveil_patrol_b"] {
        assert!(
            !cfg.anchors.contains_key(anchor),
            "combat_test still declares picket station {anchor}. Nothing in \
             this world flies that route: wave 8's override stands the \
             template's `patrol-ironveil` entry down, so the load-time \
             anchor check (`doctrine_anchor_refs`, which reads the effective \
             doctrine) never asks for it."
        );
    }
    // …which is only true because wave 8 — the one hull carrying that Patrol
    // entry — stands it DOWN rather than de-prioritising it. A `Patrol` at
    // `base_priority = 0` is still a Patrol, and a Patrol still names its
    // anchors, so this is the assertion that keeps the two anchors deleted.
    let (_, _, wave_8_overrides) = &spawns["wave_8"];
    let ironveil = wave_8_overrides
        .as_ref()
        .and_then(|o| o.get("behaviour"))
        .and_then(|b| b.get("doctrine"))
        .and_then(|d| d.as_array())
        .and_then(|d| {
            d.iter()
                .find(|e| e.get("id").and_then(|i| i.as_str()) == Some("patrol-ironveil"))
        })
        .expect("wave 8 must restate patrol-ironveil");
    assert_eq!(
        ironveil.get("directive_kind").and_then(|k| k.as_str()),
        Some("None"),
        "wave 8's picket route must be stood DOWN, not merely quietened — \
         `directive_kind = \"None\"` is what stops it naming anchors"
    );
    assert_eq!(
        ironveil
            .get("directive_anchors")
            .and_then(|a| a.as_array())
            .map(Vec::len),
        Some(0),
        "and its anchor list must be cleared: an override array is only \
         subtractive when it is empty, and a `directive_anchors` left \
         populated under `directive_kind = \"None\"` is rejected outright by \
         `validate_doctrine_directives`"
    );
    assert_eq!(
        ironveil.get("directive_loop").and_then(|l| l.as_bool()),
        Some(false),
        "…and so must `directive_loop`, the OTHER Patrol-owned field this \
         template authors. Standing an entry down means clearing every field \
         its old kind owned. Leaving this one set fails the merge, and a \
         failed merge is silent in exactly the wrong place: \
         `doctrine_anchor_refs` reads it as 'no anchors to check', so the \
         world would load and wave 8 would simply never spawn"
    );
    assert_eq!(
        ironveil.get("base_priority").and_then(|p| p.as_float()),
        Some(0.0),
        "wave 8's picket route must also score zero, so it cannot lead a \
         pool whose consumers all take the FIRST entry"
    );

    // ── Wave completion + the remaining-threat count (#943 under #960) ───
    // Each wave's objective completes on ITS OWN group being cleared, beside
    // exactly one `mission_threat_remaining` decrement. Under the clock
    // these stand alone rather than riding the next wave's spawn trigger,
    // and waves may now be cleared out of order — so the pairing has to be
    // per wave rather than a total.
    for wave in 1..=8 {
        let group = format!("wave_{wave}");
        let id = format!("obj-destroy-wave-{wave}");
        let effects = fired
            .iter()
            .find(|(c, e)| {
                matches!(c,
                    TriggerCondition::OnAllDestroyed { group: g, .. } if *g == group)
                    && e.commands.iter().any(|cmd| {
                        matches!(cmd, BufferedEffect::Cmd(ActionCmd::CompleteObjective { id: cid })
                            if *cid == id)
                    })
            })
            .map(|(_, e)| e)
            .unwrap_or_else(|| panic!("{id} must complete when {group} is cleared"));
        let paid: i64 = effects
            .commands
            .iter()
            .filter_map(|c| match c {
                BufferedEffect::Cmd(ActionCmd::MutateFlag {
                    name,
                    mutation: FlagMutation::Increment(by),
                    ..
                }) if name == "mission_threat_remaining" => Some(*by),
                _ => None,
            })
            .sum();
        assert_eq!(
            paid, -1,
            "clearing {group} must pay down mission_threat_remaining by \
             exactly one — it counts waves still to be FOUGHT, and a wave \
             that is dead is not one of them"
        );
    }
    // The counter starts at the number of waves and is paid down to zero.
    // An ABSOLUTE write (`flags.x = 8`), the scripted spelling of the
    // declarative `set_flag_value`; the decrements are composable
    // `increment`s, which is what lets them apply in any order (#981).
    let seeded: i64 = all_commands
        .iter()
        .find_map(|c| match c {
            ActionCmd::MutateFlag {
                name,
                mutation: FlagMutation::SetValue(value),
                ..
            } if name == "mission_threat_remaining" => Some(*value),
            _ => None,
        })
        .expect("combat_test must publish mission_threat_remaining");
    let paid_total: i64 = all_commands
        .iter()
        .filter_map(|c| match c {
            ActionCmd::MutateFlag {
                name,
                mutation: FlagMutation::Increment(by),
                ..
            } if name == "mission_threat_remaining" => Some(*by),
            _ => None,
        })
        .sum();
    assert_eq!(seeded, 8, "eight waves of published threat");
    assert_eq!(
        seeded + paid_total,
        0,
        "the published threat must reach zero when every wave is dead, or a \
         magazine paced against it never releases its last reserve"
    );

    let defend = all_actions
        .iter()
        .find(|a| matches!(a, TriggerAction::AddObjective { id, .. } if id == "obj-defend"))
        .expect("combat_test must add obj-defend");
    match defend {
        TriggerAction::AddObjective {
            directive, utility, ..
        } => {
            assert_eq!(
                *directive,
                crate::core::messages::AiDirective::Patrol {
                    anchors: vec![
                        "starbase_patrol_north".into(),
                        "starbase_patrol_east".into(),
                        "starbase_patrol_south".into(),
                        "starbase_patrol_west".into(),
                    ],
                    loop_path: true,
                }
            );
            assert_eq!(utility.base_priority, 20.0);
        }
        other => panic!("expected obj-defend AddObjective, got {other:?}"),
    }

    let destroy_objectives: Vec<_> = all_actions
        .iter()
        .filter_map(|a| match a {
            TriggerAction::AddObjective {
                id,
                directive,
                targets,
                utility,
                ..
            } if id.starts_with("obj-destroy-wave-") => {
                Some((id, directive, targets, utility.base_priority))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        destroy_objectives.len(),
        8,
        "combat_test must add 8 wave destroy objectives"
    );
    for wave in 1..=8 {
        let id = format!("obj-destroy-wave-{wave}");
        let target = format!("wave_{wave}");
        let Some((_, directive, targets, base_priority)) = destroy_objectives
            .iter()
            .find(|(objective_id, _, _, _)| *objective_id == &id)
        else {
            panic!("missing destroy objective {id}");
        };
        assert_eq!(
            **directive,
            crate::core::messages::AiDirective::Destroy {
                target: target.clone(),
            }
        );
        // Two-ship waves list both hulls; the directive still names one.
        assert!(
            targets.contains(&target),
            "{id} must list {target} among its targets, got {targets:?}"
        );
        assert_eq!(*base_priority, 80.0);
    }

    // ── Comms (#984) ─────────────────────────────────────────────────────
    // Twelve one-way reports, all from Starbase Alpha: 1 on_world_loaded
    // urgent brief + 8 calls on the clock, one per wave + 3 integrity bands.
    // Command warns the bridge that the next wave has LAUNCHED; under a clock
    // there is no death to report, and a player who is behind the schedule
    // still gets the warning.
    let opens: Vec<(&TriggerCondition, &crate::comms::content::OpenCommsRequest)> = fired
        .iter()
        .flat_map(|(c, e)| e.comms_opens.iter().map(move |o| (c, o)))
        .collect();
    assert_eq!(opens.len(), 12, "combat_test must open 12 comms threads");
    assert!(
        opens
            .iter()
            .all(|(_, o)| o.from == "world.entity.starbase_alpha.name"),
        "every report comes from the station the scenario defends"
    );
    let comms_timed: Vec<f32> = opens
        .iter()
        .filter_map(|(c, _)| match c {
            TriggerCondition::OnTimer { after_secs } => Some(*after_secs),
            _ => None,
        })
        .collect();
    assert_eq!(
        comms_timed,
        vec![0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0],
        "every wave call fires with its matching wave release"
    );
    assert!(
        opens
            .iter()
            .all(|(c, _)| !matches!(c, TriggerCondition::OnAllDestroyed { .. })),
        "no comms report may still wait on a wave dying"
    );
    // Each call lands with the wave it announces — the ordering that makes
    // it a warning rather than a running commentary.
    assert_eq!(comms_timed, timer_starts);
    let hull_thresholds: Vec<f32> = opens
        .iter()
        .filter_map(|(c, _)| match c {
            TriggerCondition::OnHullBelow { threshold, .. } => Some(*threshold),
            _ => None,
        })
        .collect();
    assert_eq!(
        hull_thresholds,
        vec![0.75, 0.5, 0.1],
        "the three integrity bands survive the `no_float` boundary as the \
         same f32 the declarative `threshold = 0.75` parsed to"
    );
    // Urgency: the brief, the last four wave calls and all three hull bands.
    assert_eq!(
        opens.iter().filter(|(_, o)| o.urgent).count(),
        8,
        "eight of the twelve reports are flagged urgent"
    );
    assert!(
        fired
            .iter()
            .filter(|(_, e)| !e.comms_opens.is_empty())
            .all(|(_, e)| e.commands.is_empty() && e.delayed.is_empty()),
        "a comms registration opens a thread and does nothing else — mixing \
         an effect into one would make the report's position in the tick \
         observable"
    );
    // The demo world flies its own opening brief on world load.
    assert!(
        opens
            .iter()
            .any(|(c, o)| matches!(c, TriggerCondition::OnWorldLoaded) && o.urgent),
        "combat_test must open with an urgent brief"
    );
}

// -- entity_template_paths ---------------------------------------------

#[test]
fn entity_template_paths_returns_empty_for_no_entities() {
    let world = WorldConfig::default();
    assert!(entity_template_paths(&world, &[]).is_empty());
}

#[test]
fn entity_template_paths_deduplicates_repeated_paths() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_large.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/asteroid_large.toml"
transform = { position = [10.0, 0.0, 10.0] }

[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [100.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let paths = entity_template_paths(&cfg, &[]);
    assert_eq!(paths.len(), 2, "duplicates must be collapsed");
    assert!(paths.contains(&"assets/entities/asteroid_large.toml".to_string()));
    assert!(paths.contains(&"assets/entities/star_sun.toml".to_string()));
}

#[test]
fn entity_template_paths_preserves_first_occurrence_order() {
    let toml = r#"
[[entity]]
template_path = "first.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "second.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "first.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "third.toml"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let paths = entity_template_paths(&cfg, &[]);
    assert_eq!(
        paths,
        vec![
            "first.toml".to_string(),
            "second.toml".to_string(),
            "third.toml".to_string()
        ],
        "iteration order must follow first-occurrence in the entity list"
    );
}

// -- entity_template_paths: trigger / comms walks (#475) ----------------

/// (#984) The scripted surface. A Rhai handler's `spawn_entity` map names
/// its template as a literal and the handler does not RUN until long after
/// preload, so the only thing available before the fetch is the source text.
///
/// The two negative cases matter as much as the positive one: a mention in a
/// `//` comment must not queue a fetch (an over-fetch of a path that does not
/// exist is a load error, not a wasted request), and a sibling-FILE script
/// contributes nothing because `parse_world` has no resolver to read it —
/// which is exactly why every shipped world authors `[script]` inline.
#[test]
fn entity_template_paths_includes_script_spawn_entity_templates() {
    let toml = r#"
[script]
setup = """
on_timer(0, "wave");

// A wave used to name template_path: "assets/entities/retired.toml" here.
fn wave(ctx) {
ctx.effects.spawn_entity(#{
    template_path: "assets/entities/ship_harrow_cruiser.toml",
    name: "wave_1", position: [0, 0, 0]
});
}
"""
"#;
    let cfg = parse_world(toml).expect("must parse");
    let paths = entity_template_paths(&cfg, &[]);
    assert_eq!(
        paths,
        vec!["assets/entities/ship_harrow_cruiser.toml".to_string()],
        "the scripted spawn must be discovered and the commented-out one must not"
    );

    // A sibling-file script is outside the scan by construction.
    let sibling = parse_world("script = \"combat.rhai\"\n").expect("must parse");
    assert!(sibling.script_sources.is_empty());
    assert!(entity_template_paths(&sibling, &[]).is_empty());
}

#[test]
fn entity_template_paths_combat_test_includes_wave_templates() {
    // (#475) Pin the exact bug: combat_test.toml references its
    // wave templates only inside spawn_entity actions. All of them must
    // appear in the preload list — a template that is spawned but not
    // preloaded pops in late.
    //
    // (#984) Those actions are now `ctx.effects.spawn_entity(#{…})` calls
    // in Rhai handlers, which is why `entity_template_paths` grew a fifth,
    // SCRIPT surface. This is the test that fails without it, and it is not
    // a cosmetic failure: `combat_test` is the only selectable scenario, so
    // the surface is the whole demo's raid.
    //
    // (#883) `ship_harrow_destroyer` joins the list: it is the
    // fly-through interceptor wave, and it is the one hull whose
    // helm behaviour is authored content rather than shared code,
    // so a missed preload would be especially visible.
    //
    // (#790) `ship_harrow_cruiser` joins the list: the overlapping
    // 270-degree fore/aft phaser pair only reads as a double
    // broadside if the hull is on station when its wave fires.
    let toml = include_str!("../../assets/worlds/combat_test.toml");
    let cfg = parse_world(toml).expect("combat_test.toml must parse");
    let paths = entity_template_paths(&cfg, &[]);
    for required in &[
        // (#892) `pirate_raider.toml` was retired, and the corrected
        // eight-wave table closes on a patrol cruiser rather than a
        // Warhawk — so `ship_harrow_warhawk.toml` left this list with the
        // hull that read it. Three hulls fly here now: the patrol cruiser
        // (wave 8), the destroyer, the cruiser.
        "assets/entities/ship_harrow_patrol.toml",
        "assets/entities/ship_harrow_destroyer.toml",
        "assets/entities/ship_harrow_cruiser.toml",
    ] {
        assert!(
            paths.contains(&required.to_string()),
            "combat_test wave template {required:?} must be preloaded, got {paths:?}"
        );
    }
}

// -- partition_immediate_entities --------------------------------------

#[test]
fn partition_immediate_entities_routes_asteroid_fields_separately() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [100.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/asteroid_field_outer.toml"
transform = { position = [500.0, 0.0, 500.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let (fields, others) =
        partition_immediate_entities(&cfg, |path| path.contains("asteroid_field"));
    assert_eq!(fields.len(), 2);
    assert_eq!(others.len(), 1);
    assert_eq!(others[0].template_path, "assets/entities/star_sun.toml");
}

#[test]
fn partition_immediate_entities_excludes_game_start_entries() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/asteroid_field_main.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
"#;
    let cfg = parse_world(toml).expect("must parse");
    let (fields, others) =
        partition_immediate_entities(&cfg, |path| path.contains("asteroid_field"));
    assert_eq!(fields.len(), 1);
    assert!(
        others.is_empty(),
        "game_start entries must NOT appear in the 'other' bucket"
    );
}

#[test]
fn partition_immediate_entities_empty_world_yields_two_empty_buckets() {
    let cfg = WorldConfig::default();
    let (fields, others) = partition_immediate_entities(&cfg, |_| true);
    assert!(fields.is_empty());
    assert!(others.is_empty());
}

// -- extra_worlds (issue #352) -----------------------------------------

#[test]
fn parse_world_extra_worlds_defaults_to_empty() {
    let cfg = parse_world("").expect("empty TOML should parse");
    assert!(cfg.extra_worlds.is_empty());
}

#[test]
fn parse_world_reads_extra_worlds_list() {
    let toml = r#"
extra_worlds = ["assets/worlds/patrol.toml", "assets/worlds/side_mission.toml"]
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.extra_worlds.len(), 2);
    assert_eq!(cfg.extra_worlds[0], "assets/worlds/patrol.toml");
    assert_eq!(cfg.extra_worlds[1], "assets/worlds/side_mission.toml");
}

#[test]
fn parse_world_rejects_empty_string_in_extra_worlds() {
    let toml = r#"
extra_worlds = ["assets/worlds/patrol.toml", ""]
"#;
    let err = parse_world(toml).expect_err("empty path in extra_worlds must error");
    assert!(
        err.contains("extra_worlds"),
        "error must mention extra_worlds: {err}"
    );
}

#[test]
fn parse_world_rejects_whitespace_only_string_in_extra_worlds() {
    let toml = r#"
extra_worlds = ["   "]
"#;
    let err = parse_world(toml).expect_err("whitespace-only path in extra_worlds must error");
    assert!(
        err.contains("extra_worlds"),
        "error must mention extra_worlds: {err}"
    );
}

#[test]
fn parse_world_extra_worlds_round_trips_via_worldconfig() {
    let toml = r#"
extra_worlds = ["assets/worlds/patrol.toml"]
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(
        cfg.extra_worlds,
        vec!["assets/worlds/patrol.toml".to_string()]
    );
}

// -- LoadWorld / UnloadWorld trigger actions (issue #352) -------------

#[test]
fn load_world_action_parses() {
    let toml = r#"
[[action]]
type = "load_world"
path = "assets/worlds/patrol.toml"
"#;
    let parsed = actions(toml).expect("must parse");
    assert_eq!(parsed.len(), 1);
    match &parsed[0] {
        TriggerAction::LoadWorld { path } => {
            assert_eq!(path, "assets/worlds/patrol.toml");
        }
        other => panic!("expected LoadWorld, got {other:?}"),
    }
}

#[test]
fn unload_world_action_parses() {
    let toml = r#"
[[action]]
type = "unload_world"
path = "assets/worlds/patrol.toml"
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::UnloadWorld { path } => {
            assert_eq!(path, "assets/worlds/patrol.toml");
        }
        other => panic!("expected UnloadWorld, got {other:?}"),
    }
}

#[test]
fn load_world_action_requires_path_field() {
    let toml = r#"
[[action]]
type = "load_world"
"#;
    let err = actions(toml).expect_err("load_world without path must error");
    assert!(
        err.contains("load_world") && err.contains("path"),
        "error must mention load_world and path: {err}"
    );
}

#[test]
fn unload_world_action_requires_path_field() {
    let toml = r#"
[[action]]
type = "unload_world"
"#;
    let err = actions(toml).expect_err("unload_world without path must error");
    assert!(
        err.contains("unload_world") && err.contains("path"),
        "error must mention unload_world and path: {err}"
    );
}

#[test]
fn load_scenario_action_is_rejected_as_unknown() {
    // PRD #341 removed the `load_scenario` action; `load_world` is the
    // replacement. Any lingering `load_scenario` in a world TOML must be
    // rejected as an unknown action rather than silently parsed.
    let toml = r#"
[[action]]
type = "load_scenario"
load_scenario = "assets/worlds/patrol.toml"
"#;
    let err = actions(toml).expect_err("load_scenario must be rejected as unknown action");
    assert!(
        err.contains("Unknown trigger action") && err.contains("load_scenario"),
        "error must flag load_scenario as unknown: {err}"
    );
}

#[test]
fn partition_immediate_entities_classifier_returning_false_for_all_keeps_everything_in_other() {
    let toml = r#"
[[entity]]
template_path = "a.toml"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "b.toml"
transform = { position = [0.0, 0.0, 0.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let (fields, others) = partition_immediate_entities(&cfg, |_| false);
    assert!(fields.is_empty());
    assert_eq!(others.len(), 2);
}

// ── TransformConfig (Slice 4) ─────────────────────────────────────────

#[test]
fn transform_config_parses_inline_table_with_position() {
    let toml = r#"
[[entity]]
template_path = "x.toml"
transform = { position = [1.0, 2.0, 3.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let xf = cfg.entities[0]
        .transform
        .as_ref()
        .expect("transform present");
    assert_eq!(xf.position, Some([1.0, 2.0, 3.0]));
    assert!(xf.anchor.is_none() && xf.relative_to.is_none());
    assert!(xf.rotation.is_none() && xf.scale.is_none());
}

#[test]
fn transform_config_parses_subtable_form_with_rotation_and_scale() {
    let toml = r#"
[[entity]]
template_path = "x.toml"

[entity.transform]
position = [10.0, 0.0, 20.0]
rotation = [0.0, 1.5707963, 0.0]
scale    = [2.0, 2.0, 2.0]
"#;
    let cfg = parse_world(toml).expect("must parse");
    let xf = cfg.entities[0]
        .transform
        .as_ref()
        .expect("transform present");
    assert_eq!(xf.position, Some([10.0, 0.0, 20.0]));
    let rotation = xf.rotation.expect("rotation present");
    assert_eq!(rotation[0], 0.0);
    assert!((rotation[1] - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert_eq!(rotation[2], 0.0);
    assert_eq!(xf.scale, Some([2.0, 2.0, 2.0]));
}

#[test]
fn transform_config_anchor_and_relative_to_round_trip() {
    let toml = r#"
[[entity]]
template_path = "a.toml"
transform = { anchor = "spawn_point" }

[[entity]]
template_path = "b.toml"
transform = { relative_to = "leader", offset = [0.0, 0.0, -10.0] }
"#;
    let cfg = parse_world(toml).expect("must parse");
    let xa = cfg.entities[0].transform.as_ref().unwrap();
    assert_eq!(xa.anchor.as_deref(), Some("spawn_point"));
    let xb = cfg.entities[1].transform.as_ref().unwrap();
    assert_eq!(xb.relative_to.as_deref(), Some("leader"));
    assert_eq!(xb.offset, Some([0.0, 0.0, -10.0]));
}

#[test]
fn transform_config_missing_transform_means_none() {
    let toml = r#"
[[entity]]
template_path = "x.toml"
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert!(cfg.entities[0].transform.is_none());
}

#[test]
fn transform_config_serde_round_trip_via_toml() {
    let xf = TransformConfig {
        position: Some([1.0, 2.0, 3.0]),
        anchor: None,
        relative_to: None,
        offset: None,
        rotation: Some([0.1, 0.2, 0.3]),
        scale: Some([1.5, 1.5, 1.5]),
    };
    let s = toml::to_string(&xf).expect("serialize");
    let back: TransformConfig = toml::from_str(&s).expect("deserialize");
    assert_eq!(back, xf);
}

#[test]
fn transform_config_quat_defaults_to_identity_when_no_rotation() {
    let xf = TransformConfig::default();
    let q = xf.quat();
    assert!((q.length() - 1.0).abs() < 1e-5);
    assert!((q.w - 1.0).abs() < 1e-5);
    assert!(q.x.abs() < 1e-5 && q.y.abs() < 1e-5 && q.z.abs() < 1e-5);
}

#[test]
fn transform_config_quat_matches_from_euler_xyz() {
    let xf = TransformConfig {
        rotation: Some([0.5, 1.0, -0.25]),
        ..Default::default()
    };
    let expected = bevy::math::Quat::from_euler(bevy::math::EulerRot::XYZ, 0.5, 1.0, -0.25);
    assert!(xf.quat().abs_diff_eq(expected, 1e-6));
}

#[test]
fn transform_config_scale_vec_defaults_to_one() {
    let xf = TransformConfig::default();
    assert_eq!(xf.scale_vec(), bevy::math::Vec3::ONE);
}

#[test]
fn transform_config_scale_vec_reads_explicit_scale() {
    let xf = TransformConfig {
        scale: Some([2.0, 3.0, 4.0]),
        ..Default::default()
    };
    assert_eq!(xf.scale_vec(), bevy::math::Vec3::new(2.0, 3.0, 4.0));
}

#[test]
fn transform_config_resolve_precedence_relative_to_wins() {
    let xf = TransformConfig {
        position: Some([99.0, 99.0, 99.0]),
        anchor: Some("a".into()),
        relative_to: Some("base".into()),
        offset: Some([1.0, 2.0, 3.0]),
        ..Default::default()
    };
    let anchors = anchor_table(&[("a", [50.0, 50.0, 50.0])]);
    let resolved = resolved_table(&[("base", [10.0, 0.0, 0.0])]);
    let pos = xf.resolve("x.toml", &anchors, &resolved).unwrap();
    assert_eq!(pos, [11.0, 2.0, 3.0]);
}

#[test]
fn transform_config_resolve_falls_back_to_origin_when_nothing_set() {
    let xf = TransformConfig::default();
    let pos = xf
        .resolve("x.toml", &HashMap::new(), &HashMap::new())
        .unwrap();
    assert_eq!(pos, [0.0, 0.0, 0.0]);
}

// ── AmbientLightConfig (Slice 4) ──────────────────────────────────────

#[test]
fn ambient_light_config_round_trip_via_toml() {
    let cfg = AmbientLightConfig {
        color: Some([0.6, 0.55, 0.5]),
        brightness: Some(300.0),
    };
    let s = toml::to_string(&cfg).expect("serialize");
    let back: AmbientLightConfig = toml::from_str(&s).expect("deserialize");
    assert_eq!(back, cfg);
}

#[test]
fn parse_world_reads_top_level_ambient_light_block() {
    let toml = r#"
[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0
"#;
    let cfg = parse_world(toml).expect("must parse");
    let al = cfg.ambient_light.as_ref().expect("ambient_light present");
    assert_eq!(al.color, Some([0.6, 0.55, 0.5]));
    assert_eq!(al.brightness, Some(300.0));
}

#[test]
fn parse_world_omits_ambient_light_when_block_missing() {
    let cfg = parse_world("").expect("empty TOML parses");
    assert!(cfg.ambient_light.is_none());
}

#[test]
fn parse_world_ambient_light_accepts_partial_fields() {
    let toml = r#"
[ambient_light]
brightness = 150.0
"#;
    let cfg = parse_world(toml).expect("must parse");
    let al = cfg.ambient_light.as_ref().expect("ambient_light present");
    assert!(al.color.is_none());
    assert_eq!(al.brightness, Some(150.0));
}

// ── [render] (PRD #1023, module 5) ────────────────────────────────

/// No shipped world authors a `[render]` block, so the absent case is the
/// one that actually ships — and it must mean "the documented calibration",
/// not "off".
#[test]
fn a_world_with_no_render_block_carries_none() {
    let cfg = parse_world("").expect("must parse");
    assert!(cfg.render.is_none());
    let effective = cfg.render.unwrap_or_default();
    assert!(effective.hdr, "HDR is the half the browser host can run");
    assert!(
        effective.bloom.enabled,
        "the authored default asks for the calibration it was written for; \
         whether the platform can DRAW it is BLOOM_RUNS_ON_THIS_TARGET's \
         question, not this flag's"
    );
    assert!(effective.lod_fade_secs > 0.0 && effective.materialise_secs > 0.0);
}

/// The authored config is platform-BLIND: the same TOML parses to the same
/// `BloomConfig` on every target, so a world file cannot mean two things.
/// The platform enters one place only — the component insertion.
#[test]
fn the_authored_bloom_block_parses_the_same_on_every_target() {
    let on = parse_world("[render.bloom]\nenabled = true\nintensity = 0.15\n")
        .expect("must parse")
        .render
        .expect("render block")
        .bloom;
    assert!(on.enabled);
    assert_eq!(on.intensity, 0.15);

    // And a world that does not want it says so, everywhere.
    let off = parse_world("[render.bloom]\nenabled = false\n")
        .expect("must parse")
        .render
        .expect("render block")
        .bloom;
    assert!(!off.enabled);
}

/// A designer can author one number and inherit the rest — the same partial
/// authoring `[ambient_light]` allows, which is what makes a `[render]`
/// block a tuning knob rather than a full re-specification.
#[test]
fn a_render_block_accepts_partial_fields() {
    let toml = r#"
[render]
lod_fade_secs = 0.4
"#;
    let cfg = parse_world(toml).expect("must parse");
    let render = cfg.render.expect("render present");
    assert_eq!(render.lod_fade_secs, 0.4);
    assert_eq!(
        render.materialise_secs,
        RenderConfig::default().materialise_secs,
        "an unauthored field keeps the documented default"
    );
    assert!(render.hdr, "and so does an unauthored sub-block's field");
}

/// The retreat path, authored end to end: a world that wants the old
/// clipped picture back says so in one line. And the forward path: bloom is
/// one authored key away for the day the platform can draw it.
#[test]
fn a_render_block_can_turn_hdr_off_and_bloom_on() {
    let cfg = parse_world("[render]\nhdr = false\n").expect("must parse");
    assert!(!cfg.render.expect("render present").hdr);

    let cfg = parse_world("[render.bloom]\nenabled = true\n").expect("must parse");
    assert!(cfg.render.expect("render present").bloom.enabled);
}

/// Every tonemapper Bevy offers is nameable in snake_case, so the display
/// transform is a designer's decision rather than a recompile.
#[test]
fn a_render_block_names_its_tonemapper() {
    for (authored, want) in [
        ("none", TonemapChoice::None),
        ("reinhard", TonemapChoice::Reinhard),
        ("reinhard_luminance", TonemapChoice::ReinhardLuminance),
        ("aces_fitted", TonemapChoice::AcesFitted),
        ("ag_x", TonemapChoice::AgX),
        (
            "somewhat_boring_display_transform",
            TonemapChoice::SomewhatBoringDisplayTransform,
        ),
        ("tony_mc_mapface", TonemapChoice::TonyMcMapface),
        ("blender_filmic", TonemapChoice::BlenderFilmic),
    ] {
        let toml = format!("[render]\ntonemapping = \"{authored}\"\n");
        let cfg = parse_world(&toml).expect("must parse");
        assert_eq!(cfg.render.expect("render present").tonemapping, want);
    }
}

/// A misspelled key is a content error, not a silently ignored one — the
/// same `deny_unknown_fields` contract the rest of the schema keeps.
#[test]
fn a_misspelled_render_key_is_refused() {
    assert!(parse_world("[render]\nbloomm = true\n").is_err());
    assert!(parse_world("[render.bloom]\nintesity = 0.5\n").is_err());
}

#[test]
fn parse_world_reads_top_level_audio_block() {
    let toml = r#"
[audio.red_alert]
siren_file   = "assets/sounds/red_alert_siren.ogg"
siren_volume = 0.7
music_file   = "assets/sounds/last_stand_in_space_looped.ogg"
music_volume = 0.35
"#;
    let cfg = parse_world(toml).expect("must parse");
    let ra = cfg
        .audio
        .as_ref()
        .and_then(|a| a.red_alert.as_ref())
        .expect("red_alert present");
    assert_eq!(ra.siren_file, "assets/sounds/red_alert_siren.ogg");
    assert_eq!(ra.siren_volume, 0.7);
    assert_eq!(
        ra.music_file,
        "assets/sounds/last_stand_in_space_looped.ogg"
    );
    assert_eq!(ra.music_volume, 0.35);
}

#[test]
fn parse_world_omits_audio_when_block_missing() {
    let cfg = parse_world("").expect("empty TOML parses");
    assert!(cfg.audio.is_none());
}

// ── spawn_entity / destroy_entity actions (issue #417) ────────────────

#[test]
fn spawn_entity_action_reads_a_position() {
    let toml = r#"
[[action]]
type          = "spawn_entity"
template_path = "assets/entities/ship_harrow_destroyer.toml"
name          = "raider_beta"
position      = [100.0, 0.0, -50.0]
rotation      = [0.0, 1.5707963, 0.0]
scale         = [2.0, 2.0, 2.0]
"#;
    let parsed = actions(toml).expect("must parse");
    assert_eq!(parsed.len(), 1);
    match &parsed[0] {
        TriggerAction::SpawnEntity {
            template_path,
            name,
            anchor,
            position,
            rotation,
            scale,
            groups: _,
            overrides: _,
        } => {
            assert_eq!(template_path, "assets/entities/ship_harrow_destroyer.toml");
            assert_eq!(name, "raider_beta");
            assert!(anchor.is_none());
            assert_eq!(*position, Some([100.0, 0.0, -50.0]));
            let rotation = rotation.expect("rotation present");
            assert_eq!(rotation[0], 0.0);
            assert!((rotation[1] - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
            assert_eq!(rotation[2], 0.0);
            assert_eq!(*scale, Some([2.0, 2.0, 2.0]));
        }
        other => panic!("expected SpawnEntity, got {other:?}"),
    }
}

#[test]
fn spawn_entity_action_reads_an_anchor() {
    let toml = r#"
[[action]]
type          = "spawn_entity"
template_path = "assets/entities/ship_harrow_destroyer.toml"
name          = "raider_at_anchor"
anchor        = "patrol_alpha"
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::SpawnEntity {
            anchor,
            position,
            rotation,
            scale,
            ..
        } => {
            assert_eq!(anchor.as_deref(), Some("patrol_alpha"));
            assert!(position.is_none());
            assert!(rotation.is_none());
            assert!(scale.is_none());
        }
        other => panic!("expected SpawnEntity, got {other:?}"),
    }
}

#[test]
fn spawn_entity_rejects_a_missing_template_path() {
    let toml = r#"
[[action]]
type     = "spawn_entity"
name     = "x"
position = [0.0, 0.0, 0.0]
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(
        err.contains("template_path"),
        "error must mention template_path: {err}"
    );
}

#[test]
fn spawn_entity_rejects_a_missing_name() {
    let toml = r#"
[[action]]
type          = "spawn_entity"
template_path = "t.toml"
position      = [0.0, 0.0, 0.0]
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(err.contains("name"), "error must mention name: {err}");
}

#[test]
fn spawn_entity_rejects_both_anchor_and_position() {
    let toml = r#"
[[action]]
type          = "spawn_entity"
template_path = "t.toml"
name          = "x"
anchor        = "a"
position      = [0.0, 0.0, 0.0]
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(
        err.contains("anchor") && err.contains("position"),
        "error must mention both: {err}"
    );
}

#[test]
fn spawn_entity_rejects_neither_anchor_nor_position() {
    let toml = r#"
[[action]]
type          = "spawn_entity"
template_path = "t.toml"
name          = "x"
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(
        err.contains("anchor") || err.contains("position"),
        "error must mention anchor/position: {err}"
    );
}

#[test]
fn destroy_entity_action_parses() {
    let toml = r#"
[[action]]
type   = "destroy_entity"
entity = "raider_beta"
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::DestroyEntity { entity } => {
            assert_eq!(entity, "raider_beta");
        }
        other => panic!("expected DestroyEntity, got {other:?}"),
    }
}

#[test]
fn destroy_entity_rejects_a_missing_entity() {
    let toml = r#"
[[action]]
type = "destroy_entity"
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(err.contains("entity"), "error must mention entity: {err}");
}

#[test]
fn add_faction_enemy_action_parses() {
    let toml = r#"
[[action]]
type    = "add_faction_enemy"
faction = "Harrow"
enemy   = "Federation"
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::AddFactionEnemy { faction, enemy } => {
            assert_eq!(faction, "Harrow");
            assert_eq!(enemy, "Federation");
        }
        other => panic!("expected AddFactionEnemy, got {other:?}"),
    }
}

#[test]
fn remove_faction_enemy_action_parses() {
    let toml = r#"
[[action]]
type    = "remove_faction_enemy"
faction = "Harrow"
enemy   = "Federation"
"#;
    let parsed = actions(toml).expect("must parse");
    match &parsed[0] {
        TriggerAction::RemoveFactionEnemy { faction, enemy } => {
            assert_eq!(faction, "Harrow");
            assert_eq!(enemy, "Federation");
        }
        other => panic!("expected RemoveFactionEnemy, got {other:?}"),
    }
}

#[test]
fn add_faction_enemy_rejects_a_missing_faction() {
    let toml = r#"
[[action]]
type  = "add_faction_enemy"
enemy = "Federation"
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(err.contains("faction"), "error must mention faction: {err}");
}

#[test]
fn add_faction_enemy_rejects_a_missing_enemy() {
    let toml = r#"
[[action]]
type    = "add_faction_enemy"
faction = "Harrow"
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(err.contains("enemy"), "error must mention enemy: {err}");
}

#[test]
fn remove_faction_enemy_rejects_a_missing_faction() {
    let toml = r#"
[[action]]
type  = "remove_faction_enemy"
enemy = "Federation"
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(err.contains("faction"), "error must mention faction: {err}");
}

#[test]
fn remove_faction_enemy_rejects_a_missing_enemy() {
    let toml = r#"
[[action]]
type    = "remove_faction_enemy"
faction = "Harrow"
"#;
    let err = actions(toml).expect_err("must reject");
    assert!(err.contains("enemy"), "error must mention enemy: {err}");
}

// ── The sides of a labour dispute (issue #1035) ──────────────────────────

#[test]
fn parse_world_reads_the_workforce_table_in_authored_order() {
    let toml = r#"
[[workforce]]
id = "skyway_workers"
label = "world.fs.workforce.workers.label"
on_strike = true
disposition = 30

[[workforce]]
id = "havelock_operations"
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.workforces.len(), 2);
    assert_eq!(cfg.workforces[0].id, "skyway_workers");
    assert!(cfg.workforces[0].on_strike);
    assert_eq!(cfg.workforces[0].disposition, 30);
    // The second side takes both defaults: nobody said they were out, and
    // nobody wrote down what they think of the crew.
    assert_eq!(cfg.workforces[1].id, "havelock_operations");
    assert!(
        !cfg.workforces[1].on_strike,
        "a `[[workforce]]` block declares a party, not a dispute — a side is at \
         work until a world says otherwise"
    );
    assert_eq!(cfg.workforces[1].disposition, 50);
    assert!(cfg.workforces[1].label.is_empty());
}

#[test]
fn parse_world_refuses_a_duplicate_workforce_id_naming_both_entries() {
    let toml = r#"
[[workforce]]
id = "skyway_workers"

[[workforce]]
id = "skyway_workers"
on_strike = true
"#;
    let err = parse_world(toml).expect_err("must refuse");
    assert!(err.contains("skyway_workers"), "{err}");
    assert!(
        err.contains("#0") && err.contains("#1"),
        "names both: {err}"
    );
}

#[test]
fn parse_world_refuses_a_disposition_off_its_authored_scale() {
    let toml = r#"
[[workforce]]
id = "skyway_workers"
disposition = 400
"#;
    let err = parse_world(toml).expect_err("must refuse");
    assert!(err.contains("disposition"), "{err}");
}

#[test]
fn a_world_that_declares_no_workforce_parses_with_an_empty_table() {
    let cfg = parse_world("").expect("must parse");
    assert!(
        cfg.workforces.is_empty(),
        "every world written before this vocabulary existed is unchanged by it"
    );
}

// ── Named mission deadlines (issue #1024) ────────────────────────────────

#[test]
fn parse_world_reads_the_deadline_table_in_authored_order() {
    let toml = r#"
[[deadline]]
id = "transfer_window_opens"
label = "world.fs.deadline.transfer_window.label"
due_secs = 600
visible = true

[[deadline]]
id = "stabiliser_failure"
due_secs = 900
"#;
    let cfg = parse_world(toml).expect("must parse");
    assert_eq!(cfg.deadlines.len(), 2);
    assert_eq!(cfg.deadlines[0].id, "transfer_window_opens");
    assert_eq!(cfg.deadlines[0].due_secs, 600);
    assert!(cfg.deadlines[0].visible);
    // Both optional fields default: a deadline the crew never sees needs no
    // label, and `visible` is false until a mission says otherwise.
    assert_eq!(cfg.deadlines[1].id, "stabiliser_failure");
    assert!(cfg.deadlines[1].label.is_empty());
    assert!(
        !cfg.deadlines[1].visible,
        "a deadline is the mission's business until it says otherwise"
    );
}

#[test]
fn parse_world_refuses_a_duplicate_deadline_id_naming_both_entries() {
    // The id is the ONLY handle script has on a deadline
    // (`ctx.deadlines.slip("id", …)`), so a duplicate is not a cosmetic
    // clash — it is two records competing for every mutation. The refusal
    // names both entries so a designer sees which two lines to reconcile
    // rather than which one silently won.
    let toml = r#"
[[deadline]]
id = "window"
due_secs = 600

[[deadline]]
id = "other"
due_secs = 700

[[deadline]]
id = "window"
due_secs = 900
"#;
    let err = parse_world(toml).expect_err("a duplicate deadline id must be refused");
    assert!(err.contains("window"), "names the id: {err}");
    assert!(
        err.contains("#0") && err.contains("#2"),
        "names BOTH entries: {err}"
    );
    assert!(
        err.contains("600") && err.contains("900"),
        "and carries each one's due time so they can be told apart: {err}"
    );
}

#[test]
fn parse_world_refuses_an_empty_deadline_id() {
    let toml = r#"
[[deadline]]
id = "  "
due_secs = 600
"#;
    let err = parse_world(toml).expect_err("an unaddressable deadline must be refused");
    assert!(err.contains("empty id"), "{err}");
}

#[test]
fn a_world_with_no_deadline_table_has_none() {
    // The compatibility half: every shipped world today authors no
    // `[[deadline]]`, and must parse to exactly what it did before.
    let cfg = parse_world("[global]\nseed = 1\n").expect("must parse");
    assert!(cfg.deadlines.is_empty());
}
