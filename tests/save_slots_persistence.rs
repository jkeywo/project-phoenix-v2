//! Issue #865 review guards for the target-local save lifecycle.
//!
//! This is an integration test because a real continuation claim needs the
//! complete authoritative resource inventory. A bare `App` cannot stand in for
//! that inventory: the digest intentionally distinguishes absent collision or
//! asteroid state from present-but-empty state.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use project_phoenix::command_admission::log::HostSlot;
use project_phoenix::core::messages::{StationId, SystemId};
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::lockstep::{join_fleet, FleetLockstep, FleetRoster, FleetShip};
use project_phoenix::save_slots::{ContentCheck, StartState};
use project_phoenix::save_slots_lifecycle::PendingStoredRuns;
use project_phoenix::save_slots_store::{
    install_local_save_store, request_named_manual_save, stage_new_native_session_from_slot,
    NativeRestoreOutcome, NativeResumeRefusal, SaveSlotService,
};
use project_phoenix::ship::control_source::ControlSource;
use project_phoenix::ship::rating::BACKFILL_RATING;
use project_phoenix::ship_plugin::{ActiveStationRatings, ShipSystemControlSources};
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_tick::SimTick;
use project_phoenix::snapshot::{
    ready_to_restore, reconcile_world_layers, restore, LayerReconcileStatus, LoadRefusal, StoredRun,
};
use project_phoenix::startup_restore::RESTORE_DEADLINE_FRAMES;
use vellum_save::Store;

const WORLD: &str = "assets/worlds/duel.toml";
const SEED: u64 = 865_2026;
// Bevy's nanosecond-quantized 60 Hz period makes a nominal four-tick rendered
// frame establish at tick 3, then reach 7, 11, ...; 63 is the first shared
// boundary after decision 60 that both the one- and four-tick drivers hit.
const TARGET_TICK: u64 = 63;
const CAPTURE_INTERVAL_TICKS: u64 = 15;
const FLEET_WORLD: &str = "assets/worlds/probe_fleet_duel.toml";

#[derive(Clone, Default)]
struct RecordingStore {
    state: Arc<Mutex<RecordingState>>,
}

#[derive(Default)]
struct RecordingState {
    slots: BTreeMap<String, String>,
    writes: Vec<(String, String)>,
    fail_next_write: bool,
}

impl RecordingStore {
    fn writes(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().writes.clone()
    }

    fn contains(&self, slot: &str) -> bool {
        self.state.lock().unwrap().slots.contains_key(slot)
    }

    fn fail_next_write(&self) {
        self.state.lock().unwrap().fail_next_write = true;
    }

    fn seed(&self, slot: &str, contents: &str) {
        self.state
            .lock()
            .unwrap()
            .slots
            .insert(slot.to_string(), contents.to_string());
    }
}

impl Store for RecordingStore {
    type Error = String;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok(self.state.lock().unwrap().slots.get(slot).cloned())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        let mut state = self.state.lock().unwrap();
        if state.fail_next_write {
            state.fail_next_write = false;
            return Err("peer-local write refused".into());
        }
        state.slots.insert(slot.to_string(), contents.to_string());
        state.writes.push((slot.to_string(), contents.to_string()));
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

fn args_for_hull(ticks_per_frame: u32, hull: &str) -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        side_a: vec![hull.into()],
        side_b: vec!["destroyer".into()],
        dt: f64::from(ticks_per_frame) / 60.0,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn build(ticks_per_frame: u32, store: Option<RecordingStore>) -> App {
    build_for_hull(ticks_per_frame, "cruiser", store)
}

fn build_for_hull(ticks_per_frame: u32, hull: &str, store: Option<RecordingStore>) -> App {
    let mut app =
        build_headless_app(&args_for_hull(ticks_per_frame, hull)).expect("duel app builds");
    {
        let mut config = app
            .world_mut()
            .resource_mut::<project_phoenix::world::config::WorldConfig>();
        assert_eq!(config.global.sim_tick_hz, 60.0);
        config.global.autosave_interval_secs = 0.25;
        assert_eq!(
            config.global.checked_autosave_interval_ticks(),
            Some(CAPTURE_INTERVAL_TICKS)
        );
    }
    if let Some(store) = store {
        install_local_save_store(&mut app, store);
    }
    app.finish();
    app.cleanup();
    app
}

fn fleet_roster(local: HostSlot) -> FleetRoster {
    use project_phoenix::core::messages::StationId;

    FleetRoster::new(
        vec![
            FleetShip {
                host: HostSlot(1),
                ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
                crew: vec![(StationId("helm".into()), "Std".into())],
            },
            FleetShip {
                host: HostSlot(2),
                ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
                crew: vec![(StationId("tactical".into()), "Std".into())],
            },
        ],
        local,
    )
}

fn build_connected_fleet_peer(local: HostSlot, store: RecordingStore) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: FLEET_WORLD.into(),
        dt: 1.0 / 60.0,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    })
    .expect("fleet probe app builds");
    {
        let mut config = app
            .world_mut()
            .resource_mut::<project_phoenix::world::config::WorldConfig>();
        config.global.autosave_interval_secs = 0.25;
        assert_eq!(
            config.global.checked_autosave_interval_ticks(),
            Some(CAPTURE_INTERVAL_TICKS)
        );
    }
    // Fleet admission validates the frozen seats against each preloaded hull.
    // Headless startup reads from disk; supply the same real template that a
    // native/browser host preloads before accepting this explicit fleet roster.
    const HULL: &str = "assets/entities/alliance_cruiser.toml";
    let hull = project_phoenix::entities::include_resolve::load_entity_config(HULL)
        .expect("the saved fleet's authored cruiser template resolves");
    project_phoenix::entities::config_cache::insert_native_config(HULL.into(), hull);
    let roster = fleet_roster(local);
    assert!(
        project_phoenix::lockstep::crew::roster_crew_matches_hulls(app.world(), &roster),
        "the saved fleet's frozen stations and ratings must match its loaded hulls"
    );
    assert!(
        join_fleet(app.world_mut(), roster, 2),
        "the save fixture must join the fleet before exercising peer-local recovery"
    );
    let remote = if local == HostSlot(1) {
        HostSlot(2)
    } else {
        HostSlot(1)
    };
    app.world_mut()
        .resource_mut::<FleetLockstep>()
        .observe(remote, u64::MAX);
    install_local_save_store(&mut app, store);
    app.finish();
    app.cleanup();
    app
}

