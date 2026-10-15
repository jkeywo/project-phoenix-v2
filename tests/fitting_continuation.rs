//! Whole-host fitting through ordinary joined crew, never seeded controllers.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{ClientMessage, StationId, SystemControlPayload as Control, SystemId},
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::fitting_server::ShipFitting,
    server_app::LocalShip,
    ship::power::ShipPowerSystem,
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
use std::collections::{BTreeMap, BTreeSet};

const CRUISER: &str = "assets/entities/alliance_cruiser.toml";
const FETCH_CRUISER: &str = "tests/fixtures/fitting-fetch-cruiser.toml";
const FETCH_WORLD: &str = "tests/fixtures/fitting-fetch-world.toml";
const SLOT: &str = "fore-beam";
const CHOICE: &str = "cruiser-beam-pulse";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}

fn unstarted(path: &str, hull: &str) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: path.into(),
        ship_path: hull.into(),
        deterministic: true,
        seed: Some(156_213),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app
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
fn begin(fetch: bool) -> App {
    let mut app = unstarted(
        if fetch {
            FETCH_WORLD
        } else {
            "assets/worlds/probe_recovery.toml"
        },
        if fetch { FETCH_CRUISER } else { CRUISER },
    );
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    app
}
fn claim_messages(app: &mut App, station: &str, token: &str) -> Vec<ClientMessage> {
    let entity = local(app);
    // The authored ladder is most detailed first. Selecting complexity uses
    // the public command, including on fleet peers with a different default.
    let rating = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .stations
        .iter()
        .find(|entry| entry.id.0 == station)
        .unwrap()
        .ratings
        .first()
        .unwrap()
        .name
        .clone();
    vec![
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
        ClientMessage::SelectStation {
            station: station.into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: rating,
        },
    ]
}
fn claim(app: &mut App, station: &str, token: &str) {
    for message in claim_messages(app, station, token) {
        send(app, token, message);
        steps(app, 4);
    }
}
fn authorised(
    app: &mut App,
    token: &str,
    station: &str,
    target: &SystemId,
    payload: &Control,
) -> bool {
    let entity = local(app);
    let sessions = app.world().resource::<Sessions>();
    let Some(player) = sessions
        .0
        .players()
        .iter()
        .find(|player| player.token == token)
    else {
        return false;
    };
    player.connected
        && player.ready
        && player.station.as_ref() == Some(&StationId(station.into()))
        && sessions.0.holder_for_station(&StationId(station.into())) == Some(token)
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
fn control(app: &mut App, token: &str, station: &str, target: &str, payload: Control) {
    for _ in 0..120 {
        if authorised(app, token, station, &id(target), &payload) {
            break;
        }
        app.update();
    }
    assert!(
        authorised(app, token, station, &id(target), &payload),
        "actual {station} holder must own {target}"
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
fn prepared(fetch: bool, ai: bool) -> (App, Entity) {
    let mut app = begin(fetch);
    // Claim Repair first so its ordinary Backfill cannot consume a human
    // assignment request while Tactical is still entering the bridge.
    claim(&mut app, "engineering", "fitting-engineer");
    claim(&mut app, "tactical", "fitting-tactical");
    control(
        &mut app,
        "fitting-tactical",
        "tactical",
        "phaser-fore",
        Control::RequestEquipment {
            slot: SLOT.into(),
            equipment: CHOICE.into(),
        },
    );
    let entity = local(&mut app);
    assert!(app
        .world()
        .get::<ShipFitting>(entity)
        .unwrap()
        .requested
        .contains(SLOT));
    if ai {
        send(&mut app, "fitting-engineer", ClientMessage::ReleaseStation);
        steps(&mut app, 4);
    } else {
        control(
            &mut app,
            "fitting-engineer",
            "engineering",
            "repair",
            Control::DispatchRefit {
                team_idx: 0,
                slot: SLOT.into(),
            },
        );
    }
    (app, entity)
}
fn fitting(app: &App, entity: Entity) -> &ShipFitting {
    app.world().get::<ShipFitting>(entity).unwrap()
}
fn parts(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock("machinery_parts")
        .unwrap()
        .level
}
fn inventories(app: &mut App) -> BTreeMap<String, (i64, BTreeSet<String>)> {
    let actors: Vec<_> = app
        .world_mut()
        .query::<(Entity, &EntityUuid, &ShipFitting)>()
        .iter(app.world())
        .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
        .collect();
    actors
        .into_iter()
        .map(|(entity, uuid)| {
            (
                uuid,
                (
                    parts(app, entity),
                    fitting(app, entity)
                        .equipment
                        .assemblies
                        .keys()
                        .cloned()
                        .collect(),
                ),
            )
        })
        .collect()
}
fn checkpoint(fetch: bool, ai: bool, stage: &str) -> (App, Entity) {
    let (mut app, entity) = prepared(fetch, ai);
    for _ in 0..2400 {
        if let Some(work) = fitting(&app, entity).work.get(SLOT) {
            let physical_fetch = stage == "Fetching"
                && app
                    .world()
                    .get::<project_phoenix::console::repair::server::ShipRepairTeams>(entity)
                    .unwrap()
                    .0
                    .refit_location(work.job)
                    .is_some_and(|location| matches!(location, phoenix_model::interior::TeamLocation::Crossing { elapsed, .. } if *elapsed > 0.0));
            if format!("{:?}", work.stage) == stage && (physical_fetch || work.progress > 0.0) {
                if stage != "Fetching" {
                    assert!(app
                        .world()
                        .get::<ShipPowerSystem>(entity)
                        .unwrap()
                        .0
                        .last_work_fraction(&id("repair"))
                        .is_some_and(|fraction| fraction > 0.0));
                }
                return (app, entity);
            }
        }
        app.update();
    }
    panic!(
        "ordinary fitting journey must reach active {stage}: {:?}",
        fitting(&app, entity)
    );
}
fn first_difference(
    left: &serde_json::Value,
    right: &serde_json::Value,
    path: &str,
) -> Option<String> {
    use serde_json::Value;
    if left == right {
        return None;
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                if let Some(diff) = first_difference(
                    &a.get(key).cloned().unwrap_or(Value::Null),
                    &b.get(key).cloned().unwrap_or(Value::Null),
                    &format!("{path}.{key}"),
                ) {
                    return Some(diff);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                if let Some(diff) = first_difference(a, b, &format!("{path}[{index}]")) {
                    return Some(diff);
                }
            }
        }
        _ => {}
    }
    Some(format!("{path}: live={left}, restored={right}"))
}
fn assert_continuation(live: &App, resumed: &App, step: usize) {
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "canonical digest at {step}"
    );
    let a = serde_json::to_value(capture(live.world())).unwrap();
    let b = serde_json::to_value(capture(resumed.world())).unwrap();
    assert!(
        a == b,
        "complete JSON at {step}: {}",
        first_difference(&a, &b, "$").unwrap_or_default()
    );
}

#[test]
fn real_human_and_ordinary_ai_field_fits_resume_active_removal_and_installation_exactly() {
    for (stage, ai) in [("Removing", false), ("Installing", true)] {
        let (mut live, entity) = checkpoint(false, ai, stage);
        let original_items: BTreeSet<_> = fitting(&live, entity)
            .equipment
            .assemblies
            .keys()
            .cloned()
            .collect();
        let saved: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
        let (mut resumed, restored) = prepared(false, ai);
        assert!(ready_to_restore(resumed.world(), &saved));
        let report = restore(resumed.world_mut(), &saved);
        assert!(report.is_complete(), "{stage}: {:?}", report.gaps);
        let initial_parts = parts(&live, entity);
        let mut saw_finished = false;
        let mut finished_ticks = 0;
        for step in 0..1200 {
            assert_continuation(&live, &resumed, step);
            saw_finished |= fitting(&live, entity).work.is_empty()
                && fitting(&live, entity)
                    .equipment
                    .installed(SLOT)
                    .is_some_and(|item| item.definition == CHOICE);
            live.update();
            resumed.update();
            if saw_finished {
                finished_ticks += 1;
                if finished_ticks >= 20 && step >= 359 {
                    break;
                }
            }
        }
        assert!(saw_finished, "{stage} must finish actual physical work");
        assert_eq!(
            fitting(&live, entity)
                .equipment
                .assemblies
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            original_items
        );
        assert_eq!(
            fitting(&resumed, restored)
                .equipment
                .installed(SLOT)
                .unwrap()
                .definition,
            CHOICE
        );
        assert_eq!(
            parts(&live, entity),
            initial_parts - i64::from(stage == "Removing"),
            "the one authored machinery part is consumed exactly once across continuation"
        );
    }
}

#[test]
fn accepted_source_support_spare_fetch_resumes_the_actual_team_route() {
    let (mut live, entity) = checkpoint(true, false, "Fetching");
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = prepared(true, false);
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    let items: BTreeSet<_> = fitting(&live, entity)
        .equipment
        .assemblies
        .keys()
        .cloned()
        .collect();
    let initial_parts = parts(&live, entity);
    let mut carried = false;
    let mut finished_ticks = 0;
    for step in 0..1500 {
        assert_continuation(&live, &resumed, step);
        carried |= fitting(&live, entity)
            .equipment
            .assemblies
            .values()
            .any(|item| {
                matches!(
                    item.location,
                    phoenix_model::fitting::EquipmentLocation::InWork { .. }
                )
            });
        live.update();
        resumed.update();
        if fitting(&live, entity).work.is_empty()
            && fitting(&live, entity)
                .equipment
                .installed(SLOT)
                .is_some_and(|item| item.definition == CHOICE)
        {
            finished_ticks += 1;
            if finished_ticks >= 20 && step >= 359 {
                break;
            }
        }
    }
    assert!(carried, "the team must actually collect the stored item");
    assert!(fitting(&live, entity).work.is_empty());
    assert_eq!(
        fitting(&resumed, restored)
            .equipment
            .installed(SLOT)
            .unwrap()
            .definition,
        CHOICE
    );
    assert_eq!(
        fitting(&live, entity)
            .equipment
            .assemblies
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        items
    );
    assert_eq!(parts(&live, entity), initial_parts - 1);
}

#[test]
fn a_previous_host_or_observer_cannot_request_equipment_after_the_seat_changes() {
    let mut app = begin(false);
    claim(&mut app, "engineering", "repair-observer");
    claim(&mut app, "tactical", "previous-tactical");
    send(&mut app, "previous-tactical", ClientMessage::ReleaseStation);
    steps(&mut app, 4);
    claim(&mut app, "tactical", "current-tactical");
    let target = id("phaser-fore");
    let payload = Control::RequestEquipment {
        slot: SLOT.into(),
        equipment: CHOICE.into(),
    };
    assert!(!authorised(
        &mut app,
        "previous-tactical",
        "tactical",
        &target,
        &payload
    ));
    assert!(authorised(
        &mut app,
        "current-tactical",
        "tactical",
        &target,
        &payload
    ));
    let entity = local(&mut app);
    let before = fitting(&app, entity).equipment.clone();
    for token in ["previous-tactical", "repair-observer"] {
        // Intentionally bypass the positive helper: this real input must be
        // refused by Admission, even though Repair receives fitting reports.
        send(
            &mut app,
            token,
            ClientMessage::ControlSystem {
                target: target.clone(),
                payload: payload.clone(),
            },
        );
        steps(&mut app, 4);
        assert_eq!(fitting(&app, entity).equipment, before);
        assert!(fitting(&app, entity).requested.is_empty());
        assert!(fitting(&app, entity).work.is_empty());
    }
}

#[test]
fn forged_duplicate_equipment_and_team_or_part_claims_refuse_without_world_writes() {
    let (mut app, entity) = checkpoint(false, false, "Removing");
    let uuid = app.world().get::<EntityUuid>(entity).unwrap().0.clone();
    let mut candidate = capture(app.world());
    candidate.tick += 50;
    candidate
        .entities
        .iter_mut()
        .find(|row| row.uuid == uuid)
        .unwrap()
        .red_alert = Some(true);
    let (mut positive, _) = prepared(false, false);
    assert!(
        restore(positive.world_mut(), &candidate).is_complete(),
        "positive global-write control"
    );
    for mode in 0..5 {
        let mut bad = candidate.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap();
        let state = row.fitting.as_mut().unwrap();
        match mode {
            0 => {
                let mut duplicate = state.equipment.installed(SLOT).unwrap().clone();
                duplicate.id = "forged-second-installed-item".into();
                state
                    .equipment
                    .assemblies
                    .insert(duplicate.id.clone(), duplicate);
            }
            1 => state.work.get_mut(SLOT).unwrap().job.0 += 200,
            2 => state.work.get_mut(SLOT).unwrap().reserved = false,
            3 => {
                state
                    .equipment
                    .desired
                    .insert(SLOT.into(), "unsupported-blueprint".into());
            }
            _ => state.work.get_mut(SLOT).unwrap().progress = f32::MAX,
        }
        let bad: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&bad).unwrap()).unwrap();
        let before = serde_json::to_value(capture(app.world())).unwrap();
        let digest = world_digest(app.world());
        let count = app.world().entities().len();
        let report = restore(app.world_mut(), &bad);
        assert!(!report.is_complete(), "mode {mode}");
        assert_eq!(report.entities_restored, 0);
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(report.despawned, 0);
        assert_eq!(app.world().entities().len(), count);
        assert_eq!(world_digest(app.world()), digest);
        assert_eq!(serde_json::to_value(capture(app.world())).unwrap(), before);
    }
}

