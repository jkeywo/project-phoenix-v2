//! Regression coverage through real producers, recipient routing, PaneBus and
//! the ordinary codec. Retained maps apply the same sparse-field rules as the
//! client reducer; they are not a second implementation of queue coalescing.

use std::collections::BTreeMap;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;

use super::*;
use crate::core::messages::{
    DeliveryClass, EntityStateSnapshot, HelmBlackboard, ModifierSlot, ModifierSource,
    RepairBlackboard, ServerMessage, SimSnapshot, StationHealthSnapshot, StationId,
    SystemBlackboard, SystemId,
};
use crate::entities::spawner::EntityUuid;
use crate::lobby::{Sessions, Target};
use crate::server_app::{
    broadcast_blackboard_updates, build_sim_state_entity_states, LastBroadcastBlackboards,
    LastBroadcastEntityHealth, LastBroadcastEntityPositions, LocalShip, ShipSystemBlackboards,
    SimOutbox,
};

fn open(bus: &PaneBus, n: u8) -> PaneId {
    let id = bus.open(
        PaneIdentity::adopt(
            format!("3f1a6c2e-0a11-4b3c-9d55-00000000000{n}"),
            format!("crew-{n}"),
        )
        .unwrap(),
    );
    identify_test_pane(bus, id);
    bus.mark_live(id);
    id
}

fn send(bus: &PaneBus, target: Target, message: &ServerMessage, delivery: DeliveryClass) {
    bus.transport().dispatch(TransportDispatch {
        target: &target,
        msg: message,
        delivery,
    });
}

fn decode(bus: &PaneBus, id: PaneId) -> Vec<ServerMessage> {
    bus.take_outbound(id)
        .iter()
        .map(|dispatch| JsonCodec.decode_server(&dispatch.json).unwrap())
        .collect()
}

fn snapshot(entity_states: Vec<EntityStateSnapshot>) -> SimSnapshot {
    SimSnapshot {
        entity_states,
        station_hosts: Vec::new(),
        station_health: Vec::new(),
        station_importance: Vec::new(),
        control_sources: BTreeMap::new(),
        station_puppets: Vec::new(),
    }
}

#[test]
fn producer_common_and_recipient_repair_deltas_both_reach_each_pane() {
    let bus = PaneBus::default();
    let first = open(&bus, 1);
    let second = open(&bus, 2);
    let mut sessions = Sessions(crate::lobby::session::SessionManager::new());
    for id in [first, second] {
        sessions
            .0
            .register(bus.token_of(id).unwrap(), "Crew".into())
            .unwrap();
    }
    let mut world = World::new();
    world.insert_resource(sessions);
    world.init_resource::<LastBroadcastBlackboards>();
    world.init_resource::<SimOutbox>();
    let helm = SystemBlackboard::Helm(HelmBlackboard {
        x: 42.0,
        ..Default::default()
    });
    let mut blackboards = ShipSystemBlackboards::default();
    blackboards.0.insert(SystemId("helm".into()), helm.clone());
    blackboards.0.insert(
        SystemId("repair".into()),
        SystemBlackboard::Repair(RepairBlackboard {
            aggregate_hull_fraction: Some(0.5),
            ..Default::default()
        }),
    );
    world.spawn((LocalShip, blackboards));

    world.run_system_once(broadcast_blackboard_updates).unwrap();
    let entries = world.resource_mut::<SimOutbox>().drain();
    assert_eq!(
        entries.len(),
        3,
        "one common batch plus each recipient's repair projection"
    );
    for entry in entries {
        send(&bus, entry.target, &entry.message, entry.delivery);
    }
    // The producer has already marked the keys sent, so an idle next tick does
    // not repair an update that the pane queue dropped.
    world.run_system_once(broadcast_blackboard_updates).unwrap();
    assert!(world.resource_mut::<SimOutbox>().drain().is_empty());

    for id in [first, second] {
        let messages = decode(&bus, id);
        assert_eq!(messages.len(), 1);
        let mut retained = BTreeMap::new();
        for message in messages {
            let ServerMessage::BlackboardUpdate { updates } = message else {
                panic!("blackboard")
            };
            retained.extend(updates);
        }
        assert_eq!(retained.get(&SystemId("helm".into())), Some(&helm));
        assert!(matches!(
            retained.get(&SystemId("repair".into())),
            Some(SystemBlackboard::Repair(_))
        ));
    }
}

