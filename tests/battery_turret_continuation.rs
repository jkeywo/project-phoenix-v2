//! Actual fixed beam turret uses ordinary Bay, radar, beam AI and finite power.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use bevy_rapier3d::prelude::{RapierColliderHandle, RapierRigidBodyHandle};
use project_phoenix::{
    console::weapons::beam::ActiveBeam,
    core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control, SystemId},
    entities::spawner::{EntityId, EntitySystemHull, EntityUuid},
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
fn boot(auto: bool) -> (App, Entity, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: if auto {
            "tests/fixtures/battery-turret-auto-world.toml"
        } else {
            "tests/fixtures/battery-turret-world.toml"
        }
        .into(),
        ship_path: if auto {
            "tests/fixtures/turret-auto-carrier.toml"
        } else {
            "assets/entities/probe_carrier.toml"
        }
        .into(),
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
    let child = entity(&mut app, "battery-turret");
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
                rating_name: if auto && station.id.0 == "tactical" {
                    "Simplified".into()
                } else {
                    station.ratings[0].name.clone()
                },
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
    wait(&mut app, |app| {
        app.world().get::<ShipPhysics>(parent).is_some_and(|pose| {
            [pose.forward_speed, pose.lateral_speed, pose.vertical_speed]
                .iter()
                .all(|speed| speed.abs() <= 0.03)
        })
    });
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

fn hp(app: &App, target: Entity) -> f32 {
    app.world()
        .get::<EntitySystemHull>(target)
        .unwrap()
        .0
        .total_current()
}
fn earned_hostile(app: &App, turret: Entity) -> bool {
    let Some(SystemBlackboard::TacticalRadar(board)) = app
        .world()
        .get::<ShipSystemBlackboards>(turret)
        .unwrap()
        .0
        .get(&id("tactical-radar"))
    else {
        return false;
    };
    board
        .tracks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|track| {
            track.selectable
                && !track.predicted
                && track.identity.as_ref().is_some_and(|identity| {
                    identity.name == "hostile-depot"
                        && identity.faction.as_deref()
                            == Some("cccccccc-3333-4333-8333-cccccccccccc")
                })
        })
}
fn paid_firing(app: &App, turret: Entity, hostile: &str) -> bool {
    earned_hostile(app, turret)
        && app
            .world()
            .get::<ActiveBeam>(turret)
            .unwrap()
            .bank_target("turret")
            == Some(hostile)
        && app
            .world()
            .get::<ShipPowerSystem>(turret)
            .unwrap()
            .0
            .network()
            .consumers[&id("phaser-turret")]
            .emitted_work
            > 0.0
}
fn recovery_contact(app: &App, parent: Entity, uuid: &str) -> Option<String> {
    bay(app, parent)
        .contacts
        .into_iter()
        .find(|contact| {
            project_phoenix::science::contact_routing::resolve(
                &app.world().get::<ShipConfigComponent>(parent).unwrap().0,
                app.world().get::<ShipSystemBlackboards>(parent).unwrap(),
                app.world().get::<BroadTracks>(parent),
                &id("probe-bay"),
                &contact.contact,
            )
            .as_deref()
                == Some(uuid)
        })
        .map(|contact| contact.contact)
}

