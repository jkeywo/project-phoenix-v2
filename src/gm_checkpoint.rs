//! Named GM checkpoints and the shared candidate preflight model (issue #1445).
//!
//! Two things live here, and both are deliberately PURE.
//!
//! 1. [`confirmed_checkpoint`] — the read-back that turns a requested bookmark
//!    into a *confirmed* one. A GM asks for a named capture through the ordinary
//!    save machinery ([`crate::save_slots::CaptureSlot::Manual`],
//!    [`crate::save_slots_store::request_named_manual_save`], the browser's
//!    `wasm_create_save_slot`); nothing here writes, schedules or captures. The
//!    only thing this module claims is that a row with that slot id is now IN
//!    the catalogue and carries a capture tick — which is why a capture tick may
//!    be shown at all. No row, no tick.
//!
//! 2. [`preflight`] — whether one catalogue row could be a live-restore
//!    candidate for THIS session. Issue #1446 executes the restore and does its
//!    own authoritative revalidation at the moment of execution; this is the
//!    advisory model both it and the picker share, so the two cannot describe
//!    compatibility in two different vocabularies.
//!
//! ## What "compatible" means here
//!
//! A live restore keeps the CURRENT participants and their current ship/Station
//! assignments (PRD #1420 story 9). So the question a candidate must answer is
//! not "who was sitting where when this save was taken" — that seating is
//! discarded — but "can the seating that exists RIGHT NOW still exist in the
//! world this save carries".
//!
//! That decomposes into exactly two checks, plus the version gate the catalogue
//! already ran:
//!
//! * **Same scenario/content.** [`crate::save_slots::StartState`] is the
//!   existing format/rules/content answer and is folded in unchanged; the
//!   scenario path is compared on top of it, because a content digest match
//!   across two different authored worlds is not the question being asked.
//! * **Every live ship slot exists in the candidate, flying the same hull.** A
//!   Station is authored by its hull template, and the content digest pins that
//!   template, so "same slot, same hull, same content" is the honest derivation
//!   of "every Station a human currently holds still exists". This module does
//!   not invent a station list the save never recorded.
//!
//! ## Resolving the local slot's hull before comparing it
//!
//! [`crate::lockstep::FleetShip::ship_path`] is `None` on a roster that never
//! negotiated a fleet, and it means "whatever this host's own lobby selected" —
//! a per-host PLACEHOLDER, not a shared value. Comparing two placeholders would
//! make the hull check vacuous in exactly the topology PRD #1420 ships first: a
//! single simulation peer, whose [`crate::lockstep::FleetRoster`] is still the
//! default one-ship roster. A save taken on a completely different hull would
//! then read as "can hold the current assignments".
//!
//! So each side resolves its own placeholder BEFORE the comparison, from the
//! concrete hull it actually has:
//!
//! * live — [`crate::lobby::SelectedShipResource`], the hull this peer booted;
//! * candidate — [`crate::snapshot::BootIdentity::selected_ship`], which is that
//!   same resource as it stood when the capture was taken (see
//!   [`crate::snapshot::capture`]).
//!
//! Only the roster's own [`crate::lockstep::FleetRoster::local`] slot may take
//! that substitution — it is the only slot either value describes. Where a hull
//! still cannot be resolved on either side, [`CandidateBlock::HullUnknown`] says
//! so; an unresolved hull is never reported as a match.
//!
//! ## A GM peer has no selection to substitute, and needs none
//!
//! A browser GM (`?gm=1`) is a full deterministic participant that deliberately
//! holds no [`crate::lobby::SelectedShipResource`] at all. Its identity is the
//! ROSTER: replicated, peer-identical, and carrying a concrete `ship_path` on
//! every slot of a fleet that actually formed. The GM is not one of those slots —
//! it flies nothing, so it has no local placeholder to resolve and constrains no
//! candidate — and [`crate::snapshot::BootIdentity::selected_ship`] is therefore
//! empty in the saves it takes rather than borrowed from another peer's choice.
//! An empty value resolves to "unknown" here exactly as an absent live selection
//! does, which is why neither side may quietly treat it as a hull.
//!
//! `HullUnknown` keeps its narrow meaning: a hull nobody can name — the default
//! one-ship roster of a session that neither negotiated a fleet nor chose a
//! hull. It is not the answer to "a GM took this bookmark".
//!
//! ## Privacy
//!
//! [`preflight`] takes ONE [`crate::save_slots::SaveSlotEntry`] — a row from the
//! caller's own peer-local catalogue — and the caller's own live seating. There
//! is no peer parameter, no catalogue-of-catalogues and no wire type here: a
//! peer's saves are not reachable from this model even by mistake. It equally
//! never returns the candidate's own crew: [`CandidateFleet`] carries slot and
//! hull only, so old seat ownership cannot travel out of a preflight into a live
//! assignment.

