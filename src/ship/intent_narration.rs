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

use crate::core::messages::IntentKind;

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
#[path = "intent_narration_tests.rs"]
mod tests;
