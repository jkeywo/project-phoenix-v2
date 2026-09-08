//! #1400 baseline for the unmodified Damage schedule. Test state stages a
//! simultaneous impact; ordinary beam, projectile and Rapier consumers apply it.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{
    ecs::message::MessageCursor, ecs::schedule::ExecutorKind, ecs::system::SystemState, prelude::*,
};
use bevy_rapier3d::prelude::*;
use project_phoenix::{
    command_admission::log::{stamp_accepted_command, PendingCommands, ShipKey},
    console::weapons::{
        ActiveBeam, BlasterSystemResource, LastShipAttacker, TacticalRadarSelection,
    },
    core::{
        balance::BalanceEvent,
        messages::{AdmittedCommand, GamePhase, ServerMessage, SystemControlPayload, SystemId},
    },
    entities::{
        config::{ColliderConfig, ColliderShape},
        include_resolve::load_entity_config,
        spawner::{spawn_entity, ColliderSection, EntityShipArcHull, EntitySystemHull, EntityUuid},
    },
    headless::{build_headless_app, HeadlessArgs},
    lobby::server::OutboundMessage,
    server_app::{CollisionCooldown, GameOverReason, LocalShip, SimOutbox},
    ship::{
        control_source::ControlSource,
        shields::ShipShields,
        state::{ShipPhysics, ShipRedAlert},
    },
    ship_plugin::{LastHelmInput, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::SimTick,
    snapshot,
};
use serde_json::{json, Value};

const WORLD: &str = "assets/worlds/combat_test.toml";
const HULL: &str = "assets/entities/alliance_destroyer.toml";
const SHOOTER: &str = "00000000-0000-8000-8000-000000014031";
const WALL: &str = "00000000-0000-8000-8000-000000014032";
const SEED: u64 = 140031;
const X: f32 = 100_000.0;
const PREFIX: &str = "PHOENIX_DAMAGE_BASELINE=";

struct Fixture {
    app: App,
    shooter: Entity,
    victim: Entity,
    wall: Entity,
    period: std::time::Duration,
    prepared: Vec<Value>,
    contact_before_impact: bool,
}

fn queue(app: &mut App, ship: Entity, target: SystemId, payload: SystemControlPayload) {
    let tick = app.world().resource::<SimTick>().0;
    let key = ShipKey(app.world().get::<EntityUuid>(ship).unwrap().0.clone());
    stamp_accepted_command(
        &mut app.world_mut().resource_mut::<PendingCommands>(),
        tick,
        None,
        ship,
        key,
        AdmittedCommand {
            target,
            payload,
            response_token: None,
            feedback_correlation: None,
        },
    );
}

fn human(app: &mut App, entity: Entity) {
    let ids: Vec<_> = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .systems
        .iter()
        .map(|s| s.id.clone())
        .collect();
    let mut sources = app
        .world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap();
    for id in ids {
        sources.0.set(id, ControlSource::Human);
    }
}

fn place(app: &mut App, entity: Entity, x: f32, z: f32, yaw: f32, speed: f32) {
    *app.world_mut().get_mut::<ShipPhysics>(entity).unwrap() = ShipPhysics {
        x,
        z,
        yaw,
        forward_speed: speed,
        ..Default::default()
    };
    app.world_mut()
        .entity_mut(entity)
        .insert(LastHelmInput::default());
    let mut transform = app.world_mut().get_mut::<Transform>(entity).unwrap();
    transform.translation = Vec3::new(x, 0.0, z);
    transform.rotation = Quat::from_rotation_y(-yaw);
}

fn active_contact(app: &mut App, victim: Entity, wall: Entity) -> bool {
    let mut state = SystemState::<ReadRapierContext>::new(app.world_mut());
    let param = state.get(app.world());
    let ctx = param.single().expect("ordinary Rapier context");
    let touches = ctx.contact_pairs_with(victim).any(|pair| {
        pair.has_any_active_contact()
            && (pair.collider1() == Some(wall) || pair.collider2() == Some(wall))
    });
    touches
}

fn step(app: &mut App) {
    let tick = app.world().resource::<SimTick>().0;
    app.update();
    let next = app.world().resource::<SimTick>().0;
    let playing = *app.world().resource::<State<GamePhase>>().get() == GamePhase::InProgress;
    if playing {
        assert_eq!(next, tick + 1, "one real fixed step");
    } else {
        assert!(next == tick || next == tick + 1, "bounded terminal step");
    }
}

fn observe(
    app: &App,
    shooter: Entity,
    victim: Entity,
    balance: &mut MessageCursor<BalanceEvent>,
    wire: &mut MessageCursor<OutboundMessage>,
) -> Value {
    let world = app.world();
    let event_rows: Vec<_> = balance.read(world.resource::<Messages<BalanceEvent>>()).map(|e| json!({"exact":format!("{e:?}"),"fact":serde_json::from_str::<Value>(&e.to_json()).unwrap()})).collect();
    let wire_rows: Vec<_> = wire.read(world.resource::<Messages<OutboundMessage>>()).filter_map(|row| {
        let message = serde_json::to_value(&row.msg).unwrap();
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        matches!(kind, "DamageTaken" | "ShipDestroyed" | "BlasterHit" | "GameOver" | "EntityDespawned" | "ShieldArcOffline" | "SystemDamage" | "ObjectiveSummary").then(|| json!({"target":format!("{:?}",row.target),"delivery":format!("{:?}",row.delivery),"message":message}))
    }).collect();
    let ships: Vec<_> = [shooter, victim].into_iter().map(|entity| {
        let p = world.get::<ShipPhysics>(entity).unwrap();
        json!({
            "uuid":world.get::<EntityUuid>(entity).unwrap().0,
            "physics":[p.x,p.y,p.z,p.yaw,p.forward_speed,p.lateral_speed,p.vertical_speed,p.roll],
            "hull":world.get::<EntitySystemHull>(entity).unwrap().0.iter().map(|(id,h)|json!([id,h.current,h.max])).collect::<Vec<_>>(),
            "arcs":world.get::<EntityShipArcHull>(entity).map(|h|h.0.iter().map(|(id,h)|json!([id,h.current,h.max])).collect::<Vec<_>>()),
            "shields":world.get::<ShipShields>(entity).map(|s|s.0.facings.iter().map(|f|json!([f.id,f.hp,f.max_hp,f.offline_remaining,f.is_focused])).collect::<Vec<_>>()),
            "last_attacker":world.get::<LastShipAttacker>(entity).map(|a|a.0.clone()),
            "collision_cooldown":world.get::<CollisionCooldown>(entity).map(|c|c.remaining_secs),
            "beam":world.get::<ActiveBeam>(entity).unwrap().live_banks().map(|(bank,slot)|json!({"bank":bank,"victim":slot.target_uuid,"remaining":slot.remaining_secs,"accumulator":slot.damage_accumulator,"cooldown":slot.pending_cooldown_secs})).collect::<Vec<_>>(),
            "blasters":world.get::<BlasterSystemResource>(entity).unwrap().0.iter().map(|bank|json!({"bank":bank.config.id,"volley":format!("{:?}",bank.volley),"authored_recoil":bank.config.recoil_impulse,"projectiles":bank.in_flight.iter().map(|p|json!({"id":p.id,"source":p.source_uuid,"position":[p.x,p.z],"heading":p.heading,"speed":p.speed,"life":p.lifespan_remaining,"damage":p.damage,"radius":p.collision_radius})).collect::<Vec<_>>()})).collect::<Vec<_>>()
        })
    }).collect();
    let reason = world.resource::<GameOverReason>();
    json!({"tick":world.resource::<SimTick>().0,"digest":format!("{:016x}",world_digest(world)),
        "rng":world.resource::<SimRng>().state(),"phase":format!("{:?}",world.resource::<State<GamePhase>>().get()),
        "next_phase":format!("{:?}",world.resource::<NextState<GamePhase>>()),"reason":reason.0,"outcome":format!("{:?}",reason.1),
        "ships":ships,"events":event_rows,"wire":wire_rows,
        "pending_outbox":world.resource::<SimOutbox>().iter().map(|(target,msg)|json!([format!("{target:?}"),serde_json::to_value(msg).unwrap()])).collect::<Vec<_>>()})
}

fn prepare(pinned: bool, lethal: bool) -> Fixture {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        seed: Some(SEED),
        deterministic: pinned,
        max_ticks: 300,
        ..Default::default()
    })
    .unwrap();
    let period = project_phoenix::sim_tick::sim_tick_period(
        app.world()
            .resource::<project_phoenix::world::config::WorldConfig>()
            .global
            .sim_tick_hz,
    );
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(app.world().resource::<SimRng>().seed(), SEED);
    assert_eq!(app.world().resource::<Time<Fixed>>().timestep(), period);
    assert_eq!(
        app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
        if pinned {
            ExecutorKind::SingleThreaded
        } else {
            ExecutorKind::MultiThreaded
        }
    );
    let workers = bevy::tasks::ComputeTaskPool::get().thread_num();
    if pinned {
        assert_eq!(workers, 1);
    } else {
        assert!(workers > 1);
    }
    let victim = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    human(&mut app, victim);
    place(&mut app, victim, X, 0.0, 0.0, 0.0);
    let config = load_entity_config(HULL).expect("ordinary composed authored hull");
    let shooter = spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(X, 0.0, -20.0),
        SHOOTER.into(),
        None,
    );
    app.world_mut().flush();
    human(&mut app, shooter);
    place(&mut app, shooter, X, -20.0, std::f32::consts::PI, 0.0);
    app.world_mut()
        .entity_mut(shooter)
        .insert(ShipRedAlert(true));
    let radius = app.world().get::<ColliderSection>(victim).unwrap().0.radius;
    let wall = app
        .world_mut()
        .spawn((
            EntityUuid(WALL.into()),
            Transform::from_xyz(X + radius + 0.5, 0.0, 0.0),
            Collider::ball(1.0),
            RigidBody::Fixed,
            ActiveCollisionTypes::KINEMATIC_STATIC,
            ColliderSection(ColliderConfig {
                shape: ColliderShape::Ball,
                radius: 1.0,
                length: 0.0,
                half_height: None,
                movable: false,
            }),
        ))
        .id();
    // Change current state only. All authored system IDs, maxima, coefficients,
    // firing cadence and production systems remain intact. Settle HP-derived
    // modifiers before the snapshot boundary, as ordinary restoration does.
    let ids: Vec<_> = app
        .world()
        .get::<EntitySystemHull>(victim)
        .unwrap()
        .0
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    let total = if lethal { 15.0 } else { 100.0 };
    for id in &ids {
        app.world_mut()
            .get_mut::<EntitySystemHull>(victim)
            .unwrap()
            .0
            .set_hp(id, total / ids.len() as f32);
    }
    if let Some(mut arc) = app.world_mut().get_mut::<EntityShipArcHull>(victim) {
        let ids: Vec<_> = arc.0.iter().map(|(id, _)| id.to_owned()).collect();
        for id in &ids {
            arc.0.set_hp(id, total / ids.len() as f32);
        }
    }
    let mut balance = MessageCursor::default();
    let mut wire = MessageCursor::default();
    let mut prepared = Vec::new();
    for _ in 0..3 {
        place(&mut app, victim, X, 0.0, 0.0, 0.0);
        app.world_mut()
            .get_mut::<CollisionCooldown>(victim)
            .unwrap()
            .remaining_secs = 1000.0;
        step(&mut app);
        prepared.push(observe(&app, shooter, victim, &mut balance, &mut wire));
    }
    assert!(
        active_contact(&mut app, victim, wall),
        "real side contact must exist before arming damage"
    );
    let target = app.world().get::<EntityUuid>(victim).unwrap().0.clone();
    queue(
        &mut app,
        shooter,
        project_phoenix::ship::system_registry::tactical_radar_system_id(),
        SystemControlPayload::SetTarget {
            uuid: target.clone(),
        },
    );
    place(&mut app, victim, X, 0.0, 0.0, 0.0);
    step(&mut app);
    prepared.push(observe(&app, shooter, victim, &mut balance, &mut wire));
    assert_eq!(
        app.world()
            .get::<TacticalRadarSelection>(shooter)
            .unwrap()
            .0
            .as_deref(),
        Some(target.as_str())
    );
    // The actual set-target consumer publishes the lock; fire reads that lock.
    queue(
        &mut app,
        shooter,
        SystemId("phaser-omni".into()),
        SystemControlPayload::FirePhaser,
    );
    queue(
        &mut app,
        shooter,
        SystemId("blaster-port".into()),
        SystemControlPayload::ChargeBlasterStart,
    );
    queue(
        &mut app,
        shooter,
        SystemId("blaster-starboard".into()),
        SystemControlPayload::ChargeBlasterStart,
    );
    place(&mut app, victim, X, 0.0, 0.0, 0.0);
    step(&mut app);
    prepared.push(observe(&app, shooter, victim, &mut balance, &mut wire));
    let fired: Vec<_> = prepared
        .iter()
        .flat_map(|row| row["events"].as_array().unwrap())
        .map(|row| &row["fact"])
        .filter(|event| event["event"] == "WeaponFired" && event["shooter"] == SHOOTER)
        .collect();
    assert!(
        fired
            .iter()
            .any(|event| event["kind"] == "beam" && event["weapon"] == "omni"),
        "attributed beam producer"
    );
    for bank in ["port", "starboard"] {
        assert!(
            fired
                .iter()
                .any(|event| event["kind"] == "blaster" && event["weapon"] == bank),
            "attributed {bank} producer"
        );
    }
    assert_eq!(
        app.world()
            .get::<ActiveBeam>(shooter)
            .unwrap()
            .live_banks()
            .count(),
        1,
        "real fire command lit authored bank"
    );
    assert_eq!(
        app.world()
            .get::<BlasterSystemResource>(shooter)
            .unwrap()
            .0
            .iter()
            .map(|b| b.in_flight.len())
            .sum::<usize>(),
        2,
        "two real minted launches, one will survive impact"
    );
    let contact_before_impact = active_contact(&mut app, victim, wall);
    assert!(
        contact_before_impact,
        "real contact still exists immediately before impact staging"
    );
    let bank = app
        .world()
        .get::<ActiveBeam>(shooter)
        .unwrap()
        .live_banks()
        .next()
        .unwrap()
        .0
        .clone();
    // A reachable fractional accumulator: next ordinary beam tick applies one HP.
    app.world_mut()
        .get_mut::<ActiveBeam>(shooter)
        .unwrap()
        .bank_slot_mut(&bank)
        .unwrap()
        .damage_accumulator = 0.999;
    {
        let mut blasters = app
            .world_mut()
            .get_mut::<BlasterSystemResource>(shooter)
            .unwrap();
        for bank in &mut blasters.0 {
            let p = bank.in_flight.first_mut().unwrap();
            assert_eq!(p.source_uuid, SHOOTER);
            if bank.config.id == "port" {
                p.x = X;
                p.z = -p.speed * period.as_secs_f32();
                p.heading = std::f32::consts::PI;
            } else {
                p.x = X + 10.0;
                p.z = 0.0;
                p.heading = 0.0;
            }
        }
    }
    for facing in &mut app
        .world_mut()
        .get_mut::<ShipShields>(victim)
        .unwrap()
        .0
        .facings
    {
        facing.hp = 1;
        facing.offline_remaining = 0.0;
    }
    place(&mut app, victim, X, 0.0, 0.0, 20.0);
    app.world_mut()
        .get_mut::<CollisionCooldown>(victim)
        .unwrap()
        .remaining_secs = 0.0;
    Fixture {
        app,
        shooter,
        victim,
        wall,
        period,
        prepared,
        contact_before_impact,
    }
}

