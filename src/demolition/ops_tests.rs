use super::*;

fn base_config() -> DemolitionConfig {
    DemolitionConfig {
        charges_flag: "x_charged".to_string(),
        detonated_flag: "x_detonated".to_string(),
        safe_flag: "x_safe".to_string(),
        unsupported_flag: "x_unsupported".to_string(),
        premature_flag: "x_premature".to_string(),
        stabilization_required: false,
        warning: None,
    }
}

#[test]
fn valid_config_passes() {
    assert!(base_config().validate().is_ok());
}

#[test]
fn blank_flag_is_refused_at_load() {
    let mut config = base_config();
    config.safe_flag = "   ".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn duplicate_flag_names_are_refused_at_load() {
    let mut config = base_config();
    config.unsupported_flag = config.safe_flag.clone();
    let err = config.validate().unwrap_err();
    assert!(err.contains("unsupported_flag"));
    assert!(err.contains("safe_flag"));
}

#[test]
fn blank_warning_is_refused_but_absent_one_is_fine() {
    let mut config = base_config();
    config.warning = Some(String::new());
    assert!(config.validate().is_err());
    config.warning = None;
    assert!(config.validate().is_ok());
}

#[test]
fn outcome_flag_maps_each_outcome_to_its_authored_flag() {
    let config = base_config();
    assert_eq!(config.outcome_flag(DemolitionOutcome::Safe), "x_safe");
    assert_eq!(
        config.outcome_flag(DemolitionOutcome::Unsupported),
        "x_unsupported"
    );
    assert_eq!(
        config.outcome_flag(DemolitionOutcome::Premature),
        "x_premature"
    );
}

#[test]
fn disabled_system_refuses_before_anything_else() {
    // Disabled wins even when the target is unknown and uncharged.
    assert_eq!(
        detonation_status(false, false, false, true),
        Err(DemolitionRefusal::Disabled)
    );
}

#[test]
fn unknown_target_is_refused() {
    assert_eq!(
        detonation_status(false, true, false, false),
        Err(DemolitionRefusal::NoSuchTarget)
    );
}

#[test]
fn uncharged_target_cannot_be_detonated() {
    assert_eq!(
        detonation_status(true, false, false, false),
        Err(DemolitionRefusal::NotCharged)
    );
}

#[test]
fn already_detonated_is_idempotently_refused() {
    assert_eq!(
        detonation_status(true, true, true, false),
        Err(DemolitionRefusal::AlreadyDetonated)
    );
}

#[test]
fn a_charged_undetonated_target_may_fire() {
    assert_eq!(detonation_status(true, true, false, false), Ok(()));
}

#[test]
fn team_still_present_is_always_premature() {
    // Regardless of stabilisation, a team on the target dies.
    assert_eq!(
        resolve_outcome(false, true, true),
        DemolitionOutcome::Premature
    );
    assert_eq!(
        resolve_outcome(false, false, false),
        DemolitionOutcome::Premature
    );
}

#[test]
fn clear_and_stabilised_is_safe() {
    assert_eq!(resolve_outcome(true, true, true), DemolitionOutcome::Safe);
}

#[test]
fn clear_but_unheld_when_required_is_unsupported() {
    assert_eq!(
        resolve_outcome(true, true, false),
        DemolitionOutcome::Unsupported
    );
}

#[test]
fn stabilisation_not_required_is_safe_without_a_hold() {
    // A target that does not need holding is safe with no coupling.
    assert_eq!(resolve_outcome(true, false, false), DemolitionOutcome::Safe);
}

#[test]
fn every_refusal_has_a_distinct_string_id() {
    let mut ids: Vec<&str> = DemolitionRefusal::ALL
        .iter()
        .map(|r| r.string_id())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), DemolitionRefusal::ALL.len());
}
