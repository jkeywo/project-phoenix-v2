//! Actual finite identity-only body, ordinary paid emission and Bay custody.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use bevy_rapier3d::prelude::{RapierColliderHandle, RapierRigidBodyHandle};
use project_phoenix::{
    core::messages::{ClientMessage, SystemBlackboard, SystemControlPayload as Control, SystemId},
    entities::spawner::{EntityId, EntityName, EntityUuid, FactionComponent},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    science::broad::BroadTracks,
    science::identity::{IdentityEmissions, ShipIdentityEmitters},
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
            "tests/fixtures/battery-decoy-auto-world.toml"
        } else {
            "tests/fixtures/battery-decoy-world.toml"
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
    app.init_resource::<PaidWitness>();
    app.add_systems(
        FixedUpdate,
        observe
            .in_set(project_phoenix::sim_sets::SimSet::Input)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickJamming)
            .before_ignore_deferred(project_phoenix::sim_sets::FixedStep::RefreshJammedCommsRange),
    );
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
    let child = entity(&mut app, "battery-decoy");
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
// A canonical reserve may lose at most the authored full-battery passive
// rate over this actual fixed interval. Machinery work would exceed this bound;
// no ephemeral work receipt is assumed to survive storage or restoration.
fn passive_loss_bound(app: &App, actor: Entity, owner: &str, started: u64) -> f64 {
    let power = &app.world().get::<ShipPowerSystem>(actor).unwrap().0;
    let rate = power
        .config()
        .consumers
        .iter()
        .find(|consumer| consumer.system == id(owner))
        .unwrap()
        .reserve
        .passive_loss_at_full_per_sec;
    let ticks = app
        .world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0
        - started;
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    f64::from(rate) * ticks as f64 * period.as_secs_f64() + 0.001
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

#[derive(Resource, Default, Debug)]
struct PaidWitness(std::collections::BTreeMap<String, (u64, f64, Option<String>)>);
fn observe(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    rows: Query<(&EntityUuid, &ShipPowerSystem, &IdentityEmissions)>,
    mut witness: ResMut<PaidWitness>,
) {
    witness.0.clear();
    for (uuid, power, emissions) in &rows {
        witness.0.insert(
            uuid.0.clone(),
            (
                tick.0,
                power.0.emitted_work(&id("identity-emitter")),
                emissions.claim(tick.0).map(|claim| claim.callsign.clone()),
            ),
        );
    }
}
fn paid(app: &App, uuid: &str) -> bool {
    app.world()
        .resource::<PaidWitness>()
        .0
        .get(uuid)
        .is_some_and(|(tick, work, claim)| {
            tick.checked_add(1)
                == Some(
                    app.world()
                        .resource::<project_phoenix::sim_tick::SimTick>()
                        .0,
                )
                && *work > 0.0
                && claim.as_deref() == Some("Harrow frigate")
        })
}
fn picture(
    app: &App,
    observer: Entity,
    source: &str,
) -> Option<(String, String, Option<String>, u64)> {
    let tracks = app.world().get::<BroadTracks>(observer)?;
    let owner = id("sensor-radar");
    let state = tracks.0.get(&owner)?;
    let config = app.world().get::<ShipConfigComponent>(observer)?;
    let boards = app.world().get::<ShipSystemBlackboards>(observer)?;
    let permitted = config.0.observations_for_system(&owner, &boards.0);
    let SystemBlackboard::SensorRadar(board) = permitted.get(&owner)? else {
        return None;
    };
    let published = board.tracks.as_deref()?;
    state.tracks.iter().find_map(|(alias, track)| {
        if !track.selectable || state.resolve_current(alias) != Some(source) {
            return None;
        }
        let current = published.iter().find(|record| {
            record.contact_id == *alias
                && record.selectable
                && !record.predicted
                && record.observed_tick == track.observed_tick
        })?;
        current.identity.as_ref().map(|identity| {
            (
                alias.clone(),
                identity.name.clone(),
                identity.faction.clone(),
                current.observed_tick,
            )
        })
    })
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
fn continued(live: &mut App, saved: &PhoenixSnapshot) {
    let (mut resumed, _, _) = boot(false);
    assert!(ready_to_restore(resumed.world(), saved));
    let result = restore(resumed.world_mut(), saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    for step in 0..360 {
        same(live, &resumed, step);
        live.update();
        resumed.update();
    }
    same(live, &resumed, 360);
}
fn json(app: &App) -> PhoenixSnapshot {
    serde_json::from_str(&serde_json::to_string(&capture(app.world())).unwrap()).unwrap()
}
#[test]
fn actual_sensorless_decoy_handles_paid_claim_storage_depletion_and_same_body_json360() {
    let (mut live, parent, decoy) = boot(false);
    let observer = entity(&mut live, "decoy-observer");
    let uuid = live.world().get::<EntityUuid>(decoy).unwrap().0.clone();
    assert!(stored(&live, decoy));
    assert!(
        live.world().get::<BroadTracks>(decoy).is_none(),
        "sensorless equipment has no observer state to pool"
    );
    assert!(live.world().get::<ShipIdentityEmitters>(decoy).is_some());
    assert!(live.world().get::<RapierColliderHandle>(decoy).is_none());
    assert!(live.world().get::<RapierRigidBodyHandle>(decoy).is_none());
    let before = reserve(&live, decoy, "identity-emitter");
    steps(&mut live, 30);
    assert!(!paid(&live, &uuid));
    assert!(
        picture(&live, observer, &uuid).is_none(),
        "no phantom contact is created by stored equipment"
    );
    assert!(reserve(&live, decoy, "identity-emitter") < before);
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
    let saved = json(&live);
    continued(&mut live, &saved);
    wait(&mut live, |app| {
        paid(app, &uuid)
            && picture(app, observer, &uuid).is_some_and(|(_, name, faction, _)| {
                name == "Harrow frigate"
                    && faction.as_deref() == Some("cccccccc-3333-4333-8333-cccccccccccc")
            })
    });
    let alias = picture(&live, observer, &uuid).unwrap().0;
    assert_ne!(
        alias, uuid,
        "observer's earned handle is not a true-identity grant"
    );
    assert_eq!(
        live.world().get::<EntityName>(decoy).unwrap().0,
        "battery-decoy"
    );
    assert_eq!(
        live.world()
            .get::<FactionComponent>(decoy)
            .unwrap()
            .0
            .to_string(),
        "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
    );
    assert!(!stored(&live, decoy));
    assert!(live.world().get::<RapierColliderHandle>(decoy).is_some());
    assert!(live.world().get::<RapierRigidBodyHandle>(decoy).is_some());
    assert!(reserve(&live, decoy, "identity-emitter") < before);
    wait(&mut live, |app| {
        recovery_contact(app, parent, &uuid).is_some()
    });
    let contact = recovery_contact(&live, parent, &uuid).unwrap();
    control(
        &mut live,
        "tactical",
        "probe-bay",
        Control::RecoverCraft {
            contact,
            reception: None,
        },
    );
    wait(&mut live, |app| stored(app, decoy));
    steps(&mut live, 2);
    let finite = reserve(&live, decoy, "identity-emitter");
    assert!(
        finite > 0.0 && finite < before,
        "recover a still-useful finite emitter rather than an exhausted fixture"
    );
    let stored_tick = live
        .world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0;
    let intent = live
        .world()
        .get::<ShipIdentityEmitters>(decoy)
        .unwrap()
        .clone();
    steps(&mut live, 30);
    assert!(!paid(&live, &uuid));
    assert!(reserve(&live, decoy, "identity-emitter") < finite);
    assert!(
        f64::from(finite - reserve(&live, decoy, "identity-emitter"))
            <= passive_loss_bound(&live, decoy, "identity-emitter", stored_tick),
        "funded stored emitter performs no active work"
    );
    assert_eq!(
        live.world().get::<ShipIdentityEmitters>(decoy).unwrap(),
        &intent,
        "storage preserves actual selected intent rather than switching machinery off"
    );
    assert!(live.world().get::<RapierColliderHandle>(decoy).is_none());
    let saved = json(&live);
    let original = capture(live.world());
    let digest = world_digest(live.world());
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
        assert!(!restore(live.world_mut(), &invalid).is_complete());
        assert_eq!(capture(live.world()), original);
        assert_eq!(world_digest(live.world()), digest);
    }
    continued(&mut live, &saved);
    let before_release = reserve(&live, decoy, "identity-emitter");
    control(
        &mut live,
        "tactical",
        "probe-bay",
        Control::LaunchCraft {
            craft: uuid.clone(),
        },
    );
    wait(&mut live, |app| paid(app, &uuid));
    assert_eq!(live.world().get::<EntityUuid>(decoy).unwrap().0, uuid);
    assert!(
        reserve(&live, decoy, "identity-emitter") <= before_release,
        "relaunch cannot refill the actual emitter"
    );
    let saved = json(&live);
    continued(&mut live, &saved);
    wait(&mut live, |app| {
        reserve(app, decoy, "identity-emitter") == 0.0 && !paid(app, &uuid)
    });
    wait(&mut live, |app| {
        picture(app, observer, &uuid).is_some_and(|(_, name, faction, _)| {
            name == "battery-decoy"
                && faction.as_deref() == Some("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa")
        })
    });
    assert!(!stored(&live, decoy));
    assert!(
        live.world().get::<RapierColliderHandle>(decoy).is_some(),
        "short claim life is not a bespoke body expiry"
    );
    assert_eq!(live.world().get::<EntityUuid>(decoy).unwrap().0, uuid);
}
#[test]
fn ordinary_bay_tree_can_launch_the_same_identity_only_equipment() {
    let (mut app, parent, decoy) = boot(true);
    let uuid = app.world().get::<EntityUuid>(decoy).unwrap().0.clone();
    wait(&mut app, |app| paid(app, &uuid));
    assert!(!stored(&app, decoy));
    assert!(bay(&app, parent)
        .job
        .as_ref()
        .is_some_and(|job| job.fraction == 1.0));
    assert!(app.world().get::<BroadTracks>(decoy).is_none());
    assert!(reserve(&app, decoy, "identity-emitter") < 24.0);
}
