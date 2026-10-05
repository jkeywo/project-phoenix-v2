use super::*;

fn cfg() -> IntentNarrationConfig {
    IntentNarrationConfig {
        break_off_hull_fraction: 0.5,
    }
}

fn with_target(label: &str) -> IntentSnapshot {
    IntentSnapshot {
        target_label: Some(label.into()),
        ..Default::default()
    }
}

fn hull(fraction: f32) -> IntentSnapshot {
    IntentSnapshot {
        hull_fraction: Some(fraction),
        ..Default::default()
    }
}

fn posture(pressed: bool) -> IntentSnapshot {
    IntentSnapshot {
        combat_posture: Some(pressed),
        ..Default::default()
    }
}

// ── Silence ───────────────────────────────────────────────────────────

/// AC: nothing in steady state. The first reading of a seat is not a
/// change, so it says nothing at all.
#[test]
fn the_first_observation_of_a_seat_is_silent() {
    assert_eq!(
        coalesce_intent(None, &with_target("Harrow Raider"), &cfg()),
        None
    );
}

/// AC: nothing in steady state — the case that matters most, because a
/// backfilled Tactical holding one target across a whole engagement is what
/// the shots-and-thrust-ticks noise would have come from.
#[test]
fn an_unchanged_decision_is_silent_however_long_it_is_held() {
    let held = IntentSnapshot {
        target_label: Some("Harrow Raider".into()),
        combat_posture: Some(true),
        hull_fraction: Some(0.30),
        shield_focus: Some("FORE".into()),
        brownout_groups: vec!["weapons".into()],
        manoeuvre: Some("attack_pass".into()),
    };
    for _ in 0..64 {
        assert_eq!(
            coalesce_intent(Some(&held), &held, &cfg()),
            None,
            "holding a decision must never narrate, no matter how many ticks pass"
        );
    }
}

/// Losing the target is not a decision the seat took.
#[test]
fn losing_a_target_is_silent() {
    assert_eq!(
        coalesce_intent(
            Some(&with_target("Harrow Raider")),
            &IntentSnapshot::default(),
            &cfg()
        ),
        None
    );
}

/// Staying below the break-off threshold is steady state; only the crossing
/// is a decision.
#[test]
fn hull_already_below_the_threshold_does_not_re_announce() {
    assert_eq!(
        coalesce_intent(Some(&hull(0.40)), &hull(0.35), &cfg()),
        None
    );
}

/// A group that was already browning out last tick is not news.
#[test]
fn a_brownout_that_was_already_running_is_silent() {
    let held = IntentSnapshot {
        brownout_groups: vec!["helm".into(), "weapons".into()],
        ..Default::default()
    };
    assert_eq!(coalesce_intent(Some(&held), &held, &cfg()), None);
}

/// Dropping shield focus is the "stopped" case, and stays quiet.
#[test]
fn clearing_shield_focus_is_silent() {
    let focused = IntentSnapshot {
        shield_focus: Some("FORE".into()),
        ..Default::default()
    };
    assert_eq!(
        coalesce_intent(Some(&focused), &IntentSnapshot::default(), &cfg()),
        None
    );
}

// ── One advisory per decision change ──────────────────────────────────

#[test]
fn acquiring_a_target_narrates_once() {
    let acquired = with_target("Harrow Raider");
    assert_eq!(
        coalesce_intent(Some(&IntentSnapshot::default()), &acquired, &cfg()),
        Some(IntentChange {
            kind: IntentKind::TargetAcquired,
            subject: Some("Harrow Raider".into()),
        })
    );
    // …and the tick after, holding the same target, says nothing.
    assert_eq!(coalesce_intent(Some(&acquired), &acquired, &cfg()), None);
}

#[test]
fn switching_target_narrates_exactly_one_advisory() {
    let switched = with_target("Harrow Lance");
    assert_eq!(
        coalesce_intent(Some(&with_target("Harrow Raider")), &switched, &cfg()),
        Some(IntentChange {
            kind: IntentKind::TargetSwitched,
            subject: Some("Harrow Lance".into()),
        })
    );
    assert_eq!(coalesce_intent(Some(&switched), &switched, &cfg()), None);
}

#[test]
fn entering_and_leaving_combat_posture_each_narrate_once() {
    assert_eq!(
        coalesce_intent(Some(&posture(false)), &posture(true), &cfg()),
        Some(IntentChange {
            kind: IntentKind::CombatPostureEntered,
            subject: None,
        })
    );
    assert_eq!(
        coalesce_intent(Some(&posture(true)), &posture(false), &cfg()),
        Some(IntentChange {
            kind: IntentKind::CombatPostureLeft,
            subject: None,
        })
    );
}

#[test]
fn crossing_the_break_off_threshold_narrates_once() {
    assert_eq!(
        coalesce_intent(Some(&hull(0.55)), &hull(0.45), &cfg()),
        Some(IntentChange {
            kind: IntentKind::BreakingOff,
            subject: None,
        })
    );
}

