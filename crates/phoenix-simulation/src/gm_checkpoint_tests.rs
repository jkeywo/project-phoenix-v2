use super::*;
use crate::command_admission::log::HostSlot;
use crate::core::messages::StationId;
use crate::lockstep::{FleetRoster, FleetShip};
use crate::save_slots::{MetadataStatus, SaveSlotKind};
use crate::snapshot::BootIdentity;

const LIVE_WORLD: &str = "assets/worlds/duel.toml";
const CRUISER: &str = "assets/entities/alliance_cruiser.toml";
const DESTROYER: &str = "assets/entities/alliance_destroyer.toml";

fn ship(slot: u32, hull: &str, crew: &[&str]) -> FleetShip {
    FleetShip {
        host: HostSlot(slot),
        ship_path: Some(hull.to_string()),
        authored_slot_id: None,
        crew: crew
            .iter()
            .map(|station| (StationId((*station).to_string()), "Std".to_string()))
            .collect(),
    }
}

fn roster(ships: Vec<FleetShip>) -> FleetRoster {
    FleetRoster::new(ships, HostSlot(1))
}

fn live() -> LiveSeating {
    LiveSeating::from_roster(
        LIVE_WORLD,
        &roster(vec![
            ship(1, CRUISER, &["helm", "tactical"]),
            ship(2, DESTROYER, &["helm"]),
        ]),
        Some(CRUISER),
    )
}

fn versions() -> vellum_save::Versions {
    vellum_save::Versions {
        format: 20,
        rules: "rules".to_string(),
        content: 0x1234_5678,
    }
}

fn entry(slot_id: &str, scenario: &str, fleet: Option<FleetRoster>) -> SaveSlotEntry {
    SaveSlotEntry {
        slot_id: slot_id.to_string(),
        kind: SaveSlotKind::Manual,
        display_name: format!("bookmark {slot_id}"),
        metadata: MetadataStatus::Present,
        record: Some(SaveRecordSummary {
            scenario: scenario.to_string(),
            seed: 7,
            capture_tick: 4242,
            boot_identity: fleet.map(|fleet| BootIdentity {
                selected_ship: "assets/entities/alliance_cruiser.toml".to_string(),
                fleet,
                game_start_entity_uuids: Vec::new(),
            }),
            versions: versions(),
        }),
        start: StartState::Ready,
    }
}

fn matching_fleet() -> FleetRoster {
    // Deliberately different CREW from the live roster: the candidate's own
    // seating is irrelevant, and a model that compared it would fail here.
    roster(vec![
        ship(1, "assets/entities/alliance_cruiser.toml", &["engineering"]),
        ship(2, "assets/entities/alliance_destroyer.toml", &[]),
    ])
}

#[test]
fn a_same_scenario_same_fleet_save_is_an_eligible_candidate() {
    let answer = preflight(&live(), &entry("a", LIVE_WORLD, Some(matching_fleet())));
    assert_eq!(
        answer,
        CandidatePreflight {
            eligible: true,
            blocks: Vec::new()
        }
    );
}

#[test]
fn the_candidates_own_seating_never_leaves_the_model() {
    // The candidate rosters a Station nobody holds live and leaves live
    // Stations empty. Eligibility must be unaffected, and the projected
    // fleet must carry no crew at all — that is what stops a restore from
    // importing old seat ownership.
    let candidate = entry("a", LIVE_WORLD, Some(matching_fleet()));
    assert!(preflight(&live(), &candidate).eligible);
    let fleet = CandidateFleet::from_record(candidate.record.as_ref().unwrap()).unwrap();
    let json = serde_json::to_value(&fleet.ships[0]).unwrap();
    let mut keys: Vec<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["hull", "slot"]);
}

#[test]
fn a_different_scenario_is_refused_with_both_world_paths() {
    let answer = preflight(
        &live(),
        &entry(
            "a",
            "assets/worlds/combat_test.toml",
            Some(matching_fleet()),
        ),
    );
    assert!(!answer.eligible);
    assert_eq!(
        answer.blocks,
        vec![CandidateBlock::ScenarioDiffers {
            candidate: "assets/worlds/combat_test.toml".to_string(),
            live: LIVE_WORLD.to_string(),
        }]
    );
}

