use super::*;

fn overrides() -> GmFactionOverrides {
    let mut value = GmFactionOverrides::default();
    value.record("Harrow", "Alliance", false, true);
    value
}

#[test]
fn keeps_the_pre_gm_baseline_through_later_changes_to_the_same_pair() {
    let mut value = overrides();
    // Adding a hostility owes no lock re-validation; withdrawing one does.
    assert!(!value.revalidation_pending());
    value.record("Harrow", "Alliance", true, false);
    assert!(value.revalidation_pending());
    assert_eq!(value.entries().len(), 1);
    // `before` is the value the pair held before ANY GM touched it, so a
    // restore of a save that predates the first change reverts to `false`,
    // not to the value the second change happened to see.
    assert!(!value.entries()[0].before);
    assert!(!value.entries()[0].current);
    assert_eq!(value.current("Harrow", "Alliance"), Some(false));
}

#[test]
fn stays_sorted_so_the_digest_and_snapshot_are_order_independent() {
    let mut value = GmFactionOverrides::default();
    value.record("Zephyr", "Alliance", false, true);
    value.record("Alliance", "Harrow", false, true);
    value.record("Alliance", "Alliance", false, true);
    assert_eq!(
        value
            .entries()
            .iter()
            .map(|entry| (entry.faction.as_str(), entry.enemy.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("Alliance", "Alliance"),
            ("Alliance", "Harrow"),
            ("Zephyr", "Alliance")
        ]
    );
    assert_eq!(value.current("Alliance", "Missing"), None);
}

#[test]
fn projects_faction_rows_by_name_in_a_stable_order() {
    use crate::ai::faction::{FactionConfig, FactionRegistry};
    let alliance = uuid::Uuid::from_u128(1);
    let harrow = uuid::Uuid::from_u128(2);
    let mut registry = FactionRegistry::new();
    registry.insert(FactionConfig {
        uuid: harrow,
        name: "Harrow".into(),
        display_name: Some("dossier.faction.harrow".into()),
        enemies: vec![alliance],
        compliance: None,
    });
    registry.insert(FactionConfig {
        uuid: alliance,
        name: "Alliance".into(),
        display_name: None,
        enemies: vec![],
        compliance: None,
    });
    let rows = faction_rows(Some(&registry));
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        vec!["Alliance", "Harrow"]
    );
    assert_eq!(rows[0].enemies, Vec::<String>::new());
    assert_eq!(rows[1].enemies, vec!["Alliance".to_string()]);
    assert_eq!(rows[1].label.as_deref(), Some("dossier.faction.harrow"));
    assert!(faction_rows(None).is_empty());
}
