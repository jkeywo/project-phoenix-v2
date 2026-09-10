//! Single-simulation-peer live restore (issue #1446, PRD #1420 stories 8–11,
//! 13, 15, 16).
//!
//! An integration test because every claim is about what a REAL restore does to
//! a REAL world: a real duel scenario, a real save taken through the ordinary
//! catalogue, the real `snapshot::restore` walk, the real digest fold and the
//! real canonical journal. A bare `App` can answer none of those, and a mocked
//! store would only prove that the mock behaves like itself.
//!
//! Faults are injected at the seam a real failure arrives on: the peer's own
//! storage. `PeerStore` can refuse the next write, damage a stored row, or lose
//! one entirely — which is exactly how "the recovery capture failed", "the
//! candidate would not load" and "the rollback would not come back" reach this
//! code in the wild.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use project_phoenix::command_admission::log::HostSlot;
use project_phoenix::gm_action::{
    sequence_owner_proposal, GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder,
    GmActionOutcome, GmActionProposal, GmActionRefusalReason, SimulationPaused,
};
use project_phoenix::gm_restore::{
    GmLiveRestore, GmRestoreFailure, GmRestorePhase, READINESS_FRAME_BUDGET,
};
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::lockstep::{FleetRoster, FleetShip};
use project_phoenix::save_slots::ContentCheck;
use project_phoenix::save_slots_store::{
    install_local_save_store, request_named_manual_save, SaveSlotService,
};
use project_phoenix::sim_tick::SimTick;

const WORLD: &str = "assets/worlds/duel.toml";
const SEED: u64 = 1_446_2026;
/// A hull this session is demonstrably NOT flying.
const OTHER_HULL: &str = "assets/entities/alliance_destroyer.toml";