#[test]
fn a_live_ship_slot_the_candidate_lacks_names_the_stations_at_stake() {
    let candidate = entry(
        "a",
        LIVE_WORLD,
        Some(roster(vec![ship(
            1,
            "assets/entities/alliance_cruiser.toml",
            &[],
        )])),
    );
    let answer = preflight(&live(), &candidate);
    assert_eq!(
        answer.blocks,
        vec![CandidateBlock::MissingShip {
            slot: 2,
            stations: vec!["helm".to_string()]
        }]
    );
}

#[test]
fn a_different_hull_at_a_live_slot_is_refused() {
    let candidate = entry(
        "a",
        LIVE_WORLD,
        Some(roster(vec![
            ship(1, "assets/entities/alliance_cruiser.toml", &[]),
            ship(2, "assets/entities/alliance_cruiser.toml", &[]),
        ])),
    );
    let answer = preflight(&live(), &candidate);
    assert_eq!(
        answer.blocks,
        vec![CandidateBlock::HullDiffers {
            slot: 2,
            candidate: Some("assets/entities/alliance_cruiser.toml".to_string()),
            live: Some("assets/entities/alliance_destroyer.toml".to_string()),
            stations: vec!["helm".to_string()],
        }]
    );
}

#[test]
fn every_applicable_block_is_reported_not_only_the_first() {
    let candidate = entry(
        "a",
        "assets/worlds/combat_test.toml",
        Some(roster(vec![ship(
            1,
            "assets/entities/alliance_destroyer.toml",
            &[],
        )])),
    );
    let answer = preflight(&live(), &candidate);
    assert_eq!(answer.blocks.len(), 3, "{:?}", answer.blocks);
    assert!(matches!(
        answer.blocks[0],
        CandidateBlock::ScenarioDiffers { .. }
    ));
    assert!(matches!(
        answer.blocks[1],
        CandidateBlock::HullDiffers { slot: 1, .. }
    ));
    assert!(matches!(
        answer.blocks[2],
        CandidateBlock::MissingShip { slot: 2, .. }
    ));
}

#[test]
fn extra_candidate_ships_do_not_block_representing_the_live_seating() {
    let candidate = entry(
        "a",
        LIVE_WORLD,
        Some(roster(vec![
            ship(1, "assets/entities/alliance_cruiser.toml", &[]),
            ship(2, "assets/entities/alliance_destroyer.toml", &[]),
            ship(3, "assets/entities/alliance_destroyer.toml", &[]),
        ])),
    );
    assert!(preflight(&live(), &candidate).eligible);
}

#[test]
fn the_existing_version_gate_is_folded_in_rather_than_re_answered() {
    for (start, expected) in [
        (
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Format {
                    stored: 1,
                    current: 20,
                },
            )),
            CandidateBlock::FormatMoved,
        ),
        (
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Rules {
                    stored: "a".into(),
                    current: "b".into(),
                },
            )),
            CandidateBlock::RulesMoved,
        ),
        (
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Content {
                    stored: 1,
                    current: 2,
                },
            )),
            CandidateBlock::ContentMoved,
        ),
        (
            StartState::ContentDeferred,
            CandidateBlock::ContentUnverified,
        ),
    ] {
        let mut candidate = entry("a", LIVE_WORLD, Some(matching_fleet()));
        candidate.start = start;
        let answer = preflight(&live(), &candidate);
        assert!(!answer.eligible);
        assert_eq!(answer.blocks, vec![expected]);
    }
}

#[test]
fn a_row_with_no_readable_record_is_unreadable_and_not_a_fleet_complaint() {
    let mut candidate = entry("a", LIVE_WORLD, Some(matching_fleet()));
    candidate.record = None;
    candidate.start = StartState::Refused(crate::snapshot::LoadRefusal::Empty);
    assert_eq!(
        preflight(&live(), &candidate).blocks,
        vec![CandidateBlock::Unreadable]
    );
}

#[test]
fn a_parsed_row_with_no_boot_identity_says_so_instead_of_claiming_a_match() {
    let candidate = entry("a", LIVE_WORLD, None);
    assert_eq!(
        preflight(&live(), &candidate).blocks,
        vec![CandidateBlock::NoFleetRecord]
    );
}

