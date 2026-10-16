use super::*;
use crate::ai::server::AiHighFidelity;
use crate::console::weapons::TacticalRadarSelection;
use crate::core::messages::{AdmittedCommands, CoordinationPayload};
use crate::ship::control_source::ControlSource;
use crate::ship::shields::ShipShields;
use crate::ship_plugin::{CoordinationEnqueue, ShipConfigComponent, ShipSystemControlSources};

#[derive(Resource, Default)]
struct CoordBox(Vec<CoordinationEnqueue>);

fn collect_coord(mut reader: MessageReader<CoordinationEnqueue>, mut box_: ResMut<CoordBox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

// This advisory-owner fixture begins AFTER acquisition. The real paid scanner
// lifecycle, pause and historical configuration are covered in focused_tests.
fn acquired_hint_fixture() -> crate::server_app::ShipSystemBlackboards {
    let owner = crate::ship::system_registry::sensors_system_id();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::science::scan_blackboard_key_for(&owner),
        crate::core::messages::SystemBlackboard::Scan(crate::core::messages::ScanBlackboard {
            owner: Some(owner),
            capable: true,
            history: vec![crate::core::messages::ScanReadingSnapshot {
                aim: phoenix_model::focused_enquiry::FocusedAim::Shields,
                subject_uuid: "target-1".into(),
                taken_at_tick: 0,
                shield_finding: phoenix_model::shield_enquiry::ShieldFinding::acquire(
                    0.75,
                    Default::default(),
                    0.01,
                ),
                ..Default::default()
            }],
            ..Default::default()
        }),
    );
    boards
}

// ── tick_frequency_hint_high_fidelity ─────────────────────────────────

/// Test-only glue (issue #829): seed each ship's viewscreen combat_lock from
/// its `TacticalRadarSelection` before the hint emitter reads the frozen
/// fact — standing in for the radar publisher + viewscreen aggregator the
/// full app runs, exactly like the other frequency/firing test harnesses.
fn seed_viewscreen_from_selection(
    mut q: Query<
        (
            Option<&crate::console::weapons::TacticalRadarSelection>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (tac, mut bbs) in q.iter_mut() {
        let combat_lock = tac.and_then(|t| t.0.clone());
        let mut vbb = match bbs
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(v)) => v.clone(),
            _ => crate::core::messages::ViewscreenBlackboard::default(),
        };
        vbb.combat_lock = combat_lock;
        bbs.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            crate::core::messages::SystemBlackboard::Viewscreen(vbb),
        );
    }
}

fn freq_hint_test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    // Manual `Time::advance_by` (mirroring `ai::server`'s LOD tests)
    // rather than `TimePlugin` + `TimeUpdateStrategy`: the latter reports
    // a zero delta on the frame it's added, which would otherwise force
    // every test here to burn an extra warm-up `app.update()`.
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::sensors::SensorsAiConfigResource>()
        .init_resource::<CoordBox>()
        .add_message::<CoordinationEnqueue>()
        .add_systems(
            Update,
            (
                seed_viewscreen_from_selection,
                tick_frequency_hint_high_fidelity,
            )
                .chain(),
        )
        .add_systems(PostUpdate, collect_coord);

    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Ai,
    );

    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("target-1".into()),
            ShipShields(crate::weapons::shield::ShieldSystem::default(), 0.75),
        ))
        .id();

    let source = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            control_sources,
            crate::ship_plugin::ShipConfigComponent(sensors_ship_config()),
            acquired_hint_fixture(),
            TacticalRadarSelection(Some("target-1".into())),
            ShipFrequencyHintState::default(),
            AiHighFidelity,
        ))
        .id();

    let _ = target;
    app.insert_resource(SourceShip(source));
    app
}

#[derive(Resource)]
struct SourceShip(Entity);