/// One peer's private storage, with the three faults a real one has.
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

    /// Leave the row in the catalogue but make its contents unreadable — the
    /// shape a truncated or edited `localStorage` value really has.
    fn damage(&self, slot_id: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(text) = state.slots.get_mut(slot_id) {
            *text = "(this is not a stored run)".to_string();
        }
    }

    fn forget(&self, slot_id: &str) {
        self.state.lock().unwrap().slots.remove(slot_id);
    }

    /// Leave a perfectly valid, gate-passing run whose recorded fold is wrong.
    ///
    /// This is the fault the corruption check exists for, and the only one that
    /// reaches the rollback path: an unreadable row is refused by revalidation
    /// long before anything is loaded, so damaging one proves nothing about
    /// what happens after a load.
    fn tamper_recorded_digest(&self, slot_id: &str) {
        let mut state = self.state.lock().unwrap();
        let text = state
            .slots
            .get(slot_id)
            .cloned()
            .expect("the row is in this peer's storage");
        let mut run =
            project_phoenix::snapshot::StoredRun::from_ron(&text).expect("the stored run parses");
        let snapshot = run.snapshot.as_mut().expect("the row carries a snapshot");
        snapshot.digest ^= 0xDEAD_BEEF;
        state.slots.insert(
            slot_id.to_string(),
            run.to_ron().expect("the tampered run serialises"),
        );
    }

    /// Strip every captured row's spawn recipe, leaving a gate-passing run that
    /// names entities this world can neither find nor build.
    ///
    /// The shape of a candidate whose rows were authored in a world layer this
    /// session no longer carries, and the only remaining way for a HELD world to
    /// answer "not ready" forever: with the rows rebuildable, a missing entity
    /// is built; without a spawn recipe, no amount of waiting can produce one,
    /// and a held world spends no ticks in which it might.
    fn forget_spawn_recipes(&self, slot_id: &str) {
        let mut state = self.state.lock().unwrap();
        let text = state
            .slots
            .get(slot_id)
            .cloned()
            .expect("the row is in this peer's storage");
        let mut run =
            project_phoenix::snapshot::StoredRun::from_ron(&text).expect("the stored run parses");
        let snapshot = run.snapshot.as_mut().expect("the row carries a snapshot");
        for entity in &mut snapshot.state.entities {
            entity.spawn = None;
        }
        state.slots.insert(
            slot_id.to_string(),
            run.to_ron().expect("the edited run serialises"),
        );
    }

    fn keys(&self) -> Vec<String> {
        self.state.lock().unwrap().slots.keys().cloned().collect()
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

fn paused(app: &App) -> bool {
    app.world().resource::<SimulationPaused>().0
}

fn phase(app: &App) -> GmRestorePhase {
    app.world().resource::<GmLiveRestore>().phase()
}

fn digest(app: &App) -> u64 {
    project_phoenix::sim_digest::world_digest(app.world())
}

fn run_to_in_progress(app: &mut App) {
    for _ in 0..2_000 {
        app.update();
        if *app
            .world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get()
            == project_phoenix::core::messages::GamePhase::InProgress
        {
            return;
        }
    }
    panic!("the duel scenario never reached InProgress");
}

/// Ask for a named checkpoint through the ORDINARY bookmark path and run until
/// its fixed boundary has been drained.
fn bookmark(app: &mut App, name: &str) -> String {
    let slot_id = request_named_manual_save(app.world_mut(), name).expect("a Store is installed");
    for _ in 0..30 {
        app.update();
    }
    slot_id
}

/// Terminal facts for a set of correlations, sampled AS THEY APPEAR.
///
/// A successful restore replaces the canonical journal with the candidate's
/// own, so the request's fact is gone from the derived log by the time the
/// driver settles — reading it afterwards would be reading the abandoned
/// timeline. This runs frames and takes the first terminal sighting of each.
fn settle_facts(
    app: &mut App,
    names: &[&str],
) -> BTreeMap<String, (GmActionOutcome, Option<GmActionRefusalReason>)> {
    let mut facts = BTreeMap::new();
    for _ in 0..40 {
        for entry in app
            .world()
            .resource::<project_phoenix::gm_action::GmActionLog>()
            .entries()
        {
            let name = entry.correlation.as_str();
            if entry.outcome == GmActionOutcome::Pending || !names.contains(&name) {
                continue;
            }
            facts
                .entry(name.to_string())
                .or_insert((entry.outcome, entry.reason));
        }
        if facts.len() == names.len() && !phase(app).in_flight() {
            break;
        }
        app.update();
    }
    facts
}

fn fact(
    facts: &BTreeMap<String, (GmActionOutcome, Option<GmActionRefusalReason>)>,
    name: &str,
) -> (GmActionOutcome, Option<GmActionRefusalReason>) {
    *facts
        .get(name)
        .unwrap_or_else(|| panic!("no canonical fact for {name}"))
}

/// The tick a confirmed catalogue row actually recorded.
///
/// Read back off the stored record, never guessed from the frame the bookmark
/// was asked on: a manual capture lands at the next FIXED boundary, which is
/// not where the caller was standing when it asked.
fn capture_tick(app: &App, slot_id: &str) -> u64 {
    let current =
        project_phoenix::snapshot::versions(&project_phoenix::content_ledger::frozen_or_live());
    let entries = app
        .world()
        .resource::<SaveSlotService>()
        .list(&current, ContentCheck::Full)
        .expect("the peer's own catalogue lists");
    project_phoenix::gm_checkpoint::confirmed_checkpoint(&entries, slot_id)
        .expect("the bookmark is confirmed")
        .capture_tick
}

/// Sequence one GM decision through the REAL owner path, without running it.
///
/// Deliberately `sequence_owner_proposal` rather than a hand-built grant: WHERE
/// a decision lands is the whole question this feature turns on — the held
/// branch, the slot's live protocol generation and its recovery boundary are
/// all decided there — and a test that inserted grants at a tick of its own
/// choosing would be asserting against a mirror of that rule instead of the
/// rule itself. The arguments are the ones `submit_local` computes for this
/// topology: this peer is its own ordering owner, and with no `FleetLockstep`
/// its logical frontier is its own tick.
///
/// Returns the tick the owner scheduled the grant for, so a test can say
/// whether a held world could ever reach it.
fn propose(app: &mut App, correlation: &str, operator: &str, action: GmAction) -> u64 {
    let now = tick(app);
    let current_paused = paused(app);
    let live_hold = app
        .world()
        .resource::<GmLiveRestore>()
        .phase()
        .holds_world();
    let proposal = GmActionProposal {
        from: HostSlot::SOLO,
        operator_id: operator.to_string(),
        correlation: GmActionId::new(correlation.to_string()).unwrap(),
        action,
    };
    let mut journal = app.world_mut().resource_mut::<GmActionJournal>();
    sequence_owner_proposal(
        &mut journal,
        &proposal,
        HostSlot::SOLO,
        now,
        now.saturating_sub(1),
        current_paused,
        live_hold,
    )
    .expect("the owner sequences its own proposal")
    .apply_tick
}

/// Ask for a live restore of `slot_id` and run the driver to a settled phase.
fn restore(
    app: &mut App,
    correlation: &str,
    slot_id: &str,
) -> (GmActionOutcome, Option<GmActionRefusalReason>) {
    propose(
        app,
        correlation,
        "gm-1",
        GmAction::RequestLiveRestore {
            candidate: slot_id.to_string(),
        },
    );
    let facts = settle_facts(app, &[correlation]);
    fact(&facts, correlation)
}

/// The tick the accepted request was applied at — the moment from which the
/// world is held, and therefore the tick an untouched world must still be at.
fn requested_tick(app: &App) -> u64 {
    app.world()
        .resource::<GmLiveRestore>()
        .request()
        .expect("a request was accepted")
        .requested_tick
}

fn failure(app: &App) -> Option<GmRestoreFailure> {
    app.world().resource::<GmLiveRestore>().failure().cloned()
}

/// The live crew authority on the local hull: exactly what a restore must NOT
/// overwrite with the candidate's own recorded seating.
fn live_ratings(app: &mut App) -> BTreeMap<String, String> {
    let mut query = app.world_mut().query::<(
        &project_phoenix::ship::components::ActiveStationRatings,
        &project_phoenix::server_app::LocalShip,
    )>();
    let (ratings, _) = query
        .iter(app.world())
        .next()
        .expect("the local hull is in the world");
    ratings
        .0
        .iter()
        .map(|(station, rating)| (station.0.clone(), rating.clone()))
        .collect()
}

fn set_local_rating(app: &mut App, rating: &str) -> String {
    let mut query = app.world_mut().query_filtered::<
        &mut project_phoenix::ship::components::ActiveStationRatings,
        With<project_phoenix::server_app::LocalShip>,
    >();
    let mut ratings = query
        .iter_mut(app.world_mut())
        .next()
        .expect("the local hull is in the world");
    let station = ratings
        .0
        .keys()
        .next()
        .cloned()
        .expect("the hull has at least one Station");
    ratings.0.insert(station.clone(), rating.to_string());
    station.0
}

/// One NPC hull the duel spawned from a script, named by uuid.
///
/// Script-spawned rather than authored deliberately: `restore` can rebuild a row
/// that carries a spawn recipe, so this is the entity whose disappearance a
/// rewind is expected to UNDO rather than be defeated by. The local hull is
/// excluded — the seating claims are about that one.
fn a_spawned_npc(app: &mut App) -> String {
    let mut query = app
        .world_mut()
        .query_filtered::<&project_phoenix::entities::spawner::EntityUuid, (
            With<project_phoenix::entities::spawner::EntitySpawnOrigin>,
            Without<project_phoenix::server_app::LocalShip>,
        )>();
    query
        .iter(app.world())
        .next()
        .expect("the duel spawns its NPC roster from script effects")
        .0
        .clone()
}

/// Whether a uuid is standing in the live world at all.
fn standing(app: &mut App, uuid: &str) -> bool {
    let mut query = app
        .world_mut()
        .query::<&project_phoenix::entities::spawner::EntityUuid>();
    query.iter(app.world()).any(|row| row.0 == uuid)
}

/// Take one entity out of the live world, the way a destruction or a GM removal
/// does: the row is gone, and nothing will spawn it again.
fn despawn_uuid(app: &mut App, uuid: &str) {
    let mut query = app
        .world_mut()
        .query::<(Entity, &project_phoenix::entities::spawner::EntityUuid)>();
    let entity = query
        .iter(app.world())
        .find(|(_, row)| row.0 == uuid)
        .map(|(entity, _)| entity)
        .expect("the entity is standing");
    app.world_mut().entity_mut(entity).despawn();
}

/// Run frames until the restore driver has settled, or `budget` is spent.
fn settle_phase(app: &mut App, budget: u32) {
    for _ in 0..budget {
        if !phase(app).in_flight() {
            return;
        }
        app.update();
    }
}

// ---------------------------------------------------------------------------

/// The whole happy path, end to end: a real save, a real rewind, the live
/// seating kept, a fresh protocol generation, and a world that is still held.
#[test]
fn a_live_restore_rewinds_the_world_keeps_the_live_seating_and_stays_paused() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Before the ambush");
    let captured_tick = capture_tick(&app, &slot_id);

    // Time passes, and the room changes: somebody re-rates a Station AFTER the
    // save. That is the seating a restore must keep.
    for _ in 0..120 {
        app.update();
    }
    let station = set_local_rating(&mut app, "commander");
    let live_seating = live_ratings(&mut app);
    assert!(tick(&app) > captured_tick, "the session really moved on");

    let (result, _) = restore(&mut app, "restore-1", &slot_id);
    let moved_on_tick = requested_tick(&app);

    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(phase(&app), GmRestorePhase::Restored, "{:?}", failure(&app));
    assert_eq!(failure(&app), None);

    // The world is the candidate's, held at the tick it recorded.
    let restore_state = app.world().resource::<GmLiveRestore>();
    assert_eq!(restore_state.restored_tick(), Some(captured_tick));
    assert!(restore_state.restored_digest().is_some());
    assert!(paused(&app), "a restore never resumes the session");
    assert_eq!(tick(&app), captured_tick, "the tick really rewound");

    // The seating is the LIVE one, not the one the save recorded.
    assert_eq!(
        live_ratings(&mut app).get(&station).map(String::as_str),
        Some("commander"),
        "the restore kept the seating the room actually has"
    );
    assert_eq!(live_ratings(&mut app), live_seating);

    // A fresh live protocol generation is stamped on this peer's own slot.
    let generation = app
        .world()
        .resource::<GmLiveRestore>()
        .fence_generation()
        .expect("the fence was stamped");
    assert!(generation >= 1);
    assert_eq!(
        app.world()
            .resource::<GmActionJournal>()
            .current_recovery_generation(HostSlot::SOLO),
        generation
    );

    // And the recovery checkpoint is a real, readable row of the ORDINARY
    // catalogue — not an invisible buffer that dies with the tab.
    let recovery = app
        .world()
        .resource::<GmLiveRestore>()
        .recovery_slot()
        .expect("a recovery checkpoint was taken")
        .to_string();
    assert!(store.keys().contains(&recovery));
    let current =
        project_phoenix::snapshot::versions(&project_phoenix::content_ledger::frozen_or_live());
    let entries = app
        .world()
        .resource::<SaveSlotService>()
        .list(&current, ContentCheck::Full)
        .expect("the catalogue lists");
    let row = project_phoenix::gm_checkpoint::confirmed_checkpoint(&entries, &recovery)
        .expect("the recovery checkpoint is confirmed");
    assert_eq!(
        row.capture_tick, moved_on_tick,
        "the recovery checkpoint is the world as the request found it"
    );
}