use serde::{Deserialize, Serialize};

use crate::save_slots::{SaveRecordSummary, SaveSlotEntry, StartState};

/// One live ship and the Stations humans are holding aboard it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatedShip {
    /// The technical slot flying this hull, as [`crate::command_admission::HostSlot`]'s ordinal.
    pub slot: u32,
    /// The hull template path this slot is flying, already resolved: the
    /// roster's own `None` placeholder has been substituted with this peer's
    /// concrete selected hull on its local slot (see the module doc). `None`
    /// here therefore means genuinely unknown, and blocks rather than matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<String>,
    /// The Station ids humans currently hold aboard it, sorted. Presentation
    /// only: no check below reads a station id, because the candidate save
    /// records no station list to compare it against. It exists so a refusal
    /// can name what is actually at stake on that hull.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stations: Vec<String>,
}

/// The assignments a candidate must be able to hold.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSeating {
    /// The authored world this session is running.
    pub scenario: String,
    /// Every live ship slot, ascending. Stationless GM participants are not
    /// here: they fly no hull, so they constrain no candidate.
    pub ships: Vec<SeatedShip>,
}

impl LiveSeating {
    /// Project the live, replicated roster into the seating a candidate must hold.
    ///
    /// `local_hull` is this peer's own concrete selected hull
    /// ([`crate::lobby::SelectedShipResource`]), used to resolve the roster's
    /// `None` placeholder on its own [`crate::lockstep::FleetRoster::local`]
    /// slot. Passing `None` is honest ignorance and produces an unresolved hull,
    /// which blocks; it does not produce a match.
    ///
    /// A stationless GM peer passes `None` and is unaffected by it: it owns no
    /// roster ship, so nothing here reads the missing selection, and the fleet's
    /// real hulls come off the roster's own `ship_path`s.
    pub fn from_roster(
        scenario: impl Into<String>,
        roster: &crate::lockstep::FleetRoster,
        local_hull: Option<&str>,
    ) -> Self {
        let local = roster.local();
        let mut ships: Vec<SeatedShip> = roster
            .ships()
            .iter()
            .map(|ship| {
                let mut stations: Vec<String> = ship
                    .crew
                    .iter()
                    .map(|(station, _rating)| station.0.clone())
                    .collect();
                stations.sort();
                stations.dedup();
                SeatedShip {
                    slot: ship.host.0,
                    hull: resolve_hull(ship.ship_path.as_deref(), ship.host == local, local_hull),
                    stations,
                }
            })
            .collect();
        ships.sort_by_key(|ship| ship.slot);
        Self {
            scenario: scenario.into(),
            ships,
        }
    }
}

/// Resolve one roster slot's hull placeholder against the concrete hull its
/// own peer selected. Only the roster's local slot may take the substitution:
/// the selected hull describes that slot and no other.
fn resolve_hull(
    ship_path: Option<&str>,
    is_local_slot: bool,
    selected_hull: Option<&str>,
) -> Option<String> {
    let readable = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    readable(ship_path).or_else(|| {
        if is_local_slot {
            readable(selected_hull)
        } else {
            None
        }
    })
}

/// One ship the candidate save carries. Slot and hull ONLY — see the privacy
/// note above: the candidate's own crew must never reach a live assignment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateShip {
    pub slot: u32,
    /// Resolved exactly as [`SeatedShip::hull`] is, but from the capture's own
    /// [`crate::snapshot::BootIdentity::selected_ship`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<String>,
}

