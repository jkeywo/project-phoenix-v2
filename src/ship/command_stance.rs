//! Pure Command-stance resolution (issue #1107).
//!
//! Command directs one AI-controlled proving Station by selecting a stance from
//! that Station's authored catalogue. This module owns the Bevy-free logic that
//! decides which stances a human may select, which alert-neutral stance is the
//! current default, how the selection follows an alert-level change, and which
//! posture fact the selection seeds for the directed Station's ordinary AI
//! hosts.
//!
//! See `docs/gdd/mechanics/command-and-crew-control.md`. The Bevy adapter is
//! `crate::console::command::server`, a sibling — nothing Bevy is imported here.

use crate::ship::config::{StanceKind, StationStanceConfig};

/// The id of the alert-neutral fallback stance for the given alert level.
///
/// `red_alert == true` selects the `high_alert_neutral` stance, otherwise the
/// `normal_alert_neutral`. A well-formed catalogue authors exactly one of each
/// (enforced at load by `ship::config::validate`), so the first match is the
/// answer; a hand-built catalogue missing one returns `None`.
pub fn neutral_stance_for_alert(
    catalogue: &[StationStanceConfig],
    red_alert: bool,
) -> Option<&str> {
    let wanted = if red_alert {
        StanceKind::HighAlertNeutral
    } else {
        StanceKind::NormalAlertNeutral
    };
    catalogue
        .iter()
        .find(|stance| stance.kind == wanted)
        .map(|stance| stance.id.as_str())
}

/// Look up a stance by id.
pub fn stance_by_id<'a>(
    catalogue: &'a [StationStanceConfig],
    stance_id: &str,
) -> Option<&'a StationStanceConfig> {
    catalogue.iter().find(|stance| stance.id == stance_id)
}

/// Whether a human Command operator may select `stance_id` for this station.
///
/// The authored catalogue IS the whole selectable vocabulary (criterion 3):
/// standard, normal-alert-neutral and high-alert-neutral stances only. An id
/// that names no authored stance is refused — Command "does not invent orders
/// outside the authored vocabulary".
pub fn is_selectable(catalogue: &[StationStanceConfig], stance_id: &str) -> bool {
    stance_by_id(catalogue, stance_id).is_some()
}

/// The posture fact a selected stance seeds for the directed Station's AI hosts.
///
/// `true` behaves as "at high alert" — the migrated Red Alert branch fires;
/// `false` as "stood down". With no explicit selection the posture TRACKS the
/// ship's own red alert, which is exactly the pre-#1107 behaviour and keeps a
/// hull nobody directs byte-identical.
pub fn effective_high_alert(
    catalogue: &[StationStanceConfig],
    selected: Option<&str>,
    red_alert: bool,
) -> bool {
    match selected.and_then(|id| stance_by_id(catalogue, id)) {
        Some(stance) => stance.high_alert,
        None => red_alert,
    }
}

/// The selection after an alert-level change (criterion 5).
///
/// Changing alert level switches between the two neutral stances only when the
/// station is already in one of them; it never overwrites a deliberately
/// selected `standard` stance. A station with no current selection adopts the
/// neutral stance for the new level.
pub fn selection_after_alert_change(
    catalogue: &[StationStanceConfig],
    current: Option<&str>,
    red_alert: bool,
) -> Option<String> {
    let is_neutral = current
        .and_then(|id| stance_by_id(catalogue, id))
        .is_some_and(|stance| {
            matches!(
                stance.kind,
                StanceKind::NormalAlertNeutral | StanceKind::HighAlertNeutral
            )
        });
    match current {
        // In a neutral stance → follow the alert to the other neutral.
        Some(_) if is_neutral => neutral_stance_for_alert(catalogue, red_alert).map(str::to_string),
        // In an explicit standard stance → untouched.
        Some(id) => Some(id.to_string()),
        // Nothing selected yet → adopt the level's neutral.
        None => neutral_stance_for_alert(catalogue, red_alert).map(str::to_string),
    }
}

/// A stored selection reconciled against the current authored catalogue
/// (issue #1108 criterion 4).
///
/// The catalogue is the sole membership authority — the SAME [`is_selectable`]
/// seam a human order is admitted through, and the one #1110's objective
/// stances extend when a stance enters or leaves the catalogue. A stored id the
/// catalogue still authors is kept; one that has left it returns `None`, so the
/// caller drops the stored entry and the directed Station falls back
/// deterministically to the alert-neutral tracking default — visibly removed
/// from the console readout on the next publish.
pub fn reconcile_selection(catalogue: &[StationStanceConfig], current: &str) -> Option<String> {
    is_selectable(catalogue, current).then(|| current.to_string())
}

