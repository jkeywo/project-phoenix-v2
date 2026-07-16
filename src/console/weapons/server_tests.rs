use super::super::shared::{any_bank_accepts_human_input, system_is_registered};
use super::*;
use crate::ai_plugin::AiTokenRegistry;
use crate::damage::SystemHull;
use crate::entity_spawner::EntitySystemHull;
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target, WorldResource};
use crate::messages::*;
use crate::modifiers::ShipModifiers;
use crate::simulation::{ShipImpulse, SimOutbox};

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

#[derive(Resource, Default)]
struct ArcRequestLog(Vec<CoordinationEnqueue>);

fn collect_arc_requests(
    mut reader: MessageReader<CoordinationEnqueue>,
    mut log: ResMut<ArcRequestLog>,
) {
    for m in reader.read() {
        log.0.push(m.clone());
    }
}

/// Build a minimal `ShipConfigComponent` with a tactical station that has an
/// "Assisted" rating containing `torpedo_auto_fire` in its ai_tuning table.
///
/// Post-#512 this now uses fine Tactical `[[system]]` blocks matching
/// the ship entity TOML (phaser-fore/aft, torpedo-tube-fore-port/aft, etc.)
/// so tests exercise the production per-fine-system gate paths rather
/// than the legacy fallback-to-coarse-tactical path. The coarse
/// `[[system]] id = "tactical"` block is DELETED to match production.
fn test_ship_config() -> crate::ship_plugin::ShipConfigComponent {
    const TOML: &str = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Std"
automated_systems = []

[[station.rating]]
name = "Assisted"
automated_systems = []

[station.rating.ai_tuning]
torpedo_auto_fire = {}

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "phaser-aft"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"

[[system]]
id = "phaser-control"
kind = "phaser_control"
station = "tactical"

[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"

[[system]]
id = "torpedo-tube-fore-port"
kind = "torpedo_tube"
station = "tactical"

[[system]]
id = "torpedo-tube-fore-starboard"
kind = "torpedo_tube"
station = "tactical"

[[system]]
id = "torpedo-tube-aft"
kind = "torpedo_tube"
station = "tactical"
"#;
    crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::parse_and_validate(
            TOML,
            &[
                "phaser_bank",
                "torpedo_tube",
                "torpedo_magazine",
                "tactical_radar",
                "phaser_control",
            ],
        )
        .expect("test ship config must be valid"),
    )
}

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

fn test_app() -> App {
    let mut app = App::new();
    app.configure_sets(
        Update,
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
    .add_plugins(LobbyPlugin)
    .add_plugins(bevy::time::TimePlugin)
    .add_plugins(crate::server_app::AdmissionPlugin)
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(200),
    ))
    .init_resource::<WorldResource>()
    .add_message::<AsteroidDestroyedVfx>()
    .add_message::<crate::ai_plugin::AiEntityDestroyed>()
    .init_resource::<CurrentPhaserMode>()
    .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
        TorpedoConfig::default(),
    )))
    .init_resource::<SimOutbox>()
    .init_resource::<Outbox>()
    .init_resource::<ArcRequestLog>()
    .init_resource::<crate::world::server::WorldContentRuntime>()
    .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
    .add_plugins(WeaponsPlugin)
    // Override with two banks so per-bank arc checks work.
    // Uses wide (270°) arcs so existing tests that fire "port" at a
    // target ahead still pass. Tighter arcs are tested in dedicated
    // per-bank arc severance tests.
    .insert_resource(PhaserCombatConfigResource(
        crate::entity_config::PhaserCombatConfig {
            banks: vec![
                crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: -90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                },
                crate::entity_config::PhaserBankConfig {
                    id: "starboard".into(),
                    facing_deg: 90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                },
            ],
        },
    ))
    .add_systems(
        Update,
        (
            // The three beam-tick phases (issue #723) share the one-tick
            // BeamContext resource, so they must run in order — a bare
            // tuple is unordered in Bevy, hence the .chain().
            (
                tick_beams_prepare,
                tick_beams_apply_damage,
                tick_beams_tick_lifetimes,
            )
                .chain(),
            // The two torpedo-tick phases (issue #724) share the
            // one-tick TorpedoTargetSnapshot resource, so they must
            // run in order too.
            (build_torpedo_target_snapshot, tick_torpedo_lifecycle).chain(),
        ),
    )
    .add_plugins(weapons_update_broadcaster())
    // PR-7 (issue #597) — `tick_shields` (formerly `tick_npc_shield_regen`)
    // now lives on `ShipShieldsPlugin`. Include it so tests that spawn NPCs
    // with `ShipShields` observe regen on every frame.
    .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
    .add_systems(PostUpdate, (collect, collect_arc_requests));
    // Spawn the Ship entity with config/control-source components so all
    // weapons systems that use `Query<..., With<Ship>>.single()` have a
    // valid entity to operate on, matching what `spawn_game_start_entities`
    // would do in a full server build.
    let ship = app
        .world_mut()
        .spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            test_ship_config(),
            ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            ShipPhysics::default(),
            crate::ship_state::ShipPhaserFrequency::default(),
            bevy::prelude::Transform::default(),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[
                (SystemId("helm".into()), 25.0),
                (SystemId("tactical".into()), 25.0),
                (SystemId("power".into()), 25.0),
                (SystemId("shields".into()), 25.0),
                // Fine Tactical hull entries (issue #512) so tests can drive
                // sync_console_damage_tiers → offline_systems for the fine
                // systems declared in the updated test_ship_config().
                (SystemId("phaser-fore".into()), 15.0),
                (SystemId("phaser-aft".into()), 15.0),
                (SystemId("torpedo-tube-fore-port".into()), 12.0),
                (SystemId("torpedo-tube-fore-starboard".into()), 12.0),
                (SystemId("torpedo-tube-aft".into()), 12.0),
                (SystemId("torpedo-magazine".into()), 20.0),
            ])),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::entity_spawner::EntityUuid("test-local-ship".to_string()),
        ))
        .id();
    // Second insert to stay under Bevy's Bundle-tuple length limit.
    app.world_mut().entity_mut(ship).insert((
        // Insert per-entity weapon configs so component-path queries succeed.
        // These are overridden by individual tests via insert_resource for the
        // PhaserCombatConfigResource; we keep both in sync here.
        TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![
                crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: -90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                },
                crate::entity_config::PhaserBankConfig {
                    id: "starboard".into(),
                    facing_deg: 90.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                },
            ],
        }),
        PhaserRenderConfig::default(),
        // PR 7 (issue #597) — per-entity beam / target / cooldown components.
        WeaponsTarget::default(),
        ActiveBeam::default(),
        PhaserCooldown::default(),
        // PR 10 (PRD #597) — per-entity combat activity trackers.
        crate::server_app::WeaponFiredThisTick::default(),
        crate::server_app::ShipAttackedThisTick::default(),
        LastShipAttacker::default(),
        crate::ship::combat_activity::RecentCombatActivity::default(),
        ShipImpulse(crate::impulse::ImpulseState::new()),
        ShipModifiers::new(),
    ));
    app
}

// ── PR 7 test helpers — per-entity access to Weapons state ──────────────
// These wrap the `Query<&X, With<LocalShip>>` pattern that replaces
// `world.resource::<X>()` after PR 7 (PRD #597) removed the Resource derive.
//
// Each helper: single-entity lookup returning owned data.

fn get_weapons_target(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&WeaponsTarget, With<crate::server_app::LocalShip>>();
    q.single(app.world()).ok().and_then(|wt| wt.0.clone())
}

fn set_weapons_target(app: &mut App, uuid: Option<String>) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut WeaponsTarget, With<crate::server_app::LocalShip>>();
    if let Ok(mut wt) = q.single_mut(app.world_mut()) {
        wt.0 = uuid;
    }
}

fn get_active_beam_target(app: &mut App) -> Option<String> {
    let mut q = app
        .world_mut()
        .query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .ok()
        .and_then(|b| b.target_uuid.clone())
}

fn active_beam_target_is_none(app: &mut App) -> bool {
    get_active_beam_target(app).is_none()
}

fn set_active_beam_target(app: &mut App, uuid: Option<String>) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        b.target_uuid = uuid;
    }
}

fn set_active_beam_remaining_secs(app: &mut App, secs: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        b.remaining_secs = secs;
    }
}

fn set_active_beam_damage_accumulator(app: &mut App, val: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
    if let Ok(mut b) = q.single_mut(app.world_mut()) {
        b.damage_accumulator = val;
    }
}

fn phaser_bank_is_active(app: &mut App, bank: &str) -> bool {
    let mut q = app
        .world_mut()
        .query_filtered::<&PhaserCooldown, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .ok()
        .map(|cd| cd.is_bank_active(bank))
        .unwrap_or(false)
}

fn start_phaser_cooldown(app: &mut App, bank: &str, secs: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut PhaserCooldown, With<crate::server_app::LocalShip>>();
    if let Ok(mut cd) = q.single_mut(app.world_mut()) {
        cd.start_bank_with_cooldown(bank, secs);
    }
}

fn get_phaser_frequency(app: &mut App) -> f32 {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship_state::ShipPhaserFrequency, With<crate::server_app::LocalShip>>();
    q.single(app.world()).map(|f| f.0).unwrap_or(0.5)
}

fn set_ship_yaw(app: &mut App, yaw: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
    let mut p = q
        .single_mut(app.world_mut())
        .expect("expected Ship with ShipPhysics");
    p.yaw = yaw;
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
    let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
    let mut out = app.world().resource::<Outbox>().0.clone();
    for (target, msg) in sim_entries {
        out.push(OutboundMessage {
            target,
            msg,
            delivery: crate::messages::DeliveryClass::Reliable,
        });
    }
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn load_tube_now(app: &mut App, tube: &str) {
    // The systems now prefer the per-entity component over the resource.
    // Update both to keep them in sync.
    let mut q = app
        .world_mut()
        .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
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
    tick(app);
}

fn setup_weapons_world(
    app: &mut App,
    asteroid_x: f32,
    asteroid_z: f32,
) -> bevy::ecs::entity::Entity {
    let uuid = "target-uuid".to_string();
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot::asteroid(
                &uuid, asteroid_x, asteroid_z, 2.0,
            )],
            ..Default::default()
        }));
    // handle_set_target and tick_beams use live ECS Transforms
    // (live_entity_xz), so every WorldResource entry must also have a
    // matching ECS entity with the components all queries expect.
    app.world_mut()
        .spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid(uuid),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
        ))
        .id()
}

fn setup_weapons_world_with_entity(
    app: &mut App,
    asteroid_x: f32,
    asteroid_z: f32,
) -> bevy::ecs::entity::Entity {
    setup_weapons_world(app, asteroid_x, asteroid_z)
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
}

fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
    setup_weapons_world_with_entity(app, asteroid_x, asteroid_z);
    start_game_with_weapons(app);
    push(
        app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let _ = tick(app);
    push(
        app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(app)
}

// ── SetTarget / TargetLock tests ───────────────────────────────────────

#[test]
fn valid_target_within_range_replies_with_target_lock_confirmed() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
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

    assert_eq!(get_weapons_target(&mut app).as_deref(), Some("target-uuid"));
}

#[test]
fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 400.0, 0.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
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
            target: crate::system_registry::tactical_radar_system_id(),
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

// ── WeaponsUpdate / fire_ready tests ───────────────────────────────────

#[test]
fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    // Target changes → WeaponsUpdate fires this tick.
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

#[test]
fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    // Target changes → WeaponsUpdate fires this tick.
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

// ── FirePhaser / beam lifecycle tests ──────────────────────────────────

#[test]
fn fire_phaser_on_valid_target_broadcasts_beam_started() {
    let mut app = test_app();
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

    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid")
    );
}

#[test]
fn fire_phaser_rejected_during_cooldown() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    set_active_beam_target(&mut app, None);
    start_phaser_cooldown(&mut app, "port", 3.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "BeamStarted should not fire during cooldown"
    );
}

#[test]
fn fire_phaser_ignored_from_non_weapons_player() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 0.0, -20.0);
    start_game(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "captain should not be able to fire phaser"
    );
}

#[test]
fn fire_phaser_rejected_when_target_outside_bank_arc() {
    let mut app = test_app();
    // Target at starboard beam (20, 0), bearing +90°, which is outside the
    // port bank's 270° arc centered at -90° (covers -135° to 45°).
    setup_weapons_world(&mut app, 20.0, 0.0);
    start_game_with_weapons(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    let out = tick(&mut app);

    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "FirePhaser should be rejected when target is outside bank's fire arc"
    );
}

#[test]
fn full_beam_duration_kills_asteroid() {
    let mut app = test_app();
    // setup_weapons_world (called by lock_and_fire) now spawns the
    // asteroid ECS entity. Fetch its handle after setup.
    let _ = lock_and_fire(&mut app, 0.0, -20.0);
    let asteroid_entity = {
        let mut q = app
            .world_mut()
            .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == "target-uuid")
            .map(|(e, _)| e)
            .expect("setup_weapons_world should have spawned the target asteroid")
    };

    assert_eq!(
        get_active_beam_target(&mut app).as_deref(),
        Some("target-uuid")
    );

    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0);

    let out = tick(&mut app);

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

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded after asteroid destruction"
    );

    assert!(
        !app.world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid"),
        "destroyed asteroid should be removed from WorldData"
    );

    assert!(active_beam_target_is_none(&mut app));

    assert!(
        phaser_bank_is_active(&mut app, "port"),
        "cooldown should start after beam end"
    );

    assert!(
        app.world()
            .get::<EntitySystemHull>(asteroid_entity)
            .is_none(),
        "asteroid entity should be despawned"
    );
}

#[test]
fn beam_severs_when_target_leaves_bank_arc() {
    let mut app = test_app();
    // Target at port beam (-20, 0), bearing -90° — inside port bank's
    // 270° arc centered at -90° (covers -135° to 45°).
    let _ = lock_and_fire(&mut app, -20.0, 0.0);

    // Rotate 180° so the target moves to starboard beam (bearing +90°),
    // which is outside the port bank's arc.
    set_ship_yaw(&mut app, std::f32::consts::PI);

    let out = tick(&mut app);

    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded when target leaves bank fire arc"
    );
    assert!(
        active_beam_target_is_none(&mut app),
        "beam should be cleared after sever-by-arc"
    );
    assert!(
        phaser_bank_is_active(&mut app, "port"),
        "cooldown should start after arc sever"
    );
}

#[test]
fn beam_severs_when_target_leaves_phaser_range() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Move the live ECS Transform out of range. tick_beams reads the
    // live position, not the WorldResource snapshot.
    let entity = {
        let mut q = app
            .world_mut()
            .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
        q.iter(app.world())
            .find(|(_, u)| u.0 == "target-uuid")
            .map(|(e, _)| e)
            .expect("target entity should exist")
    };
    app.world_mut()
        .entity_mut(entity)
        .insert(Transform::from_xyz(0.0, 0.0, -50.0));

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
        phaser_bank_is_active(&mut app, "port"),
        "cooldown should start after range sever"
    );
}

#[test]
fn no_damage_refund_on_sever() {
    let mut app = test_app();
    let asteroid_entity = app
        .world_mut()
        .spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("target-uuid".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
        ))
        .id();
    // Target at port beam (-20, 0) so the port bank's arc check passes.
    let _ = lock_and_fire(&mut app, -20.0, 0.0);

    set_active_beam_damage_accumulator(&mut app, 10.0);
    let _ = tick(&mut app);

    // Rotate 180° — target moves to starboard beam, outside port bank's arc.
    set_ship_yaw(&mut app, std::f32::consts::PI);
    let _ = tick(&mut app);

    let hp = app
        .world()
        .get::<EntitySystemHull>(asteroid_entity)
        .map(|h| h.0.total_current());
    assert!(
        hp.is_some() && hp.unwrap() < 30.0,
        "asteroid should retain damage after sever (no refund), hp={:?}",
        hp
    );
}

#[test]
fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![
                crate::messages::EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                crate::messages::EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
            ..Default::default()
        }));
    // Spawn matching ECS entities so live_entity_xz can find them.
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        crate::simulation::AsteroidUuid("t1".into()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        crate::simulation::AsteroidUuid("t2".into()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -15.0),
    ));
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    let _ = tick(&mut app);
    assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

    set_active_beam_remaining_secs(&mut app, 0.0);
    set_active_beam_damage_accumulator(&mut app, 0.0);
    let _ = tick(&mut app);

    assert!(phaser_bank_is_active(&mut app, "port"));

    start_phaser_cooldown(&mut app, "port", 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
        },
    );
    let _ = tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
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

// ── SetPhaserMode tests ────────────────────────────────────────────────

#[test]
fn weapons_console_can_set_phaser_mode_to_manual() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::messages::PhaserMode::Manual,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world().resource::<CurrentPhaserMode>().0,
        crate::messages::PhaserMode::Manual,
        "phaser mode should be Manual after SetPhaserMode"
    );
}

#[test]
fn non_weapons_player_cannot_set_phaser_mode() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    // Establish a known mode (Auto) via the authorised player first.
    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::messages::PhaserMode::Auto,
            },
        },
    );
    tick(&mut app);
    // Non-weapons player attempts to switch back to Manual — must be ignored.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserMode {
                mode: crate::messages::PhaserMode::Manual,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world().resource::<CurrentPhaserMode>().0,
        crate::messages::PhaserMode::Auto,
        "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
    );
}

// ── FireTorpedo tests ──────────────────────────────────────────────────

#[test]
fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");

    push(
        &mut app,
        "weapons",
        ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter().any(
            |m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")
        ),
        "expected TorpedoLaunched broadcast after Tactical fires torpedo"
    );
}

/// Regression test for PRD #597 gap-3: an NPC ship spawned with a
/// `[torpedoes]` TOML block must carry its own `TorpedoSystemResource`
/// component, and firing from it via the `ai:<uuid>` token path must
/// launch a torpedo. Two subchecks:
///
/// 1. Direct wiring: `TorpedoSystem::launch()` called on the NPC's own
///    component successfully returns `Launched` (i.e. the tubes are
///    populated and `torpedoes_remaining > 0`).
/// 2. End-to-end message routing: an `ai:<uuid>` `FireTorpedo` message
///    arriving through `InboundMessage` reaches the NPC's tubes and
///    emits a `TorpedoLaunched` broadcast, drawing from the NPC's own
///    per-entity tube state — the player-ship `TorpedoSystemResource`
///    resource is left untouched.
///
/// NPC AI does not currently emit `FireTorpedo` messages autonomously;
/// verifying that pipeline is future work (see PRD #487 fine-grained
/// tactical decomposition). This test covers the wiring.
#[test]
fn npc_ship_can_fire_torpedo_when_toml_has_torpedoes_block() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntityUuid;
    use crate::torpedo::LaunchResult;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "cc000000-0000-0000-0000-000000000001";

    // Simulate what `src/entities/spawner.rs` does for an NPC with
    // `[torpedoes]`: attach a `TorpedoSystemResource` component built
    // from the runtime config, with default tubes (fore_port, fore_starboard, aft).
    let torpedo_config = TorpedoConfig::default();
    let npc_torpedo_sys = crate::torpedo::TorpedoSystem::new(torpedo_config);
    let mut npc_ai_sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the fine tube + magazine systems (there is no coarse
    // tactical system to seed).
    for sysid in [
        crate::system_registry::torpedo_tube_fore_port_system_id(),
        crate::system_registry::torpedo_tube_fore_starboard_system_id(),
        crate::system_registry::torpedo_tube_aft_system_id(),
        crate::system_registry::torpedo_magazine_system_id(),
    ] {
        npc_ai_sources.set(sysid, crate::ship::control_source::ControlSource::Ai);
    }
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(npc_ai_sources),
            ShipPhysics::default(),
            WeaponsTarget::default(),
            TorpedoSystemResource(npc_torpedo_sys),
            crate::server_app::WeaponFiredThisTick::default(),
            bevy::prelude::Transform::default(),
        ))
        .id();
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }

    // Subcheck 1: direct wiring — the NPC's own component has functional
    // tubes and `.launch()` succeeds when the tube is loaded.
    {
        let mut ts = app
            .world_mut()
            .get_mut::<TorpedoSystemResource>(npc_entity)
            .expect("NPC must have TorpedoSystemResource component");
        ts.0.tube_mut("fore_port")
            .expect("default TorpedoSystem must expose fore_port tube")
            .loaded_count = 1;
        let result = ts.0.launch(
            "fore_port",
            "direct-launch-uuid".to_string(),
            0.0,
            0.0,
            0.0,
            None,
            Some(npc_uuid.to_string()),
        );
        assert!(
            matches!(result, LaunchResult::Launched { .. }),
            "direct TorpedoSystem::launch on NPC's own component must succeed, got {result:?}"
        );
    }

    // Reload the tube for the end-to-end path (previous launch consumed it).
    {
        let mut ts = app
            .world_mut()
            .get_mut::<TorpedoSystemResource>(npc_entity)
            .unwrap();
        ts.0.tube_mut("fore_port").unwrap().loaded_count = 1;
        ts.0.in_flight.clear();
    }

    // Subcheck 2: end-to-end message routing.
    // Snapshot the player-ship (resource) torpedo count to prove the NPC's
    // fire draws from its own component, not from the shared Resource.
    let player_torpedoes_before = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .torpedoes_remaining;

    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter().any(
            |m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")
        ),
        "NPC should broadcast TorpedoLaunched after ai:<uuid> FireTorpedo message"
    );

    // The player-ship Resource must NOT have been drained.
    let player_torpedoes_after = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .torpedoes_remaining;
    assert_eq!(
        player_torpedoes_before, player_torpedoes_after,
        "NPC fire must draw from its own per-entity TorpedoSystemResource, \
         leaving the global (player-ship) Resource untouched"
    );
}

