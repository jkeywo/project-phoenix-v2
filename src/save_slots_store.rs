//! Peer-local persistence adapter for the fixed-tick save lifecycle (#865).
//!
//! [`crate::save_slots_lifecycle`] decides *when* to capture and leaves each
//! [`PendingStoredRun`](crate::save_slots_lifecycle::PendingStoredRun) in a
//! FIFO. This module is the target-local consumer: an adapter is installed with
//! [`install_local_save_store`], then `PostUpdate` writes every captured record
//! after the fixed schedule has finished. A browser keeps its existing
//! `LocalStorage` bridge adapter; a native host installs `FileStore`; headless
//! and balance runs install nothing unless a test or embedding explicitly asks
//! for persistence.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;

use crate::save_slots::{
    self, CaptureDecision, CaptureSlot, CatalogueError, ContentCheck, RefusedManualSave,
    SaveSlotEntry,
};
use crate::save_slots_lifecycle::{
    request_manual_save, PendingStoredRun, PendingStoredRuns, RefusedManualSaves,
    SaveCaptureConsumer,
};
use crate::snapshot::{LoadRefusal, StoredRun};
use crate::startup_restore::{self, RestoreFailure, RestoreOutcome};

/// Stable sentinel whose open handle carries the native host's exclusive claim.
///
/// It deliberately is not a `.ron` file, so `vellum_save::FileStore` never
/// mistakes it for a save slot. The file remains after shutdown: removing a
/// lock file while it is held can let another process create a different file
/// at the same path and acquire a second, unrelated lock.
#[cfg(not(target_arch = "wasm32"))]
const NATIVE_SAVE_DIRECTORY_LOCK_FILE: &str = ".phoenix-host.lock";

/// Process-lifetime ownership of one native save directory.
///
/// Dropping the token closes the file and releases the operating-system lock.
/// Callers must therefore retain it for as long as any `FileStore` backed by
/// the directory can be read or written.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct NativeSaveDirectoryClaim {
    _file: std::fs::File,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeSaveDirectoryClaim {
    /// Create the directory if necessary and try to claim it without blocking.
    pub fn try_acquire(root: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(NATIVE_SAVE_DIRECTORY_LOCK_FILE))?;
        file.try_lock().map_err(std::io::Error::from)?;
        Ok(Self { _file: file })
    }
}

/// Object-safe, error-normalising view of `vellum_save::Store`.
///
/// `Store` deliberately keeps its backend error as an associated type. The
/// native app needs a type-erased Resource so tests, storefronts, and the
/// shipped `FileStore` can all inject one peer-private backend. Normalising only
/// the *adapter error* to text preserves the structural catalogue errors above
/// it and does not add a compatibility gate.
pub trait LocalSaveStore: Send + Sync + 'static {
    fn read(&self, slot: &str) -> Result<Option<String>, String>;
    fn write(&self, slot: &str, contents: &str) -> Result<(), String>;
    fn remove(&self, slot: &str) -> Result<(), String>;
    fn slots(&self) -> Result<Vec<String>, String>;
}

impl<S> LocalSaveStore for S
where
    S: vellum_save::Store + Send + Sync + 'static,
{
    fn read(&self, slot: &str) -> Result<Option<String>, String> {
        vellum_save::Store::read(self, slot).map_err(|error| error.to_string())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), String> {
        vellum_save::Store::write(self, slot, contents).map_err(|error| error.to_string())
    }

    fn remove(&self, slot: &str) -> Result<(), String> {
        vellum_save::Store::remove(self, slot).map_err(|error| error.to_string())
    }

    fn slots(&self) -> Result<Vec<String>, String> {
        vellum_save::Store::slots(self).map_err(|error| error.to_string())
    }
}

struct StoreView<'a>(&'a dyn LocalSaveStore);

impl vellum_save::Store for StoreView<'_> {
    type Error = String;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        self.0.read(slot)
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        self.0.write(slot, contents)
    }

    fn remove(&self, slot: &str) -> Result<(), Self::Error> {
        self.0.remove(slot)
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        self.0.slots()
    }
}

