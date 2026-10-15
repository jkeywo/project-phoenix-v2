//! Joined-crew movement through the real plugin graph and sole paid integrator.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::helm_control::{HelmControlMode, HelmMovementView, ThrustIntent};
use project_phoenix::{
    core::messages::{
        AdmittedCommands, ClientMessage, StationId, SystemBlackboard,
        SystemControlPayload as Control, SystemId,
    },
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const WORLD: &str = "tests/fixtures/helm-movement-world.toml";
const HULL: &str = "assets/entities/alliance_cruiser.toml";
const PILOT: &str = "movement-pilot";
const ENGINEER: &str = "movement-engineer";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            keys.into_iter().find_map(|key| {
                first_difference(
                    a.get(key).unwrap_or(&serde_json::Value::Null),
                    b.get(key).unwrap_or(&serde_json::Value::Null),
                    &format!("{path}.{key}"),
                )
            })
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (a, b))| first_difference(a, b, &format!("{path}[{index}]"))),
        _ => (a != b).then(|| format!("{path}: {a} != {b}")),
    }
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
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
fn claim(app: &mut App, station: &str, token: &str) {
    let entity = local(app);
    let rating_name = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|row| row.id.0 == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    for msg in [
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating { rating_name },
    ] {
        send(app, token, msg);
        steps(app, 4);
    }
}
fn authorised(
    app: &App,
    entity: Entity,
    token: &str,
    station: &str,
    target: &SystemId,
    payload: &Control,
) -> bool {
    let sessions = app.world().resource::<Sessions>();
    sessions.0.players().iter().any(|player| {
        player.token == token
            && player.connected
            && player.ready
            && player.station.as_ref() == Some(&StationId(station.into()))
    }) && sessions.0.holder_for_station(&StationId(station.into())) == Some(token)
        && project_phoenix::command_admission::policy::is_command_authorized(
            token,
            target,
            payload,
            app.world().get::<ShipSystemControlSources>(entity).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
            app.world().get::<HumanSeekingHosts>(entity),
        )
}
fn control(
    app: &mut App,
    entity: Entity,
    token: &str,
    station: &str,
    target: &str,
    payload: Control,
) {
    for _ in 0..120 {
        if authorised(app, entity, token, station, &id(target), &payload) {
            break;
        }
        app.update();
    }
    assert!(
        authorised(app, entity, token, station, &id(target), &payload),
        "ordinary {station} authority for {target}"
    );
    send(
        app,
        token,
        ClientMessage::ControlSystem {
            target: id(target),
            payload,
        },
    );
    steps(app, 4);
}
fn prepared() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        deterministic: true,
        seed: Some(181801),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    claim(&mut app, "helm", PILOT);
    claim(&mut app, "engineering", ENGINEER);
    let entity = local(&mut app);
    control(
        &mut app,
        entity,
        ENGINEER,
        "engineering",
        "power-reactor",
        Control::SetEnergyMode {
            mode: phoenix_model::energy::EnergyAllocationMode::Manual,
            cooling_usage: phoenix_model::energy::CoolingUsage::Low,
        },
    );
    // Isolate the actual local reserve debit from stored reactor export. The
    // prior AI's priority mode deliberately ignores manual fine-flow requests.
    for owner in ["helm-thrust", "helm-steering", "helm-lateral-thrust"] {
        control(
            &mut app,
            entity,
            ENGINEER,
            "engineering",
            "power-reactor",
            Control::SetEnergySystemFlow {
                system: id(owner),
                flow_per_sec: Some(0.0),
            },
        );
        assert_eq!(
            app.world()
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .network()
                .fine_flows
                .get(&id(owner)),
            Some(&0.0)
        );
    }
    control(
        &mut app,
        entity,
        ENGINEER,
        "engineering",
        "power-reactor",
        Control::SetEnergyGeneration { enabled: false },
    );
    assert!(
        !app.world()
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network()
            .generation_enabled
    );
    (app, entity)
}
fn pose(app: &App, entity: Entity) -> ShipPhysics {
    *app.world().get::<ShipPhysics>(entity).unwrap()
}
fn movement(app: &App, entity: Entity) -> HelmMovementView {
    app.world()
        .get::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .values()
        .find_map(|board| match board {
            SystemBlackboard::Helm(helm) => helm.movement.clone(),
            _ => None,
        })
        .expect("actual own movement publication")
}
fn velocity(pose: ShipPhysics) -> [f64; 2] {
    let yaw = f64::from(pose.yaw);
    [
        yaw.sin() * f64::from(pose.forward_speed) + yaw.cos() * f64::from(pose.lateral_speed),
        -yaw.cos() * f64::from(pose.forward_speed) + yaw.sin() * f64::from(pose.lateral_speed),
    ]
}
fn close(a: [f64; 2], b: [f64; 2]) {
    for axis in 0..2 {
        assert!(
            (a[axis] - b[axis]).abs() < 0.001,
            "world velocity {a:?} vs {b:?}"
        );
    }
}

