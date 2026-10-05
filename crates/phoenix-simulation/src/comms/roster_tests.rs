use super::*;

fn seated(uuid: &str, name: &str) -> CommsContact {
    CommsContact {
        uuid: uuid.into(),
        name: name.into(),
        in_range: false,
        is_urgent: true,
    }
}

fn derived(name: &str, uuid: &str) -> EntityContact {
    EntityContact {
        name: name.into(),
        uuid: uuid.into(),
    }
}

// ── Label precedence ──────────────────────────────────────────────────

#[test]
fn label_prefers_authored_display_name() {
    assert_eq!(
        entity_contact_label(
            Some("Axiom Station"),
            Some("world.entity.starbase_alpha.name"),
            "u1"
        ),
        "Axiom Station"
    );
}

#[test]
fn label_falls_back_to_the_entity_reference_id() {
    assert_eq!(
        entity_contact_label(None, Some("world.entity.starbase_alpha.name"), "u1"),
        "world.entity.starbase_alpha.name"
    );
}

#[test]
fn label_falls_back_to_the_uuid_when_the_entity_is_unnamed() {
    assert_eq!(entity_contact_label(None, None, "u1"), "u1");
}

// ── Merge semantics ───────────────────────────────────────────────────

#[test]
fn entity_contacts_are_appended_to_an_empty_roster() {
    let mut roster = Vec::new();
    let mut derived_in = vec![derived("Bravo", "u2"), derived("Alpha", "u1")];
    assert!(merge_entity_contacts(&mut roster, &mut derived_in));
    assert_eq!(
        roster.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["Alpha", "Bravo"]
    );
    assert!(roster.iter().all(|c| c.in_range && !c.is_urgent));
}

#[test]
fn a_seated_entry_wins_the_uuid_collision() {
    let mut roster = vec![seated("u1", "Starbase Alpha")];
    let mut derived_in = vec![derived("world.entity.starbase_alpha.name", "u1")];
    assert!(
        !merge_entity_contacts(&mut roster, &mut derived_in),
        "a colliding entity-derived contact adds nothing"
    );
    assert_eq!(roster.len(), 1);
    // Name AND the live range/urgency stamps survive untouched.
    assert_eq!(roster[0].name, "Starbase Alpha");
    assert!(!roster[0].in_range);
    assert!(roster[0].is_urgent);
}

#[test]
fn only_the_uncollided_entity_contacts_are_appended() {
    let mut roster = vec![seated("u1", "Starbase Alpha")];
    let mut derived_in = vec![derived("Alpha", "u1"), derived("Courier", "u2")];
    assert!(merge_entity_contacts(&mut roster, &mut derived_in));
    assert_eq!(
        roster
            .iter()
            .map(|c| (c.uuid.as_str(), c.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("u1", "Starbase Alpha"), ("u2", "Courier")]
    );
}

#[test]
fn merging_is_idempotent_across_ticks() {
    let mut roster = Vec::new();
    let mut first = vec![derived("Alpha", "u1")];
    assert!(merge_entity_contacts(&mut roster, &mut first));
    let mut second = vec![derived("Alpha", "u1")];
    assert!(
        !merge_entity_contacts(&mut roster, &mut second),
        "the same live entity must not be re-added on the next tick"
    );
    assert_eq!(roster.len(), 1);
}

#[test]
fn append_order_is_independent_of_input_order() {
    let names = [
        ("Delta", "u4"),
        ("Alpha", "u1"),
        ("Charlie", "u3"),
        ("Bravo", "u2"),
    ];
    let mut forward: Vec<CommsContact> = Vec::new();
    let mut a: Vec<EntityContact> = names.iter().map(|(n, u)| derived(n, u)).collect();
    merge_entity_contacts(&mut forward, &mut a);

    let mut reversed: Vec<CommsContact> = Vec::new();
    let mut b: Vec<EntityContact> = names.iter().rev().map(|(n, u)| derived(n, u)).collect();
    merge_entity_contacts(&mut reversed, &mut b);

    assert_eq!(
        forward, reversed,
        "roster order must not depend on query order"
    );
    assert_eq!(
        forward.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["Alpha", "Bravo", "Charlie", "Delta"]
    );
}

#[test]
fn duplicate_names_tie_break_on_uuid() {
    let mut roster = Vec::new();
    let mut derived_in = vec![derived("Harrow", "u9"), derived("Harrow", "u2")];
    merge_entity_contacts(&mut roster, &mut derived_in);
    assert_eq!(
        roster.iter().map(|c| c.uuid.as_str()).collect::<Vec<_>>(),
        vec!["u2", "u9"]
    );
}

#[test]
fn a_repeated_uuid_in_one_batch_is_collapsed() {
    let mut roster = Vec::new();
    let mut derived_in = vec![derived("Alpha", "u1"), derived("Alpha", "u1")];
    merge_entity_contacts(&mut roster, &mut derived_in);
    assert_eq!(roster.len(), 1);
}

#[test]
fn an_empty_derivation_leaves_the_seated_roster_alone() {
    let mut roster = vec![seated("u1", "Starbase Alpha"), seated("u2", "Raider")];
    let before = roster.clone();
    let mut derived_in = Vec::new();
    assert!(!merge_entity_contacts(&mut roster, &mut derived_in));
    assert_eq!(roster, before);
}