/// The selection after the Command station loses its human operator (lifecycle:
/// "reset-to-neutral vs persist-behind-human").
///
/// A standard stance that authored `persist_behind_human = false` (the default)
/// resets to the alert-appropriate neutral so an old aggressive order does not
/// silently resume; one that authored `true` is kept. Neutral stances are their
/// own reset target and are always kept.
pub fn selection_after_human_lost(
    catalogue: &[StationStanceConfig],
    current: Option<&str>,
    red_alert: bool,
) -> Option<String> {
    match current.and_then(|id| stance_by_id(catalogue, id)) {
        Some(stance) => match stance.kind {
            StanceKind::NormalAlertNeutral | StanceKind::HighAlertNeutral => {
                Some(stance.id.clone())
            }
            StanceKind::Standard if stance.persist_behind_human => Some(stance.id.clone()),
            StanceKind::Standard => {
                neutral_stance_for_alert(catalogue, red_alert).map(str::to_string)
            }
        },
        None => neutral_stance_for_alert(catalogue, red_alert).map(str::to_string),
    }
}

/// The current, tick-derived ship knowledge an AI-operated Command seat decides
/// from (issue #1109).
///
/// Deliberately minimal and deterministic: the ship's own Red Alert level, which
/// the ordinary Captain AI already drives off threat. No wall-clock and no RNG,
/// so [`select_stance`] is a pure function of the catalogue and this snapshot —
/// the AC5 repeatable-selection backbone. Threat/contact facts could extend it
/// later without changing the seam.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandKnowledge {
    /// Whether the ship is at Red Alert this tick.
    pub red_alert: bool,
}

/// The single `standard` stance a designer flagged (`ai_engaged`) as the posture
/// an AI-operated Command seat adopts at high alert (issue #1109), if the
/// catalogue authors one.
///
/// The catalogue is the sole authority for the choice — never a hard-coded id in
/// Rust. `ship::config::validate` guarantees at most one, and only on a
/// `standard` stance, so the first match is the answer.
pub fn ai_engaged_stance(catalogue: &[StationStanceConfig]) -> Option<&str> {
    catalogue
        .iter()
        .find(|stance| stance.ai_engaged)
        .map(|stance| stance.id.as_str())
}

