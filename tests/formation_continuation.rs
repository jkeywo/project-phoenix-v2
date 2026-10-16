//! Actual Human paid coordination -> ordinary backfill reading/Navigation/Helm.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    comms_channel::CommsAudience,
    comms_request::{CommsRequestDecision, GenericCommsRequest},
};
use project_phoenix::ship::helm_control::{
    AxisIntent, HelmControlState, SpeedDemand, ThrustIntent,
};
use project_phoenix::{
    core::messages::{
        ClientMessage, GamePhase, SystemBlackboard, SystemControlPayload as Control, SystemId,
    },
    entities::spawner::{EntityId, EntityName, EntityUuid, WeaponsConsoleSection},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{helm_planner::HelmMotionPlan, power::ShipPowerSystem, state::ShipPhysics},
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
    format!("formation-{station}")
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
        "tests/fixtures/formation-world.toml",
        "assets/entities/formation_cruiser.toml",
    )
}
fn boot_with(world_path: &str, ship_path: &str) -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: world_path.into(),
        ship_path: ship_path.into(),
        deterministic: true,
        seed: Some(181819),
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
            ClientMessage::SetStationRating {
                rating_name: station.ratings[0].name.clone(),
            },
            ClientMessage::SetReady { ready: true },
        ] {
            send(&mut app, &token, msg);
            steps(&mut app, 4);
        }
    }
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress,
        "ordinary headless/lobby bootstrap must reach the actual mission phase"
    );
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