fn build_fleet_resume_peer(store: RecordingStore) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: FLEET_WORLD.into(),
        dt: 1.0 / 60.0,
        seed: Some(SEED.wrapping_add(1)),
        deterministic: true,
        ..Default::default()
    })
    .expect("fresh fleet probe app builds");
    install_local_save_store(&mut app, store);
    app.finish();
    app.cleanup();
    app
}

#[test]
fn non_default_hull_is_recorded_gated_and_restored_on_the_native_startup_path() {
    const DESTROYER: &str = "assets/entities/alliance_destroyer.toml";
    const CRUISER: &str = "assets/entities/alliance_cruiser.toml";

    let source_store = RecordingStore::default();
    let mut source = build_for_hull(1, "destroyer", Some(source_store.clone()));
    step_to(&mut source, 61);
    let text = source_store
        .read(project_phoenix::save_slots::AUTOSAVE_SLOT)
        .unwrap()
        .expect("the non-default hull produced an autosave");
    let run = StoredRun::from_ron(&text).expect("the autosave parses");
    let snapshot = run.snapshot.as_ref().expect("the autosave carries state");
    let boot = snapshot
        .state
        .boot_identity
        .as_ref()
        .expect("a current save carries its boot identity");
    assert_eq!(boot.selected_ship, DESTROYER);
    assert_eq!(
        source
            .world()
            .resource::<project_phoenix::lobby::SelectedShipResource>()
            .0,
        DESTROYER
    );

    // The same scenario and exact same Versions used to make this look safe.
    // The explicit boot gate now refuses before any restore is staged.
    let wrong_store = RecordingStore::default();
    wrong_store.seed(project_phoenix::save_slots::AUTOSAVE_SLOT, &text);
    let mut wrong = build_for_hull(1, "cruiser", Some(wrong_store));
    assert!(matches!(
        stage_new_native_session_from_slot(
            wrong.world_mut(),
            project_phoenix::save_slots::AUTOSAVE_SLOT,
            &run.versions,
            WORLD,
        ),
        Err(NativeResumeRefusal::WrongSelectedShip { saved, loaded })
            if saved == DESTROYER && loaded == CRUISER
    ));
    assert_eq!(tick(&wrong), 0, "a mismatch never becomes a live restore");

    let resumed_store = RecordingStore::default();
    resumed_store.seed(project_phoenix::save_slots::AUTOSAVE_SLOT, &text);
    let mut resumed = build_for_hull(1, "destroyer", Some(resumed_store));
    assert_eq!(
        stage_new_native_session_from_slot(
            resumed.world_mut(),
            project_phoenix::save_slots::AUTOSAVE_SLOT,
            &run.versions,
            WORLD,
        ),
        Ok(snapshot.tick)
    );
    let (_, restored_tick) = await_native_restore(&mut resumed, 1_000);
    assert_eq!(restored_tick, snapshot.tick);
    assert_eq!(
        resumed
            .world()
            .resource::<project_phoenix::lobby::SelectedShipResource>()
            .0,
        DESTROYER
    );
    assert_eq!(world_digest(resumed.world()), snapshot.digest);
}

