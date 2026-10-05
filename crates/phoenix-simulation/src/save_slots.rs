//! Deterministic save-capture scheduling (issue #865).
//!
//! This module decides *which completed logical tick* should be captured. It
//! deliberately knows nothing about Bevy schedules, snapshot construction, or
//! storage: those adapters consume [`CaptureDecision`]s later. Keeping the
//! state machine pure makes the peer-equality rule testable without a running
//! simulation.

use std::collections::VecDeque;

/// The phase information the scheduler needs, stripped of engine state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePhase {
    /// Lobby/loading or any other state outside a resumable run.
    BeforeRun,
    InProgress,
    GameOver,
}

/// Why a capture was scheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureReason {
    /// The first committed tick of an in-progress run.
    RunStarted,
    /// An exact interval boundary relative to that first tick.
    Periodic,
    /// The first committed tick observed in `GameOver` after a run.
    GameOver,
    /// A peer-local manual request.
    Manual,
}

/// Which local slot a capture is destined for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureSlot {
    /// The one rolling automatic slot.
    RollingAutosave,
    /// A peer-local manual slot id. Display names belong to catalogue metadata,
    /// not to this stable storage key.
    Manual(String),
}

/// One ordered request for the capture adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureDecision {
    pub tick: u64,
    pub slot: CaptureSlot,
    pub reason: CaptureReason,
}

/// Why a manual request was consumed without producing a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualSaveRefusalReason {
    /// The request reached its fixed boundary after the run changed phase.
    PhaseChanged { phase: SavePhase },
    /// A fresh App is still bootstrapping the record it will restore.
    StartupRestorePending,
}

/// One peer-local manual request adapters must clear from their pending UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefusedManualSave {
    pub tick: u64,
    pub slot_id: String,
    pub reason: ManualSaveRefusalReason,
}

/// Complete result of advancing the pure scheduler once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveScheduleOutput {
    pub decisions: Vec<CaptureDecision>,
    pub refused_manual: Vec<RefusedManualSave>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingManual {
    target_tick: u64,
    slot_id: String,
}

/// Per-peer scheduler state.
///
/// Call [`Self::step`] once for every logical tick, after that tick's
/// simulation work has committed. `manual_slot_ids` are requests observed on
/// the current tick; each targets the following tick and remains a distinct
/// queue entry even when several requests name the same slot. A due request is
/// emitted only while the phase is [`SavePhase::InProgress`]; crossing a
/// terminal or before-run boundary consumes it without a capture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveSchedule {
    run_start_tick: Option<u64>,
    last_periodic_tick: Option<u64>,
    game_over_captured: bool,
    pending_manual: VecDeque<PendingManual>,
}

impl SaveSchedule {
    /// Queue peer-local manual requests for an already chosen logical tick.
    ///
    /// The Bevy adapter uses this when requests arrive between fixed steps: the
    /// current [`SimTick`](crate::sim_tick::SimTick) is the next step that will
    /// run, so its first `FixedLast` is the requested boundary. [`Self::step`]
    /// remains the convenient pure API for requests observed *during* a tick,
    /// which target `current_tick + 1`.
    pub fn queue_manual_for_tick<I, S>(&mut self, target_tick: u64, slot_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.pending_manual
            .extend(slot_ids.into_iter().map(|slot_id| PendingManual {
                target_tick,
                slot_id: slot_id.into(),
            }));
    }

    /// Consume every queued manual request with one local refusal apiece.
    ///
    /// Startup restore uses this before replacing the live bootstrap world.
    /// Requests are local presentation intent and must neither cross the
    /// restore boundary nor silently remain pending afterward.
    pub fn refuse_pending_manual(
        &mut self,
        current_tick: u64,
        reason: ManualSaveRefusalReason,
    ) -> Vec<RefusedManualSave> {
        self.pending_manual
            .drain(..)
            .map(|request| RefusedManualSave {
                tick: current_tick,
                slot_id: request.slot_id,
                reason,
            })
            .collect()
    }