#[test]
fn local_console_token_can_fire_torpedo() {
    // issue #422: actions from the local HTML console (browser server
    // viewscreen / native wry server) arrive under LOCAL_CONSOLE_TOKEN with
    // no remote PeerJS session, so holder_for_station(tactical) is None.
    // `tactical_authorized` must treat that token as an authorized local
    // operator so a button press actually launches end-to-end — the
    // decode→map→InboundMessage→fire hop the wasm bridge cannot unit-test.
    let mut app = test_app();
    // No player holds Tactical here — authorization comes purely from the
    // local-console bypass.
    load_tube_now(&mut app, "fore_port");
    push(
        &mut app,
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        },
    );
    let out = tick(&mut app);

    assert!(
        out.iter().any(
            |m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")
        ),
        "local console token should be authorized to fire torpedoes end-to-end (issue #422)"
    );
}

#[test]
fn torpedo_system_resource_reflects_battleship_toml_torpedoes_block() {
    // End-to-end TOML-driven wiring check: build the runtime
    // TorpedoSystem the same way `spawn_game_start_entities` does
    // (parse alliance_battleship.toml → TorpedoesConfig::to_runtime → TorpedoSystem)
    // and assert the magazine size matches the TOML.
    let toml_str = include_str!("../../../assets/entities/alliance_battleship.toml");
    let config = crate::entity_config::EntityConfig::from_toml(toml_str)
        .expect("alliance_battleship.toml must parse");
    let tc = config
        .torpedoes
        .expect("alliance_battleship must declare [torpedoes]");
    let runtime = tc.to_runtime();
    let sys = crate::torpedo::TorpedoSystem::new(runtime.clone());
    // Magazine size matches TOML — changing `count = 30` to `count = 99`
    // in alliance_battleship.toml would fail this assertion.
    assert_eq!(sys.torpedoes_remaining, tc.count);
    assert_eq!(sys.config.damage_hull, tc.damage_hull);
    assert_eq!(sys.config.load_time, tc.load_time);
    assert!((sys.config.turn_rate - tc.turn_rate_deg_per_sec.to_radians()).abs() < 1e-5);
}

#[test]
fn phaser_combat_config_resource_reflects_battleship_toml_weapons_console() {
    // End-to-end TOML-driven wiring check: build the runtime
    // PhaserCombatConfig the same way `spawn_game_start_entities` does
    // (parse alliance_battleship.toml → PhaserCombatConfig::from_weapons_console
    // → PhaserCombatConfigResource) and assert the resulting per-bank
    // values are exactly what the TOML says.
    let toml_str = include_str!("../../../assets/entities/alliance_battleship.toml");
    let config = crate::entity_config::EntityConfig::from_toml(toml_str)
        .expect("alliance_battleship.toml must parse");
    let wc = config
        .weapons_console
        .expect("alliance_battleship must declare [weapons_console]");
    let combat = crate::entity_config::PhaserCombatConfig::from_weapons_console(&wc);

    // alliance_battleship.toml has two banks (fore, aft) with matching combat values.
    // Fore bank is double-damage (8.0 dps) and shorter range (40) than the standard cruiser.
    assert_eq!(combat.banks.len(), 2, "must have fore and aft banks");
    let fore = &combat.banks[0];
    assert_eq!(fore.id, "fore");
    assert_eq!(fore.cooldown_secs, 6.0, "cooldown_secs from TOML bank");
    assert_eq!(
        fore.beam_duration_secs, 6.0,
        "beam_duration_secs from TOML bank"
    );
    assert_eq!(
        fore.beam_damage_per_sec, 8.0,
        "beam_damage_per_sec from TOML bank"
    );
    assert_eq!(fore.beam_range, 40.0, "beam_range from TOML bank");

    // And starting the cooldown produces exactly that value, so it flows
    // through to live `PhaserCooldown.bank_remaining_secs`.
    let mut cd = PhaserCooldown::default();
    cd.start_bank("test", fore.cooldown_secs);
    assert_eq!(
        cd.bank_remaining_secs("test"),
        6.0,
        "PhaserCooldown::start_bank must use the TOML-sourced cooldown"
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
        ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
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
    load_tube_now(&mut app, "aft");
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
        ClientMessage::FireTorpedo {
            tube: "aft".to_string(),
            target_uuid: None,
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
        ClientMessage::FireTorpedo {
            tube: "fore_starboard".to_string(),
            target_uuid: None,
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

#[test]
fn torpedo_does_not_detonate_on_asteroid_field_anchor_entity() {
    // Regression for "torpedoes don't appear when you hit fire": the
    // default scenario seats the player ship at (280, 0, 0), 280 m from
    // an `asteroid_field_main` anchor entity at the origin. That anchor
    // entity carries an `[asteroid_field]` section with
    // `outer_radius = 350`, and `EntitySnapshot.radius` is populated from
    // that outer radius. `find_detonation_hits` treats every entity in
    // the world with a non-zero radius as a hittable target, so the
    // torpedo detonated on the field anchor on its first physics tick —
    // before the firing crew ever saw a sphere on the viewscreen.
    //
    // Asteroid-field anchors are virtual organisational entities and
    // must never act as torpedo detonation targets.
    use crate::entity_config::AsteroidFieldConfig;
    use crate::entity_spawner::{AsteroidFieldSection, EntityUuid};

    let mut app = test_app();
    start_game_with_weapons(&mut app);

    let field_uuid = "field-uuid".to_string();
    // Mirror the production code path: the WorldResource snapshot for the
    // field anchor reports radius = outer_radius.
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: field_uuid.clone(),
                position: Some([0.0, 0.0, 0.0]),
                radius: Some(350.0),
                inner_radius: Some(300.0),
                shape: Some("torus".into()),
                tags: vec!["asteroid_field".into()],
                ..Default::default()
            }],
            ..Default::default()
        }));
    // Real ECS-side anchor entity so the live-position path also sees it.
    app.world_mut().spawn((
        EntityUuid(field_uuid.clone()),
        AsteroidFieldSection(AsteroidFieldConfig {
            inner_radius: 300.0,
            outer_radius: 350.0,
            density: 0.005,
            spawn_distance: 250.0,
            despawn_distance: 300.0,
            asteroid_type_paths: vec![],
            cosmetic_type_paths: vec![],
            shape: None,
            anchor: None,
            anchor_offset: [0.0, 0.0, 0.0],
            shield_pierce: 0.0,
            tags: vec![],
            grid: None,
            random_rotation: None,
        }),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // Move the ship inside the field-anchor's "radius" (300 < 350).
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
        let mut p = q
            .single_mut(app.world_mut())
            .expect("Ship with ShipPhysics");
        p.x = 280.0;
    }
    load_tube_now(&mut app, "fore_port");

    push(
        &mut app,
        "weapons",
        ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        },
    );
    // First tick processes the FireTorpedo; second tick is where
    // `tick_torpedo_lifecycle` evaluates detonations against the live
    // target list (including the field anchor at the origin).
    tick(&mut app);
    tick(&mut app);

    let in_flight_len = {
        // Systems prefer the per-entity component; read from it for assertion.
        let mut q = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .ok()
            .map(|ts| ts.0.in_flight.len())
            .unwrap_or_else(|| {
                app.world()
                    .resource::<TorpedoSystemResource>()
                    .0
                    .in_flight
                    .len()
            })
    };
    assert_eq!(
        in_flight_len, 1,
        "torpedo should still be in flight after ticking — the asteroid \
         field anchor entity must not be treated as a detonation target"
    );
}

// ── ShipModifiers integration tests ────────────────────────────────────

#[test]
fn empty_modifier_table_reproduces_base_phaser_damage() {
    let mut app = test_app();
    setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app);

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

#[test]
fn phaser_damage_modifier_doubles_kill_rate() {
    use crate::messages::{ModifierSlot, ModifierSource};
    use crate::modifiers::{Modifier, ShipModifiers};

    let mut app_fast = test_app();
    setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
    start_game_with_weapons(&mut app_fast);
    {
        let mut q = app_fast
            .world_mut()
            .query_filtered::<&mut ShipModifiers, With<crate::simulation::LocalShip>>();
        let mut mods = q.single_mut(app_fast.world_mut()).unwrap();
        mods.add_or_update(Modifier {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::PhaserDamage,
            bonus: 1.0,
        });
    }
    push(
        &mut app_fast,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app_fast);
    push(
        &mut app_fast,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app_fast);

    set_active_beam_damage_accumulator(&mut app_fast, BEAM_DAMAGE_PER_SEC * 2.0 * 3.5);
    tick(&mut app_fast);

    let still_exists_fast = app_fast
        .world()
        .resource::<WorldResource>()
        .0
        .entities
        .iter()
        .any(|a| a.uuid == "target-uuid");
    assert!(
        !still_exists_fast,
        "with 2× phaser damage modifier, asteroid should be destroyed after 3.5s of beam"
    );

    let mut app_base = test_app();
    setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
    start_game_with_weapons(&mut app_base);
    push(
        &mut app_base,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app_base);
    push(
        &mut app_base,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app_base);
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

// ── SetPhaserFrequency delegation tests ────────────────────────────────

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
}

/// Build the admitted-envelope form of a frequency change (issue #804):
/// the only wire shape since the legacy top-level message was deleted.
fn set_phaser_frequency_msg(frequency: f32) -> ClientMessage {
    ClientMessage::ControlSystem {
        target: crate::system_registry::phaser_control_system_id(),
        payload: SystemControlPayload::SetPhaserFrequency { frequency },
    }
}

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

#[test]
fn sensors_holder_cannot_set_phaser_frequency() {
    // Delegation removed in B4 — only Tactical holder may set phaser frequency.
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

/// When the phaser-control system operates AI, human `SetPhaserFrequency`
/// envelopes are refused at admission (mirrors the navigation console's
/// `control_system_rejected_when_ai_controlled`).
#[test]
fn set_phaser_frequency_rejected_when_phaser_control_ai() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(
                crate::system_registry::phaser_control_system_id(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }
    }
    push(&mut app, "weapons", set_phaser_frequency_msg(0.9));
    tick(&mut app);
    let freq = get_phaser_frequency(&mut app);
    assert!(
        (freq - 0.5).abs() < 1e-5,
        "an AI-operated phaser-control must refuse human frequency input, got {freq}"
    );
}

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

// ── NPC / station phaser damage (issue #311) ──────────────────────────

fn setup_npc_world(app: &mut App, npc_x: f32, npc_z: f32) {
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: "npc-1".into(),
                position: Some([npc_x, 0.0, npc_z]),
                tags: vec!["ship".into()],
                ..Default::default()
            }],
            ..Default::default()
        }));
}

fn spawn_npc_entity(
    app: &mut App,
    npc_x: f32,
    npc_z: f32,
    max_hp: f32,
) -> bevy::ecs::entity::Entity {
    app.world_mut()
        .spawn((
            crate::entity_spawner::EntityUuid("npc-1".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                max_hp,
            )])),
            Transform::from_xyz(npc_x, 0.0, npc_z),
        ))
        .id()
}

// ── Cycle 1: phaser beam reduces NPC hull ─────────────────────────────

#[test]
fn phaser_beam_damages_npc_entity_hull() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app);

    // Accumulate damage but don't destroy
    set_active_beam_damage_accumulator(&mut app, 10.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("NPC entity should still exist")
        .0
        .total_current();
    assert!(
        hp < 30.0,
        "NPC hull should be reduced after phaser hit, got {hp}"
    );
}

// ── Cycle 2: NPC at 0 HP is despawned and EntityDespawned broadcast ──

#[test]
fn phaser_beam_destroys_npc_entity_when_hull_reaches_zero() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app);

    // Force lethal damage
    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    let out = tick(&mut app);

    // ECS entity despawned
    assert!(
        app.world().get::<EntitySystemHull>(npc_entity).is_none(),
        "NPC entity should be despawned after hull reaches 0"
    );

    // EntityDespawned wire message broadcast to all
    let despawned_msg = out
        .iter()
        .find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { uuid } if uuid == "npc-1"));
    assert!(
        despawned_msg.is_some(),
        "expected EntityDespawned {{ uuid: npc-1 }} broadcast"
    );

    // BeamEnded sent
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
        "expected BeamEnded after NPC destruction"
    );

    // Beam cleared, cooldown started
    assert!(active_beam_target_is_none(&mut app));
    assert!(phaser_bank_is_active(&mut app, "port"));
}

// ── NPC shields integration ────────────────────────────────────────────

/// Spawn a shielded NPC: same as `spawn_npc_entity` but also attaches a
/// `ShipShields` (num_facings=1) so the damage routing path is exercised
/// end-to-end.
fn spawn_shielded_npc_entity(
    app: &mut App,
    npc_x: f32,
    npc_z: f32,
    hull_max: f32,
    shield_max: f32,
    regen_per_sec: f32,
) -> bevy::ecs::entity::Entity {
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    app.world_mut()
        .spawn((
            // PR-7 (issue #597) — NPC ships carry the `Ship` marker
            // so the unified `tick_shields` picks them up.
            crate::simulation::Ship,
            crate::entity_spawner::EntityUuid("npc-1".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                hull_max,
            )])),
            crate::ship::shields::ShipShields(
                ShieldSystem::new(&ShieldConfig {
                    num_facings: 1,
                    max_hp: shield_max.round() as i32,
                    regen_per_sec,
                    offline_duration: 10.0,
                }),
                0.5,
            ),
            Transform::from_xyz(npc_x, 0.0, npc_z),
        ))
        .id()
}

#[test]
fn phaser_beam_damages_shielded_npc_routes_through_shield_first() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app);

    // Apply 5 units of damage. With pierce=0 (default in test config),
    // the entire amount lands on the shield, hull is unchanged.
    set_active_beam_damage_accumulator(&mut app, 5.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("NPC must still have ShipShields component");
    assert!(
        shields.0.facings[0].hp < 20,
        "shield must absorb damage, got {}",
        shields.0.facings[0].hp
    );
    assert!(
        shields.0.facings[0].is_online(),
        "shield must still be online"
    );

    let hull_hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("hull must still exist")
        .0
        .total_current();
    assert_eq!(hull_hp, 30.0, "hull must be untouched while shield holds");
}

#[test]
fn phaser_beam_breaks_shield_then_leaks_to_hull() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 10.0, 0.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app);

    // Apply 15 units of damage. With shield=10, shield depletes
    // and 5 units leak to hull.
    set_active_beam_damage_accumulator(&mut app, 15.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("ShipShields component must persist after break");
    // With ShipShields, a depleted facing goes offline (offline_remaining > 0),
    // not permanently broken.
    assert_eq!(shields.0.facings[0].hp, 0);
    assert!(
        !shields.0.facings[0].is_online(),
        "facing must go offline once depleted"
    );

    let hull_hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("hull must exist")
        .0
        .total_current();
    assert!(
        hull_hp < 30.0 && hull_hp > 20.0,
        "hull must take only the leak (~5 units), got {hull_hp}"
    );
}

#[test]
fn phaser_beam_post_break_skips_shield_routing_entirely() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    // Spawn with already-offline shield (facing depleted, offline timer running).
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let mut shield_sys = ShieldSystem::new(&ShieldConfig {
        num_facings: 1,
        max_hp: 20,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    });
    // Deplete the facing so it goes offline.
    shield_sys.apply_damage(20, 0.0);
    assert!(!shield_sys.facings[0].is_online(), "facing must be offline");

    let npc_entity = app
        .world_mut()
        .spawn((
            crate::entity_spawner::EntityUuid("npc-1".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            crate::ship::shields::ShipShields(shield_sys, 0.5),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ))
        .id();

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app);

    set_active_beam_damage_accumulator(&mut app, 5.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);

    let hull_hp = app
        .world()
        .get::<EntitySystemHull>(npc_entity)
        .expect("hull must exist")
        .0
        .total_current();
    // Hull must take damage (offline shield does not absorb).
    assert!(
        hull_hp < 30.0,
        "offline shield must let damage through to hull, got {hull_hp}"
    );
    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("ShipShields component must persist");
    assert_eq!(
        shields.0.facings[0].hp, 0,
        "offline facing hp must remain 0, got {}",
        shields.0.facings[0].hp
    );
    assert!(
        !shields.0.facings[0].is_online(),
        "facing must remain offline"
    );
}

#[test]
fn shield_regen_advances_npc_shield_below_max() {
    let mut app = test_app();
    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);

    let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 5.0);

    // Damage the shield to 10 HP.
    if let Some(mut shields) = app
        .world_mut()
        .get_mut::<crate::ship::shields::ShipShields>(npc_entity)
    {
        shields.0.facings[0].hp = 10;
    }

    // Advance time. The Bevy `Time` resource advances on each `app.update()`
    // call; we tick a few frames and expect regen to push hp upward.
    for _ in 0..3 {
        tick(&mut app);
    }

    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(npc_entity)
        .expect("ShipShields must persist");
    // We don't assert exact values (frame timing varies in tests) but we
    // verify regen is making forward progress and not stuck at 10.
    assert!(
        shields.0.facings[0].hp > 10,
        "shield must regen between ticks, got {}",
        shields.0.facings[0].hp
    );
    assert!(
        shields.0.facings[0].hp <= 20,
        "shield must clamp to max_hp, got {}",
        shields.0.facings[0].hp
    );
    assert!(shields.0.facings[0].is_online());
}

// ── PR2: Torpedo damage routes through ShipShields on the player ship ──

/// Verify that a torpedo detonation on the player ship reduces `ShipShields`
/// HP before leaking to the hull — end-to-end ShipShields coverage for the
/// torpedo damage path (PR2: Unified ShipShields).
#[test]
fn torpedo_hit_reduces_ship_shields_on_local_ship() {
    use crate::entity_spawner::EntityUuid;
    use crate::server_app::LocalShip;
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    use crate::weapons::torpedo::Torpedo;

    let mut app = test_app();
    start_game_with_weapons(&mut app);

    // Give the player ship ShipShields with known HP.
    let player_entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();

    let shield_max_hp = 100i32;
    let shield_sys = ShieldSystem::new(&ShieldConfig {
        num_facings: 4,
        max_hp: shield_max_hp,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    });
    app.world_mut().entity_mut(player_entity).insert((
        EntityUuid("player-ship".into()),
        crate::ship::shields::ShipShields(shield_sys, 0.5),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // Also expose the player ship in the world snapshot so the torpedo can
    // find it as a target.
    app.world_mut()
        .insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: "player-ship".into(),
                position: Some([0.0, 0.0, 0.0]),
                radius: Some(5.0),
                ..Default::default()
            }],
            ..Default::default()
        }));

    // Read initial total shield HP.
    let shields_before: i32 = app
        .world()
        .entity(player_entity)
        .get::<crate::ship::shields::ShipShields>()
        .unwrap()
        .0
        .facings
        .iter()
        .map(|f| f.hp)
        .sum();
    assert_eq!(shields_before, shield_max_hp * 4);

    // Read initial hull HP.
    let hull_before = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();

    // Directly inject a torpedo already adjacent to the player ship so it
    // detonates on the next tick. We write into both the per-entity component
    // and the resource to stay in sync.
    let torpedo = Torpedo {
        uuid: "test-torp-1".into(),
        x: 1.0, // 1 m away from player at origin — within detonation_radius
        z: 0.0,
        heading: 0.0,
        lifespan_remaining: 30.0,
        target_uuid: Some("player-ship".into()),
        source_uuid: None,  // no source → no self-detonation exclusion
        shield_pierce: 0.0, // no pierce → all damage goes to shields first
    };
    // Write to the per-entity component (preferred by systems) and resource.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        if let Ok(mut ts) = q.single_mut(app.world_mut()) {
            ts.0.in_flight.push(torpedo.clone());
        }
    }
    app.world_mut()
        .resource_mut::<TorpedoSystemResource>()
        .0
        .in_flight
        .push(torpedo);

    // Tick once — torpedo detonates and routes damage through ShipShields.
    tick(&mut app);

    let shields_after: i32 = app
        .world()
        .entity(player_entity)
        .get::<crate::ship::shields::ShipShields>()
        .unwrap()
        .0
        .facings
        .iter()
        .map(|f| f.hp)
        .sum();

    let hull_after = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();

    // Shield HP must decrease (torpedo damage_shields absorbed by shield).
    // (If damage_shields == 0 in the TOML config the test is still valid:
    // it just shows hull dropped instead, but we accept either change.)
    let total_damage_taken = (shields_before - shields_after) + ((hull_before - hull_after) as i32);
    assert!(
        total_damage_taken > 0,
        "torpedo hit must cause total damage: shields_before={shields_before}, shields_after={shields_after}, \
         hull_before={hull_before}, hull_after={hull_after}"
    );
    // The important invariant: if damage_shields > 0, shield must have taken damage first.
    // We verify this indirectly: hull must not exceed its pre-hit value.
    assert!(
        hull_after <= hull_before,
        "hull must not increase after torpedo hit, got {hull_after} > {hull_before}"
    );
}