#[test]
fn either_fleet_peer_can_start_its_save_as_an_independent_new_session() {
    let first_store = RecordingStore::default();
    let second_store = RecordingStore::default();
    let mut first = build_connected_fleet_peer(HostSlot(1), first_store.clone());
    let mut second = build_connected_fleet_peer(HostSlot(2), second_store.clone());

    step_to(&mut first, 16);
    step_to(&mut second, 16);
    assert!(first.world().contains_resource::<FleetLockstep>());
    assert!(second.world().contains_resource::<FleetLockstep>());

    for (local, source_store) in [(HostSlot(1), first_store), (HostSlot(2), second_store)] {
        let saved_text = source_store
            .read(project_phoenix::save_slots::AUTOSAVE_SLOT)
            .unwrap()
            .expect("each connected peer wrote its own rolling save");
        let saved_run = StoredRun::from_ron(&saved_text).expect("fleet save parses");
        let saved_snapshot = saved_run.snapshot.as_ref().expect("fleet save has state");
        let saved_boot = project_phoenix::snapshot::required_boot_identity(&saved_run).unwrap();
        assert_eq!(saved_boot.fleet.len(), 2);
        assert_eq!(saved_boot.fleet.local(), local);
        assert!(saved_boot
            .fleet
            .ships()
            .iter()
            .any(|ship| !ship.crew.is_empty()));

        let resumed_store = RecordingStore::default();
        resumed_store.seed(project_phoenix::save_slots::AUTOSAVE_SLOT, &saved_text);
        let mut resumed = build_fleet_resume_peer(resumed_store);
        assert_eq!(
            stage_new_native_session_from_slot(
                resumed.world_mut(),
                project_phoenix::save_slots::AUTOSAVE_SLOT,
                &saved_run.versions,
                FLEET_WORLD,
            ),
            Ok(saved_snapshot.tick)
        );
        assert!(
            !resumed.world().contains_resource::<FleetLockstep>(),
            "a local start must not recreate the old peer wait set"
        );
        let helm_station = StationId("helm".into());
        {
            let mut sessions = resumed
                .world_mut()
                .resource_mut::<project_phoenix::lobby::Sessions>();
            sessions
                .0
                .register("new-local-helm".into(), "New Local Helm".into())
                .expect("the fresh lobby accepts its local crew");
            sessions
                .0
                .set_station("new-local-helm", Some(helm_station.clone()));
            sessions
                .0
                .set_pending_rating(&helm_station, "Simplified".into());
        }

        let (_, restored_tick) = await_native_restore(&mut resumed, 1_000);
        assert_eq!(restored_tick, saved_snapshot.tick);
        assert_eq!(world_digest(resumed.world()), saved_snapshot.digest);
        let standalone = resumed.world().resource::<FleetRoster>();
        assert_eq!(standalone.len(), 2, "the saved ship topology is retained");
        assert_eq!(standalone.local(), local);
        assert!(standalone.ships().iter().all(|ship| ship.crew.is_empty()));
        assert!(!resumed.world().contains_resource::<FleetLockstep>());

        let helm_thrust = SystemId("helm-thrust".into());
        let helm_lateral = SystemId("helm-lateral-thrust".into());
        let seeded: Vec<_> = {
            let mut ships = resumed.world_mut().query::<(
                &project_phoenix::lockstep::FleetSlotOf,
                &ActiveStationRatings,
                &ShipSystemControlSources,
            )>();
            ships
                .iter(resumed.world())
                .map(|(slot, ratings, sources)| {
                    (
                        slot.0,
                        ratings.0.clone(),
                        sources.0.source_for(&helm_thrust),
                        sources.0.source_for(&helm_lateral),
                        sources
                            .0
                            .entries()
                            .all(|(_, source)| *source == ControlSource::Ai),
                    )
                })
                .collect()
        };
        let local_ship = seeded
            .iter()
            .find(|(slot, ..)| *slot == local)
            .expect("the restored roster spawns its local ship");
        assert_eq!(
            local_ship.1.get(&helm_station).map(String::as_str),
            Some("Simplified"),
            "the independent local ship takes the fresh lobby's selected rating"
        );
        assert_eq!(local_ship.2, ControlSource::Human);
        assert_eq!(
            local_ship.3,
            ControlSource::Ai,
            "Simplified automates only the authored lateral-thrust system"
        );
        let remote_ship = seeded
            .iter()
            .find(|(slot, ..)| *slot != local)
            .expect("the restored roster retains its remote ship");
        assert!(remote_ship
            .1
            .values()
            .all(|rating| rating == BACKFILL_RATING));
        assert!(
            remote_ship.4,
            "an independent saved fleet's remote ship begins fully AI-operated"
        );

        let continuation = restored_tick + 5;
        step_to(&mut resumed, continuation);
        assert_eq!(tick(&resumed), continuation);
    }
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimTick>().0
}

fn step_to(app: &mut App, target: u64) {
    while tick(app) < target {
        app.update();
    }
    assert_eq!(tick(app), target, "frame batching overshot the target");
}

fn boot_to_restore_point(snapshot: &project_phoenix::snapshot::PhoenixSnapshot) -> App {
    // No Store is injected: this is also the real-headless guard that automatic
    // scheduling does not accumulate undrainable StoredRuns during bootstrap.
    let mut app = build(1, None);
    for _ in 0..1_000 {
        app.update();
        match reconcile_world_layers(app.world_mut(), snapshot) {
            LayerReconcileStatus::Ready if ready_to_restore(app.world(), snapshot) => {
                assert!(app.world().resource::<PendingStoredRuns>().is_empty());
                return app;
            }
            LayerReconcileStatus::Failed(path) => {
                panic!("fresh duel layer reconciliation failed at {path}")
            }
            LayerReconcileStatus::Ready | LayerReconcileStatus::Waiting => {}
        }
    }
    panic!("fresh duel never reached the restore point");
}

fn await_native_restore(app: &mut App, max_frames: usize) -> (usize, u64) {
    for frame in 1..=max_frames {
        app.update();
        let outcome = app
            .world_mut()
            .resource_mut::<SaveSlotService>()
            .pop_restore_outcome();
        match outcome {
            Some(NativeRestoreOutcome::Applied { tick }) => return (frame, tick),
            Some(NativeRestoreOutcome::Failed { detail }) => {
                panic!("native startup restore failed: {detail}")
            }
            None => {}
        }
    }
    panic!("native startup restore did not finish within {max_frames} frames");
}