    /// Rebase automatic cadence at one restored continuation boundary.
    ///
    /// The restored phase is already observed: an in-progress record therefore
    /// does not emit a duplicate `RunStarted`, and its next periodic decision is
    /// exactly `continuation_tick + interval_ticks`. A restored terminal record
    /// likewise does not repeat its automatic final capture.
    pub fn rebase_after_restore(&mut self, continuation_tick: u64, phase: SavePhase) {
        self.pending_manual.clear();
        self.last_periodic_tick = None;
        match phase {
            SavePhase::BeforeRun => {
                self.run_start_tick = None;
                self.game_over_captured = false;
            }
            SavePhase::InProgress => {
                self.run_start_tick = Some(continuation_tick);
                self.game_over_captured = false;
            }
            SavePhase::GameOver => {
                self.run_start_tick = Some(continuation_tick);
                self.game_over_captured = true;
            }
        }
    }

    /// Advance the scheduler at one committed logical tick.
    ///
    /// Automatic decisions are emitted first, followed by due, capturable
    /// manual requests in FIFO order. Due manual requests outside
    /// [`SavePhase::InProgress`] are consumed without being emitted.
    /// `interval_ticks == 0` disables periodic decisions; world configuration
    /// rejects that value before the simulation adapter runs.
    pub fn step<I, S>(
        &mut self,
        current_tick: u64,
        phase: SavePhase,
        interval_ticks: u64,
        manual_slot_ids: I,
    ) -> Vec<CaptureDecision>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.step_with_outcomes(current_tick, phase, interval_ticks, manual_slot_ids)
            .decisions
    }

    /// Advance once and retain local refusals as well as capture decisions.
    pub fn step_with_outcomes<I, S>(
        &mut self,
        current_tick: u64,
        phase: SavePhase,
        interval_ticks: u64,
        manual_slot_ids: I,
    ) -> SaveScheduleOutput
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let target_tick = current_tick.saturating_add(1);
        self.queue_manual_for_tick(target_tick, manual_slot_ids);

        let mut decisions = Vec::new();
        let mut refused_manual = Vec::new();
        match phase {
            SavePhase::BeforeRun => {
                self.run_start_tick = None;
                self.last_periodic_tick = None;
                self.game_over_captured = false;
            }
            SavePhase::InProgress => match self.run_start_tick {
                None => {
                    self.run_start_tick = Some(current_tick);
                    self.last_periodic_tick = None;
                    self.game_over_captured = false;
                    decisions.push(CaptureDecision {
                        tick: current_tick,
                        slot: CaptureSlot::RollingAutosave,
                        reason: CaptureReason::RunStarted,
                    });
                }
                Some(start_tick) => {
                    let elapsed = current_tick.saturating_sub(start_tick);
                    if interval_ticks > 0
                        && elapsed > 0
                        && elapsed % interval_ticks == 0
                        && self.last_periodic_tick != Some(current_tick)
                    {
                        self.last_periodic_tick = Some(current_tick);
                        decisions.push(CaptureDecision {
                            tick: current_tick,
                            slot: CaptureSlot::RollingAutosave,
                            reason: CaptureReason::Periodic,
                        });
                    }
                }
            },
            SavePhase::GameOver => {
                if self.run_start_tick.is_some() && !self.game_over_captured {
                    self.game_over_captured = true;
                    decisions.push(CaptureDecision {
                        tick: current_tick,
                        slot: CaptureSlot::RollingAutosave,
                        reason: CaptureReason::GameOver,
                    });
                }
            }
        }

        while self
            .pending_manual
            .front()
            .is_some_and(|request| request.target_tick <= current_tick)
        {
            let request = self
                .pending_manual
                .pop_front()
                .expect("front was present immediately above");
            // Manual saves are resumable-session actions, unlike the one
            // automatic final GameOver capture above. A request admitted while
            // the run was live may reach its next boundary after a same-frame
            // GameOver/ReturnToLobby transition. Consume it there, but never
            // emit a decision that could serialize Lobby or a terminal screen.
            if phase == SavePhase::InProgress {
                decisions.push(CaptureDecision {
                    tick: current_tick,
                    slot: CaptureSlot::Manual(request.slot_id),
                    reason: CaptureReason::Manual,
                });
            } else {
                refused_manual.push(RefusedManualSave {
                    tick: current_tick,
                    slot_id: request.slot_id,
                    reason: ManualSaveRefusalReason::PhaseChanged { phase },
                });
            }
        }

        SaveScheduleOutput {
            decisions,
            refused_manual,
        }
    }
}

