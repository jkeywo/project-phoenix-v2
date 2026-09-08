//! Fixture-only observation of the existing reconnect boundary (#1400).
//! No schedule edges, callbacks, authored data or runtime APIs are replaced.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{ecs::message::MessageCursor, prelude::*};
use project_phoenix::{
    console::{repair::visibility::LastBroadcastHull, weapons::blackboard::LastWeaponsUpdate},
    core::messages::{ClientMessage, DeliveryClass, ServerMessage, SystemControlPayload, SystemId},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{
        server::{InboundMessage, OutboundMessage},
        Sessions, Target,
    },
    server_app::{LastBroadcastBlackboards, LocalShip},
    ship::shields::ShipShields,
    sim_digest::world_digest,
    sim_rng::SimRng,
    sim_tick::{sim_tick_period, SimTick},
    world::config::WorldConfig,
};
use serde_json::{json, Value};

const OWNER: &str = "reconnect-science";
const OTHER: &str = "unaffected-helm";
const SEED: u64 = 1_400_198;

fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}

fn identify(app: &mut App, token: &str) {
    send(
        app,
        token,
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
    );
}

fn step(app: &mut App) {
    let before = app.world().resource::<SimTick>().0;
    app.update();
    assert_eq!(
        app.world().resource::<SimTick>().0,
        before + 1,
        "one authored fixed period"
    );
}

fn ship(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}

fn assert_ready_pair(app: &mut App, owner_station: &str) -> Value {
    use project_phoenix::{
        core::messages::StationId,
        ship_plugin::{ActiveStationRatings, ShipSystemControlSources},
    };
    let entity = ship(app);
    let world = app.world();
    let sessions = &world.resource::<Sessions>().0;
    let ratings = &world.get::<ActiveStationRatings>(entity).unwrap().0;
    for (token, station) in [(OWNER, owner_station), (OTHER, "helm")] {
        let id = StationId(station.into());
        let player = sessions
            .players()
            .iter()
            .find(|p| p.token == token)
            .unwrap();
        assert!(
            player.connected && player.ready,
            "real connected, ready participant"
        );
        assert_eq!(player.station.as_ref(), Some(&id));
        assert_eq!(sessions.holder_for_station(&id), Some(token));
        assert_eq!(ratings.get(&id).map(String::as_str), Some("Std"));
    }
    let sources = &world.get::<ShipSystemControlSources>(entity).unwrap().0;
    let owner_system = if owner_station == "science" {
        "shields-system"
    } else {
        "tactical-radar"
    };
    let mut policies = Vec::new();
    for name in [owner_system, "helm-thrust"] {
        let id = SystemId(name.into());
        assert!(
            sources.entries().any(|(key, _)| key == &id),
            "actual registered system"
        );
        let policy = sources.policy_for(&id);
        assert!(
            policy.accept_human_input && !policy.operate_ai,
            "ready Std controls are Human"
        );
        policies.push(json!({"system":name,"policy":format!("{policy:?}")}));
    }
    json!({"owner_station":owner_station,"rating":"Std","policies":policies})
}

fn ready_pair(app: &mut App, owner_station: &str) {
    // A mid-game claim is pending/Backfill until the real Ready operation.
    // Without this, Identify restores Std only on the reconnect host and the
    // comparison changes command eligibility instead of observing snapshots.
    for token in [OWNER, OTHER] {
        send(app, token, ClientMessage::SetReady { ready: true });
    }
    step(app);
    assert_ready_pair(app, owner_station);
}

fn shields(app: &mut App) -> Value {
    let entity = ship(app);
    let shields = app.world().get::<ShipShields>(entity).unwrap();
    json!({"facings": format!("{:?}", shields.0.snapshot()), "frequency": shields.frequency(),
        "focus": shields.0.focused_facing})
}