#[test]
fn a_destroyed_game_start_entity_keeps_its_boot_mapping_and_stays_destroyed() {
    let source_store = RecordingStore::default();
    let mut source = build(1, Some(source_store.clone()));
    step_to(&mut source, 2);

    let (player_entity, player_uuid) = {
        let mut query = source
            .world_mut()
            .query_filtered::<(Entity, &EntityUuid), With<project_phoenix::server_app::LocalShip>>(
            );
        let rows: Vec<_> = query
            .iter(source.world())
            .map(|(entity, uuid)| (entity, uuid.0.clone()))
            .collect();
        assert_eq!(rows.len(), 1, "the duel boots one local GameStart ship");
        rows.into_iter().next().unwrap()
    };
    assert!(source.world_mut().despawn(player_entity));

    let slot = request_named_manual_save(source.world_mut(), "destroyed GameStart").unwrap();
    for _ in 0..4 {
        source.update();
        if source_store.contains(&slot) {
            break;
        }
    }
    let text = source_store
        .read(&slot)
        .unwrap()
        .expect("the post-destruction manual save was written");
    let run = StoredRun::from_ron(&text).expect("the post-destruction save parses");
    let snapshot = run.snapshot.as_ref().expect("the save carries state");
    let boot = project_phoenix::snapshot::required_boot_identity(&run)
        .expect("a destroyed GameStart row remains a valid boot identity");
    assert!(
        boot.game_start_entity_uuids
            .iter()
            .any(|row| row.entity_uuid == player_uuid),
        "destroying an entity must not erase the GameStart mapping needed on resume"
    );
    assert!(
        snapshot
            .state
            .entities
            .iter()
            .all(|entity| entity.uuid != player_uuid),
        "the destroyed ship is correctly absent from captured live entities"
    );

    let resumed_store = RecordingStore::default();
    resumed_store.seed(project_phoenix::save_slots::AUTOSAVE_SLOT, &text);
    let mut resumed = build(1, Some(resumed_store));
    assert_eq!(
        stage_new_native_session_from_slot(
            resumed.world_mut(),
            project_phoenix::save_slots::AUTOSAVE_SLOT,
            &run.versions,
            WORLD,
        ),
        Ok(snapshot.tick),
        "a mapping absent from live snapshot rows is valid and stages"
    );
    let (_, restored_tick) = await_native_restore(&mut resumed, 1_000);
    assert_eq!(restored_tick, snapshot.tick);

    let remaining_uuids: Vec<_> = resumed
        .world_mut()
        .query::<&EntityUuid>()
        .iter(resumed.world())
        .map(|uuid| uuid.0.clone())
        .collect();
    assert!(
        !remaining_uuids.contains(&player_uuid),
        "the fresh GameStart ship takes its saved UUID, then restore despawns it as surplus"
    );
    assert_eq!(world_digest(resumed.world()), snapshot.digest);
}

