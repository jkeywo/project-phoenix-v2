//! Actual Bay/identity/backend custody, paid sensing and complete continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use bevy_rapier3d::prelude::{RapierColliderHandle, RapierRigidBodyHandle};
use project_phoenix::{
    core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control, SystemId},
    entities::spawner::{EntityId, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    science::broad::BroadTracks,
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{power::ShipPowerSystem, state::ShipPhysics},
    ship_plugin::ShipConfigComponent,
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn entity(app: &mut App, name: &str) -> Entity {
    app.world_mut()
        .query_filtered::<(Entity, &EntityId), Allow<Disabled>>()
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == name).then_some(entity))
        .unwrap()
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn boot() -> (App, Entity, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/physical-probe-world.toml".into(),
        ship_path: "assets/entities/probe_carrier.toml".into(),
        deterministic: true,
        seed: Some(191919),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let parent = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let child = entity(&mut app, "physical-probe");
    let stations = app
        .world()
        .get::<ShipConfigComponent>(parent)
        .unwrap()
        .0
        .stations
        .clone();
    for station in stations {
        let token = format!("craft-{}", station.id.0);
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
    control(
        &mut app,
        "helm",
        "helm-thrust",
        Control::SetThrust { value: 0.0 },
    );
    control(
        &mut app,
        "helm",
        "helm-steering",
        Control::SetSteering { value: 0.0 },
    );
    (app, parent, child)
}
fn control(app: &mut App, station: &str, owner: &str, payload: Control) {
    let token = format!("craft-{station}");
    assert!(app
        .world()
        .resource::<Sessions>()
        .0
        .players()
        .iter()
        .any(|p| p.token == token));
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
fn bay(app: &App, parent: Entity) -> phoenix_model::craft::BayView {
    let SystemBlackboard::Bay(view) =
        &app.world().get::<ShipSystemBlackboards>(parent).unwrap().0[&id("probe-bay")]
    else {
        panic!("actual typed Bay publication")
    };
    view.clone()
}
fn stored(app: &App, child: Entity) -> bool {
    app.world().get::<Disabled>(child).is_some()
}
fn wait(app: &mut App, mut predicate: impl FnMut(&App) -> bool) {
    for _ in 0..600 {
        if predicate(app) {
            return;
        }
        app.update();
    }
    panic!("actual capability must reach expected bounded state");
}
fn reserve(app: &App, actor: Entity, owner: &str) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .network()
        .consumers[&id(owner)]
        .energy
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
fn joined_paid_probe_launch_recovery_keeps_actual_body_history_and_json360() {
    let (mut live, parent, child) = boot();
    assert!(stored(&live, child));
    assert!(live.world().get::<RapierColliderHandle>(child).is_none());
    assert!(live.world().get::<RapierRigidBodyHandle>(child).is_none());
    let uuid = live.world().get::<EntityUuid>(child).unwrap().0.clone();
    let initial = reserve(&live, child, "sensor-radar");
    let own = id("sensor-radar");
    let before_tracks = live.world().get::<BroadTracks>(child).unwrap().0[&own].clone();
    steps(&mut live, 30);
    assert!(reserve(&live, child, "sensor-radar") < initial);
    assert_eq!(
        live.world().get::<BroadTracks>(child).unwrap().0[&own],
        before_tracks,
        "storage buys no new sensor samples"
    );
    control(
        &mut live,
        "engineering",
        "power-reactor",
        Control::SetEnergyMode {
            mode: phoenix_model::energy::EnergyAllocationMode::Manual,
            cooling_usage: phoenix_model::energy::CoolingUsage::Low,
        },
    );
    control(
        &mut live,
        "engineering",
        "power-reactor",
        Control::SetEnergySystemFlow {
            system: id("probe-bay"),
            flow_per_sec: Some(0.0),
        },
    );
    wait(&mut live, |app| {
        app.world().get::<ShipPhysics>(parent).is_some_and(|pose| {
            [pose.forward_speed, pose.lateral_speed, pose.vertical_speed]
                .iter()
                .all(|v| v.abs() <= 0.03)
        })
    });
    let bay_before = reserve(&live, parent, "probe-bay");
    control(
        &mut live,
        "tactical",
        "probe-bay",
        Control::LaunchCraft {
            craft: uuid.clone(),
        },
    );
    assert!(bay(&live, parent)
        .job
        .as_ref()
        .is_some_and(|job| job.fraction > 0.0 && job.fraction < 1.0));
    let saved: PhoenixSnapshot =
        serde_json::from_str(&serde_json::to_string(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, _, _) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    assert!(!stored(&live, child));
    assert_eq!(live.world().get::<EntityUuid>(child).unwrap().0, uuid);
    assert!(live.world().get::<RapierColliderHandle>(child).is_some());
    assert!(live.world().get::<RapierRigidBodyHandle>(child).is_some());
    assert!(reserve(&live, parent, "probe-bay") < bay_before);
    assert!(
        live.world().get::<BroadTracks>(child).unwrap().0[&own]
            .tracks
            .values()
            .any(|track| track.observed_tick > saved.tick),
        "real independent probe must acquire dated evidence after physical release"
    );
    let contact = bay(&live, parent)
        .contacts
        .into_iter()
        .find(|contact| {
            project_phoenix::science::contact_routing::resolve(
                &live.world().get::<ShipConfigComponent>(parent).unwrap().0,
                live.world().get::<ShipSystemBlackboards>(parent).unwrap(),
                live.world().get::<BroadTracks>(parent),
                &id("probe-bay"),
                &contact.contact,
            )
            .as_deref()
                == Some(uuid.as_str())
        })
        .expect("actual broad contact must permit recovery")
        .contact;
    control(
        &mut live,
        "tactical",
        "probe-bay",
        Control::RecoverCraft { contact },
    );
    wait(&mut live, |app| stored(app, child));
    steps(&mut live, 2);
    assert!(live.world().get::<RapierColliderHandle>(child).is_none());
    assert!(live.world().get::<RapierRigidBodyHandle>(child).is_none());
    let held = live.world().get::<BroadTracks>(child).unwrap().0[&own]
        .tracks
        .clone();
    assert!(!held.is_empty());
    let finite = reserve(&live, child, "sensor-radar");
    steps(&mut live, 30);
    assert!(reserve(&live, child, "sensor-radar") < finite);
    assert_eq!(
        live.world().get::<BroadTracks>(child).unwrap().0[&own].tracks,
        held
    );
    let saved: PhoenixSnapshot =
        serde_json::from_str(&serde_json::to_string(&capture(live.world())).unwrap()).unwrap();
    for fault in 0..4 {
        let mut malformed = saved.clone();
        if fault < 3 {
            let row = malformed
                .entities
                .iter_mut()
                .find(|row| row.uuid == uuid)
                .unwrap();
            match fault {
                0 => row.probe_custody = None,
                1 => row.physics.as_mut().unwrap()[4] = 1.0,
                _ => {
                    row.probe_custody.as_mut().unwrap().0 =
                        phoenix_model::craft::CraftLocation::Carried {
                            parent_uuid: "missing-parent".into(),
                            bay: id("probe-bay"),
                        }
                }
            }
        } else {
            malformed
                .entities
                .iter_mut()
                .find(|row| row.bay_jobs.is_some())
                .unwrap()
                .bay_jobs = None;
        }
        let before = capture(live.world());
        let digest = world_digest(live.world());
        let refused = restore(live.world_mut(), &malformed);
        assert!(!refused.is_complete());
        assert_eq!(capture(live.world()), before);
        assert_eq!(world_digest(live.world()), digest);
    }
    let (mut stored_resume, _, _) = boot();
    assert!(ready_to_restore(stored_resume.world(), &saved));
    let report = restore(stored_resume.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..360 {
        same(&live, &stored_resume, step);
        live.update();
        stored_resume.update();
    }
    same(&live, &stored_resume, 360);
    assert_eq!(live.world().get::<EntityUuid>(child).unwrap().0, uuid);
}