// Focused restore-edge tests use the ordinary App and Captain consumer, without
// staged damage. The live identity prevents the separate death-cleanup path
// from clearing the captured attacker and masking the alert-edge behavior.
fn alert_restore_fixture(active: bool) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    })
    .unwrap();
    let period = project_phoenix::sim_tick::sim_tick_period(
        app.world()
            .resource::<project_phoenix::world::config::WorldConfig>()
            .global
            .sim_tick_hz,
    );
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }
    let victim = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    human(&mut app, victim);
    app.world_mut()
        .spawn((EntityUuid(SHOOTER.into()), Transform::from_xyz(X, 0.0, 0.0)));
    ordinary_alert(&mut app, victim, active);
    app.world_mut()
        .get_mut::<LastShipAttacker>(victim)
        .unwrap()
        .0 = Some(SHOOTER.into());
    step(&mut app);
    assert_eq!(
        app.world()
            .get::<LastShipAttacker>(victim)
            .unwrap()
            .0
            .as_deref(),
        Some(SHOOTER)
    );
    (app, victim)
}

fn ordinary_alert(app: &mut App, victim: Entity, active: bool) {
    queue(
        app,
        victim,
        project_phoenix::ship::system_registry::red_alert_system_id(),
        SystemControlPayload::SetRedAlert { active },
    );
    step(app);
    assert_eq!(app.world().get::<ShipRedAlert>(victim).unwrap().0, active);
}

