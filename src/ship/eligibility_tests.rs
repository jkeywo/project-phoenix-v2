use super::*;
use crate::core::messages::{StationId, SystemId};
use crate::ship::config::{StationConfig, StationRatingConfig, SystemInstanceConfig};

fn rating(name: &str, automated: &[&str]) -> StationRatingConfig {
    StationRatingConfig {
        detailed_systems: None,
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
/// filled), with a profile requesting ALL T1 assist functions and a simple
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
        let config = crate::entities::include_resolve::load_entity_config(&key)
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
            "{stem}: no (station, rating) is eligible with all T1 assist \
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
