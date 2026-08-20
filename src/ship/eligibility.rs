//! Anonymous Station/rating accessibility eligibility (issue #1103).
//!
//! A2 (design: `accessibility-station-eligibility-contract`) evaluates whether a
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
//! four A2 assist-functions below. A station is **INELIGIBLE** for that player
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
//! Assistance itself is DECLARED-BUT-INERT in A2 (no AI implemented). This module
//! only builds the eligibility seam: the mapping registry, the evaluator, the
//! projection the client needs, and the guarantee that every base hull keeps a
//! compatible seat.

use crate::ship::config::{ShipConfig, StationConfig};
use crate::system_registry as kinds;
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

/// The A2 assist-function vocabulary, in a stable order. Mirrors the client's
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
mod tests {
    use super::*;
    use crate::messages::{StationId, SystemId};
    use crate::ship::config::{StationConfig, StationRatingConfig, SystemInstanceConfig};

    fn rating(name: &str, automated: &[&str]) -> StationRatingConfig {
        StationRatingConfig {
            name: name.into(),
            automated_systems: automated.iter().map(|s| SystemId((*s).into())).collect(),
            ai_tuning: None,
        }
    }

    fn station(id: &str, ratings: Vec<StationRatingConfig>) -> StationConfig {
        StationConfig {
            id: StationId(id.into()),
            name: id.into(),
            description: String::new(),
            rank: String::new(),
            short_code: String::new(),
            ratings,
            console: None,
            manual_overview: None,
            tutorials: vec![],
            human_seeking: false,
            host_order: vec![],
            visiting_rating: None,
            auxiliary: false,
            command_target: None,
            stances: vec![],
        }
    }

    fn system(id: &str, kind: &str, station: &str) -> SystemInstanceConfig {
        SystemInstanceConfig {
            id: SystemId(id.into()),
            kind: kind.into(),
            station: Some(StationId(station.into())),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        }
    }

    /// A helm station with a `helm-steering` (course-keeping) system, plus the
    /// requested rating.
    fn helm_ship(automated: &[&str]) -> (ShipConfig, StationConfig) {
        let st = station("helm", vec![rating("Std", automated)]);
        let ship = ShipConfig {
            stations: vec![st.clone()],
            systems: vec![system("helm-steering", kinds::HELM_STEERING_KIND, "helm")],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        };
        (ship, st)
    }

    #[test]
    fn no_requested_assistance_is_eligible_everywhere() {
        let (ship, st) = helm_ship(&[]);
        assert!(station_eligible(&st, "Std", &[], &ship));
    }

    #[test]
    fn requesting_an_unautomated_present_function_is_ineligible() {
        // helm-steering present, NOT automated at Std → requesting course-keeping
        // forces manual operation → ineligible.
        let (ship, st) = helm_ship(&[]);
        assert!(!station_eligible(
            &st,
            "Std",
            &[ASSIST_HELM_COURSE_KEEPING],
            &ship
        ));
        assert_eq!(
            ineligible_functions(&st, "Std", &[ASSIST_HELM_COURSE_KEEPING], &ship),
            vec![ASSIST_HELM_COURSE_KEEPING]
        );
    }

    #[test]
    fn requesting_an_automated_function_is_eligible() {
        // Rating automates helm-steering → the AI keeps course → eligible.
        let (ship, st) = helm_ship(&["helm-steering"]);
        assert!(station_eligible(
            &st,
            "Std",
            &[ASSIST_HELM_COURSE_KEEPING],
            &ship
        ));
        assert!(ineligible_functions(&st, "Std", &[ASSIST_HELM_COURSE_KEEPING], &ship).is_empty());
    }

    #[test]
    fn a_function_whose_system_is_absent_is_no_concern() {
        // The helm station hosts no comms system, so requesting dialogue-timing
        // does not make it ineligible.
        let (ship, st) = helm_ship(&[]);
        assert!(station_eligible(
            &st,
            "Std",
            &[ASSIST_COMMS_DIALOGUE_TIMING],
            &ship
        ));
    }

    #[test]
    fn human_seeking_systems_never_force_manual_operation() {
        // A comms system that seeks a human (never forced on this holder) does not
        // make the seat ineligible even when the rating does not automate it.
        let st = station("captain", vec![rating("Std", &[])]);
        let mut comms = system("comms", kinds::COMMS_KIND, "captain");
        comms.human_seeking = true;
        let ship = ShipConfig {
            stations: vec![st.clone()],
            systems: vec![comms],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        };
        assert!(station_eligible(
            &st,
            "Std",
            &[ASSIST_COMMS_DIALOGUE_TIMING],
            &ship
        ));
    }