#[test]
fn producer_final_contact_movement_survives_a_later_unrelated_entity_delta() {
    let bus = PaneBus::default();
    let id = open(&bus, 1);
    let mut world = World::new();
    world.init_resource::<LastBroadcastEntityPositions>();
    world.init_resource::<LastBroadcastEntityHealth>();
    let contact = world
        .spawn((EntityUuid("contact".into()), Transform::default()))
        .id();
    let other = world
        .spawn((EntityUuid("other".into()), Transform::default()))
        .id();
    let mut retained: BTreeMap<_, _> = build_sim_state_entity_states(&mut world)
        .into_iter()
        .map(|state| (state.uuid, state.position.unwrap()))
        .collect();

    world.get_mut::<Transform>(contact).unwrap().translation.x = 25.0;
    let mut first = snapshot(build_sim_state_entity_states(&mut world));
    first.station_health.push(StationHealthSnapshot {
        station: StationId("helm".into()),
        health: Some(0.5),
    });
    send(
        &bus,
        Target::All,
        &ServerMessage::SimState { snapshot: first },
        DeliveryClass::Snapshot,
    );
    world.get_mut::<Transform>(other).unwrap().translation.z = 50.0;
    let last = snapshot(build_sim_state_entity_states(&mut world));
    assert!(!last
        .entity_states
        .iter()
        .any(|state| state.uuid == "contact"));
    send(
        &bus,
        Target::All,
        &ServerMessage::SimState { snapshot: last },
        DeliveryClass::Snapshot,
    );

    let messages = decode(&bus, id);
    assert_eq!(messages.len(), 1);
    let ServerMessage::SimState { snapshot } = &messages[0] else {
        panic!("simulation")
    };
    assert!(
        snapshot.station_health.is_empty(),
        "latest complete projections clear older rows"
    );
    for state in &snapshot.entity_states {
        if let Some(position) = state.position {
            retained.insert(state.uuid.clone(), position);
        }
    }
    assert_eq!(retained["contact"], [25.0, 0.0, 0.0]);
    assert_eq!(retained["other"], [0.0, 0.0, 50.0]);
}

fn helm_update(x: f32) -> ServerMessage {
    ServerMessage::BlackboardUpdate {
        updates: vec![(
            SystemId("helm".into()),
            SystemBlackboard::Helm(HelmBlackboard {
                x,
                ..Default::default()
            }),
        )],
    }
}

#[test]
fn delta_requeue_keeps_recipient_isolation_and_replaces_a_whole_blackboard() {
    let bus = PaneBus::default();
    let first = open(&bus, 1);
    let second = open(&bus, 2);
    let target = Target::Token(bus.token_of(first).unwrap());
    let old = ServerMessage::BlackboardUpdate {
        updates: vec![(
            SystemId("repair".into()),
            SystemBlackboard::Repair(RepairBlackboard {
                damageable_systems: vec![SystemId("private-detail".into())],
                ..Default::default()
            }),
        )],
    };
    send(&bus, target.clone(), &old, DeliveryClass::Snapshot);
    let retry = bus.take_outbound(first);
    send(
        &bus,
        target.clone(),
        &helm_update(8.0),
        DeliveryClass::Snapshot,
    );
    let withheld = ServerMessage::BlackboardUpdate {
        updates: vec![(
            SystemId("repair".into()),
            SystemBlackboard::Repair(RepairBlackboard::default()),
        )],
    };
    send(&bus, target, &withheld, DeliveryClass::Snapshot);
    bus.requeue_front(first, retry);
    assert!(decode(&bus, second).is_empty());
    let messages = decode(&bus, first);
    assert_eq!(messages.len(), 1);
    let ServerMessage::BlackboardUpdate { updates } = &messages[0] else {
        panic!("blackboard")
    };
    assert_eq!(updates.len(), 2);
    let repair = updates.iter().find(|(id, _)| id.0 == "repair").unwrap();
    assert_eq!(
        repair.1,
        SystemBlackboard::Repair(RepairBlackboard::default())
    );
}

#[test]
fn lifecycle_and_modifier_events_are_ordered_barriers_for_snapshot_deltas() {
    let bus = PaneBus::default();
    let id = open(&bus, 1);
    let change_station = ServerMessage::StationAssigned {
        token: bus.token_of(id).unwrap(),
        station: None,
        station_id: None,
    };
    let added = ServerMessage::ModifierAdded {
        source: ModifierSource::ImpulseDrive,
        slot: ModifierSlot::RadarRange,
        bonus: 0.5,
    };
    let removed = ServerMessage::ModifierRemoved {
        source: ModifierSource::ImpulseDrive,
        slot: ModifierSlot::RadarRange,
    };
    let messages = [
        helm_update(1.0),
        change_station,
        helm_update(2.0),
        added.clone(),
        removed,
        added,
    ];
    for (index, message) in messages.iter().enumerate() {
        send(
            &bus,
            Target::All,
            message,
            if index == 1 {
                DeliveryClass::Reliable
            } else {
                DeliveryClass::Snapshot
            },
        );
    }
    assert_eq!(decode(&bus, id), messages);
}