// ── Peer-local Store catalogue ────────────────────────────────────────────

/// The reserved rolling automatic slot. Manual saves always use UUID-shaped
/// ids, so no display name can collide with it.
pub const AUTOSAVE_SLOT: &str = crate::snapshot::DEFAULT_SLOT;

const METADATA_SLOT_PREFIX: &str = "metadata-";
const METADATA_FORMAT_PREFIX: &str = "display-name-v1:";

/// Whether a catalogue row is the reserved rolling slot or a manual save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveSlotKind {
    Autosave,
    Manual,
}

/// What happened while resolving a manual slot's display-name sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataStatus {
    NotApplicable,
    Present,
    Missing,
    Corrupt,
    Unreadable(String),
}

/// Canonical fields projected from the stored [`crate::snapshot::StoredRun`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRecordSummary {
    pub scenario: String,
    pub seed: u64,
    pub capture_tick: u64,
    /// The hull and frozen roster this row must stage before it can restore.
    /// `None` keeps a damaged/current-format row visible and deletable; its
    /// [`StartState`] is refused.
    pub boot_identity: Option<crate::snapshot::BootIdentity>,
    pub versions: vellum_save::Versions,
}

/// Whether a row may seed a new session under the current build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartState {
    Ready,
    /// Format and rules match, but this peer has not loaded/frozen the row's
    /// scenario content yet. The row may begin a fresh-app boot; the unchanged
    /// full version gate decides after that content is loaded.
    ContentDeferred,
    Refused(crate::snapshot::LoadRefusal),
}

/// Whether catalogue construction can make the content-digest comparison now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentCheck {
    Full,
    Deferred,
}

/// One deterministic catalogue row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveSlotEntry {
    pub slot_id: String,
    pub kind: SaveSlotKind,
    /// Manual names come from sidecar metadata. A missing/corrupt sidecar falls
    /// back to `slot_id`; the run key is never derived from this value.
    pub display_name: String,
    pub metadata: MetadataStatus,
    pub record: Option<SaveRecordSummary>,
    pub start: StartState,
}

impl SaveSlotEntry {
    pub fn can_start(&self) -> bool {
        matches!(self.start, StartState::Ready | StartState::ContentDeferred)
    }
}

/// The Store operation that failed. Kept structural so target adapters can
/// localise/present it without parsing an English error sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogueOperation {
    List,
    Read,
    WriteRun,
    WriteMetadata,
    RemoveRun,
    RemoveMetadata,
}

/// A peer-local catalogue/storage failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogueError {
    InvalidSlot(String),
    ReservedAutosave,
    MissingSlot(String),
    IdCollision(String),
    Store {
        operation: CatalogueOperation,
        slot: String,
        detail: String,
    },
    /// The first write/remove succeeded and its compensating or following
    /// operation failed. The remaining slot is still discoverable/deletable.
    Partial {
        operation: CatalogueOperation,
        slot: String,
        detail: String,
        rollback_detail: Option<String>,
    },
}

/// Replace the one rolling autosave. It has no display-name sidecar.
pub fn write_autosave<S: vellum_save::Store>(
    store: &S,
    run: &crate::snapshot::StoredRun,
) -> Result<(), CatalogueError> {
    crate::snapshot::save_to(store, AUTOSAVE_SLOT, run).map_err(|detail| CatalogueError::Store {
        operation: CatalogueOperation::WriteRun,
        slot: AUTOSAVE_SLOT.to_string(),
        detail,
    })
}

/// Create a manual save with an opaque, Store-safe v4 UUID key.
///
/// OS entropy is correct here: this is peer-private catalogue identity, never
/// simulation identity and never part of the digest or mesh.
#[allow(clippy::disallowed_methods)]
pub fn new_manual_slot_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Create a manual save and return its newly allocated internal slot id.
pub fn create_manual_save<S: vellum_save::Store>(
    store: &S,
    display_name: impl Into<String>,
    run: &crate::snapshot::StoredRun,
) -> Result<String, CatalogueError> {
    let slot_id = new_manual_slot_id();
    write_manual_save(store, &slot_id, display_name, run)?;
    Ok(slot_id)
}