/// The saved world and the saved action log rewind TOGETHER: entries recorded
/// after the checkpoint are discarded rather than retained as an abandoned
/// timeline, and new work appends to the restored log.
#[test]
fn the_restored_journal_is_the_candidates_own_log_with_later_entries_discarded() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);

    // Two real decisions, a pause and its resume, on ONE ordered run: the
    // reducer applies a whole tick's run in a single call, so the session is
    // left running — a held world would never reach a later grant's boundary.
    let decisions = |app: &mut App, names: [&str; 2]| {
        let mut apply_ticks = Vec::new();
        for (name, active) in names.iter().zip([true, false]) {
            apply_ticks.push(propose(
                app,
                name,
                "gm-1",
                GmAction::SetSessionPaused { active },
            ));
        }
        assert_eq!(
            apply_ticks[0], apply_ticks[1],
            "the owner puts a resume at the very boundary its own pause stopped"
        );
        settle_facts(app, &names);
    };
    decisions(&mut app, ["before-the-save", "before-the-save-resume"]);
    let slot_id = bookmark(&mut app, "Two decisions in");
    decisions(&mut app, ["after-the-save", "after-the-save-resume"]);
    let correlations = |app: &App| -> Vec<String> {
        app.world()
            .resource::<GmActionJournal>()
            .grants()
            .iter()
            .map(|grant| grant.correlation.as_str().to_string())
            .collect()
    };
    assert!(correlations(&app).contains(&"after-the-save".to_string()));

    let (result, _) = restore(&mut app, "restore-1", &slot_id);
    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(phase(&app), GmRestorePhase::Restored, "{:?}", failure(&app));

    let restored = correlations(&app);
    assert!(
        restored.contains(&"before-the-save".to_string()),
        "the checkpoint's own log came back: {restored:?}"
    );
    assert!(
        !restored.contains(&"after-the-save".to_string()),
        "the abandoned timeline is not retained: {restored:?}"
    );
    // ...and the request that caused the restore is not retained either: it
    // belonged to the abandoned timeline, which is the whole point.
    assert!(!restored.contains(&"restore-1".to_string()));
}

