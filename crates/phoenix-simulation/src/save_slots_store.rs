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
#[path = "save_slots_store_tests.rs"]
mod tests;