/// The fleet shape recorded in one candidate save.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateFleet {
    pub scenario: String,
    pub ships: Vec<CandidateShip>,
}

impl CandidateFleet {
    /// Read the shape out of a catalogue row's record summary.
    ///
    /// `None` when the row carries no readable boot identity: a damaged row
    /// stays visible and deletable in the catalogue, but it cannot be shown as
    /// a restore candidate whose fleet "matches".
    ///
    /// A save taken by a stationless GM peer records an EMPTY `selected_ship`
    /// (it flies nothing), and [`resolve_hull`] discards an empty value the same
    /// way it discards an absent one — so the GM's own bookmark is read entirely
    /// off the roster it captured, and the slot it never owned is never invented.
    pub fn from_record(record: &SaveRecordSummary) -> Option<Self> {
        let boot = record.boot_identity.as_ref()?;
        let local = boot.fleet.local();
        let mut ships: Vec<CandidateShip> = boot
            .fleet
            .ships()
            .iter()
            .map(|ship| CandidateShip {
                slot: ship.host.0,
                hull: resolve_hull(
                    ship.ship_path.as_deref(),
                    ship.host == local,
                    Some(boot.selected_ship.as_str()),
                ),
            })
            .collect();
        ships.sort_by_key(|ship| ship.slot);
        Some(Self {
            scenario: record.scenario.clone(),
            ships,
        })
    }
}

/// Why a catalogue row cannot be a live-restore candidate for this session.
///
/// Append-only, and each variant carries the concrete values its sentence needs
/// so the picker never has to parse an English message back apart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CandidateBlock {
    /// The row has no readable captured run at all.
    Unreadable,
    /// The row parses but carries no boot identity, so its fleet is unknown.
    NoFleetRecord,
    /// A different authored world.
    ScenarioDiffers { candidate: String, live: String },
    /// The existing save-format gate refused it.
    FormatMoved,
    /// The existing rules-version gate refused it.
    RulesMoved,
    /// The existing content-digest gate refused it.
    ContentMoved,
    /// This peer has not loaded the row's scenario content, so the content
    /// digest is unanswered. Startable as a fresh session; not a live candidate.
    ContentUnverified,
    /// A ship slot that is live now is absent from the candidate, so the
    /// Stations its crew hold could not be re-seated.
    MissingShip { slot: u32, stations: Vec<String> },
    /// The candidate flies a different hull at a live slot, so that hull's
    /// Station set is not the one the live crew is seated on.
    HullDiffers {
        slot: u32,
        candidate: Option<String>,
        live: Option<String>,
        stations: Vec<String>,
    },
    /// One side of a live slot has no resolvable hull, so "same hull" is
    /// unanswered. An unproven match is a refusal, not a match: this is the
    /// block that stops an unresolved roster placeholder reading as agreement.
    HullUnknown { slot: u32, stations: Vec<String> },
}

/// The advisory answer for one candidate row.
///
/// `eligible` is exactly `blocks.is_empty()`; it is published as its own field
/// so a presentation surface never has to infer the verdict from the length of
/// a list it may have truncated.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidatePreflight {
    pub eligible: bool,
    pub blocks: Vec<CandidateBlock>,
}

impl CandidatePreflight {
    fn from_blocks(blocks: Vec<CandidateBlock>) -> Self {
        Self {
            eligible: blocks.is_empty(),
            blocks,
        }
    }
}

/// Fold the catalogue's existing version answer into the shared vocabulary.
fn start_blocks(start: &StartState) -> Vec<CandidateBlock> {
    match start {
        StartState::Ready => Vec::new(),
        StartState::ContentDeferred => vec![CandidateBlock::ContentUnverified],
        StartState::Refused(refusal) => vec![match refusal {
            crate::snapshot::LoadRefusal::Moved(vellum_save::Moved::Format { .. }) => {
                CandidateBlock::FormatMoved
            }
            crate::snapshot::LoadRefusal::Moved(vellum_save::Moved::Rules { .. }) => {
                CandidateBlock::RulesMoved
            }
            crate::snapshot::LoadRefusal::Moved(vellum_save::Moved::Content { .. }) => {
                CandidateBlock::ContentMoved
            }
            crate::snapshot::LoadRefusal::Empty
            | crate::snapshot::LoadRefusal::Unreadable(_)
            | crate::snapshot::LoadRefusal::Unparsable(_) => CandidateBlock::Unreadable,
        }],
    }
}

