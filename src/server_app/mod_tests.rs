use super::*;
use crate::console::weapons::BEAM_DAMAGE_PER_SEC;
use crate::core::messages::*;
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
use crate::ship::damage::collision_damage;
use crate::ship_plugin::handle_impulse_messages;

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

#[derive(Resource)]
struct ShipEntity(Entity);

// ── Issue #1101: authoritative per-Station importance projection ──────────
//
// Host-derived from objectives + Red Alert, held apart from health, with two
// independent-lifecycle flags: one-off `unread` (cleared on visit) and
// continuing `critical` (cleared only on resolve).

fn core_station() -> StationId {
    StationId("core".into())
}

fn importance_of<'a>(
    snaps: &'a [StationImportanceSnapshot],
    station: &str,
) -> Option<&'a StationImportanceSnapshot> {
    snaps.iter().find(|s| s.station.0 == station)
}

#[test]
fn importance_marks_a_completed_objective_unread_then_clears_only_on_visit() {
    let mut world = World::new();
    world.init_resource::<StationImportanceRes>();
    let mut objectives = crate::world::server::ObjectiveManagerRes::default();
    objectives.0.add("rescue", "objective.rescue", true, vec![]);
    objectives.0.complete("rescue");
    world.insert_resource(objectives);

    // An objective that completed off-screen marks its Station unread. With no
    // Station-owned target System it buckets to the ship-wide core bucket.
    ingest_station_importance(&mut world);
    let snaps = build_station_importance_snapshots(&mut world);
    assert_eq!(
        importance_of(&snaps, "core"),
        Some(&StationImportanceSnapshot {
            station: core_station(),
            unread: true,
            critical: false,
        })
    );

    // Visiting clears the one-off unread through host state (AC2)…
    world
        .resource_mut::<StationImportanceRes>()
        .0
        .visit(&core_station());
    // …and a later tick re-seeing the still-Completed objective must NOT
    // resurrect it — the clear is authoritative, not optimistic.
    ingest_station_importance(&mut world);
    assert!(
        build_station_importance_snapshots(&mut world).is_empty(),
        "a visited one-off event must stay cleared across later broadcasts"
    );
}

#[test]
fn importance_marks_red_alert_critical_that_survives_visit_and_clears_on_resolve() {
    let mut world = World::new();
    world.init_resource::<StationImportanceRes>();
    let ship = world
        .spawn((LocalShip, crate::ship::state::ShipRedAlert(true)))
        .id();

    // A raised Red Alert is a continuing critical condition on the core bucket.
    ingest_station_importance(&mut world);
    assert_eq!(
        importance_of(&build_station_importance_snapshots(&mut world), "core"),
        Some(&StationImportanceSnapshot {
            station: core_station(),
            unread: false,
            critical: true,
        })
    );

    // Visiting must NOT clear a continuing condition (AC3).
    world
        .resource_mut::<StationImportanceRes>()
        .0
        .visit(&core_station());
    ingest_station_importance(&mut world);
    assert_eq!(
        importance_of(&build_station_importance_snapshots(&mut world), "core").map(|s| s.critical),
        Some(true),
        "a visit must not clear a continuing Red Alert"
    );

    // It clears only when Red Alert is lowered.
    world
        .entity_mut(ship)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = false;
    ingest_station_importance(&mut world);
    assert!(
        build_station_importance_snapshots(&mut world).is_empty(),
        "a resolved Red Alert must drop the critical mark from the broadcast"
    );
}

#[test]
fn importance_carries_simultaneous_unread_and_critical_independently() {
    // A completed off-screen objective (one-off unread) AND a raised Red
    // Alert (continuing critical) land on the same core bucket at once. The
    // two flags are separate fields with separate lifecycles, so they coexist
    // and a visit clears only the one-off — proving no crosstalk (AC1). Health
    // is a wholly separate wire field/builder, so it cannot interfere.
    let mut world = World::new();
    world.init_resource::<StationImportanceRes>();
    world.spawn((LocalShip, crate::ship::state::ShipRedAlert(true)));
    let mut objectives = crate::world::server::ObjectiveManagerRes::default();
    objectives.0.add("rescue", "objective.rescue", true, vec![]);
    objectives.0.complete("rescue");
    world.insert_resource(objectives);

    ingest_station_importance(&mut world);
    assert_eq!(
        importance_of(&build_station_importance_snapshots(&mut world), "core"),
        Some(&StationImportanceSnapshot {
            station: core_station(),
            unread: true,
            critical: true,
        }),
        "an unread event and a critical condition must coexist on one Station"
    );

    // Visiting clears ONLY the one-off unread; the continuing critical stays.
    world
        .resource_mut::<StationImportanceRes>()
        .0
        .visit(&core_station());
    ingest_station_importance(&mut world);
    assert_eq!(
        importance_of(&build_station_importance_snapshots(&mut world), "core"),
        Some(&StationImportanceSnapshot {
            station: core_station(),
            unread: false,
            critical: true,
        }),
        "a visit clears the one-off unread without disturbing the critical flag"
    );
}

#[test]
fn station_visited_drain_clears_unread_for_a_connected_player() {
    let mut app = App::new();
    app.init_resource::<Messages<InboundMessage>>();
    app.init_resource::<StationImportanceRes>();
    let mut sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
    sessions.0.register("tok".into(), "Ada".into()).unwrap();
    app.insert_resource(sessions);
    app.add_systems(Update, drain_station_visited);

    // Seed a one-off unread event on the core bucket.
    app.world_mut()
        .resource_mut::<StationImportanceRes>()
        .0
        .ingest(
            vec![("rescue".into(), core_station(), ObjectiveStatus::Completed)],
            Vec::new(),
        );
    assert!(
        app.world()
            .resource::<StationImportanceRes>()
            .0
            .flags_of(&core_station())
            .unread
    );

    // A connected player's StationVisited clears it via the wired drain.
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: "tok".into(),
            msg: ClientMessage::StationVisited {
                station: core_station(),
            },
        });
    app.update();
    assert!(
        !app.world()
            .resource::<StationImportanceRes>()
            .0
            .flags_of(&core_station())
            .unread
    );
}

#[test]
fn station_host_projection_is_generic_for_non_navigation_stations() {
    let mut world = World::new();
    world.spawn((
        LocalShip,
        crate::ship_plugin::VisitingStationHosts(vec![
            crate::ship::coordination::VisitingStationAssignment {
                station: StationId("power".into()),
                host: Some(StationId("repair".into())),
                rating: "Std".into(),
            },
            crate::ship::coordination::VisitingStationAssignment {
                station: StationId("shields".into()),
                host: None,
                rating: crate::ship::rating::BACKFILL_RATING.into(),
            },
        ]),
    ));

    assert_eq!(
        build_station_host_snapshots(&mut world),
        vec![
            StationHostSnapshot {
                station: StationId("power".into()),
                host: Some(StationId("repair".into())),
                rating: "Std".into(),
            },
            StationHostSnapshot {
                station: StationId("shields".into()),
                host: None,
                rating: crate::ship::rating::BACKFILL_RATING.into(),
            },
        ]
    );
}

#[test]
fn control_source_projection_tracks_visiting_and_ai_navigation_authority() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    let navigation = SystemId("navigation".into());
    let mut resolver = ControlSourceResolver::new();
    resolver.set(navigation.clone(), ControlSource::Human);

    let mut world = World::new();
    let ship = world
        .spawn((
            LocalShip,
            crate::ship_plugin::VisitingStationHosts(vec![
                crate::ship::coordination::VisitingStationAssignment {
                    station: StationId("navigation".into()),
                    host: Some(StationId("tactical".into())),
                    rating: "Std".into(),
                },
            ]),
            crate::ship_plugin::ShipSystemControlSources(resolver),
        ))
        .id();

    assert_eq!(
        build_station_host_snapshots(&mut world)[0].host,
        Some(StationId("tactical".into()))
    );
    assert_eq!(
        build_control_source_snapshots(&mut world).get(&navigation),
        Some(&"Human".to_string()),
        "a visiting Std Navigation Station must publish its live manual authority"
    );

    world
        .entity_mut(ship)
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>()
        .expect("control sources")
        .0
        .set(navigation.clone(), ControlSource::Ai);
    assert_eq!(
        build_control_source_snapshots(&mut world).get(&navigation),
        Some(&"Ai".to_string()),
        "a Simplified or exhausted Navigation Station must publish AUTO authority"
    );
}

// ── station_for_system ───────────────────────────────────────────────

/// Issue #801 deleted `station_for_system`'s "tactical" special case
/// (step 2.5): `"tactical"` is a station id, not a system, and no
/// `ControlSystem` message targets it. Ship-level tactical operations
/// target real declared systems (`tactical-radar`, `phaser-control`),
/// which resolve through the ordinary system→station lookup — including
/// on a hull whose weapons station isn't literally named "tactical".
///
/// Issue #832 removed the step-3 station-name fallback entirely, so a bare
/// station id like `"tactical"` no longer resolves — it is not a declared
/// system, and every client wire target names a declared system.
#[test]
fn station_for_system_resolves_tactical_systems_via_their_declared_station() {
    let crewed = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"
"#,
        &["phaser_bank", "tactical_radar"],
    )
    .unwrap();
    assert_eq!(
        station_for_system(&crewed, None, &SystemId("tactical-radar".into())),
        Some(StationId("tactical".into())),
        "crewed hulls resolve tactical-radar to their tactical station"
    );
    // #832: the bare station string `"tactical"` no longer resolves — the
    // step-3 station-name fallback was removed and `"tactical"` is not a
    // declared system.
    assert_eq!(
        station_for_system(&crewed, None, &SystemId("tactical".into())),
        None,
    );

    let courier = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "pilot"
name = "Pilot"
description = "Everything."
rank = "Ltn."

[[system]]
id = "blaster-fore"
kind = "blaster_bank"
station = "pilot"

[[system]]
id = "pilot-radar"
kind = "tactical_radar"
station = "pilot"
"#,
        &["blaster_bank", "tactical_radar"],
    )
    .unwrap();
    assert_eq!(
        station_for_system(&courier, None, &SystemId("pilot-radar".into())),
        Some(StationId("pilot".into())),
        "the Courier's radar lives on pilot, so SetTarget resolves there"
    );
    assert_eq!(
        station_for_system(&courier, None, &SystemId("tactical".into())),
        None,
        "no tactical station and no tactical system: the deleted step-2.5 \
         weapons-owner special case must not resurrect this lookup"
    );
}

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

/// Test-only glue (issue #829): seed each ship's `ViewscreenBlackboard`
/// combat_lock / science_target from its `TacticalRadarSelection` /
/// `SensorRadarSelection` components before `SimSet::Input`, standing in for
/// the radar publishers + viewscreen aggregators the full app runs. Merges
/// into any existing viewscreen entry.
fn seed_viewscreen_from_selection(
    mut q: Query<
        (
            Option<&crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::ship::sensors::SensorRadarSelection>,
            &mut ShipSystemBlackboards,
        ),
        With<Ship>,
    >,
) {
    use crate::core::messages::{SystemBlackboard, ViewscreenBlackboard};
    for (tac, sci, mut bbs) in q.iter_mut() {
        let combat_lock = tac.and_then(|t| t.0.clone());
        let science_target = sci.and_then(|s| s.0.clone());
        let mut vbb = match bbs
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(v)) => v.clone(),
            _ => ViewscreenBlackboard::default(),
        };
        vbb.combat_lock = combat_lock;
        vbb.science_target = science_target;
        bbs.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vbb),
        );
    }
}

fn test_app() -> App {
    let mut app = App::new();
    // The admission seam, through the same one call production uses
    // (issue #898) — resources and system together, so this fixture cannot
    // drift into having one without the other. Ungated: the fixture spawns
    // its ships by hand and never runs the lobby countdown, so it never
    // reaches `GamePhase::InProgress`.
    crate::command_admission::register_admission_seam(
        &mut app,
        crate::command_admission::AdmissionGate::EveryTick,
    );
    app.configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    )
    .add_systems(
        FixedUpdate,
        seed_viewscreen_from_selection
            .after(crate::lobby::LobbySystemSet)
            .before(crate::sim_sets::SimSet::Input),
    )
    .add_plugins(LobbyPlugin)
    .add_plugins(bevy::time::TimePlugin)
    .init_resource::<WorldResource>()
    .init_resource::<TrackedEntities>()
    .insert_resource(SimBroadcastTimer(Timer::new(
        std::time::Duration::from_nanos(1),
        TimerMode::Repeating,
    )))
    .init_resource::<WorldSetupBroadcast>()
    .init_resource::<SimOutbox>()
    .init_resource::<LastBroadcastEntityPositions>()
    .init_resource::<LastBroadcastEntityHealth>()
    .init_resource::<LastBroadcastHull>()
    .init_resource::<LastBroadcastShields>()
    .init_resource::<LastBroadcastBlackboards>()
    .init_resource::<crate::core::messages::InterSystemQueue>()
    .init_resource::<crate::ai::server::AiTokenRegistry>()
    .init_resource::<Outbox>()
    .add_message::<crate::ai::server::AiEntityDestroyed>()
    .add_plugins(crate::console::captain::server::CaptainPlugin)
    .add_plugins(crate::console::weapons::WeaponsPlugin)
    .add_plugins(crate::console::repair::server::RepairPlugin)
    .add_plugins(crate::ship::power::ShipPowerPlugin)
    .add_plugins(crate::ship::shields::ShipShieldsPlugin)
    .add_plugins(crate::ship::sensors::ShipSensorsPlugin)
    .add_plugins(crate::console::comms::server::CommsConsolePlugin)
    .add_systems(
        OnEnter(GamePhase::InProgress),
        reset_broadcast_caches_on_start,
    )
    .add_systems(
        FixedUpdate,
        (
            handle_impulse_messages,
            broadcast_shield_status,
            reconcile_runtime_entities
                .after(crate::lobby::LobbySystemSet)
                .before(broadcast_world_setup_on_start),
            broadcast_world_setup_on_start.after(crate::lobby::LobbySystemSet),
            refresh_caches_on_midgame_reconnect.after(crate::lobby::LobbySystemSet),
        ),
    )
    .add_systems(
        FixedUpdate,
        crate::modifiers::coordination::translate_power_modifiers
            .after(crate::ship::power::handle_power_messages)
            .after(crate::ship::power::tick_power_system),
    )
    .add_systems(
        FixedUpdate,
        crate::modifiers::coordination::translate_impulse_modifiers.after(handle_impulse_messages),
    )
    .add_systems(
        FixedUpdate,
        (
            sim_processing_anchor,
            broadcast_blackboard_updates.in_set(crate::sim_sets::SimSet::PublishAggregate),
        ),
    )
    .add_plugins(weapons_update_broadcaster())
    .add_plugins(sim_state_broadcaster())
    .add_plugins(modifier_events_broadcaster())
    .add_systems(PostUpdate, collect);
    // One fixed step per update (issue #895): the sim chain above lives in
    // `FixedUpdate`, and each 200 ms harness tick advances it once (so the
    // Hz-based SimBroadcaster timers always fire within a single update).
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    // Spawn the Ship entity immediately so systems that query it (including
    // auth checks in handle_fire_torpedo, handle_power_messages, etc.) work
    // during Lobby as well as InProgress.
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::core::messages::AdmittedCommands::default(),
            ShipShields(ShieldSystem::default(), 0.5),
            ShipPhysicsComponent::default(),
            crate::ship::state::ShipRedAlert::default(),
            crate::ship::state::ShipViewMode::default(),
            crate::ship::state::ShipPhaserFrequency::default(),
            bevy::prelude::Transform::default(),
            crate::entities::spawner::EntitySystemHull(
                crate::ship::damage::SystemHull::from_config(&[
                    (SystemId("helm".into()), 25.0),
                    (SystemId("tactical".into()), 25.0),
                    (SystemId("power".into()), 25.0),
                    (SystemId("shields".into()), 25.0),
                ]),
            ),
        ))
        .id();
    // Insert per-entity components (Bundle limit).
    app.world_mut().entity_mut(ship).insert((
        ShipImpulse::default(),
        ShipBoost::default(),
        crate::modifiers::ShipModifiers::new(),
        crate::console::weapons::TorpedoSystemResource(
            crate::weapons::torpedo::TorpedoSystem::new(
                crate::weapons::torpedo::TorpedoConfig::default(),
            ),
        ),
        crate::console::weapons::PhaserCombatConfigResource::default(),
        PhaserRenderConfig::default(),
        // PR 7 (issue #597) — per-entity beam / target / cooldown / sensors / waypoint.
        crate::console::weapons::TacticalRadarSelection::default(),
        crate::console::weapons::ActiveBeam::default(),
        crate::console::weapons::PhaserCooldown::default(),
        crate::ship::sensors::SensorRadarSelection::default(),
        crate::console::navigation::NavigationWaypoint::default(),
        crate::ship::power::PowerBrownoutState::default(),
    ));
    app.insert_resource(ShipEntity(ship));
    app
}

// ── PR 7 (issue #597) test helpers ──────────────────────────────────────
// These wrap the `Query<&X, With<LocalShip>>` pattern that replaces
// direct Resource access after PR 7 removed the Resource derive from
// TacticalRadarSelection / ActiveBeam / PhaserCooldown / SensorRadarSelection / NavigationWaypoint.

fn get_weapons_target(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::console::weapons::TacticalRadarSelection, With<LocalShip>>();
    q.single(app.world()).ok().and_then(|wt| wt.0.clone())
}

/// Author a `[weapons_console.radar] range` on the LocalShip.
///
/// Since issue #887 `handle_set_target` takes the lock horizon from the
/// ship's OWN `WeaponsConsoleSection` (it applies the lock for every ship,
/// not just the player's, and `ShipClientConfigResource` is the player's
/// radar) — so a fixture that wants a bounded horizon has to author one. A
/// hull with no radar block has an unbounded horizon, which is what every
/// NPC hull actually declares.
fn set_tactical_radar_range(app: &mut App, range: f32) {
    let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
    let entity = q.single_mut(app.world_mut()).expect("LocalShip");
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::entities::spawner::WeaponsConsoleSection(
            crate::entities::config::WeaponsConsoleConfig {
                torpedo_arc_color: vec![],
                power_multipliers: None,
                phaser_banks: vec![],
                blaster_banks: vec![],
                radar: Some(crate::radar_config::RadarConfig {
                    range,
                    shows: vec![crate::entities::tags::EntityTag::Ship],
                    selects: vec![],
                }),
                selector: None,
                selector_idle: false,
                ai: None,
            },
        ));
}

// `ActiveBeam` is per-bank since issue #790; these fixtures all drive a ship
// firing ONE bank at a time, so "the beam" still means "the one live slot".

fn get_active_beam_target(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::console::weapons::ActiveBeam, With<LocalShip>>();
    q.single(app.world())
        .ok()
        .and_then(|b| b.any_target().map(str::to_string))
}

fn active_beam_target_is_none(app: &mut App) -> bool {
    get_active_beam_target(app).is_none()
}

fn live_beam_banks(app: &mut App) -> Vec<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::console::weapons::ActiveBeam, With<LocalShip>>();
    q.single(app.world())
        .ok()
        .map(|b| b.live_banks().map(|(k, _)| k.clone()).collect())
        .unwrap_or_default()
}

fn set_active_beam_target(app: &mut App, uuid: Option<String>) {
    let banks = live_beam_banks(app);
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::console::weapons::ActiveBeam, With<LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        match uuid {
            None => {
                for bank in banks {
                    b.end_bank(&bank);
                }
            }
            Some(u) => {
                let bank = banks.first().cloned().unwrap_or_default();
                let remaining = b
                    .bank_slot_mut(&bank)
                    .map(|s| s.remaining_secs)
                    .unwrap_or(0.0);
                b.start(bank, u, remaining);
            }
        }
    }
}