/// Write a captured run into an already allocated manual slot id.
///
/// Browser ingress allocates the id synchronously so it can return stable
/// identity to JavaScript, then calls this only after the requested fixed-tick
/// capture reaches its local outbox.
pub fn write_manual_save<S: vellum_save::Store>(
    store: &S,
    slot_id: &str,
    display_name: impl Into<String>,
    run: &crate::snapshot::StoredRun,
) -> Result<(), CatalogueError> {
    require_manual_slot(slot_id)?;
    let display_name = display_name.into();
    let metadata_slot = metadata_slot(slot_id);
    for candidate in [slot_id, metadata_slot.as_str()] {
        match store.read(candidate) {
            Ok(Some(_)) => return Err(CatalogueError::IdCollision(slot_id.to_string())),
            Ok(None) => {}
            Err(error) => {
                return Err(store_error(CatalogueOperation::Read, candidate, error));
            }
        }
    }

    crate::snapshot::save_to(store, slot_id, run).map_err(|detail| CatalogueError::Store {
        operation: CatalogueOperation::WriteRun,
        slot: slot_id.to_string(),
        detail,
    })?;
    if let Err(error) = store.write(&metadata_slot, &encode_display_name(&display_name)) {
        let detail = error.to_string();
        return match store.remove(slot_id) {
            Ok(()) => Err(CatalogueError::Store {
                operation: CatalogueOperation::WriteMetadata,
                slot: metadata_slot,
                detail,
            }),
            Err(rollback) => Err(CatalogueError::Partial {
                operation: CatalogueOperation::WriteMetadata,
                slot: slot_id.to_string(),
                detail,
                rollback_detail: Some(rollback.to_string()),
            }),
        };
    }
    Ok(())
}

/// List only canonical save runs, filtering metadata sidecars and unrelated
/// Store users. Autosave is first; manual saves follow newest capture first,
/// then UUID, so backend enumeration order cannot move the UI.
pub fn list_slots<S: vellum_save::Store>(
    store: &S,
    current: &vellum_save::Versions,
) -> Result<Vec<SaveSlotEntry>, CatalogueError> {
    let mut ids = store
        .slots()
        .map_err(|error| store_error(CatalogueOperation::List, "", error))?;
    ids.retain(|slot| is_run_slot(slot));
    ids.sort();
    ids.dedup();

    let mut entries: Vec<_> = ids
        .into_iter()
        .map(|slot_id| catalogue_entry(store, slot_id, current))
        .collect();
    entries.sort_by(|left, right| {
        let kind_rank = |kind| match kind {
            SaveSlotKind::Autosave => 0,
            SaveSlotKind::Manual => 1,
        };
        kind_rank(left.kind)
            .cmp(&kind_rank(right.kind))
            .then_with(|| {
                right
                    .record
                    .as_ref()
                    .map(|record| record.capture_tick)
                    .cmp(&left.record.as_ref().map(|record| record.capture_tick))
            })
            .then_with(|| left.slot_id.cmp(&right.slot_id))
    });
    Ok(entries)
}

/// Build the peer-local catalogue at the browser's current content-readiness.
///
/// Before a scenario has loaded there is no frozen content digest to compare
/// with a stored run. A content-only difference against that provisional
/// digest is therefore deferred, while format/rules and damaged Store rows
/// remain hard refusals. [`load_slot`] still performs the unchanged full
/// [`vellum_save::Versions::check`] after the selected scenario is loaded.
pub fn list_slots_with_content_check<S: vellum_save::Store>(
    store: &S,
    current: &vellum_save::Versions,
    content_check: ContentCheck,
) -> Result<Vec<SaveSlotEntry>, CatalogueError> {
    let mut entries = list_slots(store, current)?;
    if content_check == ContentCheck::Deferred {
        for entry in &mut entries {
            if matches!(
                &entry.start,
                StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                    vellum_save::Moved::Content { .. }
                ))
            ) {
                entry.start = StartState::ContentDeferred;
            }
        }
    }
    Ok(entries)
}

