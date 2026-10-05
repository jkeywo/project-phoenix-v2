use super::*;
use crate::core::messages::ObjectiveSnapshot;
use crate::core::messages::ObjectiveSource;
use bevy::ecs::system::RunSystemOnce;

fn snapshot(id: &str) -> ObjectiveSnapshot {
    ObjectiveSnapshot {
        progress: None,
        unassigned: false,
        id: id.into(),
        text: format!("objective.{id}"),
        text_params: Default::default(),
        mandatory: false,
        status: ObjectiveStatus::Active,
        targets: Vec::new(),
        source: ObjectiveSource::Mission,
    }
}

fn key(instance: &str) -> ObjectiveInstanceKey {
    ObjectiveInstanceKey {
        objective_id: "survive".into(),
        instance_id: instance.into(),
    }
}
fn ship(id: &str, slot: &str, faction: &str) -> PlayerShipMembership {
    PlayerShipMembership {
        ship_id: id.into(),
        slot_id: slot.into(),
        faction: faction.into(),
    }
}
fn spec(instance: &str, recipients: Vec<RecipientSelector>) -> ObjectiveInstanceSpec {
    ObjectiveInstanceSpec {
        key: key(instance),
        recipients,
    }
}

#[test]
fn multi_ship_explicit_slot_beats_faction_and_instances_progress_independently() {
    let fleet = [ship("a", "lead", "alliance"), ship("b", "wing", "alliance")];
    let mut manager = ObjectiveInstanceManager::default();
    manager
        .activate(
            spec("fleet", vec![RecipientSelector::Faction("alliance".into())]),
            &fleet,
        )
        .unwrap();
    manager
        .activate(
            spec("lead", vec![RecipientSelector::ShipSlot("lead".into())]),
            &fleet,
        )
        .unwrap();
    manager.set_progress(&key("fleet"), 0.25);
    manager.set_progress(&key("lead"), 0.75);
    assert_eq!(manager.view_for_ship("a")[0].key.instance_id, "lead");
    assert_eq!(manager.view_for_ship("a")[0].progress, 0.75);
    assert_eq!(manager.view_for_ship("b")[0].key.instance_id, "fleet");
}

#[test]
fn multi_ship_completion_credit_is_fixed_and_restore_replays_nothing() {
    let fleet = [ship("a", "lead", "alliance"), ship("b", "wing", "alliance")];
    let mut manager = ObjectiveInstanceManager::default();
    manager
        .activate(
            spec("fleet", vec![RecipientSelector::Faction("alliance".into())]),
            &fleet,
        )
        .unwrap();
    manager.drain_transitions();
    assert!(manager.complete(&key("fleet"), &fleet).unwrap());
    let transitions = manager.drain_transitions();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].completion_members, ["a", "b"]);
    assert!(!manager.complete(&key("fleet"), &fleet).unwrap());
    let json = serde_json::to_string(&manager).unwrap();
    let mut restored: ObjectiveInstanceManager = serde_json::from_str(&json).unwrap();
    assert!(restored.drain_transitions().is_empty());
    let changed = [
        ship("a", "lead", "dynasty"),
        ship("b", "wing", "alliance"),
        ship("c", "reserve", "alliance"),
    ];
    restored.reconcile(&changed).unwrap();
    assert_eq!(restored.records()[0].completion_members, ["a", "b"]);
    assert!(!restored.complete(&key("fleet"), &changed).unwrap());
    assert!(!restored.view_for_ship("a")[0].assigned);
    assert_eq!(
        restored.view_for_ship("c")[0].status,
        ObjectiveStatus::Completed
    );
}

#[test]
fn multi_ship_equal_specificity_conflict_is_atomic() {
    let fleet = [ship("a", "lead", "alliance")];
    let mut manager = ObjectiveInstanceManager::default();
    manager
        .activate(
            spec("one", vec![RecipientSelector::Faction("alliance".into())]),
            &fleet,
        )
        .unwrap();
    let before = manager.clone();
    let conflict = manager
        .activate(
            spec("two", vec![RecipientSelector::Faction("alliance".into())]),
            &fleet,
        )
        .unwrap_err();
    assert_eq!(conflict.instance_ids, ["one", "two"]);
    assert_eq!(manager, before);
}