/// Advance the hint by `secs` of AI-TICK time.
///
/// Issue #889: the hint emitter runs under `run_if(ai_tick_ready)` and
/// advances its delay by one authored tick period per run, not by
/// `Time::delta` — otherwise the gate would stretch the authored delay by
/// the frame-rate-to-tick-rate ratio. So the fixture now drives AI TICKS
/// rather than one oversized wall-clock jump: `secs` of hint time is
/// `secs * ai_tick_hz` updates. Wall-clock is advanced alongside purely so
/// any other `Time` reader in the harness sees a consistent world.
fn tick_with_dt(app: &mut App, secs: f32) {
    let hz = crate::entities::config::GlobalConfig::default().ai_tick_hz;
    let period = 1.0 / hz;
    let ticks = (secs * hz).ceil().max(1.0) as usize;
    for _ in 0..ticks {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(period));
        app.update();
    }
}

#[test]
fn frequency_hint_propagates_after_the_authored_reaction_delay() {
    let mut app = freq_hint_test_app();
    // 4s exceeds the 3s default delay in a single tick.
    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    let hint = coord
        .iter()
        .find(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
        .expect("expected a FrequencyHint CoordinationEnqueue after the delay elapses");

    match &hint.payload {
        CoordinationPayload::FrequencyHint { frequency } => {
            assert!(
                (*frequency - 0.75).abs() < f32::EPSILON,
                "hint should carry the locked target's shield frequency"
            );
        }
        other => panic!("expected FrequencyHint, got {other:?}"),
    }
    assert_eq!(
        hint.address,
        crate::core::messages::CoordinationAddress::Station(crate::core::messages::StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        ),),
        "frequency hint should address Tactical"
    );
}

#[test]
fn live_enemy_frequency_without_acquired_evidence_never_supplies_a_hint() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(source)
        .unwrap()
        .0
        .clear();
    tick_with_dt(&mut app, 10.0);
    assert!(app.world().resource::<CoordBox>().0.is_empty());
}

#[test]
fn changing_current_enemy_frequency_cannot_upgrade_a_retained_finding() {
    let mut app = freq_hint_test_app();
    let mut query = app.world_mut().query::<&mut ShipShields>();
    for mut shields in query.iter_mut(app.world_mut()) {
        shields.1 = 0.1;
    }
    tick_with_dt(&mut app, 4.0);
    assert!(app.world().resource::<CoordBox>().0.iter().any(|message|matches!(&message.payload,CoordinationPayload::FrequencyHint{frequency} if *frequency==0.75)));
}

#[test]
fn npc_frequency_hint_reads_its_own_tuning_not_the_global_resource() {
    // Issue #738 isolation, mirroring the shields case: the hint emitter
    // used to resolve its delay as
    // `per_entity_component.unwrap_or(&*global_resource)` while iterating
    // every ship, so any write to that Resource would have applied
    // fleet-wide. Nothing writes it today (unlike the shields Resource,
    // which `server_app` dual-writes from the player ship) — this test
    // seeds it by hand so the leak stays closed if anything ever does.
    //
    // The global Resource here carries an eager 0.5s delay; one tick of
    // 1.0s therefore fires under the global tuning but not under the
    // parse-time default (3.0s).
    let mut app = freq_hint_test_app();
    let npc = app.world().resource::<SourceShip>().0;
    app.insert_resource(crate::ship::sensors::SensorsAiConfigResource {
        frequency_hint_delay_secs: 0.5,
    });

    let mut tuned_sources = ShipSystemControlSources::default();
    tuned_sources.0.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Ai,
    );
    let tuned = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            tuned_sources,
            crate::ship_plugin::ShipConfigComponent(sensors_ship_config()),
            acquired_hint_fixture(),
            TacticalRadarSelection(Some("target-1".into())),
            ShipFrequencyHintState::default(),
            AiHighFidelity,
            crate::ship::sensors::SensorsAiConfigResource {
                frequency_hint_delay_secs: 0.5,
            },
        ))
        .id();

    tick_with_dt(&mut app, 1.0);

    let coord = &app.world().resource::<CoordBox>().0;
    let hinting_ships: Vec<Entity> = coord
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
        .map(|m| m.source_entity)
        .collect();
    assert!(
        hinting_ships.contains(&tuned),
        "a ship carrying the eager 0.5s delay on its own entity must hint after 1.0s"
    );
    assert!(
        !hinting_ships.contains(&npc),
        "an NPC without its own sensors-AI tuning must fall back to the parse-time \
         3.0s default, never to the global Resource holding the player ship's tuning"
    );
}

