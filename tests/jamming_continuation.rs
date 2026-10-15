//! Ordinary scoped Jam request, paid actual emitter and complete host continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    comms::jamming::ShipJamming,
    core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control, SystemId},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const OWNER: &str = "comms-jamming";
const TOKEN: &str = "interference-operator";
fn owner() -> SystemId {
    SystemId(OWNER.into())
}
fn boot() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/jamming-world.toml".into(),
        ship_path: "assets/entities/jamming_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156217),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    let actor = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    (app, actor)
}
fn send(app: &mut App, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: TOKEN.into(),
        msg,
    });
}
fn enabled(app: &App, actor: Entity) -> bool {
    app.world().get::<ShipJamming>(actor).unwrap().enabled(
        &app.world().get::<ShipConfigComponent>(actor).unwrap().0,
        &owner(),
    )
}
fn paid(app: &App, actor: Entity) -> bool {
    app.world()
        .get::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .emitted_work(&owner())
        > 0.0
}
fn own_notice(app: &App, actor: Entity) -> Option<bool> {
    match app
        .world()
        .get::<ShipSystemBlackboards>(actor)
        .unwrap()
        .0
        .get(&SystemId("comms".into()))
    {
        Some(SystemBlackboard::Comms(board)) => board.receiver_interference,
        _ => None,
    }
}
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) -> Option<String> {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for key in a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => {
                        if let Some(diff) = first_difference(a, b, &format!("{path}.{key}")) {
                            return Some(diff);
                        }
                    }
                    _ => return Some(format!("{path}.{key}: presence differs")),
                }
            }
            None
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: length{} vs{}", a.len(), b.len()));
            }
            a.iter()
                .zip(b)
                .enumerate()
                .find_map(|(i, (a, b))| first_difference(a, b, &format!("{path}[{i}]")))
        }
        _ => (a != b).then(|| format!("{path}: {a} vs {b}")),
    }
}

fn same(a: &App, b: &App, step: usize) {
    let left = serde_json::to_value(capture(a.world())).unwrap();
    let right = serde_json::to_value(capture(b.world())).unwrap();
    assert!(
        left == right,
        "capture at{step}: {:?}",
        first_difference(&left, &right, "$")
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest at{step}"
    );
}
#[test]
fn ordinary_scoped_request_paid_self_exposure_json360_and_human_stop_use_one_actual_owner() {
    let (mut live, actor) = boot();
    let mut acquired = false;
    for _ in 0..1800 {
        live.update();
        if enabled(&live, actor) && paid(&live, actor) && own_notice(&live, actor) == Some(true) {
            acquired = true;
            break;
        }
    }
    assert!(acquired,"ordinary Comms tree must read its explicit request, acquire permitted fullXYZ and pay the real mounted emitter");
    let contact = live
        .world()
        .get::<ShipJamming>(actor)
        .unwrap()
        .target(
            &live.world().get::<ShipConfigComponent>(actor).unwrap().0,
            &owner(),
        )
        .unwrap()
        .to_owned();
    assert!(!contact.is_empty());
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    let mut bad = saved.clone();
    bad.entities
        .iter_mut()
        .find(|row| row.jamming.is_some())
        .unwrap()
        .jamming = None;
    assert!(!restore(resumed.world_mut(), &bad).is_complete());
    assert_eq!(capture(resumed.world()), before);
    assert_eq!(world_digest(resumed.world()), digest);
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    let mut continued_paid = false;
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        continued_paid |= paid(&live, actor);
    }
    same(&live, &resumed, 360);
    assert!(
        continued_paid,
        "restored intent must buy new actual emission work"
    );
    assert_eq!(own_notice(&live, actor), Some(true));
    let graph = &live.world().get::<ShipConfigComponent>(actor).unwrap().0;
    let station = graph
        .system_home_station(graph.system(&owner()).unwrap())
        .unwrap()
        .clone();
    let rating = graph
        .stations
        .iter()
        .find(|row| row.id == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    for msg in [
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: TOKEN.into(),
        },
        ClientMessage::SelectStation {
            station: station.0.clone(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ] {
        for app in [&mut live, &mut resumed] {
            send(app, msg.clone());
            for _ in 0..4 {
                app.update();
            }
        }
    }
    let payload = Control::SetJammingEnabled { enabled: false };
    for (app, entity) in [(&mut live, actor), (&mut resumed, restored)] {
        let sessions = app.world().resource::<Sessions>();
        assert_eq!(sessions.0.holder_for_station(&station), Some(TOKEN));
        assert!(
            project_phoenix::command_admission::policy::is_command_authorized(
                TOKEN,
                &owner(),
                &payload,
                app.world().get::<ShipSystemControlSources>(entity).unwrap(),
                sessions,
                &app.world().get::<ShipConfigComponent>(entity).unwrap().0,
                app.world().get::<HumanSeekingHosts>(entity)
            )
        );
        send(
            app,
            ClientMessage::ControlSystem {
                target: owner(),
                payload: payload.clone(),
            },
        );
        for _ in 0..4 {
            app.update();
        }
        assert!(!enabled(app, entity));
        assert!(!paid(app, entity));
        assert_eq!(own_notice(app, entity), Some(false));
    }
    same(&live, &resumed, 380);
}
