//! Genuine local chart -> ordinary Navigation clearance -> paid Full3D Helm.
//! No launch/voyage, remote order, private destination or injected work/knowledge.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use bevy_rapier3d::prelude::{RapierColliderHandle, RapierRigidBodyHandle};
use project_phoenix::{
    ai::server::AiHighFidelity,
    console::navigation::{NavigationWaypoint, WaypointMode},
    core::messages::{ClientMessage, GamePhase, SystemBlackboard, SystemId},
    entities::spawner::{EntityId, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::server::InboundMessage,
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{
        control_source::ControlSource, helm_planner::HelmMotionPlan, power::ShipPowerSystem,
        state::ShipPhysics,
    },
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
use std::collections::BTreeMap;
const POINT: [f32; 3] = [25.25, 35.5, -45.75];
const OWNERS: [&str; 5] = [
    "navigation",
    "helm-thrust",
    "helm-steering",
    "helm-lateral-thrust",
    "helm-vertical-thrust",
];
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
#[derive(Resource, Default)]
struct PaidWitness(BTreeMap<String, f64>);
/// Test-only read observer after real machinery work; it pays/grants nothing.
fn observe_paid(ships: Query<(&EntityId, &ShipPowerSystem)>, mut witness: ResMut<PaidWitness>) {
    for (entity, power) in &ships {
        if entity.0 != "charted-utility" {
            continue;
        }
        for owner in OWNERS {
            *witness.0.entry(owner.into()).or_default() += power.0.emitted_work(&id(owner));
        }
    }
}
fn entity(app: &mut App, name: &str) -> Entity {
    app.world_mut()
        .query::<(Entity, &EntityId)>()
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == name).then_some(entity))
        .unwrap()
}
fn steps(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}
fn boot() -> (App, Entity) {
    // The first gate also exercises this genuine resolver/accepted schema path.
    project_phoenix::entities::include_resolve::load_entity_config(
        "tests/fixtures/charted-utility-shuttle.toml",
    )
    .unwrap();
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/charted-utility-world.toml".into(),
        ship_path: "assets/entities/alliance_courier.toml".into(),
        deterministic: true,
        seed: Some(191924),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<PaidWitness>();
    app.add_systems(FixedLast, observe_paid);
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let local = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let stations = app
        .world()
        .get::<ShipConfigComponent>(local)
        .unwrap()
        .0
        .stations
        .clone();
    for station in stations {
        let token = format!("charted-{}", station.id.0);
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
            app.world_mut().write_message(InboundMessage {
                token: token.clone(),
                msg,
            });
            steps(&mut app, 4);
        }
    }
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
    let craft = entity(&mut app, "charted-utility");
    (app, craft)
}
fn course(
    app: &App,
    craft: Entity,
) -> Option<(
    u64,
    phoenix_model::navigation_point::ChartedNavigationCourse,
)> {
    let waypoint = app.world().get::<NavigationWaypoint>(craft)?;
    match waypoint.mode()? {
        WaypointMode::ChartedXYZ { course } => Some((waypoint.generation(), course.clone())),
        _ => None,
    }
}
fn assert_real_owners(app: &App, craft: Entity) {
    assert!(
        app.world().get::<AiHighFidelity>(craft).is_some(),
        "the ordinary fleet bubble must retain the full Helm integrator"
    );
    assert!(app.world().get::<RapierRigidBodyHandle>(craft).is_some());
    assert!(app.world().get::<RapierColliderHandle>(craft).is_some());
    let config = &app.world().get::<ShipConfigComponent>(craft).unwrap().0;
    assert!(config.stations.is_empty());
    assert!(config.systems.iter().all(|system| system.ai_only));
    let sources = &app
        .world()
        .get::<ShipSystemControlSources>(craft)
        .unwrap()
        .0;
    for owner in OWNERS {
        assert_eq!(
            sources.source_for(&id(owner)),
            ControlSource::Ai,
            "ordinary owner {owner}"
        );
    }
    let power = &app.world().get::<ShipPowerSystem>(craft).unwrap().0;
    assert_eq!(
        power.network().output_actual_per_sec,
        0.0,
        "no hidden generation"
    );
    for owner in OWNERS {
        let state = power.network().consumers[&id(owner)];
        assert!(state.energy.is_finite() && state.energy > 0.0 && state.energy < 240.0);
        assert!(state.heat.is_finite());
    }
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
        "whole continuation at{step}: {:?}",
        first_difference(&left, &right, "$")
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest at{step}"
    );
}
fn settled(app: &App, craft: Entity) -> bool {
    let pose = app.world().get::<ShipPhysics>(craft).unwrap();
    (pose.x - POINT[0]).hypot(pose.z - POINT[2]) < 2.5
        && (pose.y - POINT[1]).abs() < 0.6
        && pose.forward_speed.hypot(pose.lateral_speed) < 1.0
        && pose.vertical_speed.abs() < 0.6
}
#[test]
fn actual_charted_utility_uses_paid_xyz_helm_and_whole_json360_continuation() {
    let (mut live, craft) = boot();
    for _ in 0..900 {
        let paid = live.world().resource::<PaidWitness>();
        let selected = course(&live, craft);
        let following = selected.as_ref().is_some_and(|(generation, _)| {
            live.world()
                .resource::<HelmMotionPlan>()
                .ships
                .get(&craft)
                .is_some_and(|plan| plan.navigation_generation == Some(*generation))
        });
        if following
            && ["navigation", "helm-thrust", "helm-vertical-thrust"]
                .iter()
                .all(|owner| paid.0.get(*owner).is_some_and(|work| *work > 0.0))
        {
            break;
        }
        live.update();
    }
    let (generation, selected) = course(&live, craft)
        .expect("ordinary selector must pay/admit the actual offered chart point");
    assert_eq!(selected.destination.point, POINT);
    let marker = entity(&mut live, "charted-point");
    assert_eq!(
        selected.destination.id,
        live.world().get::<EntityUuid>(marker).unwrap().0
    );
    let boards = live.world().get::<ShipSystemBlackboards>(craft).unwrap();
    let SystemBlackboard::Navigation(board) = &boards.0[&id("navigation")] else {
        panic!()
    };
    assert_eq!(board.charted_course.as_ref(), Some(&selected));
    assert!(board
        .charted_points
        .iter()
        .any(|point| point == &selected.destination));
    assert!(
        board.transit_routes.is_empty(),
        "foreign arrival points never supply this local chart choice"
    );
    assert_real_owners(&live, craft);
    let paid = live.world().resource::<PaidWitness>();
    for owner in ["navigation", "helm-thrust", "helm-vertical-thrust"] {
        assert!(
            paid.0.get(owner).is_some_and(|work| *work > 0.0),
            "actual paid owner {owner}"
        );
    }
    let checkpoint = *live.world().get::<ShipPhysics>(craft).unwrap();
    assert!(
        checkpoint.y > 4.25 && checkpoint.vertical_speed > 0.0,
        "existing paid vertical integrator must already be flying"
    );
    let reserve = live
        .world()
        .get::<ShipPowerSystem>(craft)
        .unwrap()
        .0
        .network()
        .consumers[&id("helm-vertical-thrust")]
        .energy;
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let craft_uuid = live.world().get::<EntityUuid>(craft).unwrap().0.clone();
    let (mut resumed, restored_craft) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == craft_uuid)
            .unwrap();
        match fault {
            0 => row.navigation_waypoint.as_mut().unwrap().generation = 0,
            1 => row.navigation_waypoint.as_mut().unwrap().charted = None,
            _ => {
                row.navigation_waypoint
                    .as_mut()
                    .unwrap()
                    .charted
                    .as_mut()
                    .unwrap()
                    .destination
                    .point[1] += 10.0;
                for (_, board) in &mut row.blackboards {
                    if let SystemBlackboard::Navigation(board) = board {
                        if let Some(course) = &mut board.charted_course {
                            course.destination.point[1] += 10.0;
                        }
                    }
                }
            }
        }
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "forged chart course {fault} must refuse"
        );
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{result:?}");
    let work_before = [
        live.world().resource::<PaidWitness>().0["helm-vertical-thrust"],
        resumed.world().resource::<PaidWitness>().0["helm-vertical-thrust"],
    ];
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    for (index, (app, entity)) in [(&live, craft), (&resumed, restored_craft)]
        .into_iter()
        .enumerate()
    {
        assert!(app.world().resource::<PaidWitness>().0["helm-vertical-thrust"] > work_before[index],
            "restored motion must include actual newly paid vertical work, not only inherited inertia");
        let pose = app.world().get::<ShipPhysics>(entity).unwrap();
        let displacement = Vec3::new(
            pose.x - checkpoint.x,
            pose.y - checkpoint.y,
            pose.z - checkpoint.z,
        )
        .length();
        assert!(
            displacement > 5.0,
            "post-checkpoint physical movement, not boot travel: {displacement}"
        );
        assert_eq!(course(app, entity), Some((generation, selected.clone())));
        assert_real_owners(app, entity);
        assert!(
            app.world()
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .network()
                .consumers[&id("helm-vertical-thrust")]
                .energy
                < reserve
        );
    }
    // Continue only through the same ordinary integrator; no pose or intent fixup.
    let mut settling_updates = 0;
    for _ in 0..1500 {
        if settled(&live, craft) && settled(&resumed, restored_craft) {
            break;
        }
        live.update();
        resumed.update();
        settling_updates += 1;
    }
    assert!(
        settled(&live, craft),
        "real XYZ leg must settle under its authored acceleration/braking"
    );
    assert!(settled(&resumed, restored_craft));
    same(&live, &resumed, 360 + settling_updates);
}
