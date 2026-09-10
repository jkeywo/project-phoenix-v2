//! Named GM checkpoints and the shared candidate preflight (issue #1445).
//!
//! An integration test because the claims are about what a REAL capture
//! contains and what a REAL catalogue reports. A bare `App` cannot answer
//! either: the journal only reaches a save through the fixed-tick capture the
//! save lifecycle schedules, and a preflight answer is only worth anything if
//! the row it reads was written by that same capture.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use project_phoenix::command_admission::log::HostSlot;
use project_phoenix::core::messages::StationId;
use project_phoenix::gm_action::{
    GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder, GmActionOutcome,
};
use project_phoenix::gm_checkpoint::{
    confirmed_checkpoint, preflight, CandidateBlock, LiveSeating,
};
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::lockstep::{FleetGm, FleetRoster, FleetShip};
use project_phoenix::save_slots::ContentCheck;
use project_phoenix::save_slots_store::{
    install_local_save_store, request_named_manual_save, SaveSlotService,
};
use project_phoenix::sim_tick::SimTick;
use project_phoenix::snapshot::StoredRun;

const WORLD: &str = "assets/worlds/duel.toml";
const SEED: u64 = 1_445_2026;
/// The hull `build` boots on, via `--side-a cruiser`.
const BOOTED_HULL: &str = "assets/entities/alliance_cruiser.toml";
/// A hull this session is demonstrably NOT flying.
const OTHER_HULL: &str = "assets/entities/alliance_destroyer.toml";

/// One peer's private storage, with a switch that fails the next run write.
#[derive(Clone, Default)]
struct PeerStore {
    state: Arc<Mutex<PeerState>>,
}

#[derive(Default)]
struct PeerState {
    slots: BTreeMap<String, String>,
    fail_next_write: bool,
}

impl PeerStore {
    fn fail_next_write(&self) {
        self.state.lock().unwrap().fail_next_write = true;
    }

    fn slot_count(&self) -> usize {
        self.state.lock().unwrap().slots.len()
    }
}

impl vellum_save::Store for PeerStore {
    type Error = String;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok(self.state.lock().unwrap().slots.get(slot).cloned())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        let mut state = self.state.lock().unwrap();
        if std::mem::take(&mut state.fail_next_write) {
            return Err("peer storage quota exceeded".to_string());
        }
        state.slots.insert(slot.to_string(), contents.to_string());
        Ok(())
    }

    fn remove(&self, slot: &str) -> Result<(), Self::Error> {
        self.state.lock().unwrap().slots.remove(slot);
        Ok(())
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        Ok(self.state.lock().unwrap().slots.keys().cloned().collect())
    }
}

fn build(store: PeerStore) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["destroyer".into()],
        dt: 1.0 / 60.0,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    })
    .expect("duel app builds");
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    install_local_save_store(&mut app, store);
    app.finish();
    app.cleanup();
    app
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimTick>().0
}

/// This session's live seating, resolved from the same two resources
/// `snapshot::capture` folds into a save's `BootIdentity`: the replicated
/// roster and the hull this peer actually booted on.
fn live_seating(app: &App, scenario: &str) -> LiveSeating {
    LiveSeating::from_roster(
        scenario,
        app.world().resource::<FleetRoster>(),
        Some(booted_hull(app)),
    )
}

fn booted_hull(app: &App) -> &str {
    app.world()
        .resource::<project_phoenix::lobby::SelectedShipResource>()
        .0
        .as_str()
}

/// Advance until the session is really running and a capture may be admitted.
fn run_to_in_progress(app: &mut App) {
    for _ in 0..2_000 {
        app.update();
        let phase = app
            .world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get()
            .clone();
        if phase == project_phoenix::core::messages::GamePhase::InProgress {
            return;
        }
    }
    panic!("the duel scenario never reached InProgress");
}

