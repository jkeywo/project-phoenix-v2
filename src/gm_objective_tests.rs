use super::*;

fn raw(id: String, recipients: Vec<String>) -> RawObjectivePaletteEntry {
    RawObjectivePaletteEntry {
        id,
        label: "objective.test".into(),
        recipients,
        fields: toml::toml! { text = "objective.test" },
    }
}

#[test]
fn palette_authoring_obeys_the_same_byte_and_scope_bounds_as_actions() {
    for id in ["x".repeat(128), "é".repeat(64)] {
        let recipients = (0..32).map(|n| format!("ship-{n:02}")).collect();
        let entries = parse_palette(&[raw(id.clone(), recipients)]).unwrap();
        assert!(valid_request_vocabulary(&id, &entries[0].recipients));
    }
    for id in [
        String::new(),
        "x".repeat(129),
        "é".repeat(65),
        "bad\nid".into(),
    ] {
        assert!(parse_palette(&[raw(id, vec![])]).is_err());
    }
    for recipients in [
        (0..33).map(|n| format!("ship-{n}")).collect(),
        vec!["ship".into(); 33],
        vec![String::new()],
        vec!["x".repeat(129)],
        vec!["bad\rship".into()],
    ] {
        assert!(parse_palette(&[raw("objective".into(), recipients)]).is_err());
    }
    let entries = parse_palette(&[raw("objective".into(), vec!["x".repeat(128)])]).unwrap();
    assert!(valid_request_vocabulary(
        &entries[0].id,
        &entries[0].recipients
    ));
}

#[test]
fn resolved_aliases_and_retained_records_never_advertise_impossible_requests() {
    let mut runtime = crate::world::server::WorldContentRuntime {
        gm_objective_palette: parse_palette(&[raw("objective".into(), vec!["alias".into()])])
            .unwrap(),
        ..Default::default()
    };
    for invalid in ["x".repeat(129), "bad\nship".into()] {
        runtime.name_to_uuid.insert("alias".into(), invalid.clone());
        let (palette, _) = rows(Some(&runtime), None, None, &[invalid]);
        assert!(!palette[0].available);
        assert_eq!(palette[0].recipients, ["alias"]);
    }
    runtime.name_to_uuid.insert("alias".into(), "ship-a".into());
    assert!(!rows(Some(&runtime), None, None, &[]).0[0].available);
    let valid = rows(Some(&runtime), None, None, &["ship-a".into()])
        .0
        .remove(0);
    assert!(valid.available);
    assert!(valid_request_vocabulary(&valid.id, &valid.recipients));

    let mut manager = crate::objectives::ObjectiveManager::default();
    let oversized_id = "x".repeat(129);
    manager.add(&oversized_id, "objective.test", true, vec![]);
    manager.add("too-many-ships", "objective.test", true, vec![]);
    let live: Vec<String> = (0..33).map(|n| format!("ship-{n:02}")).collect();
    manager.set_recipients("too-many-ships", live.clone());
    assert!(rows(None, Some(&manager), None, &live)
        .1
        .iter()
        .all(|row| !row.available));
}

#[test]
fn gm_projection_lists_every_named_instance_with_current_and_fixed_membership() {
    use crate::objective_instances::{
        ObjectiveInstanceKey, ObjectiveInstanceManager, ObjectiveInstanceSpec,
        PlayerShipMembership, RecipientSelector,
    };

    let mut definitions = crate::objectives::ObjectiveManager::default();
    definitions.add("hold", "objective.hold", true, vec![]);
    let fleet = [
        PlayerShipMembership {
            ship_id: "ship-a".into(),
            slot_id: "lead".into(),
            faction: "alliance".into(),
        },
        PlayerShipMembership {
            ship_id: "ship-b".into(),
            slot_id: "wing".into(),
            faction: "alliance".into(),
        },
    ];
    let mut instances = ObjectiveInstanceManager::default();
    for (instance_id, slot_id) in [("lead", "lead"), ("wing", "wing")] {
        instances
            .activate(
                ObjectiveInstanceSpec {
                    key: ObjectiveInstanceKey {
                        objective_id: "hold".into(),
                        instance_id: instance_id.into(),
                    },
                    recipients: vec![RecipientSelector::ShipSlot(slot_id.into())],
                },
                &fleet,
            )
            .unwrap();
    }
    instances
        .complete(
            &ObjectiveInstanceKey {
                objective_id: "hold".into(),
                instance_id: "lead".into(),
            },
            &fleet,
        )
        .unwrap();

    let projected = rows(
        None,
        Some(&definitions),
        Some(&instances),
        &["ship-a".into(), "ship-b".into()],
    )
    .1;
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].instance_id.as_deref(), Some("lead"));
    assert_eq!(projected[0].recipients, ["ship-a"]);
    assert_eq!(projected[0].completion_members, ["ship-a"]);
    assert_eq!(projected[1].instance_id.as_deref(), Some("wing"));
    assert_eq!(projected[1].recipients, ["ship-b"]);
    assert!(projected[1].completion_members.is_empty());
}