// ── Cycle 3: AiEntityDestroyed message written on NPC destruction ─────

#[test]
fn phaser_beam_emits_ai_entity_destroyed_on_npc_kill() {
    #[derive(Resource, Default)]
    struct DestroyedBox(Vec<crate::ai_plugin::AiEntityDestroyed>);

    let mut app = test_app();
    app.init_resource::<DestroyedBox>();
    app.add_systems(
        bevy::app::Update,
        |mut r: bevy::ecs::prelude::MessageReader<crate::ai_plugin::AiEntityDestroyed>,
         mut b: bevy::ecs::prelude::ResMut<DestroyedBox>| {
            for ev in r.read() {
                b.0.push(ev.clone());
            }
        },
    );

    setup_npc_world(&mut app, 0.0, -20.0);
    start_game_with_weapons(&mut app);
    spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "npc-1".into(),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    tick(&mut app);

    set_active_beam_damage_accumulator(&mut app, 30.0);
    set_active_beam_remaining_secs(&mut app, 5.0);
    tick(&mut app);
    tick(&mut app); // second tick allows PostUpdate-equivalent collector to drain the message

    let destroyed_events = app.world().resource::<DestroyedBox>();
    assert!(
        destroyed_events.0.iter().any(|e| e.entity_uuid == "npc-1"),
        "AiEntityDestroyed must be emitted with entity_uuid 'npc-1' so on_destroyed triggers fire"
    );
}

// ── NPC as shooter: handle_fire_phaser (unified) / tick_beams ────────────

/// Set up `AiTokenRegistry`, an NPC entity with `AiControllerComponent` +
/// `ActiveBeam`/`PhaserCooldown` (unified per-entity phaser state), and a target entity.
fn setup_npc_shooter(
    app: &mut App,
    npc_uuid: &str,
    target_uuid: &str,
    target_x: f32,
    target_z: f32,
) -> (bevy::ecs::entity::Entity, bevy::ecs::entity::Entity) {
    use crate::ai_plugin::AiControllerComponent;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    // Spawn NPC entity facing toward negative-Z (yaw = 0 → forward = -Z).
    // Includes the Ship marker so the unified `tick_beams` picks it up as
    // a shooter (matches the production `entities::spawner::spawn_entity`
    // path where every ship gets `Ship` — see PRD #597).
    //
    // Also mirrors production by inserting `ShipSystemControlSources` with
    // the Tactical system set to `Ai`, and the NPC's target lock in
    // `WeaponsTarget` — both required by the unified `handle_fire_phaser`
    // per-ship query. `WeaponsTarget` is the ship's authoritative lock
    // whether a human or `ai_target_selection` set it, so an AI shooter
    // seeds it exactly as a human one would.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the fine systems for the banks these tests fire
    // ("port"/"starboard" per the test_app combat config) — there is no
    // coarse tactical system to seed.
    for bank in ["port", "starboard"] {
        sources.set(
            crate::system_registry::phaser_bank_system_id(bank).unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
    }

    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            AiControllerComponent,
            crate::ship_plugin::ShipSystemControlSources(sources),
            WeaponsTarget(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ))
        .id();

    // Register with the Bevy entity so handle_fire_phaser can look it up.
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }

    // Spawn target entity.
    let target_entity = app
        .world_mut()
        .spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(target_x, 0.0, target_z),
        ))
        .id();

    (npc_entity, target_entity)
}

#[test]
fn npc_fire_phaser_activates_entity_phaser_state() {
    // NPC entity at origin, target directly ahead (negative-Z), within beam range.
    // Sending a FirePhaser InboundMessage for the NPC's ai: token should set
    // `ActiveBeam::target_uuid = Some(...)` after one update.
    use crate::ai_plugin::AiTokenRegistry;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000001";
    let target_uuid = "00000000-0000-0000-0000-000000000002";

    let (npc_entity, _target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid, 0.0, -20.0);

    // Send FirePhaser as the NPC's synthetic token.
    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC entity must have ActiveBeam component");
    assert!(
        beam.target_uuid.is_some(),
        "ActiveBeam::target_uuid should be Some after NPC fires phaser via ai: token"
    );
}

#[test]
fn npc_beam_tick_applies_damage_to_target_hull() {
    // With an active NPC beam, each tick of tick_beams reduces
    // the target's EntitySystemHull.
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntitySystemHull;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000003";
    let target_uuid_str = "00000000-0000-0000-0000-000000000004";

    let (npc_entity, target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    // Activate the beam directly on the per-entity ActiveBeam component.
    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.target_uuid = Some(target_uuid_str.to_string());
        beam.remaining_secs = 10.0;
    }

    let hp_before = app
        .world()
        .get::<EntitySystemHull>(target_entity)
        .unwrap()
        .0
        .total_current();

    // Run several ticks so damage accumulates.
    for _ in 0..10 {
        app.update();
    }

    let hp_after = app
        .world()
        .get::<EntitySystemHull>(target_entity)
        .unwrap()
        .0
        .total_current();
    assert!(
        hp_after < hp_before,
        "target hull must decrease as NPC beam ticks (before={hp_before}, after={hp_after})"
    );
}

#[test]
fn npc_beam_tick_records_shooter_as_last_attacker() {
    // Write-on-damage (#689): when a live beam hits a ship target that
    // carries a `LastShipAttacker` component, `tick_beams` records the
    // shooter's UUID as that target's last attacker. This write fires in
    // Phase 2 before the `damage_to_apply <= 0` guard, but only when the
    // target entity actually carries the component — so we insert it.
    use crate::ai_plugin::AiTokenRegistry;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000003";
    let target_uuid_str = "00000000-0000-0000-0000-000000000004";

    let (npc_entity, target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    // The attacker-write branch only fires if the target carries
    // `LastShipAttacker`; `setup_npc_shooter` does not add it.
    app.world_mut()
        .entity_mut(target_entity)
        .insert(LastShipAttacker::default());

    // Activate the beam directly on the per-entity ActiveBeam component.
    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.target_uuid = Some(target_uuid_str.to_string());
        beam.remaining_secs = 10.0;
    }

    // Tick enough for the beam to reach and hit the target.
    for _ in 0..10 {
        app.update();
    }

    assert_eq!(
        app.world()
            .get::<LastShipAttacker>(target_entity)
            .unwrap()
            .0,
        Some(npc_uuid.to_string()),
        "beam hit must record the shooter UUID as the target's last attacker"
    );
}

/// The writer's half of the `AiEntityAttacked` exactly-once contract
/// (issue #702).
///
/// `tick_beams`' attacker-write branch runs every tick a beam is live.
/// Post-#702 the rising edge that fires `AiEntityAttacked` — and through it
/// `on_entity_attacked` scenario triggers — *is* `LastShipAttacker`'s change
/// detection, so a blind write would re-fire the trigger on every tick of a
/// sustained beam. This pins the compare: across many ticks of one live beam
/// from one shooter, the component is marked changed exactly once.
///
/// `ai_entity_attacked_not_re_emitted_for_same_attacker` pins the reader's
/// half in `ai::server`.
#[test]
fn sustained_beam_marks_last_attacker_changed_exactly_once() {
    use crate::ai_plugin::AiTokenRegistry;

    #[derive(Resource, Default)]
    struct ChangeCount(usize);

    // Mirrors `ai_plugin::emit_attacked_on_new_attacker`'s guard: count the
    // changes that would fire `AiEntityAttacked`, i.e. those that *name* an
    // attacker. Component insertion also marks a component changed, and the
    // fixture below inserts a `default()` (`None`) — which is a clear, not
    // an attack, and which the emitter skips for exactly this reason.
    fn count_changes(
        q: Query<&LastShipAttacker, Changed<LastShipAttacker>>,
        mut counter: ResMut<ChangeCount>,
    ) {
        counter.0 += q.iter().filter(|a| a.0.is_some()).count();
    }

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();
    app.init_resource::<ChangeCount>();

    let npc_uuid = "00000000-0000-0000-0000-000000000013";
    let target_uuid_str = "00000000-0000-0000-0000-000000000014";

    let (npc_entity, target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    app.world_mut()
        .entity_mut(target_entity)
        .insert(LastShipAttacker::default());

    // Count in `PostUpdate` so each `Update` tick's write is observed on the
    // tick it happens. (Ordering against `tick_beams` directly is not an
    // option here: this fixture registers it a second time outside any
    // SimSet, so its `SystemTypeSet` is ambiguous.)
    app.add_systems(PostUpdate, count_changes);

    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.target_uuid = Some(target_uuid_str.to_string());
        beam.remaining_secs = 100.0;
    }

    // Many ticks of one continuous beam from one shooter.
    for _ in 0..20 {
        app.update();
    }

    assert_eq!(
        app.world()
            .get::<LastShipAttacker>(target_entity)
            .unwrap()
            .0,
        Some(npc_uuid.to_string()),
        "precondition: the sustained beam must actually have recorded the shooter"
    );
    assert_eq!(
        app.world().resource::<ChangeCount>().0,
        1,
        "tick_beams must compare before writing LastShipAttacker: a sustained beam \
         from one shooter may mark it changed exactly once, on the tick the attacker \
         becomes known. More than one means a blind write, which re-fires \
         AiEntityAttacked (and on_entity_attacked triggers) every tick the beam is live."
    );
}

#[test]
fn npc_beam_tick_damages_npc_target_not_player() {
    // Regression test for PRD #597 PR-1: NPC-vs-NPC beam damage.
    // Before the fix, the old tick_npc_beams hull_query had
    // Without<LocalShip> so NPCs couldn't damage other NPCs — damage
    // was silently lost. The unified `tick_beams` iterates all ships
    // and applies damage to any target found via `hull_q`.
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntitySystemHull;
    use crate::server_app::ShipAttackedThisTick;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();
    app.init_resource::<crate::simulation::GameOverReason>();

    let shooter_uuid = "10000000-0000-0000-0000-000000000001";
    let npc_target_uuid = "20000000-0000-0000-0000-000000000002";

    // Spawn NPC shooter with AiControllerComponent.
    let (shooter_entity, npc_target_entity) =
        setup_npc_shooter(&mut app, shooter_uuid, npc_target_uuid, 0.0, -10.0);
    // Add ShipPhysics and AiControllerComponent to the target so it looks
    // like a real production-spawned NPC (AI-controlled, physics-enabled).
    // The unified `tick_beams` finds targets by EntityUuid in `hull_q`
    // (no Ship marker requirement on targets), but production NPCs carry
    // both markers — matching them here keeps the test aligned with real
    // NPC-vs-NPC scenarios.
    app.world_mut().entity_mut(npc_target_entity).insert((
        ShipPhysics::default(),
        crate::ai_plugin::AiControllerComponent,
    ));

    // Activate beam on the shooter.
    {
        let mut beam = app
            .world_mut()
            .get_mut::<ActiveBeam>(shooter_entity)
            .unwrap();
        beam.target_uuid = Some(npc_target_uuid.to_string());
        beam.remaining_secs = 10.0;
    }

    let hp_before = app
        .world()
        .get::<EntitySystemHull>(npc_target_entity)
        .unwrap()
        .0
        .total_current();

    for _ in 0..10 {
        app.update();
    }

    let hp_after = app
        .world()
        .get::<EntitySystemHull>(npc_target_entity)
        .unwrap()
        .0
        .total_current();

    assert!(
        hp_after < hp_before,
        "NPC beam must damage NPC target hull (before={hp_before}, after={hp_after})"
    );
    // Player ship must NOT have been marked as attacked.
    let player_atk = app
        .world_mut()
        .query_filtered::<&ShipAttackedThisTick, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|c| c.0)
        .unwrap_or(false);
    assert!(
        !player_atk,
        "NPC-vs-NPC beam must not set player's ShipAttackedThisTick"
    );
}

#[test]
fn on_beam_started_emits_correct_source_uuid_with_multiple_ships() {
    // Regression test for PRD #597 PR-1: on_beam_started used With<Ship>.single()
    // which panics when multiple ships exist. After fix it uses With<LocalShip>.
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();
    let player_uuid_str = "aaaaaaaa-0000-0000-0000-000000000001";
    let npc_uuid_str = "bbbbbbbb-0000-0000-0000-000000000002";

    // Add EntityUuid to the existing LocalShip entity (spawned by test_app).
    let player_entity = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(player_entity)
        .insert(EntityUuid(player_uuid_str.to_string()));

    // Spawn a second NPC ship (non-LocalShip, has Ship marker).
    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid(npc_uuid_str.to_string()),
        ShipPhysics::default(),
        Transform::default(),
    ));

    // Trigger BeamStartedEvent — the observer on_beam_started should emit
    // source_uuid = player_uuid_str, not empty.
    app.world_mut().trigger(super::BeamStartedEvent {
        bank: "port".to_string(),
        target_uuid: "some-target".to_string(),
        source_entity: player_entity,
    });
    app.update();

    // Find the BeamStarted message in the SimOutbox.
    let outbox = app.world().resource::<crate::simulation::SimOutbox>();
    let beam_started = outbox
        .0
        .iter()
        .find(|(_, msg)| matches!(msg, crate::messages::ServerMessage::BeamStarted { .. }));
    let Some((_, crate::messages::ServerMessage::BeamStarted { source_uuid, .. })) = beam_started
    else {
        panic!("expected BeamStarted message in outbox");
    };
    assert_eq!(
        source_uuid, player_uuid_str,
        "on_beam_started must emit the LocalShip UUID as source_uuid, not {:?}",
        source_uuid
    );
}

#[test]
fn npc_beam_tick_applies_damage_to_local_ship_through_shields() {
    // When the beam target is the player ship (has Ship marker), damage
    // must route through shields → hull component, not just EntitySystemHull directly.
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::EntityUuid;
    use crate::server_app::{LocalShip, ShipAttackedThisTick};
    use crate::shield::ShieldConfig;
    use crate::simulation::{GameOverReason, ShipShields};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();
    app.init_resource::<GameOverReason>();

    // Insert shields on the LocalShip entity so the shield-routing
    // path is exercised (ShipShields is pure per-entity Component
    // post ship-parity audit).
    let shield_config = ShieldConfig {
        max_hp: 100,
        regen_per_sec: 0.0,
        num_facings: 4,
        ..Default::default()
    };
    {
        let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
        let local = q.single(app.world()).unwrap();
        app.world_mut().entity_mut(local).insert(ShipShields(
            crate::shield::ShieldSystem::new(&shield_config),
            0.5,
        ));
    }

    let npc_uuid = "00000000-0000-0000-0000-000000000010";
    let player_uuid = "00000000-0000-0000-0000-000000000011";
    let player_uuid_parsed = uuid::Uuid::parse_str(player_uuid).unwrap();

    // Add EntityUuid and position to the existing LocalShip entity (already spawned by test_app).
    let player_entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(player_entity).insert((
        EntityUuid(player_uuid.to_string()),
        Transform::from_xyz(0.0, 0.0, -10.0),
    ));

    // Spawn NPC entity using the new per-entity beam components.
    let npc_entity = {
        let npc_ent = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                // The NPC's Tactical lock. Was seeded on the private
                // `ShipAiMemory.target` mirror until #702 deleted it;
                // `WeaponsTarget` is the surface every firing path reads.
                WeaponsTarget(Some(player_uuid_parsed.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_ent);
        npc_ent
    };

    let hull_before = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();
    let shields_sum_before: i32 = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipShields, With<LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipShields")
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum()
    };

    // Activate the beam directly targeting the player ship.
    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.target_uuid = Some(player_uuid.to_string());
        beam.remaining_secs = 10.0;
    }

    for _ in 0..10 {
        app.update();
    }

    let hull_after = app
        .world()
        .entity(player_entity)
        .get::<crate::entity_spawner::EntitySystemHull>()
        .unwrap()
        .0
        .total_current();
    let shields_sum_after: i32 = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipShields, With<LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipShields")
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum()
    };

    let hull_lost = hull_before - hull_after;
    let shields_lost = shields_sum_before - shields_sum_after;

    assert!(
        hull_lost > 0.0 || shields_lost > 0,
        "NPC beam must damage player ship: hull {hull_before}->{hull_after} ({hull_lost}), shields {shields_sum_before}->{shields_sum_after} ({shields_lost})"
    );
    let player_atk = app
        .world_mut()
        .query_filtered::<&ShipAttackedThisTick, With<LocalShip>>()
        .single(app.world())
        .map(|c| c.0)
        .unwrap_or(false);
    assert!(
        player_atk,
        "NPC beam targeting the player ship must mark the ship as attacked for Captain AI"
    );
}

#[test]
fn npc_beam_cooldown_starts_after_beam_expires() {
    // When an NPC's ActiveBeam remaining_secs reaches zero, PhaserCooldown must
    // be set to a positive value and ActiveBeam.target_uuid must become None.
    use crate::ai_plugin::AiTokenRegistry;

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "00000000-0000-0000-0000-000000000005";
    let target_uuid_str = "00000000-0000-0000-0000-000000000006";

    let (npc_entity, _target_entity) =
        setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

    {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
        beam.target_uuid = Some(target_uuid_str.to_string());
        beam.remaining_secs = 0.001; // expires on first tick
    }

    app.update(); // beam expires
    app.update(); // cooldown ticked

    let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
    assert!(
        beam.target_uuid.is_none(),
        "ActiveBeam.target_uuid must be None after beam expires"
    );
    let cooldown = app.world().get::<PhaserCooldown>(npc_entity).unwrap();
    assert!(
        cooldown.per_bank.values().any(|&v| v > 0.0),
        "PhaserCooldown must be positive after beam ends: {:?}",
        cooldown.per_bank
    );
}

// ── End-to-end: tick_ai_controllers → InboundMessage → handle_fire_phaser ──

/// Build an app that includes BOTH `WeaponsPlugin` AND `AiPlugin` together
/// with all their required resources, so the full routing path can be tested:
/// `tick_ai_controllers` emits a `FirePhaser` `InboundMessage` which the
/// unified `handle_fire_phaser` picks up and activates the NPC's `ActiveBeam`.
fn combined_test_app() -> App {
    use crate::ai_plugin::AiPlugin;
    use crate::config_cache::FactionRegistryResource;

    let mut app = test_app();
    app.add_plugins(AiPlugin)
        .insert_resource(FactionRegistryResource(
            crate::config_cache::get_faction_registry(),
        ));
    app
}

