use super::*;

#[test]
fn palette_contains_only_actual_closed_damage_bindings_and_outcomes() {
    let schema = schema();
    assert_eq!(schema.damage_types, ["physical", "laser"]);
    assert_eq!(schema.bindings, ["operation", "intake", "cooling"]);
    assert_eq!(schema.outcomes, ["structural", "full", "patch"]);
}