/// Local result of one queued write. It is deliberately presentation state:
/// neither this FIFO nor the store resource participates in mesh or digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveWriteOutcome {
    pub decision: CaptureDecision,
    pub result: Result<(), CatalogueError>,
}

/// Enough recent local writes for an operator surface to catch up after a
/// stalled frame without retaining one entry per autosave for the process's
/// lifetime. This is presentation history, never authoritative state.
const MAX_SAVE_WRITE_OUTCOMES: usize = 64;

/// Terminal result of a startup-only native restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeRestoreOutcome {
    Applied { tick: u64 },
    Failed { detail: String },
}

/// Why a local slot could not be staged into a newly built native App.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeResumeRefusal {
    NoStore,
    LiveSession { tick: u64 },
    Load(LoadRefusal),
    WrongScenario { saved: String, loaded: String },
    WrongSelectedShip { saved: String, loaded: String },
    WrongFleet,
    AlreadyStaged,
}

impl std::fmt::Display for NativeResumeRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoStore => formatter.write_str("no local save Store is installed"),
            Self::LiveSession { tick } => write!(
                formatter,
                "live-session restore is unavailable; this App has reached tick {tick}"
            ),
            Self::Load(refusal) => write!(formatter, "{refusal}"),
            Self::WrongScenario { saved, loaded } => write!(
                formatter,
                "the save belongs to {saved:?}, but this new session loaded {loaded:?}"
            ),
            Self::WrongSelectedShip { saved, loaded } => write!(
                formatter,
                "the save requires hull {saved:?}, but this new session loaded {loaded:?}"
            ),
            Self::WrongFleet => formatter.write_str(
                "the save's frozen fleet differs from the fleet already staged for this session",
            ),
            Self::AlreadyStaged => {
                formatter.write_str("a local save is already staged for this new session")
            }
        }
    }
}

#[derive(Resource, Default)]
struct NativeStartupManualSlots(VecDeque<String>);

/// One peer's store, manual-name reservations, and local write outcomes.
#[derive(Resource)]
pub struct SaveSlotService {
    store: Box<dyn LocalSaveStore>,
    manual_names: BTreeMap<String, String>,
    outcomes: VecDeque<SaveWriteOutcome>,
    manual_refusals: VecDeque<RefusedManualSave>,
    restore_outcomes: VecDeque<NativeRestoreOutcome>,
}

impl std::fmt::Debug for SaveSlotService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SaveSlotService")
            .field("manual_names", &self.manual_names)
            .field("outcomes", &self.outcomes)
            .field("manual_refusals", &self.manual_refusals)
            .finish_non_exhaustive()
    }
}

impl SaveSlotService {
    pub fn new(store: impl LocalSaveStore) -> Self {
        Self {
            store: Box::new(store),
            manual_names: BTreeMap::new(),
            outcomes: VecDeque::new(),
            manual_refusals: VecDeque::new(),
            restore_outcomes: VecDeque::new(),
        }
    }