fn caches(app: &App) -> Value {
    // Sort the exposed maps rather than relying on HashMap debug order. These
    // are existing shared delta caches, never substitute authoritative state.
    let mut blackboards: Vec<_> = app
        .world()
        .resource::<LastBroadcastBlackboards>()
        .0
        .iter()
        .collect();
    blackboards.sort_by(|a, b| a.0.cmp(b.0));
    let mut hull: Vec<_> = app
        .world()
        .resource::<LastBroadcastHull>()
        .0
        .iter()
        .collect();
    hull.sort_by(|a, b| a.0.cmp(b.0));
    json!({"blackboards": blackboards, "hull": format!("{hull:?}"),
        "weapons": format!("{:?}", app.world().resource::<LastWeaponsUpdate>())})
}

fn read(app: &App, cursor: &mut MessageCursor<OutboundMessage>) -> Vec<Value> {
    cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .map(|m| {
            json!({
                "target": format!("{:?}", m.target), "delivery": format!("{:?}", m.delivery), "message": m.msg,
            })
        })
        .collect()
}

fn other_snapshots(app: &App, cursor: &mut MessageCursor<OutboundMessage>) -> Vec<Value> {
    cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .filter(|m| {
            m.delivery == DeliveryClass::Snapshot
                && match &m.target {
                    Target::All => true,
                    Target::Token(token) => token == OTHER,
                    Target::AllExcept(token) => token != OTHER,
                }
        })
        .map(|m| json!({"target": format!("{:?}", m.target), "delivery": format!("{:?}", m.delivery), "message": m.msg}))
        .collect()
}

fn count_targeted(app: &App, cursor: &mut MessageCursor<OutboundMessage>, shield: bool) -> usize {
    cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .filter(|m| {
            m.target == Target::Token(OWNER.into())
                && if shield {
                    m.delivery == DeliveryClass::Snapshot
                        && matches!(m.msg, ServerMessage::ShieldStatus { .. })
                } else {
                    m.delivery == DeliveryClass::Reliable
                        && matches!(m.msg, ServerMessage::Welcome { .. })
                }
        })
        .count()
}