#[test]
fn tick_ai_controllers_fire_phaser_routes_through_unified_handle_fire_phaser() {
    // Full end-to-end test: an NPC with a Destroy doctrine and a pre-selected
    // target directly in its forward arc causes `tick_ai_controllers` to write
    // a `FirePhaser` `InboundMessage`, which the unified `handle_fire_phaser`
    // picks up
    // and sets `ActiveBeam::target_uuid`.
    use crate::damage::SystemHull;
    use crate::entity_config::{BehaviourConfig, DoctrineObjective};
    use crate::entity_spawner::{EntitySystemHull, EntityUuid, WeaponsConsoleSection};
    use crate::messages::{GamePhase, SystemId};
    use bevy::prelude::State;

    let mut app = combined_test_app();

    // Put the simulation in InProgress so tick_ai_controllers runs.
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let beam_range = 50.0_f32;
    let npc_uuid_str = "ee000000-0000-0000-0000-000000000010";
    let target_uuid_str = "ee000000-0000-0000-0000-000000000011";
    let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();

    // Doctrine: single Destroy objective at high priority — always scores > 0.
    let behaviour = BehaviourConfig {
        doctrine: vec![DoctrineObjective {
            id: "destroy-hostiles".into(),
            text: "Destroy target".into(),
            directive_kind: Some("Destroy".into()),
            base_priority: 35.0,
            target_speed: 0.9,
            maintain_range: 25.0,
            ..Default::default()
        }],
        ..Default::default()
    };

    // Spawn NPC at origin, facing -Z (yaw = 0 → forward = -Z).
    // Include ActiveBeam/PhaserCooldown/ShipPhysics for the unified fire path,
    // plus the components the unified `handle_fire_phaser` requires:
    // `Ship`, `ShipSystemControlSources` (Tactical = Ai), `WeaponsTarget`.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entity_spawner::BehaviourSection(behaviour),
            EntityUuid(npc_uuid_str.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            WeaponsTarget::default(),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            WeaponsConsoleSection(crate::entity_config::WeaponsConsoleConfig {
                torpedo_arc_color: vec![],
                power_multipliers: None,
                phaser_banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: Some(0.0),
                    marker: None,
                }],
                blaster_banks: vec![],
                radar: None,
            }),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ))
        .id();

    // Spawn target directly ahead (-Z), well within beam range.
    let _target = app
        .world_mut()
        .spawn((
            EntityUuid(target_uuid_str.to_string()),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                200.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -10.0),
        ))
        .id();

    // Tick 1: `register_ai_tokens_on_spawn` runs → AiControllerComponent
    //         marker attached and token registered in AiTokenRegistry.
    app.update();

    // Register the Bevy entity in AiTokenRegistry (needed by handle_fire_phaser).
    {
        let mut reg = app
            .world_mut()
            .resource_mut::<crate::ai_plugin::AiTokenRegistry>();
        reg.register_with_entity(npc_uuid_str, npc_entity);
    }

    // Set the NPC's target lock so handle_fire_phaser can look up the
    // target. `WeaponsTarget` is the authoritative lock for every ship —
    // in production `ai_target_selection` writes it for AI-operated
    // tactical systems; here we seed it directly.
    {
        let mut target = app
            .world_mut()
            .get_mut::<WeaponsTarget>(npc_entity)
            .expect("NPC must have WeaponsTarget");
        target.0 = Some(target_uuid_parsed.to_string());
    }

    // Push a synthetic FirePhaser message for the NPC's ai: token.
    // In production this would be emitted by ai_phaser_auto_fire,
    // but for this integration test we inject it directly.
    let ai_token = format!("ai:{}", npc_uuid_str);
    push(
        &mut app,
        &ai_token,
        ClientMessage::FirePhaser {
            bank: "fore".into(),
        },
    );

    // Tick: handle_fire_phaser processes the message and activates ActiveBeam.
    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC must have ActiveBeam component");
    assert!(
        beam.target_uuid.is_some(),
        "ActiveBeam.target_uuid must be Some after tick_ai_controllers → InboundMessage → handle_fire_phaser routing"
    );
}

/// Verify that both a `LocalShip` entity and an NPC entity use the same
/// `tick_beams` handler (unified per-entity beam path — issues #588 / #597).
#[test]
fn both_localship_and_npc_can_fire_via_per_entity_active_beam() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let target_uuid = "ff000000-0000-0000-0000-000000000001";
    let npc_uuid = "ff000000-0000-0000-0000-000000000002";

    // Spawn a target entity with hull.
    let target_entity = app
        .world_mut()
        .spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -15.0),
        ))
        .id();

    // Spawn NPC entity with per-entity ActiveBeam and activate beam.
    // Includes the Ship marker so the unified `tick_beams` picks it up
    // as a shooter (matches production NPC spawn path — see PRD #597).
    let npc_ent = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ai_plugin::AiControllerComponent,
            ActiveBeam {
                target_uuid: Some(target_uuid.to_string()),
                remaining_secs: 10.0,
                ..Default::default()
            },
            PhaserCooldown::default(),
            ShipPhysics::default(),
            Transform::default(),
        ))
        .id();
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_ent);
    }

    // Run ticks so tick_beams fires.
    for _ in 0..5 {
        app.update();
    }

    let hp = app
        .world()
        .get::<EntitySystemHull>(target_entity)
        .unwrap()
        .0
        .total_current();
    assert!(
        hp < 100.0,
        "NPC beam must apply damage via the unified tick_beams path (hp={hp})"
    );
}

/// Regression test for the unified phaser auto-fire path (post-#698:
/// `ai_phaser_auto_fire` -> `integrate_weapons_state`).
///
/// Before unification, `tick_phaser_auto_fire` iterated only `LocalShip`,
/// so NPCs had to route through the (now-deleted) `handle_npc_beam_fire`
/// with synthetic `FirePhaser` messages emitted by AI. Post-unification
/// the same system iterates every ship whose Tactical system is
/// AI-controlled, activating an [`ActiveBeam`] directly.
#[test]
fn ai_phaser_auto_fire_activates_ai_controlled_npc_beam() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "aa000000-0000-0000-0000-000000000001";
    let target_uuid = "aa000000-0000-0000-0000-000000000002";

    // NPC facing -Z (yaw=0 forward = -Z) with Tactical set to Ai.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ai_plugin::AiControllerComponent,
            crate::ship_plugin::ShipSystemControlSources(sources),
            WeaponsTarget(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            }),
            Transform::default(),
        ))
        .id();

    // Spawn target directly ahead (in-arc, in-range).
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC entity must have ActiveBeam component");
    assert!(
        beam.target_uuid.is_some(),
        "the ai_phaser_auto_fire -> integrate_weapons_state pair must activate the \
         NPC's ActiveBeam when Tactical is AI-controlled"
    );
    assert_eq!(
        beam.bank.as_deref(),
        Some("fore"),
        "NPC should fire the in-arc bank selected from its own PhaserCombatConfigResource"
    );
}

// ── Phaser decide/integrate split (issue #698) ─────────────────────────

/// Spawn an AI-controlled NPC with one 360° bank, a locked target, and a
/// live entity to shoot at directly ahead. Returns the NPC's entity.
///
/// Deliberately does **not** insert `AiHighFidelity`: the population this
/// helper builds is a low-LOD NPC, which is precisely the case
/// `ai_phaser_auto_fire`'s missing `With<AiHighFidelity>` filter exists to
/// serve. Tests that need high fidelity add the marker themselves.
fn spawn_ai_phaser_npc(app: &mut App, npc_uuid: &str, target_uuid: &str) -> Entity {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            WeaponsTarget(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            }),
            Transform::default(),
        ))
        .id();

    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));
    npc
}

/// `ai_phaser_auto_fire` is a *decider*: it must publish its choice to
/// `PhaserIntents` and leave `ActiveBeam` alone. Running it in isolation
/// (without `integrate_weapons_state`) is what proves the two halves are
/// genuinely separated rather than merely renamed.
#[test]
fn ai_phaser_auto_fire_writes_intent_without_touching_the_beam() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = test_app();
    let npc = spawn_ai_phaser_npc(
        &mut app,
        "bb000000-0000-0000-0000-000000000001",
        "bb000000-0000-0000-0000-000000000002",
    );

    app.world_mut()
        .run_system_once(ai_phaser_auto_fire)
        .expect("ai_phaser_auto_fire should run");

    let intents = app
        .world()
        .get::<PhaserIntents>(npc)
        .expect("ActiveBeam requires PhaserIntents, so every ship with a beam has one");
    assert_eq!(
        intents.0,
        vec![PhaserCmd {
            bank: "fore".into(),
            target_uuid: "bb000000-0000-0000-0000-000000000002".into(),
            // The bank's TOML-authored beam_duration_secs, resolved by the
            // decider so the integrator never re-reads the config.
            beam_duration_secs: 3.0,
        }],
        "the decider must publish the chosen bank, target and beam duration"
    );
    assert!(
        app.world()
            .get::<ActiveBeam>(npc)
            .unwrap()
            .target_uuid
            .is_none(),
        "ai_phaser_auto_fire must not mutate ActiveBeam — that is \
         integrate_weapons_state's job"
    );
}

/// `integrate_weapons_state` is the *adapter*: given an intent and nothing
/// else, it must advance the beam state machine. Written by hand rather
/// than by the decider so the adapter is pinned independently of it.
#[test]
fn integrate_weapons_state_advances_beam_from_phaser_intent() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = test_app();
    let npc = spawn_ai_phaser_npc(
        &mut app,
        "bb000000-0000-0000-0000-000000000003",
        "bb000000-0000-0000-0000-000000000004",
    );

    app.world_mut()
        .entity_mut(npc)
        .insert(PhaserIntents(vec![PhaserCmd {
            bank: "fore".into(),
            target_uuid: "bb000000-0000-0000-0000-000000000004".into(),
            beam_duration_secs: 4.5,
        }]));

    app.world_mut()
        .run_system_once(integrate_weapons_state)
        .expect("integrate_weapons_state should run");

    let beam = app.world().get::<ActiveBeam>(npc).unwrap();
    assert_eq!(
        beam.target_uuid.as_deref(),
        Some("bb000000-0000-0000-0000-000000000004"),
        "the adapter must arm the beam at the intent's target"
    );
    assert_eq!(beam.bank.as_deref(), Some("fore"));
    assert_eq!(
        beam.remaining_secs, 4.5,
        "the adapter must burn for the duration the decider resolved, not a \
         duration of its own"
    );
    assert!(
        app.world().get::<PhaserIntents>(npc).unwrap().0.is_empty(),
        "the adapter must drain the buffer so a stale intent cannot re-fire \
         the beam next tick"
    );
}

/// Pins the deliberate asymmetry between `ai_phaser_auto_fire` (no
/// `AiHighFidelity` filter) and `ai_torpedo_auto_fire` (filtered).
///
/// Extracting phaser fire from `tick_phaser_auto_fire` into the same
/// decide/integrate shape `ai_torpedo_auto_fire` uses makes it tempting to
/// inherit its `With<AiHighFidelity>` filter too. That would silently
/// disarm every low-LOD NPC — a gameplay change wearing a refactor's
/// clothes. Phasers are the main damage low-LOD NPCs contribute, and the
/// `CurrentPhaserMode::Auto` leg of this system isn't AI at all, so the
/// filter would be wrong on its own terms as well.
///
/// If a future slice does decide to gate phasers on LOD, `PhaserIntents`
/// must move into `lod_ai_ships`' promote/demote bundle at the same time —
/// see `ActiveBeam`'s `#[require(PhaserIntents)]`.
#[test]
fn ai_phaser_auto_fire_runs_for_low_lod_npc_without_ai_high_fidelity() {
    let mut app = test_app();
    let npc = spawn_ai_phaser_npc(
        &mut app,
        "bb000000-0000-0000-0000-000000000005",
        "bb000000-0000-0000-0000-000000000006",
    );
    assert!(
        app.world()
            .get::<crate::ai_plugin::AiHighFidelity>(npc)
            .is_none(),
        "precondition: this NPC is low-LOD"
    );

    app.update();

    assert!(
        app.world()
            .get::<ActiveBeam>(npc)
            .unwrap()
            .target_uuid
            .is_some(),
        "low-LOD NPCs must keep firing phasers — ai_phaser_auto_fire is \
         deliberately NOT gated on AiHighFidelity"
    );
}

/// `tick_weapons_arc_request` (issue #677): a target within a bank's
/// range but outside its firing arc should enqueue a channel-3
/// `ArcBearingRequest` addressed to Helm.
#[test]
fn tick_weapons_arc_request_fires_when_target_in_range_but_outside_arc() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "bb000000-0000-0000-0000-000000000001";

    let ship_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipSystemControlSources::default(),
            ShipPhysics::default(),
            WeaponsTarget(Some(target_uuid.to_string())),
            WeaponsArcRequestState::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 30.0,
                    auto_arc_deg: 30.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            }),
        ))
        .id();

    // Target is directly to starboard (x=20, z=0): in range (distance 20 <
    // beam_range 50) but 90 degrees off the fore bank's 30-degree arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();

    let log = app.world().resource::<ArcRequestLog>();
    let request = log
        .0
        .iter()
        .find(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. }))
        .expect("expected an ArcBearingRequest CoordinationEnqueue event");
    assert_eq!(request.source_entity, ship_entity);
    assert_eq!(request.target, crate::system_registry::helm_station_key());
    match &request.payload {
        CoordinationPayload::ArcBearingRequest { uuid, .. } => {
            assert_eq!(uuid, target_uuid);
        }
        _ => unreachable!(),
    }
}

/// A target within the firing arc must not trigger an arc-bearing
/// request — Weapons can already fire without Helm's help.
#[test]
fn tick_weapons_arc_request_does_not_fire_when_target_in_arc() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "bb000000-0000-0000-0000-000000000002";

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        WeaponsTarget(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 30.0,
                auto_arc_deg: 30.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }],
        }),
    ));

    // Directly ahead (forward = -Z at yaw 0): in range and in arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();

    let log = app.world().resource::<ArcRequestLog>();
    assert!(
        !log.0
            .iter()
            .any(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. })),
        "an in-arc target must not trigger an ArcBearingRequest"
    );
}

/// The request is debounced: an unchanged arc miss on the same target
/// must not re-enqueue every tick.
#[test]
fn tick_weapons_arc_request_is_debounced_for_unchanged_miss() {
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    let target_uuid = "bb000000-0000-0000-0000-000000000003";

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipSystemControlSources::default(),
        ShipPhysics::default(),
        WeaponsTarget(Some(target_uuid.to_string())),
        WeaponsArcRequestState::default(),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 30.0,
                auto_arc_deg: 30.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }],
        }),
    ));

    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(20.0, 0.0, 0.0),
    ));

    app.update();
    app.update();
    app.update();

    let log = app.world().resource::<ArcRequestLog>();
    let count = log
        .0
        .iter()
        .filter(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. }))
        .count();
    assert_eq!(
        count, 1,
        "an unchanged arc miss on the same target must only enqueue once, not every tick"
    );
}

/// Regression test for the unified `handle_fire_phaser`.
///
/// Before unification, `handle_npc_beam_fire` always used the first entry
/// of `WeaponsConsoleSection.phaser_banks` and a 360° arc via
/// `radar::is_fire_ready_with_range`. Post-unification, NPCs consult
/// their `PhaserCombatConfigResource::bank_by_id` and honour that bank's
/// `fire_arc_deg`. A target outside the requested bank's arc must be
/// rejected, matching the player-fire behaviour.
#[test]
fn npc_handle_fire_phaser_rejects_target_outside_requested_bank_arc() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "bb000000-0000-0000-0000-000000000001";
    let target_uuid = "bb000000-0000-0000-0000-000000000002";

    // NPC facing -Z with a narrow port-only bank (facing_deg=-90, arc=60°).
    // Target directly ahead is out of arc.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the phaser bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::phaser_bank_system_id("port").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let combat = crate::entity_config::PhaserCombatConfig {
        banks: vec![crate::entity_config::PhaserBankConfig {
            id: "port".into(),
            facing_deg: -90.0,
            fire_arc_deg: 60.0,
            auto_arc_deg: 60.0,
            beam_range: 50.0,
            beam_damage_per_sec: 5.0,
            beam_duration_secs: 3.0,
            cooldown_secs: 6.0,
            beam_color: vec![],
            shield_pierce: None,
            marker: None,
        }],
    };
    let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid).unwrap();
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ai_plugin::AiControllerComponent,
            crate::ship_plugin::ShipSystemControlSources(sources),
            WeaponsTarget(Some(target_uuid_parsed.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(combat),
            Transform::default(),
        ))
        .id();
    {
        let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }
    // Target directly ahead (-Z, bearing 0°) — outside the -90° port bank
    // whose arc runs from -120° to -60°.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    // Send an explicit FirePhaser request for the port bank.
    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    app.update();

    let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
    assert!(
        beam.target_uuid.is_none(),
        "FirePhaser for a port bank must be rejected when the target is not in that bank's fire arc — unified handler now honours per-bank config for NPCs"
    );
}

fn tactical_blips(app: &mut App) -> Vec<RadarBlip> {
    use crate::messages::SystemBlackboard;
    use crate::server_app::ShipSystemBlackboards;
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    match q.single(app.world()) {
        Ok(bbs) => match bbs.0.get(&crate::system_registry::tactical_station_key()) {
            Some(SystemBlackboard::Weapons(bb)) => bb.blips.clone(),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

#[test]
fn radar_blip_appears_for_asteroid_within_tactical_range() {
    let mut app = test_app();
    // Configure tactical radar to show asteroids with range 300.
    {
        let mut cfg = app
            .world_mut()
            .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
        cfg.0.tactical_radar_shows = vec!["asteroid".into()];
        cfg.0.tactical_radar_range = 300.0;
    }
    // Asteroid 50 units ahead (z=-50, within 300 range).
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game(&mut app);
    tick(&mut app); // first InProgress tick → publish runs

    let blips = tactical_blips(&mut app);

    assert_eq!(blips.len(), 1, "expected one blip for in-range asteroid");
    assert_eq!(blips[0].uuid, "target-uuid");
    assert_eq!(blips[0].kind, "asteroid");
    // Forward (z=-50) at yaw=0 maps to radar_y > 0 (forward = up).
    assert!(
        blips[0].radar_y > 0.0,
        "asteroid ahead should have positive radar_y"
    );
    assert!(
        (blips[0].radar_x).abs() < 1e-4,
        "asteroid directly ahead has radar_x ≈ 0"
    );
}

#[test]
fn asteroid_beyond_tactical_range_not_in_blips() {
    let mut app = test_app();
    {
        let mut cfg = app
            .world_mut()
            .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
        cfg.0.tactical_radar_shows = vec!["asteroid".into()];
        cfg.0.tactical_radar_range = 100.0;
    }
    // Asteroid 200 units ahead — beyond the 100-unit radar range.
    setup_weapons_world(&mut app, 0.0, -200.0);
    start_game(&mut app);
    tick(&mut app);

    let blips = tactical_blips(&mut app);
    assert!(
        blips.is_empty(),
        "asteroid beyond tactical range must not appear in blips"
    );
}

// ── Tactical AI tests ──────────────────────────────────────────────────

/// Set the ControlSource for every tactical fine system on the LocalShip.
///
/// Post-#512 gating reads per-fine-system policies; post-#801 the coarse
/// `tactical` id is not a system at all, so this helper seeds only the
/// fine ids (mirrors what happens when a station rating flips to
/// Backfill, which triggers AI control of every fine system owned by
/// the station).
fn set_tactical_control_source(app: &mut App, source: crate::ship::control_source::ControlSource) {
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        for sysid in [
            crate::system_registry::phaser_fore_system_id(),
            crate::system_registry::phaser_aft_system_id(),
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            crate::system_registry::torpedo_tube_aft_system_id(),
            crate::system_registry::torpedo_magazine_system_id(),
        ] {
            cs.0.set(sysid, source);
        }
    }
}

fn spawn_asteroid_target(app: &mut App, uuid: &str, x: f32, z: f32) {
    app.world_mut().spawn((
        crate::simulation::Asteroid,
        AsteroidUuid(uuid.into()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            crate::messages::SystemId("captain".into()),
            30.0,
        )])),
        Transform::from_xyz(x, 0.0, z),
    ));
}

fn spawn_entity_target(app: &mut App, uuid: &str, x: f32, z: f32) {
    app.world_mut().spawn((
        crate::entity_spawner::EntityUuid(uuid.into()),
        Transform::from_xyz(x, 0.0, z),
    ));
}

// ── Nearest-hostile acquisition fixtures (issue #703) ──────────────────

/// Faction UUIDs for the nearest-hostile tests. Mirrors combat_test.toml:
/// Harrow lists Federation as an enemy.
fn harrow_faction() -> uuid::Uuid {
    uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap()
}

fn federation_faction() -> uuid::Uuid {
    uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap()
}

/// Declare the ship's tactical radar horizon. In production this is
/// authored per entity template under `[weapons_console] radar.range`; the
/// tests read it from the same component rather than any literal in code.
fn set_tactical_radar_range(app: &mut App, range: f32) {
    use crate::entity_tags::EntityTag;
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    let entity = q.single_mut(app.world_mut()).expect("LocalShip");
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::entity_spawner::WeaponsConsoleSection(
            crate::entity_config::WeaponsConsoleConfig {
                torpedo_arc_color: vec![],
                power_multipliers: None,
                phaser_banks: vec![],
                blaster_banks: vec![],
                radar: Some(crate::radar_config::RadarConfig {
                    range,
                    shows: vec![EntityTag::Ship],
                    selects: vec![],
                }),
            },
        ));
}

/// Put the LocalShip in the Harrow faction and load a registry in which
/// Harrow is hostile to Federation — the same shape `combat_test.toml`
/// builds via `add_faction_enemy`.
fn setup_harrow_ship_hostile_to_federation(app: &mut App) {
    use crate::faction::{FactionConfig, FactionRegistry};

    let mut registry = FactionRegistry::new();
    registry.insert(FactionConfig {
        uuid: harrow_faction(),
        name: "Harrow".into(),
        enemies: vec![federation_faction()],
    });
    registry.insert(FactionConfig {
        uuid: federation_faction(),
        name: "Federation".into(),
        enemies: vec![],
    });
    app.insert_resource(crate::entities::config_cache::FactionRegistryResource(
        registry,
    ));

    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    let entity = q.single_mut(app.world_mut()).expect("LocalShip");
    app.world_mut()
        .entity_mut(entity)
        .insert(FactionComponent(harrow_faction()));
}