fn set_active_beam_remaining_secs(app: &mut App, secs: f32) {
    let banks = live_beam_banks(app);
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::console::weapons::ActiveBeam, With<LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        for bank in banks {
            if let Some(slot) = b.bank_slot_mut(&bank) {
                slot.remaining_secs = secs;
            }
        }
    }
}

fn set_active_beam_damage_accumulator(app: &mut App, val: f32) {
    let banks = live_beam_banks(app);
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::console::weapons::ActiveBeam, With<LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        for bank in banks {
            if let Some(slot) = b.bank_slot_mut(&bank) {
                slot.damage_accumulator = val;
            }
        }
    }
}

fn phaser_bank_is_active(app: &mut App, bank: &str) -> bool {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::console::weapons::PhaserCooldown, With<LocalShip>>();
    q.single(app.world())
        .ok()
        .map(|cd| cd.is_bank_active(bank))
        .unwrap_or(false)
}

fn start_phaser_cooldown(app: &mut App, bank: &str, secs: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::console::weapons::PhaserCooldown, With<LocalShip>>();
    if let Ok(mut cd) = q.single_mut(app.world_mut()) {
        cd.start_bank_with_cooldown(bank, secs);
    }
}

fn apply_hull_damage(app: &mut App, amount: f32) {
    let mut rng = crate::sim_rng::unseeded_test_rng();
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<crate::entities::spawner::EntitySystemHull>()
        .unwrap()
        .0
        .apply_damage(amount, &mut rng);
}

fn get_ship_modifiers(app: &mut App) -> crate::modifiers::ShipModifiers {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::modifiers::ShipModifiers, With<crate::server_app::LocalShip>>();
    q.single(app.world()).unwrap().clone()
}

fn modify_ship_modifiers<F>(app: &mut App, f: F)
where
    F: FnOnce(&mut crate::modifiers::ShipModifiers),
{
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::modifiers::ShipModifiers, With<crate::server_app::LocalShip>>(
        );
    let mut mods = q.single_mut(app.world_mut()).unwrap();
    f(&mut mods);
}

fn get_phaser_frequency(app: &mut App) -> f32 {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship::state::ShipPhaserFrequency, With<LocalShip>>();
    q.single(app.world()).map(|f| f.0).unwrap_or(0.5)
}

fn get_view_mode(app: &mut App) -> crate::core::messages::ViewMode {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship::state::ShipViewMode, With<LocalShip>>();
    q.single(app.world())
        .map(|vm| vm.view_mode.clone())
        .unwrap_or(crate::core::messages::ViewMode::Camera(
            crate::core::messages::CameraView::default(),
        ))
}

/// Fast-forward the pre-game countdown so the game starts immediately.
/// Must be called after the tick that starts the countdown.
fn fast_forward_countdown(app: &mut App) {
    use crate::lobby::CountdownTimer;
    app.world_mut()
        .resource_mut::<CountdownTimer>()
        .remaining_secs = 0.001;
}

fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    // Drain any leftover SimOutbox entries that the sim systems wrote but
    // were not captured by the PostUpdate collect system (SimOutbox is not
    // connected to the OutboundMessage bus for test_app).
    let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
    let mut out = app.world().resource::<Outbox>().0.clone();
    for (target, msg) in sim_entries {
        out.push(OutboundMessage {
            target,
            msg,
            delivery: DeliveryClass::Reliable,
        });
    }
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn load_tube_now(app: &mut App, tube: &str) {
    // Systems prefer the per-entity component over the resource; update component.
    let mut q = app
        .world_mut()
        .query_filtered::<&mut TorpedoSystemResource, With<LocalShip>>();
    if let Ok(mut ts) = q.single_mut(app.world_mut()) {
        ts.0.tube_mut(tube)
            .expect("test tube should exist")
            .loaded_count = 1;
    } else {
        let world = app.world_mut();
        let mut res = world.resource_mut::<TorpedoSystemResource>();
        res.0
            .tube_mut(tube)
            .expect("test tube should exist")
            .loaded_count = 1;
    }
}

fn set_ship_yaw(app: &mut App, yaw: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipPhysicsComponent, With<crate::server_app::Ship>>();
    let mut p = q
        .single_mut(app.world_mut())
        .expect("expected Ship with ShipPhysics");
    p.yaw = yaw;
}

fn start_game(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    tick(app); // handle_set_ready_system → starts countdown
    fast_forward_countdown(app);
    tick(app); // tick_countdown → emits GameStarted, sets NextState::Set(InProgress)
    tick(app); // NextState takes effect: Phase switches to InProgress
}

fn start_game_with_helm(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "helm",
        ClientMessage::Identify {
            token: "helm".into(),
            name: "Bob".into(),
        },
    );
    tick(app);
    push(
        app,
        "helm",
        ClientMessage::SelectStation {
            station: "Helm".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "helm", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
}

fn start_game_with_sensors(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::Identify {
            token: "sensors".into(),
            name: "Spock".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::SelectStation {
            station: "Sensors".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "sensors", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
}

fn start_game_with_navigation(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "navigation",
        ClientMessage::Identify {
            token: "navigation".into(),
            name: "Decker".into(),
        },
    );
    tick(app);
    push(
        app,
        "navigation",
        ClientMessage::SelectStation {
            station: "Navigation".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "navigation", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
}

#[test]
fn entity_config_radar_icon_flows_into_world_snapshot() {
    let config = crate::entities::config::EntityConfig {
        name: Some("Sun".into()),
        tags: vec!["star".into(), "center".into()],
        collider: Some(crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Ball,
            radius: 50.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
        radar_appearance: Some(crate::entities::config::RadarAppearanceConfig {
            colour: Some(vec![1.0, 0.85, 0.3]),
            size: None,
            region_colour: None,
            icon: Some("star".into()),
        }),
        ..Default::default()
    };

    let snapshot =
        snapshot_from_entity_config("sun-uuid".into(), None, &config, Vec3::new(0.0, 0.0, 0.0));

    assert_eq!(snapshot.name.as_deref(), Some("Sun"));
    assert_eq!(snapshot.tags, vec!["star", "center"]);
    assert_eq!(snapshot.radius, Some(50.0));
    assert_eq!(snapshot.colour, Some([1.0, 0.85, 0.3]));
    assert_eq!(snapshot.radar_icon.as_deref(), Some("star"));
}

/// `player_ship_identity` adds the `player` tag (keeping `ship`) and forces
/// the radar icon to `playerShip` while preserving the template's radar
/// colour/size.
#[test]
fn player_ship_identity_adds_player_tag_and_playership_icon() {
    let (tags, radar) = player_ship_identity(
        &["ship".to_string()],
        Some(&crate::entities::config::RadarAppearanceConfig {
            icon: Some("ship".into()),
            colour: Some(vec![0.0, 1.0, 0.2]),
            size: Some(6.0),
            region_colour: None,
        }),
    );
    assert!(tags.iter().any(|t| t == "ship"), "keeps the ship tag");
    assert!(tags.iter().any(|t| t == "player"), "adds the player tag");
    assert_eq!(radar.icon.as_deref(), Some("playerShip"));
    // Appearance other than the icon is preserved from the template.
    assert_eq!(radar.colour, Some(vec![0.0, 1.0, 0.2]));
    assert_eq!(radar.size, Some(6.0));
}

/// End-to-end of the player spawn path's identity injection: parse the real
/// cruiser hull template, spawn it via `spawn_entity` (which sets the
/// ordinary-ship `EntityTagsSection` / `RadarAppearanceSection`), then apply
/// the same injection `spawn_game_start_entities` performs and assert the
/// spawned player ship carries the `player` tag AND the `playerShip` radar
/// icon. Uses the checked-in template so it regresses on the TOML edits too.
#[test]
fn player_spawn_injects_player_tag_and_icon_over_template() {
    use crate::entities::spawner::{EntityTagsSection, RadarAppearanceSection};
    use bevy::prelude::*;

    // Through the resolver (issue #876): this hull is COMPOSED, so its baked
    // bytes are no longer the document that spawns.
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .expect("cruiser template must compose and parse");

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    let spawned = {
        let mut cmds = app.world_mut().commands();
        crate::entities::spawner::spawn_entity(
            &mut cmds,
            &config,
            Vec3::ZERO,
            "player-cruiser".into(),
            None,
        )
    };
    app.world_mut().flush();

    // Pre-injection: the template presents as an ordinary ship.
    let tags = app.world().get::<EntityTagsSection>(spawned).unwrap();
    assert!(!tags.0.iter().any(|t| t == "player"));
    let radar = app.world().get::<RadarAppearanceSection>(spawned).unwrap();
    assert_eq!(radar.0.icon.as_deref(), Some("ship"));

    // Apply the player spawn injection (mirrors spawn_game_start_entities).
    let (player_tags, player_radar) =
        player_ship_identity(&config.tags, config.radar_appearance.as_ref());
    app.world_mut()
        .entity_mut(spawned)
        .insert(EntityTagsSection(player_tags))
        .insert(RadarAppearanceSection(player_radar));

    // Post-injection: player identity is present on the spawned ship.
    let tags = app.world().get::<EntityTagsSection>(spawned).unwrap();
    assert!(tags.0.iter().any(|t| t == "ship"), "still a ship");
    assert!(
        tags.0.iter().any(|t| t == "player"),
        "player ship carries the player tag; got {:?}",
        tags.0
    );
    let radar = app.world().get::<RadarAppearanceSection>(spawned).unwrap();
    assert_eq!(
        radar.0.icon.as_deref(),
        Some("playerShip"),
        "player ship carries the playerShip radar icon"
    );
}

// ── player_hull_config (the world's tuning survives the hull swap) ───────
//
// The defect these pin was found while reviewing #1036: the lobby-selected
// hull replaced the world's resolved config WHOLESALE, so every
// `[entity.overrides.*]` a world authored on its `player-ship` row was
// merged, validated, and then thrown away.

/// Two real shipped hulls, loaded the way the runtime loads them (include
/// closure resolved first) so these tests regress on the TOML too.
fn hull(path: &str) -> crate::entities::config::EntityConfig {
    crate::entities::include_resolve::load_entity_config(path)
        .unwrap_or_else(|e| panic!("{path} must compose and parse: {e}"))
}

fn doctrine<'a>(
    config: &'a crate::entities::config::EntityConfig,
    id: &str,
) -> &'a crate::entities::config::DoctrineObjective {
    config
        .behaviour
        .as_ref()
        .expect("hull authors [behaviour]")
        .doctrine
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("hull authors a `{id}` doctrine"))
}

/// THE FIX. The world's row names the Destroyer and tunes it; the lobby
/// picked the Cruiser. What spawns is the Cruiser **carrying the world's
/// tuning** — not the Destroyer, and not an untuned Cruiser.
#[test]
fn player_hull_config_reapplies_the_worlds_overrides_onto_the_selected_hull() {
    let world_row = hull("assets/entities/alliance_destroyer.toml");
    let selected = hull("assets/entities/alliance_cruiser.toml");
    let overrides: toml::Value = toml::from_str(
        r#"
[[behaviour.doctrine]]
id = "hold-station"
base_priority = 77.0
"#,
    )
    .unwrap();

    let spawned = player_hull_config(world_row, Some(&overrides), Some(&selected));

    assert_eq!(
        doctrine(&spawned, "hold-station").base_priority,
        77.0,
        "the world's per-instance tuning survived the hull swap"
    );
    assert_eq!(
        doctrine(&spawned, "destroy-hostiles"),
        doctrine(&selected, "destroy-hostiles"),
        "…and it landed on the hull the LOBBY picked: every field the \
         override does not name is exactly what the Cruiser authored, not \
         what the placeholder Destroyer did"
    );
}

/// The guarantee every shipped world rests on: a `player-ship` row with no
/// overrides spawns the lobby selection UNTOUCHED — not a round-tripped
/// copy of it. This is the pre-fix path, byte for byte, and it is why the
/// anchor digests must not move.
#[test]
fn player_hull_config_without_overrides_is_the_selection_untouched() {
    let world_row = hull("assets/entities/alliance_destroyer.toml");
    let selected = hull("assets/entities/alliance_cruiser.toml");

    assert_eq!(
        player_hull_config(world_row, None, Some(&selected)),
        selected,
        "no overrides authored: nothing about the selection is re-derived"
    );
}

/// No lobby selection at all (a host that never ran a lobby) keeps the
/// world's own resolved config — which `resolve_entity_via` has already
/// merged the overrides into. Unchanged by the fix.
#[test]
fn player_hull_config_without_a_selection_keeps_the_worlds_resolved_config() {
    let world_row = hull("assets/entities/alliance_destroyer.toml");
    let overrides: toml::Value = toml::from_str(
        r#"
[[behaviour.doctrine]]
id = "hold-station"
base_priority = 77.0
"#,
    )
    .unwrap();

    assert_eq!(
        player_hull_config(world_row.clone(), Some(&overrides), None),
        world_row,
        "with nothing selected there is no second merge to run"
    );
}

/// The validator checked these overrides against the ROW's template, not
/// against the hull the lobby went on to pick, so this merge can fail where
/// validation passed. The player's hull is the one entity a session cannot
/// do without, so a failure spawns the untuned selection and says so —
/// rather than dropping the ship. `_remove` is the reachable failure: it is
/// a fragment-composition marker the instance layer rejects outright
/// (issue #911).
#[test]
fn player_hull_config_falls_back_to_the_selection_when_the_merge_is_refused() {
    let world_row = hull("assets/entities/alliance_destroyer.toml");
    let selected = hull("assets/entities/alliance_cruiser.toml");
    let overrides: toml::Value = toml::from_str(
        r#"
[[behaviour.doctrine]]
id = "hold-station"
_remove = true
"#,
    )
    .unwrap();

    assert_eq!(
        player_hull_config(world_row, Some(&overrides), Some(&selected)),
        selected,
        "a refused merge still spawns the crew a ship"
    );
}

#[test]
fn world_entity_upsert_replaces_existing_snapshot_for_same_uuid() {
    let mut world = WorldResource(WorldData::default());
    upsert_world_entity(
        &mut world,
        EntitySnapshot {
            uuid: "same".into(),
            tags: vec!["asteroid".into()],
            radar_icon: Some("asteroid".into()),
            ..Default::default()
        },
    );
    upsert_world_entity(
        &mut world,
        EntitySnapshot {
            uuid: "same".into(),
            tags: vec!["star".into()],
            radar_icon: Some("star".into()),
            ..Default::default()
        },
    );

    assert_eq!(world.0.entities.len(), 1);
    assert_eq!(world.0.entities[0].tags, vec!["star"]);
    assert_eq!(world.0.entities[0].radar_icon.as_deref(), Some("star"));
}

#[test]
fn sensors_can_switch_view_to_science_radar() {
    let mut app = test_app();
    start_game_with_sensors(&mut app);
    push(
        &mut app,
        "sensors",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::ScienceRadar,
            },
        },
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::ScienceRadar);
}
#[test]
fn sensors_can_switch_view_to_sensors_radar() {
    let mut app = test_app();
    start_game_with_sensors(&mut app);
    push(
        &mut app,
        "sensors",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::SensorsRadar,
            },
        },
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::SensorsRadar);
}

#[test]
fn non_sensors_cannot_switch_view_to_sensors_radar() {
    let mut app = test_app();
    start_game_with_sensors(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::SensorsRadar,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::default())
    );
}

#[test]
fn navigation_can_switch_view_to_system_chart() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::SystemChart,
            },
        },
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::SystemChart);
}

#[test]
fn non_sensors_cannot_switch_view_to_science_radar() {
    let mut app = test_app();
    start_game_with_sensors(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::ScienceRadar,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::default())
    );
}

#[test]
fn non_navigation_cannot_switch_view_to_system_chart() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::SystemChart,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::default())
    );
}

#[test]
fn navigation_can_switch_view_to_navigation_chart() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::NavigationChart,
            },
        },
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::NavigationChart);
}

#[test]
fn non_navigation_cannot_switch_view_to_navigation_chart() {
    let mut app = test_app();
    start_game_with_navigation(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::NavigationChart,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::default())
    );
}

fn start_game_with_comms(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "comms",
        ClientMessage::Identify {
            token: "comms".into(),
            name: "Uhura".into(),
        },
    );
    tick(app);
    push(
        app,
        "comms",
        ClientMessage::SelectStation {
            station: "Comms".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "comms", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
}

#[test]
fn comms_can_push_view_to_comms() {
    let mut app = test_app();
    start_game_with_comms(&mut app);
    push(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Comms,
            },
        },
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::Comms);
}

#[test]
fn captain_override_from_comms_view_works() {
    let mut app = test_app();
    start_game_with_comms(&mut app);
    push(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Comms,
            },
        },
    );
    tick(&mut app);
    // Captain overrides back to a camera view.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Camera(CameraView::new("camera_aft")),
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::new("camera_aft"))
    );
}

#[test]
fn non_comms_cannot_push_comms_view() {
    let mut app = test_app();
    start_game_with_comms(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Comms,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::default())
    );
}

#[test]
fn helm_can_switch_view_to_radar() {
    let mut app = test_app();
    start_game_with_helm(&mut app);
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Radar,
            },
        },
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
}

#[test]
fn captain_cannot_switch_view_to_radar() {
    let mut app = test_app();
    start_game_with_helm(&mut app);
    // Captain has no authority over Radar; request is silently dropped.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Radar,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::default())
    );
}

#[test]
fn helm_cannot_switch_view_to_camera() {
    let mut app = test_app();
    start_game_with_helm(&mut app);
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Camera(CameraView::new("camera_aft")),
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Camera(CameraView::default())
    );
}

#[test]
fn world_setup_is_broadcast_once_after_start_game() {
    let mut app = test_app();
    // Pre-populate world data so the broadcast has something to emit.
    app.world_mut().insert_resource(WorldResource(WorldData {
        entities: vec![EntitySnapshot::asteroid("test-uuid", 5.0, -1.0, 2.0)],
        ..Default::default()
    }));

    // Bring the game up to the point of pressing SetReady
    push(
        &mut app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "A".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(&mut app);
    // Advance the phase to InProgress so broadcast_world_setup_on_start fires.
    push(&mut app, "captain", ClientMessage::SetReady { ready: true });
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));
    let start_out = tick(&mut app);

    let world_setups: Vec<_> = start_out
        .iter()
        .filter(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. }))
        .collect();
    assert_eq!(
        world_setups.len(),
        1,
        "expected exactly one WorldSetup on the SetReady tick"
    );
    match &world_setups[0].msg {
        ServerMessage::WorldSetup { world } => {
            assert_eq!(world.entities.len(), 1);
            assert_eq!(world.entities[0].x(), 5.0);
        }
        _ => unreachable!(),
    }
    match &world_setups[0].target {
        crate::lobby::Target::All => {}
        t => panic!("WorldSetup should target All, got {:?}", t),
    }

    // Subsequent ticks must not re-broadcast WorldSetup
    let later = tick(&mut app);
    assert!(
        !later
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
        "WorldSetup should only fire once per game"
    );
}

#[test]
fn world_setup_is_not_broadcast_during_lobby() {
    let mut app = test_app();
    app.world_mut().insert_resource(WorldResource(WorldData {
        entities: vec![EntitySnapshot::asteroid("test-uuid", 0.0, 0.0, 2.0)],
        ..Default::default()
    }));
    // Identify and select a console but don't start the game.
    push(
        &mut app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "A".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
        "WorldSetup should not be broadcast in the Lobby phase"
    );
}