#[test]
fn focusing_a_shield_arc_narrates_once() {
    let focused = IntentSnapshot {
        shield_focus: Some("PORT".into()),
        ..Default::default()
    };
    assert_eq!(
        coalesce_intent(Some(&IntentSnapshot::default()), &focused, &cfg()),
        Some(IntentChange {
            kind: IntentKind::ShieldArcFocused,
            subject: Some("PORT".into()),
        })
    );
    assert_eq!(coalesce_intent(Some(&focused), &focused, &cfg()), None);
}

#[test]
fn a_group_entering_brownout_narrates_once_and_names_the_group() {
    let brownout = IntentSnapshot {
        brownout_groups: vec!["weapons".into()],
        ..Default::default()
    };
    assert_eq!(
        coalesce_intent(Some(&IntentSnapshot::default()), &brownout, &cfg()),
        Some(IntentChange {
            kind: IntentKind::PowerBrownout,
            subject: Some("weapons".into()),
        })
    );
    assert_eq!(coalesce_intent(Some(&brownout), &brownout, &cfg()), None);
}

#[test]
fn beginning_a_manoeuvre_narrates_the_authored_state_name() {
    let flying = IntentSnapshot {
        manoeuvre: Some("attack_pass".into()),
        ..Default::default()
    };
    assert_eq!(
        coalesce_intent(Some(&IntentSnapshot::default()), &flying, &cfg()),
        Some(IntentChange {
            kind: IntentKind::ManoeuvreBegun,
            subject: Some("attack_pass".into()),
        })
    );
}

// ── Zero-or-one, and determinism ──────────────────────────────────────

/// AC: **at most one** advisory per decision change. Five axes move on the
/// same tick and exactly one advisory comes out, chosen by the fixed
/// ladder rather than by whichever field the implementation happened to
/// test first.
#[test]
fn simultaneous_changes_yield_exactly_one_advisory() {
    let before = IntentSnapshot {
        target_label: Some("Harrow Raider".into()),
        combat_posture: Some(false),
        hull_fraction: Some(0.9),
        shield_focus: None,
        brownout_groups: vec![],
        manoeuvre: Some("shadow".into()),
    };
    let after = IntentSnapshot {
        target_label: Some("Harrow Lance".into()),
        combat_posture: Some(true),
        hull_fraction: Some(0.1),
        shield_focus: Some("AFT".into()),
        brownout_groups: vec!["weapons".into()],
        manoeuvre: Some("attack_pass".into()),
    };
    let change = coalesce_intent(Some(&before), &after, &cfg())
        .expect("five simultaneous changes must still produce an advisory");
    assert_eq!(
        change.kind,
        IntentKind::BreakingOff,
        "the ladder is most-urgent-first and fixed, so both hosts resolving \
             this tick pick the same one of the five"
    );
}

/// The unreported axes of a multi-change tick are still *recorded* by the
/// caller, so they do not surface later as changes that never happened.
/// Driving the same pair a second time proves the ladder is a function of
/// the pair alone and carries no hidden backlog.
#[test]
fn the_ladder_holds_no_backlog_of_unreported_axes() {
    let before = IntentSnapshot {
        hull_fraction: Some(0.9),
        target_label: Some("Harrow Raider".into()),
        ..Default::default()
    };
    let after = IntentSnapshot {
        hull_fraction: Some(0.1),
        target_label: Some("Harrow Lance".into()),
        ..Default::default()
    };
    let first = coalesce_intent(Some(&before), &after, &cfg());
    let second = coalesce_intent(Some(&before), &after, &cfg());
    assert_eq!(
        first, second,
        "the coalescer is a pure function of its pair"
    );
    assert_eq!(
        coalesce_intent(Some(&after), &after, &cfg()),
        None,
        "once the caller stores the new snapshot the unreported target switch \
             is history, not a queued advisory"
    );
}

// ── The #737 information boundary ─────────────────────────────────────

/// AC: the coarsening matches the #737 boundary — the coarse fact crosses,
/// the figure it was decided from does not.
#[test]
fn advisory_never_carries_a_figure_from_the_snapshot() {
    let before = IntentSnapshot {
        hull_fraction: Some(0.51),
        ..Default::default()
    };
    let after = IntentSnapshot {
        hull_fraction: Some(0.4917),
        ..Default::default()
    };
    let change = coalesce_intent(Some(&before), &after, &cfg()).expect("crossing narrates");
    assert_eq!(change.kind, IntentKind::BreakingOff);
    assert_eq!(
        change.subject, None,
        "the hull figure the decision was made from must not ride along; \
             #737's rule is that the tier crosses and the number does not"
    );
}

/// AGENTS.md #11: the threshold is authored, so moving it moves the
/// crossing. A literal in the comparison would make both of these fire the
/// same way.
#[test]
fn the_break_off_threshold_is_authored_not_hardcoded() {
    let cautious = IntentNarrationConfig {
        break_off_hull_fraction: 0.8,
    };
    let stoic = IntentNarrationConfig {
        break_off_hull_fraction: 0.2,
    };
    assert!(
        coalesce_intent(Some(&hull(0.9)), &hull(0.7), &cautious).is_some(),
        "a hull authored to break off at 0.8 narrates when it drops to 0.7"
    );
    assert_eq!(
        coalesce_intent(Some(&hull(0.9)), &hull(0.7), &stoic),
        None,
        "a hull authored to break off at 0.2 has decided nothing at 0.7"
    );
}