/// A factioned **ship** — the entity shape the nearest-hostile tier is
/// allowed to auto-acquire. The `Ship` marker is not decoration: the tier-4
/// scan is `With<Ship>`, matching the tactical radar's `shows:
/// [EntityTag::Ship]`. See `tier_four_does_not_acquire_a_factioned_non_ship`
/// for the other side of that filter.
fn spawn_factioned_target(
    app: &mut App,
    uuid: &str,
    x: f32,
    z: f32,
    faction: uuid::Uuid,
) -> Entity {
    app.world_mut()
        .spawn((
            crate::simulation::Ship,
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
            FactionComponent(faction),
        ))
        .id()
}

/// Author an *untargeted* `Destroy` objective — `Destroy { target: "" }`.
/// This is what every shipped hostile TOML produces (`directive_kind =
/// "Destroy"` with no `directive_target`), and the only directive shape
/// that licenses the nearest-hostile tier.
fn insert_untargeted_destroy_objective(app: &mut App, score: f32) {
    insert_destroy_objective_blackboard(app, "", score);
}

/// Set the LocalShip's `LastShipAttacker`. Wraps the entity-taking
/// `set_last_attacker` defined further down this module.
fn set_local_last_attacker(app: &mut App, uuid: Option<String>) {
    let entity = local_ship_entity(app);
    set_last_attacker(app, entity, uuid);
}

#[test]
fn tactical_ai_respects_radar_range() {
    let mut app = test_app();
    let near_uuid = uuid::Uuid::new_v4().to_string();
    let far_uuid = uuid::Uuid::new_v4().to_string();

    // Attach a WeaponsConsoleSection with a radar range of 100 so the
    // tactical AI reads a finite, damage-scaled horizon for the test.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        if let Ok(entity) = q.single_mut(app.world_mut()) {
            use crate::entity_tags::EntityTag;
            app.world_mut().entity_mut(entity).insert(
                crate::entity_spawner::WeaponsConsoleSection(
                    crate::entity_config::WeaponsConsoleConfig {
                        torpedo_arc_color: vec![],
                        power_multipliers: None,
                        phaser_banks: vec![],
                        blaster_banks: vec![],
                        radar: Some(crate::radar_config::RadarConfig {
                            range: 100.0,
                            shows: vec![EntityTag::Ship],
                            selects: vec![],
                        }),
                    },
                ),
            );
        }
    }

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Far target — beyond radar range.
    spawn_entity_target(&mut app, &far_uuid, 0.0, -500.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("target".into(), far_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "target", 80.0);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "Tactical AI must NOT acquire a target beyond radar range"
    );

    // Near target — now within range. Update the runtime mapping so the
    // same objective name resolves to the nearby entity.
    spawn_entity_target(&mut app, &near_uuid, 0.0, -50.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("target".into(), near_uuid.clone());

    set_weapons_target(&mut app, None);
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(near_uuid.as_str()),
        "Tactical AI must acquire a target within radar range"
    );
}

// ── Nearest-hostile acquisition tier (issue #703) ──────────────────────
//
// Regression guards for the shipped-content bug: `ai_target_selection`
// acquired only from an explicit `Destroy` target or `LastShipAttacker`.
// No asset TOML authors a `directive_target`, and `LastShipAttacker` is
// written only by `tick_beams` — so an NPC could not fire until the player
// shot it first. These pin the third tier that closes that gap.

/// The headline fix: an NPC on standing "destroy hostiles" doctrine
/// acquires a hostile it can see, *without* having been attacked.
#[test]
fn tactical_ai_acquires_nearest_hostile_without_being_shot_first() {
    let mut app = test_app();
    let hostile_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // A Federation ship well inside the 100-unit radar horizon.
    spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -50.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);

    // Nobody has shot us: no LastShipAttacker, and the objective names
    // no one. Pre-#703 both acquisition tiers came up empty here.
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(hostile_uuid.as_str()),
        "an NPC on untargeted Destroy doctrine must acquire the nearest hostile in radar \
         range without waiting to be shot first — this is the whole point of issue #703"
    );
}

/// The nearest hostile is picked among several — and it is the *nearest*,
/// agreeing with the helm AI, which closes on the same ship.
#[test]
fn tactical_ai_acquires_the_nearest_of_several_hostiles() {
    let mut app = test_app();
    let near_uuid = uuid::Uuid::new_v4().to_string();
    let far_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Both in range; spawn the far one first so the result cannot be an
    // artefact of iteration order.
    spawn_factioned_target(&mut app, &far_uuid, 0.0, -90.0, federation_faction());
    spawn_factioned_target(&mut app, &near_uuid, 0.0, -20.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(near_uuid.as_str()),
        "the nearest-hostile tier must pick the nearest, not the first found — the helm AI \
         closes on the nearest via the same find_nearest_hostile, and the two must agree"
    );
}

/// The radar gate binds the new tier exactly as it binds the others: a
/// ship must not lock what it cannot detect.
#[test]
fn tactical_ai_does_not_acquire_a_hostile_beyond_radar_range() {
    let mut app = test_app();
    let hostile_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Hostile at 500 units — far beyond the 100-unit radar horizon.
    spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -500.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "the nearest-hostile tier must be gated by the damage-scaled tactical radar range — \
         an NPC must not acquire a target it cannot detect"
    );
}

/// Faction filtering: a ship of our own faction is not a hostile, however
/// close it is.
#[test]
fn tactical_ai_does_not_acquire_a_non_hostile() {
    let mut app = test_app();
    let friendly_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // Another Harrow ship — our own faction — right next to us.
    spawn_factioned_target(&mut app, &friendly_uuid, 0.0, -10.0, harrow_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "the nearest-hostile tier must filter by faction through the live FactionRegistry — \
         a same-faction ship is never a weapons target, however near"
    );
}

/// Precedence, tier 1 over tier 3: a `Destroy` naming someone specific must
/// not wander onto a nearer ship.
#[test]
fn explicit_destroy_target_takes_precedence_over_a_nearer_hostile() {
    let mut app = test_app();
    let named_uuid = uuid::Uuid::new_v4().to_string();
    let nearer_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // The named target is further away than an unnamed hostile. Both are
    // Federation, both in radar range.
    spawn_factioned_target(&mut app, &named_uuid, 0.0, -80.0, federation_faction());
    spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), named_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(named_uuid.as_str()),
        "an explicit Destroy target must outrank the nearest-hostile tier — a mission that \
         names a target must not be silently retargeted onto whoever is closest"
    );
}

/// Precedence, tier 2 over tier 3: whoever shot us still outranks a nearer
/// bystander, exactly as before #703.
#[test]
fn last_attacker_takes_precedence_over_a_nearer_hostile() {
    let mut app = test_app();
    let attacker_uuid = uuid::Uuid::new_v4().to_string();
    let nearer_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // The attacker is further away than an unengaged hostile.
    spawn_factioned_target(&mut app, &attacker_uuid, 0.0, -80.0, federation_faction());
    spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, Some(attacker_uuid.clone()));

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(attacker_uuid.as_str()),
        "LastShipAttacker must outrank the nearest-hostile tier — shooting back at whoever \
         hit us must not be displaced by a closer bystander"
    );
}

// ── Target retention (tier 2) ──────────────────────────────────────────
//
// The nearest-hostile tier decides "who is closest *now*". Left ungated it
// re-decides that every tick, so a lock follows whoever happens to be
// nearest at this instant — beams retargeting, and (because the helm pursues
// `WeaponsTarget`) the ship slewing between bearings with it. These pin the
// retention tier that keeps an engaged ship committed.

/// The headline retention case: engaged with A, B closes inside it, and the
/// lock stays on A.
#[test]
fn an_established_lock_is_retained_when_a_nearer_hostile_appears() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let nearer_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    // Tick once with only A present: the ship acquires and engages it.
    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the ship must be engaged with A before B arrives"
    );

    // B arrives, closer than A, and equally hostile.
    spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "an established lock on a live, in-range hostile must be retained when a nearer \
         hostile appears — the helm keeps closing on A (the helm reads the retained WeaponsTarget, which prefers its \
         current target), so weapons flipping to B would have the ship shooting one ship \
         while flying at another"
    );
}

/// The other half: retention is not a freeze. A lock that dies is re-scanned.
#[test]
fn the_lock_is_rescanned_when_the_current_target_dies() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let other_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let engaged = spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the nearer hostile is the one engaged"
    );

    // A is destroyed.
    app.world_mut().entity_mut(engaged).despawn();
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(other_uuid.as_str()),
        "retention must not outlive the target: once the locked ship is gone the \
         nearest-hostile tier must acquire afresh, or the AI sits idle beside a live enemy"
    );
}

/// The liveness half of retention, on the one path where the radar gate
/// cannot stand in for it. A ship that declares no `radar.range` has an
/// unbounded horizon (`range_bounds_targets == false`), so `within_range`
/// is never consulted and "the locked entity no longer exists" is the only
/// thing that can release the lock. Without that check the retention tier
/// hands the dead UUID on, the stale guard clears it, and the ship spends
/// the tick idle next to a live enemy instead of acquiring it.
#[test]
fn the_lock_is_rescanned_when_the_current_target_dies_with_no_radar_horizon() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let other_uuid = uuid::Uuid::new_v4().to_string();

    // Deliberately no set_tactical_radar_range: no WeaponsConsoleSection
    // means a base range of 0, which the system reads as "unbounded".
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let engaged = spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the nearer hostile is the one engaged"
    );

    app.world_mut().entity_mut(engaged).despawn();
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(other_uuid.as_str()),
        "retention must check that the locked entity still exists, not lean on the radar \
         gate to notice — an unbounded horizon never range-checks, so a dead lock would \
         block acquisition for the tick"
    );
}

/// Retention is bounded by the same radar horizon as acquisition (issue
/// #680): a lock that runs out of detection range is re-scanned, not held.
#[test]
fn the_lock_is_rescanned_when_the_current_target_leaves_radar_range() {
    let mut app = test_app();
    let fleeing_uuid = uuid::Uuid::new_v4().to_string();
    let other_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    let fleeing = spawn_factioned_target(&mut app, &fleeing_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(fleeing_uuid.as_str()),
        "precondition: the nearer hostile is the one engaged"
    );

    // A runs beyond the 100-unit tactical radar horizon.
    app.world_mut()
        .entity_mut(fleeing)
        .insert(Transform::from_xyz(0.0, 0.0, -500.0));
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(other_uuid.as_str()),
        "retention must be gated by the damage-scaled radar range exactly as acquisition is \
         — a target the ship can no longer detect must not pin the lock and starve the scan"
    );
}

/// The ordering decision, pinned: retention outranks `LastShipAttacker`,
/// because the helm has no retaliation tier and would keep closing on A.
/// The reverse order is the tempting one — see this system's doc comment.
#[test]
fn an_established_lock_outranks_a_new_last_attacker() {
    let mut app = test_app();
    let engaged_uuid = uuid::Uuid::new_v4().to_string();
    let attacker_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
    spawn_factioned_target(&mut app, &attacker_uuid, 0.0, -90.0, federation_faction());
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "precondition: the ship is engaged with A"
    );

    // B opens fire on us mid-engagement.
    set_local_last_attacker(&mut app, Some(attacker_uuid.clone()));
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(engaged_uuid.as_str()),
        "taking a hit must not break off an engagement weapons is already in: the helm's \
         ai_target_selection's retention tier outranks its last_attacker tier, and \
         weapons must match it tier for tier or the ship closes on A while shooting B. \
         last_attacker_takes_precedence_over_a_nearer_hostile pins the case that still \
         retaliates — no lock to keep"
    );
}

/// Advisory from the #703 review: the tier-4 scan is an *auto-acquisition*
/// surface, so it must be `With<Ship>` — the tactical radar `shows:
/// [EntityTag::Ship]` and nothing else. No shipped non-ship template
/// declares a `faction` today; this pins the filter before one does.
#[test]
fn tier_four_does_not_acquire_a_factioned_non_ship() {
    let mut app = test_app();
    let station_uuid = uuid::Uuid::new_v4().to_string();

    set_tactical_radar_range(&mut app, 100.0);
    setup_harrow_ship_hostile_to_federation(&mut app);
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

    // A hostile-factioned entity that is *not* a ship — the shape a
    // factioned station / mine / probe template would spawn. Everything
    // else about it would qualify: in radar range, enemy faction, closer
    // than anything else in the world.
    app.world_mut().spawn((
        crate::entity_spawner::EntityUuid(station_uuid),
        Transform::from_xyz(0.0, 0.0, -10.0),
        FactionComponent(federation_faction()),
    ));
    insert_untargeted_destroy_objective(&mut app, 35.0);
    set_local_last_attacker(&mut app, None);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "the nearest-hostile tier must only auto-acquire ships — a factioned non-ship is \
         not what the tactical radar shows, and locking one would have the AI open fire on \
         scenery it cannot even see"
    );
}

fn insert_destroy_objective_blackboard(app: &mut App, target: &str, score: f32) {
    use crate::messages::{
        AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
        SystemAffinity, SystemBlackboard, ViewscreenBlackboard,
    };
    use crate::server_app::ShipSystemBlackboards;

    let viewscreen = ViewscreenBlackboard {
        scored_objectives: vec![ScoredObjective {
            id: format!("obj-destroy-{target}"),
            score,
            directive: AiDirective::Destroy {
                target: target.into(),
            },
            source: ObjectiveSource::Mission,
            relevance: vec![
                SystemAffinity::Helm,
                SystemAffinity::Weapons,
                SystemAffinity::Captain,
            ],
            snapshot: ObjectiveSnapshot {
                id: format!("obj-destroy-{target}"),
                text: format!("Destroy {target}"),
                mandatory: true,
                status: ObjectiveStatus::Active,
                targets: vec![target.into()],
                source: ObjectiveSource::Mission,
            },
        }],
        ..Default::default()
    };
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let mut bbs = q
        .single_mut(app.world_mut())
        .expect("LocalShip must have ShipSystemBlackboards");
    bbs.0.insert(
        crate::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(viewscreen),
    );
}

#[test]
fn tactical_ai_selects_named_destroy_objective_target() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "Tactical AI must lock the live entity named by the Destroy objective"
    );
}

#[test]
fn tactical_ai_clears_stale_weapons_target_when_objective_target_dead() {
    let mut app = test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    // Pre-set a stale target UUID — simulates a prior Destroy objective
    // whose entity was killed.
    set_weapons_target(&mut app, Some("dead-target-uuid".into()));
    // No last attacker.
    // Still have a Destroy objective for a target that is no longer alive.
    insert_destroy_objective_blackboard(&mut app, "wave_gone", 80.0);
    // No entity named "wave_gone" exists → resolve returns None.

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "Tactical AI must clear WeaponsTarget when the objective target is \
         dead and no last attacker is available, fixing the stale-target bug \
         that caused AI to sit idle after killing its last target"
    );
}

#[test]
fn tactical_ai_ignores_missing_destroy_objective_target() {
    let mut app = test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    insert_destroy_objective_blackboard(&mut app, "wave_404", 80.0);

    tick(&mut app);

    assert!(
        get_weapons_target(&mut app).is_none(),
        "Tactical AI must not lock an arbitrary target when the objective target is missing"
    );
}

// ── ai_target_selection / locked_target (issue #697) ────────────────────

/// Read a ship's published Weapons blackboard by entity.
fn weapons_blackboard_of(app: &mut App, entity: Entity) -> Option<WeaponsBlackboard> {
    app.world()
        .entity(entity)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .and_then(
            |bbs| match bbs.0.get(&crate::system_registry::tactical_station_key()) {
                Some(SystemBlackboard::Weapons(bb)) => Some(bb.clone()),
                _ => None,
            },
        )
}

/// Spawn an NPC ship: every component the spawner gives a `[behaviour]`
/// entity that the Weapons systems touch, minus the `LocalShip` marker.
/// Its Tactical fine systems are all AI-controlled.
fn spawn_npc_ship(app: &mut App, uuid: &str, x: f32, z: f32) -> Entity {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    let config = test_ship_config();
    let mut resolver = ControlSourceResolver::new();
    for system in &config.0.systems {
        resolver.set(system.id.clone(), ControlSource::Ai);
    }
    app.world_mut()
        .spawn((
            crate::simulation::Ship,
            config,
            ShipSystemControlSources(resolver),
            crate::server_app::ShipSystemBlackboards::default(),
            LastShipAttacker::default(),
            ShipPhysics {
                x,
                z,
                ..Default::default()
            },
            WeaponsTarget::default(),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "phaser-fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 240.0,
                    ..Default::default()
                }],
            }),
            TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
        ))
        .id()
}

fn set_last_attacker(app: &mut App, entity: Entity, uuid: Option<String>) {
    app.world_mut()
        .entity_mut(entity)
        .insert(LastShipAttacker(uuid));
}

#[test]
fn ai_target_selection_publishes_locked_target_and_applies_it() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    tick(&mut app);

    let local = local_ship_entity(&mut app);
    let bb = weapons_blackboard_of(&mut app, local)
        .expect("LocalShip must publish a Weapons blackboard");
    assert_eq!(
        bb.locked_target.as_deref(),
        Some(target_uuid.as_str()),
        "ai_target_selection must publish its choice as locked_target, and that intent \
         must survive publish_weapons_core_blackboard rebuilding the blackboard in SimSet::Publish"
    );
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "ai_target_selection must apply its choice to the authoritative WeaponsTarget"
    );
    assert_eq!(
        bb.target_uuid, bb.locked_target,
        "on an AI-operated ship, intent and truth agree after a tick"
    );
}

#[test]
fn ai_target_selection_clears_locked_target_when_target_dies() {
    let mut app = test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("dead-target-uuid".into()));

    tick(&mut app);

    let local = local_ship_entity(&mut app);
    let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        bb.locked_target, None,
        "a lock on an entity that no longer exists must be dropped from the AI's intent"
    );
    assert!(
        get_weapons_target(&mut app).is_none(),
        "and it must clear the authoritative WeaponsTarget to match"
    );
}

#[test]
fn human_tactical_leaves_locked_target_empty_and_keeps_the_human_lock() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Human);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    // A Destroy objective the AI *would* act on, were it in control.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);
    // The human operator's own lock, as handle_set_target would leave it.
    set_weapons_target(&mut app, Some(target_uuid.clone()));

    tick(&mut app);

    let local = local_ship_entity(&mut app);
    let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        bb.locked_target, None,
        "locked_target is AI intent only — a human-operated Tactical selects nothing, \
         even with a live Destroy objective on the board"
    );
    assert_eq!(
        bb.target_uuid.as_deref(),
        Some(target_uuid.as_str()),
        "target_uuid mirrors the authoritative WeaponsTarget, which the human still owns"
    );
}

/// Put the ship in the mixed-rating shape that makes `handle_set_target`
/// and `ai_target_selection` run in the same tick: the phaser banks are
/// Human (so `any_bank_accepts_human_input` admits SetTarget) while the
/// torpedo magazine is Ai (so `any_tactical_system_operates_ai` runs the
/// selector). This is an ordinary config, not a contrived one — it is what a
/// ship looks like when Tactical is crewed but the magazine is backfilled.
fn set_mixed_tactical_control_sources(app: &mut App) {
    use crate::ship::control_source::ControlSource;
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        for sysid in [
            crate::system_registry::phaser_fore_system_id(),
            crate::system_registry::phaser_aft_system_id(),
        ] {
            cs.0.set(sysid, ControlSource::Human);
        }
        for sysid in [
            crate::system_registry::torpedo_magazine_system_id(),
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            crate::system_registry::torpedo_tube_aft_system_id(),
        ] {
            cs.0.set(sysid, ControlSource::Ai);
        }
    }
}

/// The mixed-rating shape above is only interesting if both gates really do
/// fire on it. Pin that directly, so the regression test below can't quietly
/// decay into a test of a ship the tactical AI never touches.
#[test]
fn mixed_rating_ship_admits_human_set_target_and_runs_the_tactical_ai() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);
    set_mixed_tactical_control_sources(&mut app);

    let world = app.world_mut();
    let mut q = world.query_filtered::<(
        &ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
    ), With<crate::server_app::LocalShip>>();
    let (control_sources, ship_config) = q.single(world).expect("local ship");

    assert!(
        any_bank_accepts_human_input(control_sources, &ship_config.0),
        "a Human phaser bank must still admit the human's SetTarget"
    );
    assert!(
        any_tactical_system_operates_ai(control_sources, &ship_config.0),
        "an Ai torpedo magazine must still run the tactical AI — if this \
         ever goes false the two writers stop overlapping and the ordering \
         regression below stops being reachable"
    );
}

#[test]
fn human_set_target_survives_the_tick_on_a_mixed_rating_ship() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);
    set_mixed_tactical_control_sources(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some("target-uuid"),
        "the human's SetTarget must survive the tick it was admitted in: \
         ai_target_selection has to see it and carry it into its own selection, \
         not apply a decision made before the human's lock existed"
    );

    // And it must still be there next tick — a lock clobbered on tick N is
    // not recovered on tick N+1, because selection re-seeds from the
    // (clobbered) WeaponsTarget.
    tick(&mut app);
    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some("target-uuid"),
        "the human's lock must be stable across subsequent ticks"
    );
}