/// Replace a manual slot's display-name sidecar. The run id never changes.
pub fn rename_slot<S: vellum_save::Store>(
    store: &S,
    slot_id: &str,
    display_name: impl Into<String>,
) -> Result<(), CatalogueError> {
    require_manual_slot(slot_id)?;
    match store.read(slot_id) {
        Ok(Some(_)) => {}
        Ok(None) => return Err(CatalogueError::MissingSlot(slot_id.to_string())),
        Err(error) => return Err(store_error(CatalogueOperation::Read, slot_id, error)),
    }
    let sidecar = metadata_slot(slot_id);
    store
        .write(&sidecar, &encode_display_name(&display_name.into()))
        .map_err(|error| store_error(CatalogueOperation::WriteMetadata, &sidecar, error))
}

/// Remove a run and its sidecar. For a manual save the sidecar is removed
/// first: if removing the run then fails, the run remains visible with its
/// documented metadata fallback and can be retried.
pub fn delete_slot<S: vellum_save::Store>(store: &S, slot_id: &str) -> Result<(), CatalogueError> {
    let kind = slot_kind(slot_id).ok_or_else(|| CatalogueError::InvalidSlot(slot_id.into()))?;
    if kind == SaveSlotKind::Manual {
        let sidecar = metadata_slot(slot_id);
        store
            .remove(&sidecar)
            .map_err(|error| store_error(CatalogueOperation::RemoveMetadata, &sidecar, error))?;
        if let Err(error) = store.remove(slot_id) {
            return Err(CatalogueError::Partial {
                operation: CatalogueOperation::RemoveRun,
                slot: slot_id.to_string(),
                detail: error.to_string(),
                rollback_detail: None,
            });
        }
        return Ok(());
    }
    store
        .remove(slot_id)
        .map_err(|error| store_error(CatalogueOperation::RemoveRun, slot_id, error))
}

/// Export the selected canonical run without applying this build's version
/// gate. An older save may still be exported to a build that can honour it;
/// incompatibility prevents starting here, not copying the artifact.
pub fn export_slot<S: vellum_save::Store>(
    store: &S,
    slot_id: &str,
) -> Result<String, crate::snapshot::LoadRefusal> {
    let text = store
        .read(slot_id)
        .map_err(|error| crate::snapshot::LoadRefusal::Unreadable(error.to_string()))?
        .ok_or(crate::snapshot::LoadRefusal::Empty)?;
    let run = crate::snapshot::StoredRun::from_ron(&text)
        .map_err(|error| crate::snapshot::LoadRefusal::Unparsable(error.to_string()))?;
    crate::snapshot::export_artifact(&run).map_err(crate::snapshot::LoadRefusal::Unparsable)
}

/// Load a selected slot for a new session through the existing, single
/// read→parse→[`vellum_save::Versions::check`] gate.
pub fn load_slot<S: vellum_save::Store>(
    store: &S,
    slot_id: &str,
    current: &vellum_save::Versions,
) -> Result<crate::snapshot::StoredRun, crate::snapshot::LoadRefusal> {
    crate::snapshot::load_from(store, slot_id, current)
}

/// Read only the scenario and pre-world identity needed to assemble a native
/// App before that App can compute its content version.
///
/// This is deliberately not a compatibility decision. The ordinary
/// [`load_slot`] gate still runs after content is loaded and remains the one
/// authority for format/rules/content. An older artifact can therefore return
/// `None` here and receive its precise format refusal later rather than being
/// mislabeled as corrupt.
pub fn peek_slot_boot_identity<S: vellum_save::Store>(
    store: &S,
    slot_id: &str,
) -> Result<(String, Option<crate::snapshot::BootIdentity>), crate::snapshot::LoadRefusal> {
    let text = store
        .read(slot_id)
        .map_err(|error| crate::snapshot::LoadRefusal::Unreadable(error.to_string()))?
        .ok_or(crate::snapshot::LoadRefusal::Empty)?;
    let run = crate::snapshot::StoredRun::from_ron(&text)
        .map_err(|error| crate::snapshot::LoadRefusal::Unparsable(error.to_string()))?;
    let boot_identity = run
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.state.boot_identity.clone());
    Ok((run.scenario, boot_identity))
}