/// Decide whether one peer-local catalogue row could hold this session's seating.
///
/// Every block that applies is reported, not just the first: a GM choosing
/// between saves needs the whole reason, and a picker that revealed one problem
/// per attempt would be a worse tool than the list it replaced.
pub fn preflight(live: &LiveSeating, entry: &SaveSlotEntry) -> CandidatePreflight {
    let mut blocks = start_blocks(&entry.start);
    let Some(record) = entry.record.as_ref() else {
        // No summary at all: the row did not parse. `start_blocks` has already
        // said so; do not add a fleet complaint on top of an unreadable row.
        if blocks.is_empty() {
            blocks.push(CandidateBlock::Unreadable);
        }
        return CandidatePreflight::from_blocks(blocks);
    };
    if record.scenario != live.scenario {
        blocks.push(CandidateBlock::ScenarioDiffers {
            candidate: record.scenario.clone(),
            live: live.scenario.clone(),
        });
    }
    let Some(fleet) = CandidateFleet::from_record(record) else {
        blocks.push(CandidateBlock::NoFleetRecord);
        return CandidatePreflight::from_blocks(blocks);
    };
    for seated in &live.ships {
        let Some(ship) = fleet.ships.iter().find(|ship| ship.slot == seated.slot) else {
            blocks.push(CandidateBlock::MissingShip {
                slot: seated.slot,
                stations: seated.stations.clone(),
            });
            continue;
        };
        // Both hulls are already resolved (see the module doc). An absent one is
        // therefore genuinely unknown, and an unanswered check must refuse.
        match (ship.hull.as_deref(), seated.hull.as_deref()) {
            (Some(candidate), Some(live)) if candidate == live => {}
            (Some(_), Some(_)) => blocks.push(CandidateBlock::HullDiffers {
                slot: seated.slot,
                candidate: ship.hull.clone(),
                live: seated.hull.clone(),
                stations: seated.stations.clone(),
            }),
            _ => blocks.push(CandidateBlock::HullUnknown {
                slot: seated.slot,
                stations: seated.stations.clone(),
            }),
        }
    }
    CandidatePreflight::from_blocks(blocks)
}

/// A bookmark that really reached this peer's catalogue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedCheckpoint {
    pub slot_id: String,
    pub display_name: String,
    pub scenario: String,
    /// The tick the capture was actually taken between, read back off the
    /// stored record. This is the ONLY source a surface may show a tick from.
    pub capture_tick: u64,
}

/// Find a requested bookmark in the catalogue that came back after it.
///
/// `None` means "not confirmed": either the write never landed, or the row is
/// present but carries no readable record, which is not a checkpoint anybody
/// should be told they now hold.
pub fn confirmed_checkpoint(
    entries: &[SaveSlotEntry],
    slot_id: &str,
) -> Option<ConfirmedCheckpoint> {
    let entry = entries.iter().find(|entry| entry.slot_id == slot_id)?;
    let record = entry.record.as_ref()?;
    Some(ConfirmedCheckpoint {
        slot_id: entry.slot_id.clone(),
        display_name: entry.display_name.clone(),
        scenario: record.scenario.clone(),
        capture_tick: record.capture_tick,
    })
}

#[cfg(test)]
mod tests {
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
        let seating =
            LiveSeating::from_roster(LIVE_WORLD, &FleetRoster::default(), Some(DESTROYER));
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
        let seating =
            LiveSeating::from_roster(LIVE_WORLD, &FleetRoster::default(), Some(DESTROYER));
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
        let seating =
            LiveSeating::from_roster(LIVE_WORLD, &FleetRoster::default(), Some(DESTROYER));
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
}