#[test]
fn a_capture_tick_exists_only_once_the_row_is_really_in_the_catalogue() {
    let entries = vec![entry("kept", LIVE_WORLD, Some(matching_fleet()))];
    assert_eq!(confirmed_checkpoint(&entries, "never-written"), None);
    assert_eq!(
        confirmed_checkpoint(&entries, "kept"),
        Some(ConfirmedCheckpoint {
            slot_id: "kept".to_string(),
            display_name: "bookmark kept".to_string(),
            scenario: LIVE_WORLD.to_string(),
            capture_tick: 4242,
        })
    );
}

#[test]
fn a_present_but_unreadable_row_is_not_a_confirmed_checkpoint() {
    let mut damaged = entry("kept", LIVE_WORLD, Some(matching_fleet()));
    damaged.record = None;
    assert_eq!(confirmed_checkpoint(&[damaged], "kept"), None);
}

/// A solo session's catalogue row: the default one-ship roster, whose
/// `ship_path` is the "whatever this host selected" placeholder, plus the
/// concrete hull that host really booted on.
fn solo_capture(hull: &str) -> SaveSlotEntry {
    let mut candidate = entry("a", LIVE_WORLD, Some(FleetRoster::default()));
    candidate
        .record
        .as_mut()
        .unwrap()
        .boot_identity
        .as_mut()
        .unwrap()
        .selected_ship = hull.to_string();
    candidate
}

#[test]
fn a_solo_roster_takes_its_hull_from_the_hull_this_peer_actually_booted() {
    let seating = LiveSeating::from_roster(LIVE_WORLD, &FleetRoster::default(), Some(DESTROYER));
    assert_eq!(
        seating.ships,
        vec![SeatedShip {
            slot: 0,
            hull: Some(DESTROYER.to_string()),
            stations: Vec::new()
        }]
    );
}

#[test]
fn a_solo_capture_taken_on_a_different_hull_is_refused() {
    // Both rosters are the placeholder-carrying default: only the resolved
    // hulls differ. Comparing the placeholders would call this a match.
    let seating = LiveSeating::from_roster(LIVE_WORLD, &FleetRoster::default(), Some(DESTROYER));
    let answer = preflight(&seating, &solo_capture(CRUISER));
    assert!(!answer.eligible);
    assert_eq!(
        answer.blocks,
        vec![CandidateBlock::HullDiffers {
            slot: 0,
            candidate: Some(CRUISER.to_string()),
            live: Some(DESTROYER.to_string()),
            stations: Vec::new(),
        }]
    );
}

#[test]
fn a_solo_capture_taken_on_the_same_hull_is_still_eligible() {
    let seating = LiveSeating::from_roster(LIVE_WORLD, &FleetRoster::default(), Some(DESTROYER));
    assert!(preflight(&seating, &solo_capture(DESTROYER)).eligible);
}

#[test]
fn an_unresolved_hull_blocks_instead_of_reading_as_agreement() {
    // This peer cannot say what it is flying. That is not evidence that the
    // save flies the same thing.
    let seating = LiveSeating::from_roster(LIVE_WORLD, &FleetRoster::default(), None);
    assert_eq!(seating.ships[0].hull, None);
    let answer = preflight(&seating, &solo_capture(CRUISER));
    assert!(!answer.eligible);
    assert_eq!(
        answer.blocks,
        vec![CandidateBlock::HullUnknown {
            slot: 0,
            stations: Vec::new()
        }]
    );

    // ...and equally when the unresolvable side is the candidate: a remote
    // slot the save recorded with no hull of its own.
    let live_now = LiveSeating::from_roster(
        LIVE_WORLD,
        &roster(vec![ship(1, CRUISER, &["helm"])]),
        Some(CRUISER),
    );
    let mut candidate = entry("a", LIVE_WORLD, Some(matching_fleet()));
    let fleet = &mut candidate
        .record
        .as_mut()
        .unwrap()
        .boot_identity
        .as_mut()
        .unwrap()
        .fleet;
    // Slot 1 is the candidate roster's own local slot; slot 2 is not, so the
    // capture's `selected_ship` cannot speak for it.
    *fleet = FleetRoster::new(
        vec![
            FleetShip {
                host: HostSlot(1),
                ship_path: None,
                authored_slot_id: None,
                crew: Vec::new(),
            },
            ship(2, DESTROYER, &[]),
        ],
        HostSlot(2),
    );
    assert_eq!(
        preflight(&live_now, &candidate).blocks,
        vec![CandidateBlock::HullUnknown {
            slot: 1,
            stations: vec!["helm".to_string()]
        }]
    );
}