/// Put two real canonical GM decisions on the owner's ordered lane and let the
/// ordinary reducer apply them, so the journal a capture folds is the ordinary
/// one and not a hand-built fixture written straight into the snapshot.
///
/// Both are stamped for the SAME tick, in sequence order: a pause and its
/// resume. `apply_due_actions` runs the whole tick's ordered run in one call,
/// so the session is left running — which matters, because a paused session
/// stops advancing and a later grant's boundary would then never arrive.
fn record_two_gm_actions(app: &mut App) {
    let apply_tick = tick(app) + 2;
    let from = HostSlot::SOLO;
    for (index, active) in [(1_u64, true), (2, false)] {
        let grant = GmActionGrant {
            from,
            sequenced_by: from,
            operator_id: "gm-1".to_string(),
            correlation: GmActionId::new(format!("checkpoint-{index}")).unwrap(),
            recovery_generation: 0,
            apply_tick,
            order: GmActionOrder::new(from, index),
            action: GmAction::SetSessionPaused { active },
        };
        app.world_mut()
            .resource_mut::<GmActionJournal>()
            .insert(grant)
            .expect("the owner's own contiguous grant is admitted");
    }
    for _ in 0..16 {
        app.update();
    }
    assert!(
        !app.world()
            .resource::<project_phoenix::gm_action::SimulationPaused>()
            .0,
        "the resume applied in the same ordered run as the pause"
    );
}

/// Ask for a named checkpoint and run until its fixed boundary has been drained.
fn bookmark(app: &mut App, name: &str) -> String {
    let slot_id = request_named_manual_save(app.world_mut(), name).expect("a Store is installed");
    for _ in 0..30 {
        app.update();
    }
    slot_id
}

fn catalogue(app: &App) -> Vec<project_phoenix::save_slots::SaveSlotEntry> {
    let current =
        project_phoenix::snapshot::versions(&project_phoenix::content_ledger::frozen_or_live());
    app.world()
        .resource::<SaveSlotService>()
        .list(&current, ContentCheck::Full)
        .expect("the peer's own catalogue lists")
}

#[test]
fn a_named_bookmark_captures_the_ordinary_action_journal_and_its_inverse_state() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    record_two_gm_actions(&mut app);
    let slot_id = bookmark(&mut app, "Before the ambush");

    // The write really landed, under the name the GM typed, and the tick shown
    // comes from the stored record rather than from the request.
    let entries = catalogue(&app);
    let confirmed = confirmed_checkpoint(&entries, &slot_id).expect("the bookmark is confirmed");
    assert_eq!(confirmed.display_name, "Before the ambush");
    assert_eq!(confirmed.scenario, WORLD);
    assert!(confirmed.capture_tick > 0);

    // The captured save carries the ORDINARY journal: the same durable grants,
    // and enough of them to recompute the terminal outcomes a restore would
    // replay. That derived log is the inverse state this build has — #1441's
    // panel and #1442's inverse both read it, and neither could if a manual
    // capture wrote a journal-free snapshot.
    let text = vellum_save::Store::read(&store, &slot_id)
        .unwrap()
        .expect("the run is in this peer's Store");
    let run = StoredRun::from_ron(&text).expect("the stored run parses");
    let snapshot = run.snapshot.expect("the capture carries a snapshot");
    let mut journal = snapshot.state.gm_actions.clone();
    let log = journal.apply_through(snapshot.tick);
    let recorded: Vec<_> = log
        .entries()
        .iter()
        .map(|entry| (entry.correlation.as_str().to_string(), entry.outcome))
        .collect();
    assert_eq!(
        recorded,
        vec![
            ("checkpoint-1".to_string(), GmActionOutcome::Applied),
            ("checkpoint-2".to_string(), GmActionOutcome::Applied),
        ],
        "the checkpoint replays the run's own GM decisions"
    );
}

#[test]
fn a_refused_storage_write_leaves_no_checkpoint_and_reports_the_failure() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let before = store.slot_count();
    store.fail_next_write();
    let slot_id = bookmark(&mut app, "Doomed");

    let failure = app
        .world_mut()
        .resource_mut::<SaveSlotService>()
        .outcomes()
        .find(|outcome| {
            outcome.decision.slot
                == project_phoenix::save_slots::CaptureSlot::Manual(slot_id.clone())
        })
        .map(|outcome| outcome.result.clone())
        .expect("the manual capture reached the storage adapter");
    assert!(
        failure.is_err(),
        "the refused write is reported: {failure:?}"
    );

    // Nothing was left behind to be shown as a checkpoint, and the read-back
    // that a surface must perform before showing a tick therefore says no.
    assert!(confirmed_checkpoint(&catalogue(&app), &slot_id).is_none());
    assert!(
        store.slot_count() <= before + 1,
        "a failed manual write left at most the unrelated rolling autosave"
    );
    assert!(vellum_save::Store::read(&store, &slot_id)
        .unwrap()
        .is_none());
}

