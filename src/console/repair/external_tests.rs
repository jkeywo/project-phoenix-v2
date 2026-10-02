use super::*;
use crate::modifiers::repair_teams::RepairTeams;

// ── The dispatch verdict ─────────────────────────────────────────────────

#[test]
fn a_free_team_a_designated_target_in_range_may_be_dispatched() {
    assert_eq!(
        dispatch_status(true, Some("ally"), Some(300.0), 600.0),
        Ok(())
    );
    // Exactly at the range boundary still dispatches.
    assert_eq!(
        dispatch_status(true, Some("ally"), Some(600.0), 600.0),
        Ok(())
    );
}

#[test]
fn no_free_team_refuses_first_of_all() {
    // The tool-state gate wins even when there is also no target and nothing
    // in range: the crew are told the one nearest the tool itself.
    assert_eq!(
        dispatch_status(false, None, None, 600.0),
        Err(ExternalRepairRefusal::NoFreeTeam)
    );
    assert_eq!(
        dispatch_status(false, Some("ally"), Some(10.0), 600.0),
        Err(ExternalRepairRefusal::NoFreeTeam)
    );
}

#[test]
fn no_designated_target_refuses_no_target_before_range() {
    assert_eq!(
        dispatch_status(true, None, None, 600.0),
        Err(ExternalRepairRefusal::NoTarget)
    );
}

#[test]
fn a_target_past_the_authored_range_refuses_out_of_range() {
    assert_eq!(
        dispatch_status(true, Some("ally"), Some(600.1), 600.0),
        Err(ExternalRepairRefusal::OutOfRange)
    );
    // A present target whose entity cannot be found (no separation) is also
    // "nothing in range" — the same reading the tractor takes.
    assert_eq!(
        dispatch_status(true, Some("ally"), None, 600.0),
        Err(ExternalRepairRefusal::OutOfRange)
    );
}

// ── Withdrawal from the internal sweep (the #1027 availability answer) ─────

#[test]
fn a_dispatched_team_is_withdrawn_from_the_internal_sweep() {
    // Three idle teams, the one the crew CHOSE working abroad: the internal
    // sweep sees the other two — the same "one place which teams are
    // available is answered". This is what makes helping an ally a real
    // trade against fixing your own shields: the console and the repair AI
    // both read this answer.
    let teams = RepairTeams::new(3);
    assert_eq!(teams.free_team_indices(None), vec![0, 1, 2]);
    assert_eq!(
        teams.free_team_indices(Some(1)),
        vec![0, 2],
        "the team that actually went is unavailable to the internal sweep — issue #1386 \
             names it rather than truncating the tail, so a seat may send the middle team"
    );
    // The dispatched team is still Idle in every readout — held back, not
    // moved — so it is `is_committed_to_operation`, not a busy slot.
    assert!(teams.is_committed_to_operation(1, Some(1)));
    assert!(!teams.is_committed_to_operation(0, Some(1)));
}

#[test]
fn a_one_team_hull_that_sends_its_team_abroad_has_none_for_its_own_damage() {
    let teams = RepairTeams::new(1);
    assert!(
        teams.free_team_indices(Some(0)).is_empty(),
        "the only team spoken for abroad leaves the hull's own sweep nothing — the \
             capacity-as-cost trade"
    );
}

// ── The NAMED dispatch verdict (issue #1386) ─────────────────────────────

#[test]
fn a_named_idle_team_with_the_claim_free_may_be_dispatched() {
    assert_eq!(
        named_dispatch_status(true, None, 1, Some("ally"), Some(300.0), 600.0),
        Ok(())
    );
}

#[test]
fn naming_a_team_that_is_not_idle_refuses_team_busy_first_of_all() {
    // The team the seat tapped is reported before the ship-wide claim: it
    // is the more actionable of the two, and telling a seat "another team is
    // already out there" about a team that could not have gone anyway would
    // have it recall the wrong team.
    assert_eq!(
        named_dispatch_status(false, Some(2), 0, Some("ally"), Some(10.0), 600.0),
        Err(ExternalRepairRefusal::TeamBusy)
    );
    // A slot this hull does not have reads the same way — a team that is not
    // there cannot go.
    assert_eq!(
        named_dispatch_status(false, None, 9, Some("ally"), Some(10.0), 600.0),
        Err(ExternalRepairRefusal::TeamBusy)
    );
}

