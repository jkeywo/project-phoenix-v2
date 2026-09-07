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
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fmt;
    use vellum_save::Store;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FakeOperation {
        Read,
        Write,
        Remove,
        Slots,
    }

    #[derive(Clone, Debug)]
    struct FakeFailure {
        operation: FakeOperation,
        slot: String,
        detail: String,
    }

    #[derive(Clone, Debug)]
    struct FakeStoreError(String);

    impl fmt::Display for FakeStoreError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(&self.0)
        }
    }

    #[derive(Default)]
    struct FakeStore {
        values: RefCell<BTreeMap<String, String>>,
        advertised: RefCell<Vec<String>>,
        failures: RefCell<Vec<FakeFailure>>,
    }

    impl FakeStore {
        fn put(&self, slot: &str, value: impl Into<String>) {
            self.values
                .borrow_mut()
                .insert(slot.to_string(), value.into());
        }

        fn get(&self, slot: &str) -> Option<String> {
            self.values.borrow().get(slot).cloned()
        }

        fn contains(&self, slot: &str) -> bool {
            self.values.borrow().contains_key(slot)
        }

        fn advertise(&self, slot: &str) {
            self.advertised.borrow_mut().push(slot.to_string());
        }

        fn fail_next(&self, operation: FakeOperation, slot: &str, detail: &str) {
            self.failures.borrow_mut().push(FakeFailure {
                operation,
                slot: slot.to_string(),
                detail: detail.to_string(),
            });
        }

        fn maybe_fail(&self, operation: FakeOperation, slot: &str) -> Result<(), FakeStoreError> {
            let mut failures = self.failures.borrow_mut();
            let Some(index) = failures
                .iter()
                .position(|failure| failure.operation == operation && failure.slot == slot)
            else {
                return Ok(());
            };
            Err(FakeStoreError(failures.remove(index).detail))
        }
    }

    impl Store for FakeStore {
        type Error = FakeStoreError;

        fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
            self.maybe_fail(FakeOperation::Read, slot)?;
            Ok(self.get(slot))
        }

        fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
            self.maybe_fail(FakeOperation::Write, slot)?;
            self.put(slot, contents);
            Ok(())
        }

        fn remove(&self, slot: &str) -> Result<(), Self::Error> {
            self.maybe_fail(FakeOperation::Remove, slot)?;
            self.values.borrow_mut().remove(slot);
            Ok(())
        }

        fn slots(&self) -> Result<Vec<String>, Self::Error> {
            self.maybe_fail(FakeOperation::Slots, "")?;
            let mut slots: Vec<_> = self.values.borrow().keys().cloned().collect();
            slots.extend(self.advertised.borrow().iter().cloned());
            // Deliberately non-canonical: ordering belongs to the wrapper.
            slots.reverse();
            Ok(slots)
        }
    }

    const SLOT_A: &str = "00000000-0000-4000-8000-000000000001";
    const SLOT_B: &str = "00000000-0000-4000-8000-000000000002";
    const SLOT_C: &str = "00000000-0000-4000-8000-000000000003";
    const SLOT_D: &str = "00000000-0000-4000-8000-000000000004";
    const SLOT_E: &str = "00000000-0000-4000-8000-000000000005";
    const SLOT_F: &str = "00000000-0000-4000-8000-000000000006";

    fn current_versions() -> vellum_save::Versions {
        vellum_save::Versions::new(7, "rules-a", 0x1234)
    }

    fn stored_run(
        tick: u64,
        scenario: &str,
        seed: u64,
        versions: vellum_save::Versions,
    ) -> crate::snapshot::StoredRun {
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
            tick.wrapping_mul(17),
            seed,
            scenario,
            versions,
        )
    }

    fn put_run(store: &FakeStore, slot: &str, run: &crate::snapshot::StoredRun) {
        crate::snapshot::save_to(store, slot, run).expect("fake Store write succeeds");
    }

    fn boot_identity_mut(
        run: &mut crate::snapshot::StoredRun,
    ) -> &mut crate::snapshot::BootIdentity {
        run.snapshot
            .as_mut()
            .and_then(|snapshot| snapshot.state.boot_identity.as_mut())
            .expect("stored-run fixture carries boot identity")
    }

    fn manual_entry<'a>(entries: &'a [SaveSlotEntry], slot: &str) -> &'a SaveSlotEntry {
        entries
            .iter()
            .find(|entry| entry.slot_id == slot)
            .expect("manual row is listed")
    }

    fn step(
        schedule: &mut SaveSchedule,
        tick: u64,
        phase: SavePhase,
        interval: u64,
    ) -> Vec<CaptureDecision> {
        schedule.step(tick, phase, interval, std::iter::empty::<String>())
    }

    #[test]
    fn current_format_refuses_malformed_game_start_identity_maps() {
        let versions = current_versions();
        let base = stored_run(9, "scenario", 17, versions.clone());
        let mut malformed = Vec::new();

        let mut missing = base.clone();
        boot_identity_mut(&mut missing)
            .game_start_entity_uuids
            .clear();
        malformed.push(missing);

        let mut out_of_order = base.clone();
        boot_identity_mut(&mut out_of_order).game_start_entity_uuids = vec![
            crate::snapshot::GameStartEntityUuid {
                authored_index: 2,
                entity_uuid: crate::world_id::WorldId::new(
                    crate::world_id::IdNamespace::Entity,
                    0,
                    2,
                )
                .render(),
            },
            crate::snapshot::GameStartEntityUuid {
                authored_index: 1,
                entity_uuid: crate::world_id::WorldId::new(
                    crate::world_id::IdNamespace::Entity,
                    0,
                    1,
                )
                .render(),
            },
        ];
        malformed.push(out_of_order);

        let mut repeated = base.clone();
        let repeated_uuid = boot_identity_mut(&mut repeated).game_start_entity_uuids[0]
            .entity_uuid
            .clone();
        boot_identity_mut(&mut repeated)
            .game_start_entity_uuids
            .push(crate::snapshot::GameStartEntityUuid {
                authored_index: 1,
                entity_uuid: repeated_uuid,
            });
        malformed.push(repeated);

        let mut wrong_namespace = base.clone();
        boot_identity_mut(&mut wrong_namespace).game_start_entity_uuids[0].entity_uuid =
            crate::world_id::WorldId::new(crate::world_id::IdNamespace::Message, 0, 0).render();
        malformed.push(wrong_namespace);

        let mut repeated_snapshot_row = base.clone();
        let repeated_snapshot_uuid = boot_identity_mut(&mut repeated_snapshot_row)
            .game_start_entity_uuids[0]
            .entity_uuid
            .clone();
        repeated_snapshot_row
            .snapshot
            .as_mut()
            .unwrap()
            .state
            .entities = vec![
            crate::snapshot::EntityState {
                uuid: repeated_snapshot_uuid.clone(),
                ..Default::default()
            },
            crate::snapshot::EntityState {
                uuid: repeated_snapshot_uuid,
                ..Default::default()
            },
        ];
        malformed.push(repeated_snapshot_row);

        let mut noncanonical = base;
        boot_identity_mut(&mut noncanonical).game_start_entity_uuids[0].entity_uuid =
            "00000000-0000-8000-8000-00000000000A".into();
        malformed.push(noncanonical);

        for (index, run) in malformed.iter().enumerate() {
            let store = FakeStore::default();
            put_run(&store, SLOT_A, run);
            assert!(
                matches!(
                    load_slot(&store, SLOT_A, &versions),
                    Err(crate::snapshot::LoadRefusal::Unparsable(_))
                ),
                "malformed GameStart identity case {index} must be refused"
            );
        }
    }

    #[test]
    fn automatic_captures_cover_start_periodic_and_final_boundaries_once() {
        let mut schedule = SaveSchedule::default();

        assert!(step(&mut schedule, 8, SavePhase::BeforeRun, 10).is_empty());
        assert_eq!(
            step(&mut schedule, 10, SavePhase::InProgress, 10),
            vec![CaptureDecision {
                tick: 10,
                slot: CaptureSlot::RollingAutosave,
                reason: CaptureReason::RunStarted,
            }]
        );
        assert!(step(&mut schedule, 19, SavePhase::InProgress, 10).is_empty());
        assert_eq!(
            step(&mut schedule, 20, SavePhase::InProgress, 10),
            vec![CaptureDecision {
                tick: 20,
                slot: CaptureSlot::RollingAutosave,
                reason: CaptureReason::Periodic,
            }]
        );
        assert!(step(&mut schedule, 20, SavePhase::InProgress, 10).is_empty());
        assert_eq!(
            step(&mut schedule, 21, SavePhase::GameOver, 10),
            vec![CaptureDecision {
                tick: 21,
                slot: CaptureSlot::RollingAutosave,
                reason: CaptureReason::GameOver,
            }]
        );
        assert!(step(&mut schedule, 22, SavePhase::GameOver, 10).is_empty());
    }

    #[test]
    fn periodic_ticks_are_relative_to_the_run_start_not_absolute_tick_zero() {
        let mut schedule = SaveSchedule::default();
        step(&mut schedule, 7, SavePhase::InProgress, 5);

        assert!(step(&mut schedule, 10, SavePhase::InProgress, 5).is_empty());
        assert_eq!(
            step(&mut schedule, 12, SavePhase::InProgress, 5)[0].reason,
            CaptureReason::Periodic
        );
        assert_eq!(
            step(&mut schedule, 17, SavePhase::InProgress, 5)[0].reason,
            CaptureReason::Periodic
        );
    }

    #[test]
    fn every_manual_request_survives_and_fires_on_the_next_live_tick_in_fifo_order() {
        let mut schedule = SaveSchedule::default();
        assert!(schedule
            .step(40, SavePhase::InProgress, 10, ["alpha", "alpha", "beta"])
            .iter()
            .all(|decision| decision.reason == CaptureReason::RunStarted));

        assert_eq!(
            step(&mut schedule, 41, SavePhase::InProgress, 10),
            vec![
                CaptureDecision {
                    tick: 41,
                    slot: CaptureSlot::Manual("alpha".into()),
                    reason: CaptureReason::Manual,
                },
                CaptureDecision {
                    tick: 41,
                    slot: CaptureSlot::Manual("alpha".into()),
                    reason: CaptureReason::Manual,
                },
                CaptureDecision {
                    tick: 41,
                    slot: CaptureSlot::Manual("beta".into()),
                    reason: CaptureReason::Manual,
                },
            ]
        );
    }

    #[test]
    fn manual_requests_due_after_terminal_or_lobby_boundaries_are_dropped() {
        for boundary in [SavePhase::GameOver, SavePhase::BeforeRun] {
            let mut schedule = SaveSchedule::default();
            step(&mut schedule, 40, SavePhase::InProgress, 10);
            assert!(schedule
                .step(40, SavePhase::InProgress, 10, ["alpha", "beta"])
                .is_empty());

            let output =
                schedule.step_with_outcomes(41, boundary, 10, std::iter::empty::<String>());
            assert!(output
                .decisions
                .iter()
                .all(|decision| decision.reason != CaptureReason::Manual));
            assert_eq!(
                output.refused_manual,
                ["alpha", "beta"]
                    .into_iter()
                    .map(|slot_id| RefusedManualSave {
                        tick: 41,
                        slot_id: slot_id.into(),
                        reason: ManualSaveRefusalReason::PhaseChanged { phase: boundary },
                    })
                    .collect::<Vec<_>>()
            );

            // A refused request is consumed at its due boundary. It must not
            // appear later when another run becomes capturable.
            let restarted = step(&mut schedule, 50, SavePhase::InProgress, 10);
            assert!(restarted
                .iter()
                .all(|decision| decision.reason != CaptureReason::Manual));
        }
    }

    #[test]
    fn game_over_still_emits_only_the_automatic_final_capture() {
        let mut schedule = SaveSchedule::default();
        step(&mut schedule, 8, SavePhase::InProgress, 10);
        assert!(schedule
            .step(8, SavePhase::InProgress, 10, ["manual"])
            .is_empty());

        assert_eq!(
            step(&mut schedule, 9, SavePhase::GameOver, 10),
            vec![CaptureDecision {
                tick: 9,
                slot: CaptureSlot::RollingAutosave,
                reason: CaptureReason::GameOver,
            }]
        );
    }

    #[test]
    fn automatic_decision_precedes_manual_decisions_due_on_the_same_tick() {
        let mut schedule = SaveSchedule::default();
        step(&mut schedule, 0, SavePhase::InProgress, 5);
        schedule.step(4, SavePhase::InProgress, 5, ["manual-1", "manual-2"]);

        let decisions = step(&mut schedule, 5, SavePhase::InProgress, 5);
        assert_eq!(
            decisions
                .iter()
                .map(|decision| decision.reason)
                .collect::<Vec<_>>(),
            vec![
                CaptureReason::Periodic,
                CaptureReason::Manual,
                CaptureReason::Manual
            ]
        );
    }

    #[test]
    fn returning_before_run_resets_the_automatic_run_boundary() {
        let mut schedule = SaveSchedule::default();
        step(&mut schedule, 3, SavePhase::InProgress, 10);
        step(&mut schedule, 4, SavePhase::GameOver, 10);
        step(&mut schedule, 5, SavePhase::BeforeRun, 10);

        assert_eq!(
            step(&mut schedule, 30, SavePhase::InProgress, 10)[0].reason,
            CaptureReason::RunStarted
        );
        assert_eq!(
            step(&mut schedule, 40, SavePhase::InProgress, 10)[0].reason,
            CaptureReason::Periodic
        );
    }

    #[test]
    fn restored_continuation_rebases_periodic_cadence_without_run_start() {
        let mut schedule = SaveSchedule::default();
        step(&mut schedule, 0, SavePhase::InProgress, 15);
        schedule.queue_manual_for_tick(61, ["pre-restore"]);
        assert_eq!(
            schedule.refuse_pending_manual(61, ManualSaveRefusalReason::StartupRestorePending),
            vec![RefusedManualSave {
                tick: 61,
                slot_id: "pre-restore".into(),
                reason: ManualSaveRefusalReason::StartupRestorePending,
            }]
        );

        schedule.rebase_after_restore(61, SavePhase::InProgress);
        for tick in 61..76 {
            assert!(step(&mut schedule, tick, SavePhase::InProgress, 15).is_empty());
        }
        assert_eq!(
            step(&mut schedule, 76, SavePhase::InProgress, 15),
            vec![CaptureDecision {
                tick: 76,
                slot: CaptureSlot::RollingAutosave,
                reason: CaptureReason::Periodic,
            }]
        );

        schedule.rebase_after_restore(90, SavePhase::GameOver);
        assert!(step(&mut schedule, 90, SavePhase::GameOver, 15).is_empty());
    }

    #[test]
    fn manual_ids_are_store_safe_and_display_names_never_become_keys() {
        let store = FakeStore::default();
        let run = stored_run(12, "scenario-a", 41, current_versions());
        let display_name = "Same / name 🌌\nwith punctuation";

        let first = create_manual_save(&store, display_name, &run).expect("first manual save");
        let second = create_manual_save(&store, display_name, &run).expect("second manual save");

        assert_ne!(first, second);
        for slot in [&first, &second] {
            let parsed = uuid::Uuid::parse_str(slot).expect("internal id is UUID-shaped");
            assert_eq!(parsed.to_string(), *slot);
            assert!(vellum_save::is_slot(slot));
            assert!(store.contains(slot));
            assert!(store.contains(&metadata_slot(slot)));
        }
        assert!(!store.contains(display_name));

        let entries = list_slots(&store, &current_versions()).expect("catalogue lists");
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|entry| entry.display_name == display_name));
    }

    #[test]
    fn stores_are_isolated_and_autosave_rolls_without_touching_manual_rows() {
        let first = FakeStore::default();
        let second = FakeStore::default();
        let old = stored_run(4, "first", 1, current_versions());
        let new = stored_run(8, "first", 2, current_versions());
        let other = stored_run(6, "second", 3, current_versions());

        write_autosave(&first, &old).expect("first autosave");
        write_manual_save(&first, SLOT_A, "manual", &old).expect("first manual");
        write_autosave(&second, &other).expect("second autosave");
        write_autosave(&first, &new).expect("rolling replacement");

        assert_eq!(
            load_slot(&first, AUTOSAVE_SLOT, &current_versions())
                .expect("first loads")
                .seed,
            2
        );
        assert_eq!(
            load_slot(&second, AUTOSAVE_SLOT, &current_versions())
                .expect("second loads")
                .seed,
            3
        );
        assert!(first.contains(SLOT_A));
        assert!(!second.contains(SLOT_A));

        delete_slot(&first, SLOT_A).expect("delete is local");
        assert!(!first.contains(SLOT_A));
        assert_eq!(list_slots(&second, &current_versions()).unwrap().len(), 1);
    }

    #[test]
    fn listing_filters_sidecars_and_is_stably_ordered_from_run_ticks() {
        let store = FakeStore::default();
        write_autosave(&store, &stored_run(3, "auto", 30, current_versions())).unwrap();
        write_manual_save(
            &store,
            SLOT_A,
            "older",
            &stored_run(10, "older-scenario", 10, current_versions()),
        )
        .unwrap();
        write_manual_save(
            &store,
            SLOT_C,
            "same tick c",
            &stored_run(20, "new-scenario", 31, current_versions()),
        )
        .unwrap();
        write_manual_save(
            &store,
            SLOT_B,
            "same tick b",
            &stored_run(20, "new-scenario", 32, current_versions()),
        )
        .unwrap();
        store.put(SLOT_D, "not a run");
        store.put("metadata-orphan", "ignored");
        store.put("another_subsystem", "ignored");

        let entries = list_slots(&store, &current_versions()).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.slot_id.as_str())
                .collect::<Vec<_>>(),
            vec![AUTOSAVE_SLOT, SLOT_B, SLOT_C, SLOT_A, SLOT_D]
        );
        let newest = manual_entry(&entries, SLOT_B);
        assert_eq!(
            newest.record,
            Some(SaveRecordSummary {
                scenario: "new-scenario".into(),
                seed: 32,
                capture_tick: 20,
                boot_identity: Some(crate::snapshot::BootIdentity {
                    selected_ship: "assets/entities/alliance_cruiser.toml".into(),
                    fleet: crate::lockstep::FleetRoster::default(),
                    game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                        authored_index: 0,
                        entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                    }],
                }),
                versions: current_versions(),
            })
        );
        assert!(newest.can_start());
        assert!(matches!(
            manual_entry(&entries, SLOT_D).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Unparsable(_))
        ));
    }

    #[test]
    fn missing_and_corrupt_metadata_fall_back_to_internal_id_and_can_be_renamed() {
        let store = FakeStore::default();
        let run = stored_run(5, "scenario", 9, current_versions());
        put_run(&store, SLOT_A, &run);
        put_run(&store, SLOT_B, &run);
        store.put(&metadata_slot(SLOT_B), "damaged-sidecar");

        let entries = list_slots(&store, &current_versions()).unwrap();
        let missing = manual_entry(&entries, SLOT_A);
        assert_eq!(missing.display_name, SLOT_A);
        assert_eq!(missing.metadata, MetadataStatus::Missing);
        let corrupt = manual_entry(&entries, SLOT_B);
        assert_eq!(corrupt.display_name, SLOT_B);
        assert_eq!(corrupt.metadata, MetadataStatus::Corrupt);

        rename_slot(&store, SLOT_B, "Recovered 🛰️").expect("rename repairs sidecar");
        let entries = list_slots(&store, &current_versions()).unwrap();
        let renamed = manual_entry(&entries, SLOT_B);
        assert_eq!(renamed.display_name, "Recovered 🛰️");
        assert_eq!(renamed.metadata, MetadataStatus::Present);
        assert_eq!(renamed.slot_id, SLOT_B);
    }

    #[test]
    fn every_version_refusal_and_bad_record_remains_a_deletable_row() {
        let store = FakeStore::default();
        let current = current_versions();
        put_run(&store, SLOT_A, &stored_run(1, "ready", 1, current.clone()));
        put_run(
            &store,
            SLOT_B,
            &stored_run(
                2,
                "format",
                2,
                vellum_save::Versions::new(8, "rules-a", 0x1234),
            ),
        );
        put_run(
            &store,
            SLOT_C,
            &stored_run(
                3,
                "rules",
                3,
                vellum_save::Versions::new(7, "rules-b", 0x1234),
            ),
        );
        put_run(
            &store,
            SLOT_D,
            &stored_run(
                4,
                "content",
                4,
                vellum_save::Versions::new(7, "rules-a", 0x9999),
            ),
        );
        store.put(SLOT_E, "corrupt run");
        put_run(
            &store,
            SLOT_F,
            &stored_run(6, "unreadable", 6, current.clone()),
        );
        store.fail_next(FakeOperation::Read, SLOT_F, "device unavailable");

        let entries = list_slots(&store, &current).unwrap();
        assert_eq!(manual_entry(&entries, SLOT_A).start, StartState::Ready);
        assert!(matches!(
            manual_entry(&entries, SLOT_B).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Format { .. }
            ))
        ));
        assert!(matches!(
            manual_entry(&entries, SLOT_C).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Rules { .. }
            ))
        ));
        assert!(matches!(
            manual_entry(&entries, SLOT_D).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Content { .. }
            ))
        ));
        assert!(matches!(
            manual_entry(&entries, SLOT_E).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Unparsable(_))
        ));
        assert_eq!(
            manual_entry(&entries, SLOT_F).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Unreadable(
                "device unavailable".into()
            ))
        );

        assert!(matches!(
            load_slot(&store, SLOT_B, &current),
            Err(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Format { .. }
            ))
        ));
        for slot in [SLOT_B, SLOT_C, SLOT_D, SLOT_E, SLOT_F] {
            delete_slot(&store, slot).expect("refused rows remain deletable");
            assert!(!store.contains(slot));
        }
    }

    #[test]
    fn pre_contact_override_scenario_ron_keeps_metadata_and_reaches_format_refusal() {
        let store = FakeStore::default();
        let current =
            vellum_save::Versions::new(crate::snapshot::SNAPSHOT_FORMAT, "rules-a", 0x1234);
        let mut old = stored_run(
            42,
            "historical-scenario",
            1309,
            vellum_save::Versions::new(27, "rules-a", 0x1234),
        );
        old.snapshot.as_mut().unwrap().state.scenario =
            Some(crate::snapshot::ScenarioState::default());
        let mut ron = old.to_ron().unwrap();
        // Remove the new field itself: merely editing the version number leaves
        // a current-shaped payload and cannot exercise historical deserialization.
        let start = ron
            .find("contact_overrides:")
            .expect("current scenario writes its contact map");
        let end = start + ron[start..].find('}').unwrap() + 1;
        assert!(ron[start..end].ends_with("{}"));
        let comma = end + ron[end..].find(',').unwrap();
        assert!(ron[end..comma].trim().is_empty());
        ron.replace_range(start..=comma, "");
        assert!(!ron.contains("contact_overrides"));
        store.put(SLOT_A, &ron);
        let parsed = crate::snapshot::StoredRun::from_ron(&ron).unwrap();
        assert!(parsed
            .snapshot
            .unwrap()
            .state
            .scenario
            .unwrap()
            .contact_overrides
            .is_empty());
        let entries = list_slots(&store, &current).unwrap();
        let entry = manual_entry(&entries, SLOT_A);
        let summary = entry
            .record
            .as_ref()
            .expect("historical metadata survives parsing");
        assert_eq!(summary.scenario, "historical-scenario");
        assert_eq!(summary.seed, 1309);
        assert_eq!(summary.capture_tick, 42);
        assert!(matches!(
            &entry.start,
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Format { .. }
            ))
        ));
        assert!(matches!(
            load_slot(&store, SLOT_A, &current),
            Err(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Format { .. }
            ))
        ));
        assert!(matches!(
            crate::snapshot::load_from(&store, SLOT_A, &current),
            Err(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Format { .. }
            ))
        ));
    }

    #[test]
    fn unfrozen_catalogue_defers_only_content_and_full_load_still_refuses_it() {
        let store = FakeStore::default();
        let current = current_versions();
        put_run(&store, SLOT_A, &stored_run(1, "ready", 1, current.clone()));
        put_run(
            &store,
            SLOT_B,
            &stored_run(
                2,
                "format",
                2,
                vellum_save::Versions::new(8, "rules-a", 0x1234),
            ),
        );
        put_run(
            &store,
            SLOT_C,
            &stored_run(
                3,
                "rules",
                3,
                vellum_save::Versions::new(7, "rules-b", 0x1234),
            ),
        );
        put_run(
            &store,
            SLOT_D,
            &stored_run(
                4,
                "content",
                4,
                vellum_save::Versions::new(7, "rules-a", 0x9999),
            ),
        );
        store.put(SLOT_E, "corrupt run");

        let entries = list_slots_with_content_check(&store, &current, ContentCheck::Deferred)
            .expect("unfrozen catalogue lists");
        assert_eq!(manual_entry(&entries, SLOT_A).start, StartState::Ready);
        assert_eq!(
            manual_entry(&entries, SLOT_D).start,
            StartState::ContentDeferred
        );
        assert!(manual_entry(&entries, SLOT_A).can_start());
        assert!(manual_entry(&entries, SLOT_D).can_start());
        for slot in [SLOT_B, SLOT_C, SLOT_E] {
            assert!(!manual_entry(&entries, slot).can_start());
        }

        assert!(matches!(
            manual_entry(&entries, SLOT_B).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Format { .. }
            ))
        ));
        assert!(matches!(
            manual_entry(&entries, SLOT_C).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Rules { .. }
            ))
        ));
        assert!(matches!(
            manual_entry(&entries, SLOT_E).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Unparsable(_))
        ));

        // Deferred is presentation/readiness only. The selected row still
        // crosses the existing full Versions::check after its world loads.
        assert!(matches!(
            load_slot(&store, SLOT_D, &current),
            Err(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Content { .. }
            ))
        ));
        assert!(matches!(
            manual_entry(
                &list_slots_with_content_check(&store, &current, ContentCheck::Full).unwrap(),
                SLOT_D
            )
            .start,
            StartState::Refused(crate::snapshot::LoadRefusal::Moved(
                vellum_save::Moved::Content { .. }
            ))
        ));
    }

    #[test]
    fn an_advertised_but_missing_run_is_empty_and_still_deletable() {
        let store = FakeStore::default();
        store.advertise(SLOT_A);
        let entries = list_slots(&store, &current_versions()).unwrap();
        assert_eq!(
            manual_entry(&entries, SLOT_A).start,
            StartState::Refused(crate::snapshot::LoadRefusal::Empty)
        );
        delete_slot(&store, SLOT_A).expect("Store removal is idempotent");
    }

    #[test]
    fn rename_and_delete_failures_preserve_a_visible_retry_path() {
        let rename_store = FakeStore::default();
        let run = stored_run(5, "scenario", 7, current_versions());
        write_manual_save(&rename_store, SLOT_A, "before", &run).unwrap();
        let sidecar = metadata_slot(SLOT_A);
        let before = rename_store.get(&sidecar).unwrap();
        rename_store.fail_next(FakeOperation::Write, &sidecar, "write refused");
        assert!(matches!(
            rename_slot(&rename_store, SLOT_A, "after"),
            Err(CatalogueError::Store {
                operation: CatalogueOperation::WriteMetadata,
                ..
            })
        ));
        assert_eq!(rename_store.get(&sidecar).unwrap(), before);

        let metadata_failure = FakeStore::default();
        write_manual_save(&metadata_failure, SLOT_B, "name", &run).unwrap();
        let sidecar = metadata_slot(SLOT_B);
        metadata_failure.fail_next(FakeOperation::Remove, &sidecar, "locked metadata");
        assert!(matches!(
            delete_slot(&metadata_failure, SLOT_B),
            Err(CatalogueError::Store {
                operation: CatalogueOperation::RemoveMetadata,
                ..
            })
        ));
        assert!(metadata_failure.contains(SLOT_B));
        assert!(metadata_failure.contains(&sidecar));

        let run_failure = FakeStore::default();
        write_manual_save(&run_failure, SLOT_C, "name", &run).unwrap();
        run_failure.fail_next(FakeOperation::Remove, SLOT_C, "locked run");
        assert!(matches!(
            delete_slot(&run_failure, SLOT_C),
            Err(CatalogueError::Partial {
                operation: CatalogueOperation::RemoveRun,
                rollback_detail: None,
                ..
            })
        ));
        assert!(run_failure.contains(SLOT_C));
        assert!(!run_failure.contains(&metadata_slot(SLOT_C)));
        assert_eq!(
            manual_entry(
                &list_slots(&run_failure, &current_versions()).unwrap(),
                SLOT_C
            )
            .metadata,
            MetadataStatus::Missing
        );
        delete_slot(&run_failure, SLOT_C).expect("retry removes visible fallback row");
    }

    #[test]
    fn failed_metadata_create_rolls_back_or_reports_the_surviving_run() {
        let run = stored_run(7, "scenario", 11, current_versions());
        let rolled_back = FakeStore::default();
        rolled_back.fail_next(
            FakeOperation::Write,
            &metadata_slot(SLOT_A),
            "metadata full",
        );
        assert!(matches!(
            write_manual_save(&rolled_back, SLOT_A, "name", &run),
            Err(CatalogueError::Store {
                operation: CatalogueOperation::WriteMetadata,
                ..
            })
        ));
        assert!(!rolled_back.contains(SLOT_A));

        let partial = FakeStore::default();
        partial.fail_next(
            FakeOperation::Write,
            &metadata_slot(SLOT_B),
            "metadata full",
        );
        partial.fail_next(FakeOperation::Remove, SLOT_B, "rollback locked");
        assert!(matches!(
            write_manual_save(&partial, SLOT_B, "name", &run),
            Err(CatalogueError::Partial {
                operation: CatalogueOperation::WriteMetadata,
                rollback_detail: Some(_),
                ..
            })
        ));
        assert!(partial.contains(SLOT_B));
        let entries = list_slots(&partial, &current_versions()).unwrap();
        assert_eq!(
            manual_entry(&entries, SLOT_B).metadata,
            MetadataStatus::Missing
        );
    }

    #[test]
    fn selected_slot_export_uses_the_canonical_run_and_does_not_add_a_gate() {
        let store = FakeStore::default();
        let old_versions = vellum_save::Versions::new(6, "rules-old", 0x7777);
        let selected = stored_run(44, "selected", 88, old_versions);
        let other = stored_run(45, "other", 99, current_versions());
        put_run(&store, SLOT_A, &selected);
        put_run(&store, SLOT_B, &other);

        let text = export_slot(&store, SLOT_A).expect("incompatible saves remain exportable");
        assert_eq!(
            crate::snapshot::StoredRun::from_ron(&text).expect("export is a StoredRun"),
            selected
        );
        assert!(matches!(
            load_slot(&store, SLOT_A, &current_versions()),
            Err(crate::snapshot::LoadRefusal::Moved(_))
        ));
        assert_eq!(
            load_slot(&store, SLOT_B, &current_versions())
                .expect("selected other run remains compatible")
                .seed,
            99
        );

        store.put(SLOT_C, "broken");
        assert!(matches!(
            export_slot(&store, SLOT_C),
            Err(crate::snapshot::LoadRefusal::Unparsable(_))
        ));
        assert_eq!(
            export_slot(&store, SLOT_D),
            Err(crate::snapshot::LoadRefusal::Empty)
        );
    }

    #[test]
    fn list_errors_and_reserved_or_invalid_mutations_are_structural() {
        let store = FakeStore::default();
        store.fail_next(FakeOperation::Slots, "", "catalogue unavailable");
        assert!(matches!(
            list_slots(&store, &current_versions()),
            Err(CatalogueError::Store {
                operation: CatalogueOperation::List,
                ..
            })
        ));
        assert_eq!(
            rename_slot(&store, AUTOSAVE_SLOT, "name"),
            Err(CatalogueError::ReservedAutosave)
        );
        assert_eq!(
            delete_slot(&store, "../escape"),
            Err(CatalogueError::InvalidSlot("../escape".into()))
        );
    }
}