    fn store(&self) -> StoreView<'_> {
        StoreView(self.store.as_ref())
    }

    /// Reserve an opaque Store-safe id while retaining arbitrary display text
    /// locally until its next-fixed-tick capture reaches this adapter.
    pub fn reserve_manual(&mut self, display_name: impl Into<String>) -> String {
        let slot_id = save_slots::new_manual_slot_id();
        self.manual_names
            .insert(slot_id.clone(), display_name.into());
        slot_id
    }

    pub fn list(
        &self,
        current: &vellum_save::Versions,
        content_check: ContentCheck,
    ) -> Result<Vec<SaveSlotEntry>, CatalogueError> {
        save_slots::list_slots_with_content_check(&self.store(), current, content_check)
    }

    /// List against one fully loaded scenario. Other scenarios retain hard
    /// format/rules/damage failures, but their content decision stays deferred
    /// until a new App loads that row's own world.
    pub fn list_for_loaded_scenario(
        &self,
        current: &vellum_save::Versions,
        loaded_scenario: &str,
    ) -> Result<Vec<SaveSlotEntry>, CatalogueError> {
        let mut entries = self.list(current, ContentCheck::Full)?;
        defer_other_scenario_content(&mut entries, loaded_scenario);
        Ok(entries)
    }

    pub fn rename(&self, slot_id: &str, display_name: &str) -> Result<(), CatalogueError> {
        save_slots::rename_slot(&self.store(), slot_id, display_name)
    }

    pub fn delete(&mut self, slot_id: &str) -> Result<(), CatalogueError> {
        self.manual_names.remove(slot_id);
        save_slots::delete_slot(&self.store(), slot_id)
    }

    pub fn export(&self, slot_id: &str) -> Result<String, LoadRefusal> {
        save_slots::export_slot(&self.store(), slot_id)
    }

    /// Load through the unchanged full `Versions::check` gate. A native pane
    /// can use the returned run to boot the named scenario and call the existing
    /// snapshot restore path; no second compatibility answer lives here.
    pub fn load(
        &self,
        slot_id: &str,
        current: &vellum_save::Versions,
    ) -> Result<StoredRun, LoadRefusal> {
        save_slots::load_slot(&self.store(), slot_id, current)
    }

    pub fn pop_outcome(&mut self) -> Option<SaveWriteOutcome> {
        self.outcomes.pop_front()
    }

    pub fn outcomes(&self) -> impl Iterator<Item = &SaveWriteOutcome> {
        self.outcomes.iter()
    }

    pub fn pop_manual_refusal(&mut self) -> Option<RefusedManualSave> {
        self.manual_refusals.pop_front()
    }

    pub fn manual_refusals(&self) -> impl Iterator<Item = &RefusedManualSave> {
        self.manual_refusals.iter()
    }

    pub fn pop_restore_outcome(&mut self) -> Option<NativeRestoreOutcome> {
        self.restore_outcomes.pop_front()
    }

    fn push_restore_outcome(&mut self, outcome: NativeRestoreOutcome) {
        while self.restore_outcomes.len() >= MAX_SAVE_WRITE_OUTCOMES {
            self.restore_outcomes.pop_front();
        }
        self.restore_outcomes.push_back(outcome);
    }

    fn push_manual_refusal(&mut self, refusal: RefusedManualSave) {
        while self.manual_refusals.len() >= MAX_SAVE_WRITE_OUTCOMES {
            self.manual_refusals.pop_front();
        }
        self.manual_refusals.push_back(refusal);
    }

    /// Write one captured run into this peer's Store and record the outcome.
    ///
    /// Public because a HELD world is already a stable capture boundary and a
    /// caller that holds it must be able to take a save without waiting for a
    /// fixed tick that the hold itself has stopped: the live restore in
    /// `crate::gm_restore` pauses first and then captures its recovery
    /// checkpoint, which the `FixedLast` scheduler could never deliver. It is
    /// the SAME write the scheduler performs — same manual/autosave routing,
    /// same display-name consumption, same outcome FIFO — so nothing about the
    /// stored row differs from a bookmark's.
    pub fn persist(&mut self, pending: PendingStoredRun) {
        let decision = pending.decision;
        let result = match &decision.slot {
            CaptureSlot::RollingAutosave => save_slots::write_autosave(&self.store(), &pending.run),
            CaptureSlot::Manual(slot_id) => {
                let display_name = self
                    .manual_names
                    .remove(slot_id)
                    .unwrap_or_else(|| slot_id.clone());
                save_slots::write_manual_save(&self.store(), slot_id, &display_name, &pending.run)
            }
        };
        while self.outcomes.len() >= MAX_SAVE_WRITE_OUTCOMES {
            self.outcomes.pop_front();
        }
        self.outcomes
            .push_back(SaveWriteOutcome { decision, result });
    }
}