    #[test]
    fn ai_only_systems_never_force_manual_operation() {
        let st = station("tactical", vec![rating("Std", &[])]);
        let mut radar = system("tactical-radar", kinds::TACTICAL_RADAR_KIND, "tactical");
        radar.ai_only = true;
        let ship = ShipConfig {
            stations: vec![st.clone()],
            systems: vec![radar],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        };
        assert!(station_eligible(
            &st,
            "Std",
            &[ASSIST_TACTICAL_TARGET_SELECTION],
            &ship
        ));
    }

    #[test]
    fn projection_lists_only_nonempty_rating_gaps() {
        // Std does not automate helm-steering (gap); Simplified does (no gap).
        let st = station(
            "helm",
            vec![rating("Std", &[]), rating("Simplified", &["helm-steering"])],
        );
        let ship = ShipConfig {
            stations: vec![st.clone()],
            systems: vec![system("helm-steering", kinds::HELM_STEERING_KIND, "helm")],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        };
        let gaps = projected_assist_gaps(&st, &ship);
        assert_eq!(
            gaps.get("Std").map(Vec::as_slice),
            Some([ASSIST_HELM_COURSE_KEEPING.to_string()].as_slice())
        );
        assert!(
            !gaps.contains_key("Simplified"),
            "automated rating has no gap"
        );
    }

    #[test]
    fn unknown_rating_is_permissively_eligible() {
        let (ship, st) = helm_ship(&[]);
        assert!(station_eligible(
            &st,
            "Backfill",
            &[ASSIST_HELM_COURSE_KEEPING],
            &ship
        ));
    }

    /// AC4: every base playable hull, at full crew (every non-auxiliary seat
    /// filled), with a profile requesting ALL A2 assist functions and a simple
    /// scenario (empty scenario detail-floor), retains AT LEAST ONE compatible
    /// (station, rating) combination.
    ///
    /// Mirrors `rating.rs::every_shipped_hull_boots_fully_ai_when_nobody_is_connected`
    /// — the same top-level hull-walk over `assets/entities/*.toml` through the
    /// include resolver, with a `checked_hulls >= N` floor so a walk that
    /// silently finds nothing fails instead of passing vacuously.
    #[test]
    fn every_base_playable_hull_keeps_a_compatible_seat_with_all_assists() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("assets/entities must be readable")
            .map(|e| e.expect("readable dir entry").path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        entries.sort();

        let mut checked_hulls = 0usize;
        for path in entries {
            let stem = path
                .file_stem()
                .expect("toml file has a stem")
                .to_string_lossy()
                .to_string();
            let key = path.to_string_lossy().replace('\\', "/");
            let config = crate::entity_includes::load_entity_config(&key)
                .unwrap_or_else(|e| panic!("{stem} must parse: {e}"));
            let Some(ship) = config.ship_config.as_ref() else {
                continue; // scenery / NPC-only: no stations to crew
            };
            // "Base playable hull" == one with at least one claimable (non-auxiliary)
            // seat. Full crew fills exactly those seats.
            if !ship.stations.iter().any(|s| !s.auxiliary) {
                continue;
            }
            checked_hulls += 1;

            // The stress profile: request EVERY assist function at once (the
            // "complete supported option set" of the contract).
            let compatible = ship
                .stations
                .iter()
                .filter(|station| !station.auxiliary)
                .any(|station| {
                    station
                        .ratings
                        .iter()
                        .any(|r| station_eligible(station, &r.name, ASSIST_FUNCTIONS, ship))
                });
            assert!(
                compatible,
                "{stem}: no (station, rating) is eligible with all A2 assist \
                 functions requested — the full-crew accessibility guarantee is \
                 violated for this base hull"
            );
        }

        // Matches the sibling ratchet in `rating.rs`
        // (`every_shipped_hull_boots_fully_ai_when_nobody_is_connected`): the
        // walk visits every top-level hull declaring `[[station]]`, so a loose
        // floor would let a hull silently drop out and still report the
        // guarantee green.
        assert!(
            checked_hulls >= 9,
            "expected the walk to find every base playable hull, got {checked_hulls}"
        );
    }
}
