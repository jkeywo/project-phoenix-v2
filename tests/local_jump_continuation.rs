//! Joined actual drive preparation, atomic fuel commitment, arrival and replenishment.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    console::helm::transit::ShipTransitDrives,
    core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control, SystemId},
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::logistics_server::ShipLogistics,
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn token(station: &str) -> String {
    format!("jump-{station}")
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn boot() -> (App, Entity) {
    boot_with(
        "tests/fixtures/local-jump-world.toml",
        "assets/entities/local_jump_cruiser.toml",
    )
}
fn boot_with(world_path: &str, ship_path: &str) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: world_path.into(),
        ship_path: ship_path.into(),
        deterministic: true,
        seed: Some(181820),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let stations = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .clone();
    for station in stations {
        let token = token(&station.id.0);
        for msg in [
            ClientMessage::Identify {
                token: token.clone(),
                name: token.clone(),
            },
            ClientMessage::SelectStation {
                station: station.id.0.clone(),
            },
            ClientMessage::SetReady { ready: true },
            ClientMessage::SetStationRating {
                rating_name: station.ratings[0].name.clone(),
            },
        ] {
            send(&mut app, &token, msg);
            steps(&mut app, 4);
        }
    }
    // All actual crew are Human: existing deliveries may finish, but no AI
    // fabricates a replacement order while the finite stock baseline settles.
    for _ in 0..1800 {
        if app
            .world()
            .get::<ShipLogistics>(entity)
            .unwrap()
            .0
            .batches
            .is_empty()
        {
            break;
        }
        app.update();
    }
    assert!(app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .is_empty());
    (app, entity)
}
fn human_token(app: &App, entity: Entity, owner: &str, payload: &Control) -> Option<String> {
    let sessions = app.world().resource::<Sessions>();
    let sources = app.world().get::<ShipSystemControlSources>(entity).unwrap();
    let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    sessions
        .0
        .players()
        .iter()
        .find(|player| {
            project_phoenix::command_admission::policy::is_command_authorized(
                &player.token,
                &id(owner),
                payload,
                sources,
                sessions,
                config,
                app.world().get::<HumanSeekingHosts>(entity),
            )
        })
        .map(|player| player.token.clone())
}
fn control(app: &mut App, entity: Entity, owner: &str, payload: Control) {
    let token = human_token(app, entity, owner, &payload)
        .unwrap_or_else(|| panic!("actual Human authority for {owner}"));
    send(
        app,
        &token,
        ClientMessage::ControlSystem {
            target: id(owner),
            payload,
        },
    );
    steps(app, 4);
}
fn phase(app: &App, entity: Entity) -> serde_json::Value {
    serde_json::to_value(
        &app.world().get::<ShipTransitDrives>(entity).unwrap().0[&id("local-jump")].phase,
    )
    .unwrap()
}
fn amount(app: &App, entity: Entity, capacity: &str) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity(capacity)
        .unwrap()
}
fn fuel(app: &App, entity: Entity) -> i64 {
    // Dispatch withdraws actual source Stock immediately, including Loading.
    let carried = app
        .world()
        .get::<ShipLogistics>(entity)
        .unwrap()
        .0
        .batches
        .values()
        .filter(|batch| batch.source == "hold-transit-fuel")
        .map(|batch| batch.amount)
        .sum::<i64>();
    amount(app, entity, "hold-transit-fuel") + amount(app, entity, "drive-transit-fuel") + carried
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => a
            .keys()
            .chain(b.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .find_map(|key| match (a.get(key), b.get(key)) {
                (Some(a), Some(b)) => first_difference(a, b, &format!("{path}.{key}")),
                _ => Some(format!("{path}.{key}: presence")),
            }),
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            if a.len() != b.len() {
                Some(format!("{path}: length{} vs{}", a.len(), b.len()))
            } else {
                a.iter()
                    .zip(b)
                    .enumerate()
                    .find_map(|(i, (a, b))| first_difference(a, b, &format!("{path}[{i}]")))
            }
        }
        _ => (a != b).then(|| format!("{path}: {a} vs {b}")),
    }
}
fn same(a: &App, b: &App, step: usize) {
    let left = serde_json::to_value(capture(a.world())).unwrap();
    let right = serde_json::to_value(capture(b.world())).unwrap();
    assert!(
        left == right,
        "complete continuation at{step}: {:?}",
        first_difference(&left, &right, "$")
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest at{step}"
    );
}
#[test]
fn actual_joined_partial_jump_json360_handoff_arrival_and_finite_replenishment() {
    let (mut live, entity) = boot();
    assert_eq!(fuel(&live, entity), 24);
    control(
        &mut live,
        entity,
        "navigation",
        Control::SetNavigationWaypoint {
            x: 100.0,
            z: 30.0,
            source_uuid: None,
        },
    );
    // An ordinary course is not an implicit Transit order.
    assert_eq!(phase(&live, entity)["phase"], "idle");
    control(
        &mut live,
        entity,
        "local-jump",
        Control::BeginTransitToWaypoint { request: None },
    );
    assert_eq!(phase(&live, entity)["phase"], "preparing");
    assert!(
        live.world().get::<ShipTransitDrives>(entity).unwrap().0[&id("local-jump")].worked_secs()
            > 0.0
    );
    assert!(
        live.world()
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .emitted_work(&id("local-jump"))
            > 0.0
    );
    assert_eq!(
        fuel(&live, entity),
        24,
        "preparation has not committed physical fuel"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut bad = saved.clone();
        let tick = bad.tick;
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.transit.is_some())
            .unwrap();
        if fault == 0 {
            row.transit = None;
        } else if fault == 1 {
            row.transit
                .as_mut()
                .unwrap()
                .0
                .get_mut(&id("local-jump"))
                .unwrap()
                .last_tick = Some(tick + 1);
        } else {
            row.physics = None;
        }
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "fault{fault}"
        );
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    let mut arrived = false;
    for step in 0..360 {
        same(&live, &resumed, step);
        for (app, actor) in [(&live, entity), (&resumed, restored)] {
            let phase = phase(app, actor);
            let committed = matches!(phase["phase"].as_str(), Some("committed" | "arrived"));
            assert_eq!(fuel(app, actor), if committed { 22 } else { 24 });
        }
        if step == 30 {
            // Actual human handoff while paid preparation is retained. The
            // normal drive AI has no explicit directive and must not begin anew.
            let token = token("helm");
            send(&mut live, &token, ClientMessage::ReleaseStation);
            send(&mut resumed, &token, ClientMessage::ReleaseStation);
        }
        live.update();
        resumed.update();
        if !arrived && phase(&live, entity)["phase"] == "arrived" {
            let public = match &live.world().get::<ShipSystemBlackboards>(entity).unwrap().0
                [&id("local-jump")]
            {
                SystemBlackboard::Transit(view) => view,
                _ => panic!("actual typed Transit producer"),
            };
            let physical = live.world().get::<ShipPhysics>(entity).unwrap();
            assert_eq!(
                Some([physical.x, physical.y, physical.z]),
                public.actual_arrival
            );
            assert_eq!(
                [
                    physical.forward_speed,
                    physical.lateral_speed,
                    physical.vertical_speed
                ],
                [0.0; 3]
            );
            let private = serde_json::to_value(public).unwrap();
            for field in ["gravity", "scatter_offset", "profile", "origin"] {
                assert!(private.get(field).is_none());
            }
            arrived = true;
        }
    }
    same(&live, &resumed, 360);
    assert!(arrived);
    assert_eq!(fuel(&live, entity), 22);
    // Replenishment is an ordinary paid physical Logistics batch, not another
    // drive counter or a refill granted by arrival/restore.
    let local_before = amount(&live, entity, "drive-transit-fuel");
    let hold_before = amount(&live, entity, "hold-transit-fuel");
    for (app, actor) in [(&mut live, entity), (&mut resumed, restored)] {
        control(
            app,
            actor,
            "logistics",
            Control::DispatchDelivery {
                source: "hold-transit-fuel".into(),
                destination: "local-transit-fuel".into(),
                amount: 2,
            },
        );
    }
    let mut received = false;
    for step in 0..1800 {
        same(&live, &resumed, step + 361);
        assert_eq!(fuel(&live, entity), 22);
        if amount(&live, entity, "drive-transit-fuel") == local_before + 2 {
            assert_eq!(amount(&live, entity, "hold-transit-fuel"), hold_before - 2);
            let outcome = live
                .world()
                .get::<ShipLogistics>(entity)
                .unwrap()
                .0
                .last_outcome
                .as_ref()
                .unwrap();
            assert_eq!(outcome.destination, "local-transit-fuel");
            assert_eq!(outcome.amount, 2);
            assert_eq!(outcome.outcome, phoenix_model::job::JobOutcome::Completed);
            assert!(!outcome.returned);
            received = true;
            break;
        }
        live.update();
        resumed.update();
    }
    assert!(
        received,
        "actual paid batch must reach the drive's own Capacity endpoint"
    );
}

