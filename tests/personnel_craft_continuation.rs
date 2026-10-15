//! Joined actual personnel escort/Bay handoff, finite custody and complete continuation.
//! One-host evidence only; not network, browser/device or full increment20 acceptance.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use bevy_rapier3d::prelude::{RapierColliderHandle, RapierRigidBodyHandle};
use phoenix_model::{
    job::JobOutcome,
    personnel::{OfficerLocation, PersonnelDestination},
};
use project_phoenix::{
    console::repair::server::ShipRepairTeams,
    core::messages::{
        ClientMessage, StationId, SystemBlackboard, SystemControlPayload as Control, SystemId,
    },
    entities::spawner::{EntityId, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    science::broad::BroadTracks,
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
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
        world_path: "tests/fixtures/personnel-carrier-world.toml".into(),
        ship_path: "assets/entities/personnel_carrier.toml".into(),
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
    let child = entity(&mut app, "personnel-pod");
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
    let parent = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let sessions = app.world().resource::<Sessions>();
    assert_eq!(
        sessions.0.holder_for_station(&StationId(station.into())),
        Some(token.as_str())
    );
    assert!(
        project_phoenix::command_admission::policy::is_command_authorized(
            &token,
            &id(owner),
            &payload,
            app.world().get::<ShipSystemControlSources>(parent).unwrap(),
            sessions,
            &app.world().get::<ShipConfigComponent>(parent).unwrap().0,
            app.world().get::<HumanSeekingHosts>(parent)
        ),
        "actual holder must own the changed System"
    );
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

fn officer(app: &App, parent: Entity, name: &str) -> OfficerLocation {
    let uuid = &app.world().get::<EntityUuid>(parent).unwrap().0;
    capture(app.world()).personnel.origins[uuid].state.officers[name]
        .location
        .clone()
}
fn people(app: &App, parent: Entity) -> phoenix_model::personnel::PersonnelView {
    let SystemBlackboard::Repair(board) =
        &app.world().get::<ShipSystemBlackboards>(parent).unwrap().0[&id("repair")]
    else {
        panic!("actual Repair publication");
    };
    board
        .personnel
        .clone()
        .expect("actual permitted local personnel")
}
fn prepare_escort(app: &mut App, parent: Entity, child: Entity) -> String {
    control(
        app,
        "engineering",
        "repair",
        Control::SetDutyOfficerPosting {
            officer: "repair-chief".into(),
            team_idx: None,
        },
    );
    wait(app, |app| {
        people(app, parent)
            .carried_destinations
            .iter()
            .any(|row| row.craft == app.world().get::<EntityUuid>(child).unwrap().0)
    });
    let uuid = app.world().get::<EntityUuid>(child).unwrap().0.clone();
    control(
        app,
        "engineering",
        "repair",
        Control::SetDutyOfficerDestination {
            officer: "repair-chief".into(),
            destination: Some(PersonnelDestination::Craft {
                craft: uuid.clone(),
            }),
        },
    );
    assert!(
        matches!(
            officer(app, parent, "repair-chief"),
            OfficerLocation::AtCompartment { .. }
        ),
        "intent alone cannot move a person"
    );
    uuid
}
fn dispatch(app: &mut App, person: &str, team: u32) {
    let parent = app
        .world()
        .try_query_filtered::<Entity, With<LocalShip>>()
        .unwrap()
        .single(app.world())
        .unwrap();
    let request = people(app, parent)
        .officers
        .into_iter()
        .find(|officer| officer.id == person)
        .unwrap()
        .destination_request
        .unwrap();
    control(
        app,
        "engineering",
        "repair",
        Control::DispatchOfficerTransfer {
            officer: person.into(),
            team_idx: team,
            request,
        },
    );
}

fn continuation(app: &mut App) {
    continuation_with_operator(app, false);
}
fn continuation_with_operator(app: &mut App, engineer_afk: bool) {
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(app.world())).unwrap()).unwrap();
    let (mut resumed, _, _) = boot();
    if engineer_afk {
        // Session authority is deliberately local to each host. Reproduce the
        // real AFK holder before restoring canonical machinery, so an active
        // bootstrap Human cannot override the saved ordinary Backfill policy.
        send(
            &mut resumed,
            "craft-engineering",
            ClientMessage::SetAfk { afk: true },
        );
        steps(&mut resumed, 4);
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..360 {
        if engineer_afk {
            assert_backfill(app);
            assert_backfill(&resumed);
        }
        same(app, &resumed, step);
        app.update();
        resumed.update();
    }
    if engineer_afk {
        assert_backfill(app);
        assert_backfill(&resumed);
    }
    same(app, &resumed, 360);
}
fn assert_backfill(app: &App) {
    let actor = app
        .world()
        .try_query_filtered::<Entity, With<LocalShip>>()
        .unwrap()
        .single(app.world())
        .unwrap();
    let sources = &app
        .world()
        .get::<ShipSystemControlSources>(actor)
        .unwrap()
        .0;
    assert_eq!(
        sources.source_for(&id("repair")),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    assert!(
        sources.policy_for(&id("repair")).operate_ai,
        "ordinary Repair controller must remain enabled"
    );
}
fn recovery_contact(app: &App, parent: Entity, craft: &str) -> Option<String> {
    bay(app, parent).contacts.into_iter().find_map(|contact| {
        (project_phoenix::science::contact_routing::resolve(
            &app.world().get::<ShipConfigComponent>(parent).unwrap().0,
            app.world().get::<ShipSystemBlackboards>(parent).unwrap(),
            app.world().get::<BroadTracks>(parent),
            &id("probe-bay"),
            &contact.contact,
        )
        .as_deref()
            == Some(craft))
        .then_some(contact.contact)
    })
}
#[test]
fn joined_escort_physical_handoff_launch_recovery_unload_and_json360() {
    let (mut live, parent, child) = boot();
    assert!(stored(&live, child));
    assert!(live.world().get::<RapierColliderHandle>(child).is_none());
    let uuid = prepare_escort(&mut live, parent, child);
    dispatch(&mut live, "repair-chief", 0);
    wait(&mut live, |app| {
        matches!(
            officer(app, parent, "repair-chief"),
            OfficerLocation::WithTransfer { .. }
        )
    });
    let jobs = live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .save_state();
    let assignment = jobs.teams[0].as_ref().unwrap().id;
    assert!(live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .claims_personnel_bay(&id("probe-bay")));
    control(
        &mut live,
        "tactical",
        "probe-bay",
        Control::LaunchCraft {
            craft: uuid.clone(),
        },
    );
    assert!(stored(&live, child));
    assert!(
        bay(&live, parent).job.is_none(),
        "an actual escort owns the handoff boundary"
    );
    control(
        &mut live,
        "engineering",
        "repair",
        Control::SetDutyOfficerDestination {
            officer: "technician".into(),
            destination: Some(PersonnelDestination::Craft {
                craft: uuid.clone(),
            }),
        },
    );
    dispatch(&mut live, "technician", 1);
    assert!(matches!(
        officer(&live, parent, "technician"),
        OfficerLocation::AtCompartment { .. }
    ));
    assert!(!live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .is_personnel_transfer_team(1));
    let saved = capture(live.world());
    let parent_uuid = live.world().get::<EntityUuid>(parent).unwrap().0.clone();
    for fault in 0..8 {
        let mut bad = saved.clone();
        match fault {
            0 => {
                bad.personnel
                    .origins
                    .get_mut(&parent_uuid)
                    .unwrap()
                    .state
                    .officers
                    .remove("repair-chief");
            }
            1 => {
                bad.personnel
                    .origins
                    .get_mut(&parent_uuid)
                    .unwrap()
                    .state
                    .officers
                    .get_mut("repair-chief")
                    .unwrap()
                    .location = OfficerLocation::AboardCraft {
                    craft: uuid.clone(),
                    berth: "missing".into(),
                };
            }
            2 | 3 | 4 | 5 => {
                let job = bad
                    .entities
                    .iter_mut()
                    .find(|row| row.uuid == parent_uuid)
                    .unwrap()
                    .repair
                    .as_mut()
                    .unwrap()
                    .jobs
                    .as_mut()
                    .unwrap()
                    .teams[0]
                    .as_mut()
                    .unwrap();
                let project_phoenix::jobs::repair::RepairActivity::PersonnelTransfer {
                    work, ..
                } = &mut job.activity
                else {
                    panic!("actual escort Job");
                };
                match fault {
                    2 => work.plan.berth = "invented".into(),
                    3 => work.request.requested_tick = bad.tick + 1,
                    4 => work.plan.pickup_compartment = "bridge".into(),
                    _ => work.completed = true,
                }
            }
            6 => {
                bad.entities
                    .iter_mut()
                    .find(|row| row.uuid == uuid)
                    .unwrap()
                    .probe_custody
                    .as_mut()
                    .unwrap()
                    .0 = phoenix_model::craft::CraftLocation::Deployed {};
            }
            _ => {
                bad.entities
                    .iter_mut()
                    .find(|row| row.uuid == parent_uuid)
                    .unwrap()
                    .repair
                    .as_mut()
                    .unwrap()
                    .personnel_destinations
                    .remove("repair-chief");
            }
        }
        let before = capture(live.world());
        let digest = world_digest(live.world());
        let report = restore(live.world_mut(), &bad);
        assert!(!report.is_complete(), "forgery {fault}");
        assert_eq!(capture(live.world()), before);
        assert_eq!(world_digest(live.world()), digest);
    }
    continuation(&mut live);
    assert_eq!(
        officer(&live, parent, "repair-chief"),
        OfficerLocation::AboardCraft {
            craft: uuid.clone(),
            berth: "passenger".into()
        }
    );
    assert!(live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .work_list()
        .iter()
        .any(|job| job.id == assignment && job.outcome == Some(JobOutcome::Completed)));
    assert_eq!(people(&live, parent).carried_destinations[0].free_berths, 0);
    dispatch(&mut live, "technician", 1);
    assert!(
        matches!(
            officer(&live, parent, "technician"),
            OfficerLocation::AtCompartment { .. }
        ),
        "occupied berth cannot be duplicated"
    );
    control(
        &mut live,
        "engineering",
        "repair",
        Control::SetDutyOfficerDestination {
            officer: "technician".into(),
            destination: None,
        },
    );
    let initial = reserve(&live, child, "sensor-radar");
    control(
        &mut live,
        "tactical",
        "probe-bay",
        Control::LaunchCraft {
            craft: uuid.clone(),
        },
    );
    wait(&mut live, |app| !stored(app, child));
    steps(&mut live, 2);
    assert!(live.world().get::<RapierRigidBodyHandle>(child).is_some());
    assert_eq!(
        officer(&live, parent, "repair-chief"),
        OfficerLocation::AboardCraft {
            craft: uuid.clone(),
            berth: "passenger".into()
        }
    );
    assert!(
        people(&live, parent)
            .officers
            .iter()
            .all(|person| person.id != "repair-chief"),
        "deployment cannot reveal remote current crew facts"
    );
    continuation(&mut live);
    assert!(reserve(&live, child, "sensor-radar") <= initial);
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
    wait(&mut live, |app| stored(app, child));
    steps(&mut live, 2);
    assert_eq!(live.world().get::<EntityUuid>(child).unwrap().0, uuid);
    control(
        &mut live,
        "engineering",
        "repair",
        Control::SetDutyOfficerDestination {
            officer: "repair-chief".into(),
            destination: Some(PersonnelDestination::Home),
        },
    );
    dispatch(&mut live, "repair-chief", 0);
    wait(&mut live, |app| {
        matches!(
            officer(app, parent, "repair-chief"),
            OfficerLocation::WithTransfer { .. }
        )
    });
    continuation(&mut live);
    assert_eq!(
        officer(&live, parent, "repair-chief"),
        OfficerLocation::AtCompartment {
            compartment: "engineering".into()
        }
    );
    assert_eq!(people(&live, parent).carried_destinations[0].free_berths, 1);
    assert_eq!(
        capture(live.world()).personnel.origins[&parent_uuid]
            .state
            .officers
            .len(),
        2
    );
}
#[test]
fn ordinary_backfill_executes_explicit_destination_with_the_same_physical_team_job() {
    let (mut live, parent, child) = boot();
    let uuid = prepare_escort(&mut live, parent, child);
    send(
        &mut live,
        "craft-engineering",
        ClientMessage::SetAfk { afk: true },
    );
    steps(&mut live, 4);
    wait(&mut live, |app| {
        matches!(
            officer(app, parent, "repair-chief"),
            OfficerLocation::WithTransfer { .. }
        )
    });
    assert_eq!(
        live.world()
            .get::<ShipSystemControlSources>(parent)
            .unwrap()
            .0
            .source_for(&id("repair")),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    continuation_with_operator(&mut live, true);
    assert_eq!(
        officer(&live, parent, "repair-chief"),
        OfficerLocation::AboardCraft {
            craft: uuid,
            berth: "passenger".into()
        }
    );
    assert_eq!(
        live.world()
            .get::<ShipRepairTeams>(parent)
            .unwrap()
            .0
            .free_team_indices(None)
            .len(),
        2
    );
    assert!(live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .work_list()
        .iter()
        .any(|job| job.outcome == Some(JobOutcome::Completed)));
}

#[test]
fn actual_world_removal_preserves_offship_people_and_resolves_only_actual_host_casualties() {
    for remove_origin in [true, false] {
        let (mut live, parent, child) = boot();
        let uuid = prepare_escort(&mut live, parent, child);
        dispatch(&mut live, "repair-chief", 0);
        wait(&mut live, |app| {
            matches!(
                officer(app, parent, "repair-chief"),
                OfficerLocation::AboardCraft { .. }
            )
        });
        wait(&mut live, |app| {
            !app.world()
                .get::<ShipRepairTeams>(parent)
                .unwrap()
                .0
                .claims_personnel_bay(&id("probe-bay"))
        });
        control(
            &mut live,
            "tactical",
            "probe-bay",
            Control::LaunchCraft {
                craft: uuid.clone(),
            },
        );
        wait(&mut live, |app| !stored(app, child));
        let parent_uuid = live.world().get::<EntityUuid>(parent).unwrap().0.clone();
        let removed = if remove_origin {
            parent_uuid.clone()
        } else {
            uuid.clone()
        };
        let mut deaths = live
            .world()
            .resource::<Messages<project_phoenix::ai::server::AiEntityDestroyed>>()
            .get_cursor();
        // Exercise the existing trusted World removal inbox and its actual
        // DestroyEntity/event consumer. This is not a claim about GM admission.
        live.world_mut()
            .resource_mut::<project_phoenix::world::server::WorldContentRuntime>()
            .pending_gm_despawns
            .push(removed.clone());
        live.update();
        assert!(
            live.world()
                .get_entity(if remove_origin { parent } else { child })
                .is_err(),
            "occupancy cannot protect a body from ordinary removal"
        );
        assert!(
            deaths
                .read(
                    live.world()
                        .resource::<Messages<project_phoenix::ai::server::AiEntityDestroyed>>()
                )
                .any(|event| event.entity_uuid == removed),
            "ordinary destruction event must still be emitted"
        );
        let saved = capture(live.world());
        let origin = &saved.personnel.origins[&parent_uuid];
        if remove_origin {
            assert!(origin.removed_tick.is_some());
            assert_eq!(
                origin.state.officers["repair-chief"].location,
                OfficerLocation::AboardCraft {
                    craft: uuid.clone(),
                    berth: "passenger".into()
                }
            );
            assert_eq!(
                origin.state.officers["technician"].location,
                OfficerLocation::Lost {
                    actor: parent_uuid.clone()
                }
            );
            assert!(live.world().get_entity(child).is_ok());
        } else {
            assert!(origin.removed_tick.is_none());
            assert_eq!(
                origin.state.officers["repair-chief"].location,
                OfficerLocation::Lost {
                    actor: uuid.clone()
                }
            );
            assert!(matches!(
                origin.state.officers["technician"].location,
                OfficerLocation::AtCompartment { .. }
            ));
            assert!(people(&live, parent)
                .officers
                .iter()
                .all(|person| person.id != "repair-chief"));
        }
        continuation(&mut live);
        assert_eq!(
            capture(live.world()).personnel.origins[&parent_uuid]
                .state
                .officers,
            origin.state.officers
        );
    }
}

#[test]
fn retargeted_actual_escort_returns_the_same_person_physically_before_releasing_its_team() {
    let (mut live, parent, child) = boot();
    prepare_escort(&mut live, parent, child);
    dispatch(&mut live, "repair-chief", 0);
    wait(&mut live, |app| {
        matches!(
            officer(app, parent, "repair-chief"),
            OfficerLocation::WithTransfer { .. }
        )
    });
    let assignment = live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .work_list()[0]
        .id;
    control(
        &mut live,
        "engineering",
        "repair",
        Control::SetDutyOfficerDestination {
            officer: "repair-chief".into(),
            destination: Some(PersonnelDestination::Home),
        },
    );
    let job = live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .work_list()[0]
        .clone();
    assert_eq!(job.id, assignment);
    assert!(job.outcome.is_none());
    assert!(
        matches!(
            officer(&live, parent, "repair-chief"),
            OfficerLocation::WithTransfer { .. }
        ),
        "retargeting cannot teleport a committed escort home"
    );
    continuation(&mut live);
    assert_eq!(
        officer(&live, parent, "repair-chief"),
        OfficerLocation::AtCompartment {
            compartment: "engineering".into()
        }
    );
    let job = live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .work_list()[0]
        .clone();
    assert_eq!(job.id, assignment);
    assert_eq!(job.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(people(&live, parent).carried_destinations[0].free_berths, 1);
    assert!(live
        .world()
        .get::<ShipRepairTeams>(parent)
        .unwrap()
        .0
        .free_team_indices(None)
        .contains(&0));
}
