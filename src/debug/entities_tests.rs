use super::*;

fn input(name: &str, x: f32, target: &str) -> EntityBehaviorInput {
    EntityBehaviorInput {
        name: name.to_string(),
        x,
        y: 0.0,
        z: 0.0,
        target: target.to_string(),
    }
}

#[test]
fn empty_projects_to_an_empty_versioned_payload() {
    let payload = project_entity_behavior(vec![]);
    assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
    assert!(payload.entries.is_empty());
}

#[test]
fn rows_are_sorted_by_name_and_carry_position_and_target() {
    let payload = project_entity_behavior(vec![
        input("Zephyr", 10.0, "player"),
        input("Aurora", -5.0, "none"),
    ]);
    assert_eq!(payload.entries.len(), 2);
    // Sorted by name: Aurora before Zephyr, regardless of input order.
    assert_eq!(payload.entries[0].name, "Aurora");
    assert!((payload.entries[0].x + 5.0).abs() < f32::EPSILON);
    assert_eq!(payload.entries[0].target, "none");
    assert_eq!(payload.entries[1].name, "Zephyr");
    assert_eq!(payload.entries[1].target, "player");
}