#[test]
#[ignore = "source-bound default/default/pinned reconnect baseline; no production ordering prescribed"]
fn observe_same_tick_shield_change_and_identify() {
    // Each mode must be launched in a fresh process because pools are global.
    // Programmatic seed alone does not request deterministic boot.
    let pinned = match std::env::var("PHOENIX_RECONNECT_PINNED").as_deref() {
        Ok("1") => true,
        Err(std::env::VarError::NotPresent) => false,
        other => panic!("unexpected test-only mode: {other:?}"),
    };
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_fleet_duel.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(SEED),
        deterministic: pinned,
        ..Default::default()
    };
    let mut apps = [
        build_headless_app(&args).unwrap(),
        build_headless_app(&args).unwrap(),
    ];
    for app in &mut apps {
        let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
        app.finish();
        app.cleanup();
        for _ in 0..6 {
            app.update();
        }
        assert_eq!(app.world().resource::<Time<Fixed>>().timestep(), period);
        for token in [OWNER, OTHER] {
            identify(app, token);
        }
        step(app);
        send(
            app,
            OWNER,
            ClientMessage::SelectStation {
                station: "science".into(),
            },
        );
        send(
            app,
            OTHER,
            ClientMessage::SelectStation {
                station: "helm".into(),
            },
        );
        step(app);
        assert_eq!(
            app.world().resource::<Sessions>().0.holder_for_station(
                &project_phoenix::core::messages::StationId("science".into())
            ),
            Some(OWNER)
        );
        assert_eq!(
            app.world()
                .resource::<Sessions>()
                .0
                .holder_for_station(&project_phoenix::core::messages::StationId("helm".into())),
            Some(OTHER)
        );
        ready_pair(app, "science");
        for _ in 0..20 {
            step(app);
        }
        assert_eq!(app.world().resource::<SimRng>().seed(), SEED);
    }
    let workers = bevy::tasks::ComputeTaskPool::get().thread_num();
    let expected = if pinned {
        assert_eq!(workers, 1);
        bevy::ecs::schedule::ExecutorKind::SingleThreaded
    } else {
        assert!(workers > 1);
        bevy::ecs::schedule::ExecutorKind::MultiThreaded
    };
    for app in &apps {
        assert_eq!(
            app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
            expected
        );
    }
    println!(
        "\nPHOENIX_RECONNECT_EXECUTION={}",
        json!({"process":std::process::id(),
        "compute_threads":workers,"executor":format!("{expected:?}"),"seed":SEED})
    );

    let mut all = [MessageCursor::default(), MessageCursor::default()];
    let mut others = [MessageCursor::default(), MessageCursor::default()];
    let mut shield_counts = [MessageCursor::default(), MessageCursor::default()];
    let mut welcome_counts = [MessageCursor::default(), MessageCursor::default()];
    for i in 0..2 {
        read(&apps[i], &mut all[i]);
        other_snapshots(&apps[i], &mut others[i]);
        count_targeted(&apps[i], &mut shield_counts[i], true);
        count_targeted(&apps[i], &mut welcome_counts[i], false);
    }
    let entity = ship(&mut apps[0]);
    assert_eq!(
        assert_ready_pair(&mut apps[0], "science"),
        assert_ready_pair(&mut apps[1], "science")
    );
    let current = apps[0]
        .world()
        .get::<ShipShields>(entity)
        .unwrap()
        .0
        .focused_facing;
    let next = apps[0]
        .world()
        .get::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter()
        .enumerate()
        .find(|(index, _)| Some(*index) != current)
        .map(|(index, arc)| (index, arc.id.clone()))
        .unwrap();
    let delay = apps[0]
        .world()
        .resource::<project_phoenix::command_admission::log::CommandDelay>()
        .0;
    let apply_tick = apps[0].world().resource::<SimTick>().0 + delay;
    for app in &mut apps {
        assert_eq!(
            app.world()
                .resource::<project_phoenix::command_admission::log::CommandDelay>()
                .0,
            delay
        );
        send(
            app,
            OWNER,
            ClientMessage::ControlSystem {
                target: SystemId(format!("shield-arc-{}", next.1)),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            },
        );
    }
    // Preserve the real admission delay. Advance only to the command's due
    // tick, then enqueue Identify before that fixed step executes.
    for _ in 0..delay {
        for app in &mut apps {
            step(app);
        }
        assert_eq!(world_digest(apps[0].world()), world_digest(apps[1].world()));
    }
    for i in 0..2 {
        assert_eq!(apps[i].world().resource::<SimTick>().0, apply_tick);
        assert_eq!(
            shields(&mut apps[i])["focus"],
            json!(current),
            "command not applied early"
        );
        read(&apps[i], &mut all[i]);
        other_snapshots(&apps[i], &mut others[i]);
        count_targeted(&apps[i], &mut shield_counts[i], true);
        count_targeted(&apps[i], &mut welcome_counts[i], false);
    }
    let mut trace = Vec::new();
    for frame in 0..12 {
        let before = [shields(&mut apps[0]), shields(&mut apps[1])];
        if frame == 0 {
            // The control host gets the identical ordinary command but no
            // reconnect. Neither host receives synthetic outbox payloads.
            identify(&mut apps[0], OWNER);
        }
        for app in &mut apps {
            step(app);
        }
        let after = [shields(&mut apps[0]), shields(&mut apps[1])];
        assert_eq!(
            after[0], after[1],
            "reconnect must not alter shield state at {frame}"
        );
        if frame == 0 {
            assert_eq!(
                after[0]["focus"],
                json!(next.0),
                "ordinary command actually applied"
            );
            assert_ne!(
                before[0]["focus"], after[0]["focus"],
                "non-vacuous mutation"
            );
        }
        let boundary = [
            (
                apps[0].world().resource::<SimTick>().0,
                world_digest(apps[0].world()),
            ),
            (
                apps[1].world().resource::<SimTick>().0,
                world_digest(apps[1].world()),
            ),
        ];
        assert_eq!(
            boundary[0], boundary[1],
            "reconnect authoritative neutrality at {frame}"
        );
        let cache = [caches(&apps[0]), caches(&apps[1])];
        assert_eq!(cache[0], cache[1], "shared delta caches at {frame}");
        let other = [
            other_snapshots(&apps[0], &mut others[0]),
            other_snapshots(&apps[1], &mut others[1]),
        ];
        assert_eq!(
            other[0], other[1],
            "unaffected client's actual snapshot stream at {frame}"
        );
        let shield = [
            count_targeted(&apps[0], &mut shield_counts[0], true),
            count_targeted(&apps[1], &mut shield_counts[1], true),
        ];
        let welcome = [
            count_targeted(&apps[0], &mut welcome_counts[0], false),
            count_targeted(&apps[1], &mut welcome_counts[1], false),
        ];
        if frame == 0 {
            assert_eq!(welcome, [1, 0]);
            assert_eq!(
                shield[0],
                shield[1] + 1,
                "registered Shields resync is actually delivered"
            );
        } else {
            assert_eq!(welcome, [0, 0]);
        }
        trace.push(json!({"frame":frame,"boundary":boundary[0],"before":before,"after":after,
            "shared_cache":cache[0],"unaffected_snapshot_output":other[0],
            "reconnect_output":read(&apps[0], &mut all[0]),"control_output":read(&apps[1], &mut all[1])}));
    }
    println!(
        "\nPHOENIX_RECONNECT_TRACE={}",
        json!({"delay":delay,"apply_tick":apply_tick,"frames":trace})
    );
}

