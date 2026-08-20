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
            },
            StationStanceConfig {
                id: "hold".into(),
                label: String::new(),
                kind: StanceKind::Standard,
                high_alert: false,
                persist_behind_human: false,
            },
            StationStanceConfig {
                id: "normal".into(),
                label: String::new(),
                kind: StanceKind::NormalAlertNeutral,
                high_alert: false,
                persist_behind_human: false,
            },
            StationStanceConfig {
                id: "high".into(),
                label: String::new(),
                kind: StanceKind::HighAlertNeutral,
                high_alert: true,
                persist_behind_human: false,
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
}
