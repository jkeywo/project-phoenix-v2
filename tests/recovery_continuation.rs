//! Actual finite recovery through ordinary crew commands and the whole host.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId},
    entities::spawner::EntityUuid,
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    lobby::{server::InboundMessage, Sessions},
    resources::recovery_server::{RecoverableSource, ShipRecovery},
    science::{broad::BroadTracks, contact_routing},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::power::ShipPowerSystem,
    ship_plugin::{
        ActiveStationRatings, HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources,
    },
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
use std::collections::{BTreeMap, BTreeSet};

fn unstarted(path: &str) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: path.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156_214),
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
fn stock(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock("scrap")
        .unwrap()
        .level
}
fn source(app: &mut App) -> (Entity, String) {
    let sources: Vec<_> = app
        .world_mut()
        .query::<(Entity, &EntityUuid, &RecoverableSource)>()
        .iter(app.world())
        .filter(|(_, _, manifest)| manifest.0.resources.contains_key("scrap"))
        .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
        .collect();
    assert_eq!(sources.len(), 1);
    sources[0].clone()
}
fn conservation(app: &mut App) -> (i64, BTreeMap<String, i64>) {
    let (source, _) = source(app);
    let actors: Vec<_> = app
        .world_mut()
        .query::<(Entity, &EntityUuid, &ShipRecovery)>()
        .iter(app.world())
        .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
        .collect();
    let recovered: BTreeMap<_, _> = actors
        .into_iter()
        .map(|(entity, uuid)| (uuid, stock(app, entity)))
        .collect();
    let supply = stock(app, source);
    assert_eq!(
        supply + recovered.values().sum::<i64>(),
        12,
        "finite goods have exactly one actual owner"
    );
    (supply, recovered)
}
fn commands(target: &str) -> Vec<ClientMessage> {
    vec![
        ClientMessage::Identify {
            token: "recovery-operator".into(),
            name: "Recovery operator".into(),
        },
        ClientMessage::SelectStation {
            station: "engineering".into(),
        },
        // A mid-mission claim is pending until the ordinary Ready message;
        // selecting a seat alone intentionally leaves Backfill in control.
        ClientMessage::SetReady { ready: true },
        ClientMessage::ControlSystem {
            target: SystemId("logistics".into()),
            payload: SystemControlPayload::SetStockTarget {
                store: "hold-scrap".into(),
                amount: 12,
            },
        },
        ClientMessage::ReleaseStation,
        ClientMessage::SelectStation {
            station: "science".into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::ControlSystem {
            target: SystemId("sensors".into()),
            payload: SystemControlPayload::SetScienceTarget {
                uuid: target.into(),
            },
        },
    ]
}
/// Read the actual joined Session and resolved machinery authority before
/// sending an input. This does not install crew or alter controller state.
fn assert_operator(app: &mut App, token: &str, msg: &ClientMessage) {
    let ClientMessage::ControlSystem { target, payload } = msg else {
        return;
    };
    let ship = local(app);
    let sessions = app.world().resource::<Sessions>();
    let player = sessions
        .0
        .players()
        .iter()
        .find(|p| p.token == token)
        .expect("Identify must register the actual input sender");
    assert!(
        player.connected && player.ready,
        "{token} must complete the mid-mission Ready handoff"
    );
    let expected_station = StationId(
        match target.0.as_str() {
            "logistics" => "engineering",
            "sensors" => "science",
            other => panic!("unexpected journey target {other}"),
        }
        .into(),
    );
    assert_eq!(player.station.as_ref(), Some(&expected_station));
    assert_eq!(
        sessions.0.holder_for_station(&expected_station),
        Some(token)
    );
    let sources = app.world().get::<ShipSystemControlSources>(ship).unwrap();
    assert!(
        sources.0.policy_for(target).accept_human_input,
        "target={target:?}, source={:?}, ratings={:?}, phase={:?}, tick={}",
        sources.0.source_for(target),
        app.world().get::<ActiveStationRatings>(ship).unwrap().0,
        app.world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get(),
        app.world().resource::<SimTick>().0
    );
    assert!(!sources.0.policy_for(target).operate_ai);
    assert!(
        project_phoenix::command_admission::policy::is_command_authorized(
            token,
            target,
            payload,
            sources,
            sessions,
            &app.world().get::<ShipConfigComponent>(ship).unwrap().0,
            app.world().get::<HumanSeekingHosts>(ship),
        ),
        "the actual System → Console → host Station must authorise this sender"
    );
}
/// Read only this actual Science Console's held selectable picture. The
/// private association is a test witness that the earned contact is the intended
/// finite source; only the published handle enters ordinary command admission.
fn science_handle(app: &App, entity: Entity, expected_source: &str) -> Option<String> {
    let config = &app.world().get::<ShipConfigComponent>(entity)?.0;
    let boards = app.world().get::<ShipSystemBlackboards>(entity)?;
    let tracks = app.world().get::<BroadTracks>(entity)?;
    let owner = SystemId("sensors".into());
    let permitted = config.observations_for_system(&owner, &boards.0);
    let board = permitted.get(&SystemId("sensor-radar".into()))?;
    contact_routing::observations(board)?
        .iter()
        .filter(|record| record.selectable && !record.predicted)
        .find_map(|record| {
            let actual =
                contact_routing::resolve(config, boards, Some(tracks), &owner, &record.contact_id)?;
            (actual == expected_source).then(|| record.contact_id.clone())
        })
}
fn with_earned_science_handle(
    app: &App,
    entity: Entity,
    mut message: ClientMessage,
    source: &str,
) -> ClientMessage {
    if let ClientMessage::ControlSystem {
        target,
        payload: SystemControlPayload::SetScienceTarget { uuid },
    } = &mut message
    {
        assert_eq!(target, &SystemId("sensors".into()));
        *uuid=science_handle(app,entity,source).expect("normal paid Broad sampling must have published a current selectable own Science contact");
        assert_ne!(
            uuid.as_str(),
            source,
            "private source UUID is not an admitted radar handle"
        );
    }
    message
}
fn assert_science_selection(app: &App, entity: Entity, source: &str) {
    let selection = &app
        .world()
        .get::<project_phoenix::ship::sensors::SensorRadarSelection>(entity)
        .unwrap()
        .0;
    assert_eq!(
        selection.as_deref(),
        Some(source),
        "actual admitted Science handle must resolve to the intended finite source"
    );
    let config = &app.world().get::<ShipConfigComponent>(entity).unwrap().0;
    let boards = app.world().get::<ShipSystemBlackboards>(entity).unwrap();
    let tracks = app.world().get::<BroadTracks>(entity).unwrap();
    let owner = SystemId("sensors".into());
    let permitted = config.observations_for_system(&owner, &boards.0);
    let project_phoenix::core::messages::SystemBlackboard::SensorRadar(board) =
        permitted.get(&SystemId("sensor-radar".into())).unwrap()
    else {
        panic!("actual typed Science radar publication");
    };
    let selected = board
        .selected_target
        .as_ref()
        .expect("normal publication carries the admitted selection");
    assert_eq!(
        contact_routing::resolve(config, boards, Some(tracks), &owner, selected).as_deref(),
        Some(source)
    );
}
fn started() -> (App, Entity) {
    let mut app = unstarted("assets/worlds/probe_recovery.toml");
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get(),
        &project_phoenix::core::messages::GamePhase::InProgress,
        "normal actual host must be running before source work"
    );
    let entity = local(&mut app);
    let (_, target) = source(&mut app);
    for msg in commands(&target) {
        let msg = with_earned_science_handle(&app, entity, msg, &target);
        assert_operator(&mut app, "recovery-operator", &msg);
        send(&mut app, "recovery-operator", msg);
        for _ in 0..4 {
            app.update();
        }
    }
    assert_science_selection(&app, entity, &target);
    (app, entity)
}
fn mid_work() -> (App, Entity) {
    let (mut app, entity) = started();
    for _ in 0..600 {
        app.update();
        let (_, recovered) = conservation(&mut app);
        let state = &app.world().get::<ShipRecovery>(entity).unwrap().0;
        if recovered.values().sum::<i64>() > 0 && state.fraction > 0.1 && state.fraction < 0.9 {
            assert!(state.enabled);
            assert_eq!(state.destination_store.as_deref(), Some("hold-scrap"));
            assert!(state.last_outcome.as_ref().unwrap().settled);
            let power = &app.world().get::<ShipPowerSystem>(entity).unwrap().0;
            assert!(power
                .last_work_fraction(&state.owner)
                .is_some_and(|fraction| fraction > 0.0));
            return (app, entity);
        }
    }
    panic!("ordinary recovery tree must acquire a permitted Science selection and perform paid fractional work after an actual finite transfer");
}
fn first_json_difference(
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
            let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                if let Some(diff) = first_json_difference(
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
                if let Some(diff) = first_json_difference(a, b, &format!("{path}[{index}]")) {
                    return Some(diff);
                }
            }
        }
        _ => {}
    }
    Some(format!("{path}: live={left}, resumed={right}"))
}
#[test]
fn actual_recovery_json_restore_preserves_paid_fraction_and_finite_goods() {
    let (mut live, entity) = mid_work();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = started();
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..360 {
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "step {step}"
        );
        let live_snapshot = serde_json::to_value(capture(live.world())).unwrap();
        let resumed_snapshot = serde_json::to_value(capture(resumed.world())).unwrap();
        assert!(
            live_snapshot == resumed_snapshot,
            "step {step}: {}",
            first_json_difference(&live_snapshot, &resumed_snapshot, "$").unwrap_or_default()
        );
        assert_eq!(conservation(&mut live), conservation(&mut resumed));
        let tick = live.world().resource::<SimTick>().0;
        live.update();
        resumed.update();
        assert_eq!(live.world().resource::<SimTick>().0, tick + 1);
        assert_eq!(resumed.world().resource::<SimTick>().0, tick + 1);
    }
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "final360 digest"
    );
    assert_eq!(
        capture(live.world()),
        capture(resumed.world()),
        "final360 full capture"
    );
    assert_eq!(conservation(&mut live), conservation(&mut resumed));
    assert_eq!(stock(&live, entity), 12);
    assert_eq!(stock(&resumed, restored), 12);
    assert_eq!(conservation(&mut live).0, 0);
}
#[test]
fn invalid_recovery_owner_receipt_and_destination_refuse_without_live_writes() {
    let (mut app, entity) = mid_work();
    let uuid = app.world().get::<EntityUuid>(entity).unwrap().0.clone();
    let mut valid = capture(app.world());
    valid.tick += 250;
    let row = valid
        .entities
        .iter_mut()
        .find(|row| row.uuid == uuid)
        .unwrap();
    row.red_alert = Some(!row.red_alert.unwrap_or(false));
    let (mut positive, _) = started();
    assert!(
        restore(positive.world_mut(), &valid).is_complete(),
        "positive control must admit the changed global fields"
    );
    for mode in 0..7 {
        let mut bad = valid.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap();
        if mode == 0 {
            row.recovery = None;
        } else {
            let state = row.recovery.as_mut().unwrap();
            match mode {
                1 => state.owner = SystemId("logistics".into()),
                2 => state.last_outcome = None,
                3 => state.last_outcome.as_mut().unwrap().tick -= 1,
                4 => state.destination_store = Some("absent-store".into()),
                5 => state.destination_store = Some("repair-machinery".into()),
                _ => state.destination_store = Some("hold-machinery-parts".into()),
            }
        }
        let bad: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&bad).unwrap()).unwrap();
        let before = serde_json::to_value(capture(app.world())).unwrap();
        let digest = world_digest(app.world());
        let entities = app.world().entities().len();
        let report = restore(app.world_mut(), &bad);
        assert!(!report.is_complete(), "mode {mode}");
        assert!(
            format!("{:?}", report.gaps).contains(if mode == 4 {
                "managed store"
            } else {
                "recovery"
            }),
            "{:?}",
            report.gaps
        );
        assert_eq!(report.entities_restored, 0);
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(report.despawned, 0);
        assert_eq!(app.world().entities().len(), entities);
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
    for host in hosts.iter_mut() {
        host.update();
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
fn actual_recovery_competes_for_one_finite_source_on_both_local_ship_peers() {
    use project_phoenix::{
        command_admission::HostSlot,
        lockstep::{authored_delay, join_fleet, FleetRoster, FleetShip, FleetSlotOf},
    };
    let mut hosts = [HostSlot(1), HostSlot(2)].map(|local_slot| {
        let mut app = unstarted("tests/fixtures/recovery-fleet-world.toml");
        let roster = FleetRoster::new(
            [HostSlot(1), HostSlot(2)]
                .into_iter()
                .map(|host| FleetShip {
                    host,
                    ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
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
    for host in &hosts {
        assert_eq!(
            host.world()
                .resource::<State<project_phoenix::core::messages::GamePhase>>()
                .get(),
            &project_phoenix::core::messages::GamePhase::InProgress,
            "both actual fleet hosts must be running"
        );
    }
    let targets = hosts.each_mut().map(|app| source(app).1);
    assert_eq!(targets[0], targets[1]);
    let journey_len = commands(&targets[0]).len();
    let mut journeys = [
        commands(&targets[0]).into_iter(),
        commands(&targets[1]).into_iter(),
    ];
    for _ in 0..journey_len {
        for index in 0..2 {
            let mut msg = journeys[index].next().unwrap();
            let token = format!("recovery-operator-{index}");
            if let ClientMessage::Identify {
                token: identify_token,
                ..
            } = &mut msg
            {
                *identify_token = token.clone();
            }
            if let ClientMessage::ControlSystem { target, .. } = &msg {
                for _ in 0..120 {
                    let ship = local(&mut hosts[index]);
                    if hosts[index]
                        .world()
                        .get::<ShipSystemControlSources>(ship)
                        .unwrap()
                        .0
                        .policy_for(target)
                        .accept_human_input
                    {
                        break;
                    }
                    mesh_step(&mut hosts);
                }
            }
            let own = local(&mut hosts[index]);
            msg = with_earned_science_handle(&hosts[index], own, msg, &targets[index]);
            assert_operator(&mut hosts[index], &token, &msg);
            send(&mut hosts[index], &token, msg);
        }
        for _ in 0..6 {
            mesh_step(&mut hosts);
        }
    }
    for index in 0..2 {
        let own = local(&mut hosts[index]);
        assert_science_selection(&hosts[index], own, &targets[index]);
    }
    let mut paid = BTreeSet::new();
    let mut locals = BTreeMap::new();
    for _ in 0..720 {
        mesh_step(&mut hosts);
        for (index, app) in hosts.iter_mut().enumerate() {
            conservation(app);
            let entity = local(app);
            locals
                .entry(index)
                .or_insert(app.world().get::<FleetSlotOf>(entity).unwrap().0);
            let actors: Vec<_> = app
                .world_mut()
                .query::<(Entity, &EntityUuid, &ShipRecovery)>()
                .iter(app.world())
                .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
                .collect();
            assert_eq!(actors.len(), 2);
            for (entity, uuid) in actors {
                let state = &app.world().get::<ShipRecovery>(entity).unwrap().0;
                if state.fraction > 0.1
                    && state.fraction < 0.9
                    && app
                        .world()
                        .get::<ShipPowerSystem>(entity)
                        .unwrap()
                        .0
                        .last_work_fraction(&state.owner)
                        .is_some_and(|fraction| fraction > 0.0)
                {
                    paid.insert((index, uuid));
                }
            }
        }
    }
    assert_ne!(locals[&0], locals[&1]);
    assert_eq!(
        paid.len(),
        4,
        "both real tools must perform positive paid work on both peers"
    );
    for app in &mut hosts {
        let (supply, recovered) = conservation(app);
        assert_eq!(supply, 0);
        assert_eq!(recovered.len(), 2);
        assert!(
            recovered.values().all(|amount| *amount > 0),
            "competition must deliver to both ships"
        );
    }
}