fn set_captured_stance(app: &mut App, victim: Entity, stance: &str) {
    use project_phoenix::core::messages::StationId;
    let station = StationId("tactical".into());
    let config = &app.world().get::<ShipConfigComponent>(victim).unwrap().0;
    assert!(config
        .station(&station)
        .unwrap()
        .stances
        .iter()
        .any(|s| s.id == stance));
    // Both neutral entries are selectable authored states, including selecting
    // the other neutral while the alert itself stays unchanged. Stage captured
    // state, then exercise the real restore/alert consumers without replacing
    // their schedule or adding an observer system.
    app.world_mut()
        .get_mut::<project_phoenix::console::command::server::ShipStationStances>(victim)
        .unwrap()
        .0
        .insert(station, stance.into());
}

fn stored_stance(app: &App, victim: Entity) -> Option<&str> {
    app.world()
        .get::<project_phoenix::console::command::server::ShipStationStances>(victim)
        .unwrap()
        .0
        .get(&project_phoenix::core::messages::StationId(
            "tactical".into(),
        ))
        .map(String::as_str)
}

fn check_alert_replacement(unconsumed_bootstrap: bool) {
    use bevy::ecs::change_detection::Tick;
    for captured in [false, true] {
        let (mut source, source_ship) = alert_restore_fixture(captured);
        let stance = if captured {
            "tactical-normal"
        } else {
            "tactical-high"
        };
        set_captured_stance(&mut source, source_ship, stance);
        let checkpoint = snapshot::capture(source.world());
        let digest = world_digest(source.world());
        for destination in [false, true] {
            let (mut target, target_ship) = alert_restore_fixture(destination);
            if unconsumed_bootstrap {
                // A newly inserted component whose change no ordinary reader
                // has consumed. Bypass-only restoration leaves this edge live.
                target
                    .world_mut()
                    .entity_mut(target_ship)
                    .remove::<ShipRedAlert>();
                target
                    .world_mut()
                    .entity_mut(target_ship)
                    .insert(ShipRedAlert(destination));
            }
            let report = snapshot::restore(target.world_mut(), &checkpoint);
            assert!(report.is_complete(), "{:?}", report.gaps);
            assert_eq!(world_digest(target.world()), digest);
            let changed = target
                .world()
                .entity(target_ship)
                .get_ref::<ShipRedAlert>()
                .unwrap()
                .last_changed();
            assert!(
                !changed.is_newer_than(Tick::new(0), target.world().read_change_tick()),
                "even a first-run Changed reader must see a replacement, not a live edge"
            );
            for _ in 0..2 {
                step(&mut target);
                assert_eq!(
                    target.world().get::<ShipRedAlert>(target_ship).unwrap().0,
                    captured
                );
                assert_eq!(target.world().get::<LastShipAttacker>(target_ship).unwrap().0.as_deref(), Some(SHOOTER),
                    "captured={captured}, destination={destination}, bootstrap={unconsumed_bootstrap}");
                assert_eq!(stored_stance(&target, target_ship), Some(stance),
                    "restore must preserve the captured selectable stance, independently of bootstrap alert");
            }
        }
    }
}