#[test]
fn joined_direct_force_coast_and_turn_resume_before_ordinary_ai_assisted_handoff() {
    let (mut live, entity) = prepared();
    for owner in ["helm-thrust", "helm-steering", "helm-lateral-thrust"] {
        control(
            &mut live,
            entity,
            PILOT,
            "helm",
            owner,
            Control::SetHelmMode {
                mode: HelmControlMode::Direct,
            },
        );
    }
    control(
        &mut live,
        entity,
        PILOT,
        "helm",
        "helm-thrust",
        Control::SetThrust { value: 0.0 },
    );
    control(
        &mut live,
        entity,
        PILOT,
        "helm",
        "helm-steering",
        Control::SetSteering { value: 0.0 },
    );
    control(
        &mut live,
        entity,
        PILOT,
        "helm",
        "helm-lateral-thrust",
        Control::LateralThrustInput { lateral: 0.0 },
    );
    let before = pose(&live, entity);
    let reserve = live
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&id("helm-thrust")]
        .energy;
    control(
        &mut live,
        entity,
        PILOT,
        "helm",
        "helm-thrust",
        Control::SetThrust { value: 0.5 },
    );
    steps(&mut live, 16);
    assert!(pose(&live, entity).forward_speed > before.forward_speed);
    let power = &live.world().get::<ShipPowerSystem>(entity).unwrap().0;
    assert!(power
        .last_work_fraction(&id("helm-thrust"))
        .is_some_and(|fraction| fraction > 0.0));
    assert!(power.emitted_work(&id("helm-thrust")) > 0.0);
    assert!(power.network().consumers[&id("helm-thrust")].energy < reserve);
    control(
        &mut live,
        entity,
        PILOT,
        "helm",
        "helm-thrust",
        Control::SetThrust { value: 0.0 },
    );
    let coasting = pose(&live, entity);
    for _ in 0..20 {
        steps(&mut live, 1);
        // The grant fraction retains the last requested actuator work. Neutral
        // force does not request work, so prove actual expenditure each tick.
        // Battery passive drain can still lower reserve without actuator work.
        assert_eq!(
            live.world()
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .emitted_work(&id("helm-thrust")),
            0.0
        );
    }
    close(velocity(coasting), velocity(pose(&live, entity)));
    assert_ne!(
        [coasting.x, coasting.z],
        [pose(&live, entity).x, pose(&live, entity).z]
    );
    control(
        &mut live,
        entity,
        PILOT,
        "helm",
        "helm-steering",
        Control::SetSteering { value: 0.5 },
    );
    steps(&mut live, 20);
    assert_ne!(pose(&live, entity).yaw, coasting.yaw);
    close(velocity(coasting), velocity(pose(&live, entity)));
    assert!(live
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .last_work_fraction(&id("helm-steering"))
        .is_some_and(|fraction| fraction > 0.0));
    control(
        &mut live,
        entity,
        PILOT,
        "helm",
        "helm-thrust",
        Control::SetThrust { value: 0.25 },
    );
    assert!(
        matches!(movement(&live, entity).thrust.intent, ThrustIntent::Direct { force } if force == 0.25)
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, _) = prepared();
    assert!(ready_to_restore(resumed.world(), &saved));
    let rejected_before = capture(resumed.world());
    let rejected_digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut forged = saved.clone();
        let tick = forged.tick;
        let row = forged
            .entities
            .iter_mut()
            .find(|row| row.helm_movement.is_some())
            .unwrap();
        match fault {
            0 => row.helm_movement = None,
            1 => {
                row.helm_movement
                    .as_mut()
                    .unwrap()
                    .axes
                    .remove(&id("helm-steering"));
            }
            _ => {
                row.helm_movement
                    .as_mut()
                    .unwrap()
                    .match_speed(
                        &id("helm-thrust"),
                        "held-public-handle",
                        7.0,
                        tick + 1,
                        tick + 1,
                    )
                    .unwrap();
                // Match's raw projection is neutral; isolate its future date refusal.
                if let Some(raw) = row.control.as_mut() {
                    raw.thrust = 0.0;
                }
            }
        }
        assert!(
            !restore(resumed.world_mut(), &forged).is_complete(),
            "forged frontier {fault}"
        );
        assert_eq!(capture(resumed.world()), rejected_before);
        assert_eq!(world_digest(resumed.world()), rejected_digest);
    }
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    let mut ai_assisted = false;
    let mut ai_demand = false;
    for step in 0..360 {
        let original = serde_json::to_value(capture(live.world())).unwrap();
        let restored = serde_json::to_value(capture(resumed.world())).unwrap();
        assert!(
            original == restored,
            "complete JSON continuation {step}: {:?}",
            first_difference(&original, &restored, "$")
        );
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "digest continuation {step}"
        );
        if step == 180 {
            send(&mut live, PILOT, ClientMessage::ReleaseStation);
            send(&mut resumed, PILOT, ClientMessage::ReleaseStation);
        }
        live.update();
        resumed.update();
        if step >= 180 {
            let admitted = live.world().get::<AdmittedCommands>(entity).unwrap();
            ai_assisted |= admitted.for_target("helm-thrust").any(|command| {
                matches!(
                    &command.payload,
                    Control::SetHelmMode {
                        mode: HelmControlMode::Assisted
                    }
                )
            });
            ai_demand |= admitted
                .for_target("helm-thrust")
                .any(|command| matches!(&command.payload, Control::SetThrust { .. }));
        }
    }
    assert!(
        ai_assisted && ai_demand,
        "ordinary compiled AI must actually emit Assisted and a scalar after release"
    );
    assert!(matches!(
        movement(&live, entity).thrust.intent,
        ThrustIntent::Assisted { .. }
    ));
}