fn count_weapons(app: &App, cursor: &mut MessageCursor<OutboundMessage>) -> usize {
    cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .filter(|m| {
            m.target == Target::Token(OWNER.into())
                && m.delivery == DeliveryClass::Snapshot
                && matches!(m.msg, ServerMessage::WeaponsUpdate { .. })
        })
        .count()
}
// Draft extension; not yet appended to the frozen reviewed test.
fn weapons_source(app: &mut App) -> Value {
    use project_phoenix::{
        console::weapons::{beam::TacticalRadarSelection, TorpedoSystemResource},
        core::messages::SystemBlackboard,
        server_app::ShipSystemBlackboards,
    };
    let entity = ship(app);
    let world = app.world();
    let torpedoes = &world.get::<TorpedoSystemResource>(entity).unwrap().0;
    let lock = match world
        .get::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .get(&project_phoenix::ship::system_registry::viewscreen_system_id())
    {
        Some(SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
        _ => None,
    };
    json!({"lock":lock,"selection":world.get::<TacticalRadarSelection>(entity).unwrap().0,
        "magazine":torpedoes.torpedoes_remaining,"torpedoes":format!("{torpedoes:?}")})
}

fn control(app: &mut App, target: &str, payload: SystemControlPayload) {
    send(
        app,
        OWNER,
        ClientMessage::ControlSystem {
            target: SystemId(target.into()),
            payload,
        },
    );
}

#[test]
#[ignore = "source-bound default/default/pinned weapon reconnect baseline"]
fn observe_prior_lock_and_live_ammo_on_reconnect() {
    use project_phoenix::{
        console::weapons::TorpedoSystemResource, entities::spawner::EntityUuid,
        weapons::torpedo::TubeLoadState,
    };
    let pinned = match std::env::var("PHOENIX_RECONNECT_PINNED").as_deref() {
        Ok("1") => true,
        Err(std::env::VarError::NotPresent) => false,
        other => panic!("unexpected mode {other:?}"),
    };
    let args = HeadlessArgs {
        world_path: "assets/worlds/probe_fleet_duel.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(SEED),
        deterministic: pinned,
        ..Default::default()
    };
    let mut apps = [
        build_headless_app(&args).unwrap(),
        build_headless_app(&args).unwrap(),
    ];
    for app in &mut apps {
        let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
        app.finish();
        app.cleanup();
        for _ in 0..6 {
            app.update();
        }
        for token in [OWNER, OTHER] {
            identify(app, token);
        }
        step(app);
        send(
            app,
            OWNER,
            ClientMessage::SelectStation {
                station: "tactical".into(),
            },
        );
        send(
            app,
            OTHER,
            ClientMessage::SelectStation {
                station: "helm".into(),
            },
        );
        step(app);
        assert_eq!(
            app.world().resource::<Sessions>().0.holder_for_station(
                &project_phoenix::core::messages::StationId("tactical".into())
            ),
            Some(OWNER)
        );
        ready_pair(app, "tactical");
    }
    let workers = bevy::tasks::ComputeTaskPool::get().thread_num();
    let expected = if pinned {
        assert_eq!(workers, 1);
        bevy::ecs::schedule::ExecutorKind::SingleThreaded
    } else {
        assert!(workers > 1);
        bevy::ecs::schedule::ExecutorKind::MultiThreaded
    };
    for app in &apps {
        assert_eq!(
            app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
            expected
        );
        assert_eq!(app.world().resource::<SimRng>().seed(), SEED);
    }
    println!(
        "\nPHOENIX_RECONNECT_EXECUTION={}",
        json!({"process":std::process::id(),"compute_threads":workers,"executor":format!("{expected:?}"),"seed":SEED})
    );
    let delay = apps[0]
        .world()
        .resource::<project_phoenix::command_admission::log::CommandDelay>()
        .0;
    assert!(delay < 60, "bounded authored probe delay");
    let local = ship(&mut apps[0]);
    let mut nearby: Vec<_> = apps[0]
        .world_mut()
        .query_filtered::<(Entity, &EntityUuid, &Transform), With<project_phoenix::server_app::Ship>>()
        .iter(apps[0].world())
        .filter(|(e, _, _)| *e != local)
        .map(|(_, id, t)| (t.translation.length_squared(), id.0.clone()))
        .collect();
    nearby.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let target = nearby
        .first()
        .expect("authored nearby fleet hull")
        .1
        .clone();
    // Fixture placement only: the authored spare hull starts beyond this
    // cruiser's tactical range. Keep its configuration and motion state, but
    // place its authoritative and rendered positions together within reach.
    let mut target_eligibility = Vec::new();
    for app in &mut apps {
        use project_phoenix::{
            core::messages::ModifierSlot, entities::spawner::WeaponsConsoleSection,
            modifiers::ShipModifiers, ship::state::ShipPhysics,
        };
        let owner = ship(app);
        let owner_physics = *app.world().get::<ShipPhysics>(owner).unwrap();
        let target_entity = app
            .world_mut()
            .query_filtered::<(Entity, &EntityUuid), With<project_phoenix::server_app::Ship>>()
            .iter(app.world())
            .find_map(|(entity, uuid)| (uuid.0 == target).then_some(entity))
            .expect("same existing authored target is live on both hosts");
        assert_ne!(owner, target_entity);
        let position = Vec3::new(owner_physics.x, owner_physics.y, owner_physics.z)
            + Quat::from_rotation_y(owner_physics.yaw) * Vec3::new(0.0, 0.0, -20.0);
        {
            let mut physics = app
                .world_mut()
                .get_mut::<ShipPhysics>(target_entity)
                .unwrap();
            physics.x = position.x;
            physics.y = position.y;
            physics.z = position.z;
        }
        app.world_mut()
            .get_mut::<Transform>(target_entity)
            .unwrap()
            .translation = position;
        let base_range = app
            .world()
            .get::<WeaponsConsoleSection>(owner)
            .unwrap()
            .0
            .radar
            .as_ref()
            .unwrap()
            .range;
        let multiplier = app
            .world()
            .get::<ShipModifiers>(owner)
            .map(|m| m.get(&ModifierSlot::RadarRange))
            .unwrap_or(1.0);
        let effective_range = base_range * multiplier;
        let separation =
            Vec2::new(position.x - owner_physics.x, position.z - owner_physics.z).length();
        assert!(effective_range.is_finite() && effective_range > separation);
        assert!((separation - 20.0).abs() < 0.001);
        let physics = app.world().get::<ShipPhysics>(target_entity).unwrap();
        assert_eq!(
            Vec3::new(physics.x, physics.y, physics.z),
            app.world()
                .get::<Transform>(target_entity)
                .unwrap()
                .translation
        );
        target_eligibility.push(json!({"target":target,"position":position.to_array(),
            "owner":format!("{owner_physics:?}"),"target_physics":format!("{physics:?}"),
            "effective_range":effective_range,"separation":separation,
            "crew":assert_ready_pair(app,"tactical")}));
    }
    assert_eq!(
        target_eligibility[0], target_eligibility[1],
        "identical live target and source eligibility before ordinary priming"
    );
    let tube_ids: Vec<_> = apps[0]
        .world()
        .get::<TorpedoSystemResource>(local)
        .unwrap()
        .0
        .tubes
        .iter()
        .map(|t| t.id.clone())
        .collect();
    assert!(!tube_ids.is_empty());
    // Ordinary controls cancel early in-flight loads and stop automatic refill.
    // No tube config, loaded count, magazine or target UUID is written directly.
    for app in &mut apps {
        for id in &tube_ids {
            let system =
                project_phoenix::ship::system_registry::torpedo_tube_system_id(id).unwrap();
            control(
                app,
                &system.0,
                SystemControlPayload::SetTorpedoVolleyTarget { count: 0 },
            );
            control(app, &system.0, SystemControlPayload::UnloadTube);
        }
        control(
            app,
            "tactical-radar",
            SystemControlPayload::SetTarget {
                uuid: target.clone(),
            },
        );
    }
    for _ in 0..delay + 3 {
        for app in &mut apps {
            step(app);
        }
    }
    let tube_id = tube_ids[0].clone();
    let system = project_phoenix::ship::system_registry::torpedo_tube_system_id(&tube_id).unwrap();
    for app in &mut apps {
        let entity = ship(app);
        let torp = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
        assert!(
            torp.tubes
                .iter()
                .all(|t| t.loaded_count == 0 && t.load_state == TubeLoadState::Unloaded),
            "early ordinary cancel must settle before observation"
        );
        assert_eq!(
            weapons_source(app)["lock"],
            json!(target),
            "ordinary target reached published combat lock"
        );
        control(app, &system.0, SystemControlPayload::LoadTube);
    }
    let mut previous_remaining = [None; 2];
    for _ in 0..delay + 3 {
        for (index, app) in apps.iter_mut().enumerate() {
            step(app);
            let entity = ship(app);
            let torpedoes = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
            let tube = torpedoes
                .tubes
                .iter()
                .find(|tube| tube.id == tube_id)
                .unwrap();
            if let TubeLoadState::Loading { remaining, total } = tube.load_state {
                let expected = previous_remaining[index].map_or(tube.load_time, |previous: f32| {
                    previous - app.world().resource::<Time<Fixed>>().delta_secs()
                });
                assert_eq!(total, tube.load_time);
                assert_eq!(
                    remaining, expected,
                    "a reserved round starts after lifecycle and advances next tick"
                );
                previous_remaining[index] = Some(remaining);
            } else {
                assert!(previous_remaining[index].is_none());
            }
        }
        assert_eq!(
            weapons_source(&mut apps[0]),
            weapons_source(&mut apps[1]),
            "reserved loading starts at the same lifecycle boundary"
        );
        assert_eq!(caches(&apps[0]), caches(&apps[1]));
    }
    assert!(previous_remaining.iter().all(Option::is_some));
    for app in &mut apps {
        let entity = ship(app);
        let torp = &app.world().get::<TorpedoSystemResource>(entity).unwrap().0;
        assert!(
            matches!(
                torp.tubes
                    .iter()
                    .find(|t| t.id == tube_id)
                    .unwrap()
                    .load_state,
                TubeLoadState::Loading { .. }
            ),
            "real reserved round is actively loading"
        );
    }
    assert_eq!(
        assert_ready_pair(&mut apps[0], "tactical"),
        assert_ready_pair(&mut apps[1], "tactical")
    );
    let apply_tick = apps[0].world().resource::<SimTick>().0 + delay;
    for app in &mut apps {
        control(
            app,
            "tactical-radar",
            SystemControlPayload::SetTarget {
                uuid: String::new(),
            },
        );
        control(app, &system.0, SystemControlPayload::UnloadTube);
    }
    for _ in 0..delay {
        for app in &mut apps {
            step(app);
        }
    }
    let mut all = [MessageCursor::default(), MessageCursor::default()];
    let mut others = [MessageCursor::default(), MessageCursor::default()];
    for i in 0..2 {
        assert_eq!(apps[i].world().resource::<SimTick>().0, apply_tick);
        read(&apps[i], &mut all[i]);
        other_snapshots(&apps[i], &mut others[i]);
    }
    let mut weapons_counts = [MessageCursor::default(), MessageCursor::default()];
    for i in 0..2 {
        count_weapons(&apps[i], &mut weapons_counts[i]);
    }
    let mut trace = Vec::new();
    for frame in 0..12 {
        let before = [weapons_source(&mut apps[0]), weapons_source(&mut apps[1])];
        if frame == 0 {
            assert_eq!(before[0]["lock"], json!(target));
            identify(&mut apps[0], OWNER);
        }
        for app in &mut apps {
            step(app);
        }
        let after = [weapons_source(&mut apps[0]), weapons_source(&mut apps[1])];
        assert_eq!(after[0], after[1]);
        if frame == 0 {
            assert_eq!(after[0]["selection"], Value::Null);
            assert_eq!(
                after[0]["magazine"].as_u64().unwrap(),
                before[0]["magazine"].as_u64().unwrap() + 1,
                "ordinary unload returns its reserved round"
            );
        }
        let digests = [world_digest(apps[0].world()), world_digest(apps[1].world())];
        assert_eq!(digests[0], digests[1]);
        let cache = [caches(&apps[0]), caches(&apps[1])];
        assert_eq!(cache[0], cache[1]);
        let other = [
            other_snapshots(&apps[0], &mut others[0]),
            other_snapshots(&apps[1], &mut others[1]),
        ];
        assert_eq!(other[0], other[1]);
        let projected = [
            count_weapons(&apps[0], &mut weapons_counts[0]),
            count_weapons(&apps[1], &mut weapons_counts[1]),
        ];
        if frame == 0 {
            assert_eq!(
                projected[0],
                projected[1] + 1,
                "actual registered Weapons reconnect projection"
            );
        }
        let output = [read(&apps[0], &mut all[0]), read(&apps[1], &mut all[1])];
        trace.push(json!({"frame":frame,"tick":apps[0].world().resource::<SimTick>().0,"digest":digests[0],"before":before,"after":after,"cache":cache[0],"unaffected_snapshot_output":other[0],"reconnect_output":output[0],"control_output":output[1]}));
    }
    println!(
        "\nPHOENIX_RECONNECT_WEAPONS_TRACE={}",
        json!({"delay":delay,"apply_tick":apply_tick,"frames":trace})
    );
}