/// Issue #873, replacing `ai_frequency_hint_skips_ships_where_sensors_are
/// _not_ai_operated`.
///
/// That test pinned the branch this issue exists to delete: the emitter
/// stood down whenever a human held Sensors, so the ship's frequency
/// advisory came from the AI operator path rather than from authoritative
/// state. Its premise is gone, so it is re-pointed at the rule that
/// replaced it — the same fact, from the same state, for a human-held
/// console — with a strictly stronger assertion: not merely that something
/// is emitted, but that it carries the human origin as a routing TAG.
///
/// `sender_origin == Human` is the whole point. It proves the emitter read
/// the control source (so the tag is live, not a hardcoded `Ai` the way
/// `tick_power_brownout_advisory` used to stamp one) while proving the
/// value did not gate the emission.
#[test]
fn frequency_hint_fires_from_a_human_held_sensors_station_and_tags_it_human() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;
    app.world_mut()
        .entity_mut(source)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::sensors_system_id(),
            ControlSource::Human,
        );

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    let hint = coord
        .iter()
        .find(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
        .expect(
            "a human-held Sensors console must still feed the ship's coordination bus \
             — the fact comes from authoritative state, not from who is sitting there",
        );
    assert_eq!(
        hint.sender_origin,
        ControlSource::Human,
        "sender_origin must report the live control source, and be used only as a \
         delivery-routing tag"
    );
    match &hint.payload {
        CoordinationPayload::FrequencyHint { frequency } => assert!(
            (*frequency - 0.75).abs() < f32::EPSILON,
            "the human-sent hint must carry the same authoritative shield frequency \
             the AI-sent one does"
        ),
        other => panic!("expected FrequencyHint, got {other:?}"),
    }
}

// ── The retired `auto_hint` rating gate (issue #873) ────────────────────
//
// These two tests used to pin a claimed/unclaimed split copied from
// `ai_torpedo_auto_fire`: once a human session held Sensors, the hint
// additionally required that holder's active rating to declare `auto_hint`
// in its `ai_tuning` table, and stayed silent otherwise.
//
// That is a coordination fact whose emission turned on the presence of a
// human, which AGENTS.md rule 6 forbids and issue #873 removes. Both are
// kept — the fixture is exactly the interesting one — and re-pointed at the
// surviving rule: the rating table is now irrelevant to emission in BOTH
// directions, which takes two tests to state and could not be stated by
// deleting either.

fn sensors_ship_config() -> crate::ship::config::ShipConfig {
    let toml = r#"
[[station]]
id = "sensors"
name = "Sensors"
description = "Long-range sensors."
rank = "Ens."

[[station.rating]]
name = "Assisted"
automated_systems = []
[station.rating.ai_tuning]
auto_hint = {}

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons control."
rank = "Ltn."

[[console]]
id = "science-console"
home_station = "sensors"
screen = "gui/battleship/sensors.html"
[[console.feed]]
source = "viewscreen"
kind = "Viewscreen"

[[console]]
id = "weapons-console"
home_station = "tactical"
screen = "gui/battleship/tactical.html"

[[system]]
id = "sensors"
kind = "sensors"
console = "science-console"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
console = "weapons-console"
"#;
    crate::ship::config::ShipConfig::from_toml(toml, &["sensors", "tactical_radar"]).unwrap()
}

#[test]
fn console_combat_lock_requires_the_reading_systems_exact_feed() {
    let mut config = sensors_ship_config();
    let sensors = crate::ship::system_registry::sensors_system_id();
    let tactical = crate::ship::system_registry::tactical_radar_system_id();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(
            crate::core::messages::ViewscreenBlackboard {
                combat_lock: Some("target-1".into()),
                ..Default::default()
            },
        ),
    );
    assert_eq!(
        console_combat_lock(&config, &sensors, &boards).as_deref(),
        Some("target-1")
    );
    // Co-hosting is not a feed, even when both Console home seats match.
    config.consoles[1].home_station = config.consoles[0].home_station.clone();
    assert_eq!(console_combat_lock(&config, &tactical, &boards), None);
    config.consoles[0].feeds[0].kind = phoenix_model::console::ConsoleObservationKind::Sensors;
    assert_eq!(console_combat_lock(&config, &sensors, &boards), None);
}