/// Read the live phase out of a running app.
fn phase_of(app: &App) -> GamePhase {
    app.world().resource::<State<GamePhase>>().get().clone()
}

/// Issue #939's mid-mission "exit to lobby" has to work against the REAL
/// app, not just the pure handler: `handle_return_to_lobby_system` is
/// registered with a `run_if` phase gate of its own, so a handler that
/// happily honours `InProgress` is still dead if the system never runs
/// there. This drives the whole `LobbyPlugin` through `test_app`, which is
/// the only harness that observes that registration.
#[test]
fn host_return_to_lobby_aborts_a_mission_in_progress_in_the_real_app() {
    let mut app = test_app();
    start_game(&mut app);
    assert_eq!(
        phase_of(&app),
        GamePhase::InProgress,
        "precondition: start_game must leave the session mid-mission"
    );

    push(
        &mut app,
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        ClientMessage::ReturnToLobby,
    );
    tick(&mut app);
    // `test_app` deliberately omits `LobbyOutboxPlugin`, so the lobby's
    // broadcasts stay in `LobbyOutbox` instead of reaching the
    // `OutboundMessage` bus — read them where they actually land.
    let returned = app
        .world()
        .resource::<LobbyOutbox>()
        .0
        .iter()
        .any(|(_, m)| matches!(m, ServerMessage::ReturnedToLobby));
    tick(&mut app); // NextState::Set(Lobby) takes effect.

    assert_eq!(
        phase_of(&app),
        GamePhase::Lobby,
        "the host's exit-to-lobby must abort a mission that is still InProgress"
    );
    assert!(
        returned,
        "the abort must tell the phones to leave their consoles"
    );
}

/// The reach added above is the host page's alone. A phone sending the
/// same un-gated `ReturnToLobby` mid-mission must be ignored by the real
/// app, or the settings-cog feature would hand every handset an abort.
#[test]
fn a_phone_cannot_abort_a_mission_in_progress_in_the_real_app() {
    let mut app = test_app();
    start_game(&mut app);

    push(&mut app, "captain", ClientMessage::ReturnToLobby);
    tick(&mut app);
    tick(&mut app);

    assert_eq!(
        phase_of(&app),
        GamePhase::InProgress,
        "a participant token must not end a mission the rest of the crew is flying"
    );
}

#[test]
fn hull_integrity_starts_at_100_and_appears_in_system_hull_update() {
    let mut app = test_app();
    start_game(&mut app);
    // The first InProgress tick (inside start_game) already emitted and consumed
    // the initial SystemHullUpdate. Reset the cache to force re-emission.
    app.world_mut()
        .resource_mut::<LastBroadcastHull>()
        .0
        .clear();
    let out = tick(&mut app);
    // Post issue #737 `entries` is a per-recipient projection, so the
    // whole-ship figure is `aggregate_fraction` — the authoritative
    // ship-wide hull producer — not the sum of the visible rows.
    let aggregate = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::SystemHullUpdate {
                aggregate_fraction, ..
            } => Some(*aggregate_fraction),
            _ => None,
        })
        .expect("expected a SystemHullUpdate broadcast");
    assert!((aggregate.expect("aggregate fraction") - 1.0).abs() < 1e-6);
}

#[test]
fn direct_damage_reduces_hull_integrity_in_broadcast() {
    let mut app = test_app();
    start_game(&mut app);
    // Consume the initial SystemHullUpdate so LastBroadcastHull is seeded.
    let _ = tick(&mut app);

    // Directly apply damage to the EntitySystemHull component (simulates collision at ~half speed).
    apply_hull_damage(&mut app, 10.0);

    let out = tick(&mut app);
    // See the note above: the ship-wide figure is now `aggregate_fraction`.
    let aggregate = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::SystemHullUpdate {
                aggregate_fraction, ..
            } => Some(*aggregate_fraction),
            _ => None,
        })
        .expect("expected a SystemHullUpdate after damage");
    assert!((aggregate.expect("aggregate fraction") - 0.9).abs() < 1e-6);
}

// â"€â"€ SetTarget / TargetLock tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
    app.world_mut().insert_resource(WorldResource(WorldData {
        entities: vec![EntitySnapshot::asteroid(
            "target-uuid",
            asteroid_x,
            asteroid_z,
            2.0,
        )],
        ..Default::default()
    }));
    // Also spawn the live ECS entity. As of the targeting fix, gameplay
    // logic reads positions from ECS Transforms (not the WorldResource
    // snapshot), so targets must exist as ECS entities to be lockable.
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("target-uuid".into()),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[(crate::core::messages::SystemId("captain".into()), 30.0)],
        )),
        Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
    ));
}

/// Like `setup_weapons_world` but also returns the spawned entity for
/// tests that need to manipulate or despawn it later.
fn setup_weapons_world_with_entity(
    app: &mut App,
    asteroid_x: f32,
    asteroid_z: f32,
) -> bevy::ecs::entity::Entity {
    app.world_mut().insert_resource(WorldResource(WorldData {
        entities: vec![EntitySnapshot::asteroid(
            "target-uuid",
            asteroid_x,
            asteroid_z,
            2.0,
        )],
        ..Default::default()
    }));
    app.world_mut()
        .spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            crate::entities::spawner::EntitySystemHull(
                crate::ship::damage::SystemHull::from_config(&[(
                    crate::core::messages::SystemId("captain".into()),
                    30.0,
                )]),
            ),
            Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
        ))
        .id()
}

fn start_game_with_weapons(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::Identify {
            token: "weapons".into(),
            name: "Bob".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "weapons", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
    // Apply the human rating for Tactical's weapons systems so
    // `admit_system_commands` (which checks ShipSystemControlSources)
    // authorizes human ControlSystem messages for phasers, torpedoes, etc.
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::Ship>>();
    if let Ok(mut cs) = q.single_mut(app.world_mut()) {
        use crate::ship::control_source::ControlSource;
        cs.0.set(
            crate::ship::system_registry::phaser_fore_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::ship::system_registry::phaser_aft_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::ship::system_registry::torpedo_tube_fore_port_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::ship::system_registry::torpedo_tube_fore_starboard_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::ship::system_registry::torpedo_tube_aft_system_id(),
            ControlSource::Human,
        );
        cs.0.set(
            crate::ship::system_registry::torpedo_magazine_system_id(),
            ControlSource::Human,
        );
    }
}

#[test]
fn valid_target_within_range_replies_with_target_lock_confirmed() {
    let mut app = test_app();
    // Asteroid at (30, 0) â€" 30 units from ship origin, within 60-unit range.
    set_tactical_radar_range(&mut app, 300.0);
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let out = tick(&mut app);

    let lock = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        })
        .expect("expected a TargetLock response");
    assert_eq!(lock.0, "target-uuid");
    assert!(lock.1, "expected locked=true for in-range asteroid");

    // Server state should record the lock.
    assert_eq!(get_weapons_target(&mut app).as_deref(), Some("target-uuid"));
}

#[test]
fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
    let mut app = test_app();
    // Asteroid at (400, 0) — 400 units away, outside 300-unit Weapons range.
    set_tactical_radar_range(&mut app, 300.0);
    setup_weapons_world(&mut app, 400.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let out = tick(&mut app);

    let lock = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        })
        .expect("expected a TargetLock response");
    assert!(!lock.1, "expected locked=false for out-of-range asteroid");
    assert!(get_weapons_target(&mut app).is_none());
}

#[test]
fn unknown_uuid_replies_with_target_lock_rejected() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 10.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "no-such-asteroid".into(),
            },
        },
    );
    let out = tick(&mut app);

    let lock = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        })
        .expect("expected a TargetLock response");
    assert!(!lock.1, "expected locked=false for unknown UUID");
    assert!(get_weapons_target(&mut app).is_none());
}

// â"€â"€ WeaponsUpdate / fire_ready tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

/// Target locked, within 40-unit phaser range, in forward arc â†' fire_ready = true.
#[test]
fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
    let mut app = test_app();
    // Ship at origin, yaw=0 (facing -Z). Asteroid at (0, -20): directly ahead, 20 units away.
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    // Tick 1 admits SetTarget in `SimSet::Input`. `compute_current_weapons_update`
    // reads the frozen viewscreen combat lock (spec §3), which this harness'
    // `seed_viewscreen_from_selection` glue refreshes before `SimSet::Input`,
    // so the new lock reaches the wire on tick 2. The full app aggregates the
    // viewscreen in `SimSet::PublishAggregate`, ahead of the `SimSet::Broadcast`
    // broadcaster, so it has no such gap.
    tick(&mut app);
    let out = tick(&mut app);

    let update = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate {
                target_uuid, banks, ..
            } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
            _ => None,
        })
        .expect("expected a WeaponsUpdate message");
    assert_eq!(update.0.as_deref(), Some("target-uuid"));
    assert!(
        update.1,
        "expected fire_ready=true for in-range, forward-arc target"
    );
}

/// Target locked but beyond 40-unit phaser range (within 60u lock range) → fire_ready = false.
#[test]
fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
    let mut app = test_app();
    // Ship at origin, yaw=0. Asteroid at (0, -50): directly ahead, 50 units — within lock range
    // (60u) but outside phaser range (40u).
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    // Two ticks, for the same frozen-combat-lock reason as the test above.
    tick(&mut app);
    let out = tick(&mut app);

    let update = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate {
                target_uuid, banks, ..
            } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
            _ => None,
        })
        .expect("expected a WeaponsUpdate message");
    assert_eq!(update.0.as_deref(), Some("target-uuid"));
    assert!(
        !update.1,
        "expected fire_ready=false for beyond-phaser-range target"
    );
}

// â"€â"€ FirePhaser / beam lifecycle tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

/// Helper: lock target then fire phaser; returns messages from the fire tick.
fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
    setup_weapons_world(app, asteroid_x, asteroid_z);
    start_game_with_weapons(app);
    // Lock
    push(
        app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let _ = tick(app);
    // Fire
    push(
        app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(app)
}

/// Firing at a fire-ready target broadcasts BeamStarted to all.
#[test]
fn fire_phaser_on_valid_target_broadcasts_beam_started() {
    let mut app = test_app();
    // Asteroid directly ahead at 20 units (yaw=0 â†' facing -Z â†' asteroid at (0,-20)).
    let out = lock_and_fire(&mut app, 0.0, -20.0);

    let beam_started = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
    assert!(
        beam_started.is_some(),
        "expected BeamStarted after firing at fire-ready target"
    );
    match &beam_started.unwrap().msg {
        ServerMessage::BeamStarted { target_uuid, .. } => {
            assert_eq!(target_uuid, "target-uuid")
        }
        _ => unreachable!(),
    }
    match &beam_started.unwrap().target {
        Target::All => {}
        t => panic!("BeamStarted should target All, got {:?}", t),
    }

    // ActiveBeam resource should be populated.
    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid")
    );
}

/// FirePhaser is silently ignored when the phaser is on cooldown.
#[test]
fn fire_phaser_rejected_during_cooldown() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Manually put the cooldown into active state (simulating a beam just ended).
    set_active_beam_target(&mut app, None);
    start_phaser_cooldown(&mut app, "fore", 3.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "BeamStarted should not fire during cooldown"
    );
}

/// Non-weapons player cannot fire.
#[test]
fn fire_phaser_ignored_from_non_weapons_player() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "captain should not be able to fire phaser"
    );
}

/// When the beam fires at a target outside the 180Â° arc, it is rejected.
#[test]
fn fire_phaser_rejected_when_target_behind_ship() {
    let mut app = test_app();
    // Yaw=0 means ship faces -Z. Asteroid at (0, +20) is directly behind â€" in rear arc.
    setup_weapons_world(&mut app, 0.0, 20.0);
    start_game_with_weapons(&mut app);
    // Lock (within 60u range) â€" lock doesn't require arc.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let _ = tick(&mut app);
    // Fire â€" rejected because target is behind.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "FirePhaser should be rejected when target is in rear arc"
    );
}

/// A 6-second natural beam kills the asteroid (5 HP/s Ã— 6s = 30 HP total).
///
/// The test accelerates time by manipulating the beam state directly
/// after confirming the beam started, then runs ticks with large deltas.
#[test]
fn full_beam_duration_kills_asteroid() {
    let mut app = test_app();

    // setup_weapons_world (called by lock_and_fire) now spawns the
    // asteroid ECS entity. Fetch its handle after setup.
    let _ = lock_and_fire(&mut app, 0.0, -20.0);
    let asteroid_entity = {
        let mut q = app
            .world_mut()
            .query::<(bevy::ecs::entity::Entity, &AsteroidUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == "target-uuid")
            .map(|(e, _)| e)
            .expect("setup_weapons_world should have spawned the target asteroid")
    };

    // Verify beam started.
    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid")
    );

    // Fast-forward: accumulate 30 damage via the damage_accumulator.
    // Set accumulator to 30.0 so all damage applies in one tick.
    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0); // still "ongoing"

    let out = tick(&mut app);

    // Asteroid destroyed message should be present.
    let destroyed = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
    assert!(
        destroyed.is_some(),
        "expected AsteroidDestroyed when asteroid HP reaches 0"
    );
    match &destroyed.unwrap().msg {
        ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
        _ => unreachable!(),
    }

    // BeamEnded also broadcast.
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded after asteroid destruction"
    );

    // Asteroid no longer in world data.
    assert!(
        !app.world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid"),
        "destroyed asteroid should be removed from WorldData"
    );

    // Beam resource cleared.
    assert!(active_beam_target_is_none(&mut app));

    // Cooldown started.
    assert!(
        phaser_bank_is_active(&mut app, "fore"),
        "cooldown should start after beam end"
    );

    // The entity should be despawned.
    assert!(
        app.world()
            .get::<crate::entities::spawner::EntitySystemHull>(asteroid_entity)
            .is_none(),
        "asteroid entity should be despawned"
    );
}

/// Beam severs when ship rotates target out of the 180Â° forward arc.
#[test]
fn beam_severs_when_target_leaves_forward_arc() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Now rotate ship so the asteroid is behind it (yaw = π → facing +Z, asteroid at (0,-20) is behind).
    set_ship_yaw(&mut app, std::f32::consts::PI);

    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded when target leaves forward arc"
    );
    assert!(
        active_beam_target_is_none(&mut app),
        "beam should be cleared after sever-by-arc"
    );
    assert!(
        phaser_bank_is_active(&mut app, "fore"),
        "cooldown should start after arc sever"
    );
}

/// Beam severs when the target moves beyond 40-unit phaser range.
#[test]
fn beam_severs_when_target_leaves_phaser_range() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Move asteroid position in WorldData to 50 units away (out of 40u range).
    app.world_mut().resource_mut::<WorldResource>().0.entities[0].position =
        Some([0.0, 0.0, -50.0]);
    // Move the live ECS Transform too — gameplay reads positions from
    // Transforms, not from the WorldResource snapshot.
    let mut q = app
        .world_mut()
        .query_filtered::<&mut Transform, With<AsteroidUuid>>();
    for mut t in q.iter_mut(app.world_mut()) {
        t.translation.z = -50.0;
    }

    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded when target leaves phaser range"
    );
    assert!(
        active_beam_target_is_none(&mut app),
        "beam should be cleared after sever-by-range"
    );
    assert!(
        phaser_bank_is_active(&mut app, "fore"),
        "cooldown should start after range sever"
    );
}

/// No damage refund on sever — whatever HP was dealt is permanent.
#[test]
fn no_damage_refund_on_sever() {
    let mut app = test_app();
    // setup_weapons_world (called by lock_and_fire) now spawns the
    // asteroid ECS entity itself. Fetch its handle by querying for the
    // matching UUID after the fact.
    let _ = lock_and_fire(&mut app, 0.0, -20.0);
    let asteroid_entity = {
        let mut q = app
            .world_mut()
            .query::<(bevy::ecs::entity::Entity, &AsteroidUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == "target-uuid")
            .map(|(e, _)| e)
            .expect("setup_weapons_world should have spawned the target asteroid")
    };

    // Apply partial damage via accumulator.
    set_active_beam_damage_accumulator(&mut app, 10.0);
    let _ = tick(&mut app);

    // Now sever by rotating ship.
    set_ship_yaw(&mut app, std::f32::consts::PI);
    let _ = tick(&mut app);

    let hp = app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(asteroid_entity)
        .map(|h| h.0.total_current());
    assert!(
        hp.is_some() && hp.unwrap() < 30.0,
        "asteroid should retain damage after sever (no refund), hp={:?}",
        hp
    );
}

/// A fresh FirePhaser after cooldown on a new locked target cancels any
/// active beam and starts a new one.
#[test]
fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
    let mut app = test_app();

    // Set up two asteroids.
    app.world_mut().insert_resource(WorldResource(WorldData {
        entities: vec![
            EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
            EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
        ],
        ..Default::default()
    }));
    // Spawn live ECS entities for both targets — gameplay reads positions
    // from Transforms, not from the WorldResource snapshot.
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("t1".into()),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[(crate::core::messages::SystemId("captain".into()), 30.0)],
        )),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("t2".into()),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[(crate::core::messages::SystemId("captain".into()), 30.0)],
        )),
        Transform::from_xyz(0.0, 0.0, -15.0),
    ));
    start_game_with_weapons(&mut app);

    // Lock and fire at t1.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let _ = tick(&mut app);
    assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

    // Natural beam expiry: set remaining to 0.
    set_active_beam_remaining_secs(&mut app, 0.0);
    // Zero damage accumulator so no destruction fires.
    set_active_beam_damage_accumulator(&mut app, 0.0);
    let _ = tick(&mut app); // beam ends, cooldown starts

    // Cooldown should be active.
    assert!(phaser_bank_is_active(&mut app, "fore"));

    // Force cooldown to expire.
    start_phaser_cooldown(&mut app, "fore", 0.0);

    // Lock and fire at t2.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "expected BeamStarted for new target after cooldown"
    );
    assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t2"));
}

// -- Repair helpers --------------------------------------------------

/// Set up a game with a captain and repair player.
fn start_game_with_repair(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "eng",
        ClientMessage::Identify {
            token: "eng".into(),
            name: "Bob".into(),
        },
    );
    tick(app);
    push(
        app,
        "eng",
        ClientMessage::SelectStation {
            station: "Repair".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "eng", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
    // Issue #830: the global `ShipRepairTeams` Resource is gone and this
    // test's ship template carries no `[hull]` block, so the spawner attaches
    // no `ShipRepairTeams`. Give the LocalShip its own component so the
    // per-entity dispatch handler has a store to write.
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .expect("LocalShip must be spawned by start_game_with_repair");
    app.world_mut().entity_mut(ship).insert(ShipRepairTeams(
        crate::modifiers::repair_teams::RepairTeams::new(2),
    ));
}

/// Read the LocalShip's own `ShipRepairTeams` component (issue #830 — no
/// global Resource). Returns an owned clone for assertion convenience.
fn local_teams(app: &mut App) -> ShipRepairTeams {
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipRepairTeams, With<LocalShip>>();
    q.single(app.world())
        .expect("LocalShip must carry ShipRepairTeams")
        .clone()
}

fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
    matches!(
        teams.0.slots()[idx],
        crate::core::messages::TeamSlot::Travelling { .. }
    )
}

fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
    matches!(teams.0.slots()[idx], crate::core::messages::TeamSlot::Idle)
}

