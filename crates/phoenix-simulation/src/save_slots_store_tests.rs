use super::*;
use std::sync::{Arc, Mutex};

use crate::save_slots::{CaptureReason, ManualSaveRefusalReason, SaveSlotKind, StartState};

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_save_directory_claim_is_exclusive_until_drop_and_not_a_slot() {
    let dir =
        std::env::temp_dir().join(format!("phoenix-native-save-claim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let first = NativeSaveDirectoryClaim::try_acquire(&dir)
        .expect("the first native host claims a new directory");
    assert!(dir.join(NATIVE_SAVE_DIRECTORY_LOCK_FILE).is_file());
    assert!(
        vellum_save::Store::slots(&vellum_save::FileStore::new(&dir))
            .expect("the lock sentinel does not break catalogue listing")
            .is_empty()
    );

    let second = NativeSaveDirectoryClaim::try_acquire(&dir)
        .expect_err("another native host cannot share the directory");
    assert_eq!(second.kind(), std::io::ErrorKind::WouldBlock);

    drop(first);
    let reclaimed = NativeSaveDirectoryClaim::try_acquire(&dir)
        .expect("closing the first host releases the directory claim");
    drop(reclaimed);
    let _ = std::fs::remove_dir_all(dir);
}

#[derive(Clone, Default)]
struct FakeStore {
    state: Arc<Mutex<FakeState>>,
}

#[derive(Default)]
struct FakeState {
    slots: BTreeMap<String, String>,
    fail_writes: bool,
}

impl FakeStore {
    fn fail_writes(&self) {
        self.state.lock().unwrap().fail_writes = true;
    }

    fn contains(&self, slot: &str) -> bool {
        self.state.lock().unwrap().slots.contains_key(slot)
    }
}

impl vellum_save::Store for FakeStore {
    type Error = String;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok(self.state.lock().unwrap().slots.get(slot).cloned())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        let mut state = self.state.lock().unwrap();
        if state.fail_writes {
            return Err("peer-local write refused".to_string());
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

fn versions() -> vellum_save::Versions {
    vellum_save::Versions::new(7, "rules-a", 0x1234)
}

fn run(tick: u64, current: vellum_save::Versions) -> StoredRun {
    run_in_scenario(tick, "assets/worlds/probe.toml", current)
}

fn run_in_scenario(tick: u64, scenario: &str, current: vellum_save::Versions) -> StoredRun {
    crate::snapshot::run_for(
        crate::snapshot::PhoenixSnapshot {
            tick,
            boot_identity: Some(crate::snapshot::BootIdentity {
                selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                fleet: crate::lockstep::FleetRoster::default(),
                game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                    authored_index: 0,
                    entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                }],
            }),
            ..Default::default()
        },
        0xfeed,
        42,
        scenario,
        current,
    )
}

fn pending(tick: u64, slot: CaptureSlot, current: vellum_save::Versions) -> PendingStoredRun {
    PendingStoredRun {
        decision: CaptureDecision {
            tick,
            reason: if matches!(slot, CaptureSlot::RollingAutosave) {
                CaptureReason::Periodic
            } else {
                CaptureReason::Manual
            },
            slot,
        },
        run: run(tick.wrapping_add(1), current),
    }
}

fn app_with(store: FakeStore) -> App {
    let mut app = App::new();
    app.init_resource::<PendingStoredRuns>();
    app.insert_resource(crate::lobby::SelectedShipResource(
        "assets/entities/alliance_cruiser.toml".into(),
    ));
    app.init_resource::<crate::lockstep::FleetRoster>();
    install_local_save_store(&mut app, store);
    app
}

#[test]
fn two_apps_drain_identical_automatic_ticks_but_a_failure_is_peer_local() {
    let good_store = FakeStore::default();
    let bad_store = FakeStore::default();
    let mut good = app_with(good_store.clone());
    let mut bad = app_with(bad_store.clone());
    good.insert_resource(crate::sim_tick::SimTick(700));
    bad.insert_resource(crate::sim_tick::SimTick(700));
    let good_digest_before = crate::sim_digest::world_digest(good.world());
    let bad_digest_before = crate::sim_digest::world_digest(bad.world());

    // Both peers begin with their own successful record. The later backend
    // failure therefore cannot be mistaken for an empty/unconfigured peer.
    let good_manual = request_named_manual_save(good.world_mut(), "local baseline").unwrap();
    let bad_manual = request_named_manual_save(bad.world_mut(), "local baseline").unwrap();
    good.world_mut()
        .resource_mut::<PendingStoredRuns>()
        .push_back(pending(
            0,
            CaptureSlot::Manual(good_manual.clone()),
            versions(),
        ));
    bad.world_mut()
        .resource_mut::<PendingStoredRuns>()
        .push_back(pending(
            0,
            CaptureSlot::Manual(bad_manual.clone()),
            versions(),
        ));
    good.update();
    bad.update();
    assert!(good_store.contains(&good_manual));
    assert!(bad_store.contains(&bad_manual));
    bad_store.fail_writes();

    for tick in [1, 31, 61] {
        good.world_mut()
            .resource_mut::<PendingStoredRuns>()
            .push_back(pending(tick, CaptureSlot::RollingAutosave, versions()));
        bad.world_mut()
            .resource_mut::<PendingStoredRuns>()
            .push_back(pending(tick, CaptureSlot::RollingAutosave, versions()));
    }
    good.update();
    bad.update();

    let good_ticks: Vec<_> = good
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .skip(1)
        .map(|outcome| outcome.decision.tick)
        .collect();
    let bad_ticks: Vec<_> = bad
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .skip(1)
        .map(|outcome| outcome.decision.tick)
        .collect();
    assert_eq!(good_ticks, [1, 31, 61]);
    assert_eq!(bad_ticks, good_ticks);
    assert!(good
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .skip(1)
        .all(|outcome| outcome.result.is_ok()));
    assert!(bad
        .world()
        .resource::<SaveSlotService>()
        .outcomes()
        .skip(1)
        .all(|outcome| outcome.result.is_err()));
    assert!(good_store.contains(save_slots::AUTOSAVE_SLOT));
    assert!(!bad_store.contains(save_slots::AUTOSAVE_SLOT));
    assert_eq!(good.world().resource::<crate::sim_tick::SimTick>().0, 700);
    assert_eq!(bad.world().resource::<crate::sim_tick::SimTick>().0, 700);
    assert_eq!(
        crate::sim_digest::world_digest(good.world()),
        good_digest_before
    );
    assert_eq!(
        crate::sim_digest::world_digest(bad.world()),
        bad_digest_before
    );
    assert_eq!(
        crate::snapshot::load_from(&good_store, save_slots::AUTOSAVE_SLOT, &versions())
            .unwrap()
            .ledger
            .final_tick,
        62
    );
}

#[test]
fn manual_names_deletion_and_export_stay_inside_the_requesting_peer() {
    let first_store = FakeStore::default();
    let second_store = FakeStore::default();
    let mut first = app_with(first_store.clone());
    let mut second = app_with(second_store.clone());

    let first_id = request_named_manual_save(first.world_mut(), "Bridge save").unwrap();
    let second_id = request_named_manual_save(second.world_mut(), "Bridge save").unwrap();
    assert_ne!(first_id, second_id, "display text is never slot identity");
    first
        .world_mut()
        .resource_mut::<PendingStoredRuns>()
        .push_back(pending(
            9,
            CaptureSlot::Manual(first_id.clone()),
            versions(),
        ));
    second
        .world_mut()
        .resource_mut::<PendingStoredRuns>()
        .push_back(pending(
            9,
            CaptureSlot::Manual(second_id.clone()),
            versions(),
        ));
    first.update();
    second.update();

    assert!(first_store.contains(&first_id));
    assert!(!second_store.contains(&first_id));
    assert!(second_store.contains(&second_id));
    assert!(!first_store.contains(&second_id));

    let service = first.world().resource::<SaveSlotService>();
    let rows = service.list(&versions(), ContentCheck::Full).unwrap();
    let row = rows.iter().find(|row| row.slot_id == first_id).unwrap();
    assert_eq!(row.display_name, "Bridge save");
    assert_eq!(row.kind, SaveSlotKind::Manual);
    let exported = service.export(&first_id).unwrap();
    assert_eq!(StoredRun::from_ron(&exported).unwrap(), run(10, versions()));
    let second_export = second
        .world()
        .resource::<SaveSlotService>()
        .export(&second_id)
        .unwrap();
    assert_eq!(
        StoredRun::from_ron(&second_export).unwrap(),
        run(10, versions())
    );

    first
        .world_mut()
        .resource_mut::<SaveSlotService>()
        .delete(&first_id)
        .unwrap();
    assert!(!first_store.contains(&first_id));
    assert!(second_store.contains(&second_id));
    assert!(second
        .world()
        .resource::<SaveSlotService>()
        .list(&versions(), ContentCheck::Full)
        .unwrap()
        .iter()
        .any(|row| row.slot_id == second_id));
}

#[test]
fn refused_manual_capture_clears_reservation_and_emits_once() {
    let store = FakeStore::default();
    let mut app = app_with(store);
    let slot_id = request_named_manual_save(app.world_mut(), "pending name").unwrap();
    assert!(app
        .world()
        .resource::<SaveSlotService>()
        .manual_names
        .contains_key(&slot_id));

    crate::save_slots_lifecycle::begin_startup_restore(app.world_mut());
    app.update();

    {
        let service = app.world_mut().resource_mut::<SaveSlotService>();
        assert!(!service.manual_names.contains_key(&slot_id));
        assert_eq!(
            service.manual_refusals.iter().cloned().collect::<Vec<_>>(),
            vec![RefusedManualSave {
                tick: 0,
                slot_id: slot_id.clone(),
                reason: ManualSaveRefusalReason::StartupRestorePending,
            }]
        );
    }
    let mut service = app.world_mut().resource_mut::<SaveSlotService>();
    assert_eq!(service.pop_manual_refusal().unwrap().slot_id, slot_id);
    assert!(service.pop_manual_refusal().is_none());
}

#[test]
fn incompatibility_blocks_load_and_a_compatible_record_restores_a_fresh_app() {
    let store = FakeStore::default();
    let mut source = app_with(store.clone());
    source
        .world_mut()
        .resource_mut::<PendingStoredRuns>()
        .push_back(pending(27, CaptureSlot::RollingAutosave, versions()));
    source.update();

    let incompatible = vellum_save::Versions::new(8, "rules-a", 0x1234);
    assert!(matches!(
        source
            .world()
            .resource::<SaveSlotService>()
            .load(save_slots::AUTOSAVE_SLOT, &incompatible),
        Err(LoadRefusal::Moved(_))
    ));
    let row = source
        .world()
        .resource::<SaveSlotService>()
        .list(&incompatible, ContentCheck::Full)
        .unwrap()
        .remove(0);
    assert!(matches!(row.start, StartState::Refused(_)));

    let loaded = source
        .world()
        .resource::<SaveSlotService>()
        .load(save_slots::AUTOSAVE_SLOT, &versions())
        .unwrap();
    let snapshot = loaded.snapshot.unwrap().state;
    let mut fresh = App::new();
    let report = crate::snapshot::restore(fresh.world_mut(), &snapshot);
    assert!(report.is_complete());
    assert_eq!(fresh.world().resource::<crate::sim_tick::SimTick>().0, 28);
}

#[test]
fn outcome_history_is_a_bounded_recent_ring() {
    let store = FakeStore::default();
    let mut service = SaveSlotService::new(store);
    let total = MAX_SAVE_WRITE_OUTCOMES as u64 + 3;

    for tick in 0..total {
        service.persist(pending(tick, CaptureSlot::RollingAutosave, versions()));
    }

    let retained: Vec<_> = service
        .outcomes()
        .map(|outcome| outcome.decision.tick)
        .collect();
    assert_eq!(retained.len(), MAX_SAVE_WRITE_OUTCOMES);
    assert_eq!(retained.first(), Some(&3));
    assert_eq!(retained.last(), Some(&(total - 1)));
}

#[test]
fn native_catalogue_defers_only_other_scenario_content() {
    const OTHER_SLOT: &str = "00000000-0000-4000-8000-000000000099";
    let store = FakeStore::default();
    let current = versions();
    crate::snapshot::save_to(
        &store,
        save_slots::AUTOSAVE_SLOT,
        &run_in_scenario(10, "assets/worlds/current.toml", current.clone()),
    )
    .unwrap();
    crate::snapshot::save_to(
        &store,
        OTHER_SLOT,
        &run_in_scenario(
            11,
            "assets/worlds/other.toml",
            vellum_save::Versions::new(7, "rules-a", 0x9999),
        ),
    )
    .unwrap();
    let service = SaveSlotService::new(store);

    let rows = service
        .list_for_loaded_scenario(&current, "assets/worlds/current.toml")
        .unwrap();
    assert!(matches!(rows[0].start, StartState::Ready));
    let other = rows.iter().find(|row| row.slot_id == OTHER_SLOT).unwrap();
    assert_eq!(other.start, StartState::ContentDeferred);
}

#[test]
fn native_new_session_staging_runs_full_gate_and_refuses_live_restore() {
    let store = FakeStore::default();
    crate::snapshot::save_to(&store, save_slots::AUTOSAVE_SLOT, &run(27, versions())).unwrap();
    let mut app = app_with(store);
    app.insert_resource(crate::sim_tick::SimTick(0));

    let incompatible = vellum_save::Versions::new(8, "rules-a", 0x1234);
    assert!(matches!(
        stage_new_native_session_from_slot(
            app.world_mut(),
            save_slots::AUTOSAVE_SLOT,
            &incompatible,
            "assets/worlds/probe.toml",
        ),
        Err(NativeResumeRefusal::Load(LoadRefusal::Moved(_)))
    ));
    assert!(matches!(
        stage_new_native_session_from_slot(
            app.world_mut(),
            save_slots::AUTOSAVE_SLOT,
            &versions(),
            "assets/worlds/different.toml",
        ),
        Err(NativeResumeRefusal::WrongScenario { .. })
    ));
    assert_eq!(
        stage_new_native_session_from_slot(
            app.world_mut(),
            save_slots::AUTOSAVE_SLOT,
            &versions(),
            "assets/worlds/probe.toml",
        ),
        Ok(27)
    );
    assert!(startup_restore::is_pending(app.world()));

    let live_store = FakeStore::default();
    crate::snapshot::save_to(&live_store, save_slots::AUTOSAVE_SLOT, &run(27, versions())).unwrap();
    let mut live = app_with(live_store);
    live.insert_resource(crate::sim_tick::SimTick(9));
    assert_eq!(
        stage_new_native_session_from_slot(
            live.world_mut(),
            save_slots::AUTOSAVE_SLOT,
            &versions(),
            "assets/worlds/probe.toml",
        ),
        Err(NativeResumeRefusal::LiveSession { tick: 9 })
    );
}