#[test]
fn a_real_capture_is_an_eligible_candidate_for_the_session_that_took_it() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Compatible");
    let entries = catalogue(&app);
    let row = entries
        .iter()
        .find(|entry| entry.slot_id == slot_id)
        .expect("the row is in the catalogue");

    assert_eq!(booted_hull(&app), BOOTED_HULL);
    let live = live_seating(&app, WORLD);
    let answer = preflight(&live, row);
    assert!(answer.eligible, "{:?}", answer.blocks);
}

/// The single-simulation-peer topology PRD #1420 ships first: the roster is the
/// default one-ship one, so its `ship_path` is the per-host "whatever this host
/// selected" placeholder on BOTH sides. The hull that actually differs is the
/// booted one, and a real capture taken on another hull must be refused by name.
#[test]
fn a_real_capture_taken_on_a_different_hull_is_refused_even_on_a_solo_roster() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Wrong hull");
    let entries = catalogue(&app);
    let row = entries
        .iter()
        .find(|entry| entry.slot_id == slot_id)
        .expect("the row is in the catalogue");

    let roster = app.world().resource::<FleetRoster>();
    assert!(
        roster.ships().iter().all(|ship| ship.ship_path.is_none()),
        "the solo roster really is carrying the placeholder this test is about"
    );
    assert_ne!(booted_hull(&app), OTHER_HULL);

    // The session is now flying a different hull from the one the save recorded.
    let live = LiveSeating::from_roster(WORLD, roster, Some(OTHER_HULL));
    let answer = preflight(&live, row);
    assert!(!answer.eligible, "a different hull is not a candidate");
    assert_eq!(
        answer.blocks,
        vec![CandidateBlock::HullDiffers {
            slot: HostSlot::SOLO.0,
            candidate: Some(BOOTED_HULL.to_string()),
            live: Some(OTHER_HULL.to_string()),
            stations: Vec::new(),
        }]
    );

    // And a live slot whose hull cannot be resolved at all does not read as
    // agreement either.
    let unresolved = LiveSeating::from_roster(WORLD, roster, None);
    assert_eq!(
        preflight(&unresolved, row).blocks,
        vec![CandidateBlock::HullUnknown {
            slot: HostSlot::SOLO.0,
            stations: Vec::new(),
        }]
    );
}

#[test]
fn a_capture_that_cannot_hold_the_current_seating_is_refused_with_its_reason() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "One ship short");
    let entries = catalogue(&app);
    let row = entries
        .iter()
        .find(|entry| entry.slot_id == slot_id)
        .expect("the row is in the catalogue");

    // A second crewed ship has joined since the capture. Its Station could not
    // be re-seated in that save, and the refusal names both the slot and what
    // is at stake on it.
    let mut ships: Vec<FleetShip> = app.world().resource::<FleetRoster>().ships().to_vec();
    ships.push(FleetShip {
        host: HostSlot(9),
        ship_path: Some("assets/entities/alliance_destroyer.toml".to_string()),
        crew: vec![(StationId("helm".into()), "Std".to_string())],
    });
    let live = LiveSeating::from_roster(
        WORLD,
        &FleetRoster::new(ships, HostSlot::SOLO),
        Some(booted_hull(&app)),
    );
    let answer = preflight(&live, row);
    assert!(!answer.eligible);
    assert_eq!(
        answer.blocks,
        vec![CandidateBlock::MissingShip {
            slot: 9,
            stations: vec!["helm".to_string()],
        }]
    );

    // ...and a different world is refused as a different world, with both paths.
    let elsewhere = live_seating(&app, "assets/worlds/combat_test.toml");
    assert!(matches!(
        preflight(&elsewhere, row).blocks.first(),
        Some(CandidateBlock::ScenarioDiffers { .. })
    ));
}

#[test]
fn one_peers_checkpoints_never_appear_in_another_peers_catalogue() {
    let mine = PeerStore::default();
    let theirs = PeerStore::default();
    let mut my_app = build(mine.clone());
    let mut their_app = build(theirs.clone());
    run_to_in_progress(&mut my_app);
    run_to_in_progress(&mut their_app);
    let my_slot = bookmark(&mut my_app, "Mine alone");
    let their_slot = bookmark(&mut their_app, "Theirs alone");
    assert_ne!(my_slot, their_slot);

    // Each catalogue is built from its own Store and can only ever be: the
    // model takes one entry list and this session's own seating, and there is
    // no parameter through which another operator's rows could arrive.
    let my_ids: Vec<_> = catalogue(&my_app)
        .iter()
        .map(|entry| entry.slot_id.clone())
        .collect();
    let their_ids: Vec<_> = catalogue(&their_app)
        .iter()
        .map(|entry| entry.slot_id.clone())
        .collect();
    assert!(my_ids.contains(&my_slot));
    assert!(!my_ids.contains(&their_slot));
    assert!(their_ids.contains(&their_slot));
    assert!(!their_ids.contains(&my_slot));
    assert!(confirmed_checkpoint(&catalogue(&my_app), &their_slot).is_none());
}