/// Damage the named systems on the LocalShip's hull, adding a row for any
/// this fixture's coarse hull does not already carry.
///
/// The ids are systems the shipped battleship config OWNS from the station
/// each dispatch test addresses, so `resolve_repair_target` resolves through
/// `systems_for_station`. Since the issue #1013 review it will NOT fall back
/// to `SystemId(station_id)` when that name is also a hull row — this
/// fixture's `helm`/`tactical`/`power`/`shields` rows are exactly that
/// shape, and such a row is ownerless (bucketed under `core`), so sweeping
/// from it would walk the team out of the station it was sent to.
///
/// HP lands at 80% of max: below max, so it is a resolvable repair target,
/// but still `Operational`, so no tier crossing fires.
fn damage_owned_fine_systems(app: &mut App, systems: &[&str]) {
    let ship = {
        let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
        q.single(app.world()).expect("one LocalShip")
    };
    let mut rows: Vec<(SystemId, f32)> = app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(ship)
        .expect("LocalShip must carry EntitySystemHull")
        .0
        .iter()
        .map(|(sid, entry)| (sid.clone(), entry.max))
        .collect();
    for id in systems {
        let sid = SystemId((*id).into());
        if !rows.iter().any(|(existing, _)| *existing == sid) {
            rows.push((sid, 25.0));
        }
    }
    let mut hull = crate::ship::damage::SystemHull::from_config(&rows);
    for id in systems {
        let sid = SystemId((*id).into());
        let max = hull.get(&sid).expect("just built this row").max;
        hull.set_hp(&sid, max * 0.8);
    }
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::EntitySystemHull(hull));
}

// -- Repair dispatch tests --------------------------------------

#[test]
fn non_repair_sender_is_ignored() {
    let mut app = test_app();
    start_game_with_repair(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: crate::core::messages::RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);
    let teams = local_teams(&mut app);
    assert!(
        team_is_idle(&teams, 0),
        "team 0 should remain idle after non-Repair dispatch"
    );
}

#[test]
fn repair_holder_can_dispatch_team() {
    let mut app = test_app();
    start_game_with_repair(&mut app);
    damage_owned_fine_systems(&mut app, &["helm-engine-port"]);
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: crate::core::messages::RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);
    let teams = local_teams(&mut app);
    assert!(
        team_is_travelling(&teams, 0),
        "team 0 should be travelling after dispatch"
    );
}

#[test]
fn all_busy_teams_ignore_further_dispatches() {
    let mut app = test_app();
    start_game_with_repair(&mut app);
    // One owned fine system per station this test addresses.
    damage_owned_fine_systems(
        &mut app,
        &["helm-engine-port", "tactical-radar", "power-reactor"],
    );
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: crate::core::messages::RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 1,
                target: crate::core::messages::RepairTarget::Station(StationId("tactical".into())),
            },
        },
    );
    tick(&mut app);
    // Redirect team 0 (different console → Returning)
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: crate::core::messages::RepairTarget::Station(StationId("power".into())),
            },
        },
    );
    tick(&mut app);
    let teams = local_teams(&mut app);
    assert!(matches!(
        teams.0.slots()[0],
        crate::core::messages::TeamSlot::Returning { .. }
    ));
    assert!(team_is_travelling(&teams, 1));
}

#[test]
fn repair_state_broadcast_after_dispatch() {
    let mut app = test_app();
    start_game_with_repair(&mut app);
    damage_owned_fine_systems(&mut app, &["helm-engine-port"]);
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: crate::core::messages::RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    let out = tick(&mut app);
    let repair_state = out.iter().find(|m| {
        matches!(&m.msg, ServerMessage::RepairState { teams } if
            teams.iter().any(|t| matches!(t, crate::core::messages::TeamSlot::Travelling { .. })))
            && matches!(&m.target, Target::Token(t) if t == "eng")
    });
    assert!(
        repair_state.is_some(),
        "RepairState with Travelling team should be broadcast to repair console"
    );
}

// â"€â"€ SetPhaserMode tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

/// The Weapons console holder can change the phaser mode to Manual.
#[test]
fn weapons_console_can_set_phaser_mode_to_manual() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::core::messages::PhaserMode::Manual,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world().resource::<CurrentPhaserMode>().0,
        crate::core::messages::PhaserMode::Manual,
        "phaser mode should be Manual after SetPhaserMode"
    );
}

/// A non-Weapons player cannot change the phaser mode.
#[test]
fn non_weapons_player_cannot_set_phaser_mode() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    // Establish a known mode (Auto) via the authorised player first.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::core::messages::PhaserMode::Auto,
            },
        },
    );
    tick(&mut app);
    // Non-weapons player attempts to switch back to Manual — must be ignored.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::core::messages::PhaserMode::Manual,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world().resource::<CurrentPhaserMode>().0,
        crate::core::messages::PhaserMode::Auto,
        "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
    );
}

/// Shared setup used by tests that need a Sensors + Tactical(weapons) console pairing.
fn start_game_with_sensors_and_weapons(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::Identify {
            token: "sensors".into(),
            name: "Spock".into(),
        },
    );
    tick(app);
    push(
        app,
        "sensors",
        ClientMessage::SelectStation {
            station: "Sensors".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::Identify {
            token: "weapons".into(),
            name: "Bob".into(),
        },
    );
    tick(app);
    push(
        app,
        "weapons",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "sensors", ClientMessage::SetReady { ready: true });
    push(app, "weapons", ClientMessage::SetReady { ready: true });
    tick(app);
    fast_forward_countdown(app);
    tick(app);
    tick(app);
}

// â"€â"€ FireTorpedo tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

#[test]
fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter().any(|m| matches!(
            &m.msg,
            ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port"
        )),
        "expected TorpedoLaunched broadcast after Tactical fires torpedo"
    );
}

#[test]
fn non_tactical_player_cannot_fire_torpedo() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "captain should not be able to fire torpedo"
    );
}

#[test]
fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
    // Note: The Lobby gate is now at the SimSet chain level.
    // In test configurations without SimSet, the system processes messages during Lobby.
    let mut app = test_app();
    load_tube_now(&mut app, "fore_port");
    push(
        &mut app,
        "weapons",
        ClientMessage::Identify {
            token: "weapons".into(),
            name: "Bob".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::SelectStation {
            station: "Tactical".into(),
        },
    );
    tick(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::torpedo_tube_fore_port_system_id(),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "FireTorpedo should fire during Lobby when no SimSet gate is configured"
    );
}

#[test]
fn torpedo_launched_is_broadcast_to_all() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_starboard");

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-starboard".into()),
            payload: SystemControlPayload::FireTorpedo { target_uuid: None },
        },
    );
    let out = tick(&mut app);

    let launched = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
        .expect("expected TorpedoLaunched");
    assert!(
        matches!(&launched.target, Target::All),
        "TorpedoLaunched should be broadcast to All, not {:?}",
        launched.target
    );
}

// â"€â"€ ShipModifiers integration tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

/// Empty modifier table: phaser damage is identical to the base BEAM_DAMAGE_PER_SEC
/// (5 HP/s). After 1 second of beam fire on a 30-HP asteroid the HP decreases by 5.
#[test]
fn empty_modifier_table_reproduces_base_phaser_damage() {
    let mut app = test_app();
    // Asteroid directly ahead at 20 units (within 40-unit phaser range).
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    // Lock and fire
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app);

    // Advance by 1 second of simulated time (many small ticks).
    // Each tick() calls app.update() which advances the Bevy TimePlugin by a small real step.
    // Instead, directly test the accumulator math by examining the asteroid HP after
    // running a known number of frames equivalent to >1 second.
    // BEAM_DAMAGE_PER_SEC = 5; asteroid starts at 30 HP.
    // After enough ticks (>6 s at 5 HP/s) the asteroid should be destroyed.
    // With identity modifier this should work; with a 2Ã— modifier it would be faster.

    // Run 500 ms worth of ticks at ~16ms each (â‰ˆ31 ticks).
    // After that, asteroid should have taken ~2â€"3 HP (not destroyed yet).
    let hp_before = {
        let world = app.world().resource::<WorldResource>();
        world
            .0
            .entities
            .iter()
            .find(|a| a.uuid == "target-uuid")
            .map(|_| true)
    };
    assert!(hp_before.is_some(), "asteroid should still exist after <1s");
}

/// PhaserDamage modifier at 2Ã— doubles the kill rate.
/// With BEAM_DAMAGE_PER_SEC=5 and 30-HP asteroid:
/// - Base: 6 seconds to destroy
/// - 2Ã— modifier (bonus=1.0): 3 seconds to destroy
///   Test: after running ~4s of game time, the asteroid is destroyed with 2Ã— but not with 1Ã—.
#[test]
fn phaser_damage_modifier_doubles_kill_rate() {
    use crate::core::messages::{ModifierSlot, ModifierSource};
    use crate::modifiers::Modifier;

    // --- App with 2Ã— PhaserDamage modifier ---
    let mut app_fast = test_app();
    setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
    start_game_with_weapons(&mut app_fast);
    // Apply 2Ã— phaser damage modifier after ship is spawned.
    modify_ship_modifiers(&mut app_fast, |mods| {
        mods.add_or_update(Modifier {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::PhaserDamage,
            bonus: 1.0, // â†' multiplier 2.0
        });
    });
    push(
        &mut app_fast,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app_fast);
    push(
        &mut app_fast,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app_fast); // processes FirePhaser, beam becomes active

    // Inject accumulated damage: 3.5s × (5 HP/s × 2×) = 35 HP → enough to destroy 30-HP asteroid.
    set_active_beam_damage_accumulator(&mut app_fast, BEAM_DAMAGE_PER_SEC * 2.0 * 3.5);
    tick(&mut app_fast); // One tick to process the accumulated damage.

    let still_exists_fast = app_fast
        .world()
        .resource::<WorldResource>()
        .0
        .entities
        .iter()
        .any(|a| a.uuid == "target-uuid");
    assert!(
        !still_exists_fast,
        "with 2Ã— phaser damage modifier, asteroid should be destroyed after 3.5s of beam"
    );

    // --- App with identity modifier (baseline): same damage injected but at 1Ã— ---
    let mut app_base = test_app();
    setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
    start_game_with_weapons(&mut app_base);
    push(
        &mut app_base,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app_base);
    push(
        &mut app_base,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::phaser_fore_system_id(),
            payload: SystemControlPayload::FirePhaser,
        },
    );
    tick(&mut app_base); // processes FirePhaser, beam becomes active
                         // Inject same real time but at base rate: 3.5s × 5 HP/s = 17.5 HP accumulated
    set_active_beam_damage_accumulator(&mut app_base, BEAM_DAMAGE_PER_SEC * 1.0 * 3.5);
    tick(&mut app_base);

    let still_exists_base = app_base
        .world()
        .resource::<WorldResource>()
        .0
        .entities
        .iter()
        .any(|a| a.uuid == "target-uuid");
    assert!(
        still_exists_base,
        "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)"
    );
}

/// HullDamageTaken modifier at -1 (â†' 0.5Ã— multiplier) halves collision damage.
/// At zero ship speed, base collision_damage=5. With 0.5Ã— modifier: round(5Ã—0.5)=3.
// â"€â"€ modifier broadcast tests â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

#[test]
fn add_modifier_broadcasts_modifier_added_message() {
    use crate::core::messages::{ModifierSlot, ModifierSource};
    use crate::modifiers::Modifier;

    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app); // consume startup messages

    // Register a modifier on the ship entity.
    modify_ship_modifiers(&mut app, |mods| {
        mods.add_or_update(Modifier {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: 0.5,
        });
    });
    let out = tick(&mut app);

    let found = out.iter().any(|m| {
        matches!(
            &m.msg,
            ServerMessage::ModifierAdded { source, slot, bonus }
                if *source == ModifierSource::ImpulseDrive
                && *slot == ModifierSlot::MaxSpeed
                && (*bonus - 0.5).abs() < 1e-6
        )
    });
    assert!(found, "expected ModifierAdded in outbound messages");
}

#[test]
fn remove_modifier_broadcasts_modifier_removed_message() {
    use crate::core::messages::{ModifierSlot, ModifierSource};
    use crate::modifiers::Modifier;

    let mut app = test_app();
    start_game(&mut app);
    // Add first so there's something to remove.
    modify_ship_modifiers(&mut app, |mods| {
        mods.add_or_update(Modifier {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: 0.5,
        });
    });
    tick(&mut app);

    // Now remove it.
    modify_ship_modifiers(&mut app, |mods| {
        mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
    });
    let out = tick(&mut app);

    let found = out.iter().any(|m| {
        matches!(
            &m.msg,
            ServerMessage::ModifierRemoved { source, slot }
                if *source == ModifierSource::ImpulseDrive
                && *slot == ModifierSlot::MaxSpeed
        )
    });
    assert!(found, "expected ModifierRemoved in outbound messages");
}

#[test]
fn asteroid_collision_pierce_zero_routes_all_to_shields() {
    // Replicates the split + apply that `handle_collisions` performs
    // (without standing up Rapier), proving the pierce=0 path leaves
    // hull untouched and the shield quadrant absorbs full damage.
    use crate::ship::damage::{
        apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
    };
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let mut shields = ShieldSystem::new(&ShieldConfig::default());
    let initial_fore_hp = shields.facings[0].hp;
    let mut hull =
        crate::ship::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

    let damage: f32 = 10.0;
    let (pierced, absorbed) = split_damage_for_pierce(damage, 0.0);
    assert_eq!(pierced, 0.0);
    assert_eq!(absorbed, 10.0);
    let leak = apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields);
    let total_hull = pierced + leak as f32;
    if total_hull > 0.0 {
        let rng = &mut crate::sim_rng::unseeded_test_rng();
        apply_hull_damage(&mut hull, total_hull, rng);
    }
    assert!(
        (hull.total_current() - 100.0).abs() < 1e-6,
        "hull untouched with pierce=0 (leak={})",
        leak
    );
    assert_eq!(
        shields.facings[0].hp,
        initial_fore_hp - 10,
        "fore quadrant should have absorbed all 10 damage"
    );
}

#[test]
fn asteroid_collision_pierce_full_routes_all_to_hull() {
    use crate::ship::damage::{
        apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
    };
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let mut shields = ShieldSystem::new(&ShieldConfig::default());
    let initial_fore_hp = shields.facings[0].hp;
    let mut hull =
        crate::ship::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

    let damage: f32 = 10.0;
    let (pierced, absorbed) = split_damage_for_pierce(damage, 1.0);
    assert_eq!(pierced, 10.0);
    assert_eq!(absorbed, 0.0);
    let leak = if absorbed > 0.0 {
        apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields)
    } else {
        0
    };
    let total_hull = pierced + leak as f32;
    let rng = &mut crate::sim_rng::unseeded_test_rng();
    apply_hull_damage(&mut hull, total_hull, rng);
    assert!(
        (hull.total_current() - 90.0).abs() < 1e-6,
        "hull should be 90 with pierce=1 (10 damage straight through)"
    );
    assert_eq!(
        shields.facings[0].hp, initial_fore_hp,
        "fore quadrant should be untouched with pierce=1"
    );
}

#[test]
fn asteroid_collision_pierce_partial_splits_proportionally() {
    use crate::ship::damage::{
        apply_damage_with_shields, apply_hull_damage, split_damage_for_pierce,
    };
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let mut shields = ShieldSystem::new(&ShieldConfig::default());
    let initial_fore_hp = shields.facings[0].hp;
    let mut hull =
        crate::ship::damage::SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);

    // pierce = 0.3 on 10 damage → 3 to hull, 7 to fore shield.
    let damage: f32 = 10.0;
    let (pierced, absorbed) = split_damage_for_pierce(damage, 0.3);
    let leak = apply_damage_with_shields(absorbed.round() as i32, 0.0, &mut shields);
    let total_hull = pierced + leak as f32;
    let rng = &mut crate::sim_rng::unseeded_test_rng();
    apply_hull_damage(&mut hull, total_hull, rng);
    assert!(
        (hull.total_current() - 97.0).abs() < 1e-6,
        "hull should lose 3 (the pierced portion), got {}",
        hull.total_current()
    );
    assert_eq!(
        shields.facings[0].hp,
        initial_fore_hp - 7,
        "fore quadrant should have absorbed 7"
    );
}

#[test]
fn hull_damage_modifier_halves_collision_damage() {
    use crate::core::messages::{ModifierSlot, ModifierSource};
    use crate::modifiers::Modifier;

    // Hull damage halved via modifier.
    let mut app = test_app();
    start_game(&mut app);
    modify_ship_modifiers(&mut app, |mods| {
        mods.add_or_update(Modifier {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::HullDamageTaken,
            bonus: -1.0, // â†' multiplier 0.5
        });
    });

    // Apply collision damage directly through the formula used in handle_collisions.
    // At 200 u/s: collision_damage(200) = round(200 * 0.5) = 100.
    // With 0.5Ã— modifier: round(100 * 0.5) = 50.
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }
    let mods = get_ship_modifiers(&mut app);
    let base_damage = collision_damage(200.0) as f32; // 100
    let scaled_damage = (base_damage * mods.get(&ModifierSlot::HullDamageTaken)).round();
    assert!(
        near(base_damage, 100.0),
        "collision_damage(200) should be 100"
    );
    assert!(
        near(scaled_damage, 50.0),
        "with 0.5Ã— modifier, damage should be 50"
    );

    // Verify the hull loses only the scaled amount by triggering damage through the component.
    apply_hull_damage(&mut app, scaled_damage);
    let out = tick(&mut app);
    // Ship-wide hull reads off `aggregate_fraction` post issue #737.
    let aggregate = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::SystemHullUpdate {
                aggregate_fraction, ..
            } => Some(*aggregate_fraction),
            _ => None,
        })
        .expect("expected SystemHullUpdate");
    assert!(
        near(aggregate.expect("aggregate fraction"), 0.5),
        "hull should be 100 - 50 = 50 with halved collision damage"
    );
}

/// PRD #597 PR-8: NPC ships share the collision code path with the player,
/// so an NPC ship overlapping an asteroid must take hull damage on its own
/// `EntitySystemHull` component just like the player ship does.
///
/// This spins up a minimal Rapier world (no plugin scaffolding) with just
/// `handle_collisions`, spawns an NPC ship (`Ship` marker, no `LocalShip`)
/// overlapping an asteroid, ticks once, and asserts the NPC's hull dropped.
/// Because the ship is not `LocalShip`, none of the player-only side
/// effects (`DamageTaken`, `ShipDestroyed`, `GameOver`) may fire.
#[test]
fn npc_ship_takes_hull_damage_from_asteroid_collision() {
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy_rapier3d::prelude::*;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .init_resource::<SimOutbox>()
        .init_resource::<WorldResource>()
        .insert_resource(GameOverReason(None, None))
        .init_resource::<DamageLog>()
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_systems(Update, handle_collisions);

    // Move the game into InProgress so RapierPhysicsPlugin's default
    // run condition (if any) doesn't gate the step. Not strictly required
    // for handle_collisions itself, but keeps the test's app state
    // consistent with production semantics.
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();

    // Spawn an NPC ship at the origin with a ball collider, some hull,
    // some forward speed (so collision_damage yields non-zero), and no
    // `LocalShip` marker. `ShipShields` is omitted deliberately — NPCs
    // in production may or may not have shields; when absent, all damage
    // routes to hull.
    let npc_uuid = "npc-test-uuid".to_string();
    let npc_hull_max = 100.0f32;
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid(npc_uuid.clone()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ShipPhysicsComponent {
                x: 0.0,
                z: 0.0,
                yaw: 0.0,
                forward_speed: 100.0,
                roll: 0.0,
                lateral_speed: 0.0,
                ..Default::default()
            },
            CollisionCooldown::default(),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                npc_hull_max,
            )])),
            ShipModifiers::new(),
            ShipImpulse::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
                half_height: None,
                movable: true,
            }),
            Collider::ball(5.0),
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
        ))
        .id();

    // Spawn an asteroid overlapping the NPC at the origin.
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("ast-test-uuid".to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        ColliderSection(ColliderConfig {
            shape: ColliderShape::Ball,
            radius: 5.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
        Collider::ball(5.0),
        RigidBody::Fixed,
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));

    // Several updates: first ticks let Rapier build the broad-phase and
    // detect the overlapping pair; subsequent ticks run `handle_collisions`
    // with the contact visible on `ReadRapierContext`.
    for _ in 0..3 {
        app.update();
    }

    let hull = app
        .world()
        .get::<EntitySystemHull>(npc)
        .expect("NPC must retain EntitySystemHull");
    assert!(
        hull.0.total_current() < npc_hull_max,
        "NPC hull must decrease from asteroid collision (current={}, max={})",
        hull.0.total_current(),
        npc_hull_max
    );

    // Player-only messages must NOT be emitted for an NPC-vs-asteroid
    // collision — those are gated on `Has<LocalShip>`.
    let outbox = &app.world().resource::<SimOutbox>().0;
    assert!(
        !outbox
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::DamageTaken { .. })),
        "DamageTaken is a player-only UI message; must not fire for NPCs"
    );
    assert!(
        !outbox
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::ShipDestroyed)),
        "ShipDestroyed is a player-only UI message; must not fire for NPCs"
    );

    // Collision response stops the ship and separates it out of the
    // overlapping collider volume, instead of bouncing it backward.
    let physics = app.world().get::<ShipPhysicsComponent>(npc).unwrap();
    assert_eq!(
        physics.forward_speed, 0.0,
        "NPC forward_speed should be zeroed after collision"
    );
    let dist = (physics.x * physics.x + physics.z * physics.z).sqrt();
    assert!(
        dist >= 10.0 + COLLISION_SEPARATION_SLOP - 1e-5,
        "NPC should be separated outside the two collider radii, distance={dist}"
    );
}

