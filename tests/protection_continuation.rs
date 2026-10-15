//! Actual independently paid jammer, finite own priorities and complete continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    comms::jamming::ShipJamming,
    core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control, SystemId},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    science::protection::{ProtectionGrants, ShipSignalProtection},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const OWNER: &str = "signal-protection";
const TOKEN: &str = "protection-operator";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
#[derive(Resource, Default)]
struct PaidWitness {
    tick: Option<u64>,
    protection: f64,
    jammer: f64,
    suppression: f64,
}
fn observe_paid_input(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    owners: Query<(
        &ShipPowerSystem,
        Option<&ProtectionGrants>,
        Has<LocalShip>,
        Option<&ShipJamming>,
    )>,
    mut witness: ResMut<PaidWitness>,
) {
    *witness = PaidWitness {
        tick: Some(tick.0),
        ..Default::default()
    };
    for (power, grants, local, jammer) in &owners {
        if local {
            witness.protection = power.0.emitted_work(&id(OWNER));
            witness.suppression =
                grants.map_or(0.0, |grants| grants.suppression_for(&id("comms"), tick.0));
        }
        if jammer.is_some() {
            witness.jammer = power.0.emitted_work(&id("comms-jamming"));
        }
    }
}
fn boot() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/protection-world.toml".into(),
        ship_path: "assets/entities/protection_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156217),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<PaidWitness>();
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
fn claim(app: &mut App, station: &str, rating: &str) {
    for msg in [
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: TOKEN.into(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating.into(),
        },
    ] {
        send(app, msg);
        for _ in 0..4 {
            app.update();
        }
    }
}
fn control(app: &mut App, payload: Control) {
    send(
        app,
        ClientMessage::ControlSystem {
            target: id(OWNER),
            payload,
        },
    );
    for _ in 0..4 {
        app.update();
    }
}
fn notice(app: &App, actor: Entity) -> Option<bool> {
    match app
        .world()
        .get::<ShipSystemBlackboards>(actor)
        .unwrap()
        .0
        .get(&id("comms"))
    {
        Some(SystemBlackboard::Comms(board)) => board.receiver_interference,
        _ => None,
    }
}
fn paid(app: &App) -> bool {
    let witness = app.world().resource::<PaidWitness>();
    witness.tick.and_then(|tick| tick.checked_add(1))
        == Some(
            app.world()
                .resource::<project_phoenix::sim_tick::SimTick>()
                .0,
        )
        && witness.protection > 0.0
        && witness.jammer > 0.0
}
fn state(app: &App, actor: Entity) -> &ShipSignalProtection {
    app.world().get::<ShipSignalProtection>(actor).unwrap()
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
fn ordinary_protection_and_independent_jammer_restore_exactly_then_human_priorities_change_only_own_coverage(
) {
    let (mut live, actor) = boot();
    let mut acquired = false;
    for _ in 0..1800 {
        live.update();
        if paid(&live)
            && notice(&live, actor) == Some(false)
            && live.world().resource::<PaidWitness>().suppression > 0.0
        {
            acquired = true;
            break;
        }
    }
    assert!(acquired,"real NPC doctrine must acquire its permitted subject and pay while ordinary protection funds its own selected Comms receiver");
    let graph = &live.world().get::<ShipConfigComponent>(actor).unwrap().0;
    let station = graph
        .system_home_station(graph.system(&id(OWNER)).unwrap())
        .unwrap()
        .clone();
    let rating = graph
        .stations
        .iter()
        .find(|row| row.id == station)
        .unwrap()
        .ratings
        .iter()
        .find(|rating| rating.name == "Std")
        .unwrap()
        .name
        .clone();
    claim(&mut live, &station.0, &rating);
    let payload = Control::SetSignalProtectionPriorities {
        systems: vec![id("comms"), id("sensors")],
    };
    let sessions = live.world().resource::<Sessions>();
    assert_eq!(sessions.0.holder_for_station(&station), Some(TOKEN));
    assert!(
        project_phoenix::command_admission::policy::is_command_authorized(
            TOKEN,
            &id(OWNER),
            &payload,
            live.world().get::<ShipSystemControlSources>(actor).unwrap(),
            sessions,
            &live.world().get::<ShipConfigComponent>(actor).unwrap().0,
            live.world().get::<HumanSeekingHosts>(actor)
        )
    );
    control(&mut live, payload);
    assert_eq!(
        state(&live, actor).priorities(
            &live.world().get::<ShipConfigComponent>(actor).unwrap().0,
            &id(OWNER)
        ),
        [id("comms"), id("sensors")]
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    claim(&mut resumed, &station.0, &rating);
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.signal_protection.is_some())
            .unwrap();
        if fault == 0 {
            row.signal_protection = None;
        } else {
            let channels = row
                .signal_protection
                .as_mut()
                .unwrap()
                .0
                .get_mut(&id(OWNER))
                .unwrap();
            channels.0.insert(
                "priorities".into(),
                project_phoenix::ship::composition::CommandChannelValue::SystemPriority(
                    if fault == 1 {
                        vec![id("comms"), id("comms")]
                    } else {
                        vec![id("helm-thrust")]
                    },
                ),
            );
        }
        assert!(!restore(resumed.world_mut(), &bad).is_complete());
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    let mut continued_paid = false;
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        continued_paid |= paid(&live) && live.world().resource::<PaidWitness>().suppression > 0.0;
    }
    same(&live, &resumed, 360);
    assert!(
        continued_paid,
        "restored canonical intent must buy fresh protection and jammer work"
    );
    for (app, entity) in [(&mut live, actor), (&mut resumed, restored)] {
        control(
            app,
            Control::SetSignalProtectionPriorities {
                systems: vec![id("tactical-radar"), id("comms"), id("sensors")],
            },
        );
        assert!(paid(app));
        assert_eq!(app.world().resource::<PaidWitness>().suppression, 0.0);
        assert_eq!(notice(app,entity),Some(true),"capacity3 funds whole radar2 plus sensors1, leaving Comms unprotected while both peers still pay");
        control(
            app,
            Control::SetSignalProtectionPriorities {
                systems: vec![id("comms"), id("sensors")],
            },
        );
        assert!(paid(app));
        assert!(app.world().resource::<PaidWitness>().suppression > 0.0);
        assert_eq!(notice(app, entity), Some(false));
        control(app, Control::SetSignalProtectionEnabled { enabled: false });
        assert_eq!(app.world().resource::<PaidWitness>().protection, 0.0);
        assert!(app.world().resource::<PaidWitness>().jammer > 0.0);
        assert_eq!(notice(app, entity), Some(true));
    }
    same(&live, &resumed, 372);
}
