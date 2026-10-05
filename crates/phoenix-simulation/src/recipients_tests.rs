use super::*;
use crate::objective_instances::ObjectiveInstanceKey;
use crate::objective_instances::ObjectiveInstanceSpec;
use crate::objective_instances::RecipientSelector;

fn fleet() -> Vec<PlayerShipMembership> {
    vec![
        PlayerShipMembership {
            ship_id: "z".into(),
            slot_id: "lead".into(),
            faction: "Alliance".into(),
        },
        PlayerShipMembership {
            ship_id: "a".into(),
            slot_id: "wing".into(),
            faction: "Dynasty".into(),
        },
    ]
}

fn catalog() -> RecipientCatalog {
    RecipientCatalog {
        ship_slots: ["lead", "wing", "absent"].map(String::from).into(),
        factions: ["Alliance", "Dynasty"].map(String::from).into(),
        objective_instances: [key()].into(),
    }
}

fn key() -> ObjectiveInstanceKey {
    ObjectiveInstanceKey {
        objective_id: "escort".into(),
        instance_id: "pair".into(),
    }
}

#[test]
fn omitted_and_explicit_empty_have_different_meanings() {
    assert_eq!(
        RecipientSelection::from_fields(None, None, None, None).unwrap(),
        None
    );
    let empty = RecipientSelection::from_fields(Some(vec![]), None, None, None)
        .unwrap()
        .unwrap();
    assert!(empty
        .resolve(&catalog(), &fleet(), &Default::default())
        .unwrap()
        .is_empty());
    assert!(
        RecipientSelection::from_fields(None, None, Some(false), None)
            .unwrap()
            .is_some()
    );
}

#[test]
fn malformed_present_fields_are_errors_instead_of_broadcast() {
    for source in [
            "#{ all_player_ships: 1 }",
            "#{ recipient_ship_slots: false }",
            "#{ recipient_factions: [1] }",
            "#{ recipient_objective_instances: [#{ objective_id: \"x\" }] }",
            "#{ recipient_objective_instances: [#{ objective_id: \"x\", instance_id: \"y\", typo: true }] }",
        ] {
            let map = crate::world::script::engine::runtime_engine().eval::<rhai::Map>(source).unwrap();
            assert!(RecipientSelection::from_rhai_map(&map).is_err(), "{source}");
        }
}

#[test]
fn union_is_stable_and_invalid_names_never_broaden() {
    let mut selection = RecipientSelection {
        selectors: vec![
            RecipientSelector::AllPlayerShips,
            RecipientSelector::ShipSlot("lead".into()),
        ],
        ..Default::default()
    };
    assert_eq!(
        selection
            .resolve(&catalog(), &fleet(), &Default::default())
            .unwrap(),
        ["a", "z"]
    );
    selection
        .selectors
        .push(RecipientSelector::ShipSlot("typo".into()));
    assert_eq!(
        selection.resolve(&catalog(), &fleet(), &Default::default()),
        Err(RecipientRefusal::UnknownShipSlot("typo".into()))
    );
}

#[test]
fn absent_slot_and_inactive_declared_instance_are_valid_empty() {
    let selection = RecipientSelection {
        selectors: vec![RecipientSelector::ShipSlot("absent".into())],
        objective_instances: vec![key()],
    };
    assert!(selection
        .resolve(&catalog(), &fleet(), &Default::default())
        .unwrap()
        .is_empty());
}

#[test]
fn current_instance_membership_changes_and_survives_restore() {
    let mut manager = ObjectiveInstanceManager::default();
    let mut fleet = fleet();
    manager
        .activate(
            ObjectiveInstanceSpec {
                key: key(),
                recipients: vec![RecipientSelector::Faction("Alliance".into())],
            },
            &fleet,
        )
        .unwrap();
    let selection = RecipientSelection {
        objective_instances: vec![key()],
        ..Default::default()
    };
    assert_eq!(
        selection.resolve(&catalog(), &fleet, &manager).unwrap(),
        ["z"]
    );
    fleet[0].faction = "Dynasty".into();
    fleet[1].faction = "Alliance".into();
    manager.reconcile(&fleet).unwrap();
    let restored = serde_json::from_str(&serde_json::to_string(&manager).unwrap()).unwrap();
    assert_eq!(
        selection.resolve(&catalog(), &fleet, &restored).unwrap(),
        ["a"]
    );
}