#[test]
fn live_faction_conflict_restores_the_ship_and_keeps_frozen_progress() {
    let alpha = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
    let beta = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();
    let mut registry = crate::ai::faction::FactionRegistry::new();
    for (uuid, name) in [(alpha, "alpha"), (beta, "beta")] {
        registry.insert(crate::ai::faction::FactionConfig {
            uuid,
            name: name.into(),
            display_name: None,
            enemies: Vec::new(),
            compliance: None,
        });
    }
    let original = [ship("ship-a", "lead", "alpha")];
    let mut manager = ObjectiveInstanceManager::default();
    manager
        .activate(
            spec("alpha", vec![RecipientSelector::Faction("alpha".into())]),
            &original,
        )
        .unwrap();
    manager
        .activate(
            spec("beta-one", vec![RecipientSelector::Faction("beta".into())]),
            &original,
        )
        .unwrap();
    manager
        .activate(
            spec("beta-two", vec![RecipientSelector::Faction("beta".into())]),
            &original,
        )
        .unwrap();
    manager.set_progress(&key("alpha"), 0.6);
    let restored: ObjectiveInstanceManager =
        serde_json::from_str(&serde_json::to_string(&manager).unwrap()).unwrap();
    // Save data intentionally excludes transient story edges and dirty.
    let before = restored.clone();
    let peer_manager = restored.clone();
    let peer_registry = registry.clone();

    let mut world = World::new();
    world.insert_resource(crate::entities::config_cache::FactionRegistryResource(
        registry,
    ));
    world.insert_resource(crate::world::server::ObjectiveInstanceManagerRes(restored));
    let entity = world
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("ship-a".into()),
            crate::ship_slots::AuthoredShipSlotId("lead".into()),
            crate::entities::spawner::FactionComponent(beta),
        ))
        .id();

    world.run_system_once(reconcile_memberships).unwrap();
    let diagnostics = world.resource::<crate::recipients::RecipientDiagnostics>();
    assert_eq!(diagnostics.0.len(), 1);
    for label in ["survive", "ship-a", "beta-one", "beta-two"] {
        assert!(diagnostics.0[0].message.contains(label));
    }
    assert_eq!(
        world
            .get::<crate::entities::spawner::FactionComponent>(entity)
            .unwrap()
            .0,
        alpha
    );
    assert_eq!(
        world
            .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
            .0,
        before
    );
    assert_eq!(
        world
            .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
            .0
            .view_for_ship("ship-a")[0]
            .progress,
        0.6
    );

    // A second deterministic host applies the same candidate against a
    // restored manager. Both reject the faction edit and retain identical
    // projections, including the previous accepted membership.
    let mut peer = World::new();
    peer.insert_resource(crate::entities::config_cache::FactionRegistryResource(
        peer_registry,
    ));
    peer.insert_resource(crate::world::server::ObjectiveInstanceManagerRes(
        peer_manager,
    ));
    let peer_entity = peer
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("ship-a".into()),
            crate::ship_slots::AuthoredShipSlotId("lead".into()),
            crate::entities::spawner::FactionComponent(beta),
        ))
        .id();
    peer.run_system_once(reconcile_memberships).unwrap();
    assert_eq!(
        peer.get::<crate::entities::spawner::FactionComponent>(peer_entity)
            .unwrap()
            .0,
        alpha
    );
    assert_eq!(
        peer.resource::<crate::world::server::ObjectiveInstanceManagerRes>()
            .0,
        world
            .resource::<crate::world::server::ObjectiveInstanceManagerRes>()
            .0,
    );
}

#[test]
fn switching_instances_replaces_displayed_progress_without_leaking_old_updates() {
    let alpha = [ship("ship-a", "lead", "alpha")];
    let beta = [ship("ship-a", "lead", "beta")];
    let outside = [ship("ship-a", "lead", "outside")];
    let mut manager = ObjectiveInstanceManager::default();
    manager
        .activate(
            spec("alpha", vec![RecipientSelector::Faction("alpha".into())]),
            &alpha,
        )
        .unwrap();
    manager
        .activate(
            spec("beta", vec![RecipientSelector::Faction("beta".into())]),
            &alpha,
        )
        .unwrap();
    manager.set_progress(&key("alpha"), 0.25);
    manager.set_progress(&key("beta"), 0.75);

    manager.reconcile(&beta).unwrap();
    assert_eq!(manager.view_for_ship("ship-a")[0].key, key("beta"));
    assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);
    manager.set_progress(&key("alpha"), 0.9);
    assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);

    manager.reconcile(&outside).unwrap();
    assert!(!manager.view_for_ship("ship-a")[0].assigned);
    assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);
    manager.set_progress(&key("beta"), 0.8);
    assert_eq!(manager.view_for_ship("ship-a")[0].progress, 0.75);
    let mut restored: ObjectiveInstanceManager =
        serde_json::from_str(&serde_json::to_string(&manager).unwrap()).unwrap();
    for peer in [&mut manager, &mut restored] {
        let frozen = peer.project_snapshots_for_ship("ship-a", vec![snapshot("survive")]);
        assert_eq!(frozen[0].progress, Some(0.75));
        assert!(frozen[0].unassigned);
        peer.complete(&key("beta"), &outside).unwrap();
        let frozen = peer.project_snapshots_for_ship("ship-a", vec![snapshot("survive")]);
        assert_eq!(frozen[0].status, ObjectiveStatus::Active);
        assert_eq!(frozen[0].progress, Some(0.75));
        peer.reconcile(&beta).unwrap();
        let joined = peer.project_snapshots_for_ship("ship-a", vec![snapshot("survive")]);
        assert_eq!(joined[0].status, ObjectiveStatus::Completed);
        assert_eq!(joined[0].progress, Some(0.8));
        assert!(!joined[0].unassigned);
        assert!(peer
            .records()
            .iter()
            .find(|row| row.spec.key == key("beta"))
            .unwrap()
            .completion_members
            .is_empty());
    }
    assert_eq!(
        serde_json::to_value(manager).unwrap(),
        serde_json::to_value(restored).unwrap()
    );
}