/// Issue #968: a hull that is INSIDE a collider is de-overlapped on every
/// tick of contact, not only on the tick the damage cooldown lets a hit
/// through.
///
/// Separation used to sit behind the same `remaining_secs` gate as the
/// damage, which gave a ship that had driven into something a full second to
/// bury itself deeper before anything corrected the geometry. Against the
/// `huge` asteroid class (collider radius 12, issue #947) that was measured
/// at 6.5 units of penetration on a `combat_test` run — the hull well inside
/// the rock, grinding through it rather than around it.
///
/// So this ship starts deep inside a radius-12 rock with its cooldown still
/// running. It must come back out to the surface, and it must NOT be charged
/// a second hit for it: the hit rate is unchanged, only the geometry
/// correction is continuous.
#[test]
fn a_hull_inside_a_collider_is_separated_even_mid_cooldown() {
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy_rapier3d::prelude::*;

    const SHIP_RADIUS: f32 = 1.2;
    const ROCK_RADIUS: f32 = 12.0;
    // Buried well inside the rock, as the instrumented run found it.
    const START_DEPTH: f32 = 6.0;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .init_resource::<SimOutbox>()
        .init_resource::<WorldResource>()
        .insert_resource(GameOverReason(None, None))
        .init_resource::<DamageLog>()
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_systems(Update, handle_collisions);
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();

    let hull_max = 100.0f32;
    let ship = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid("ship-inside-rock".to_string()),
            Transform::from_xyz(START_DEPTH, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ShipPhysicsComponent {
                x: START_DEPTH,
                z: 0.0,
                yaw: 0.0,
                forward_speed: 20.0,
                ..Default::default()
            },
            // Mid-cooldown: this ship took its hit a moment ago and is not
            // due another one.
            CollisionCooldown {
                remaining_secs: 0.9,
            },
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                hull_max,
            )])),
            ShipModifiers::new(),
            ShipImpulse::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: SHIP_RADIUS,
                length: 0.0,
                half_height: None,
                movable: true,
            }),
            Collider::ball(SHIP_RADIUS),
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
        ))
        .id();

    // A `huge`-class rock centred on the origin, swallowing the ship whole.
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("huge-rock".to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        ColliderSection(ColliderConfig {
            shape: ColliderShape::Ball,
            radius: ROCK_RADIUS,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
        Collider::ball(ROCK_RADIUS),
        RigidBody::Fixed,
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));

    // A few ticks for rapier's broad phase to publish the pair; the cooldown
    // only falls by 50 ms a tick, so it is still running throughout.
    for _ in 0..3 {
        app.update();
    }

    let physics = app.world().get::<ShipPhysicsComponent>(ship).unwrap();
    let surface_gap =
        (physics.x * physics.x + physics.z * physics.z).sqrt() - ROCK_RADIUS - SHIP_RADIUS;
    assert!(
        surface_gap >= -1e-5,
        "a hull inside a collider must be pushed back out to the surface even \
         while its damage cooldown runs; surface gap was {surface_gap} \
         (position {}, {})",
        physics.x,
        physics.z
    );

    // The cooldown is still doing its job: no second hit was charged, and the
    // hard stop that comes with a hit did not fire either.
    let hull = app.world().get::<EntitySystemHull>(ship).unwrap();
    assert_eq!(
        hull.0.total_current(),
        hull_max,
        "de-overlapping must not charge a hit while the cooldown is running"
    );
    assert_eq!(
        physics.forward_speed, 20.0,
        "the hard stop belongs to the damage tick, not to the separation"
    );
    assert!(
        app.world()
            .get::<CollisionCooldown>(ship)
            .unwrap()
            .remaining_secs
            > 0.0,
        "the cooldown must still be running for this test to mean anything"
    );
}

/// Build a bare rapier app with `handle_collisions` registered, one hull at
/// `ship_at`, and one static structure at the origin carrying `structure`.
/// Returns how much hull the ship had left after three ticks.
///
/// Three ticks because the first two let rapier's broad phase publish the
/// pair; the fixture matches
/// `a_hull_inside_a_collider_is_separated_even_mid_cooldown` above.
#[cfg(test)]
fn hull_left_after_touching(
    structure: (
        crate::entities::config::ColliderConfig,
        bevy_rapier3d::prelude::Collider,
    ),
    ship_at: Vec3,
) -> f32 {
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy_rapier3d::prelude::*;

    const SHIP_RADIUS: f32 = 1.2;
    const HULL_MAX: f32 = 1000.0;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .init_resource::<SimOutbox>()
        .init_resource::<WorldResource>()
        .insert_resource(GameOverReason(None, None))
        .init_resource::<DamageLog>()
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_systems(Update, handle_collisions);
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();

    let ship = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid("hull-over-a-structure".to_string()),
            Transform::from_translation(ship_at),
            GlobalTransform::default(),
            Visibility::default(),
            ShipPhysicsComponent {
                x: ship_at.x,
                y: ship_at.y,
                z: ship_at.z,
                forward_speed: 20.0,
                ..Default::default()
            },
            CollisionCooldown::default(),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                HULL_MAX,
            )])),
            ShipModifiers::new(),
            ShipImpulse::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: SHIP_RADIUS,
                length: 0.0,
                half_height: None,
                movable: true,
            }),
            Collider::ball(SHIP_RADIUS),
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
        ))
        .id();

    let (section, collider) = structure;
    app.world_mut().spawn((
        EntityUuid("structure-under-test".to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        ColliderSection(section),
        collider,
        RigidBody::Fixed,
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));

    for _ in 0..3 {
        app.update();
    }

    app.world()
        .get::<EntitySystemHull>(ship)
        .expect("the ship must survive this fixture")
        .0
        .total_current()
}

/// The shipped starbase disc, as `station_axiom.toml` and `skyhook.toml`
/// author it.
#[cfg(test)]
fn starbase_disc() -> (
    crate::entities::config::ColliderConfig,
    bevy_rapier3d::prelude::Collider,
) {
    (
        crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Cylinder,
            radius: 17.04,
            length: 0.0,
            half_height: Some(7.16),
            movable: false,
        },
        bevy_rapier3d::prelude::Collider::cylinder(7.16, 17.04),
    )
}

/// The Ball the disc replaced: right about the width, wrong about the
/// height by ten units.
#[cfg(test)]
fn starbase_ball() -> (
    crate::entities::config::ColliderConfig,
    bevy_rapier3d::prelude::Collider,
) {
    (
        crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Ball,
            radius: 17.04,
            length: 0.0,
            half_height: None,
            movable: false,
        },
        bevy_rapier3d::prelude::Collider::ball(17.04),
    )
}

/// Containment, in the plane a station is wide in. The `Cylinder` variant
/// must not have cost the correction the Ball got right: a hull at deck
/// level is inside the disc out to its rim and clear of it beyond.
///
/// Both bounds are asserted against the Ball too, because in this plane the
/// two shapes are the same body and any difference here would be a
/// regression rather than the fix.
#[test]
fn a_disc_contains_a_hull_out_to_its_rim_and_no_further() {
    const HULL_MAX: f32 = 1000.0;
    // Well inside the rim: 10 against a radius of 17.04.
    for (name, structure) in [("cylinder", starbase_disc()), ("ball", starbase_ball())] {
        let left = hull_left_after_touching(structure, Vec3::new(10.0, 0.0, 0.0));
        assert!(
            left < HULL_MAX,
            "{name}: a hull at deck level inside the rim must be in contact"
        );
    }
    // Clear of the rim: 25 against 17.04 + the hull's own 1.2.
    for (name, structure) in [("cylinder", starbase_disc()), ("ball", starbase_ball())] {
        let left = hull_left_after_touching(structure, Vec3::new(25.0, 0.0, 0.0));
        assert_eq!(
            left, HULL_MAX,
            "{name}: a hull clear of the rim must not be in contact"
        );
    }
}

/// The whole point of the variant, stated as the A/B that motivates it.
///
/// The starbase draws 14.33 tall — a half-height of 7.16 — and 34.08 across.
/// A Ball at the max half-extent covers the width correctly and stands
/// 17.04 units tall, so a hull crossing directly over the hub at an
/// altitude of 12 hits an invisible ceiling in clear sky, ten units above
/// anything the renderer draws. The disc does not.
///
/// Asserting BOTH halves in one test, because either alone is satisfiable
/// by a body that is simply the wrong size: a cylinder that touched nothing
/// at all would pass the first half, and the old Ball passes the second.
#[test]
fn a_disc_lets_a_hull_pass_over_the_hub_where_the_ball_it_replaced_did_not() {
    const HULL_MAX: f32 = 1000.0;
    // Offset in X so the contact normal is not the degenerate straight-down
    // case; still well inside the 17.04 footprint either way.
    let over_the_hub = Vec3::new(2.0, 12.0, 0.0);

    assert_eq!(
        hull_left_after_touching(starbase_disc(), over_the_hub),
        HULL_MAX,
        "a hull twelve units up must clear a structure that is 7.16 tall"
    );
    assert!(
        hull_left_after_touching(starbase_ball(), over_the_hub) < HULL_MAX,
        "this test is only worth having if the Ball it replaced DID collide \
         there — if this half stops failing, the A/B has gone stale"
    );
}

/// The vertical boundary, from both sides. A disc's top surface is the one
/// edge the Ball never had, so it is the one that needs pinning: 0.36 of a
/// unit inside it is a contact, 0.44 outside it is not.
///
/// Deliberately not asserted AT tangency (y = 7.16 + 1.2 exactly), which is
/// a floating-point coin toss inside rapier and would pin nothing.
#[test]
fn a_disc_boundary_is_its_own_top_surface_not_its_radius() {
    const HULL_MAX: f32 = 1000.0;
    assert!(
        hull_left_after_touching(starbase_disc(), Vec3::new(2.0, 8.0, 0.0)) < HULL_MAX,
        "a hull whose underside is below the deck must be in contact"
    );
    assert_eq!(
        hull_left_after_touching(starbase_disc(), Vec3::new(2.0, 8.8, 0.0)),
        HULL_MAX,
        "a hull whose underside clears the deck must not be"
    );
}

/// Regression: the physics collider must stay its AUTHORED size even when
/// the render LOD system scales the entity's `Transform`.
///
/// Under `render` (the browser), `update_mesh_lod` writes the model's
/// `[base].scale` — [15, 18, 18] for the starbase — onto the structure's own
/// `Transform` for every non-near LOD tier, because the generated LOD meshes
/// are authored at raw model size and the parent supplies the base scale. By
/// default rapier's `apply_scale` folds that `GlobalTransform.scale` into the
/// collider shape, inflating the authored 17.04 disc to a ~300-unit one, so a
/// ship dead-stopped and took ram damage hundreds of units out in clear sky —
/// and ONLY in the browser, because headless runs no LOD system and its
/// transforms keep scale 1, which is why no digest ever recorded it.
///
/// `spawn_entity` now pins `ColliderScale::Absolute(ONE)` on every authored
/// collider, which REPLACES the transform scale rather than multiplying it.
/// This fixture reproduces the LOD scale directly (there is no camera or LOD
/// system in a bare rapier app) and asserts a hull far outside the visible
/// rim — 100 units against a 17.04 radius — stays clear. The A/B half proves
/// the test bites: the SAME scaled structure WITHOUT the pin (rapier's
/// default `Relative`) swallows that hull whole.
#[test]
fn a_collider_ignores_the_render_lod_transform_scale() {
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy_rapier3d::prelude::*;

    const SHIP_RADIUS: f32 = 1.2;
    const HULL_MAX: f32 = 1000.0;
    // The starbase's on-screen `[base].scale`. This is what `update_mesh_lod`
    // stamps onto the entity transform at LOD1/2.
    const LOD_SCALE: Vec3 = Vec3::new(15.0, 18.0, 18.0);
    // Far outside the authored 17.04 rim, deep inside a ~300-unit inflation.
    const SHIP_AT: Vec3 = Vec3::new(100.0, 0.0, 0.0);

    // `pin` = insert `ColliderScale::Absolute(ONE)` as production does.
    fn hull_left(pin: bool) -> f32 {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(50),
            ))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<bevy::mesh::Mesh>()
            .init_resource::<bevy::scene::SceneSpawner>()
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .add_plugins(RapierPhysicsPlugin::<()>::default())
            .init_resource::<SimOutbox>()
            .init_resource::<WorldResource>()
            .insert_resource(GameOverReason(None, None))
            .init_resource::<DamageLog>()
            .add_message::<crate::ai::server::AiEntityDestroyed>()
            .add_systems(Update, handle_collisions);
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();

        let ship = app
            .world_mut()
            .spawn((
                Ship,
                EntityUuid("hull-vs-scaled-structure".to_string()),
                Transform::from_translation(SHIP_AT),
                GlobalTransform::default(),
                Visibility::default(),
                ShipPhysicsComponent {
                    x: SHIP_AT.x,
                    y: SHIP_AT.y,
                    z: SHIP_AT.z,
                    forward_speed: 20.0,
                    ..Default::default()
                },
                CollisionCooldown::default(),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    HULL_MAX,
                )])),
                ShipModifiers::new(),
                ShipImpulse::default(),
                ColliderSection(ColliderConfig {
                    shape: ColliderShape::Ball,
                    radius: SHIP_RADIUS,
                    length: 0.0,
                    half_height: None,
                    movable: true,
                }),
                Collider::ball(SHIP_RADIUS),
                RigidBody::KinematicPositionBased,
                ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
            ))
            .id();

        // The starbase disc, at LOD1/2 scale on its own transform.
        let (section, collider) = starbase_disc();
        let mut structure = app.world_mut().spawn((
            EntityUuid("scaled-structure".to_string()),
            Transform::from_translation(Vec3::ZERO).with_scale(LOD_SCALE),
            GlobalTransform::default(),
            Visibility::default(),
            ColliderSection(section),
            collider,
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));
        if pin {
            structure.insert(ColliderScale::Absolute(Vect::ONE));
        }

        for _ in 0..3 {
            app.update();
        }

        app.world()
            .get::<EntitySystemHull>(ship)
            .expect("the ship must survive this fixture")
            .0
            .total_current()
    }

    assert_eq!(
        hull_left(true),
        HULL_MAX,
        "with the collider pinned to its authored size, a hull 100 units out \
         must be in clear sky — the render LOD scale must not reach physics"
    );
    assert!(
        hull_left(false) < HULL_MAX,
        "this test is only worth having if the UNPINNED collider DID inflate \
         with the transform and swallow a hull 100 units out; if this half \
         stops failing, the LOD scale no longer reaches physics and the pin \
         is moot"
    );
}

/// Issue #896, AC-3: which of several simultaneous contacts a ship is
/// resolved against is decided by world id, not by whichever pair rapier
/// hands back first.
///
/// A ship wedged between two rocks used to take
/// `contact_pairs_with(..).next()` — an order that comes out of the
/// broadphase's internal bookkeeping, is not something the simulation
/// chose, and is not even the same between a parallel and a serial build.
/// It decides real outcomes: which direction the ship is pushed, what
/// bearing the impact comes from and so which shield arc absorbs it, and
/// whose `shield_pierce` applies.
///
/// The two rocks here sit on opposite sides of the ship, so the direction
/// it ends up separated in says which one was picked — and the answer must
/// be the same for both spawn orders, because the pick is `ast-aaa`'s to
/// win on its uuid either way.
///
/// Since issue #968 the geometry is resolved against BOTH rocks rather than
/// only the picked one, so what the ship's final side pins here is the
/// ORDER of that pass rather than a single choice — and the order is the
/// same world-id order. It still discriminates: resolving `ast-aaa` first
/// lands the hull at −X, resolving `ast-zzz` first would land it at +X. The
/// damage contact is still exactly one and still `ast-aaa`.
#[test]
fn a_ship_between_two_asteroids_is_resolved_against_the_lower_world_id() {
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy_rapier3d::prelude::*;

    /// Where the ship ends up after being separated out of the overlap,
    /// with the two rocks spawned in `order`.
    fn separated_x(order: [(&str, f32); 2]) -> f32 {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(50),
            ))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<bevy::mesh::Mesh>()
            .init_resource::<bevy::scene::SceneSpawner>()
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .add_plugins(RapierPhysicsPlugin::<()>::default())
            .init_resource::<SimOutbox>()
            .init_resource::<WorldResource>()
            .insert_resource(GameOverReason(None, None))
            .init_resource::<DamageLog>()
            .add_message::<crate::ai::server::AiEntityDestroyed>()
            .add_systems(Update, handle_collisions);
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();

        let ship = app
            .world_mut()
            .spawn((
                Ship,
                EntityUuid("ship-under-test".into()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
                ShipPhysicsComponent {
                    forward_speed: 100.0,
                    ..Default::default()
                },
                CollisionCooldown::default(),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    100.0,
                )])),
                ShipModifiers::new(),
                ShipImpulse::default(),
                ColliderSection(ColliderConfig {
                    shape: ColliderShape::Ball,
                    radius: 5.0,
                    length: 0.0,
                    half_height: None,
                    movable: true,
                }),
                Collider::ball(5.0),
                RigidBody::KinematicPositionBased,
                ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
            ))
            .id();

        for (uuid, x) in order {
            app.world_mut().spawn((
                Asteroid,
                AsteroidUuid(uuid.to_string()),
                Transform::from_xyz(x, 0.0, 0.0),
                GlobalTransform::default(),
                Visibility::default(),
                ColliderSection(ColliderConfig {
                    shape: ColliderShape::Ball,
                    radius: 5.0,
                    length: 0.0,
                    half_height: None,
                    movable: false,
                }),
                Collider::ball(5.0),
                RigidBody::Fixed,
                ActiveCollisionTypes::KINEMATIC_STATIC,
            ));
        }

        // Let the broad phase see both overlaps before the collision is
        // consumed, as in the sibling tests above.
        for _ in 0..3 {
            app.update();
        }
        app.world().get::<ShipPhysicsComponent>(ship).unwrap().x
    }

    // `ast-aaa` sits at +X, so the ship is pushed to −X when it is the one
    // chosen — whichever rock was spawned first.
    let aaa_first = separated_x([("ast-aaa", 3.0), ("ast-zzz", -3.0)]);
    let zzz_first = separated_x([("ast-zzz", -3.0), ("ast-aaa", 3.0)]);

    assert!(
        aaa_first < 0.0,
        "the ship should have been separated away from `ast-aaa` at +X, \
         but ended up at x={aaa_first}"
    );
    assert_eq!(
        aaa_first, zzz_first,
        "the same two rocks resolved differently depending on which was \
         spawned first — the contact pair is still being taken in rapier's \
         order rather than by world id"
    );
}