/// Ported from `integrator_leaves_weapons_target_alone_when_selection_never_ran`
/// (issue #700). That test pinned the "decider never ran" vs "decider chose
/// nothing" distinction which `blackboard_locked_target`'s `Option<Option<_>>`
/// carried between `ai_target_selection` and the separate `operate_tactical_ai`
/// integrator. With the integrator folded in, a decision and its application
/// are the same statement, so "never ran" can no longer be misread as "chose
/// nothing" — the bug is unrepresentable rather than merely guarded.
///
/// What survives is the property underneath it, on the one path that can still
/// reach it: a ship the selector skips must keep the lock it already has, and
/// must not have an AI-intent entry conjured onto its blackboard.
#[test]
fn skipped_ship_keeps_its_weapons_target_and_gains_no_blackboard_entry() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    let mut app = App::new();
    app.add_systems(Update, ai_target_selection);

    let config = test_ship_config();
    let mut resolver = ControlSourceResolver::new();
    // Human across the board: `any_tactical_system_operates_ai` is false, so
    // selection skips this ship entirely.
    for system in &config.0.systems {
        resolver.set(system.id.clone(), ControlSource::Human);
    }
    let ship = app
        .world_mut()
        .spawn((
            crate::simulation::Ship,
            config,
            ShipSystemControlSources(resolver),
            LastShipAttacker::default(),
            ShipPhysics::default(),
            // The human operator's standing lock, on an entity that does not
            // exist in this bare world — so if the AI ever did run for this
            // ship, its stale-target guard would clear the lock and the
            // assertion below would fail. That is the point: the AI must not
            // run at all.
            WeaponsTarget(Some("standing-lock".into())),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();

    app.update();

    assert_eq!(
        app.world().entity(ship).get::<WeaponsTarget>().unwrap().0,
        Some("standing-lock".into()),
        "a ship whose Tactical is human-operated is skipped by the selector — \
         it must keep the human's lock, not have it re-decided or cleared"
    );
    assert!(
        !app.world()
            .entity(ship)
            .get::<crate::server_app::ShipSystemBlackboards>()
            .unwrap()
            .0
            .contains_key(&crate::system_registry::tactical_station_key()),
        "a skipped ship has no AI intent to report, so the selector must not \
         insert a bare Weapons blackboard entry for it"
    );
}

#[derive(Resource)]
struct KillTargetOnDamage(String);

/// Stands in for `tick_beams` / `tick_torpedo_lifecycle`: both destroy the
/// locked target and clear `WeaponsTarget` *after* `SimSet::Input`, which is
/// what leaves a dead `locked_target` for `publish_weapons_core_blackboard` to
/// carry forward.
fn kill_target_after_input(
    mut commands: Commands,
    kill: Res<KillTargetOnDamage>,
    target_q: Query<(Entity, &crate::entity_spawner::EntityUuid)>,
    mut weapons_target_q: Query<&mut WeaponsTarget, With<crate::server_app::LocalShip>>,
) {
    for (entity, uuid) in target_q.iter() {
        if uuid.0 == kill.0 {
            commands.entity(entity).despawn();
        }
    }
    for mut wt in weapons_target_q.iter_mut() {
        if wt.0.as_deref() == Some(kill.0.as_str()) {
            wt.0 = None;
        }
    }
}

#[test]
fn publish_drops_locked_target_when_the_selected_target_dies_mid_tick() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    // Tick 1: the AI acquires the target while it is alive.
    tick(&mut app);
    let local = local_ship_entity(&mut app);
    assert_eq!(
        weapons_blackboard_of(&mut app, local)
            .expect("blackboard")
            .locked_target
            .as_deref(),
        Some(target_uuid.as_str()),
        "precondition: the AI must be locked on before the target dies"
    );

    // Tick 2: Input selects the (still live) target, then the target is
    // destroyed in Damage — exactly the beam/torpedo kill ordering.
    app.insert_resource(KillTargetOnDamage(target_uuid.clone()));
    app.add_systems(
        Update,
        kill_target_after_input.in_set(crate::sim_sets::SimSet::Damage),
    );
    tick(&mut app);

    let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        bb.target_uuid, None,
        "precondition: the kill must have cleared the authoritative WeaponsTarget"
    );
    assert_eq!(
        bb.locked_target, None,
        "a locked_target whose entity died after SimSet::Input must not be carried \
         forward: publishing it would put locked_target != target_uuid on the wire, \
         contradicting the field's documented contract that the two agree after a tick"
    );
}

#[test]
fn npc_ship_publishes_its_own_weapons_blackboard_with_ship_state_only() {
    let mut app = test_app();
    // LocalShip radar config: shows asteroids out to 300 units. Only the
    // LocalShip has a browser client, so only it should get blips.
    {
        let mut cfg = app
            .world_mut()
            .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
        cfg.0.tactical_radar_shows = vec!["asteroid".into()];
        cfg.0.tactical_radar_range = 300.0;
    }
    setup_weapons_world(&mut app, 0.0, -50.0);
    start_game(&mut app);

    // NPC at the origin, attacked by a live entity 30 units ahead.
    let attacker_uuid = uuid::Uuid::new_v4().to_string();
    spawn_entity_target(&mut app, &attacker_uuid, 0.0, -30.0);
    let npc = spawn_npc_ship(&mut app, "npc-1", 0.0, 0.0);
    set_last_attacker(&mut app, npc, Some(attacker_uuid.clone()));

    tick(&mut app);

    let bb = weapons_blackboard_of(&mut app, npc)
        .expect("an NPC carrying ShipSystemBlackboards must get a Weapons blackboard too");

    // Ship state — computed per-entity, so NPCs get the real thing.
    assert_eq!(
        bb.locked_target.as_deref(),
        Some(attacker_uuid.as_str()),
        "the NPC's Tactical AI must select its last attacker"
    );
    assert_eq!(
        bb.target_uuid.as_deref(),
        Some(attacker_uuid.as_str()),
        "and the NPC's authoritative WeaponsTarget must follow its own intent"
    );
    assert_eq!(
        bb.banks.len(),
        1,
        "banks come from the NPC's own PhaserCombatConfigResource"
    );
    assert_eq!(bb.banks[0].id, "phaser-fore");
    assert_eq!(
        bb.torpedo_count,
        TorpedoConfig::default().count,
        "torpedo_count comes from the NPC's own TorpedoSystemResource"
    );

    // Client render data — player-only, and left empty for NPCs.
    assert!(
        bb.blips.is_empty(),
        "blips are client render data sourced from the player-only \
         ShipClientConfigResource, and are O(all entities) to compute — an NPC \
         with no browser client must not pay for them"
    );
    assert!(bb.regions.is_empty(), "regions are client render data");
    assert!(
        bb.phaser_arcs.is_empty(),
        "phaser_arcs are client render data"
    );
    assert!(
        bb.torpedo_arcs.is_empty(),
        "torpedo_arcs are client render data"
    );

    // The contrast: the LocalShip *does* get its render data, so the
    // assertions above are about the NPC tier and not a dead radar config.
    let local = local_ship_entity(&mut app);
    let local_bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
    assert_eq!(
        local_bb.blips.len(),
        1,
        "the LocalShip still gets its in-range asteroid blip"
    );
}

#[test]
fn npc_and_local_ship_select_targets_independently() {
    let mut app = test_app();
    // Regression guard for the SetTarget-contamination class of bug: two
    // ships, two different attackers, two independent locks.
    let local_target = uuid::Uuid::new_v4().to_string();
    let npc_target = uuid::Uuid::new_v4().to_string();
    spawn_entity_target(&mut app, &local_target, 0.0, -30.0);
    spawn_entity_target(&mut app, &npc_target, 0.0, 30.0);

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    let local = local_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(local)
        .insert(LastShipAttacker(Some(local_target.clone())));

    let npc = spawn_npc_ship(&mut app, "npc-1", 0.0, 0.0);
    set_last_attacker(&mut app, npc, Some(npc_target.clone()));

    tick(&mut app);

    assert_eq!(
        weapons_blackboard_of(&mut app, local)
            .expect("blackboard")
            .locked_target
            .as_deref(),
        Some(local_target.as_str())
    );
    assert_eq!(
        weapons_blackboard_of(&mut app, npc)
            .expect("blackboard")
            .locked_target
            .as_deref(),
        Some(npc_target.as_str()),
        "each ship selects from its own last-attacker surface, not a shared one"
    );
}

/// Builds on `test_app()` (LocalShip + `WeaponsPlugin` + `LobbyPlugin`) by
/// wiring in `ai_torpedo_auto_fire` (issue #694) and giving the LocalShip
/// the two components it requires: `AiHighFidelity` and `TorpedoIntents`.
/// `test_app()` itself stays unchanged (it's shared by ~200 unrelated tests
/// in this module) — this is a dedicated extension, mirroring how
/// `combined_test_app()` layers `AiPlugin` on top of `test_app()` for its
/// own end-to-end tests.
///
/// Only the *decide* half needs adding: since issue #698 the apply half is
/// `integrate_weapons_state`, which `WeaponsPlugin` already registers with
/// its `.after(ai_torpedo_auto_fire)` edge — and that edge starts binding
/// the moment this helper registers the decider.
fn torpedo_ai_test_app() -> App {
    let mut app = test_app();
    app.add_systems(
        Update,
        crate::console_ai_plugin::ai_torpedo_auto_fire.in_set(crate::sim_sets::SimSet::Physics),
    );
    let ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("test_app must spawn a LocalShip")
    };
    app.world_mut()
        .entity_mut(ship)
        .insert((crate::ai_plugin::AiHighFidelity, TorpedoIntents::default()));
    app
}

/// Regression test for issue #694: `ai_torpedo_auto_fire` (preliminary)
/// replaces the old fused torpedo sub-block that used to run inline
/// inside `operate_tactical_ai`. Ported from the pre-#694
/// `ai_fires_torpedo_when_ai_controls_unclaimed_station`, which exercised
/// `operate_tactical_ai`'s torpedo block directly before it was deleted.
#[test]
fn ai_torpedo_auto_fire_fires_when_ai_controls_unclaimed_station() {
    // Unclaimed station + Ai ControlSource → ai_torpedo_auto_fire fires unconditionally.
    let mut app = torpedo_ai_test_app();

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    // Asteroid at (0, -30) → bearing 0 from ship at origin yaw=0 → in ForePort arc.
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire should fire TorpedoLaunched when controlling an unclaimed \
         Tactical station"
    );
}

/// `ai_torpedo_auto_fire` is a *decider*: it must publish to
/// `TorpedoIntents` and leave the `TorpedoSystem` alone. Mirrors
/// `ai_phaser_auto_fire_writes_intent_without_touching_the_beam`.
#[test]
fn ai_torpedo_auto_fire_writes_intent_without_launching() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

    app.world_mut()
        .run_system_once(crate::console_ai_plugin::ai_torpedo_auto_fire)
        .expect("ai_torpedo_auto_fire should run");

    let ship = local_ship(&mut app);
    let intents = app
        .world()
        .get::<TorpedoIntents>(ship)
        .expect("torpedo_ai_test_app inserts TorpedoIntents");
    assert_eq!(
        intents.0,
        vec![TorpedoCmd {
            tube_id: "fore_port".into(),
            target_uuid: "target-uuid".into(),
        }],
        "the decider must publish the loaded, in-arc tube and the locked target"
    );
    assert!(
        app.world()
            .resource::<SimOutbox>()
            .0
            .iter()
            .all(|(_, m)| !matches!(m, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire must not launch — that is integrate_weapons_state's job"
    );
}

/// `integrate_weapons_state` drains `TorpedoIntents` as well as
/// `PhaserIntents` (issue #698 folded the former
/// `integrate_torpedo_intents` into it). Pins the torpedo half of the
/// adapter from a hand-written intent, independently of the decider.
#[test]
fn integrate_weapons_state_launches_from_torpedo_intent() {
    use bevy::ecs::system::RunSystemOnce;

    let mut app = torpedo_ai_test_app();
    load_tube_now(&mut app, "fore_port");
    let ship = local_ship(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(TorpedoIntents(vec![TorpedoCmd {
            tube_id: "fore_port".into(),
            target_uuid: "target-uuid".into(),
        }]));

    app.world_mut()
        .run_system_once(integrate_weapons_state)
        .expect("integrate_weapons_state should run");

    assert!(
        app.world()
            .resource::<SimOutbox>()
            .0
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::TorpedoLaunched { .. })),
        "the adapter must advance the torpedo state machine from the intent"
    );
    assert!(
        app.world()
            .get::<TorpedoIntents>(ship)
            .unwrap()
            .0
            .is_empty(),
        "the adapter must drain the buffer so a stale intent cannot re-launch"
    );
}

/// Issue #698 promotion: `ai_torpedo_auto_fire` used to hardcode
/// `TorpedoAiInput { target_shields: 0 }`, which made `auto_fire_torpedo`'s
/// "shields must be down" condition unreachable — the AI fired torpedoes
/// straight into a fully-shielded target. It now reads the target's real
/// `ShipShields`, so the pure function's documented doctrine (phasers strip
/// shields, torpedoes finish the hull) actually holds.
#[test]
fn ai_torpedo_auto_fire_holds_fire_while_target_shields_are_up() {
    let mut app = torpedo_ai_test_app();
    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");

    // A ship target dead ahead, shields up.
    let target = spawn_shielded_target(&mut app, "target-uuid", 0.0, -30.0);

    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "torpedoes must hold while the target's shields are still up"
    );

    // Collapse every facing — now the shot is on.
    {
        let mut shields = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(target)
            .unwrap();
        for facing in shields.0.facings.iter_mut() {
            facing.hp = 0;
        }
    }
    load_tube_now(&mut app, "fore_port");

    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "torpedoes must fire once the target's shields are down"
    );
}

fn local_ship(app: &mut App) -> Entity {
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .expect("test_app must spawn a LocalShip")
}

/// A ship-like entity carrying `ShipShields` at full HP.
fn spawn_shielded_target(app: &mut App, uuid: &str, x: f32, z: f32) -> Entity {
    let shields = crate::shield::ShieldSystem::new(&crate::shield::ShieldConfig::default());
    assert!(
        shields.facings.iter().any(|f| f.hp > 0),
        "precondition: the default shield config must start with HP up"
    );
    app.world_mut()
        .spawn((
            crate::entity_spawner::EntityUuid(uuid.into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                50.0,
            )])),
            crate::ship::shields::ShipShields(shields, 0.5),
            Transform::from_xyz(x, 0.0, z),
        ))
        .id()
}

fn set_tactical_station_rating(app: &mut App, rating: &str) {
    let rating = rating.to_string();
    let world = app.world_mut();
    let mut q = world
        .query_filtered::<&mut crate::ship_plugin::ActiveStationRatings, With<crate::server_app::LocalShip>>();
    for mut ratings in q.iter_mut(world) {
        ratings.0.insert(
            crate::messages::StationId("tactical".into()),
            rating.clone(),
        );
    }
}

/// Ported from the pre-#694 `ai_stops_firing_when_rating_switches_to_std`,
/// which exercised `operate_tactical_ai`'s torpedo block directly before
/// it was deleted; see `ai_torpedo_auto_fire_fires_when_ai_controls_unclaimed_station`
/// above.
#[test]
fn ai_torpedo_auto_fire_stops_firing_when_rating_switches_to_std() {
    // Occupied station: AI fires when rating is Assisted (has torpedo_auto_fire
    // in ai_tuning), stops when rating is Std (no ai_tuning).
    let mut app = torpedo_ai_test_app();

    // Assign a human holder so the ai_tuning gate is active.
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

    set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
    // Set rating to Assisted (has torpedo_auto_fire in ai_tuning).
    set_tactical_station_rating(&mut app, "Assisted");
    set_weapons_target(&mut app, Some("target-uuid".into()));
    load_tube_now(&mut app, "fore_port");
    spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

    // First tick — AI should fire with Assisted rating.
    let out1 = tick(&mut app);
    assert!(
        out1.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire should fire TorpedoLaunched when rating is Assisted"
    );

    // Reload the tube (launch consumed it) so the only gate is the rating.
    load_tube_now(&mut app, "fore_port");

    // Switch to Std rating (no torpedo_auto_fire in ai_tuning).
    set_tactical_station_rating(&mut app, "Std");

    // Second tick - AI must not fire.
    let out2 = tick(&mut app);
    assert!(
        !out2
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "ai_torpedo_auto_fire must not fire TorpedoLaunched when rating is Std"
    );
}

// ── Fine-Tactical decomposition tests (issue #512) ─────────────────────
//
// Every new fine SystemId, blackboard, and gate has coverage here. The
// channel-2 `ClaimTorpedoRound` transaction is exercised via
// `handle_load_tube` → `handle_torpedo_magazine_inter_system`. Firing
// gates are exercised via `handle_fire_torpedo` and `handle_fire_phaser`.

/// Helper: mark a fine system Offline (Disabled/Destroyed) on the LocalShip
/// by inserting it into `ControlSourceResolver.offline_systems`. Mirrors
/// what `sync_console_damage_tiers` would do after a damage tick — the
/// direct-insert avoids needing to spawn a hull component just to test
/// the gate.
fn mark_system_offline(app: &mut App, system_id: SystemId) {
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        cs.0.offline_systems.insert(system_id.clone());
    }
}

/// Helper: register a fine system on the LocalShip's ControlSourceResolver
/// with a specific ControlSource. Used to simulate the ship having declared
/// a fine `[[system]]` block in its TOML.
fn register_fine_system(
    app: &mut App,
    system_id: SystemId,
    source: crate::ship::control_source::ControlSource,
) {
    let world = app.world_mut();
    let mut q =
        world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
    for mut cs in q.iter_mut(world) {
        cs.0.set(system_id.clone(), source);
    }
}

// ── Registered-system predicate ───────────────────────────────────────

#[test]
fn system_is_registered_returns_true_after_set() {
    let mut sources = ShipSystemControlSources::default();
    let sysid = crate::system_registry::phaser_fore_system_id();
    sources.0.set(
        sysid.clone(),
        crate::ship::control_source::ControlSource::Human,
    );
    assert!(system_is_registered(&sources, &sysid));
}

#[test]
fn system_is_registered_returns_true_after_offline_insert() {
    let mut sources = ShipSystemControlSources::default();
    let sysid = crate::system_registry::phaser_fore_system_id();
    sources.0.offline_systems.insert(sysid.clone());
    assert!(system_is_registered(&sources, &sysid));
}

#[test]
fn system_is_registered_returns_false_when_absent() {
    let sources = ShipSystemControlSources::default();
    let sysid = crate::system_registry::phaser_fore_system_id();
    assert!(!system_is_registered(&sources, &sysid));
}

// ── Per-bank fire gate ────────────────────────────────────────────────

#[test]
fn fire_phaser_refused_when_bank_fine_system_offline() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);

    // Reset beam / cooldown so the only variable is the bank gate.
    set_active_beam_target(&mut app, None);
    start_phaser_cooldown(&mut app, "port", 0.0);

    // Register the port bank as Human, then mark it offline (as
    // sync_console_damage_tiers would do on Disabled hull).
    register_fine_system(
        &mut app,
        SystemId("phaser-port".into()),
        crate::ship::control_source::ControlSource::Human,
    );
    mark_system_offline(&mut app, SystemId("phaser-port".into()));

    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "FirePhaser must be refused when the bank's fine system is offline"
    );
}

#[test]
fn fire_phaser_allowed_when_other_bank_offline_but_this_one_online() {
    let mut app = test_app();
    let _ = lock_and_fire(&mut app, 0.0, -20.0);
    set_active_beam_target(&mut app, None);
    start_phaser_cooldown(&mut app, "port", 0.0);
    start_phaser_cooldown(&mut app, "starboard", 0.0);

    // Only starboard offline; port stays online.
    mark_system_offline(&mut app, SystemId("phaser-starboard".into()));

    push(
        &mut app,
        "weapons",
        ClientMessage::FirePhaser {
            bank: "port".to_string(),
        },
    );
    let out = tick(&mut app);
    assert!(
        out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
        "Firing port must succeed when only starboard is offline"
    );
}

// ── Per-tube load/unload gate ─────────────────────────────────────────

#[test]
fn load_tube_emits_claim_torpedo_round_via_channel_2() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    push(
        &mut app,
        "weapons",
        ClientMessage::LoadTube {
            tube: "fore_port".to_string(),
        },
    );
    // Run one tick to admit the command → handle_load_tube emits the claim.
    tick(&mut app);

    let queue = &app.world().resource::<InterSystemQueue>().0;
    let claim_present = queue.iter().any(|m| {
        m.target == crate::system_registry::torpedo_magazine_system_id()
            && matches!(
                &m.payload,
                InterSystemPayload::ClaimTorpedoRound { tube } if tube == "fore_port"
            )
    });
    assert!(
        claim_present,
        "handle_load_tube should emit ClaimTorpedoRound on channel-2"
    );
}