#[test]
fn multi_ship_activation_refuses_empty_and_dangling_selectors() {
    let fleet = [ship("a", "lead", "alliance")];
    let slots = BTreeSet::from(["lead".to_string()]);
    let factions = BTreeSet::from(["alliance".to_string()]);
    let mut manager = ObjectiveInstanceManager::default();
    assert_eq!(
        manager.activate_validated(spec("empty", vec![]), &fleet, &slots, &factions),
        Err(ActivationRefusal::EmptyRecipients)
    );
    assert_eq!(
        manager.activate_validated(
            spec("missing", vec![RecipientSelector::ShipSlot("wing".into())]),
            &fleet,
            &slots,
            &factions,
        ),
        Err(ActivationRefusal::UnknownShipSlot("wing".into()))
    );
    assert!(manager.records().is_empty());
}

#[test]
fn duplicate_explicit_slot_is_rejected_before_that_ship_joins() {
    let slots = BTreeSet::from(["lead".to_string()]);
    let mut manager = ObjectiveInstanceManager::default();
    assert_eq!(
        manager.activate_validated(
            spec("one", vec![RecipientSelector::ShipSlot("lead".into())]),
            &[],
            &slots,
            &BTreeSet::new(),
        ),
        Ok(true)
    );
    let before = manager.clone();
    assert_eq!(
        manager.activate_validated(
            spec("two", vec![RecipientSelector::ShipSlot("lead".into())]),
            &[],
            &slots,
            &BTreeSet::new(),
        ),
        Err(ActivationRefusal::StaticShipSlotConflict {
            objective_id: "survive".into(),
            slot_id: "lead".into(),
            instance_ids: vec!["one".into(), "two".into()],
        })
    );
    assert_eq!(manager, before);
}

#[test]
fn multi_ship_projection_preserves_unrelated_legacy_and_frozen_history() {
    let fleet = [ship("a", "lead", "alliance")];
    let mut manager = ObjectiveInstanceManager::default();
    manager
        .activate(
            spec("lead", vec![RecipientSelector::ShipSlot("lead".into())]),
            &fleet,
        )
        .unwrap();

    let projected =
        manager.project_snapshots_for_ship("a", vec![snapshot("survive"), snapshot("legacy")]);
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].id, "survive::lead");
    assert!(!projected[0].unassigned);
    assert_eq!(projected[1].id, "legacy");

    manager
        .reconcile(&[ship("a", "reserve", "alliance")])
        .unwrap();
    let projected = manager.project_snapshots_for_ship("a", vec![snapshot("survive")]);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].id, "survive::lead");
    assert!(projected[0].unassigned);
    assert!(manager.is_unassigned_display_key("a", "survive::lead"));
}

#[test]
fn multi_ship_toml_action_parses_typed_instance_recipients_and_progress() {
    use crate::world::config::{parse_action_entry, RawActionEntry, TriggerAction};

    let add = parse_action_entry(&RawActionEntry {
        kind: "add_objective".into(),
        id: Some("survive".into()),
        instance_id: Some("lead".into()),
        text: Some("objective.survive".into()),
        recipient_ship_slots: Some(vec!["lead".into()]),
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        add,
        TriggerAction::AddObjectiveInstance { spec, .. }
            if spec.key == key("lead")
                && spec.recipients == [RecipientSelector::ShipSlot("lead".into())]
    ));

    let progress = parse_action_entry(&RawActionEntry {
        kind: "set_objective_progress".into(),
        id: Some("survive".into()),
        instance_id: Some("lead".into()),
        progress: Some(0.5),
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        progress,
        TriggerAction::SetObjectiveInstanceProgress { key: parsed, progress }
            if parsed == key("lead") && progress == 0.5
    ));
}
#[test]
fn unassigned_history_does_not_mark_live_objective_targets() {
    let entity = crate::core::messages::EntitySnapshot {
        uuid: "marker".into(),
        objective_target: true,
        ..Default::default()
    };
    let mut objective = snapshot("hold");
    objective.targets = vec!["marker".into()];
    assert!(
        crate::objectives::project_entity_targets(
            std::slice::from_ref(&entity),
            &[objective.clone()]
        )[0]
        .objective_target
    );
    objective.unassigned = true;
    assert!(
        !crate::objectives::project_entity_targets(&[entity], &[objective])[0].objective_target
    );
}