fn mesh_step(hosts: &mut [App; 2]) {
    use project_phoenix::lockstep::{MeshInbox, MeshOutbox};
    let frames = hosts
        .each_mut()
        .map(|app| app.world_mut().resource_mut::<MeshOutbox>().drain());
    for index in 0..2 {
        for frame in &frames[1 - index] {
            hosts[index]
                .world_mut()
                .resource_mut::<MeshInbox>()
                .push(frame.clone());
        }
    }
    for app in hosts.iter_mut() {
        app.update();
    }
    assert_eq!(
        hosts[0].world().resource::<SimTick>().0,
        hosts[1].world().resource::<SimTick>().0
    );
    assert_eq!(
        world_digest(hosts[0].world()),
        world_digest(hosts[1].world())
    );
}

#[test]
fn two_actual_local_ship_peers_apply_human_requests_and_ordinary_ai_fits_identically() {
    use project_phoenix::{
        command_admission::HostSlot,
        lockstep::{authored_delay, join_fleet, FleetRoster, FleetShip, FleetSlotOf},
    };
    let mut hosts = [HostSlot(1), HostSlot(2)].map(|local_slot| {
        let mut app = unstarted("tests/fixtures/recovery-fleet-world.toml", CRUISER);
        let roster = FleetRoster::new(
            [HostSlot(1), HostSlot(2)]
                .into_iter()
                .map(|host| FleetShip {
                    host,
                    ship_path: Some(CRUISER.into()),
                    authored_slot_id: None,
                    crew: vec![],
                })
                .collect(),
            local_slot,
        );
        let delay = authored_delay(app.world());
        assert!(join_fleet(app.world_mut(), roster, delay));
        app.finish();
        app.cleanup();
        app
    });
    for _ in 0..10 {
        mesh_step(&mut hosts);
    }
    let initial = inventories(&mut hosts[0]);
    assert_eq!(initial, inventories(&mut hosts[1]));
    let mut journeys: Vec<_> = hosts
        .each_mut()
        .into_iter()
        .enumerate()
        .map(|(index, app)| claim_messages(app, "tactical", &format!("fleet-fitting-{index}")))
        .collect();
    for _ in 0..4 {
        for index in 0..2 {
            let token = format!("fleet-fitting-{index}");
            send(&mut hosts[index], &token, journeys[index].remove(0));
        }
        for _ in 0..6 {
            mesh_step(&mut hosts);
        }
    }
    let payload = Control::RequestEquipment {
        slot: SLOT.into(),
        equipment: CHOICE.into(),
    };
    for index in 0..2 {
        let token = format!("fleet-fitting-{index}");
        for _ in 0..120 {
            if authorised(
                &mut hosts[index],
                &token,
                "tactical",
                &id("phaser-fore"),
                &payload,
            ) {
                break;
            }
            mesh_step(&mut hosts);
        }
        assert!(authorised(
            &mut hosts[index],
            &token,
            "tactical",
            &id("phaser-fore"),
            &payload
        ));
        send(
            &mut hosts[index],
            &token,
            ClientMessage::ControlSystem {
                target: id("phaser-fore"),
                payload: payload.clone(),
            },
        );
    }
    let mut local_slots = BTreeMap::new();
    let mut paid = BTreeSet::new();
    let mut finished_ticks = 0;
    for step in 0..1800 {
        mesh_step(&mut hosts);
        for (index, app) in hosts.iter_mut().enumerate() {
            let entity = local(app);
            local_slots
                .entry(index)
                .or_insert(app.world().get::<FleetSlotOf>(entity).unwrap().0);
            let actors: Vec<_> = app
                .world_mut()
                .query::<(Entity, &EntityUuid, &ShipFitting)>()
                .iter(app.world())
                .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
                .collect();
            assert_eq!(actors.len(), 2);
            for (entity, uuid) in actors {
                if fitting(app, entity)
                    .work
                    .get(SLOT)
                    .is_some_and(|work| work.progress > 0.0)
                    && app
                        .world()
                        .get::<ShipPowerSystem>(entity)
                        .unwrap()
                        .0
                        .last_work_fraction(&id("repair"))
                        .is_some_and(|fraction| fraction > 0.0)
                {
                    paid.insert((index, uuid));
                }
            }
        }
        if hosts.iter().all(|app| {
            app.world()
                .try_query::<&ShipFitting>()
                .is_some_and(|mut query| {
                    query.iter(app.world()).all(|state| {
                        state.work.is_empty()
                            && state
                                .equipment
                                .installed(SLOT)
                                .is_some_and(|item| item.definition == CHOICE)
                    })
                })
        }) {
            finished_ticks += 1;
            if finished_ticks >= 20 && step >= 359 {
                break;
            }
        }
    }
    assert_ne!(local_slots[&0], local_slots[&1]);
    assert_eq!(
        paid.len(),
        4,
        "both actual Repair tools must perform paid fitting on both peers"
    );
    for app in &mut hosts {
        let actual = inventories(app);
        assert_eq!(actual.len(), initial.len());
        for (uuid, (initial_parts, initial_items)) in &initial {
            assert_eq!(
                actual[uuid].0,
                initial_parts - 1,
                "one real part per fleet fit"
            );
            assert_eq!(
                &actual[uuid].1, initial_items,
                "fleet fitting cannot mint or lose an assembly"
            );
        }
        let states: Vec<_> = app
            .world_mut()
            .query::<&ShipFitting>()
            .iter(app.world())
            .collect();
        assert_eq!(states.len(), 2);
        assert!(states.iter().all(|state| state.work.is_empty()
            && state
                .equipment
                .installed(SLOT)
                .is_some_and(|item| item.definition == CHOICE)));
    }
}