/// The bounded delivery is VISIBLE. More than one simulation peer refuses by
/// name, at the canonical apply boundary, with nothing captured or loaded.
#[test]
fn a_multi_simulation_peer_session_refuses_the_request_without_touching_the_world() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Never restored");

    // A second technical participant, exactly as a joined peer would appear.
    let roster = FleetRoster::with_participants(
        vec![FleetShip::new(HostSlot::SOLO), FleetShip::new(HostSlot(1))],
        vec![HostSlot::SOLO, HostSlot(1)],
        HostSlot::SOLO,
        HostSlot::SOLO,
    )
    .expect("a two-peer roster is representable");
    app.world_mut().insert_resource(roster);
    app.update();

    let before_slots = store.keys();
    let saved_tick = tick(&app);

    let (result, reason) = restore(&mut app, "restore-1", &slot_id);

    assert_eq!(result, GmActionOutcome::Refused);
    assert_eq!(reason, Some(GmActionRefusalReason::MultipleSimulationPeers));
    assert_eq!(phase(&app), GmRestorePhase::Idle);
    assert!(!paused(&app), "a refused request does not hold the session");
    assert!(tick(&app) >= saved_tick, "the world did not rewind");
    assert_eq!(
        store.keys(),
        before_slots,
        "no recovery checkpoint was taken"
    );
}

/// The first accepted ordered request wins; a second at the same boundary is
/// refused as concurrent and cannot displace the first one's attribution.
#[test]
fn a_concurrent_second_request_is_refused_and_the_first_one_wins() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let first_slot = bookmark(&mut app, "First");
    let second_slot = bookmark(&mut app, "Second");
    assert_ne!(first_slot, second_slot);

    // Both requests land on ONE apply tick, in sequence order, because that is
    // what the owner's own rule does with them: the first request holds the
    // session, so the second is scheduled at the boundary that hold stopped.
    // `apply_due_actions` runs a whole ordered run in a single call, so this is
    // exactly two GMs pressing at the same moment.
    let mut apply_ticks = Vec::new();
    for (correlation, slot, operator) in [
        ("restore-first", first_slot.as_str(), "gm-1"),
        ("restore-second", second_slot.as_str(), "gm-2"),
    ] {
        apply_ticks.push(propose(
            &mut app,
            correlation,
            operator,
            GmAction::RequestLiveRestore {
                candidate: slot.to_string(),
            },
        ));
    }
    assert_eq!(
        apply_ticks[0], apply_ticks[1],
        "a concurrent request is answered, not deferred to a tick the hold forbids"
    );
    let facts = settle_facts(&mut app, &["restore-first", "restore-second"]);

    assert_eq!(fact(&facts, "restore-first").0, GmActionOutcome::Applied);
    assert_eq!(
        fact(&facts, "restore-second"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::LiveRestoreInProgress)
        )
    );
    let accepted = app
        .world()
        .resource::<GmLiveRestore>()
        .request()
        .cloned()
        .expect("the accepted request is recorded");
    assert_eq!(accepted.operator_id, "gm-1");
    assert_eq!(accepted.candidate_slot, first_slot);
}