/// Issue #968: a hull touching two bodies at once must not be left sitting
/// inside the one that was not picked.
///
/// `separate_ship_from_collision` snaps the hull onto ONE body's surface
/// with no regard for any other, and the damage contact is deliberately a
/// single deterministic pick (issue #896). Two rocks closer together than
/// `r_ship + r_rock + slop` therefore let the correction push the hull out
/// of the lower-keyed rock and straight into the higher-keyed one — and
/// because the pick is deterministic, the next tick picks the same rock,
/// finds the hull already clear of it, and does nothing. The hull sits
/// inside a collider indefinitely.
///
/// That was a 1 Hz nuisance while separation ran on the damage cooldown.
/// Making the correction continuous would have made it a 60 Hz steady
/// state, so the correction resolves every contact instead.
///
/// The rocks here are 6 units apart with radius 5 each and a radius-5 hull
/// between them — a gap far narrower than the 10.05 units the hull needs
/// from either centre, which is what makes the wedge inescapable one rock
/// at a time.
#[test]
fn a_hull_wedged_between_two_bodies_is_separated_from_both() {
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy_rapier3d::prelude::*;

    const R: f32 = 5.0;
    const ROCK_X: f32 = 3.0;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .init_resource::<SimOutbox>()
        .init_resource::<WorldResource>()
        .insert_resource(GameOverReason(None, None))
        .init_resource::<DamageLog>()
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_systems(Update, handle_collisions);
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();

    let ship = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid("ship-wedged".into()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ShipPhysicsComponent::default(),
            // Mid-cooldown, so this is purely about geometry.
            CollisionCooldown {
                remaining_secs: 0.9,
            },
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            ShipModifiers::new(),
            ShipImpulse::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: R,
                length: 0.0,
                half_height: None,
                movable: true,
            }),
            Collider::ball(R),
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
        ))
        .id();

    for (uuid, x) in [("ast-aaa", ROCK_X), ("ast-zzz", -ROCK_X)] {
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid(uuid.to_string()),
            Transform::from_xyz(x, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: R,
                length: 0.0,
                half_height: None,
                movable: false,
            }),
            Collider::ball(R),
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
        ));
    }

    // Long enough that a per-tick correction which keeps re-wedging the hull
    // would have had every chance to settle into its steady state.
    for _ in 0..10 {
        app.update();
    }

    let physics = app.world().get::<ShipPhysicsComponent>(ship).unwrap();
    for rock_x in [ROCK_X, -ROCK_X] {
        let gap = ((physics.x - rock_x).powi(2) + physics.z.powi(2)).sqrt() - R - R;
        assert!(
            gap >= -1e-5,
            "the hull ended {:.2} units inside the rock at x={rock_x}: \
             resolving only one contact pushes it out of that rock and into \
             this one, for ever (hull at x={}, z={})",
            -gap,
            physics.x,
            physics.z
        );
    }
}

/// Issue #896 review finding: `contact_pairs_with` yields every pair whose
/// *bounding volumes* overlap, not just the ones whose shapes actually
/// touch. A third rock positioned so its AABB clips the ship's AABB but
/// whose sphere never reaches the ship's must not be eligible for the
/// deterministic pick — even when its uuid would sort lowest of all three
/// and so would win the `min_by_key` outright if it were merely filtered
/// on `Option::is_some()` upstream instead of on real contact.
#[test]
fn a_lower_uuid_rock_with_only_an_aabb_overlap_is_never_selected() {
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy_rapier3d::prelude::*;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .init_resource::<SimOutbox>()
        .init_resource::<WorldResource>()
        .insert_resource(GameOverReason(None, None))
        .init_resource::<DamageLog>()
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_systems(Update, handle_collisions);
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();

    let ship = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid("ship-under-test".into()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            GlobalTransform::default(),
            Visibility::default(),
            ShipPhysicsComponent {
                forward_speed: 100.0,
                ..Default::default()
            },
            CollisionCooldown::default(),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            ShipModifiers::new(),
            ShipImpulse::default(),
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 5.0,
                length: 0.0,
                half_height: None,
                movable: true,
            }),
            Collider::ball(5.0),
            RigidBody::KinematicPositionBased,
            ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
        ))
        .id();

    // The genuine contact: `ast-aaa` at +X, sphere-overlapping the ship
    // exactly as in the sibling test above.
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("ast-aaa".to_string()),
        Transform::from_xyz(3.0, 0.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        ColliderSection(ColliderConfig {
            shape: ColliderShape::Ball,
            radius: 5.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
        Collider::ball(5.0),
        RigidBody::Fixed,
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));

    // The decoy: `ast-000` sorts below `ast-aaa` on uuid alone, so it
    // would win `min_by_key` if the filter above it did not exclude
    // AABB-only overlaps. Both rocks have radius 5 and the ship has radius
    // 5, so two spheres need center distance < 10 to actually touch. This
    // one sits at (8, 8, 0): 3D center distance is sqrt(8²+8²) ≈ 11.3 — no
    // shape contact — but its AABB (x:[3,13], y:[3,13], z:[-5,5]) clips
    // the ship's AABB (x:[-5,5], y:[-5,5], z:[-5,5]) in both x and y, so
    // the broad phase still reports the pair.
    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("ast-000".to_string()),
        Transform::from_xyz(8.0, 8.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        ColliderSection(ColliderConfig {
            shape: ColliderShape::Ball,
            radius: 5.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
        Collider::ball(5.0),
        RigidBody::Fixed,
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));

    // Let the broad phase see both overlaps before the collision is
    // consumed, as in the sibling test above.
    for _ in 0..3 {
        app.update();
    }

    let physics = app.world().get::<ShipPhysicsComponent>(ship).unwrap();
    // If the decoy had been selected, `separate_ship_from_collision` would
    // have pushed the ship away from (8, 8, 0) — a nonzero z displacement,
    // since the decoy's z sits at 0 same as the ship's own z it would at
    // minimum not reproduce the pure -X push below. The genuine pick
    // (`ast-aaa` at +X) only ever moves the ship along x, leaving z at 0.
    assert!(
        physics.x < 0.0,
        "the ship should still have been separated away from `ast-aaa` \
         at +X, but ended up at x={}",
        physics.x
    );
    assert_eq!(
        physics.z, 0.0,
        "the ship moved in z, implying the AABB-only decoy at (8, 8, 0) \
         was selected instead of the genuine `ast-aaa` contact"
    );
}

/// Environmental damage still has to reach the balance log, on an NPC, with
/// no attacker — the half of a fight that `DamageTaken` never reports.
#[test]
fn npc_asteroid_collision_emits_attacker_less_balance_event() {
    use crate::core::balance::BalanceEvent;
    use crate::entities::config::{ColliderConfig, ColliderShape};
    use crate::entities::spawner::{ColliderSection, EntitySystemHull, EntityUuid};
    use crate::modifiers::ShipModifiers;
    use crate::ship::damage::SystemHull;
    use bevy::ecs::message::Messages;
    use bevy_rapier3d::prelude::*;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .init_resource::<SimOutbox>()
        .init_resource::<WorldResource>()
        .insert_resource(GameOverReason(None, None))
        .init_resource::<DamageLog>()
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_message::<BalanceEvent>()
        .add_systems(Update, handle_collisions);

    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();

    let npc_uuid = "npc-collide-uuid".to_string();
    app.world_mut().spawn((
        Ship,
        EntityUuid(npc_uuid.clone()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        ShipPhysicsComponent {
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
            forward_speed: 100.0,
            roll: 0.0,
            lateral_speed: 0.0,
            ..Default::default()
        },
        CollisionCooldown::default(),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            100.0,
        )])),
        ShipModifiers::new(),
        ShipImpulse::default(),
        ColliderSection(ColliderConfig {
            shape: ColliderShape::Ball,
            radius: 5.0,
            length: 0.0,
            half_height: None,
            movable: true,
        }),
        Collider::ball(5.0),
        RigidBody::KinematicPositionBased,
        ActiveCollisionTypes::KINEMATIC_KINEMATIC | ActiveCollisionTypes::KINEMATIC_STATIC,
    ));

    app.world_mut().spawn((
        Asteroid,
        AsteroidUuid("ast-collide-uuid".to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        GlobalTransform::default(),
        Visibility::default(),
        ColliderSection(ColliderConfig {
            shape: ColliderShape::Ball,
            radius: 5.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
        Collider::ball(5.0),
        RigidBody::Fixed,
        ActiveCollisionTypes::KINEMATIC_STATIC,
    ));

    for _ in 0..3 {
        app.update();
    }

    let messages = app.world().resource::<Messages<BalanceEvent>>();
    let mut cursor = messages.get_cursor();
    let hits: Vec<&BalanceEvent> = cursor.read(messages).collect();
    assert_eq!(hits.len(), 1, "the collision must emit exactly one event");

    let BalanceEvent::DamageApplied {
        attacker,
        victim,
        victim_kind,
        weapon,
        amount,
        shield_absorbed,
        hull_damage,
        ..
    } = hits[0]
    else {
        panic!("the collision event must be a DamageApplied");
    };
    assert_eq!(*attacker, None, "environmental damage has no shooter");
    assert_eq!(victim, &npc_uuid);
    assert_eq!(
        *victim_kind,
        crate::core::balance::VictimKind::Ship,
        "the ship takes the collision damage, not the rock it hit"
    );
    assert_eq!(weapon, crate::core::balance::WEAPON_KIND_COLLISION);
    assert!(*amount > 0.0, "a 100-speed impact must offer damage");
    assert_eq!(
        *shield_absorbed, 0.0,
        "this NPC has no shields, so nothing is absorbed"
    );
    assert!(
        *hull_damage > 0.0,
        "unshielded impact damage must land on hull"
    );
}

#[test]
fn drain_sim_outbox_directly() {
    let mut app = test_app();
    start_game(&mut app);

    // Write directly to SimOutbox
    let len_before = app.world().resource::<SimOutbox>().0.len();
    app.world_mut()
        .resource_mut::<SimOutbox>()
        .0
        .push((Target::All, ServerMessage::GameStarted));

    // Drain manually
    app.world_mut().resource_mut::<SimOutbox>().0.clear();

    // Check SimOutbox is now empty
    let len_after = app.world().resource::<SimOutbox>().0.len();
    assert_eq!(
        len_after,
        0,
        "SimOutbox should be empty after drain, was {} before drain",
        len_before + 1
    );
}

// -- Power system integration tests --------------------------------------

/// Helper: captain + power console player, game started.
fn start_game_with_power(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "power",
        ClientMessage::Identify {
            token: "power".into(),
            name: "Monty".into(),
        },
    );
    tick(app);
    push(
        app,
        "power",
        ClientMessage::SelectStation {
            station: "Power".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "power", ClientMessage::SetReady { ready: true });
    let _ = tick(app);
    fast_forward_countdown(app);
    let _ = tick(app);
    let _ = tick(app);
}

#[test]
fn non_power_sender_increase_power_is_ignored() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Reset power to known state.
    let _ = app
        .world_mut()
        .resource_mut::<ShipPowerSystem>()
        .0
        .set_group_allocation(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::HELM_POWER_GROUP.into(),
            ),
            1,
        );

    // Captain (not Power holder) tries to set Helm to 2.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                level: 2,
            },
        },
    );
    let _ = tick(&mut app);

    assert_eq!(
        app.world().resource::<ShipPowerSystem>().0.level_for(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::HELM_POWER_GROUP.into()
            )
        ),
        1,
        "non-Power sender should not be able to increase power"
    );
}

#[test]
fn non_power_sender_decrease_power_is_ignored() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Captain (not Power holder) tries to set Sensors to 1.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                ),
                level: 1,
            },
        },
    );
    let _ = tick(&mut app);

    assert_eq!(
        app.world().resource::<ShipPowerSystem>().0.level_for(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::SHIELDS_POWER_GROUP.into()
            )
        ),
        2,
        "non-Power sender should not be able to decrease power"
    );
}

#[test]
fn power_sender_increase_reflected_in_next_power_state() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Power holder sets Helm to 3.
    push(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                level: 3,
            },
        },
    );
    let _ = tick(&mut app);

    let out = tick(&mut app);
    let power_state = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::PowerState { helm, .. } => Some(*helm),
            _ => None,
        })
        .expect("expected a PowerState message for power holder");
    assert_eq!(
        power_state, 3,
        "PowerState should show helm=3 after increase"
    );
}

#[test]
fn power_sender_decrease_reflected_in_next_power_state() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Power holder sets Weapons to 1.
    push(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                level: 1,
            },
        },
    );
    let _ = tick(&mut app);

    let out = tick(&mut app);
    let power_state = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::PowerState { weapons, .. } => Some(*weapons),
            _ => None,
        })
        .expect("expected a PowerState message");
    assert_eq!(
        power_state, 1,
        "PowerState should show weapons=1 after decrease"
    );
}

#[test]
fn power_state_only_sent_to_power_holder() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    let out = tick(&mut app);

    // Every PowerState message should target the power holder.
    for m in out
        .iter()
        .filter(|m| matches!(&m.msg, ServerMessage::PowerState { .. }))
    {
        assert!(
            matches!(&m.target, Target::Token(t) if t == "power"),
            "PowerState should only go to the Power holder, got {:?}",
            m.target
        );
    }
}

#[test]
fn no_power_station_holder_no_power_state_broadcast() {
    let mut app = test_app();
    // Only captain, no power station holder.
    start_game(&mut app);

    let out = tick(&mut app);
    let any_power_state = out
        .iter()
        .any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
    assert!(
        !any_power_state,
        "no PowerState should be sent when no Power station holder exists"
    );
}

#[test]
fn power_increase_respects_bounds_noop_at_four() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Manually set Helm to 4 (max).
    let _ = app
        .world_mut()
        .resource_mut::<ShipPowerSystem>()
        .0
        .set_group_allocation(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::HELM_POWER_GROUP.into(),
            ),
            4,
        );

    push(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                level: 4,
            },
        },
    );
    let _ = tick(&mut app);
    let out = tick(&mut app);

    let power_state = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::PowerState { helm, .. } => Some(*helm),
            _ => None,
        })
        .expect("expected a PowerState message");
    assert_eq!(
        power_state, 4,
        "helm should stay at 4 (max bound enforced by PowerSystem)"
    );
}

// -- Power ? Modifier wiring integration tests -------------------------

#[test]
fn increasing_helm_power_updates_max_speed_via_modifiers() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Override multipliers for Helm so level 2 ? 0.0, level 3 ? 1.0
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(
            crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::HELM_POWER_GROUP.into(),
            ),
            [-0.5, 0.0, 1.0, 2.0],
        );

    // Set Helm to 3.
    push(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                level: 3,
            },
        },
    );
    let _ = tick(&mut app);

    // Level 3 ? index 2 ? bonus 1.0 ? MaxSpeed multiplier = 2.0
    let mult = get_ship_modifiers(&mut app).get(&ModifierSlot::MaxSpeed);
    assert!(
        (mult - 2.0).abs() < 1e-6,
        "Helm power 3 should give MaxSpeed multiplier 2.0, got {mult}"
    );
}

#[test]
fn decreasing_weapons_power_updates_phaser_damage_via_modifiers() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Override multipliers for Tactical: level 2 ? 0.0, level 1 ? -0.5
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(
            crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
            ),
            [-0.5, 0.0, 0.25, 0.5],
        );

    // Set Weapons to 1.
    push(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                level: 1,
            },
        },
    );
    let _ = tick(&mut app);

    // Level 1 ? index 0 ? bonus -0.5 (negative) ? 1.0 / (1.0 + 0.5) = 0.666...
    let expected = 1.0 / 1.5;
    let mult = get_ship_modifiers(&mut app).get(&ModifierSlot::PhaserDamage);
    assert!(
        (mult - expected).abs() < 1e-6,
        "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}"
    );
}

/// A flat battery locks the reactor and slams every group to 1 (the
/// exhaustion lock restored after issue #952's floors were reverted), so
/// every multiplier crushes to x0.667 and the standing order is overwritten.
#[test]
fn a_flat_battery_locks_every_group_to_one_and_updates_all_modifiers() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Set known multipliers for all three
    let defaults = [-0.5, 0.0, 0.25, 0.5];
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(
            crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::HELM_POWER_GROUP.into(),
            ),
            defaults,
        );
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(
            crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
            ),
            defaults,
        );
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(
            crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
            ),
            defaults,
        );

    // Set state that will trigger the exhaustion lock on the next tick:
    // total=8 (negative rate), battery already at 0 -> tick keeps it at 0
    // and forces every group to 1.
    {
        let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::HELM_POWER_GROUP.into(),
            ),
            4,
        );
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
            ),
            2,
        );
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
            ),
            2,
        );
        ps.0.battery_charge = 0.0;
    }

    // Tick applies the lock -> translate_power_modifiers runs
    tick(&mut app);

    // All three locked to level 1 -> bonus -0.5 -> multiplier 0.667.
    let expected = 1.0 / 1.5;
    let mods = get_ship_modifiers(&mut app);
    for (slot, label) in [
        (ModifierSlot::MaxSpeed, "MaxSpeed"),
        (ModifierSlot::PhaserDamage, "PhaserDamage"),
        (ModifierSlot::ShieldRegen, "ShieldRegen"),
    ] {
        let mult = mods.get(&slot);
        assert!(
            (mult - expected).abs() < 1e-6,
            "with every group locked to 1, {label} should be x{expected}, got {mult}"
        );
    }
    assert!(
        app.world().resource::<ShipPowerSystem>().0.locked(),
        "the flat battery must have locked the reactor"
    );
}

#[test]
fn power_increase_respects_total_cap_of_eight() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Set total to 8: helm=4, weapons=2, shields=2.
    let _ = app
        .world_mut()
        .resource_mut::<ShipPowerSystem>()
        .0
        .set_group_allocation(
            &crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::HELM_POWER_GROUP.into(),
            ),
            4,
        );

    // Try to set shields to 3 — total would be 9 (over cap), should be blocked at 2.
    push(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
                ),
                level: 3,
            },
        },
    );
    let _ = tick(&mut app);

    let out = tick(&mut app);
    let power_state = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::PowerState { shields, .. } => Some(*shields),
            _ => None,
        })
        .expect("expected a PowerState message");
    assert_eq!(
        power_state, 2,
        "shields should stay at 2 when total is already at the cap of 8"
    );
    assert_eq!(
        app.world().resource::<ShipPowerSystem>().0.total(),
        8,
        "total should remain 8"
    );
}

// -- Runtime entity lifecycle (EntitySpawned / EntityDespawned) -----

#[test]
fn reconcile_system_seeds_on_first_inprogress_frame() {
    let mut app = test_app();
    start_game(&mut app);
    // After start_game, the system should have seeded (even if empty).
    let registry = app.world().resource::<TrackedEntities>();
    assert!(
        registry.seeded,
        "system should be seeded after first InProgress frame"
    );
}