fn follower(app: &mut App) -> Entity {
    app.world_mut()
        .query::<(Entity, &EntityId)>()
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == "formation-wing").then_some(entity))
        .unwrap()
}
fn earned_leader_motion(app: &App, observer: Entity, public_name: &str) -> bool {
    let config = &app.world().get::<ShipConfigComponent>(observer).unwrap().0;
    let boards = app.world().get::<ShipSystemBlackboards>(observer).unwrap();
    config
        .observations_for_system(&id("navigation"), &boards.0)
        .values()
        .any(|board| {
            let SystemBlackboard::TacticalRadar(board) = board else {
                return false;
            };
            board.tracks.as_ref().is_some_and(|records| {
                records.iter().any(|record| {
                    !record.predicted
                        && record.selectable
                        && record.velocity.is_some()
                        && record
                            .identity
                            .as_ref()
                            .is_some_and(|identity| identity.name == public_name)
                })
            })
        })
}
fn following(app: &App, actor: Entity) -> bool {
    let Some(waypoint) = app
        .world()
        .get::<project_phoenix::console::navigation::NavigationWaypoint>(actor)
    else {
        return false;
    };
    if !matches!(waypoint.mode(), Some(project_phoenix::console::navigation::WaypointMode::Formation { course }) if !course.paused)
    {
        return false;
    }
    let generation = waypoint.generation();
    app.world().get::<HelmControlState>(actor).is_some_and(|state|
        matches!(state.axes.get(&id("helm-thrust")), Some(AxisIntent::Thrust(ThrustIntent::Assisted { demand: SpeedDemand::Navigation { generation: held } })) if *held == generation))
        && app.world().resource::<HelmMotionPlan>().ships.get(&actor).is_some_and(|plan| plan.navigation_generation == Some(generation))
        && app.world().get::<ShipPowerSystem>(actor).unwrap().0.emitted_work(&id("helm-thrust")) > 0.0
}
#[test]
fn actual_human_form_up_drives_ordinary_backfill_motion_and_whole_json360_continuation() {
    let (mut live, leader) = boot();
    let wing = follower(&mut live);
    let receiver = (0..1800)
        .find_map(|_| {
            let found = live
                .world()
                .get::<ShipSystemBlackboards>(leader)
                .unwrap()
                .0
                .values()
                .find_map(|board| {
                    let SystemBlackboard::Comms(board) = board else {
                        return None;
                    };
                    board
                        .contacts
                        .iter()
                        .find(|contact| contact.name == "formation-wing" && contact.in_range)
                        .map(|contact| contact.uuid.clone())
                });
            if found.is_none() {
                live.update();
            }
            found
        })
        .expect("actual own Comms roster must acquire the explicitly hailable receiver");
    assert_eq!(live.world().get::<EntityUuid>(wing).unwrap().0, receiver);
    let slot = [10_i32, 20_i32];
    let radar_range = live
        .world()
        .get::<WeaponsConsoleSection>(wing)
        .unwrap()
        .0
        .radar
        .as_ref()
        .unwrap()
        .range;
    assert_eq!(
        radar_range, 50.0,
        "the ordinary hull radar remains unchanged"
    );
    assert!(
        (slot[0] as f32).hypot(slot[1] as f32) < radar_range,
        "the requested slot must fit inside the independently observed motion range"
    );
    // Only a prerequisite assertion uses the actual sender's public name. No
    // private actor identity or position is supplied to Navigation or Helm.
    let leader_name = live.world().get::<EntityName>(leader).unwrap().0.clone();
    for _ in 0..1800 {
        if earned_leader_motion(&live, wing, &leader_name) {
            break;
        }
        live.update();
    }
    assert!(
        earned_leader_motion(&live, wing, &leader_name),
        "Navigation must independently earn the leader's public identity and motion through its permitted radar feed before coordination"
    );
    control(
        &mut live,
        leader,
        "comms",
        Control::SendCommsRequest {
            channel: "public".into(),
            audience: CommsAudience::Contact {
                target_uuid: receiver.clone(),
            },
            request: GenericCommsRequest::FormUp { slot },
            planned_request: None,
        },
    );
    control(
        &mut live,
        leader,
        "helm-thrust",
        Control::SetThrust { value: 0.2 },
    );
    for _ in 0..2400 {
        if following(&live, wing) {
            break;
        }
        live.update();
    }
    assert!(following(&live, wing), "ordinary paid reading, accepted coordination, earned identity, cleared Navigation and Helm travel must join without a synthetic order; waypoint={:?}, axes={:?}, paid_thrust={}",
        live.world().get::<project_phoenix::console::navigation::NavigationWaypoint>(wing),
        live.world().get::<HelmControlState>(wing),
        live.world().get::<ShipPowerSystem>(wing).unwrap().0.emitted_work(&id("helm-thrust")));

    let own_uuid = live.world().get::<EntityUuid>(leader).unwrap().0.clone();
    let waypoint = live
        .world()
        .get::<project_phoenix::console::navigation::NavigationWaypoint>(wing)
        .unwrap();
    let Some(project_phoenix::console::navigation::WaypointMode::Formation { course }) =
        waypoint.mode()
    else {
        panic!("actual selected course")
    };
    let request = live
        .world()
        .resource::<project_phoenix::comms::server::CommsInboxRes>()
        .0
        .messages()
        .into_iter()
        .find(|message| message.id == course.request.message_id)
        .unwrap();
    assert_eq!(request.sender_uuid.as_str(), own_uuid.as_str());
    assert!(request
        .request_progress
        .as_ref()
        .is_some_and(|progress| progress
            .facts
            .decision
            .as_ref()
            .is_some_and(|decision| decision.choice == CommsRequestDecision::Accept)));
    assert!(live.world().get::<ShipSystemBlackboards>(wing).unwrap().0.values().any(|board| {
        let SystemBlackboard::TacticalRadar(board) = board else { return false; };
        board.tracks.as_ref().is_some_and(|records| records.iter().any(|record| record.contact_id == course.leader
            && record.identity.as_ref().is_some_and(|identity| identity.name == course.request.source_name)
            && record.velocity.is_some()))
    }), "Navigation selected an independently earned public name and motion, never the radio's hidden route UUID");
    let selected_generation = waypoint.generation();
    let checkpoint = *live.world().get::<ShipPhysics>(wing).unwrap();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored_leader) = boot();
    let restored_wing = follower(&mut resumed);
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for mutate_axis in [false, true] {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == receiver)
            .unwrap();
        if mutate_axis {
            row.helm_movement.as_mut().unwrap().axes.insert(
                id("helm-thrust"),
                AxisIntent::Thrust(ThrustIntent::Assisted {
                    demand: SpeedDemand::Navigation {
                        generation: selected_generation + 1,
                    },
                }),
            );
        } else {
            row.motion_plan.as_mut().unwrap().navigation_generation = Some(selected_generation + 1);
        }
        assert!(!restore(resumed.world_mut(), &bad).is_complete());
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{result:?}");
    for step in 0..360 {
        same(&live, &resumed, step);
        if step == 120 {
            // Deliver the same real Human request on the next ordinary update;
            // no hidden extra simulation steps are inserted into JSON360.
            for (app, actor) in [(&mut live, leader), (&mut resumed, restored_leader)] {
                let payload = Control::SetSteering { value: 0.05 };
                let token = human_token(app, actor, "helm-steering", &payload).unwrap();
                send(
                    app,
                    &token,
                    ClientMessage::ControlSystem {
                        target: id("helm-steering"),
                        payload,
                    },
                );
            }
        }
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    for (app, wing) in [(&live, wing), (&resumed, restored_wing)] {
        let moved = app.world().get::<ShipPhysics>(wing).unwrap();
        let travelled = ((f64::from(moved.x) - f64::from(checkpoint.x)).powi(2)
            + (f64::from(moved.z) - f64::from(checkpoint.z)).powi(2))
        .sqrt();
        assert!(travelled > 5.0, "the ordinary follower must continue physical travel after its first paid acknowledgement: {travelled}");
        assert_eq!(
            app.world()
                .get::<project_phoenix::console::navigation::NavigationWaypoint>(wing)
                .unwrap()
                .generation(),
            selected_generation,
            "legitimate leader refresh retains semantic course identity"
        );
    }
}