/// The recovery checkpoint is a precondition, not a courtesy. A storage refusal
/// stops the restore with the candidate world untouched.
#[test]
fn a_refused_recovery_capture_refuses_the_restore_without_loading_anything() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Never loaded");
    for _ in 0..120 {
        app.update();
    }
    let saved_tick = tick(&app);

    store.fail_next_write();
    let (result, _) = restore(&mut app, "restore-1", &slot_id);

    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(phase(&app), GmRestorePhase::RolledBack);
    assert!(matches!(
        failure(&app),
        Some(GmRestoreFailure::RecoveryCaptureFailed { .. })
    ));
    // Nothing was loaded: the world is exactly where the accepted request held
    // it, and it never went back to the candidate's tick.
    assert_eq!(tick(&app), requested_tick(&app));
    assert!(tick(&app) >= saved_tick, "the world did not rewind");
    assert!(paused(&app));
}

/// The candidate is revalidated AT EXECUTION, against live state, in the same
/// vocabulary the picker used — not against the advisory answer a GM saw.
#[test]
fn a_candidate_that_stopped_fitting_is_refused_at_execution() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Wrong hull now");
    let saved_tick = tick(&app);

    // The session is now flying a hull the save never recorded.
    app.world_mut()
        .insert_resource(project_phoenix::lobby::SelectedShipResource(
            OTHER_HULL.to_string(),
        ));
    app.update();

    restore(&mut app, "restore-1", &slot_id);

    assert_eq!(phase(&app), GmRestorePhase::RolledBack);
    let Some(GmRestoreFailure::CandidateIneligible { blocks }) = failure(&app) else {
        panic!("expected an eligibility refusal, got {:?}", failure(&app));
    };
    assert!(
        blocks.iter().any(|block| matches!(
            block,
            project_phoenix::gm_checkpoint::CandidateBlock::HullDiffers { .. }
        )),
        "the concrete block is named: {blocks:?}"
    );
    assert_eq!(tick(&app), requested_tick(&app), "nothing was loaded");
    assert!(tick(&app) >= saved_tick, "the world did not rewind");
    assert!(paused(&app));
}

/// A row that is present but unreadable is refused before anything is loaded,
/// and the hold it placed really holds.
#[test]
fn an_unreadable_candidate_is_refused_before_any_world_is_touched() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Damaged");
    for _ in 0..60 {
        app.update();
    }

    // Still IN the catalogue; its contents are not a run. That is what a
    // truncated or hand-edited browser value actually looks like.
    store.damage(&slot_id);
    let (result, _) = restore(&mut app, "restore-1", &slot_id);

    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(phase(&app), GmRestorePhase::RolledBack);
    assert!(
        matches!(
            failure(&app),
            Some(GmRestoreFailure::CandidateIneligible { .. })
        ),
        "{:?}",
        failure(&app)
    );
    assert_eq!(tick(&app), requested_tick(&app), "nothing was loaded");
    // And the hold is a real hold: the world does not move on afterwards.
    let held = digest(&app);
    for _ in 0..30 {
        app.update();
    }
    assert!(paused(&app));
    assert_eq!(digest(&app), held, "a held world stays where it is");
}

/// A candidate whose loaded world does not reproduce its recorded fold is
/// rolled back to the recovery checkpoint, and the session stays held.
#[test]
fn a_candidate_that_does_not_reproduce_rolls_back_to_the_recovery_checkpoint() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Does not reproduce");
    let candidate_tick = tick(&app);
    for _ in 0..120 {
        app.update();
    }
    let saved_tick = tick(&app);
    assert!(saved_tick > candidate_tick);

    // The row still gates clean — same build, same rules, same content — and
    // the world it restores to simply is not the one it claims.
    store.tamper_recorded_digest(&slot_id);
    let (result, _) = restore(&mut app, "restore-1", &slot_id);

    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(
        phase(&app),
        GmRestorePhase::RolledBack,
        "{:?}",
        failure(&app)
    );
    assert!(
        matches!(failure(&app), Some(GmRestoreFailure::DigestMismatch { .. })),
        "{:?}",
        failure(&app)
    );
    // The world is the recovery checkpoint's, not the candidate's.
    assert_eq!(tick(&app), requested_tick(&app));
    assert_ne!(
        tick(&app),
        candidate_tick,
        "the candidate was not left behind"
    );
    assert!(paused(&app), "a failed restore stays held");

    // Held is not stuck. This journal is the LIVE one — the rolled-back restore
    // request is still in it, with its applied result — so the owner reads the
    // hold off the canonical prefix rather than off a live flag, and the GM's
    // own resume is due at the tick the world is standing on.
    let held_tick = tick(&app);
    let resume_tick = propose(
        &mut app,
        "resume-1",
        "gm-1",
        GmAction::SetSessionPaused { active: false },
    );
    assert_eq!(
        resume_tick, held_tick,
        "a rolled-back restore can be resumed out of"
    );
    let facts = settle_facts(&mut app, &["resume-1"]);
    assert_eq!(fact(&facts, "resume-1").0, GmActionOutcome::Applied);
    assert!(!paused(&app));
    for _ in 0..30 {
        app.update();
    }
    assert!(tick(&app) > held_tick, "the recovered world runs on");
}