#[test]
fn naming_a_second_team_while_one_is_abroad_refuses_already_abroad() {
    assert_eq!(
        named_dispatch_status(true, Some(2), 0, Some("ally"), Some(10.0), 600.0),
        Err(ExternalRepairRefusal::AlreadyAbroad)
    );
}

#[test]
fn re_pointing_the_team_already_abroad_is_not_a_refusal() {
    // The claim stays single, so naming the team that already holds it is a
    // re-target onto whatever Tactical has locked now — the same same-claim
    // re-target the fieldless verb has always allowed.
    assert_eq!(
        named_dispatch_status(true, Some(2), 2, Some("ally-2"), Some(10.0), 600.0),
        Ok(())
    );
}

#[test]
fn a_named_dispatch_still_reports_acquisition_after_the_team_gates() {
    // Delegated to `dispatch_status`, so `NoTarget` precedes `OutOfRange` by
    // construction rather than by a second copy of the order.
    assert_eq!(
        named_dispatch_status(true, None, 0, None, None, 600.0),
        Err(ExternalRepairRefusal::NoTarget)
    );
    assert_eq!(
        named_dispatch_status(true, None, 0, Some("ally"), Some(600.1), 600.0),
        Err(ExternalRepairRefusal::OutOfRange)
    );
}

#[test]
fn every_refusal_resolves_a_distinct_strings_csv_id() {
    // The `string_id` match is what lets `check-strings.mjs` see every id a
    // new variant needs a row for; asserting they are distinct is what
    // catches a copy-pasted arm.
    let ids = [
        ExternalRepairRefusal::NoFreeTeam.string_id(),
        ExternalRepairRefusal::NoTarget.string_id(),
        ExternalRepairRefusal::OutOfRange.string_id(),
        ExternalRepairRefusal::TeamBusy.string_id(),
        ExternalRepairRefusal::AlreadyAbroad.string_id(),
    ];
    let unique: std::collections::BTreeSet<&str> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
    assert_eq!(
        ExternalRepairRefusal::TeamBusy.string_id(),
        "repair.dispatch.refused.team_busy"
    );
    assert_eq!(
        ExternalRepairRefusal::AlreadyAbroad.string_id(),
        "repair.dispatch.refused.already_abroad"
    );
}

// ── Config validation ────────────────────────────────────────────────────

#[test]
fn a_well_formed_external_repair_config_validates() {
    assert!(ExternalRepairConfig {
        range: 600.0,
        repair_rate: 8.0,
    }
    .validate()
    .is_ok());
}

#[test]
fn a_zero_or_negative_range_or_rate_is_rejected() {
    assert!(ExternalRepairConfig {
        range: 0.0,
        repair_rate: 8.0
    }
    .validate()
    .is_err());
    assert!(ExternalRepairConfig {
        range: -1.0,
        repair_rate: 8.0
    }
    .validate()
    .is_err());
    assert!(ExternalRepairConfig {
        range: 600.0,
        repair_rate: 0.0
    }
    .validate()
    .is_err());
    assert!(ExternalRepairConfig {
        range: 600.0,
        repair_rate: -3.0
    }
    .validate()
    .is_err());
}

#[test]
fn the_config_round_trips_through_toml() {
    let authored = r#"
range = 800.0
repair_rate = 12.0
"#;
    let parsed: ExternalRepairConfig =
        toml::from_str(authored).expect("external dispatch config parses");
    assert_eq!(parsed.range, 800.0);
    assert_eq!(parsed.repair_rate, 12.0);
    parsed.validate().expect("valid");
}

#[test]
fn an_unknown_field_is_a_parse_error_rather_than_a_silently_ignored_typo() {
    let err = toml::from_str::<ExternalRepairConfig>("range = 600.0\nrepar_rate = 8.0")
        .expect_err("a misspelt field must not be swallowed");
    assert!(
        err.to_string().contains("repar_rate"),
        "the error must name the offending field, got {err}"
    );
}