#[test]
fn snapshot_alert_restore_replaces_all_boolean_combinations() {
    check_alert_replacement(false);
}

#[test]
fn snapshot_alert_restore_rebases_unconsumed_bootstrap_changes() {
    check_alert_replacement(true);
}

#[test]
fn snapshot_alert_restore_keeps_ordinary_explicit_stand_down() {
    let (mut app, victim) = alert_restore_fixture(false);
    set_captured_stance(&mut app, victim, "tactical-normal");
    let checkpoint = snapshot::capture(app.world());
    let report = snapshot::restore(app.world_mut(), &checkpoint);
    assert!(report.is_complete(), "{:?}", report.gaps);
    ordinary_alert(&mut app, victim, false);
    assert_eq!(
        app.world()
            .get::<LastShipAttacker>(victim)
            .unwrap()
            .0
            .as_deref(),
        Some(SHOOTER),
        "same-value ordinary alert command remains idempotent"
    );
    assert_eq!(stored_stance(&app, victim), Some("tactical-normal"));
    ordinary_alert(&mut app, victim, true);
    assert_eq!(
        stored_stance(&app, victim),
        Some("tactical-high"),
        "the actual Captain raise-alert command still updates a stored neutral stance"
    );
    assert_eq!(
        app.world()
            .get::<LastShipAttacker>(victim)
            .unwrap()
            .0
            .as_deref(),
        Some(SHOOTER)
    );
    ordinary_alert(&mut app, victim, false);
    assert_eq!(
        app.world().get::<LastShipAttacker>(victim).unwrap().0,
        None,
        "the actual Captain stand-down command still clears the attacker"
    );
    assert_eq!(
        stored_stance(&app, victim),
        Some("tactical-normal"),
        "the actual Captain stand-down command still updates the stored stance"
    );
}