/// A rollback that cannot be read is not quietly reported as a rollback. The
/// world is in neither state, and this build says so and stays held.
#[test]
fn a_rollback_that_cannot_be_read_stays_honestly_failed_and_paused() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Does not reproduce");
    for _ in 0..120 {
        app.update();
    }
    store.tamper_recorded_digest(&slot_id);

    // Lose the recovery checkpoint the moment it has been taken, so the
    // rollback has nothing to read back.
    propose(
        &mut app,
        "restore-1",
        "gm-1",
        GmAction::RequestLiveRestore {
            candidate: slot_id.clone(),
        },
    );
    // Lost only once the checkpoint has been CONFIRMED, so the restore really
    // proceeds to the load and the rollback is the step that fails.
    let mut dropped = false;
    for _ in 0..40 {
        if !dropped && phase(&app) == GmRestorePhase::Loading {
            let recovery = app
                .world()
                .resource::<GmLiveRestore>()
                .recovery_slot()
                .map(str::to_string)
                .expect("the confirmed checkpoint has a slot");
            store.forget(&recovery);
            dropped = true;
        }
        if dropped && !phase(&app).in_flight() {
            break;
        }
        app.update();
    }
    assert!(
        dropped,
        "the recovery checkpoint was confirmed and then lost"
    );

    assert_eq!(phase(&app), GmRestorePhase::Failed, "{:?}", failure(&app));
    assert!(matches!(
        failure(&app),
        Some(GmRestoreFailure::RollbackFailed { .. })
    ));
    assert!(paused(&app), "an honestly failed restore never resumes");
}

/// A restore that cannot be fenced is not a restore that succeeded.
///
/// The one candidate this cannot be done for is one whose own journal already
/// fences this slot at or beyond the tick it was captured at: the boundary
/// cannot go backwards, and a repeat at the same boundary hands back the
/// generation already there, which would report a fence while leaving
/// pre-restore work admissible. So it rolls back and says which step failed.
#[test]
fn a_candidate_that_cannot_be_fenced_rolls_back_instead_of_reporting_success() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);

    // #1119's ordinary shape: a slot recovery opened for a boundary the session
    // has not reached yet. Recorded BEFORE the checkpoint, so the candidate
    // carries it and no fresh boundary can be claimed at the restored tick.
    let boundary = tick(&app) + 5;
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .record_slot_recovery(HostSlot::SOLO, boundary)
        .expect("a fresh recovery boundary is admitted");
    let slot_id = bookmark(&mut app, "Already fenced");
    assert!(capture_tick(&app, &slot_id) < boundary);
    for _ in 0..60 {
        app.update();
    }
    let live_tick = tick(&app);

    let (result, _) = restore(&mut app, "restore-1", &slot_id);
    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(
        phase(&app),
        GmRestorePhase::RolledBack,
        "{:?}",
        failure(&app)
    );
    assert!(
        matches!(failure(&app), Some(GmRestoreFailure::FenceFailed { .. })),
        "{:?}",
        failure(&app)
    );
    // The recovery checkpoint is what is on screen, not the unfenced candidate.
    assert!(tick(&app) >= live_tick, "the world came back to the hold");
    assert!(paused(&app), "an unfenced restore never resumes");
}

/// The worst case: the candidate was COMMITTED and then the rollback could not
/// be read back. The world is on a timeline nobody chose, and it must stop
/// there — held, reported failed, and staying that way.
///
/// This combines the two faults the cases above inject one at a time, and the
/// combination is what makes it dangerous. A commit re-installs the CANDIDATE's
/// own `SimulationPaused`, which for a bookmark taken while the session was
/// running is `false`; if the failure settle left it there, the driver would
/// read its own unpaused world as the GM's explicit resume next frame, clear
/// the phase, and let an unfenced timeline run with the "Do not resume" banner
/// never shown.
#[test]
fn a_rollback_lost_after_the_candidate_committed_stays_failed_and_stopped() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);

    // Fault one: a recovery boundary the candidate already carries, so the
    // fence step fails AFTER `gate_and_restore` has committed the candidate.
    let boundary = tick(&app) + 5;
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .record_slot_recovery(HostSlot::SOLO, boundary)
        .expect("a fresh recovery boundary is admitted");
    let slot_id = bookmark(&mut app, "Committed then stranded");
    assert!(capture_tick(&app, &slot_id) < boundary);
    for _ in 0..60 {
        app.update();
    }
    assert!(!paused(&app), "the bookmark was taken on a running session");

    propose(
        &mut app,
        "restore-1",
        "gm-1",
        GmAction::RequestLiveRestore {
            candidate: slot_id.clone(),
        },
    );
    // Fault two: lose the recovery row once it is CONFIRMED, so the rollback
    // the fence failure triggers has nothing to read back.
    let mut dropped = false;
    for _ in 0..40 {
        if !dropped && phase(&app) == GmRestorePhase::Loading {
            let recovery = app
                .world()
                .resource::<GmLiveRestore>()
                .recovery_slot()
                .map(str::to_string)
                .expect("the confirmed checkpoint has a slot");
            store.forget(&recovery);
            dropped = true;
        }
        if dropped && !phase(&app).in_flight() {
            break;
        }
        app.update();
    }
    assert!(
        dropped,
        "the recovery checkpoint was confirmed and then lost"
    );

    assert_eq!(phase(&app), GmRestorePhase::Failed, "{:?}", failure(&app));
    assert!(
        matches!(failure(&app), Some(GmRestoreFailure::RollbackFailed { .. })),
        "{:?}",
        failure(&app)
    );
    assert!(paused(&app), "a stranded world is stopped");

    // And it STAYS stopped: no frame of the driver's own may lift the hold.
    let stranded_tick = tick(&app);
    for _ in 0..60 {
        app.update();
    }
    assert_eq!(
        phase(&app),
        GmRestorePhase::Failed,
        "the failure does not clear itself"
    );
    assert!(paused(&app), "the hold outlives the failure");
    assert_eq!(
        tick(&app),
        stranded_tick,
        "a stranded world spends no simulation ticks"
    );
}