#[test]
fn lifecycle_artifacts_are_frame_batch_independent_and_resume_at_t_plus_one() {
    let per_tick_store = RecordingStore::default();
    let per_four_store = RecordingStore::default();
    let mut per_tick = build(1, Some(per_tick_store.clone()));
    let mut per_four = build(4, Some(per_four_store.clone()));

    // Decision 60 is captured by both. On the four-tick host it happens while
    // the same rendered frame still has ticks 61..62 queued; none of that
    // catch-up backlog may enter the artifact.
    step_to(&mut per_tick, 61);
    step_to(&mut per_four, TARGET_TICK);

    let per_tick_decisions: Vec<_> = per_tick
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .map(|outcome| outcome.decision.clone())
        .collect();
    let per_four_decisions: Vec<_> = per_four
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .map(|outcome| outcome.decision.clone())
        .collect();
    assert_eq!(per_tick_decisions, per_four_decisions);
    assert!(!per_tick_decisions.is_empty());
    assert!(per_tick_decisions
        .windows(2)
        .all(|pair| pair[1].tick - pair[0].tick == CAPTURE_INTERVAL_TICKS));

    let per_tick_writes = per_tick_store.writes();
    let per_four_writes = per_four_store.writes();
    assert_eq!(per_tick_writes.len(), per_four_writes.len());
    // RON does not promise a byte-canonical order for hash-backed fields.
    // Decode and compare the entire StoredRun so every artifact field remains
    // covered without mistaking equivalent map iteration orders for drift.
    for (index, ((per_tick_slot, per_tick_artifact), (per_four_slot, per_four_artifact))) in
        per_tick_writes.iter().zip(&per_four_writes).enumerate()
    {
        let per_tick_run = StoredRun::from_ron(per_tick_artifact).expect("per-tick run parses");
        let per_four_run = StoredRun::from_ron(per_four_artifact).expect("per-four run parses");
        let per_tick_snapshot = per_tick_run.snapshot.as_ref().unwrap();
        let per_four_snapshot = per_four_run.snapshot.as_ref().unwrap();
        assert!(
            per_tick_run == per_four_run,
            "stored artifact {index} differs: slots {per_tick_slot:?}/{per_four_slot:?}, \
             ticks {}/{}, digests {:#018x}/{:#018x}, states_equal={}, versions_equal={}, \
             scenarios_equal={}, seeds_equal={}, snapshots_equal={}, commands_equal={}, \
             ledgers_equal={}",
            per_tick_snapshot.tick,
            per_four_snapshot.tick,
            per_tick_snapshot.digest,
            per_four_snapshot.digest,
            per_tick_snapshot.state == per_four_snapshot.state,
            per_tick_run.versions == per_four_run.versions,
            per_tick_run.scenario == per_four_run.scenario,
            per_tick_run.seed == per_four_run.seed,
            per_tick_run.snapshot == per_four_run.snapshot,
            per_tick_run.commands == per_four_run.commands,
            per_tick_run.ledger == per_four_run.ledger,
        );
    }
    for ((_, artifact), decision) in per_tick_writes.iter().zip(&per_tick_decisions) {
        let run = StoredRun::from_ron(artifact).expect("stored text is a canonical run");
        let stored = run.snapshot.as_ref().expect("a save carries a snapshot");
        assert_eq!(stored.tick, decision.tick.wrapping_add(1));
        assert_eq!(run.ledger.final_tick, decision.tick.wrapping_add(1));
        assert_eq!(stored.state.fixed_overstep_nanos, Some(0));
    }

    // Bring the per-tick source to the same boundary and prove different frame
    // batching did not alter the authoritative run either.
    step_to(&mut per_tick, TARGET_TICK);
    assert_eq!(
        world_digest(per_tick.world()),
        world_digest(per_four.world())
    );

    let latest = per_tick_store
        .read(project_phoenix::save_slots::AUTOSAVE_SLOT)
        .unwrap()
        .expect("rolling autosave exists");
    let run = StoredRun::from_ron(&latest).expect("autosave parses");
    let stored = run.snapshot.as_ref().expect("autosave carries state");
    assert_eq!(stored.tick, 61, "decision T=60 resumes at T+1");

    // A genuinely new app bootstraps the same scenario, then takes the stored
    // continuation through the existing snapshot path.
    let mut resumed = boot_to_restore_point(&stored.state);
    let report = restore(resumed.world_mut(), &stored.state);
    assert!(report.is_complete(), "restore gaps: {:?}", report.gaps);
    assert_eq!(tick(&resumed), stored.tick);
    assert_eq!(world_digest(resumed.world()), stored.digest);

    // Execute 61 and 62 exactly once. Re-executing decision tick 60 would
    // leave this app one step behind the two live sources (and change the fold).
    step_to(&mut resumed, TARGET_TICK);
    assert_eq!(
        world_digest(resumed.world()),
        world_digest(per_tick.world())
    );
    assert_eq!(
        world_digest(resumed.world()),
        world_digest(per_four.world())
    );

    // A manual request is peer-local even though both canonical apps continue
    // the same authoritative run. The requester captures at decision 63
    // (continuation 64); the other Store receives no write at all.
    let requester_only = request_named_manual_save(per_tick.world_mut(), "requester only").unwrap();
    let other_writes_before = per_four_store.writes().len();
    per_four.update();
    step_to(&mut per_tick, tick(&per_four));
    assert_eq!(tick(&per_tick), 67);
    assert_eq!(
        world_digest(per_tick.world()),
        world_digest(per_four.world())
    );
    assert!(per_tick_store.contains(&requester_only));
    assert!(!per_four_store.contains(&requester_only));
    assert_eq!(per_four_store.writes().len(), other_writes_before);
    let requester_run = StoredRun::from_ron(
        &per_tick_store
            .read(&requester_only)
            .unwrap()
            .expect("requester's manual run exists"),
    )
    .unwrap();
    assert_eq!(requester_run.snapshot.as_ref().unwrap().tick, 64);

    // Refuse exactly one backend write on the other peer. Its local catalogue
    // changes, but the control peer reaches the identical tick and digest.
    let refused = request_named_manual_save(per_four.world_mut(), "will fail").unwrap();
    per_four_store.fail_next_write();
    per_four.update();
    step_to(&mut per_tick, tick(&per_four));
    assert_eq!(tick(&per_tick), 71);
    assert!(!per_four_store.contains(&refused));
    assert!(!per_tick_store.contains(&refused));
    assert_eq!(
        world_digest(per_tick.world()),
        world_digest(per_four.world())
    );

    // Populate both peer catalogues with the same display text. Their opaque
    // identities and subsequent operations remain Store-local.
    let per_tick_slot =
        request_named_manual_save(per_tick.world_mut(), "shared display name").unwrap();
    let per_four_slot =
        request_named_manual_save(per_four.world_mut(), "shared display name").unwrap();
    assert_ne!(per_tick_slot, per_four_slot);
    per_four.update();
    step_to(&mut per_tick, tick(&per_four));
    assert_eq!(tick(&per_tick), 75);
    assert_eq!(
        world_digest(per_tick.world()),
        world_digest(per_four.world())
    );
    assert!(per_tick_store.contains(&per_tick_slot));
    assert!(!per_tick_store.contains(&per_four_slot));
    assert!(per_four_store.contains(&per_four_slot));
    assert!(!per_four_store.contains(&per_tick_slot));

    let selected_text = per_four_store
        .read(&per_four_slot)
        .unwrap()
        .expect("selected peer-local run exists");
    let selected_run = StoredRun::from_ron(&selected_text).unwrap();
    for (app, slot) in [(&per_tick, &per_tick_slot), (&per_four, &per_four_slot)] {
        let row = app
            .world()
            .resource::<SaveSlotService>()
            .list(&selected_run.versions, ContentCheck::Full)
            .unwrap()
            .into_iter()
            .find(|row| row.slot_id == *slot)
            .expect("each populated peer lists its own manual slot");
        assert_eq!(row.display_name, "shared display name");
    }
    let selected_export = per_four
        .world()
        .resource::<SaveSlotService>()
        .export(&per_four_slot)
        .unwrap();
    assert_eq!(
        StoredRun::from_ron(&selected_export).unwrap(),
        selected_run,
        "export routes through the selected slot rather than autosave"
    );

    let mut incompatible = selected_run.versions.clone();
    incompatible.format = incompatible.format.wrapping_add(1);
    assert!(matches!(
        per_four
            .world()
            .resource::<SaveSlotService>()
            .load(&per_four_slot, &incompatible),
        Err(LoadRefusal::Moved(vellum_save::Moved::Format { .. }))
    ));
    let refused_row = per_four
        .world()
        .resource::<SaveSlotService>()
        .list(&incompatible, ContentCheck::Full)
        .unwrap()
        .into_iter()
        .find(|row| row.slot_id == per_four_slot)
        .expect("incompatible selected row remains listed");
    assert!(matches!(
        refused_row.start,
        StartState::Refused(LoadRefusal::Moved(vellum_save::Moved::Format { .. }))
    ));

    per_tick
        .world_mut()
        .resource_mut::<SaveSlotService>()
        .delete(&per_tick_slot)
        .unwrap();
    assert!(!per_tick_store.contains(&per_tick_slot));
    assert!(per_tick_store.contains(&requester_only));
    assert!(per_four_store.contains(&per_four_slot));
    assert!(per_tick
        .world()
        .resource::<SaveSlotService>()
        .list(&selected_run.versions, ContentCheck::Full)
        .unwrap()
        .into_iter()
        .all(|row| row.slot_id != per_tick_slot));
    assert!(per_four
        .world()
        .resource::<SaveSlotService>()
        .list(&selected_run.versions, ContentCheck::Full)
        .unwrap()
        .into_iter()
        .any(|row| row.slot_id == per_four_slot));
}