#[test]
fn frequency_hint_cannot_read_an_unbound_combat_lock() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;
    app.world_mut()
        .entity_mut(source)
        .get_mut::<ShipConfigComponent>()
        .unwrap()
        .0
        .consoles[0]
        .feeds
        .clear();
    tick_with_dt(&mut app, 10.0);
    assert!(
        app.world().resource::<CoordBox>().0.is_empty(),
        "a live target elsewhere on the ship does not grant Sensors its lock"
    );
}

/// Adds `ShipConfigComponent` + `ActiveStationRatings` to the ship spawned
/// by `freq_hint_test_app`, and a `Sessions` resource with the Sensors
/// station claimed by `holder_token`. Returns the source ship entity.
fn claim_sensors_station(app: &mut App, holder_token: &str, rating: &str) -> Entity {
    let source = app.world().resource::<SourceShip>().0;
    let sensors_station = crate::core::messages::StationId("sensors".into());

    let mut sm = crate::lobby::session::SessionManager::new();
    sm.register(holder_token.into(), "Operator".into()).unwrap();
    sm.set_station(holder_token, Some(sensors_station.clone()));
    app.insert_resource(crate::lobby::Sessions(sm));

    let mut active_ratings = crate::ship_plugin::ActiveStationRatings::default();
    active_ratings.0.insert(sensors_station, rating.into());

    app.world_mut()
        .entity_mut(source)
        .insert(crate::ship_plugin::ShipConfigComponent(
            sensors_ship_config(),
        ))
        .insert(active_ratings);

    source
}

#[test]
fn frequency_hint_fires_when_a_claimed_station_rating_declares_auto_hint() {
    let mut app = freq_hint_test_app();
    claim_sensors_station(&mut app, "op1", "Assisted");

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    assert!(
        coord
            .iter()
            .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
        "a claimed Sensors station whose active rating declares auto_hint \
         must be hinted"
    );
}

/// The half that changed. `"Std"` is a rating with no `ai_tuning` table at
/// all, held by a live session — the configuration that used to silence the
/// ship's frequency advisory completely.
#[test]
fn frequency_hint_fires_when_a_claimed_station_rating_lacks_auto_hint() {
    let mut app = freq_hint_test_app();
    claim_sensors_station(&mut app, "op1", "Std");

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    assert!(
        coord
            .iter()
            .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
        "a station rating's ai_tuning table must not decide whether a coordination \
         fact derived from authoritative state is emitted at all (issue #873): a human \
         on a rating without auto_hint still feeds the ship's backfilled Tactical"
    );
}

#[test]
fn frequency_hint_fires_unconditionally_when_sensors_station_is_unclaimed() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;

    // Ship config + ratings present, but no session holds the station —
    // e.g. an NPC, or the player ship before anyone takes Sensors.
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.world_mut()
        .entity_mut(source)
        .insert(crate::ship_plugin::ShipConfigComponent(
            sensors_ship_config(),
        ))
        .insert(crate::ship_plugin::ActiveStationRatings::default());

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    assert!(
        coord
            .iter()
            .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
        "an unclaimed Sensors station must be hinted unconditionally, \
         regardless of any rating's ai_tuning table"
    );
}

// Power tree hosts have independent finite-hardware/knowledge fixtures.
#[path = "power_ai_tests.rs"]
mod power_ai_tests;

// ── AI torpedo loading (admitted-command path) ──────────────────

