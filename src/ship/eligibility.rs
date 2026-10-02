//! Anonymous Station/rating accessibility eligibility (issue #1103).
//!
//! T1 (design: `accessibility-station-eligibility-contract`) evaluates whether a
//! complete Station surface at a required rating is compatible with a player's
//! PRIVATE assistance profile, and publishes only an anonymous eligible /
//! ineligible result. This module is the RUST source of truth for that rule —
//! the pure, Bevy-free evaluator the AC4 full-crew guarantee is proven against,
//! and the same rule the client mirrors from a projected table
//! (`ShipClientConfig::station_assist_gaps`, built by [`projected_assist_gaps`]).
//!
//! ## The rule
//!
//! A player's profile may request assistance (`ASSIST_REQUEST`) on one of the
//! four T1 assist-functions below. A station is **INELIGIBLE** for that player
//! iff the player requests assistance on some assist-function whose underlying
//! system the station would force them to operate MANUALLY at the required
//! rating — i.e. the station hosts a directly-operated system of that kind that
//! the rating does not automate. If the profile requests no assistance the
//! player is eligible everywhere.
//!
//! "Forced to operate manually" deliberately excludes two kinds of system that
//! never land on the seat holder as manual work:
//!   * `ai_only` systems (never human-operated — always assisted), and
//!   * `human_seeking` systems, which float to whichever human the ship's seek
//!     order finds and otherwise fall to AI, so the holder is never forced onto
//!     one.
//!
//! Both facts come from authored `[[system]]` config, so this stays pure config
//! evaluation with no hidden gameplay values.
//!
//! Assistance itself is DECLARED-BUT-INERT in T1 (no AI implemented). This module
//! only builds the eligibility seam: the mapping registry, the evaluator, the
//! projection the client needs, and the guarantee that every base hull keeps a
//! compatible seat.

use crate::ship::config::{ShipConfig, StationConfig};
use crate::ship::system_registry as kinds;
use std::collections::HashMap;

// ── Assist-function vocabulary ───────────────────────────────────────────────
//
// These ids MUST mirror `gui/accessibility-profile.js` `ASSISTANCE_FUNCTIONS`
// exactly — they are the shared machine vocabulary the client's profile keys
// onto and the projection is keyed by. They are a code-level id registry (like
// the system-kind constants), NOT tunable gameplay values.

/// Keeping the ship on course at the Helm.
pub const ASSIST_HELM_COURSE_KEEPING: &str = "helm.course-keeping";
/// Choosing a weapons target at Tactical.
pub const ASSIST_TACTICAL_TARGET_SELECTION: &str = "tactical.target-selection";
/// Triaging sensor contacts at Sensors/Science.
pub const ASSIST_SENSORS_CONTACT_TRIAGE: &str = "sensors.contact-triage";
/// Timing dialogue responses at Comms.
pub const ASSIST_COMMS_DIALOGUE_TIMING: &str = "comms.dialogue-timing";

/// The T1 assist-function vocabulary, in a stable order. Mirrors the client's
/// `ASSISTANCE_FUNCTIONS`.
pub const ASSIST_FUNCTIONS: &[&str] = &[
    ASSIST_HELM_COURSE_KEEPING,
    ASSIST_TACTICAL_TARGET_SELECTION,
    ASSIST_SENSORS_CONTACT_TRIAGE,
    ASSIST_COMMS_DIALOGUE_TIMING,
];

/// Map an assist-function id to the system KIND whose automation satisfies it.
///
/// The explicit table — not a hull tunable — the whole rule turns on. An
/// unknown id maps to `None` and never affects eligibility (forward-compatible
/// with a client that names a function this build does not know).
fn assist_system_kind(func: &str) -> Option<&'static str> {
    match func {
        ASSIST_HELM_COURSE_KEEPING => Some(kinds::HELM_STEERING_KIND),
        ASSIST_TACTICAL_TARGET_SELECTION => Some(kinds::TACTICAL_RADAR_KIND),
        ASSIST_SENSORS_CONTACT_TRIAGE => Some(kinds::SENSOR_RADAR_KIND),
        ASSIST_COMMS_DIALOGUE_TIMING => Some(kinds::COMMS_KIND),
        _ => None,
    }
}

// ── The evaluator ────────────────────────────────────────────────────────────

/// Would this station force its holder to operate `func`'s system MANUALLY at
/// `required_rating`? See the module rule for the exact meaning.
fn is_gap(station: &StationConfig, required_rating: &str, func: &str, ship: &ShipConfig) -> bool {
    let Some(kind) = assist_system_kind(func) else {
        return false;
    };
    // Unknown / Backfill rating ⇒ treat as fully assisted (permissive), matching
    // the projection's missing-entry default and the session side-map's DEFAULT
    // TRUE. `Backfill` automates every system anyway, so this is also correct.
    let Some(rating) = station.ratings.iter().find(|r| r.name == required_rating) else {
        return false;
    };
    ship.systems.iter().any(|s| {
        s.station.as_ref() == Some(&station.id)
            && s.kind == kind
            && !s.ai_only
            && !s.human_seeking
            && !rating.automated_systems.contains(&s.id)
    })
}

/// Is the complete `station` surface at `required_rating` compatible with a
/// profile that requests assistance on `requested_functions`?
///
/// The canonical evaluator (AC1/AC4 source of truth). Eligible unless some
/// requested function is a manual gap on this station at this rating.
pub fn station_eligible(
    station: &StationConfig,
    required_rating: &str,
    requested_functions: &[&str],
    ship: &ShipConfig,
) -> bool {
    !requested_functions
        .iter()
        .any(|func| is_gap(station, required_rating, func, ship))
}

/// The requested assist-functions that make this station ineligible at
/// `required_rating` — the functional-reason variant, for the LOCAL player's
/// private explanation only. Empty iff the player is eligible.
pub fn ineligible_functions<'f>(
    station: &StationConfig,
    required_rating: &str,
    requested_functions: &'f [&'f str],
    ship: &ShipConfig,
) -> Vec<&'f str> {
    requested_functions
        .iter()
        .copied()
        .filter(|func| is_gap(station, required_rating, func, ship))
        .collect()
}

/// Project, per rating, the assist-functions this station would force manual —
/// the anonymous, hull-derived table the client needs to run the SAME rule
/// without the profile ever leaving the device. Only ratings with a non-empty
/// gap set are included (a missing entry means "no gaps ⇒ eligible", the
/// permissive default the client relies on).
///
/// This is projected CONFIG (a property of the hull and rating), never the
/// profile: it says nothing about any player.
pub fn projected_assist_gaps(
    station: &StationConfig,
    ship: &ShipConfig,
) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for rating in &station.ratings {
        let gaps: Vec<String> = ASSIST_FUNCTIONS
            .iter()
            .copied()
            .filter(|func| is_gap(station, &rating.name, func, ship))
            .map(str::to_string)
            .collect();
        if !gaps.is_empty() {
            out.insert(rating.name.clone(), gaps);
        }
    }
    out
}

#[cfg(test)]
#[path = "eligibility_tests.rs"]
mod tests;