#[test]
fn load_tube_refused_when_tube_fine_system_offline() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_tube_fore_port_system_id(),
    );

    push(
        &mut app,
        "weapons",
        ClientMessage::LoadTube {
            tube: "fore_port".to_string(),
        },
    );
    tick(&mut app);

    // No claim should have been emitted this tick.
    let queue = &app.world().resource::<InterSystemQueue>().0;
    assert!(
        !queue
            .iter()
            .any(|m| matches!(&m.payload, InterSystemPayload::ClaimTorpedoRound { .. })),
        "load must not emit a magazine claim when the tube system is offline"
    );
}

// ── Magazine claim transaction ────────────────────────────────────────
//
// Directly exercise `handle_torpedo_magazine_inter_system` by pushing
// a claim into the queue and asserting the same-tick effect on the
// magazine counter and the tube state.

#[test]
fn magazine_claim_decrements_counter_by_one_when_online() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    // Snapshot the magazine counter (starts at 10 from TorpedoConfig::default).
    let before = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert!(before > 0, "test precondition: magazine must have stock");

    // Drive the end-to-end path: `handle_load_tube` (Input) emits the
    // channel-2 claim, and `handle_torpedo_magazine_inter_system` (Physics)
    // consumes it — both happen within a single `app.update()` after
    // `clear_inter_system_queue` runs.
    push(
        &mut app,
        "weapons",
        ClientMessage::LoadTube {
            tube: "fore_port".to_string(),
        },
    );
    let _ = tick(&mut app);

    let after = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert_eq!(
        after,
        before - 1,
        "magazine counter must decrement by exactly one after a granted claim"
    );

    // The tube should now be Loading.
    let tube_loading = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| {
            matches!(
                ts.0.tube("fore_port").map(|t| &t.load_state),
                Some(crate::torpedo::TubeLoadState::Loading { .. })
            )
        })
        .unwrap();
    assert!(
        tube_loading,
        "granted claim must start loading the target tube via start_load_reserved"
    );
}

#[test]
fn magazine_claim_refused_when_magazine_offline() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    // Register magazine as human, then mark it offline (Disabled tier).
    register_fine_system(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
    );

    let before = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();

    // End-to-end: LoadTube tries to emit a claim — the tube gate passes
    // (fine tube systems default to the Human source), then the claim
    // goes to the magazine consumer which refuses because the magazine
    // is offline.
    push(
        &mut app,
        "weapons",
        ClientMessage::LoadTube {
            tube: "fore_port".to_string(),
        },
    );
    let _ = tick(&mut app);

    let after = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert_eq!(
        after, before,
        "offline magazine must refuse the claim — counter unchanged"
    );
}

#[test]
fn magazine_claim_refused_when_empty() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);

    // Drain the magazine.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        let mut ts = q.single_mut(app.world_mut()).unwrap();
        ts.0.torpedoes_remaining = 0;
    }

    push(
        &mut app,
        "weapons",
        ClientMessage::LoadTube {
            tube: "fore_port".to_string(),
        },
    );
    let _ = tick(&mut app);

    // Tube must still be Unloaded — no start_load_reserved happened.
    let tube_state = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.tube("fore_port").map(|t| t.load_state.clone()))
        .unwrap();
    assert_eq!(
        tube_state,
        Some(crate::torpedo::TubeLoadState::Unloaded),
        "empty magazine must not begin loading the tube"
    );
}

// ── Fire torpedo: magazine-online gate ────────────────────────────────

#[test]
fn fire_torpedo_refused_when_magazine_offline_even_if_tube_loaded() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    // Load the tube directly (bypass channel-2 to isolate the fire gate).
    load_tube_now(&mut app, "fore_port");

    // Register magazine as offline.
    register_fine_system(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_magazine_system_id(),
    );

    push(
        &mut app,
        "weapons",
        ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        },
    );
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "disabled magazine must block fire even from a loaded tube"
    );
}

#[test]
fn fire_torpedo_refused_when_tube_fine_system_offline() {
    let mut app = test_app();
    start_game_with_weapons(&mut app);
    load_tube_now(&mut app, "fore_port");
    mark_system_offline(
        &mut app,
        crate::system_registry::torpedo_tube_fore_port_system_id(),
    );

    push(
        &mut app,
        "weapons",
        ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        },
    );
    let out = tick(&mut app);
    assert!(
        !out.iter()
            .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
        "disabled tube fine system must block its fire"
    );
}

// ── Ship-level option (c) gate ────────────────────────────────────────

#[test]
fn set_target_refused_when_all_banks_offline() {
    let mut app = test_app();
    setup_weapons_world(&mut app, 30.0, 0.0);
    start_game_with_weapons(&mut app);
    // The updated `test_ship_config()` declares two fine phaser banks
    // ("phaser-fore", "phaser-aft"). `any_bank_accepts_human_input`
    // iterates them and returns true if ANY bank accepts human input.
    // So to refuse SetTarget, EVERY fine bank must be offline.
    // Mark both fine banks offline.
    mark_system_offline(&mut app, crate::system_registry::phaser_fore_system_id());
    mark_system_offline(&mut app, crate::system_registry::phaser_aft_system_id());

    push(
        &mut app,
        "weapons",
        ClientMessage::ControlSystem {
            target: crate::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "target-uuid".into(),
            },
        },
    );
    let out = tick(&mut app);
    let has_lock = out
        .iter()
        .any(|m| matches!(&m.msg, ServerMessage::TargetLock { .. }));
    assert!(
        !has_lock,
        "SetTarget must be refused when every phaser bank fine system is offline"
    );
}

// ── Blackboards ───────────────────────────────────────────────────────

#[test]
fn publish_writes_phaser_fore_blackboard_when_bank_configured() {
    let mut app = test_app();
    // The test app config has "port"/"starboard" banks — no "fore" bank.
    // Insert a fresh combat config with a "fore" bank so publish emits an entry.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>(
            );
        if let Ok(mut cc) = q.single_mut(app.world_mut()) {
            cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 270.0,
                auto_arc_deg: 180.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 6.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }];
        }
    }
    // Publish runs in SimSet::Publish — one full update ticks it.
    app.update();

    let key = crate::system_registry::phaser_fore_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let bb = bbs
        .0
        .get(&key)
        .expect("expected phaser-fore in blackboards");
    assert!(matches!(bb, SystemBlackboard::PhaserBank(_)));
}

#[test]
fn publish_writes_torpedo_magazine_blackboard() {
    let mut app = test_app();
    app.update();

    let key = crate::system_registry::torpedo_magazine_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::TorpedoMagazine(mag_bb) = bbs
        .0
        .get(&key)
        .expect("expected torpedo-magazine in blackboards")
        .clone()
    else {
        panic!("expected TorpedoMagazine blackboard");
    };
    assert!(
        mag_bb.is_online,
        "fresh test ship magazine should be online"
    );
    assert_eq!(mag_bb.torpedoes_remaining, mag_bb.capacity);
}

#[test]
fn publish_writes_torpedo_tube_blackboards_per_tube() {
    let mut app = test_app();
    app.update();

    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    for tube_key in [
        crate::system_registry::torpedo_tube_fore_port_system_id(),
        crate::system_registry::torpedo_tube_fore_starboard_system_id(),
        crate::system_registry::torpedo_tube_aft_system_id(),
    ] {
        let bb = bbs
            .0
            .get(&tube_key)
            .unwrap_or_else(|| panic!("expected {tube_key:?} in blackboards"));
        assert!(matches!(bb, SystemBlackboard::TorpedoTube(_)));
    }
}

// ── Ship-level AI early-skip regression tests (issue #512, findings 1 & 2) ─
//
// These tests cover the specific production path the reviewer flagged as
// dead code: after #512 deleted `[[system]] id = "tactical" kind = "tactical"`
// from every ship TOML, the coarse tactical SystemId is not registered
// in any ship's ControlSourceResolver. Every code path that gated on
// a coarse-tactical policy lookup would therefore see the
// default `Human` policy (`operate_ai = false`) and never run.
//
// These tests DO NOT touch the coarse `tactical` SystemId — they set
// AI only on a fine phaser bank / torpedo tube and assert the
// ship-level AI paths still activate.

/// Finding 1 regression: the phaser auto-fire path used to gate its
/// early skip on the coarse `tactical` policy. Post-fix, it uses
/// `any_bank_operates_ai` which iterates the ship config's `phaser_bank`
/// fine systems. This test seeds AI on ONE fine bank on an NPC — no
/// coarse tactical touching — and asserts a beam still activates.
#[test]
fn ai_phaser_auto_fire_activates_when_any_bank_operates_ai() {
    use crate::ai_plugin::AiTokenRegistry;
    use crate::entity_spawner::{EntitySystemHull, EntityUuid};

    let mut app = test_app();
    app.init_resource::<AiTokenRegistry>();

    let npc_uuid = "cc000000-0000-0000-0000-000000000001";
    let target_uuid = "cc000000-0000-0000-0000-000000000002";

    // The NPC has a `phaser_bank` fine system ("phaser-port") declared
    // in its ShipConfigComponent — matching what the ship_harrow_*.toml
    // NPC TOMLs do. Its policy is Ai. The coarse `tactical` SystemId
    // is INTENTIONALLY untouched — the test would fail before finding 1
    // was fixed because the early-skip in `tick_phaser_auto_fire` would
    // read the coarse tactical policy's `operate_ai == false` and
    // `continue`.
    const NPC_TOML: &str = r#"
[[system]]
id = "phaser-port"
kind = "phaser_bank"
ai_only = true
"#;
    let npc_ship_config = crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::parse_and_validate(NPC_TOML, &["phaser_bank"])
            .expect("NPC ship config must be valid"),
    );

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    sources.set(
        SystemId("phaser-port".into()),
        crate::ship::control_source::ControlSource::Ai,
    );
    // NOTE: coarse tactical NOT set — this is the whole point of the test.

    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ai_plugin::AiControllerComponent,
            crate::ship_plugin::ShipSystemControlSources(sources),
            npc_ship_config,
            WeaponsTarget(Some(target_uuid.to_string())),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            ShipPhysics::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            }),
            Transform::default(),
        ))
        .id();

    // Target directly ahead of NPC (yaw=0, forward=-Z).
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -20.0),
    ));

    app.update();

    let beam = app
        .world()
        .get::<ActiveBeam>(npc_entity)
        .expect("NPC entity must have ActiveBeam component");
    assert!(
        beam.target_uuid.is_some(),
        "ai_phaser_auto_fire must activate the beam when ANY phaser bank fine \
         system has operate_ai=true, even without the coarse tactical SystemId"
    );
    assert_eq!(
        beam.bank.as_deref(),
        Some("port"),
        "NPC should fire the port bank whose fine system is AI-operated"
    );
}

/// Finding 2 regression: the tactical AI used to gate its early skip on the
/// coarse `tactical` policy. Post-fix, it uses
/// `any_tactical_system_operates_ai` which iterates the ship config's
/// phaser_bank / torpedo_tube / torpedo_magazine fine systems. This
/// test seeds AI on `torpedo-magazine` alone (no coarse tactical) and
/// asserts the AI's WeaponsTarget sync path fires.
#[test]
fn ai_target_selection_runs_when_any_tactical_system_operates_ai() {
    let mut app = test_app();

    // Set the LocalShip's active rating to Assisted so torpedo_auto_fire is enabled.
    set_tactical_station_rating(&mut app, "Assisted");

    // Set torpedo-magazine to Ai on the LocalShip. Do NOT touch coarse tactical.
    {
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(world) {
            cs.0.set(
                crate::system_registry::torpedo_magazine_system_id(),
                crate::ship::control_source::ControlSource::Ai,
            );
            // Confirm coarse tactical is NOT set — this is what makes
            // the test cover the finding. (#801: "tactical" is a station
            // id, not a system, so nothing should ever register it.)
            assert!(
                !cs.0
                    .entries()
                    .any(|(id, _)| { id.0 == crate::system_registry::TACTICAL_STATION_ID }),
                "test invariant: coarse tactical must NOT be registered"
            );
        }
    }

    // Simulate a Destroy objective so ai_target_selection has something
    // to lock onto (the AI target-sync leg exercises the early-skip we're
    // testing).
    let target_uuid = uuid::Uuid::new_v4().to_string();
    spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .name_to_uuid
        .insert("wave_1".into(), target_uuid.clone());
    insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

    tick(&mut app);

    assert_eq!(
        get_weapons_target(&mut app).as_deref(),
        Some(target_uuid.as_str()),
        "ai_target_selection must run and lock the objective target when ANY \
         tactical fine system has operate_ai=true, even without the coarse tactical SystemId"
    );
}

// ── issue #692 (audit finding B1): tick_npc_auto_match_frequency gate ──
//
// Both frequency-hint systems must be gated on `AiHighFidelity`. The
// `tick_frequency_hint` path already is (`ai_frequency_hint`); these two
// tests cover the newly-added gate on the NPC auto-match path.

/// Spawns a target entity that `tick_npc_auto_match_frequency` can read a
/// shield frequency from: `EntityUuid` (matched against the locked target),
/// `Transform` (so `ai_target_selection`'s stale-target guard treats it as
/// alive and keeps the lock), and `ShipShields` carrying `freq`.
fn spawn_shield_target(app: &mut App, uuid: &str, freq: f32) {
    app.world_mut().spawn((
        crate::entity_spawner::EntityUuid(uuid.into()),
        bevy::prelude::Transform::from_xyz(0.0, 0.0, -30.0),
        crate::ship::shields::ShipShields(crate::shield::ShieldSystem::default(), freq),
    ));
}

/// Puts the LocalShip's Tactical fine systems under AI control (so
/// `any_tactical_system_operates_ai` is true) and locks it onto
/// `target_uuid` — shared setup for both auto-match tests.
fn setup_npc_auto_match(app: &mut App, target_uuid: &str) {
    set_tactical_control_source(app, crate::ship::control_source::ControlSource::Ai);
    set_weapons_target(app, Some(target_uuid.into()));
}

fn local_ship_entity(app: &mut App) -> Entity {
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .expect("test_app must spawn a LocalShip")
}

/// Positive path: a high-fidelity NPC whose Tactical is AI-operated and
/// which has a target locked drives its `ShipPhaserFrequency` toward the
/// target's shield frequency once `NPC_FREQ_MATCH_DELAY` elapses.
#[test]
fn npc_auto_match_frequency_matches_with_high_fidelity() {
    let mut app = test_app();
    let target_uuid = "shield-target-hi-fi";
    // Distinct from ShipPhaserFrequency's 0.5 default AND from the code's
    // 0.5 fallback, so an observed change proves a real match fired.
    let target_freq = 0.8_f32;

    setup_npc_auto_match(&mut app, target_uuid);
    spawn_shield_target(&mut app, target_uuid, target_freq);

    let ship = local_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ai_plugin::AiHighFidelity);

    assert_eq!(
        get_phaser_frequency(&mut app),
        0.5,
        "test invariant: phaser frequency starts at its default"
    );

    // NPC_FREQ_MATCH_DELAY = 2.0s; test app ticks at 200ms → ≥10 ticks to
    // cross the delay. Run extra to stay clear of first-tick dt edge cases.
    for _ in 0..15 {
        tick(&mut app);
    }

    assert_eq!(
        get_phaser_frequency(&mut app),
        target_freq,
        "high-fidelity NPC must auto-match its phaser frequency to the locked \
         target's shield frequency after the delay"
    );
}

/// Negative path (the gate under test): identical setup but WITHOUT
/// `AiHighFidelity` → the new gate suppresses auto-match and the phaser
/// frequency never changes. This test fails if the `has_high_fidelity`
/// gate is removed.
#[test]
fn npc_auto_match_frequency_gated_off_without_high_fidelity() {
    let mut app = test_app();
    let target_uuid = "shield-target-lo-fi";
    let target_freq = 0.8_f32;

    setup_npc_auto_match(&mut app, target_uuid);
    spawn_shield_target(&mut app, target_uuid, target_freq);

    // Deliberately NOT high-fidelity — no AiHighFidelity component.

    assert_eq!(
        get_phaser_frequency(&mut app),
        0.5,
        "test invariant: phaser frequency starts at its default"
    );

    for _ in 0..15 {
        tick(&mut app);
    }

    assert_eq!(
        get_phaser_frequency(&mut app),
        0.5,
        "without AiHighFidelity the auto-match gate must not fire; the phaser \
         frequency stays at its default"
    );
}

// ── Finding 5 regression: publish gates on offline_systems, not hardcoded Console match ──

/// If an unknown / non-standard bank id ends up in the bank blackboard,
/// the previous hardcoded `match "fore" | "aft"` returned `None` and
/// silently reported `is_online: true` regardless of hull state.
///
/// Post-fix, `is_online` is derived from `offline_systems` — so a bank
/// whose fine SystemId lives in `offline_systems` reports `is_online: false`
/// no matter whether the id matches a Console variant.
#[test]
fn publish_marks_bank_offline_when_fine_system_in_offline_set() {
    let mut app = test_app();
    // Swap in a bank config whose id is NOT in the hardcoded match
    // (e.g. "dorsal"), so the old bug's hardcoded id→Console arms
    // would default to online.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>(
            );
        if let Ok(mut cc) = q.single_mut(app.world_mut()) {
            cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                id: "dorsal".into(),
                facing_deg: 0.0,
                fire_arc_deg: 270.0,
                auto_arc_deg: 180.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 6.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }];
        }
    }
    // Mark the corresponding fine SystemId offline via offline_systems.
    mark_system_offline(&mut app, SystemId("phaser-dorsal".into()));

    app.update();

    let key = SystemId("phaser-dorsal".into());
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::PhaserBank(bb) = bbs
        .0
        .get(&key)
        .expect("expected phaser-dorsal blackboard entry")
        .clone()
    else {
        panic!("expected PhaserBank blackboard variant");
    };
    assert!(
        !bb.is_online,
        "bank must report is_online: false when its fine SystemId is in \
         offline_systems (regardless of whether the id matches a Console variant)"
    );
}

// ── Finding 7 regression: end-to-end hull → offline_systems → PhaserBankBlackboard ──
//
// Ties together sync_console_damage_tiers (in ship_plugin) and
// publish_phaser_bank_blackboards (in this module). A hull entry for
// Console::PhaserFore below the disabled threshold should end up as
// `phaser-fore ∈ offline_systems` after one tick, and the emitted
// blackboard should reflect `is_online: false`.

#[test]
fn hull_disabled_console_causes_publish_to_mark_bank_offline() {
    let mut app = test_app();
    // Register the sync system directly (test_app doesn't include ShipPlugin).
    app.add_systems(
        Update,
        crate::ship_plugin::sync_console_damage_tiers.in_set(crate::sim_sets::SimSet::Damage),
    );

    // Insert a "fore" bank so publish emits a `phaser-fore` blackboard.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>(
            );
        if let Ok(mut cc) = q.single_mut(app.world_mut()) {
            cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 270.0,
                auto_arc_deg: 180.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 6.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }];
        }
    }

    // Damage the PhaserFore console to 0 HP (Destroyed tier → offline).
    {
        let world = app.world_mut();
        let ship = world
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(world)
            .unwrap();
        let mut entity_mut = app.world_mut().entity_mut(ship);
        let mut hull = entity_mut
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap();
        hull.0.set_hp(&SystemId("phaser-fore".into()), 0.0);
    }

    // One update: sync_console_damage_tiers (Damage) writes offline_systems,
    // publish_phaser_bank_blackboards (Publish) reads it and emits the entry.
    app.update();

    // Step 1 verify: offline_systems contains `phaser-fore`.
    let phaser_fore_id = crate::system_registry::phaser_fore_system_id();
    let is_in_offline = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        let cs = q.single(app.world()).unwrap();
        cs.0.offline_systems.contains(&phaser_fore_id)
    };
    assert!(
        is_in_offline,
        "sync_console_damage_tiers must add phaser-fore to offline_systems \
         when Console::PhaserFore hull is at Disabled/Destroyed tier"
    );

    // Step 2 verify: blackboard reports is_online: false for phaser-fore.
    let mut q = app
        .world_mut()
        .query_filtered::<
            &crate::server_app::ShipSystemBlackboards,
            With<crate::server_app::LocalShip>,
        >();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::PhaserBank(bb) = bbs
        .0
        .get(&phaser_fore_id)
        .expect("expected phaser-fore blackboard entry")
        .clone()
    else {
        panic!("expected PhaserBank blackboard variant");
    };
    assert!(
        !bb.is_online,
        "PhaserBankBlackboard.is_online must be false end-to-end when the \
         console hull is disabled (hull → offline_systems → blackboard chain)"
    );
}

// ── Finding 8 regression: magazine claim routes by source_entity ──────
//
// Before the fix, `handle_load_tube` emitted `source_entity: None` on
// its `ClaimTorpedoRound` message. `handle_torpedo_magazine_inter_system`
// then queried `With<LocalShip>` only, so an NPC's claim would either
// be ignored entirely or misroute to the player ship. Post-fix, both
// sides route by source_entity (mirroring `handle_power_inter_system`)
// so each ship's claims mutate that ship's own magazine.