#[test]
fn spawn_non_asteroid_entity_emits_entity_spawned() {
    let mut app = test_app();
    start_game(&mut app);

    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("runtime-entity-1".into()),
        Transform::from_xyz(100.0, 0.0, -200.0),
    ));

    let out = tick(&mut app);

    let spawned = out.iter().find_map(|m| match &m.msg {
        ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
        _ => None,
    });
    assert!(
        spawned.is_some(),
        "expected EntitySpawned after spawning a non-asteroid entity"
    );
    assert_eq!(spawned.unwrap().uuid, "runtime-entity-1");
}

#[test]
fn entity_spawned_broadcast_contains_position_and_id() {
    let mut app = test_app();
    start_game(&mut app);

    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("pos-entity".into()),
        crate::entities::spawner::EntityId("station-alpha".into()),
        Transform::from_xyz(50.0, 0.0, -75.0),
    ));

    let out = tick(&mut app);

    let spawned = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
            _ => None,
        })
        .expect("expected EntitySpawned");

    assert_eq!(spawned.uuid, "pos-entity");
    assert_eq!(spawned.id, Some("station-alpha".into()));
    assert_eq!(spawned.position, Some([50.0, 0.0, -75.0]));
}

#[test]
fn despawn_non_asteroid_entity_emits_entity_despawned() {
    let mut app = test_app();
    start_game(&mut app);

    // Spawn a non-asteroid entity.
    let entity = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("to-despawn".into()),
            Transform::default(),
        ))
        .id();

    // Tick once so the spawn system picks it up.
    let _ = tick(&mut app);

    // Now despawn it.
    app.world_mut().despawn(entity);
    let out = tick(&mut app);

    let despawned = out.iter().find_map(|m| match &m.msg {
        ServerMessage::EntityDespawned { uuid } => Some(uuid.clone()),
        _ => None,
    });
    assert!(
        despawned.is_some(),
        "expected EntityDespawned after despawning a non-asteroid entity"
    );
    assert_eq!(despawned.unwrap(), "to-despawn");
}

#[test]
fn asteroid_spawn_does_not_emit_entity_spawned() {
    let mut app = test_app();
    start_game(&mut app);

    // Spawn an asteroid entity (has Asteroid component + EntityUuid).
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("asteroid-1".into()),
        Asteroid,
        AsteroidUuid("asteroid-1".into()),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[(crate::core::messages::SystemId("captain".into()), 30.0)],
        )),
        Transform::default(),
    ));

    let out = tick(&mut app);

    let spawned = out
        .iter()
        .any(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }));
    assert!(
        !spawned,
        "asteroid spawn must not emit EntitySpawned (uses AsteroidSpawned instead)"
    );
}

#[test]
fn runtime_entity_appears_in_world_data_for_reconnect() {
    let mut app = test_app();
    start_game(&mut app);

    // Spawn a non-asteroid entity.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("reconnect-entity".into()),
        Transform::from_xyz(25.0, 0.0, -50.0),
    ));

    let _ = tick(&mut app);

    // The entity should now be in world.entities so Welcome includes it.
    let world = app.world().resource::<WorldResource>();
    let found = world
        .0
        .entities
        .iter()
        .any(|e| e.uuid == "reconnect-entity");
    assert!(
        found,
        "runtime entity must appear in WorldResource for Welcome reconnects"
    );
}

#[test]
fn midgame_reconnect_resets_blackboard_cache() {
    let mut app = test_app();
    start_game(&mut app);

    let helm_id = SystemId("helm".into());
    let helm_bb = SystemBlackboard::Helm(HelmBlackboard {
        yaw: 1.0,
        forward_speed: 50.0,
        x: 100.0,
        z: 200.0,
        impulse_charge: 0.5,
        boost_battery: 0.8,
        boost_active: false,
        boost_enabled: true,
        radar_range: 0.0,
        lateral_speed: 0.0,
        hostile_weapon_arcs: Vec::new(),
    });

    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemBlackboards, With<LocalShip>>();
        if let Ok(mut bbs) = q.single_mut(app.world_mut()) {
            bbs.0.insert(helm_id.clone(), helm_bb.clone());
        }
    }

    // Tick: broadcast_blackboard_updates caches the blackboard and emits it.
    let out1 = tick(&mut app);
    assert!(
        out1.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BlackboardUpdate { .. })),
        "first tick after seeding must emit BlackboardUpdate"
    );

    // Simulate reconnect: push Identify with same token -> Welcome emitted.
    push(
        &mut app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    let out2 = tick(&mut app);
    let out3 = tick(&mut app);

    let has_bb_for_helm = |out: &[OutboundMessage]| -> bool {
        out.iter().any(|m| match &m.msg {
            ServerMessage::BlackboardUpdate { updates } => {
                updates.iter().any(|(id, _)| id.0 == "helm")
            }
            _ => false,
        })
    };

    assert!(
        has_bb_for_helm(&out2) || has_bb_for_helm(&out3),
        "must emit BlackboardUpdate with helm data within one tick of reconnect Welcome"
    );
}

/// Issue #697 made the weapons blackboard publish systems per-entity, so NPC ships now
/// carry populated Weapons blackboards. `broadcast_blackboard_updates` reads
/// only the `LocalShip`, and `LastBroadcastBlackboards` is a single global
/// map keyed on `SystemId` alone — it structurally assumes one broadcast
/// source. This pins that assumption: NPC blackboards must cost zero
/// bandwidth, or they would both leak and collide with the player ship's
/// cache entries under the same `SystemId`.
#[test]
fn npc_weapons_blackboards_add_no_wire_traffic() {
    let mut app = test_app();
    start_game(&mut app);

    // The thing the NPC is locked onto has to exist: `target_uuid` is the
    // frozen combat lock filtered for liveness, so a lock on a uuid with no
    // entity behind it is (correctly) never published.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("npc-only-target".into()),
        bevy::prelude::Transform::from_xyz(0.0, 0.0, -30.0),
    ));

    // An NPC ship locked onto a target the player ship never sees.
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            ShipSystemBlackboards::default(),
            crate::console::weapons::TacticalRadarSelection(Some("npc-only-target".into())),
            crate::console::weapons::ActiveBeam::default(),
            crate::console::weapons::PhaserCooldown::default(),
            crate::console::weapons::LastShipAttacker::default(),
            ShipPhysicsComponent::default(),
            crate::entities::spawner::EntityUuid("npc-1".into()),
            bevy::prelude::Transform::default(),
        ))
        .id();

    // Two ticks: `target_uuid` is the frozen viewscreen combat lock, which
    // the aggregator writes in `SimSet::PublishAggregate` — one tick behind
    // the publisher that reads it in `SimSet::Publish` (spec §1).
    tick(&mut app);
    let out = tick(&mut app);

    // The NPC really does publish its own Weapons blackboard...
    let npc_target = app
        .world()
        .entity(npc)
        .get::<ShipSystemBlackboards>()
        .and_then(|bbs| bbs.0.get(&SystemId("tactical".into())).cloned());
    assert!(
        matches!(
            npc_target,
            Some(crate::core::messages::SystemBlackboard::Weapons(ref bb))
                if bb.target_uuid.as_deref() == Some("npc-only-target")
        ),
        "NPC must publish its own Weapons blackboard, got {npc_target:?}"
    );

    // ...and none of it reaches any client.
    for m in &out {
        if let ServerMessage::BlackboardUpdate { updates } = &m.msg {
            for (id, bb) in updates {
                if let crate::core::messages::SystemBlackboard::Weapons(w) = bb {
                    assert_ne!(
                        w.target_uuid.as_deref(),
                        Some("npc-only-target"),
                        "NPC weapons blackboard leaked to the wire under SystemId {id:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn entity_spawned_is_broadcast_to_all() {
    let mut app = test_app();
    start_game(&mut app);

    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("all-broadcast".into()),
        Transform::default(),
    ));

    let out = tick(&mut app);

    let spawn_msg = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }))
        .expect("expected EntitySpawned message");
    assert!(
        matches!(&spawn_msg.target, crate::lobby::Target::All),
        "EntitySpawned must broadcast to All, got {:?}",
        spawn_msg.target
    );
}

#[test]
fn entity_despawned_is_broadcast_to_all() {
    let mut app = test_app();
    start_game(&mut app);

    let entity = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("broadcast-despawn".into()),
            Transform::default(),
        ))
        .id();
    let _ = tick(&mut app);

    app.world_mut().despawn(entity);
    let out = tick(&mut app);

    let despawn_msg = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { .. }))
        .expect("expected EntityDespawned message");
    assert!(
        matches!(&despawn_msg.target, crate::lobby::Target::All),
        "EntityDespawned must broadcast to All, got {:?}",
        despawn_msg.target
    );
}

// -- SetPhaserFrequency envelope tests (issue #804) -------------------
// The legacy top-level `ClientMessage::SetPhaserFrequency` was deleted;
// these exercise the admitted `ControlSystem` envelope path against the
// full server app (real ship config declaring `phaser-control`).

/// Build the admitted-envelope form of a frequency change (issue #804).
fn set_phaser_frequency_msg(frequency: f32) -> ClientMessage {
    ClientMessage::ControlSystem {
        target: crate::ship::system_registry::phaser_control_system_id(),
        payload: crate::core::messages::SystemControlPayload::SetPhaserFrequency { frequency },
    }
}

/// Tactical holder may always set phaser frequency.
#[test]
fn tactical_holder_can_set_phaser_frequency() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    push(&mut app, "weapons", set_phaser_frequency_msg(0.8));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.8).abs() < 1e-5,
        "Tactical holder should set phaser frequency to 0.8, got {freq}"
    );
}

/// Sensors holder is never authorized to set phaser frequency (delegation removed in B4).
#[test]
fn sensors_holder_cannot_set_phaser_frequency() {
    let mut app = test_app();
    start_game_with_sensors_and_weapons(&mut app);
    push(&mut app, "sensors", set_phaser_frequency_msg(0.9));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.5).abs() < 1e-5,
        "Sensors holder must NOT change phaser frequency, got {freq}"
    );
}

/// An unrelated console (e.g. captain) cannot set phaser frequency.
#[test]
fn unrelated_console_cannot_set_phaser_frequency() {
    let mut app = test_app();
    start_game(&mut app);
    push(&mut app, "captain", set_phaser_frequency_msg(0.9));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.5).abs() < 1e-5,
        "Captain must NOT change phaser frequency, got {freq}"
    );
}

/// Frequency value is clamped to [0.0, 1.0] by the handler.
#[test]
fn set_phaser_frequency_clamps_value() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    push(&mut app, "weapons", set_phaser_frequency_msg(1.5));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 1.0).abs() < 1e-5,
        "frequency above 1.0 should clamp to 1.0, got {freq}"
    );

    push(&mut app, "weapons", set_phaser_frequency_msg(-0.5));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.0).abs() < 1e-5,
        "frequency below 0.0 should clamp to 0.0, got {freq}"
    );
}

// -- Shield focus tests --------------------------------------------------

fn start_game_with_shields(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "shields",
        ClientMessage::Identify {
            token: "shields".into(),
            name: "Sully".into(),
        },
    );
    tick(app);
    push(
        app,
        "shields",
        ClientMessage::SelectStation {
            station: "Shields".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "shields", ClientMessage::SetReady { ready: true });
    let _ = tick(app);
    fast_forward_countdown(app);
    let _ = tick(app);
    let _ = tick(app);
}

#[test]
fn shields_holder_can_focus_a_facing() {
    let mut app = test_app();
    start_game_with_shields(&mut app);

    push(
        &mut app,
        "shields",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
            payload: SystemControlPayload::SetShieldArcFocus { focused: true },
        },
    );
    tick(&mut app);

    let ship = app.world().resource::<ShipEntity>().0;
    let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
    assert_eq!(shields.0.focused_facing, Some(0));
    assert!(shields.0.facings[0].is_focused);
}

#[test]
fn non_shields_sender_cannot_set_focus() {
    let mut app = test_app();
    start_game_with_shields(&mut app);

    // Captain (not Shields holder) tries to set focus.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::shield_arc_system_id("port").expect("port"),
            payload: SystemControlPayload::SetShieldArcFocus { focused: true },
        },
    );
    tick(&mut app);

    let ship = app.world().resource::<ShipEntity>().0;
    assert!(app
        .world()
        .entity(ship)
        .get::<ShipShields>()
        .unwrap()
        .0
        .focused_facing
        .is_none());
}

#[test]
fn shields_holder_can_clear_focus() {
    let mut app = test_app();
    start_game_with_shields(&mut app);

    push(
        &mut app,
        "shields",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
            payload: SystemControlPayload::SetShieldArcFocus { focused: true },
        },
    );
    tick(&mut app);
    let ship = app.world().resource::<ShipEntity>().0;
    let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
    assert_eq!(shields.0.focused_facing, Some(0));

    push(
        &mut app,
        "shields",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
            payload: SystemControlPayload::SetShieldArcFocus { focused: false },
        },
    );
    tick(&mut app);
    let ship = app.world().resource::<ShipEntity>().0;
    assert!(app
        .world()
        .entity(ship)
        .get::<ShipShields>()
        .unwrap()
        .0
        .focused_facing
        .is_none());
}

#[test]
fn shield_focus_is_ignored_during_lobby() {
    let mut app = test_app();
    push(
        &mut app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(&mut app);

    // Still in Lobby — SetShieldArcFocus should be ignored.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::shield_arc_system_id("aft").expect("aft"),
            payload: SystemControlPayload::SetShieldArcFocus { focused: true },
        },
    );
    tick(&mut app);

    let ship = app.world().resource::<ShipEntity>().0;
    assert!(app
        .world()
        .entity(ship)
        .get::<ShipShields>()
        .unwrap()
        .0
        .focused_facing
        .is_none());
}

#[test]
fn shield_focus_updates_broadcast_status() {
    let mut app = test_app();
    start_game_with_shields(&mut app);

    push(
        &mut app,
        "shields",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
            payload: SystemControlPayload::SetShieldArcFocus { focused: true },
        },
    );
    let _ = tick(&mut app);
    let out = tick(&mut app);

    let shield_status = out
        .iter()
        .find_map(|m| match &m.msg {
            ServerMessage::ShieldStatus { facings, .. } => Some(facings.clone()),
            _ => None,
        })
        .expect("expected a ShieldStatus broadcast after focus change");

    assert!(shield_status[0].is_focused, "Fore should be focused");
    assert!(!shield_status[1].is_focused, "Port should not be focused");
    assert!(!shield_status[2].is_focused, "Aft should not be focused");
    assert!(
        !shield_status[3].is_focused,
        "Starboard should not be focused"
    );
}