#[test]
fn human_launch_ordinary_acquired_beam_and_stored_custody_continue_json360() {
    let (mut live, parent, turret) = boot(false);
    let hostile = entity(&mut live, "hostile-depot");
    let friendly = entity(&mut live, "friendly-depot");
    let hostile_uuid = live.world().get::<EntityUuid>(hostile).unwrap().0.clone();
    let uuid = live.world().get::<EntityUuid>(turret).unwrap().0.clone();
    let friendly_hp = hp(&live, friendly);
    let hostile_hp = hp(&live, hostile);
    assert!(stored(&live, turret));
    assert!(live.world().get::<ActiveBeam>(turret).is_some());
    assert!(live
        .world()
        .get::<project_phoenix::server_app::Ship>(turret)
        .is_some());
    let stored_tracks = live.world().get::<BroadTracks>(turret).unwrap().clone();
    steps(&mut live, 30);
    assert_eq!(
        live.world().get::<BroadTracks>(turret).unwrap().0,
        stored_tracks.0
    );
    assert_eq!(hp(&live, hostile), hostile_hp);
    assert!(!live.world().get::<ActiveBeam>(turret).unwrap().is_firing());
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
    let (mut resumed, _, _) = boot(false);
    assert!(ready_to_restore(resumed.world(), &saved));
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    assert!(!stored(&live, turret));
    assert_eq!(live.world().get::<EntityUuid>(turret).unwrap().0, uuid);
    assert!(live.world().get::<RapierColliderHandle>(turret).is_some());
    assert!(reserve(&live, parent, "probe-bay") < bay_before);
    wait(&mut live, |app| paid_firing(app, turret, &hostile_uuid));
    // A second fresh checkpoint is inside an actual acquired, paid beam cycle,
    // not only the handling phase. It must keep the real weapon/Radar/custody
    // owner graph and ordinary autonomous continuation exactly.
    let firing_saved: PhoenixSnapshot =
        serde_json::from_str(&serde_json::to_string(&capture(live.world())).unwrap()).unwrap();
    let (mut firing_resume, _, _) = boot(false);
    assert!(ready_to_restore(firing_resume.world(), &firing_saved));
    let result = restore(firing_resume.world_mut(), &firing_saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    for step in 0..360 {
        same(&live, &firing_resume, step);
        live.update();
        firing_resume.update();
    }
    same(&live, &firing_resume, 360);
    let weapon_before = reserve(&live, turret, "phaser-turret");
    steps(&mut live, 60);
    assert!(reserve(&live, turret, "phaser-turret") < weapon_before);
    assert!(
        hp(&live, hostile) < hostile_hp,
        "real paid ordinary beam must physically damage acquired hostile"
    );
    assert_eq!(
        hp(&live, friendly),
        friendly_hp,
        "closer known friendly is not an autonomous target"
    );
    // No Comms equipment or parent feed is installed on this local autonomous
    // turret. This proves onboard work without a command link, not a separate
    // remote order/link-loss or media feature.
    assert!(live
        .world()
        .get::<ShipConfigComponent>(turret)
        .unwrap()
        .0
        .systems
        .iter()
        .all(|system| system.kind != "comms"));
    wait(&mut live, |app| {
        recovery_contact(app, parent, &uuid).is_some()
    });
    let contact = recovery_contact(&live, parent, &uuid).unwrap();
    control(
        &mut live,
        "tactical",
        "probe-bay",
        Control::RecoverCraft { contact },
    );
    wait(&mut live, |app| stored(app, turret));
    steps(&mut live, 2);
    assert!(live.world().get::<RapierColliderHandle>(turret).is_none());
    assert!(live.world().get::<RapierRigidBodyHandle>(turret).is_none());
    let hp_stored = hp(&live, hostile);
    let finite = reserve(&live, turret, "phaser-turret");
    let held = live.world().get::<BroadTracks>(turret).unwrap().clone();
    steps(&mut live, 30);
    assert_eq!(
        hp(&live, hostile),
        hp_stored,
        "Carried entity cannot keep dealing beam damage"
    );
    assert!(
        reserve(&live, turret, "phaser-turret") < finite,
        "ordinary passive reserve loss continues"
    );
    assert_eq!(live.world().get::<BroadTracks>(turret).unwrap().0, held.0);
    assert_eq!(
        live.world()
            .get::<ShipPowerSystem>(turret)
            .unwrap()
            .0
            .network()
            .consumers[&id("phaser-turret")]
            .emitted_work,
        0.0
    );
    let saved: PhoenixSnapshot =
        serde_json::from_str(&serde_json::to_string(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, _, _) = boot(false);
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut invalid = saved.clone();
        let row = invalid
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
                        parent_uuid: "absent".into(),
                        bay: id("probe-bay"),
                    }
            }
        }
        assert!(!restore(resumed.world_mut(), &invalid).is_complete());
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    assert_eq!(live.world().get::<EntityUuid>(turret).unwrap().0, uuid);
    assert_eq!(hp(&live, hostile), hp_stored);
}

#[test]
fn ordinary_bay_tree_launches_same_turret_and_individual_systems_acquire_and_fire() {
    let (mut app, parent, turret) = boot(true);
    let hostile = entity(&mut app, "hostile-depot");
    let hostile_uuid = app.world().get::<EntityUuid>(hostile).unwrap().0.clone();
    wait(&mut app, |app| paid_firing(app, turret, &hostile_uuid));
    assert!(!stored(&app, turret));
    assert!(bay(&app, parent)
        .job
        .as_ref()
        .is_some_and(|job| job.fraction == 1.0));
    assert!(reserve(&app, turret, "phaser-turret") < 80.0);
    assert!(app
        .world()
        .get::<ShipConfigComponent>(turret)
        .unwrap()
        .0
        .stations
        .is_empty());
}