/// The fence is real: work stamped with the pre-restore generation is stale on
/// the restored timeline, and the reducer refuses it.
#[test]
fn old_in_flight_work_is_fenced_off_after_a_restore() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Before the fence");
    for _ in 0..60 {
        app.update();
    }
    let (result, _) = restore(&mut app, "restore-1", &slot_id);
    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(phase(&app), GmRestorePhase::Restored, "{:?}", failure(&app));

    let journal = app.world().resource::<GmActionJournal>();
    let generation = journal.current_recovery_generation(HostSlot::SOLO);
    assert!(generation >= 1, "a fresh generation was stamped");

    // One grant from before the restore, re-delivered onto the new timeline.
    let stale = GmActionGrant {
        from: HostSlot::SOLO,
        sequenced_by: HostSlot::SOLO,
        operator_id: "gm-1".to_string(),
        correlation: GmActionId::new("stale".to_string()).unwrap(),
        recovery_generation: generation - 1,
        apply_tick: tick(&app) + 4,
        order: GmActionOrder::new(HostSlot::SOLO, journal.next_sequence()),
        action: GmAction::SetSessionPaused { active: false },
    };
    assert!(
        !journal.grant_generation_is_valid(&stale, stale.apply_tick),
        "the pre-restore incarnation is stale on the restored timeline"
    );
    // And genuinely new work, stamped with the current generation, is not.
    let fresh = GmActionGrant {
        recovery_generation: generation,
        correlation: GmActionId::new("fresh".to_string()).unwrap(),
        ..stale.clone()
    };
    assert!(journal.grant_generation_is_valid(&fresh, fresh.apply_tick));
}

/// Success stays held until the GM says otherwise, and their resume is the
/// ordinary session-pause action — there is no second resume.
#[test]
fn a_restored_session_stays_held_until_an_explicit_gm_resume() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Held");
    for _ in 0..60 {
        app.update();
    }
    let (result, _) = restore(&mut app, "restore-1", &slot_id);
    assert_eq!(result, GmActionOutcome::Applied);
    assert_eq!(phase(&app), GmRestorePhase::Restored, "{:?}", failure(&app));

    // Nothing resumes on its own, however long the desk is left alone.
    for _ in 0..60 {
        app.update();
    }
    assert!(paused(&app));
    assert_eq!(phase(&app), GmRestorePhase::Restored);

    // The GM presses Resume, through the OWNER'S OWN rule. This is the whole
    // claim: a held world spends no ticks, so the one tick this grant may be
    // scheduled for is the tick the world is standing on. Anything later — the
    // fence's boundary included — is a grant that can never come due, and a
    // session nobody can ever resume.
    let held_tick = tick(&app);
    let resume_tick = propose(
        &mut app,
        "resume-1",
        "gm-1",
        GmAction::SetSessionPaused { active: false },
    );
    assert_eq!(
        resume_tick,
        held_tick,
        "the resume is due at the held tick, not past the fence at {:?}",
        app.world()
            .resource::<GmActionJournal>()
            .recovery_generations(),
    );
    // And it is stamped with the incarnation the fence just minted, which is
    // what a hand-inserted grant could not have told us.
    let grant = app
        .world()
        .resource::<GmActionJournal>()
        .grants()
        .iter()
        .find(|grant| grant.correlation.as_str() == "resume-1")
        .cloned()
        .expect("the owner sequenced the resume");
    assert_eq!(
        grant.recovery_generation,
        app.world()
            .resource::<GmLiveRestore>()
            .fence_generation()
            .expect("the fence was stamped"),
    );

    let facts = settle_facts(&mut app, &["resume-1"]);
    assert_eq!(fact(&facts, "resume-1").0, GmActionOutcome::Applied);
    assert!(!paused(&app), "the GM's own explicit resume ends the hold");
    app.update();
    assert_eq!(
        phase(&app),
        GmRestorePhase::Idle,
        "the reported restore clears once the GM resumes"
    );

    // The world is not merely marked running: it spends ticks again.
    let resumed_at = tick(&app);
    for _ in 0..30 {
        app.update();
    }
    assert!(
        tick(&app) > resumed_at,
        "the restored world runs on from the tick it was held at"
    );
}

