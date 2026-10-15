use super::*;

fn owner(value: &str) -> SystemId {
    SystemId(value.into())
}

#[test]
fn individual_override_survives_later_group_orders_and_rejoins_latest_base() {
    let bank = owner("phaser-port");
    let mut state = WeaponTargetAssignments::default();
    assert_eq!(state.target_for(&bank, Some("primary")), Some("primary"));
    state.set_group(
        bank.clone(),
        "broadside".into(),
        Some("first-contact".into()),
    );
    state.set_individual(bank.clone(), Some("chosen-contact".into()));
    state.set_group(
        bank.clone(),
        "broadside".into(),
        Some("later-contact".into()),
    );
    assert_eq!(
        state.target_for(&bank, Some("primary")),
        Some("chosen-contact")
    );
    state.follow_group(&bank);
    assert_eq!(
        state.target_for(&bank, Some("primary")),
        Some("later-contact")
    );
    state.follow_primary(&bank);
    assert_eq!(
        state.target_for(&bank, Some("new-primary")),
        Some("new-primary")
    );
    assert!(state.0.is_empty());
}

#[test]
fn explicit_hold_survives_json_and_never_falls_back_to_primary_or_group() {
    let bank = owner("phaser-port");
    let mut state = WeaponTargetAssignments::default();
    state.set_group(
        bank.clone(),
        "broadside".into(),
        Some("first-contact".into()),
    );
    state.set_individual(bank.clone(), None);
    let mut resumed: WeaponTargetAssignments =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(resumed, state);
    assert_eq!(resumed.target_for(&bank, Some("primary")), None);
    resumed.set_group(
        bank.clone(),
        "broadside".into(),
        Some("new-group-contact".into()),
    );
    assert_eq!(resumed.target_for(&bank, Some("primary")), None);
    resumed.follow_group(&bank);
    assert_eq!(
        resumed.target_for(&bank, Some("primary")),
        Some("new-group-contact")
    );
    resumed.set_group(bank.clone(), "broadside".into(), None);
    assert_eq!(resumed.target_for(&bank, Some("primary")), None);
}

#[test]
fn a_group_request_changes_only_the_addressed_bank() {
    let first = owner("phaser-port");
    let second = owner("phaser-starboard");
    let mut state = WeaponTargetAssignments::default();
    state.set_group(first.clone(), "all".into(), Some("contact".into()));
    assert_eq!(state.target_for(&first, Some("primary")), Some("contact"));
    assert_eq!(state.target_for(&second, Some("primary")), Some("primary"));
    state.set_individual(second.clone(), None);
    assert_eq!(state.target_for(&first, Some("primary")), Some("contact"));
    assert_eq!(state.target_for(&second, Some("primary")), None);
}