/// One tube, `volley_max = 2`, no per-tube AI override — so its resolved
/// `ai_target_count` is `volley_max`.
fn torpedo_load_app(tube_source: ControlSource) -> (App, Entity) {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    // `Sessions` because the emit goes through the admission seam
    // (`emit_ai_command`), which asks it about station tenure.
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ))
    .add_systems(
        Update,
        (
            ai_torpedo_load,
            crate::console::weapons::handle_set_torpedo_volley_target,
        )
            .chain(),
    );

    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::torpedo_magazine_system_id(),
        ControlSource::Ai,
    );
    control_sources.0.set(
        crate::ship::system_registry::torpedo_tube_system_id("fore_port").unwrap(),
        tube_source,
    );

    let torpedoes = crate::weapons::torpedo::TorpedoSystem::from_configs(
        &[crate::entities::config::TorpedoTubeConfig {
            marking: None,
            load_priority: 100,
            targets: None,
            impact: phoenix_model::damage::DamageProfile::default(),
            id: "fore_port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 2,
            ai_target_count: None,
            ai: None,
        }],
        crate::weapons::torpedo::TorpedoConfig::default(),
    );

    let e = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            control_sources,
            crate::console::weapons::TorpedoSystemResource(torpedoes),
            AdmittedCommands::default(),
            // The SHIPPED authored per-tube policy. Since #885b stage 5d a
            // tube with no entry in `TorpedoTubeAiPolicies` is never ordered
            // to load — there is no synthesised stand-in.
            crate::console::weapons::TorpedoTubeAiPolicies(
                [(
                    "fore_port".to_string(),
                    crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube")
                        .to_policy()
                        .expect("the shipped torpedo-tube policy decodes"),
                )]
                .into_iter()
                .collect(),
            ),
        ))
        .id();
    (app, e)
}

fn tube_target_count(app: &App, e: Entity) -> u32 {
    app.world()
        .entity(e)
        .get::<crate::console::weapons::TorpedoSystemResource>()
        .unwrap()
        .0
        .tube("fore_port")
        .unwrap()
        .target_count
}

/// The gap this system closes: an AI-crewed ship now asks for its tubes to
/// be loaded, and it does so through the same `SetTorpedoVolleyTarget`
/// command a human console sends — so the order lands on an NPC's own
/// torpedo system, which the LocalShip-only handler could never do.
#[test]
fn ai_torpedo_load_sets_volley_target_through_admitted_commands() {
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    app.update();

    assert_eq!(
        tube_target_count(&app, e),
        2,
        "an AI-operated tube should be ordered to its configured \
         ai_target_count (volley_max = 2 here)"
    );
    let admitted = app.world().entity(e).get::<AdmittedCommands>().unwrap();
    assert_eq!(
        admitted.0.len(),
        1,
        "exactly one SetTorpedoVolleyTarget should have been issued"
    );
    assert!(
        matches!(
            admitted.0[0].payload,
            crate::core::messages::SystemControlPayload::SetTorpedoVolleyTarget { count: 2 }
        ),
        "the AI must issue the ordinary console command, not poke state"
    );
}

/// The tube is already where the AI wants it, so no second order goes out.
#[test]
fn ai_torpedo_load_does_not_reissue_an_identical_order() {
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    app.update();
    app.update();

    let admitted = app.world().entity(e).get::<AdmittedCommands>().unwrap();
    assert_eq!(
        admitted.0.len(),
        1,
        "the AI must not re-issue a volley order the tube already satisfies"
    );
}

/// A human-crewed tube is the operator's to load. The AI must not touch it
/// — this is the behaviour a non-zero `target_count` default in
/// `TorpedoSystem::from_configs` would have broken.
#[test]
fn ai_torpedo_load_leaves_human_controlled_tubes_alone() {
    let (mut app, e) = torpedo_load_app(ControlSource::Human);
    app.update();

    assert_eq!(
        tube_target_count(&app, e),
        0,
        "a Human-controlled tube must stay exactly as its operator left it"
    );
    assert!(app
        .world()
        .entity(e)
        .get::<AdmittedCommands>()
        .unwrap()
        .0
        .is_empty());
}

/// Offline (rating- or damage-driven) means nobody loads it, AI included.
#[test]
fn ai_torpedo_load_skips_offline_tubes() {
    let (mut app, e) = torpedo_load_app(ControlSource::Offline);
    app.update();

    assert_eq!(tube_target_count(&app, e), 0);
}

// ── Per-tube LOAD policy gate (issue #782) ──────────────────────────────