#[test]
fn staged_restore_suppresses_bootstrap_and_rebases_periodic_cadence() {
    let source_store = RecordingStore::default();
    let mut source = build(1, Some(source_store.clone()));
    step_to(&mut source, 61);
    let seeded_text = source_store
        .read(project_phoenix::save_slots::AUTOSAVE_SLOT)
        .unwrap()
        .expect("source rolling autosave exists");
    let seeded_run = StoredRun::from_ron(&seeded_text).unwrap();
    let seeded_snapshot = seeded_run.snapshot.as_ref().unwrap();
    assert_eq!(seeded_snapshot.tick, 61);

    let fast_store = RecordingStore::default();
    let delayed_store = RecordingStore::default();
    fast_store.seed(project_phoenix::save_slots::AUTOSAVE_SLOT, &seeded_text);
    delayed_store.seed(project_phoenix::save_slots::AUTOSAVE_SLOT, &seeded_text);
    let mut fast = build(1, Some(fast_store.clone()));
    let mut delayed = build(4, Some(delayed_store.clone()));

    for app in [&mut fast, &mut delayed] {
        assert_eq!(
            stage_new_native_session_from_slot(
                app.world_mut(),
                project_phoenix::save_slots::AUTOSAVE_SLOT,
                &seeded_run.versions,
                WORLD,
            ),
            Ok(61)
        );
    }

    // Keep one fresh App in Lobby for several rendered frames. Restore cadence
    // must depend only on the saved continuation, never this bootstrap delay.
    delayed.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    let lobby_frames = RESTORE_DEADLINE_FRAMES as usize + 7;
    for _ in 0..lobby_frames {
        delayed.update();
    }
    assert_eq!(tick(&delayed), 0);
    assert!(delayed
        .world_mut()
        .resource_mut::<SaveSlotService>()
        .pop_restore_outcome()
        .is_none());
    delayed.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        4.0 / 60.0,
    )));

    let (fast_frames, fast_tick) = await_native_restore(&mut fast, 1_000);
    let (delayed_active_frames, delayed_tick) = await_native_restore(&mut delayed, 1_000);
    assert_eq!(fast_tick, 61);
    assert_eq!(delayed_tick, 61);
    assert!(lobby_frames + delayed_active_frames > fast_frames);
    assert_eq!(tick(&fast), 61);
    assert_eq!(tick(&delayed), 61);
    assert_eq!(world_digest(fast.world()), seeded_snapshot.digest);
    assert_eq!(world_digest(delayed.world()), seeded_snapshot.digest);

    // Neither bootstrap is allowed to replace the selected rolling save with
    // a transient RunStarted snapshot before PostUpdate applies restore.
    assert!(fast_store.writes().is_empty());
    assert!(delayed_store.writes().is_empty());
    assert_eq!(
        fast_store
            .read(project_phoenix::save_slots::AUTOSAVE_SLOT)
            .unwrap(),
        Some(seeded_text.clone())
    );
    assert_eq!(
        delayed_store
            .read(project_phoenix::save_slots::AUTOSAVE_SLOT)
            .unwrap(),
        Some(seeded_text)
    );

    step_to(&mut fast, 77);
    while tick(&delayed) < 77 {
        delayed.update();
    }
    let shared_live_tick = tick(&delayed);
    step_to(&mut fast, shared_live_tick);
    let fast_outcomes = fast
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .collect::<Vec<_>>();
    let delayed_outcomes = delayed
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .collect::<Vec<_>>();
    for outcomes in [&fast_outcomes, &delayed_outcomes] {
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].decision.tick, 76);
        assert_eq!(
            outcomes[0].decision.reason,
            project_phoenix::save_slots::CaptureReason::Periodic
        );
    }
    let fast_periodic = StoredRun::from_ron(&fast_store.writes()[0].1).unwrap();
    let delayed_periodic = StoredRun::from_ron(&delayed_store.writes()[0].1).unwrap();
    let fast_snapshot = fast_periodic.snapshot.as_ref().unwrap();
    let delayed_snapshot = delayed_periodic.snapshot.as_ref().unwrap();
    assert_eq!(fast_periodic.versions, delayed_periodic.versions);
    assert_eq!(fast_periodic.scenario, delayed_periodic.scenario);
    assert_eq!(fast_periodic.seed, delayed_periodic.seed);
    assert_eq!(fast_periodic.commands, delayed_periodic.commands);
    assert_eq!(fast_periodic.ledger, delayed_periodic.ledger);
    assert_eq!(fast_snapshot.tick, delayed_snapshot.tick);
    assert_eq!(fast_snapshot.digest, delayed_snapshot.digest);
    assert_eq!(fast_snapshot.state.tick, delayed_snapshot.state.tick);
    assert_eq!(fast_snapshot.state.fixed_overstep_nanos, Some(0));
    assert_eq!(delayed_snapshot.state.fixed_overstep_nanos, Some(0));
    // EntityState currently includes a non-digest projection whose exact
    // representation is rendered-frame dependent. The save lifecycle cannot
    // change that snapshot contract; the authoritative artifact identity is
    // the snapshot digest, which is equal here and remains equal below after
    // both restored worlds continue.
    assert_eq!(fast_periodic.ledger.final_tick, 77);
    assert_eq!(world_digest(fast.world()), world_digest(delayed.world()));

    delayed.update();
    step_to(&mut fast, tick(&delayed));
    assert_eq!(world_digest(fast.world()), world_digest(delayed.world()));
}