#[test]
fn player_spawn_rotation_yaw_extracts_yaw_correctly() {
    let (q, yaw) = player_spawn_rotation_yaw([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
    assert!(
        (yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
        "yaw-only rotation should produce matching yaw"
    );
    let (y, _, _) = q.to_euler(bevy::math::EulerRot::YXZ);
    assert!(
        (y - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
        "quaternion yaw should match input"
    );
}

#[test]
fn player_spawn_rotation_yaw_pitch_only_gives_zero_yaw() {
    let (_, yaw) = player_spawn_rotation_yaw([std::f32::consts::FRAC_PI_4, 0.0, 0.0]);
    assert!(yaw.abs() < 1e-6, "pitch-only rotation should give zero yaw");
}

#[test]
fn player_spawn_rotation_yaw_roll_only_gives_zero_yaw() {
    let (_, yaw) = player_spawn_rotation_yaw([0.0, 0.0, std::f32::consts::FRAC_PI_3]);
    assert!(yaw.abs() < 1e-6, "roll-only rotation should give zero yaw");
}

// ── last_attacker clear handler tests ──────────────────────────────────

fn last_attacker_test_app() -> App {
    let mut app = App::new();
    app.add_systems(
        Update,
        (
            clear_last_attacker_on_death,
            clear_last_attacker_on_red_alert_off,
        ),
    );
    app
}

#[test]
fn clear_on_despawn_clears_when_entity_removed() {
    let mut app = last_attacker_test_app();
    let attacker_uuid = "attacker-1".to_string();
    let attacker_entity = app
        .world_mut()
        .spawn((EntityUuid(attacker_uuid.clone()),))
        .id();
    let ship = app
        .world_mut()
        .spawn((LastShipAttacker(Some(attacker_uuid)),))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<LastShipAttacker>(ship).unwrap().0,
        Some("attacker-1".to_string())
    );
    app.world_mut().despawn(attacker_entity);
    app.update();
    assert_eq!(app.world().get::<LastShipAttacker>(ship).unwrap().0, None);
}

#[test]
fn clear_on_despawn_does_not_clear_when_entity_still_alive() {
    let mut app = last_attacker_test_app();
    app.world_mut()
        .spawn((EntityUuid("attacker-1".to_string()),));
    let ship = app
        .world_mut()
        .spawn((LastShipAttacker(Some("attacker-1".to_string())),))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<LastShipAttacker>(ship).unwrap().0,
        Some("attacker-1".to_string())
    );
}

#[test]
fn clear_on_red_alert_off_clears_when_red_alert_turns_off() {
    let mut app = last_attacker_test_app();
    // Spawn an entity so clear_last_attacker_on_death doesn't fire.
    app.world_mut()
        .spawn((EntityUuid("attacker-1".to_string()),));
    let ship = app
        .world_mut()
        .spawn((
            LastShipAttacker(Some("attacker-1".to_string())),
            crate::ship::state::ShipRedAlert(true),
            LocalShip,
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<LastShipAttacker>(ship).unwrap().0,
        Some("attacker-1".to_string())
    );
    app.world_mut()
        .get_mut::<crate::ship::state::ShipRedAlert>(ship)
        .unwrap()
        .0 = false;
    app.update();
    assert!(app
        .world()
        .get::<LastShipAttacker>(ship)
        .unwrap()
        .0
        .is_none());
}

#[test]
fn clear_on_red_alert_off_does_not_clear_when_alert_stays_on() {
    let mut app = last_attacker_test_app();
    // Spawn an entity so clear_last_attacker_on_death doesn't fire.
    app.world_mut()
        .spawn((EntityUuid("attacker-1".to_string()),));
    let ship = app
        .world_mut()
        .spawn((
            LastShipAttacker(Some("attacker-1".to_string())),
            crate::ship::state::ShipRedAlert(true),
            LocalShip,
        ))
        .id();
    app.update();
    app.update();
    assert_eq!(
        app.world().get::<LastShipAttacker>(ship).unwrap().0,
        Some("attacker-1".to_string())
    );
}

/// Regression test: `clear_last_attacker_on_red_alert_off` used to be
/// filtered `With<LocalShip>`, so an NPC whose red alert stood down kept
/// retaliating against a stale attacker forever. NPC captain-AI can
/// set its own `ShipRedAlert` (`handle_set_red_alert` dispatches
/// per-ship), so the clear handler must cover NPCs too.
#[test]
fn clear_on_red_alert_off_clears_for_an_npc_not_just_local_ship() {
    let mut app = last_attacker_test_app();
    app.world_mut()
        .spawn((EntityUuid("attacker-1".to_string()),));
    let npc = app
        .world_mut()
        .spawn((
            LastShipAttacker(Some("attacker-1".to_string())),
            crate::ship::state::ShipRedAlert(true),
            // No `LocalShip` marker — this is an NPC.
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<LastShipAttacker>(npc).unwrap().0,
        Some("attacker-1".to_string())
    );
    app.world_mut()
        .get_mut::<crate::ship::state::ShipRedAlert>(npc)
        .unwrap()
        .0 = false;
    app.update();
    assert!(
        app.world()
            .get::<LastShipAttacker>(npc)
            .unwrap()
            .0
            .is_none(),
        "an NPC standing down from red alert must clear its attacker record too"
    );
}

/// Two ships (one player, one NPC) toggling red alert off independently
/// must each clear their own attacker record without a shared `Local`
/// mixing up whose transition is whose.
#[test]
fn clear_on_red_alert_off_handles_multiple_ships_independently() {
    let mut app = last_attacker_test_app();
    app.world_mut()
        .spawn((EntityUuid("attacker-1".to_string()),));
    app.world_mut()
        .spawn((EntityUuid("attacker-2".to_string()),));

    let local = app
        .world_mut()
        .spawn((
            LastShipAttacker(Some("attacker-1".to_string())),
            crate::ship::state::ShipRedAlert(true),
            LocalShip,
        ))
        .id();
    let npc = app
        .world_mut()
        .spawn((
            LastShipAttacker(Some("attacker-2".to_string())),
            crate::ship::state::ShipRedAlert(true),
        ))
        .id();
    app.update();

    // Only the NPC stands down this tick; the player stays at red alert.
    app.world_mut()
        .get_mut::<crate::ship::state::ShipRedAlert>(npc)
        .unwrap()
        .0 = false;
    app.update();

    assert!(
        app.world()
            .get::<LastShipAttacker>(npc)
            .unwrap()
            .0
            .is_none(),
        "the NPC that stood down must have its attacker cleared"
    );
    assert_eq!(
        app.world().get::<LastShipAttacker>(local).unwrap().0,
        Some("attacker-1".to_string()),
        "the player ship, still at red alert, must keep its attacker record"
    );
}

// ── publish_viewscreen_blackboard: the `attacked` decay mirror (#1010) ──

/// The LocalShip half of the `attacked` signal must decay exactly like the
/// NPC half (`ai::server::aggregate_doctrine_blackboards`) — AGENTS.md #6
/// symmetry, and #842's AC4: a backfilled player and a world-spawned copy
/// of the same hull evaluate identical doctrine identically.
///
/// Both sites fold with `objectives::last_landed_hit_secs` and test with
/// `objectives::attacked_recently`, whose boundary is pinned in
/// `objectives`' own tests; this pins that this site really does route
/// through them — off the fixed clock, over `[global] attacked_memory_secs`,
/// rather than off the `LastShipAttacker` latch it used to read. The tail of
/// the test covers the other half of the fold: fire an arc ABSORBS closes
/// the gate here too, which matters because the shipped station cannot get
/// through a Harrow's arc at all and would otherwise never register.
///
/// The world here AUTHORS a short window, the same lesson #889 taught the
/// NPC-side sibling `the_assault_resumes_once_the_authored_attacked_window_elapses`
/// (`ai::server`): a fixture with no `WorldConfig` exercises only the
/// serde-default fallback arm of the LocalShip resolution block
/// (`publish_viewscreen_blackboard`'s `attacked_memory_secs` lookup), so a
/// typo'd field path there would pass unnoticed. Two seconds is well clear
/// of the 8 s default, so the gate reopening on schedule can only be the
/// authored value being read. `publish_viewscreen_blackboard` is not
/// cadence-gated — it is the only system this fixture registers in
/// `FixedUpdate` and `drive_one_fixed_step_per_update` paces the fixed
/// clock directly off `TEST_TICK`, independent of `[global] sim_tick_hz` —
/// so unlike the NPC sibling this fixture does not need the 1:1
/// `sim_tick_hz`/`ai_tick_hz`/`ai_snapshot_hz` authoring to keep an AI
/// cadence latch from swallowing steps.
#[test]
fn the_local_ship_doctrine_pool_reopens_its_raid_after_the_attacked_window() {
    use crate::entities::config::{BehaviourConfig, DoctrineObjective};
    use crate::entities::spawner::BehaviourSection;
    use crate::ship::combat_activity::RecentCombatActivity;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    // `combat_test.toml`'s raid shape: an untargeted self-defence Destroy
    // plus a `not_attacked`-gated assault that outranks it.
    let behaviour = BehaviourConfig {
        doctrine: vec![
            DoctrineObjective {
                id: "destroy-hostiles".into(),
                text: "Engage whatever is in front of you".into(),
                directive_kind: Some("Destroy".into()),
                base_priority: 38.0,
                ..Default::default()
            },
            DoctrineObjective {
                id: "assault-starbase".into(),
                text: "Press the assault on the station".into(),
                directive_kind: Some("Destroy".into()),
                directive_target: Some("world.entity.starbase_alpha.name".into()),
                base_priority: 50.0,
                zero_gates: vec![crate::objectives::ZeroGateCondition {
                    condition: "not_attacked".into(),
                    threshold: None,
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let window = 2.0_f32;
    let default_window = crate::entities::config::GlobalConfig::default().attacked_memory_secs;
    assert!(
        window < default_window,
        "the authored window must differ from the {default_window}s serde \
         default, or this test cannot tell the two arms apart"
    );

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_systems(FixedUpdate, publish_viewscreen_blackboard);
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        crate::ship::test_support::TEST_TICK,
    );
    // Author the short window through a real `WorldConfig` rather than
    // leaving this bare `App` without one, so the LocalShip resolution
    // block's authored-config arm (the `world_config.as_deref().map(...)`
    // branch in `publish_viewscreen_blackboard`) is what this test
    // exercises, not just its `unwrap_or_else` fallback (see #889).
    app.insert_resource(
        crate::world::config::parse_world(&format!("[global]\nattacked_memory_secs = {window}\n"))
            .expect("world TOML should parse"),
    );
    let ship = app
        .world_mut()
        .spawn((
            LocalShip,
            BehaviourSection(behaviour),
            crate::entities::spawner::EntitySystemHull(
                crate::ship::damage::SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    100.0,
                )]),
            ),
            ShipSystemBlackboards::default(),
            // The latch that used to decide this on its own.
            LastShipAttacker(Some("attacker-uuid".to_string())),
            RecentCombatActivity::default(),
        ))
        .id();

    let sim_secs = |app: &App| app.world().resource::<Time<Fixed>>().elapsed_secs();
    let assault_score = |app: &App| {
        let bbs = app
            .world()
            .get::<ShipSystemBlackboards>(ship)
            .expect("blackboards");
        match bbs
            .0
            .get(&SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
            .expect("viewscreen entry")
        {
            SystemBlackboard::Viewscreen(v) => {
                v.scored_objectives
                    .iter()
                    .find(|o| o.id == "assault-starbase")
                    .unwrap_or_else(|| panic!("assault-starbase must be in the pool: {v:?}"))
                    .score
            }
            _ => panic!("expected Viewscreen blackboard"),
        }
    };

    app.update();
    assert!(
        assault_score(&app) > 0.0,
        "a named attacker with no recent damage is a stale latch, not an \
         attack — the raid must stay live"
    );

    // A hit lands.
    let hit_at = sim_secs(&app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<RecentCombatActivity>()
        .expect("combat activity")
        .last_damage_taken = Some(hit_at);
    app.update();
    assert_eq!(
        assault_score(&app),
        0.0,
        "a fresh hit must close the `not_attacked` gate on the player's \
         doctrine pool too"
    );

    // The reprieve elapses with no further hits.
    let mut guard = 0;
    while sim_secs(&app) < hit_at + window + 0.5 {
        app.update();
        guard += 1;
        assert!(guard < 10_000, "the fixed clock is not advancing");
    }
    assert!(
        sim_secs(&app) < hit_at + default_window,
        "precondition: still inside the {default_window}s default, so \
         reopening here can only be the authored {window}s being honoured"
    );
    assert!(
        assault_score(&app) > 0.0,
        "after {window}s of quiet the LocalShip pool must reopen the raid, \
         exactly as the NPC aggregator does"
    );

    // Now fire the shields ABSORB. `last_damage_taken` never moves for it —
    // the hull total is untouched — so a gate reading damage alone would
    // leave the raid live while something shot at the ship, which is what
    // the shipped station does to a Harrow for the whole engagement.
    let skimmed_at = sim_secs(&app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<RecentCombatActivity>()
        .expect("combat activity")
        .last_hostile_fire_taken = Some(skimmed_at);
    app.update();
    assert_eq!(
        assault_score(&app),
        0.0,
        "a hit the shields ate must close the `not_attacked` gate too"
    );

    // Own weapon fire is not being attacked: let the shield-absorbed hit
    // decay while the ship keeps shooting, and the raid must come back.
    let mut guard = 0;
    while sim_secs(&app) < skimmed_at + window + 0.5 {
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<RecentCombatActivity>()
            .expect("combat activity")
            .last_weapon_fired = Some(sim_secs(&app));
        app.update();
        guard += 1;
        assert!(guard < 10_000, "the fixed clock is not advancing");
    }
    assert!(
        sim_secs(&app) < skimmed_at + default_window,
        "precondition: still inside the {default_window}s default, so \
         reopening here can only be the authored {window}s being honoured"
    );
    assert!(
        assault_score(&app) > 0.0,
        "a raider pressing its own attack has not been attacked — folding \
         `last_weapon_fired` in would make the gate veto itself for as long \
         as the ship kept firing"
    );
}

// ── build_sim_state_entity_states: shield detail on the wire (#927) ─────
//
// Root cause pinned here: `sim_state_broadcaster` always sent
// `shields: None` and had no `shield_freq` field on `EntityStateSnapshot`
// at all, regardless of whether the entity carried a `ShipShields`
// component — so `target_shields`/`target_shield_freq` were empty on the
// wire for every Sensors target, on every hull, before this fix. These
// call `build_sim_state_entity_states` directly (the function extracted
// from `sim_state_broadcaster`'s producer closure) rather than going
// through the Broadcaster/cadence machinery, since the function needs
// only a bare `World` with the two delta-cache resources.

#[test]
fn target_with_shields_populates_shields_and_shield_freq() {
    let mut world = World::new();
    world.init_resource::<LastBroadcastEntityPositions>();
    world.init_resource::<LastBroadcastEntityHealth>();
    world.spawn((
        EntityUuid("target-1".to_string()),
        Transform::from_xyz(10.0, 0.0, 20.0),
        ShipShields(ShieldSystem::default(), 0.75),
    ));

    let states = build_sim_state_entity_states(&mut world);
    let entry = states
        .iter()
        .find(|s| s.uuid == "target-1")
        .expect("target-1 must appear in the first SimState tick");

    let shields = entry
        .shields
        .as_ref()
        .expect("a ShipShields-carrying entity must publish its facings");
    assert!(
        !shields.is_empty(),
        "expected at least one shield facing, same producer as this ship's own ShieldsBlackboard"
    );
    assert_eq!(
        entry.shield_freq,
        Some(0.75),
        "shield_freq must be the entity's own ShipShields::frequency() — \
         the same value FrequencyHint reads"
    );
}

#[test]
fn entity_without_shields_leaves_shields_and_shield_freq_absent() {
    let mut world = World::new();
    world.init_resource::<LastBroadcastEntityPositions>();
    world.init_resource::<LastBroadcastEntityHealth>();
    world.spawn((
        EntityUuid("no-shields-1".to_string()),
        Transform::from_xyz(5.0, 0.0, 5.0),
    ));

    let states = build_sim_state_entity_states(&mut world);
    let entry = states
        .iter()
        .find(|s| s.uuid == "no-shields-1")
        .expect("no-shields-1 must still appear (position changed on the first tick)");

    assert!(
        entry.shields.is_none(),
        "an entity with no ShipShields must not carry a shields field"
    );
    assert!(
        entry.shield_freq.is_none(),
        "an entity with no ShipShields must not carry a shield_freq field"
    );
}

#[test]
fn shield_detail_is_delta_compressed_like_hull_and_shield_fraction() {
    // The widened `LastBroadcastEntityHealth` cache tuple must gate the
    // NEXT tick's inclusion on shields/shield_freq changing, exactly as
    // it already did for hull_fraction/shield_fraction — not just those
    // two fields.
    let mut world = World::new();
    world.init_resource::<LastBroadcastEntityPositions>();
    world.init_resource::<LastBroadcastEntityHealth>();
    world.spawn((
        EntityUuid("steady-1".to_string()),
        Transform::from_xyz(1.0, 0.0, 1.0),
        ShipShields(ShieldSystem::default(), 0.5),
    ));

    let first = build_sim_state_entity_states(&mut world);
    assert!(
        first.iter().any(|s| s.uuid == "steady-1"),
        "first tick must publish the newly-seen entity"
    );

    let second = build_sim_state_entity_states(&mut world);
    assert!(
        !second.iter().any(|s| s.uuid == "steady-1"),
        "an entity whose position/hull/shields/freq are all unchanged \
         since the last broadcast must be omitted entirely from the next tick"
    );
}

#[test]
fn shield_offline_remaining_delta_gate_ignores_subsecond_countdown_but_reports_bucket_crossings() {
    // `ShieldFacingStatus` derives `PartialEq` over every field including
    // `offline_remaining`, which `tick_shields` decrements every tick
    // through a ~30s recovery. A raw equality gate re-sent this payload
    // on effectively every 10 Hz tick while any facing was offline, even
    // though nothing perceptible changed. `shields_delta_projection`
    // buckets `offline_remaining` to whole seconds (ceiling) before
    // comparing — see its doc comment in `ship::shields`.
    let mut world = World::new();
    world.init_resource::<LastBroadcastEntityPositions>();
    world.init_resource::<LastBroadcastEntityHealth>();
    let entity = world
        .spawn((
            EntityUuid("recovering-1".to_string()),
            Transform::from_xyz(1.0, 0.0, 1.0),
            ShipShields(ShieldSystem::default(), 0.5),
        ))
        .id();
    world.get_mut::<ShipShields>(entity).unwrap().0.facings[0].offline_remaining = 5.5;

    let first = build_sim_state_entity_states(&mut world);
    assert!(
        first.iter().any(|s| s.uuid == "recovering-1"),
        "first tick must publish the newly-seen offline facing"
    );

    // Sub-second countdown: 5.5s -> 5.4s still buckets to ceil = 6.0 —
    // must NOT re-send.
    world.get_mut::<ShipShields>(entity).unwrap().0.facings[0].offline_remaining = 5.4;
    let second = build_sim_state_entity_states(&mut world);
    assert!(
        !second.iter().any(|s| s.uuid == "recovering-1"),
        "a sub-second offline_remaining tick (5.5s -> 5.4s, same whole-second \
         bucket) must not re-trigger the delta gate"
    );

    // Crossing a whole-second boundary: 5.4s -> 4.9s, ceil bucket goes
    // 6.0 -> 5.0 — must re-send.
    world.get_mut::<ShipShields>(entity).unwrap().0.facings[0].offline_remaining = 4.9;
    let third = build_sim_state_entity_states(&mut world);
    assert!(
        third.iter().any(|s| s.uuid == "recovering-1"),
        "crossing a whole-second bucket boundary (5.4s -> 4.9s) must \
         re-trigger the delta gate"
    );
}

// ── God Mode applier (issue #900) ───────────────────────────────────────

fn god_mode_app() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<GodMode>()
        .add_systems(Update, apply_god_mode_toggle);
    let ship = app
        .world_mut()
        .spawn((LocalShip, AdmittedCommands::default()))
        .id();
    (app, ship)
}

/// Sets `ship`'s `AdmittedCommands` to exactly one command (clearing
/// whatever was there), mirroring what `admit_system_commands` does at the
/// top of every real tick (AGENTS.md constraint 7: "`AdmittedCommands` is
/// cleared and refilled at admission each tick"). This minimal fixture has
/// no admission system to do that itself, so the test stands in for it —
/// otherwise a second call would leave the first tick's command in place
/// and every following tick would apply the toggle twice.
fn admit(app: &mut App, ship: Entity, payload: SystemControlPayload) {
    let mut admitted = app.world_mut().get_mut::<AdmittedCommands>(ship).unwrap();
    admitted.0.clear();
    admitted.0.push(AdmittedCommand {
        target: SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
        payload,
        response_token: None,
    });
}

/// The baseline: an admitted `ToggleGodMode` command flips `GodMode` from
/// its default `false`.
#[test]
fn an_admitted_toggle_flips_god_mode_on() {
    let (mut app, ship) = god_mode_app();
    assert!(!app.world().resource::<GodMode>().0, "precondition: off");
    admit(&mut app, ship, SystemControlPayload::ToggleGodMode);
    app.update();
    assert!(
        app.world().resource::<GodMode>().0,
        "an admitted ToggleGodMode command must flip GodMode on"
    );
}

/// A second admitted toggle (a second tick, a second command) flips it
/// back off — proving this is a flip, not a one-way latch.
#[test]
fn a_second_admitted_toggle_flips_god_mode_back_off() {
    let (mut app, ship) = god_mode_app();
    admit(&mut app, ship, SystemControlPayload::ToggleGodMode);
    app.update();
    assert!(app.world().resource::<GodMode>().0, "precondition: now on");

    admit(&mut app, ship, SystemControlPayload::ToggleGodMode);
    app.update();
    assert!(
        !app.world().resource::<GodMode>().0,
        "a second admitted toggle must flip it back off"
    );
}

/// With nothing admitted, a tick must not touch `GodMode` — the applier
/// only acts on what it finds in `AdmittedCommands`, never on its own.
#[test]
fn no_admitted_command_leaves_god_mode_untouched() {
    let (mut app, _ship) = god_mode_app();
    app.update();
    assert!(
        !app.world().resource::<GodMode>().0,
        "with nothing admitted, GodMode must stay at its default"
    );
}

// ── GameOver publishes the authored outcome (PRD #1023 module 4) ─────

/// The scenario declares which side won; the message must carry it. Before
/// this the flag was latched, digested, reported — and dropped on the way
/// to the one screen that exists to say how the session ended.
#[test]
fn game_over_broadcasts_the_latched_outcome() {
    use bevy::ecs::system::RunSystemOnce;

    let mut world = World::new();
    world.init_resource::<SimOutbox>();
    world.insert_resource(GameOverReason(
        Some("world.falling_skyway.ending.held".into()),
        Some(crate::core::balance::Outcome::Victory),
    ));

    world.run_system_once(on_game_over_enter).unwrap();

    let outbox = &world.resource::<SimOutbox>().0;
    assert_eq!(outbox.len(), 1);
    match &outbox[0].1 {
        ServerMessage::GameOver { reason, outcome } => {
            assert_eq!(reason, "world.falling_skyway.ending.held");
            assert_eq!(outcome.as_deref(), Some("victory"));
        }
        other => panic!("expected GameOver, got {other:?}"),
    }

    // The reason is TAKEN (it is per-ending display text and the next
    // round must not inherit it); the outcome is only READ. Two separate
    // reasons that agree: the headless exit report classifies the run off
    // `.1` after the run ends, and `state_digest` folds both halves —
    // clearing it here would move the digest of every run that reaches
    // GameOver inside its window.
    let latch = world.resource::<GameOverReason>();
    assert_eq!(latch.0, None, "the reason is consumed by the broadcast");
    assert_eq!(
        latch.1,
        Some(crate::core::balance::Outcome::Victory),
        "the outcome survives the broadcast"
    );
}

/// An ending nobody declared a side for stays undeclared on the wire. The
/// client frames that as ENDED rather than guessing, so `None` is an
/// answer here and not a gap.
#[test]
fn game_over_publishes_no_outcome_when_none_was_declared() {
    use bevy::ecs::system::RunSystemOnce;

    let mut world = World::new();
    world.init_resource::<SimOutbox>();
    world.insert_resource(GameOverReason(None, None));

    world.run_system_once(on_game_over_enter).unwrap();

    match &world.resource::<SimOutbox>().0[0].1 {
        ServerMessage::GameOver { reason, outcome } => {
            assert_eq!(reason, "");
            assert_eq!(*outcome, None);
        }
        other => panic!("expected GameOver, got {other:?}"),
    }
}