/// Attach a single-tube `TorpedoTubeAiPolicies` map to `e` for the
/// `fore_port` tube, built from an authored `when` guard on the
/// `torpedo_load` channel.
fn attach_load_policy(app: &mut App, e: Entity, when: &str) {
    let ai = crate::entities::config::FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 0,
            channel: crate::entities::config::TORPEDO_LOAD_CHANNEL.into(),
            when: when.into(),
            verb: crate::entities::config::TORPEDO_LOAD_VERB.into(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let mut map = std::collections::HashMap::new();
    map.insert("fore_port".to_string(), ai.to_policy().unwrap());
    app.world_mut()
        .entity_mut(e)
        .insert(crate::console::weapons::TorpedoTubeAiPolicies(map));
}

/// An idle tube policy holds the load: no `SetTorpedoVolleyTarget` is issued
/// even though the tube is AI-operated and off its configured volley target.
#[test]
fn ai_torpedo_load_idle_tube_policy_holds() {
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    let mut map = std::collections::HashMap::new();
    map.insert(
        "fore_port".to_string(),
        crate::ai::policy::AiPolicy {
            idle: true,
            ..Default::default()
        },
    );
    app.world_mut()
        .entity_mut(e)
        .insert(crate::console::weapons::TorpedoTubeAiPolicies(map));
    app.update();

    assert_eq!(
        tube_target_count(&app, e),
        0,
        "an idle tube policy must hold the AI load order"
    );
    assert!(
        app.world()
            .entity(e)
            .get::<AdmittedCommands>()
            .unwrap()
            .0
            .is_empty(),
        "no volley command should be admitted when the tube policy is idle"
    );
}

/// The #779 empty-facts lesson: the host seeds real per-tube facts, so a
/// `fact(...)` guard actually evaluates. A guard that can never hold over the
/// live magazine count (`fact(magazine) > 100`, magazine is 10) holds the
/// load; the complementary guard (`fact(magazine) > 0`) fires it — proving the
/// facts are seeded, not empty.
#[test]
fn ai_torpedo_load_fact_guard_fires_over_seeded_facts() {
    // Unsatisfiable guard → hold.
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    attach_load_policy(&mut app, e, "fact(magazine) > 100");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        0,
        "a load guard that never holds over the seeded magazine fact must hold"
    );

    // Satisfiable guard → fire. If facts were empty (#779), `fact(magazine)`
    // would read 0 and this guard would also hold — so a fire here proves the
    // magazine fact was seeded.
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    attach_load_policy(&mut app, e, "fact(magazine) > 0");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        2,
        "a load guard satisfied by the seeded magazine fact must fire the order"
    );
}

/// Issue #891 stage 2, per-host both-directions proof for the Torpedo tube
/// LOAD host: a `flag()` guard fires when the scenario sets the flag and
/// reads false when it does not — through the full decide → admit → apply
/// pipeline, not the policy evaluator alone.
#[test]
fn ai_torpedo_load_flag_guard_reads_the_world_in_both_directions() {
    // Flag CLEAR → the guard reads false and the load holds.
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    attach_load_policy(&mut app, e, "flag(resupply_authorised)");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        0,
        "with the world flag clear the load guard must read false and hold"
    );

    // Flag SET → the SAME guard fires and the volley order lands.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("resupply_authorised");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        2,
        "with the world flag set the same guard must fire the load order"
    );
}

#[test]
fn concealed_contact_stops_high_fidelity_frequency_hints_until_normal() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;
    app.world_mut()
        .entity_mut(source)
        .insert(crate::entities::spawner::EntityUuid("observer".into()));
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "target-1",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    tick_with_dt(&mut app, 4.0);
    assert!(!app
        .world()
        .resource::<CoordBox>()
        .0
        .iter()
        .any(|row| matches!(row.payload, CoordinationPayload::FrequencyHint { .. })));
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "target-1",
        crate::gm::operations::contact::ContactMode::Normal,
    );
    tick_with_dt(&mut app, 4.0);
    assert!(app
        .world()
        .resource::<CoordBox>()
        .0
        .iter()
        .any(|row| matches!(row.payload, CoordinationPayload::FrequencyHint { .. })));
}