#[test]
fn same_target_damage_baseline_agrees_across_default_default_and_pinned() {
    assert!(std::env::var_os("PHOENIX_DAMAGE_BASELINE_ROLE").is_none());
    for case in ["survive", "near-lethal"] {
        let mut reports = Vec::new();
        for role in ["default-1", "default-2", "pinned"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "same_target_damage_baseline_child",
                    "--nocapture",
                ])
                .env("PHOENIX_DAMAGE_BASELINE_ROLE", role)
                .env("PHOENIX_DAMAGE_BASELINE_CASE", case)
                .env("RUST_TEST_THREADS", "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            print!("{stdout}");
            eprint!("{stderr}");
            assert!(output.status.success(), "{case}/{role}: {}", output.status);
            assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"));
            let rows: Vec<_> = stdout
                .lines()
                .filter_map(|line| line.strip_prefix(PREFIX))
                .collect();
            assert_eq!(rows.len(), 1);
            let value: Value = serde_json::from_str(rows[0]).unwrap();
            assert_eq!(value["role"], role);
            assert_eq!(value["case"], case);
            reports.push(value);
        }
        for other in &reports[1..] {
            assert_eq!(
                reports[0]["prepared"], other["prepared"],
                "{case}: preparation diverged"
            );
            assert_eq!(
                reports[0]["before"], other["before"],
                "{case}: pre-impact eligibility diverged"
            );
            assert_eq!(
                reports[0]["expected_restore_refresh"], other["expected_restore_refresh"],
                "{case}: captured objective refresh diverged"
            );
            assert_eq!(
                reports[0]["baseline"], other["baseline"],
                "{case}: live damage semantics diverged"
            );
            assert_eq!(
                reports[0]["restored"], other["restored"],
                "{case}: restore continuation diverged"
            );
        }
    }
}

