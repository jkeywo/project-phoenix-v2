//! The intent-narration coalescer (issue #879) — pure, Bevy-free (AGENTS.md #10).
//!
//! A backfilled seat takes a decision every AI tick, but the crew must hear
//! about it only when the decision *changes*. This module is the one place that
//! decides "did anything worth saying happen between these two snapshots?", and
//! it answers with **zero or one** coarsened advisory. Steady state — the same
//! decision held across ticks, however many shots or thrust ticks it produced —
//! produces nothing at all.
//!
//! # Why a snapshot pair rather than an event stream
//!
//! Every emitter that already lives on the channel-3 bus grew its own bespoke
//! debounce: `SensorsFrequencyState` remembers the last target and frequency it
//! sent, `PowerBrownoutState` keeps the set of groups it has already announced,
//! `ShieldsCoordinationState` tracks a per-facing down/restore cycle. Each of
//! those is a hand-rolled edge detector, and each is a place a future change can
//! reintroduce spam. Narration covers five decision axes at once, so it takes
//! the state-change detection out of the emitter entirely: the adapter reads
//! authoritative state into an [`IntentSnapshot`] and hands the previous one and
//! the new one to [`coalesce_intent`]. The emitter cannot spam because it has no
//! say in the matter.
//!
//! # The #737 information boundary
//!
//! [`IntentSnapshot`] carries the exact figures the decision was *made* from —
//! the hull fraction the break-off threshold is compared against, for one. The
//! advisory carries none of them. That is the same boundary issue #737 drew for
//! `CoordinationPayload::RepairRequest`, where the tier crossing still reaches
//! Engineering and the exact HP deficit does not: the coarse fact travels, the
//! number stays home. `advisory_never_carries_a_figure_from_the_snapshot` pins
//! it, and the delivery side re-applies #737's own
//! `coarsen_repair_request` per recipient, so a ship-wide broadcast cannot
//! become a way around the gate for any payload that does carry a number.

use crate::messages::IntentKind;

/// One backfilled seat's decision state at one AI decision tick.
///
/// Every field is `Option`/empty for a seat that does not report on that axis:
/// Tactical fills [`Self::target_label`] and nothing else, Helm fills posture,
/// hull and manoeuvre, Shields fills the focused arc, Power fills the brownout
/// set. A field left at its default is "this seat has nothing to say here",
/// which reads identically to "unchanged" and therefore stays silent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntentSnapshot {
    /// Human-readable label of the seat's committed target, if it holds one.
    pub target_label: Option<String>,
    /// Whether the ship's alert state licenses the aggressive half of the
    /// class doctrine — the same distinction the helm AI's `posture` fact
    /// draws (`crate::ship::helm_ai::POSTURE_FACT`).
    pub combat_posture: Option<bool>,
    /// Hull integrity as a fraction of maximum, `0.0..=1.0`.
    ///
    /// The **exact** figure, deliberately: the break-off threshold is authored
    /// data and the comparison belongs here, in the pure function a test can
    /// drive, rather than in the Bevy adapter. It never reaches the advisory.
    pub hull_fraction: Option<f32>,
    /// Label of the shield facing the seat has focused, if any.
    pub shield_focus: Option<String>,
    /// Power groups currently browning out, **sorted**.
    ///
    /// Sorted because the advisory names the group that newly appeared, and a
    /// `HashSet`'s iteration order would make which group that is depend on
    /// hash seeding — a lockstep divergence between two hosts running the same
    /// tick.
    pub brownout_groups: Vec<String>,
    /// The authored state name of the manoeuvre the seat is flying, if its
    /// policy runs a state machine.
    pub manoeuvre: Option<String>,
}

/// The coarsened advisory a decision change produces: what changed, and the one
/// label naming it. Never a figure — see the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentChange {
    pub kind: IntentKind,
    pub subject: Option<String>,
}

/// Authored thresholds the coalescer compares against (AGENTS.md #11).
///
/// The struct exists so the threshold arrives as a parameter rather than as a
/// literal in the comparison: a hull fraction is exactly the kind of value a
/// designer retunes, and hardcoding it here would make the "breaking off"
/// advisory fire at a number nobody authored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntentNarrationConfig {
    /// Hull fraction at or below which a seat is deemed to be breaking off.
    /// Authored as `[global] intent_break_off_hull_fraction`.
    pub break_off_hull_fraction: f32,
}

/// Map a seat's previous and new decision snapshots to zero or one advisory.
///
/// `prev` is `None` the first time a seat is ever observed, which is silent:
/// there is no *change* in a first reading, and firing on one would announce
/// the whole bridge's opening state to a crew that was already there.
///
/// # The priority ladder
///
/// Several axes can move on the same tick — a hull crossing and a target switch
/// arrive together often enough. The contract is zero-or-one, so the ladder
/// below picks exactly one, most-urgent first, and the unreported axes are
/// still recorded in the snapshot the caller stores, so they do not re-fire
/// later as phantom changes. The order is fixed rather than incidental, so two
/// hosts resolving the same tick pick the same advisory.
pub fn coalesce_intent(
    prev: Option<&IntentSnapshot>,
    next: &IntentSnapshot,
    cfg: &IntentNarrationConfig,
) -> Option<IntentChange> {
    let prev = prev?;

    // 1. Breaking off: the hull has just crossed the authored threshold
    //    DOWNWARD. A ship that stays below it is not deciding anything new.
    if let (Some(before), Some(now)) = (prev.hull_fraction, next.hull_fraction) {
        let t = cfg.break_off_hull_fraction;
        if before > t && now <= t {
            return Some(IntentChange {
                kind: IntentKind::BreakingOff,
                subject: None,
            });
        }
    }

    // 2. Brownout: the rising edge of a group entering brownout. A group that
    //    was already browning out last tick is steady state.
    if let Some(group) = next
        .brownout_groups
        .iter()
        .find(|g| !prev.brownout_groups.contains(g))
    {
        return Some(IntentChange {
            kind: IntentKind::PowerBrownout,
            subject: Some(group.clone()),
        });
    }

    // 3. Combat posture, both directions.
    if let (Some(before), Some(now)) = (prev.combat_posture, next.combat_posture) {
        if before != now {
            return Some(IntentChange {
                kind: if now {
                    IntentKind::CombatPostureEntered
                } else {
                    IntentKind::CombatPostureLeft
                },
                subject: None,
            });
        }
    }

    // 4. Target acquire / switch. Losing a target is not narrated: nothing was
    //    decided, the contact simply stopped existing, and the crew's own radar
    //    already shows that.
    if prev.target_label != next.target_label {
        if let Some(label) = &next.target_label {
            return Some(IntentChange {
                kind: if prev.target_label.is_some() {
                    IntentKind::TargetSwitched
                } else {
                    IntentKind::TargetAcquired
                },
                subject: Some(label.clone()),
            });
        }
    }

    // 5. Shield arc focus. Dropping focus is the "stopped doing a thing" case
    //    again, and is silent for the same reason.
    if prev.shield_focus != next.shield_focus {
        if let Some(label) = &next.shield_focus {
            return Some(IntentChange {
                kind: IntentKind::ShieldArcFocused,
                subject: Some(label.clone()),
            });
        }
    }

    // 6. A new manoeuvre leg. The authored state name is the subject — the
    //    doctrine's own vocabulary, not one invented here.
    if prev.manoeuvre != next.manoeuvre {
        if let Some(label) = &next.manoeuvre {
            return Some(IntentChange {
                kind: IntentKind::ManoeuvreBegun,
                subject: Some(label.clone()),
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
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
}