/// Install one peer-local backend and its non-fixed drain. Calling code owns
/// the policy decision to persist: headless and balance builders never call
/// this; the native binary does, and tests/embedders may inject a fake.
pub fn install_local_save_store(app: &mut App, store: impl LocalSaveStore) {
    app.insert_resource(SaveCaptureConsumer)
        .insert_resource(SaveSlotService::new(store))
        .init_resource::<NativeStartupManualSlots>()
        .add_systems(
            OnEnter(crate::core::messages::GamePhase::InProgress),
            admit_native_startup_manual_saves,
        )
        .add_systems(
            PostUpdate,
            (
                apply_native_startup_restore,
                drain_pending_stored_runs,
                drain_refused_manual_saves,
            )
                .chain(),
        );
}

/// Reserve a named manual slot now and request its capture only after this new
/// native session enters `InProgress`. Startup CLI handling can therefore run
/// while the App is still in Lobby without ever creating a Lobby snapshot.
pub fn queue_named_manual_save_for_new_session(
    world: &mut World,
    display_name: impl Into<String>,
) -> Result<String, &'static str> {
    let slot_id = {
        let Some(mut service) = world.get_resource_mut::<SaveSlotService>() else {
            return Err("no local save Store is installed");
        };
        service.reserve_manual(display_name)
    };
    world
        .get_resource_or_insert_with(NativeStartupManualSlots::default)
        .0
        .push_back(slot_id.clone());
    Ok(slot_id)
}

/// Fully gate and stage a local slot for this newly built native App.
///
/// The `SimTick == 0` guard is the native half of T3 M5: this startup route
/// cannot become a live-session restore by being called later.
pub fn stage_new_native_session_from_slot(
    world: &mut World,
    slot_id: &str,
    current: &vellum_save::Versions,
    loaded_scenario: &str,
) -> Result<u64, NativeResumeRefusal> {
    let tick = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    if tick != 0 {
        return Err(NativeResumeRefusal::LiveSession { tick });
    }
    if startup_restore::is_pending(world) {
        return Err(NativeResumeRefusal::AlreadyStaged);
    }
    let run = world
        .get_resource::<SaveSlotService>()
        .ok_or(NativeResumeRefusal::NoStore)?
        .load(slot_id, current)
        .map_err(NativeResumeRefusal::Load)?;
    if run.scenario != loaded_scenario {
        return Err(NativeResumeRefusal::WrongScenario {
            saved: run.scenario,
            loaded: loaded_scenario.to_string(),
        });
    }
    let boot = crate::snapshot::required_boot_identity(&run)
        .map_err(NativeResumeRefusal::Load)?
        .clone();
    if let Some(world_config) = world.get_resource::<crate::world::config::WorldConfig>() {
        crate::snapshot::validate_boot_identity_for_world(&boot, world_config)
            .map_err(NativeResumeRefusal::Load)?;
    }
    let loaded_ship = world
        .get_resource::<crate::lobby::SelectedShipResource>()
        .map(|selected| selected.0.clone())
        .unwrap_or_default();
    if loaded_ship != boot.selected_ship {
        return Err(NativeResumeRefusal::WrongSelectedShip {
            saved: boot.selected_ship,
            loaded: loaded_ship,
        });
    }
    if world.contains_resource::<crate::lockstep::FleetLockstep>()
        && world
            .get_resource::<crate::lockstep::FleetRoster>()
            .is_some_and(|loaded| loaded != &boot.fleet)
    {
        return Err(NativeResumeRefusal::WrongFleet);
    }
    crate::server_app::stage_resume_game_start_entity_uuids(world, &boot);
    crate::lockstep::start_saved_fleet_standalone(world, boot.fleet);
    let capture_tick = run
        .snapshot
        .as_ref()
        .map_or(run.ledger.final_tick, |snapshot| snapshot.tick);
    startup_restore::stage(world, run);
    Ok(capture_tick)
}

