use super::*;
use crate::objective_instances::*;

#[test]
fn effective_reading_keeps_legacy_rows_and_instance_history_distinct_from_actions() {
    let mut definitions = ObjectiveManager::default();
    definitions.add("survive", "mission.survive", true, vec!["subject".into()]);
    definitions.add("legacy", "mission.legacy", false, vec![]);
    let fleet = [PlayerShipMembership {
        ship_id: "a".into(),
        slot_id: "lead".into(),
        faction: "alliance".into(),
    }];
    let key = ObjectiveInstanceKey {
        objective_id: "survive".into(),
        instance_id: "fleet".into(),
    };
    let mut instances = ObjectiveInstanceManager::default();
    instances
        .activate(
            ObjectiveInstanceSpec {
                key: key.clone(),
                recipients: vec![RecipientSelector::AllPlayerShips],
            },
            &fleet,
        )
        .unwrap();
    let conditions = WorldConditions::default();
    let effective =
        definitions.effective_scored_for_ship(&conditions, Some("survive"), "a", Some(&instances));
    assert_eq!(effective.len(), 2);
    let instance = effective.iter().find(|o| o.id != "legacy").unwrap();
    assert_ne!(instance.id, "survive");
    assert_eq!(instance.snapshot.targets, ["subject"]);
    assert_eq!(
        definitions.effective_scored_for_ship(&conditions, None, "a", None),
        definitions.scored_pool_for(&conditions, "a")
    );
    instances.complete(&key, &fleet).unwrap();
    let actions = definitions.effective_scored_for_ship(&conditions, None, "a", Some(&instances));
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "legacy");
    instances.reconcile(&[]).unwrap();
    let display = definitions.effective_visible_for_ship(&conditions, None, "a", Some(&instances));
    let history = display.iter().find(|o| o.id != "legacy").unwrap();
    assert_eq!(history.status, ObjectiveStatus::Completed);
    assert!(history.unassigned);
    assert_eq!(
        definitions
            .effective_scored_for_ship(&conditions, None, "a", Some(&instances))
            .len(),
        1
    );
}
