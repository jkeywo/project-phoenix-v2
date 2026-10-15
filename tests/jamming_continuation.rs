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
/// Read-only proof of the real Input work interval, before the ordinary
/// Physics energy tick clears per-interval emission scratch. Never game state.
#[derive(Resource, Default)]
struct PaidInputWitness {
    actor: Option<Entity>,
    tick: Option<u64>,
    amount: f32,
}
fn observe_paid_input(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    ships: Query<(Entity, &ShipPowerSystem), With<LocalShip>>,
    mut witness: ResMut<PaidInputWitness>,
) {
    *witness = PaidInputWitness::default();
    if let Ok((actor, power)) = ships.single() {
        witness.actor = Some(actor);
        witness.tick = Some(tick.0);
        witness.amount = power.0.emitted_work(&owner());
    }
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
    app.init_resource::<PaidInputWitness>();
    app.add_systems(
        FixedUpdate,
        observe_paid_input
            .in_set(project_phoenix::sim_sets::SimSet::Input)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickJamming)
            .before_ignore_deferred(project_phoenix::sim_sets::FixedStep::RefreshJammedCommsRange),
    );
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
    let witness = app.world().resource::<PaidInputWitness>();
    witness.actor == Some(actor)
        && witness.tick.and_then(|tick| tick.checked_add(1))
            == Some(
                app.world()
                    .resource::<project_phoenix::sim_tick::SimTick>()
                    .0,
            )
        && witness.amount > 0.0
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
fn acquisition_diagnostic(app: &App, actor: Entity) -> serde_json::Value {
    let world = app.world();
    let config = &world.get::<ShipConfigComponent>(actor).unwrap().0;
    let boards = &world.get::<ShipSystemBlackboards>(actor).unwrap().0;
    let view = config.observations_for_system(&owner(), boards);
    let permitted = boards
        .keys()
        .filter_map(|key| {
            let board = view.get(key)?;
            if let Some(records) = project_phoenix::science::contact_routing::observations(board) {
                Some((key.0.clone(), serde_json::json!({"records": records})))
            } else if let SystemBlackboard::Viewscreen(board) = board {
                Some((
                    key.0.clone(),
                    serde_json::json!({"requests": board.scored_objectives}),
                ))
            } else {
                None
            }
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let state = world.get::<ShipJamming>(actor).unwrap();
    let power = &world.get::<ShipPowerSystem>(actor).unwrap().0;
    let sources = &world.get::<ShipSystemControlSources>(actor).unwrap().0;
    serde_json::json!({
        "tick": world.resource::<project_phoenix::sim_tick::SimTick>().0,
        "channels": state.0.get(&owner()), "notice": own_notice(app,actor),
        "source": format!("{:?}",sources.source_for(&owner())),
        "policy": format!("{:?}",sources.policy_for(&owner())),
        "input_paid": world.get_resource::<PaidInputWitness>().map(|witness|(witness.tick,witness.amount)),
        "work": power.emitted_work(&owner()), "last_fraction": power.last_work_fraction(&owner()),
        "reserve": power.network().consumers.get(&owner()).map(|consumer|consumer.energy),
        "radar_work": power.emitted_work(&SystemId("tactical-radar".into())),
        "radar_reserve": power.network().consumers.get(&SystemId("tactical-radar".into())).map(|consumer|consumer.energy),
        "tree": world.get::<project_phoenix::ai::state_tree_runtime::ShipAiTreeRuntime>(actor).and_then(|runtime|runtime.0.get(&owner())),
        "mount": world.get::<project_phoenix::weapons::mounts::ShipMounts>(actor).and_then(|mounts|mounts.0.get(&owner())),
        "own_pose": format!("{:?}",world.get::<project_phoenix::ship::state::ShipPhysics>(actor)),
        "permitted": permitted,
    })
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
    let mut diagnostics = Vec::new();
    for step in 0..1800 {
        live.update();
        if [0, 60, 180, 600, 1200, 1799].contains(&step) {
            diagnostics.push(acquisition_diagnostic(&live, actor));
        }
        if enabled(&live, actor) && paid(&live, actor) && own_notice(&live, actor) == Some(true) {
            acquired = true;
            break;
        }
    }
    assert!(acquired,"ordinary Comms tree must read its explicit request, acquire permitted fullXYZ and pay the real mounted emitter: {}",serde_json::to_string_pretty(&diagnostics).unwrap());
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