/// Queue a named manual save for the next fixed tick. The display name never
/// becomes a Store key; the returned UUID-shaped id is the stable identity.
pub fn request_named_manual_save(
    world: &mut World,
    display_name: impl Into<String>,
) -> Result<String, &'static str> {
    let slot_id = {
        let Some(mut service) = world.get_resource_mut::<SaveSlotService>() else {
            return Err("no local save Store is installed");
        };
        service.reserve_manual(display_name)
    };
    request_manual_save(world, slot_id.clone());
    Ok(slot_id)
}

fn defer_other_scenario_content(entries: &mut [SaveSlotEntry], loaded_scenario: &str) {
    use crate::save_slots::StartState;

    for entry in entries {
        if entry
            .record
            .as_ref()
            .is_some_and(|record| record.scenario == loaded_scenario)
        {
            continue;
        }
        if matches!(
            &entry.start,
            StartState::Ready
                | StartState::Refused(LoadRefusal::Moved(vellum_save::Moved::Content { .. }))
        ) {
            entry.start = StartState::ContentDeferred;
        }
    }
}

fn admit_native_startup_manual_saves(world: &mut World) {
    let slots = world
        .get_resource_mut::<NativeStartupManualSlots>()
        .map(|mut slots| std::mem::take(&mut slots.0))
        .unwrap_or_default();
    for slot_id in slots {
        request_manual_save(world, slot_id);
    }
}

/// Report the shared startup driver's terminal result through the native Store.
fn apply_native_startup_restore(world: &mut World) {
    let Some(outcome) = startup_restore::advance(world) else {
        return;
    };
    let outcome = match outcome {
        RestoreOutcome::Applied { tick } => NativeRestoreOutcome::Applied { tick },
        RestoreOutcome::Failed(failure) => NativeRestoreOutcome::Failed {
            detail: match failure {
                RestoreFailure::NoSnapshot => "the save carries no captured state".to_string(),
                RestoreFailure::LayerFailed { path } => {
                    format!("required world layer {path:?} could not be reconstructed")
                }
                RestoreFailure::NotReady { tick, .. } => {
                    format!("the new session never built the saved roster for tick {tick}")
                }
                RestoreFailure::DigestMismatch { expected, actual } => format!(
                    "restored digest {actual:016x} did not match saved digest {expected:016x}"
                ),
                RestoreFailure::Incomplete { tick, gaps } => {
                    format!("restore at tick {tick} retained {gaps} unresolved gap(s)")
                }
            },
        },
    };
    if let Some(mut service) = world.get_resource_mut::<SaveSlotService>() {
        service.push_restore_outcome(outcome);
    }
}

fn drain_pending_stored_runs(world: &mut World) {
    loop {
        let pending = world
            .get_resource_mut::<PendingStoredRuns>()
            .and_then(|mut runs| runs.pop_front());
        let Some(pending) = pending else {
            return;
        };
        let Some(mut service) = world.get_resource_mut::<SaveSlotService>() else {
            // This system is only registered by `install_local_save_store`, but
            // retain the queue if a caller removes the resource at runtime.
            world
                .get_resource_or_insert_with(PendingStoredRuns::default)
                .push_back(pending);
            return;
        };
        service.persist(pending);
    }
}

fn drain_refused_manual_saves(world: &mut World) {
    if !world.contains_resource::<SaveSlotService>() {
        return;
    }
    loop {
        let refusal = world
            .get_resource_mut::<RefusedManualSaves>()
            .and_then(|mut refusals| refusals.pop_front());
        let Some(refusal) = refusal else {
            return;
        };
        let mut service = world.resource_mut::<SaveSlotService>();
        service.manual_names.remove(&refusal.slot_id);
        service.push_manual_refusal(refusal);
    }
}

#[cfg(test)]
mod tests {
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
        crate::snapshot::save_to(&live_store, save_slots::AUTOSAVE_SLOT, &run(27, versions()))
            .unwrap();
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
}