fn catalogue_entry<S: vellum_save::Store>(
    store: &S,
    slot_id: String,
    current: &vellum_save::Versions,
) -> SaveSlotEntry {
    let kind = slot_kind(&slot_id).expect("list filtered to canonical run slots");
    let (display_name, metadata) = match kind {
        SaveSlotKind::Autosave => (slot_id.clone(), MetadataStatus::NotApplicable),
        SaveSlotKind::Manual => read_display_name(store, &slot_id),
    };
    let (record, start) = match store.read(&slot_id) {
        Err(error) => (
            None,
            StartState::Refused(crate::snapshot::LoadRefusal::Unreadable(error.to_string())),
        ),
        Ok(None) => (
            None,
            StartState::Refused(crate::snapshot::LoadRefusal::Empty),
        ),
        Ok(Some(text)) => match crate::snapshot::StoredRun::from_ron(&text) {
            Err(error) => (
                None,
                StartState::Refused(crate::snapshot::LoadRefusal::Unparsable(error.to_string())),
            ),
            Ok(run) => {
                let summary = SaveRecordSummary {
                    scenario: run.scenario.clone(),
                    seed: run.seed,
                    capture_tick: run
                        .snapshot
                        .as_ref()
                        .map_or(run.ledger.final_tick, |snapshot| snapshot.tick),
                    boot_identity: run
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.state.boot_identity.clone()),
                    versions: run.versions.clone(),
                };
                let start = match run.versions.check(current) {
                    Ok(()) => match crate::snapshot::required_boot_identity(&run) {
                        Ok(_) => StartState::Ready,
                        Err(refusal) => StartState::Refused(refusal),
                    },
                    Err(moved) => StartState::Refused(crate::snapshot::LoadRefusal::Moved(moved)),
                };
                (Some(summary), start)
            }
        },
    };
    SaveSlotEntry {
        slot_id,
        kind,
        display_name,
        metadata,
        record,
        start,
    }
}

fn read_display_name<S: vellum_save::Store>(store: &S, slot_id: &str) -> (String, MetadataStatus) {
    let sidecar = metadata_slot(slot_id);
    match store.read(&sidecar) {
        Ok(Some(text)) => match decode_display_name(&text) {
            Some(name) => (name, MetadataStatus::Present),
            None => (slot_id.to_string(), MetadataStatus::Corrupt),
        },
        Ok(None) => (slot_id.to_string(), MetadataStatus::Missing),
        Err(error) => (
            slot_id.to_string(),
            MetadataStatus::Unreadable(error.to_string()),
        ),
    }
}

fn slot_kind(slot: &str) -> Option<SaveSlotKind> {
    if slot == AUTOSAVE_SLOT {
        Some(SaveSlotKind::Autosave)
    } else if uuid::Uuid::parse_str(slot).is_ok_and(|uuid| uuid.to_string() == slot) {
        Some(SaveSlotKind::Manual)
    } else {
        None
    }
}

fn is_run_slot(slot: &str) -> bool {
    slot_kind(slot).is_some()
}

fn require_manual_slot(slot_id: &str) -> Result<(), CatalogueError> {
    match slot_kind(slot_id) {
        Some(SaveSlotKind::Manual) => Ok(()),
        Some(SaveSlotKind::Autosave) => Err(CatalogueError::ReservedAutosave),
        None => Err(CatalogueError::InvalidSlot(slot_id.to_string())),
    }
}

fn metadata_slot(slot_id: &str) -> String {
    let slot = format!("{METADATA_SLOT_PREFIX}{slot_id}");
    debug_assert!(vellum_save::is_slot(&slot));
    slot
}

fn encode_display_name(display_name: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(METADATA_FORMAT_PREFIX.len() + display_name.len() * 2);
    encoded.push_str(METADATA_FORMAT_PREFIX);
    for byte in display_name.as_bytes() {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_display_name(encoded: &str) -> Option<String> {
    let hex = encoded.strip_prefix(METADATA_FORMAT_PREFIX)?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks_exact(2) {
        let high = decode_hex(pair[0])?;
        let low = decode_hex(pair[1])?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).ok()
}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn store_error<E: std::fmt::Display>(
    operation: CatalogueOperation,
    slot: &str,
    error: E,
) -> CatalogueError {
    CatalogueError::Store {
        operation,
        slot: slot.to_string(),
        detail: error.to_string(),
    }
}

#[cfg(test)]
#[path = "save_slots_tests.rs"]
mod tests;