// These regressions drive the installed native adapter through real App updates.
// The fixture may hold a bootstrap prerequisite, but never calls the restore
// driver or its terminal cleanup directly.
fn startup_restore_record() -> StoredRun {
    let store = RecordingStore::default();
    let mut source = build(1, Some(store.clone()));
    step_to(&mut source, 61);
    StoredRun::from_ron(
        &store
            .read(project_phoenix::save_slots::AUTOSAVE_SLOT)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}

fn stage_restore_record(run: &StoredRun) -> (App, RecordingStore) {
    let store = RecordingStore::default();
    let text = run.to_ron().unwrap();
    store.seed(project_phoenix::save_slots::AUTOSAVE_SLOT, &text);
    let mut app = build(1, Some(store.clone()));
    stage_new_native_session_from_slot(
        app.world_mut(),
        project_phoenix::save_slots::AUTOSAVE_SLOT,
        &run.versions,
        WORLD,
    )
    .unwrap();
    (app, store)
}

#[derive(Resource)]
struct RestoreFixturePrepared;

fn freeze_restore_bootstrap(world: &mut World) {
    if !world
        .get_resource::<State<project_phoenix::core::messages::GamePhase>>()
        .is_some_and(|phase| phase.get() == &project_phoenix::core::messages::GamePhase::InProgress)
        || world.contains_resource::<RestoreFixturePrepared>()
    {
        return;
    }
    world.insert_resource(RestoreFixturePrepared);
    world.insert_resource(State::new(
        project_phoenix::core::messages::GamePhase::GameOver,
    ));
    world.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
}

fn await_restore_failure(app: &mut App) -> String {
    for _ in 0..=RESTORE_DEADLINE_FRAMES + 10 {
        app.update();
        match app
            .world_mut()
            .resource_mut::<SaveSlotService>()
            .pop_restore_outcome()
        {
            Some(NativeRestoreOutcome::Failed { detail }) => return detail,
            Some(other) => panic!("expected terminal failure, got {other:?}"),
            None => {}
        }
    }
    panic!("startup restore did not terminate");
}

fn assert_capture_resumes_once(app: &mut App, store: &RecordingStore) {
    assert!(!project_phoenix::startup_restore::is_pending(app.world()));
    assert!(!project_phoenix::save_slots_lifecycle::startup_restore_pending(app.world()));
    assert!(store.writes().is_empty(), "no bootstrap capture leaked");
    // Fixture isolation: test capture recovery on the retained World without
    // running a second OnEnter(InProgress), which would respawn its roster.
    app.insert_resource(State::new(
        project_phoenix::core::messages::GamePhase::InProgress,
    ));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    let manual = request_named_manual_save(app.world_mut(), "After restore").unwrap();
    let resume_tick = tick(app);
    for _ in 0..CAPTURE_INTERVAL_TICKS + 4 {
        app.update();
        assert!(app
            .world_mut()
            .resource_mut::<SaveSlotService>()
            .pop_restore_outcome()
            .is_none());
    }
    assert!(store.contains(&manual), "manual capture resumed");
    assert!(
        store.writes().iter().any(|(slot, text)| {
            slot == project_phoenix::save_slots::AUTOSAVE_SLOT
                && StoredRun::from_ron(text).unwrap().snapshot.unwrap().tick > resume_tick
        }),
        "periodic capture resumed"
    );
}

#[test]
fn startup_restore_finishes_after_game_over_during_roster_wait() {
    let run = startup_restore_record();
    let snapshot = run.snapshot.as_ref().unwrap();
    let held_uuid = snapshot.state.entities[0].uuid.clone();
    let ready_state = snapshot.state.clone();
    let (mut app, store) = stage_restore_record(&run);
    #[derive(Resource)]
    struct HeldRestoreEntity(Entity, String);
    app.add_systems(Update, move |world: &mut World| {
        if world.contains_resource::<RestoreFixturePrepared>()
            || !ready_to_restore(world, &ready_state)
        {
            return;
        }
        let entity = world
            .query::<(Entity, &EntityUuid)>()
            .iter(world)
            .find(|(_, uuid)| uuid.0 == held_uuid)
            .map(|(entity, _)| entity);
        if let Some(entity) = entity {
            world.insert_resource(RestoreFixturePrepared);
            world
                .resource_mut::<NextState<project_phoenix::core::messages::GamePhase>>()
                .set(project_phoenix::core::messages::GamePhase::GameOver);
            world.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
            world.entity_mut(entity).remove::<EntityUuid>();
            world.insert_resource(HeldRestoreEntity(entity, held_uuid.clone()));
        }
    });
    for _ in 0..10 {
        app.update();
        if app.world().contains_resource::<HeldRestoreEntity>() {
            break;
        }
    }
    assert!(app.world().contains_resource::<HeldRestoreEntity>());
    let refused = request_named_manual_save(app.world_mut(), "Still restoring").unwrap();
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<State<project_phoenix::core::messages::GamePhase>>()
            .get(),
        &project_phoenix::core::messages::GamePhase::GameOver,
        "App updates applied the production transition while restore still waited"
    );
    assert!(project_phoenix::save_slots_lifecycle::startup_restore_pending(app.world()));
    assert!(!store.contains(&refused));
    let HeldRestoreEntity(entity, uuid) = app
        .world_mut()
        .remove_resource::<HeldRestoreEntity>()
        .unwrap();
    app.world_mut().entity_mut(entity).insert(EntityUuid(uuid));
    let (_, restored_tick) = await_native_restore(&mut app, 10);
    assert_eq!(restored_tick, snapshot.tick);
    assert_eq!(world_digest(app.world()), snapshot.digest);
    assert!(
        !store.contains(&refused),
        "waiting manual intent is refused at resolution"
    );
    assert_capture_resumes_once(&mut app, &store);
}