#[test]
#[ignore = "parent starts one fresh process for each case and pool mode"]
fn same_target_damage_baseline_child() {
    let role = std::env::var("PHOENIX_DAMAGE_BASELINE_ROLE").expect("parent role");
    let case = std::env::var("PHOENIX_DAMAGE_BASELINE_CASE").expect("parent case");
    assert!(matches!(
        role.as_str(),
        "default-1" | "default-2" | "pinned"
    ));
    assert!(matches!(case.as_str(), "survive" | "near-lethal"));
    let mut f = prepare(role == "pinned", case == "near-lethal");
    let checkpoint = snapshot::capture(f.app.world());
    let checkpoint_digest = world_digest(f.app.world());
    let target = f.app.world().get::<EntityUuid>(f.victim).unwrap().0.clone();
    let expected_restore_refresh = json!([
        "All",
        ServerMessage::ObjectiveSummary {
            objectives: f
                .app
                .world()
                .resource::<project_phoenix::world::server::ObjectiveManagerRes>()
                .0
                .snapshots_for(&target),
        }
    ]);
    let mut balance = MessageCursor::default();
    let mut wire = MessageCursor::default();
    balance
        .read(f.app.world().resource::<Messages<BalanceEvent>>())
        .for_each(drop);
    wire.read(f.app.world().resource::<Messages<OutboundMessage>>())
        .for_each(drop);
    let mut before = observe(&f.app, f.shooter, f.victim, &mut balance, &mut wire);
    before["active_contact"] = json!(f.contact_before_impact);
    assert_eq!(before["active_contact"], true);
    assert_eq!(before["ships"][1]["collision_cooldown"], 0.0);
    assert_eq!(before["ships"][1]["physics"][4], 20.0);
    let live_beam = &before["ships"][0]["beam"][0];
    assert_eq!(live_beam["victim"], target);
    assert!(live_beam["remaining"].as_f64().unwrap() > f.period.as_secs_f64());
    assert!(live_beam["accumulator"].as_f64().unwrap() > 0.99);
    let incoming = &before["ships"][0]["blasters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bank| bank["bank"] == "port")
        .unwrap()["projectiles"][0];
    assert_eq!(incoming["source"], SHOOTER);
    assert!(incoming["damage"].as_u64().unwrap() > 0);
    assert!(incoming["life"].as_f64().unwrap() > f.period.as_secs_f64());
    assert_eq!(incoming["position"][0], X);
    assert!(incoming["position"][1].as_f64().unwrap() < 0.0);
    let mut baseline = Vec::new();
    for _ in 0..5 {
        step(&mut f.app);
        baseline.push(observe(
            &f.app,
            f.shooter,
            f.victim,
            &mut balance,
            &mut wire,
        ));
    }
    println!(
        "\nPHOENIX_DAMAGE_LIVE={} ",
        json!({"role":role,"case":case,"prepared":f.prepared,"before":before,"baseline":baseline})
    );
    // Restore over the same ordinary boot/fixture topology. No final outcomes,
    // event lists, RNG positions or damage are copied from the completed branch.
    let mut resumed = prepare(role == "pinned", case == "near-lethal");
    let report = snapshot::restore(resumed.app.world_mut(), &checkpoint);
    println!(
        "\nPHOENIX_DAMAGE_RESTORE={} ",
        json!({"role":role,"case":case,"complete":report.is_complete(),"gaps":format!("{:?}",report.gaps),"expected_digest":format!("{checkpoint_digest:016x}"),"actual_digest":format!("{:016x}",world_digest(resumed.app.world()))})
    );
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(
        world_digest(resumed.app.world()),
        checkpoint_digest,
        "exact restore boundary"
    );
    let mut balance = MessageCursor::default();
    let mut wire = MessageCursor::default();
    balance
        .read(resumed.app.world().resource::<Messages<BalanceEvent>>())
        .for_each(drop);
    wire.read(resumed.app.world().resource::<Messages<OutboundMessage>>())
        .for_each(drop);
    let mut restored = Vec::new();
    for _ in 0..5 {
        step(&mut resumed.app);
        restored.push(observe(
            &resumed.app,
            resumed.shooter,
            resumed.victim,
            &mut balance,
            &mut wire,
        ));
    }
    println!(
        "\n{PREFIX}{}",
        json!({"schema":1,"role":role,"case":case,"world":WORLD,"hull":HULL,"seed":SEED,"pool_workers":bevy::tasks::ComputeTaskPool::get().thread_num(),"executor":format!("{:?}",f.app.get_schedule(FixedUpdate).unwrap().get_executor_kind()),"period_nanos":f.period.as_nanos(),"checkpoint_digest":format!("{checkpoint_digest:016x}"),"prepared":f.prepared,"before":before,"expected_restore_refresh":expected_restore_refresh,"baseline":baseline,"restored":restored})
    );
    // Restore deliberately dirties the objective projection. The publisher is
    // ordered before Sim dispatch, so its exact refresh drains in this impact
    // tick, before the fixed StateTransition can enter GameOver. Retain that
    // wire observation above, prove its payload/cardinality/position, then
    // compare every other message and observation unchanged.
    let expected_restore_wire = json!({
        "target": "All",
        "delivery": "Reliable",
        "message": &expected_restore_refresh[1],
    });
    let mut comparable_restored = restored.clone();
    for (index, row) in comparable_restored.iter_mut().enumerate() {
        if case == "near-lethal" {
            assert_eq!(row["phase"], "GameOver");
        }
        assert!(
            row["pending_outbox"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry != &expected_restore_refresh),
            "the restore refresh must not remain queued after the impact tick at row {index}"
        );
        let wire = row["wire"].as_array_mut().unwrap();
        let refresh_positions: Vec<_> = wire
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| (entry == &expected_restore_wire).then_some(i))
            .collect();
        let expected_count = usize::from(index == 0);
        assert_eq!(refresh_positions.len(), expected_count,
            "the exact reliable restore refresh is delivered once on the impact tick at row {index}");
        if expected_count == 1 {
            let position = refresh_positions[0];
            let original_wire = baseline[index]["wire"].as_array().unwrap();
            let expected_position = original_wire
                .iter()
                .position(|entry| entry["message"]["type"] == "GameOver")
                .unwrap_or(original_wire.len());
            assert_eq!(
                position, expected_position,
                "the refresh follows impact publications and precedes the terminal transition"
            );
            assert_eq!(wire.remove(position), expected_restore_wire);
        }
    }
    assert_eq!(
        baseline, comparable_restored,
        "uninterrupted versus restored complete observable trace"
    );
    assert_eq!(
        baseline[0]["tick"].as_u64().unwrap(),
        checkpoint.tick + 1,
        "the simultaneous impact must be an advancing simulation tick"
    );
    let events: Vec<_> = baseline[0]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["fact"])
        .filter(|e| e["event"] == "DamageApplied" && e["victim"] == target)
        .collect();
    // The survivor is the all-family witness. A lethal consumer may suppress
    // later damage/hit emission; the near-lethal trace must retain that fact.
    if case == "survive" {
        for weapon in ["omni", "port", "collision"] {
            assert!(
                events
                    .iter()
                    .any(|e| e["weapon"] == weapon && e["amount"].as_f64().unwrap() > 0.0),
                "real simultaneous {weapon} damage"
            );
        }
        assert!(
            baseline[0]["wire"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["message"]["type"] == "BlasterHit"),
            "ordinary reliable hit projection"
        );
        assert!(
            active_contact(&mut f.app, f.victim, f.wall)
                || baseline[0]["ships"][1]["collision_cooldown"]
                    .as_f64()
                    .unwrap()
                    > 0.0,
            "contact was handled"
        );
    }
    assert!(
        events.iter().all(|event| {
            if event["weapon"] == "collision" {
                event["attacker"].is_null()
            } else {
                matches!(event["weapon"].as_str(), Some("omni" | "port"))
                    && event["attacker"] == SHOOTER
            }
        }),
        "actual damage retains its producer attribution"
    );
    assert!(
        events
            .iter()
            .any(|e| e["shield_absorbed"].as_f64().unwrap() > 0.0),
        "real shield absorption"
    );
    assert!(
        events
            .iter()
            .any(|e| e["hull_damage"].as_f64().unwrap() > 0.0),
        "real hull damage"
    );
    assert!(
        baseline[0]["wire"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["message"]["type"] == "DamageTaken"),
        "ordinary crew damage projection"
    );
    assert_ne!(
        before["rng"], baseline[0]["rng"],
        "damage must consume seeded state"
    );
    assert!(
        baseline[0]["ships"][0]["blasters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| !b["projectiles"].as_array().unwrap().is_empty()),
        "an actual other bolt survives"
    );
    if case == "near-lethal" {
        // OnEnter has already consumed the reason at the observed boundary.
        // The actual reliable terminal message, retained in both raw traces,
        // owns the exact reason; the resource still retains its Outcome.
        for row in &baseline {
            assert_eq!(row["phase"], "GameOver");
            assert!(row["reason"].is_null());
            assert_eq!(row["outcome"], "Some(Defeat)");
        }
        let terminal_messages: Vec<_> = baseline
            .iter()
            .flat_map(|row| row["wire"].as_array().unwrap())
            .filter(|row| row["message"]["type"] == "GameOver")
            .collect();
        assert_eq!(
            terminal_messages.len(),
            1,
            "one ordinary terminal crew projection"
        );
        let terminal = terminal_messages[0];
        assert_eq!(terminal["target"], "All");
        assert_eq!(terminal["delivery"], "Reliable");
        assert_eq!(
            terminal["message"]["data"]["reason"],
            "server.game_over.ship_destroyed"
        );
        assert_eq!(terminal["message"]["data"]["outcome"], "defeat");
        let deaths: Vec<_> = baseline
            .iter()
            .flat_map(|r| r["events"].as_array().unwrap())
            .map(|row| &row["fact"])
            .filter(|event| event["event"] == "EntityDestroyed" && event["victim"] == target)
            .collect();
        assert!(!deaths.is_empty(), "real victim destruction");
        assert!(
            deaths
                .iter()
                .all(|event| event["killer"].is_null() || event["killer"] == SHOOTER),
            "record the actual environmental or shooter kill credit"
        );
    } else {
        assert_eq!(baseline.last().unwrap()["phase"], "InProgress");
    }
}