/// The stance an AI-operated Command seat selects for its directed Station, from
/// EXACTLY the authored catalogue a human Command operator selects from
/// (issue #1109).
///
/// Pure and deterministic (AC5): at high alert it adopts the authored
/// `ai_engaged` standard stance when the catalogue still authors one, otherwise
/// it tracks the alert-appropriate neutral. Every branch resolves through the
/// same [`is_selectable`] / [`neutral_stance_for_alert`] seams a human order
/// passes, so it can NEVER return an id the catalogue does not author — that is
/// the catalogue-parity guarantee (AC2/AC3) as a property of the function rather
/// than of caller discipline. `None` only when the catalogue is malformed
/// (missing its alert-neutral), which `validate` rejects at load.
pub fn select_stance(catalogue: &[StationStanceConfig], facts: CommandKnowledge) -> Option<String> {
    if facts.red_alert {
        if let Some(engaged) = ai_engaged_stance(catalogue) {
            if is_selectable(catalogue, engaged) {
                return Some(engaged.to_string());
            }
        }
    }
    neutral_stance_for_alert(catalogue, facts.red_alert).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Vec<StationStanceConfig> {
        vec![
            StationStanceConfig {
                id: "weapons-free".into(),
                label: String::new(),
                kind: StanceKind::Standard,
                high_alert: true,
                persist_behind_human: true,
                ai_engaged: true,
            },
            StationStanceConfig {
                id: "hold".into(),
                label: String::new(),
                kind: StanceKind::Standard,
                high_alert: false,
                persist_behind_human: false,
                ai_engaged: false,
            },
            StationStanceConfig {
                id: "normal".into(),
                label: String::new(),
                kind: StanceKind::NormalAlertNeutral,
                high_alert: false,
                persist_behind_human: false,
                ai_engaged: false,
            },
            StationStanceConfig {
                id: "high".into(),
                label: String::new(),
                kind: StanceKind::HighAlertNeutral,
                high_alert: true,
                persist_behind_human: false,
                ai_engaged: false,
            },
        ]
    }

    #[test]
    fn neutral_tracks_the_alert_level() {
        let c = catalogue();
        assert_eq!(neutral_stance_for_alert(&c, false), Some("normal"));
        assert_eq!(neutral_stance_for_alert(&c, true), Some("high"));
    }

    #[test]
    fn only_authored_stances_are_selectable() {
        let c = catalogue();
        assert!(is_selectable(&c, "weapons-free"));
        assert!(is_selectable(&c, "normal"));
        assert!(!is_selectable(&c, "invented-order"));
    }

    #[test]
    fn no_selection_tracks_the_ships_own_alert() {
        let c = catalogue();
        // This is the byte-identical default: posture == red_alert.
        assert!(!effective_high_alert(&c, None, false));
        assert!(effective_high_alert(&c, None, true));
    }

    #[test]
    fn a_standard_stance_seeds_its_authored_posture() {
        let c = catalogue();
        // "hold" forces stood-down even at red alert; "weapons-free" forces
        // high alert even when the ship is not.
        assert!(!effective_high_alert(&c, Some("hold"), true));
        assert!(effective_high_alert(&c, Some("weapons-free"), false));
    }

    #[test]
    fn alert_change_switches_neutral_to_neutral() {
        let c = catalogue();
        assert_eq!(
            selection_after_alert_change(&c, Some("normal"), true).as_deref(),
            Some("high"),
        );
        assert_eq!(
            selection_after_alert_change(&c, Some("high"), false).as_deref(),
            Some("normal"),
        );
    }

    #[test]
    fn alert_change_never_overwrites_a_standard_stance() {
        let c = catalogue();
        assert_eq!(
            selection_after_alert_change(&c, Some("weapons-free"), true).as_deref(),
            Some("weapons-free"),
        );
        assert_eq!(
            selection_after_alert_change(&c, Some("hold"), false).as_deref(),
            Some("hold"),
        );
    }

    #[test]
    fn absent_selection_adopts_the_current_neutral() {
        let c = catalogue();
        assert_eq!(
            selection_after_alert_change(&c, None, true).as_deref(),
            Some("high"),
        );
    }

    #[test]
    fn reconcile_keeps_authored_ids_and_drops_vanished_ones() {
        // Criterion 4: the catalogue is the single membership authority. An id
        // still authored survives; one no longer present falls out so the
        // caller clears it back to the alert-neutral tracking default.
        let c = catalogue();
        assert_eq!(
            reconcile_selection(&c, "weapons-free").as_deref(),
            Some("weapons-free"),
        );
        assert_eq!(reconcile_selection(&c, "normal").as_deref(), Some("normal"));
        // A stance that has left the catalogue (e.g. an objective stance whose
        // objective ended, #1110) is dropped.
        assert_eq!(reconcile_selection(&c, "objective-escort"), None);
    }

    #[test]
    fn human_handoff_resets_only_non_persistent_standard_stances() {
        let c = catalogue();
        // Non-persistent "hold" resets to the level's neutral…
        assert_eq!(
            selection_after_human_lost(&c, Some("hold"), true).as_deref(),
            Some("high"),
        );
        // …but a persist-behind-human standard order is kept…
        assert_eq!(
            selection_after_human_lost(&c, Some("weapons-free"), false).as_deref(),
            Some("weapons-free"),
        );
        // …and a neutral is always kept.
        assert_eq!(
            selection_after_human_lost(&c, Some("normal"), false).as_deref(),
            Some("normal"),
        );
    }

    // ── AI Command selection (issue #1109) ──────────────────────────────────

    #[test]
    fn ai_command_selects_only_authored_stances() {
        // AC2/AC3 catalogue parity: whatever the ship knowledge, the pick is
        // always an id the catalogue authors — never invented. Exhaustively over
        // both alert levels.
        let c = catalogue();
        for red_alert in [false, true] {
            let picked = select_stance(&c, CommandKnowledge { red_alert })
                .expect("a well-formed catalogue always resolves a stance");
            assert!(
                is_selectable(&c, &picked),
                "AI Command must only ever select an authored stance; picked {picked:?}"
            );
        }
    }

    #[test]
    fn ai_command_adopts_the_engaged_stance_at_high_alert() {
        // At Red Alert the AI adopts the authored `ai_engaged` posture…
        let c = catalogue();
        assert_eq!(
            select_stance(&c, CommandKnowledge { red_alert: true }).as_deref(),
            Some("weapons-free"),
        );
        // …and stands down to the normal-alert neutral otherwise.
        assert_eq!(
            select_stance(&c, CommandKnowledge { red_alert: false }).as_deref(),
            Some("normal"),
        );
    }

    #[test]
    fn ai_command_selection_is_repeatable_for_the_same_knowledge() {
        // AC5 repeatability: a pure function of catalogue + knowledge, so the
        // same inputs yield the same id every time.
        let c = catalogue();
        for red_alert in [false, true] {
            let facts = CommandKnowledge { red_alert };
            let first = select_stance(&c, facts);
            for _ in 0..8 {
                assert_eq!(select_stance(&c, facts), first);
            }
        }
    }

    #[test]
    fn ai_command_without_an_engaged_stance_tracks_the_neutral() {
        // A catalogue that flags no `ai_engaged` posture falls back to the
        // alert-appropriate neutral at every level — the byte-identical tracking
        // default, never an invented escalation.
        let mut c = catalogue();
        for stance in &mut c {
            stance.ai_engaged = false;
        }
        assert_eq!(
            select_stance(&c, CommandKnowledge { red_alert: true }).as_deref(),
            Some("high"),
        );
        assert_eq!(
            select_stance(&c, CommandKnowledge { red_alert: false }).as_deref(),
            Some("normal"),
        );
    }
}