/// The browser GM profile, as `wasm_prepare_game_master` really builds it: a
/// full deterministic peer that holds NO `SelectedShipResource` (see the
/// `if !is_browser_gm` guard in `src/server/bridge.rs`) and owns no roster
/// ship, seated in a fleet that has formed and therefore carries concrete
/// `ship_path`s.
///
/// The roster goes in before the first update so the world is built from it,
/// exactly as `wasm_join_fleet`'s adopted topology is installed before Startup.
fn build_gm_peer(store: PeerStore) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["destroyer".into()],
        dt: 1.0 / 60.0,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    })
    .expect("duel app builds");
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    app.world_mut()
        .remove_resource::<project_phoenix::lobby::SelectedShipResource>();
    app.insert_resource(
        FleetRoster::with_participants_and_gms(
            vec![FleetShip {
                host: HostSlot(1),
                ship_path: Some(BOOTED_HULL.to_string()),
                crew: vec![(StationId("helm".into()), "Std".to_string())],
            }],
            vec![HostSlot(1), HostSlot(2)],
            vec![FleetGm {
                host: HostSlot(2),
                operator_id: "gm-1".to_string(),
            }],
            HostSlot(2),
            HostSlot(1),
        )
        .expect("a GM peer beside one crewed ship is a valid topology"),
    );
    install_local_save_store(&mut app, store);
    app.finish();
    app.cleanup();
    app
}

/// A bookmark taken by a stationless GM is a candidate for its own session.
///
/// This is the leg no unit test could reach: every other case here supplies a
/// local hull, and `build_headless_app` always inserts one. On the real GM page
/// there is none, and requiring one turned a GM's own checkpoint into an
/// unreadable row the moment it was written — `snapshot::capture` recorded no
/// boot identity at all, so the catalogue refused the save it had just taken.
#[test]
fn a_stationless_gm_peers_bookmark_is_an_eligible_candidate_for_its_own_session() {
    let store = PeerStore::default();
    let mut app = build_gm_peer(store.clone());
    assert!(
        app.world()
            .get_resource::<project_phoenix::lobby::SelectedShipResource>()
            .is_none(),
        "the GM profile really is the one with no local ship selection"
    );
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "GM checkpoint");

    // The capture carries a boot identity derived from the ROSTER: no hull of
    // its own, and the fleet's real hulls where the fleet named them.
    let entries = catalogue(&app);
    let row = entries
        .iter()
        .find(|entry| entry.slot_id == slot_id)
        .expect("the GM's own bookmark is in its own catalogue");
    let boot = row
        .record
        .as_ref()
        .and_then(|record| record.boot_identity.as_ref())
        .expect("a GM capture records its boot identity");
    assert_eq!(boot.selected_ship, "", "a GM peer flies no hull");
    assert_eq!(
        boot.fleet.ships()[0].ship_path.as_deref(),
        Some(BOOTED_HULL),
        "the fleet's own concrete hull is what the identity carries"
    );

    // ...and the catalogue admits it rather than reporting the row it has just
    // written as unreadable.
    assert_eq!(
        row.start,
        project_phoenix::save_slots::StartState::Ready,
        "the GM's own save is startable: {:?}",
        row.start
    );
    assert!(confirmed_checkpoint(&entries, &slot_id).is_some());

    // The preflight this GM's picker runs — with no local hull to substitute,
    // because there is no local ship — reads the fleet off the same roster and
    // finds the session it is looking at.
    let live = LiveSeating::from_roster(WORLD, app.world().resource::<FleetRoster>(), None);
    assert_eq!(
        live.ships.first().and_then(|ship| ship.hull.as_deref()),
        Some(BOOTED_HULL),
        "the live hull comes off the replicated roster, not a local selection"
    );
    let answer = preflight(&live, row);
    assert!(answer.eligible, "{:?}", answer.blocks);
}