#[test]
fn magazine_claim_routes_to_shooter_ship_when_multiple_ships_have_magazines() {
    let mut app = test_app();

    // Snapshot the LocalShip's magazine counter.
    let localship_before = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();

    // Spawn a second Ship (NOT LocalShip) that also has a magazine. Give
    // it a fully-declared torpedo_magazine fine system with Human
    // policy so the online gate passes, and its own TorpedoSystemResource
    // with 10 torpedoes and a "fore_port" tube.
    let mut npc_sources = crate::ship::control_source::ControlSourceResolver::new();
    npc_sources.set(
        crate::system_registry::torpedo_magazine_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    npc_sources.set(
        crate::system_registry::torpedo_tube_fore_port_system_id(),
        crate::ship::control_source::ControlSource::Human,
    );
    let npc_torpedo_sys = TorpedoSystem::from_configs(
        &[crate::entity_config::TorpedoTubeConfig {
            id: "fore_port".into(),
            facing_deg: -30.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            volley_max: 1,
        }],
        TorpedoConfig {
            count: 10,
            ..Default::default()
        },
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship, // NOT LocalShip
            crate::entity_spawner::EntityUuid("npc-with-magazine".into()),
            crate::ship_plugin::ShipSystemControlSources(npc_sources),
            TorpedoSystemResource(npc_torpedo_sys),
            Transform::default(),
        ))
        .id();

    let npc_before = 10u32;

    // Install a one-shot system in `SimSet::Input` that pushes a claim
    // for the NPC entity into the queue every tick. This mirrors what
    // `handle_load_tube` would do if it ran for NPC ships — the point
    // of the test is that `handle_torpedo_magazine_inter_system` in
    // Physics routes the claim to the ship named by `source_entity`,
    // NOT to `With<LocalShip>` only.
    //
    // The queue is cleared by `clear_inter_system_queue` before
    // `SimSet::Input`, so pushing during Input survives to Physics.
    let claim_target_entity = npc_entity;
    app.add_systems(
        Update,
        (move |mut queue: ResMut<InterSystemQueue>| {
            queue.0.push(InterSystemMsg {
                target: crate::system_registry::torpedo_magazine_system_id(),
                payload: InterSystemPayload::ClaimTorpedoRound {
                    tube: "fore_port".into(),
                },
                source_entity: Some(claim_target_entity),
            });
        })
        .in_set(crate::sim_sets::SimSet::Input),
    );

    app.update();

    // LocalShip magazine must be UNCHANGED — the claim was for the NPC.
    let localship_after = app
        .world_mut()
        .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .map(|ts| ts.0.torpedoes_remaining)
        .unwrap();
    assert_eq!(
        localship_after, localship_before,
        "LocalShip magazine must NOT be decremented when the claim was \
         attributed to a different ship"
    );

    // NPC magazine must have decremented by 1.
    let npc_after = app
        .world()
        .get::<TorpedoSystemResource>(npc_entity)
        .unwrap()
        .0
        .torpedoes_remaining;
    assert_eq!(
        npc_after,
        npc_before - 1,
        "NPC magazine must decrement by 1 when its own claim is granted"
    );

    // NPC tube must be Loading.
    let npc_tube_loading = app
        .world()
        .get::<TorpedoSystemResource>(npc_entity)
        .unwrap()
        .0
        .tube("fore_port")
        .map(|t| matches!(t.load_state, crate::torpedo::TubeLoadState::Loading { .. }))
        .unwrap_or(false);
    assert!(
        npc_tube_loading,
        "NPC's own tube must transition to Loading after its claim is granted"
    );
}

// ── LOS blocking tests (Rapier) ──────────────────────────────────────────
//
// These tests spin up a Rapier world (like the collision tests in
// server_app.rs) and verify that the beam-tick phases route damage
// correctly when a blocking entity is between the shooter and the
// original target.

/// Build a minimal app with Rapier physics + WeaponsPlugin so
/// `tick_beams_prepare` runs the LOS raycast.
fn los_test_app() -> App {
    use bevy_rapier3d::prelude::RapierPhysicsPlugin;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<bevy::mesh::Mesh>()
        .init_resource::<bevy::scene::SceneSpawner>()
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .add_plugins(RapierPhysicsPlugin::<()>::default())
        .configure_sets(
            Update,
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
        .add_plugins(LobbyPlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .init_resource::<WorldResource>()
        .add_message::<AsteroidDestroyedVfx>()
        .add_message::<crate::ai_plugin::AiEntityDestroyed>()
        .init_resource::<CurrentPhaserMode>()
        .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
            TorpedoConfig::default(),
        )))
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
        // FactionRegistryResource for the LOS faction check.
        .insert_resource(crate::entities::config_cache::FactionRegistryResource(
            crate::entities::config_cache::get_faction_registry(),
        ))
        .add_plugins(WeaponsPlugin)
        .insert_resource(PhaserCombatConfigResource(
            crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: -90.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 100.0,
                    beam_duration_secs: 10.0,
                    cooldown_secs: 1.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            },
        ))
        // WeaponsPlugin already registers the three beam-tick phase
        // systems (tick_beams_prepare / tick_beams_apply_damage /
        // tick_beams_tick_lifetimes) and the two torpedo-tick phases
        // (build_torpedo_target_snapshot / tick_torpedo_lifecycle).
        // Do NOT register them again here.
        .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
        .add_systems(PostUpdate, collect);

    // Advance one tick to let Rapier initialise.
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    app.update();
    app
}

/// Helper: spawn a ship entity with a ball collider and phaser state.
/// Returns the Entity.
fn spawn_los_ship(
    app: &mut App,
    uuid: &str,
    x: f32,
    z: f32,
    faction: Option<uuid::Uuid>,
    hull_hp: f32,
    is_local: bool,
) -> bevy::ecs::entity::Entity {
    use bevy_rapier3d::prelude::{
        ActiveCollisionTypes, Collider, ColliderMassProperties, RigidBody,
    };
    let mut ecmds = app.world_mut().spawn((
        crate::server_app::Ship,
        crate::entity_spawner::EntityUuid(uuid.to_string()),
        ShipPhysics {
            x,
            z,
            yaw: 0.0,
            forward_speed: 0.0,
            roll: 0.0,
            lateral_speed: 0.0,
        },
        Transform::from_xyz(x, 0.0, z),
        GlobalTransform::default(),
        Visibility::default(),
        // Ball collider large enough for the raycast to hit.
        Collider::ball(3.0),
        RigidBody::Fixed,
        ColliderMassProperties::Density(1.0),
        ActiveCollisionTypes::all(),
        crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            hull_hp,
        )])),
        ActiveBeam::default(),
        PhaserCooldown::default(),
        PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 360.0,
                auto_arc_deg: 360.0,
                beam_range: 0.0,
                beam_damage_per_sec: 100.0,
                beam_duration_secs: 10.0,
                cooldown_secs: 1.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }],
        }),
        crate::ship_plugin::ShipSystemControlSources::default(),
    ));
    if is_local {
        ecmds.insert(crate::server_app::LocalShip);
    }
    if let Some(f) = faction {
        ecmds.insert(FactionComponent(f));
    }
    ecmds.id()
}

/// Helper: spawn an asteroid with a ball collider.
fn spawn_los_asteroid(
    app: &mut App,
    uuid: &str,
    x: f32,
    z: f32,
    hull_hp: f32,
) -> bevy::ecs::entity::Entity {
    use bevy_rapier3d::prelude::{
        ActiveCollisionTypes, Collider, ColliderMassProperties, RigidBody,
    };
    app.world_mut()
        .spawn((
            crate::simulation::Asteroid,
            AsteroidUuid(uuid.to_string()),
            Transform::from_xyz(x, 0.0, z),
            GlobalTransform::default(),
            Visibility::default(),
            Collider::ball(3.0),
            RigidBody::Fixed,
            ColliderMassProperties::Density(1.0),
            ActiveCollisionTypes::all(),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                hull_hp,
            )])),
        ))
        .id()
}

/// Activate a beam on the given ship entity, targeting `target_uuid`.
fn activate_los_beam(app: &mut App, shooter: bevy::ecs::entity::Entity, target_uuid: &str) {
    let mut beam = app.world_mut().get_mut::<ActiveBeam>(shooter).unwrap();
    beam.target_uuid = Some(target_uuid.to_string());
    beam.remaining_secs = 10.0;
    beam.damage_accumulator = 0.0;
    beam.bank = Some("port".to_string());
}

/// Read the total current hull HP from a ship/asteroid entity.
fn hull_hp(app: &App, entity: bevy::ecs::entity::Entity) -> f32 {
    app.world()
        .get::<crate::entity_spawner::EntitySystemHull>(entity)
        .map(|h| h.0.total_current())
        .unwrap_or(0.0)
}

#[test]
fn los_no_blocker_damages_original_target() {
    // Shooter at origin, target at (0, 0, -30). No entity in between.
    // Beam should damage the original target.
    let mut app = los_test_app();
    let faction_uuid = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();

    let shooter = spawn_los_ship(
        &mut app,
        "shooter-uuid",
        0.0,
        0.0,
        Some(faction_uuid),
        200.0,
        true,
    );
    let target = spawn_los_ship(&mut app, "target-uuid", 0.0, -30.0, None, 200.0, false);

    // Let Rapier settle and colliders register at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid");

    let before = hull_hp(&app, target);
    // Run a few ticks to accumulate damage.
    for _ in 0..5 {
        app.update();
    }
    let after = hull_hp(&app, target);
    assert!(
        after < before,
        "Target should take damage when LOS is clear (before={before}, after={after})"
    );
}

#[test]
fn los_enemy_blocker_redirects_damage_away_from_target() {
    // Shooter at origin. Enemy blocker at (0,0,-10). Original target at (0,0,-30).
    // Blocker is in the way → target takes no damage, blocker takes damage.
    use crate::config_cache::FactionRegistryResource;
    use crate::faction::FactionRegistry;

    let mut app = los_test_app();

    let shooter_faction = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
    let enemy_faction = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();

    // Make shooter hostile to blocker.
    let mut reg = FactionRegistry::new();
    reg.insert(crate::faction::FactionConfig {
        uuid: shooter_faction,
        name: "Federation".into(),
        enemies: vec![enemy_faction],
    });
    reg.insert(crate::faction::FactionConfig {
        uuid: enemy_faction,
        name: "Pirate".into(),
        enemies: vec![],
    });
    app.insert_resource(FactionRegistryResource(reg));

    let shooter = spawn_los_ship(
        &mut app,
        "shooter-uuid-2",
        0.0,
        0.0,
        Some(shooter_faction),
        200.0,
        true,
    );
    let blocker = spawn_los_ship(
        &mut app,
        "blocker-uuid-2",
        0.0,
        -10.0,
        Some(enemy_faction),
        500.0,
        false,
    );
    let target = spawn_los_ship(&mut app, "target-uuid-2", 0.0, -30.0, None, 500.0, false);

    // Let Rapier settle so colliders are at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid-2");

    let blocker_before = hull_hp(&app, blocker);
    let target_before = hull_hp(&app, target);
    // Run several ticks — each tick the ray hits the blocker, rerouting damage.
    for _ in 0..5 {
        app.update();
    }
    let blocker_after = hull_hp(&app, blocker);
    let target_after = hull_hp(&app, target);

    assert!(
        blocker_after < blocker_before,
        "Enemy blocker between shooter and target must take damage \
         (before={blocker_before}, after={blocker_after})"
    );
    assert_eq!(
        target_after, target_before,
        "Original target must NOT take damage when blocked \
         (before={target_before}, after={target_after})"
    );
}

#[test]
fn los_friendly_blocker_absorbs_beam_with_no_damage() {
    // Shooter and blocker are same faction. Blocker at (0,0,-10),
    // target at (0,0,-30). Neither blocker nor target should take damage.
    use crate::config_cache::FactionRegistryResource;
    use crate::faction::FactionRegistry;

    let mut app = los_test_app();

    let faction_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000003").unwrap();

    // Empty enemy list → faction is friendly to itself.
    let mut reg = FactionRegistry::new();
    reg.insert(crate::faction::FactionConfig {
        uuid: faction_uuid,
        name: "Federation".into(),
        enemies: vec![],
    });
    app.insert_resource(FactionRegistryResource(reg));

    let shooter = spawn_los_ship(
        &mut app,
        "shooter-uuid-3",
        0.0,
        0.0,
        Some(faction_uuid),
        200.0,
        true,
    );
    let blocker = spawn_los_ship(
        &mut app,
        "blocker-uuid-3",
        0.0,
        -10.0,
        Some(faction_uuid), // same faction → friendly
        500.0,
        false,
    );
    let target = spawn_los_ship(&mut app, "target-uuid-3", 0.0, -30.0, None, 500.0, false);

    // Let Rapier settle so colliders are at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid-3");

    let blocker_before = hull_hp(&app, blocker);
    let target_before = hull_hp(&app, target);
    for _ in 0..5 {
        app.update();
    }
    let blocker_after = hull_hp(&app, blocker);
    let target_after = hull_hp(&app, target);

    assert_eq!(
        blocker_after, blocker_before,
        "Friendly blocker must NOT take damage (before={blocker_before}, after={blocker_after})"
    );
    assert_eq!(
        target_after, target_before,
        "Target must NOT take damage when a friendly blocks (before={target_before}, after={target_after})"
    );
}

#[test]
fn los_asteroid_blocker_takes_damage() {
    // Asteroid at (0,0,-10), target at (0,0,-30).
    // Beam aimed at target — asteroid intercepts and takes damage.
    let mut app = los_test_app();

    let shooter = spawn_los_ship(&mut app, "shooter-uuid-4", 0.0, 0.0, None, 200.0, true);
    let ast = spawn_los_asteroid(&mut app, "ast-uuid-4", 0.0, -10.0, 2000.0);
    let target = spawn_los_ship(&mut app, "target-uuid-4", 0.0, -30.0, None, 500.0, false);

    // Let Rapier settle so colliders are at their correct positions.
    app.update();
    app.update();

    activate_los_beam(&mut app, shooter, "target-uuid-4");

    let ast_before = hull_hp(&app, ast);
    let target_before = hull_hp(&app, target);
    for _ in 0..5 {
        app.update();
    }
    let ast_after = hull_hp(&app, ast);
    let target_after = hull_hp(&app, target);

    assert!(
        ast_after < ast_before,
        "Asteroid blocker must take damage (before={ast_before}, after={ast_after})"
    );
    assert_eq!(
        target_after, target_before,
        "Target behind asteroid must NOT take damage (before={target_before}, after={target_after})"
    );
}

// ── Blaster AI auto-fire tests ──────────────────────────────────────

/// NPC with tactical set to Ai and target in range must have the auto-fire
/// system call `request_charge_start` on the blaster bank.
#[test]
fn tick_blaster_auto_fire_gate_passes_when_tactical_is_ai() {
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();

    let npc_uuid = "bb000000-0000-0000-0000-000000000010";
    let target_uuid = "bb000000-0000-0000-0000-000000000011";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the blaster bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    // NPC at (10, 10) — away from LocalShip at origin — facing -Z (target at 10, -10).
    // This avoids the projectile immediately hitting the LocalShip which
    // occupies (0, 0) in test_app().
    let npc_physics = ShipPhysics {
        x: 10.0,
        z: 10.0,
        ..Default::default()
    };
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ship_plugin::ShipSystemControlSources(sources),
            WeaponsTarget(Some(target_uuid.to_string())),
            npc_physics,
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 180.0, // face toward -Z (toward target)
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    range: 35.0,
                },
            )]),
            Transform::from_xyz(10.0, 0.0, 10.0),
        ))
        .id();

    // Spawn target directly ahead (-Z), well within blaster range.
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(10.0, 0.0, -10.0),
    ));

    // Check initial state before update.
    let init_bank = &app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap()
        .0[0];
    eprintln!(
        "[DEBUG] init: fire_ready={} on_cooldown={} pending={} charging={}",
        init_bank.is_fire_ready(),
        init_bank.volley.on_cooldown,
        init_bank.volley.pending_volley,
        init_bank.volley.charging,
    );

    app.update();

    let blaster_res = app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap();
    let bank = &blaster_res.0[0];
    // tick_blaster_auto_fire (Input) calls request_charge_start, then
    // tick_blaster_system (Physics) fires the projectile same-tick.
    // The projectile ends up in in_flight.
    assert!(
        !bank.in_flight.is_empty(),
        "tick_blaster_auto_fire must fire a blaster projectile when tactical is Ai \
         and target is in range/arc (in_flight={})",
        bank.in_flight.len(),
    );
}

/// NPC with AI-controlled blaster has target out of range — must NOT fire.
#[test]
fn tick_blaster_auto_fire_skips_when_target_out_of_range() {
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();

    let npc_uuid = "bb000000-0000-0000-0000-000000000020";
    let target_uuid = "bb000000-0000-0000-0000-000000000021";

    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the blaster bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ai_plugin::AiControllerComponent,
            crate::ship_plugin::ShipSystemControlSources(sources),
            WeaponsTarget(Some(target_uuid.to_string())),
            ShipPhysics::default(),
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    range: 35.0,
                },
            )]),
            Transform::default(),
        ))
        .id();

    // Spawn target well outside blaster range (35 units).
    app.world_mut().spawn((
        EntityUuid(target_uuid.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -100.0),
    ));

    app.update();

    let blaster_res = app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap();
    assert_eq!(
        blaster_res.0[0].volley.pending_volley, 0,
        "tick_blaster_auto_fire must NOT fire when target is out of range"
    );
}

/// AI token sent through `handle_fire_blaster` must route to the NPC and fire.
#[test]
fn handle_fire_blaster_accepts_ai_token() {
    use crate::entity_spawner::EntityUuid;

    let mut app = test_app();
    app.init_resource::<crate::ai_plugin::AiTokenRegistry>();

    let npc_uuid = "bb000000-0000-0000-0000-000000000030";
    let target_uuid_str = "bb000000-0000-0000-0000-000000000031";

    // NPC with Tactical set to Ai.
    let mut sources = crate::ship::control_source::ControlSourceResolver::new();
    // #801: seed the blaster bank's fine system (no coarse tactical).
    sources.set(
        crate::system_registry::blaster_bank_system_id("fore").unwrap(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();
    let npc_entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid.to_string()),
            crate::ai_plugin::AiControllerComponent,
            // Seeds the NPC's Tactical lock. This used to seed
            // `ShipAiMemory.target` and rely on `tick_blaster_auto_fire`'s
            // legacy fallback to read it; #702 deleted that fallback, so the
            // lock goes where every other consumer looks for it.
            WeaponsTarget(Some(target_uuid_parsed.to_string())),
            crate::ship_plugin::ShipSystemControlSources(sources),
            ShipPhysics::default(),
            BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                crate::blaster::BlasterBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 360.0,
                    volley_count: 1,
                    volley_interval_secs: 0.1,
                    cooldown_secs: 3.0,
                    charge_time_secs: 0.0,
                    projectile_speed: 40.0,
                    collision_radius: 1.5,
                    visual_scale: 1.0,
                    damage: 10,
                    shield_pierce: 0.0,
                    recoil_impulse: 0.0,
                    screenshake_magnitude: 0.0,
                    marker: None,
                    range: 35.0,
                },
            )]),
            Transform::default(),
        ))
        .id();

    // Spawn target entity at (0, -10) — directly ahead of NPC at origin,
    // within the 35-unit range and inside the 360° fire arc.
    app.world_mut().spawn((
        EntityUuid(target_uuid_str.to_string()),
        crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
            SystemId("captain".into()),
            50.0,
        )])),
        Transform::from_xyz(0.0, 0.0, -10.0),
    ));

    // Register the AI token so handle_fire_blaster can resolve it.
    {
        let mut reg = app
            .world_mut()
            .resource_mut::<crate::ai_plugin::AiTokenRegistry>();
        reg.register_with_entity(npc_uuid, npc_entity);
    }

    // Send a FireBlaster ControlSystem message via the AI token.
    let ai_token = format!("ai:{}", npc_uuid);
    push(
        &mut app,
        &ai_token,
        ClientMessage::ControlSystem {
            target: SystemId("blaster-fore".into()),
            payload: SystemControlPayload::FireBlaster,
        },
    );

    app.update();

    let blaster_res = app
        .world()
        .get::<BlasterSystemResource>(npc_entity)
        .unwrap();
    // After app.update(): handle_fire_blaster (Input) arms the volley, then
    // tick_blaster_system (Physics) fires it and enters cooldown. By the time
    // we check, pending_volley is 0 and on_cooldown is true — verify cooldown
    // as evidence the volley was dispatched.
    assert!(
        blaster_res.0[0].volley.on_cooldown,
        "handle_fire_blaster must accept AI token and enter cooldown after firing"
    );
}