#[test]
fn startup_restore_layer_wait_expires_and_resumes_capture() {
    use project_phoenix::world::server::{PendingWorldLayerChanges, WorldLayerChange};
    let run = startup_restore_record();
    let (mut app, store) = stage_restore_record(&run);
    app.add_systems(Update, |world: &mut World| {
        if world.contains_resource::<RestoreFixturePrepared>() {
            return;
        }
        freeze_restore_bootstrap(world);
        if world.contains_resource::<RestoreFixturePrepared>() {
            world
                .resource_mut::<PendingWorldLayerChanges>()
                .0
                .push(WorldLayerChange::Load {
                    path: "assets/worlds/held-restore-layer.toml".into(),
                    loader_path: None,
                });
        }
    });
    let detail = await_restore_failure(&mut app);
    assert!(detail.contains("never built"), "{detail}");
    // The fixture held the loader, not a real content error. Release it after
    // proving the wait cannot avoid the driver's deadline.
    app.world_mut()
        .resource_mut::<PendingWorldLayerChanges>()
        .0
        .clear();
    assert_capture_resumes_once(&mut app, &store);
}

#[test]
fn startup_restore_unrebuildable_roster_expires_after_game_over() {
    let mut run = startup_restore_record();
    let snapshot = run.snapshot.as_mut().unwrap();
    let mut missing = snapshot.state.entities[0].clone();
    missing.uuid = "unrebuildable-restored-entity".into();
    missing.spawn = None;
    snapshot.state.entities.push(missing);
    let (mut app, store) = stage_restore_record(&run);
    app.add_systems(Update, freeze_restore_bootstrap);
    let detail = await_restore_failure(&mut app);
    assert!(detail.contains("never built"), "{detail}");
    assert_capture_resumes_once(&mut app, &store);
}

#[test]
fn startup_restore_failed_layer_is_terminal_without_retry() {
    use project_phoenix::world::server::{WorldLayerMap, WorldRuntime};
    const FAILED_LAYER: &str = "assets/worlds/refused-restore-layer.toml";
    let mut run = startup_restore_record();
    run.snapshot
        .as_mut()
        .unwrap()
        .state
        .layer_flags
        .push(project_phoenix::snapshot::LayerFlags {
            path: FAILED_LAYER.into(),
            loader_path: None,
            declared_entity_uuids: vec![],
            owned_objective_ids: vec![],
            flags: Default::default(),
        });
    let (mut app, store) = stage_restore_record(&run);
    app.add_systems(Update, |world: &mut World| {
        if world.contains_resource::<RestoreFixturePrepared>() {
            return;
        }
        freeze_restore_bootstrap(world);
        if world.contains_resource::<RestoreFixturePrepared>() {
            world
                .resource_mut::<WorldLayerMap>()
                .0
                .insert(FAILED_LAYER.into(), WorldRuntime::default());
        }
    });
    let detail = await_restore_failure(&mut app);
    assert!(detail.contains(FAILED_LAYER), "{detail}");
    assert!(!app.world().resource::<WorldLayerMap>().0[FAILED_LAYER].is_active);
    assert_capture_resumes_once(&mut app, &store);
}

#[test]
fn startup_restore_digest_failure_cancels_without_rolling_back() {
    let mut run = startup_restore_record();
    let captured_tick = run.snapshot.as_ref().unwrap().tick;
    run.snapshot.as_mut().unwrap().digest ^= 1;
    let (mut app, store) = stage_restore_record(&run);
    let detail = await_restore_failure(&mut app);
    assert!(detail.contains("did not match saved digest"), "{detail}");
    assert_eq!(
        tick(&app),
        captured_tick,
        "failed verification does not undo the write"
    );
    assert_capture_resumes_once(&mut app, &store);
}

#[test]
fn startup_restore_incomplete_report_fails_even_with_matching_digest() {
    let mut run = startup_restore_record();
    let snapshot = run.snapshot.as_mut().unwrap();
    let triggers = &mut snapshot.state.scenario.as_mut().unwrap().triggers;
    assert!(!triggers.is_empty());
    triggers.push(triggers[0].clone());
    // Deliberately supply the digest of this incomplete write: a matching
    // digest alone must never turn the report's unresolved gap into success.
    let mut reference = boot_to_restore_point(&snapshot.state);
    let report = restore(reference.world_mut(), &snapshot.state);
    assert!(!report.is_complete());
    snapshot.digest = world_digest(reference.world());
    let (mut app, store) = stage_restore_record(&run);
    let detail = await_restore_failure(&mut app);
    assert!(detail.contains("unresolved gap"), "{detail}");
    assert_capture_resumes_once(&mut app, &store);
}

#[test]
fn startup_restore_rebuilds_a_missing_dynamic_entity_at_expiry() {
    let mut run = startup_restore_record();
    let snapshot = run.snapshot.as_mut().unwrap();
    let mut reference = boot_to_restore_point(&snapshot.state);
    let mut spawned = snapshot
        .state
        .entities
        .iter()
        .find(|row| row.spawn.is_some())
        .expect("duel's script-spawned opponent carries a rebuild recipe")
        .clone();
    spawned.uuid = "saved-dynamic-reinforcement".into();
    snapshot.state.entities.push(spawned);
    let report = restore(reference.world_mut(), &snapshot.state);
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(report.entities_spawned, 1);
    snapshot.digest = world_digest(reference.world());
    let expected_digest = snapshot.digest;
    let expected_tick = snapshot.tick;

    let (mut app, store) = stage_restore_record(&run);
    app.add_systems(Update, freeze_restore_bootstrap);
    let (frames, restored_tick) =
        await_native_restore(&mut app, RESTORE_DEADLINE_FRAMES as usize + 10);
    assert!(
        frames >= RESTORE_DEADLINE_FRAMES as usize,
        "rebuilding waits for the bootstrap budget"
    );
    assert_eq!(restored_tick, expected_tick);
    assert_eq!(world_digest(app.world()), expected_digest);
    assert!(app
        .world_mut()
        .query::<&EntityUuid>()
        .iter(app.world())
        .any(|uuid| uuid.0 == "saved-dynamic-reinforcement"));
    assert_capture_resumes_once(&mut app, &store);
}