/// A resume cannot be slipped underneath a restore that is still working: the
/// world it would run is the one about to be overwritten.
#[test]
fn a_resume_while_a_restore_is_working_is_refused_by_name() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Working");

    // Arm the restore without letting the driver run: the reducer applies the
    // request, and the very next ordered grant is the resume. Both go through
    // the owner's own rule, which is what puts them on one tick — an accepted
    // request holds the session, so the resume is scheduled at the boundary the
    // hold stopped rather than one tick past a clock that has stopped.
    let mut apply_ticks = Vec::new();
    for (correlation, action) in [
        (
            "restore-1",
            GmAction::RequestLiveRestore {
                candidate: slot_id.clone(),
            },
        ),
        ("resume-1", GmAction::SetSessionPaused { active: false }),
    ] {
        apply_ticks.push(propose(&mut app, correlation, "gm-1", action));
    }
    assert_eq!(
        apply_ticks[0], apply_ticks[1],
        "a resume pressed under a working restore is answered, not stranded"
    );
    let facts = settle_facts(&mut app, &["restore-1", "resume-1"]);

    assert_eq!(fact(&facts, "restore-1").0, GmActionOutcome::Applied);
    assert_eq!(
        fact(&facts, "resume-1"),
        (
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::LiveRestoreInProgress)
        )
    );
    assert!(paused(&app), "the hold survived the refused resume");
}

/// The case the feature exists for: something the candidate captured is GONE
/// from the live world by the time the rewind is asked for.
///
/// A destroyed hull is despawned outright, and so is a GM removal, so a
/// candidate's roster is routinely NOT a subset of the roster standing when the
/// GM presses Restore. Rewinding is exactly the act of putting those rows back,
/// and it must actually do so — and, whatever the answer, it must settle, or the
/// desk is left in a phase whose own Resume is refused by name.
#[test]
fn a_rewind_rebuilds_an_entity_the_live_world_has_since_lost() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Before the loss");
    for _ in 0..60 {
        app.update();
    }

    let lost = a_spawned_npc(&mut app);
    despawn_uuid(&mut app, &lost);
    assert!(
        !standing(&mut app, &lost),
        "the hull the checkpoint captured is gone from the live world"
    );

    let (result, _) = restore(&mut app, "restore-1", &slot_id);
    assert_eq!(result, GmActionOutcome::Applied);
    settle_phase(&mut app, READINESS_FRAME_BUDGET + 60);

    assert!(
        !phase(&app).in_flight(),
        "the restore settled rather than waiting on a hull nothing will respawn"
    );
    assert_eq!(phase(&app), GmRestorePhase::Restored, "{:?}", failure(&app));
    assert!(
        standing(&mut app, &lost),
        "the rewind rebuilt the hull the live world had lost"
    );
    assert!(paused(&app), "a restore never resumes the session");

    // And the desk is not stranded: the ordinary resume is admitted again.
    propose(
        &mut app,
        "resume-1",
        "gm-1",
        GmAction::SetSessionPaused { active: false },
    );
    let facts = settle_facts(&mut app, &["resume-1"]);
    assert_eq!(fact(&facts, "resume-1").0, GmActionOutcome::Applied);
    assert!(!paused(&app), "the GM's own explicit resume ends the hold");
}

/// A candidate this world can never house is REPORTED, not waited on forever.
///
/// The hold that makes a rewind safe also stops `FixedUpdate`, so a world that
/// is not ready to be overwritten will never become ready on its own. Without a
/// bound that is a session nobody can leave — every phase in flight refuses the
/// GM's Resume by name — so the wait ends in the ordinary failure lifecycle:
/// rolled back to the recovery checkpoint, held, and said out loud.
#[test]
fn a_candidate_this_world_can_never_house_is_reported_rather_than_waited_on() {
    let store = PeerStore::default();
    let mut app = build(store.clone());
    run_to_in_progress(&mut app);
    let slot_id = bookmark(&mut app, "Unhousable");
    for _ in 0..60 {
        app.update();
    }

    // Fault one: a captured hull leaves the live world. Fault two: the stored
    // candidate carries no spawn recipe for it, so nothing can build it back.
    let lost = a_spawned_npc(&mut app);
    despawn_uuid(&mut app, &lost);
    store.forget_spawn_recipes(&slot_id);

    let (result, _) = restore(&mut app, "restore-1", &slot_id);
    assert_eq!(result, GmActionOutcome::Applied);
    let held_tick = tick(&app);
    settle_phase(&mut app, READINESS_FRAME_BUDGET + 120);

    assert!(
        !phase(&app).in_flight(),
        "the wait is bounded: {:?} after {} frames",
        phase(&app),
        READINESS_FRAME_BUDGET + 120
    );
    assert_eq!(
        phase(&app),
        GmRestorePhase::RolledBack,
        "{:?}",
        failure(&app)
    );
    assert!(
        matches!(failure(&app), Some(GmRestoreFailure::LoadRefused { .. })),
        "{:?}",
        failure(&app)
    );
    assert!(paused(&app), "a reported failure leaves the world stopped");
    assert_eq!(
        tick(&app),
        held_tick,
        "the held world spent no ticks while it waited"
    );
    assert!(
        !standing(&mut app, &lost),
        "the rollback is the world as the request found it, loss and all"
    );

    // The session is recoverable: the GM's own resume is admitted again.
    propose(
        &mut app,
        "resume-1",
        "gm-1",
        GmAction::SetSessionPaused { active: false },
    );
    let facts = settle_facts(&mut app, &["resume-1"]);
    assert_eq!(fact(&facts, "resume-1").0, GmActionOutcome::Applied);
    assert!(
        !paused(&app),
        "the desk is never stranded in a failed phase"
    );
}