#[test]
fn a_full_queue_faults_instead_of_shedding_a_final_delta() {
    let bus = PaneBus::with_capacity(1);
    let id = open(&bus, 1);
    send(
        &bus,
        Target::All,
        &helm_update(1.0),
        DeliveryClass::Snapshot,
    );
    send(
        &bus,
        Target::All,
        &ServerMessage::GameStarted,
        DeliveryClass::Reliable,
    );
    assert_eq!(bus.take_faulted(), vec![(id, PaneFault::ReliableOverflow)]);
    assert_eq!(decode(&bus, id), vec![helm_update(1.0)]);
}

#[test]
fn a_change_only_hull_withdrawal_is_never_silently_shed_at_capacity() {
    // The host need not repeat either complete projection without another
    // change. A queue cannot promise that a lost withdrawal will arrive later.
    let withdrawal = ServerMessage::SystemHullUpdate {
        entries: Vec::new(),
        aggregate_fraction: Some(0.8),
        destroyed_fraction: Some(0.0),
    };
    let unrelated = ServerMessage::ShieldStatus {
        facings: Vec::new(),
        frequency: 0.2,
    };
    for messages in [[&withdrawal, &unrelated], [&unrelated, &withdrawal]] {
        let bus = PaneBus::with_capacity(1);
        let id = open(&bus, 1);
        for message in messages {
            send(&bus, Target::All, message, DeliveryClass::Snapshot);
        }
        assert_eq!(bus.take_faulted(), vec![(id, PaneFault::ReliableOverflow)]);
        assert_eq!(decode(&bus, id), vec![messages[0].clone()]);
    }
}

#[test]
fn sparse_rows_preserve_optional_health_fields_and_apply_latest_complete_facts() {
    let bus = PaneBus::default();
    let id = open(&bus, 1);
    let old = EntityStateSnapshot {
        uuid: "contact".into(),
        position: Some([1.0, 0.0, 2.0]),
        yaw: Some(0.5),
        hull_fraction: Some(0.9),
        shield_fraction: Some(0.8),
        flags: Vec::new(),
        shields: Some(Vec::new()),
        shield_freq: Some(0.3),
        warp_out_remaining_secs: Some(5.0),
    };
    let new = EntityStateSnapshot {
        uuid: "contact".into(),
        position: None,
        yaw: None,
        hull_fraction: Some(0.0),
        shield_fraction: None,
        flags: Vec::new(),
        shields: None,
        shield_freq: None,
        warp_out_remaining_secs: None,
    };
    for row in [old.clone(), new] {
        send(
            &bus,
            Target::All,
            &ServerMessage::SimState {
                snapshot: snapshot(vec![row]),
            },
            DeliveryClass::Snapshot,
        );
    }
    let messages = decode(&bus, id);
    let ServerMessage::SimState { snapshot } = &messages[0] else {
        panic!("simulation")
    };
    assert_eq!(messages.len(), 1);
    let row = &snapshot.entity_states[0];
    assert_eq!(row.position, old.position);
    assert_eq!(row.yaw, old.yaw);
    assert_eq!(row.hull_fraction, Some(0.0));
    assert_eq!(row.shield_fraction, old.shield_fraction);
    assert_eq!(row.shields, old.shields);
    assert_eq!(row.shield_freq, old.shield_freq);
    assert_eq!(row.warp_out_remaining_secs, None);
}

#[test]
fn complete_snapshot_still_replaces_an_older_complete_snapshot() {
    let bus = PaneBus::default();
    let id = open(&bus, 1);
    let state = |frequency| ServerMessage::ShieldStatus {
        facings: Vec::new(),
        frequency,
    };
    send(&bus, Target::All, &state(0.1), DeliveryClass::Snapshot);
    send(&bus, Target::All, &state(0.9), DeliveryClass::Snapshot);
    assert_eq!(decode(&bus, id), vec![state(0.9)]);
}

#[test]
fn a_reopened_pane_never_merges_with_its_previous_incarnations_delta() {
    let bus = PaneBus::default();
    let old = open(&bus, 1);
    send(
        &bus,
        Target::All,
        &helm_update(1.0),
        DeliveryClass::Snapshot,
    );
    bus.close(old);
    let new = open(&bus, 1);
    send(
        &bus,
        Target::All,
        &helm_update(2.0),
        DeliveryClass::Snapshot,
    );
    assert_ne!(old, new);
    assert!(decode(&bus, old).is_empty());
    assert_eq!(decode(&bus, new), vec![helm_update(2.0)]);
}