#[test]
fn actual_hardened_drive_uses_original_gravity_and_keeps_paid_json360_continuation() {
    let boot = || {
        boot_with(
            "tests/fixtures/gravity-hardened-jump-world.toml",
            "assets/entities/gravity_hardened_jump_cruiser.toml",
        )
    };
    let (mut live, actor) = boot();
    control(
        &mut live,
        actor,
        "navigation",
        Control::SetNavigationWaypoint {
            x: 100.0,
            z: 0.0,
            source_uuid: None,
        },
    );
    control(
        &mut live,
        actor,
        "local-jump",
        Control::BeginTransitToWaypoint { request: None },
    );
    assert_eq!(phase(&live, actor)["phase"], "preparing");
    assert!(
        live.world().get::<ShipTransitDrives>(actor).unwrap().0[&id("local-jump")].worked_secs()
            > 0.0
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..2 {
        let mut forged = saved.clone();
        let row = forged
            .entities
            .iter_mut()
            .find(|row| row.transit.is_some())
            .unwrap();
        let state = row
            .transit
            .as_mut()
            .unwrap()
            .0
            .get_mut(&id("local-jump"))
            .unwrap();
        let phoenix_sim_gameplay::ship::transit::TransitPhase::Preparing { order, .. } =
            &mut state.phase
        else {
            panic!("actual partial drive")
        };
        if fault == 0 {
            order.profile.config.gravity_response = 0.5;
        } else {
            order.profile.commit_energy *= 0.5;
        }
        assert!(!restore(resumed.world_mut(), &forged).is_complete());
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    assert!(restore(resumed.world_mut(), &saved).is_complete());
    let mut arrived = false;
    for step in 0..360 {
        same(&live, &resumed, step);
        for (app, entity) in [(&live, actor), (&resumed, restored)] {
            let value = phase(app, entity);
            let committed = matches!(value["phase"].as_str(), Some("committed" | "arrived"));
            assert_eq!(fuel(app, entity), if committed { 22 } else { 24 });
            if value["phase"] == "arrived" {
                let position = app.world().get::<ShipPhysics>(entity).unwrap();
                let radius =
                    ((position.x - 300.0).powi(2) + position.y.powi(2) + position.z.powi(2)).sqrt();
                assert!((198.0..=202.0).contains(&radius), "the selected course is inside original512 but outside effective128 exclusion: {radius}");
                assert!(
                    radius > 75.0,
                    "ordinary real planet collider remains outside arrival"
                );
                arrived = true;
            }
        }
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    assert!(arrived);
    assert_eq!(fuel(&live, actor), 22);
}
