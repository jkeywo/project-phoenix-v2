//! The Bevy side of the portable-record transfer (issue #1117).
//!
//! [`crate::lockstep::transfer`] is the pure protocol — chunk, reassemble,
//! integrity. This module is its adapter into a running host: it captures the one
//! canonical [`crate::snapshot`] record on the sending side, receives and
//! reassembles the chunks on the other, runs the SAME version and content gate a
//! local load runs, restores through the SAME `snapshot::restore` walk, and proves
//! the post-restore agreement the same way `tests/snapshot_resume.rs` and
//! `src/server/bridge.rs` already do.
//!
//! # No second serializer, said three ways
//!
//! * The sender's bytes come from [`crate::snapshot::export_artifact`] — the same
//!   `TransferStore`-backed RON a file export produces.
//! * The receiver's gate is [`crate::snapshot::import_artifact`] — the same
//!   `load_from` + `Versions::check` a file import runs, refusing before a single
//!   component is written.
//! * The receiver's restore is [`crate::snapshot::restore`] and the post-restore
//!   check is [`crate::sim_digest::world_digest`] against the recorded digest —
//!   the exact pair `drain_snapshot_restore` uses.
//!
//! This module adds framing around those calls and nothing to them.
//!
//! # Why the receiver is not gated on being in the fleet
//!
//! Receiving the record is HOW a host becomes an agreeing member (a join, or a
//! #1120 slot recovery). So [`crate::lockstep::apply_mesh_inbox`] hands snapshot
//! chunks here whether or not a [`crate::lockstep::FleetLockstep`] session exists
//! yet — unlike tick and digest frames, which have no meaning before the barrier
//! is up. The chunks only ever accumulate a buffer bounded by
//! [`crate::lockstep::transfer::SNAPSHOT_MAX_TRANSFER_BYTES`]; nothing is
//! committed until the whole record has arrived, gated and restored.
//!
//! That single-transfer bound is the pure [`SnapshotReceiver`]'s. This adapter
//! adds one more slot on top of it: a completed record sits in `staged` until
//! [`drain_mesh_restore`] consumes it next system, so if one drain accepts
//! transfer A's final chunk (staging it) and then transfer B's first chunk, the
//! receiver transiently holds up to ~2x `SNAPSHOT_MAX_TRANSFER_BYTES` — one staged
//! record plus one fresh in-flight transfer. Still bounded, and back down to one
//! the next time `drain_mesh_restore` runs.

use bevy::prelude::*;

use vellum_save::Versions;

use crate::command_admission::log::HostSlot;
use crate::lockstep::transfer::{self, Accepted, SnapshotChunk, SnapshotReceiver, TransferError};
use crate::lockstep::{MeshFrame, MeshOutbox};
use crate::logging::LogCat;
use crate::snapshot::{self, StoredRun};

/// What a received transfer resolved to, once every chunk was in hand.
///
/// Refusals are NOT uniformly clean, and the boundary is load-bearing for a host
/// deciding whether it can retry in place. The version/content gate runs BEFORE a
/// single component is written, so a [`Self::RefusedGate`] on a build, rules or
/// content mismatch leaves the receiving world byte-identical — that transfer is
/// discarded and the host is exactly where it was. But [`Self::RefusedIntegrity`]
/// and [`Self::Incomplete`] are decided AFTER `snapshot::restore` has already
/// overwritten the world, and there is no rollback: the world is left holding the
/// very record the check then judged bad. A host that hits one cannot simply retry
/// against the same world — it must re-bootstrap the scenario to a clean state
/// first. (A `RefusedGate` naming a world layer that could not be reconstructed is
/// the one gate-side exception: `reconcile_world_layers` has already mutated layer
/// state by the time it returns, so that sub-case is not byte-clean either.)
///
/// #1118 (divergence recovery) is expected to close this asymmetry: restore into a
/// rollback-able checkpoint and swap the live world in only on a matching fold, so
/// an integrity/incomplete outcome becomes a discarded checkpoint rather than a
/// destroyed world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshRestoreOutcome {
    /// The gate passed, the restore was complete, and the restored world folds to
    /// the digest the capture recorded — a correct join.
    Committed { tick: u64, digest: u64 },
    /// The version/content gate refused it: a save from a different build, a
    /// different rules revision, or different scenario content. The world is
    /// untouched. The string is `vellum_save::Moved`'s own sentence — which
    /// dimension moved and to what — or a parse failure.
    RefusedGate(String),
    /// The whole payload reassembled and passed the gate, but the restored world
    /// did not fold to the recorded digest: the state is intact enough to parse
    /// and gate yet does not reproduce, which the corruption check names.
    RefusedIntegrity { recorded: u64, restored: u64 },
    /// The restore ran but the fresh world could not house every captured row
    /// (missing entities, an unreconciled layer). Reported rather than hidden.
    Incomplete { tick: u64, gaps: usize },
    /// The receiving world is not yet far enough along to restore into — its
    /// layers have not reconciled, or its authored entities do not exist yet. The
    /// caller should retry on a later frame; the reassembled record is retained.
    NotReady,
}

/// A receiver's in-progress transfer plus the outcome of the last one.
///
/// One per host. Present on every host `register_lockstep` touches, because a
/// host that is not expecting a transfer simply never has a chunk pushed into it.
#[derive(Resource, Default)]
pub struct MeshSnapshotReceiver {
    rx: SnapshotReceiver,
    /// A completed, verified record waiting for a frame on which the world is
    /// ready to restore into. Held here rather than restored inline because a
    /// restore needs exclusive world access and a chunk arrives in a query system.
    staged: Option<String>,
    /// The most recent transfer's outcome, for the operator surface and the tests.
    last: Option<MeshRestoreOutcome>,
    /// The most recent chunk-level fault, if any — a corrupt or oversized piece.
    last_fault: Option<TransferError>,
}

impl MeshSnapshotReceiver {
    /// The most recent completed transfer's outcome.
    pub fn last_outcome(&self) -> Option<&MeshRestoreOutcome> {
        self.last.as_ref()
    }

    /// The most recent chunk-level fault, if the last chunk was refused.
    pub fn last_fault(&self) -> Option<&TransferError> {
        self.last_fault.as_ref()
    }

    /// Whether a record has fully arrived and is waiting to be restored.
    pub fn has_staged_record(&self) -> bool {
        self.staged.is_some()
    }

    /// Whether a transfer is mid-flight.
    pub fn is_receiving(&self) -> bool {
        self.rx.is_receiving()
    }

    /// The sequences still outstanding on the transfer in progress.
    pub fn missing(&self) -> Vec<u32> {
        self.rx.missing()
    }

    /// Take one chunk into the transfer, staging the record when it completes.
    ///
    /// The log-free core of [`receive_chunk`]: [`receive_chunk`] calls this and
    /// then narrates the outcome. Returned so a caller (a test, or the system)
    /// can see whether the transfer advanced, completed or was refused.
    pub fn accept_chunk(&mut self, chunk: &SnapshotChunk) -> Result<Accepted, TransferError> {
        let outcome = self.rx.accept(chunk);
        match &outcome {
            Ok(Accepted::Complete(text)) => {
                self.last_fault = None;
                self.staged = Some(text.clone());
            }
            Ok(Accepted::More { .. }) => self.last_fault = None,
            Err(fault) => self.last_fault = Some(fault.clone()),
        }
        outcome
    }
}

/// Take one snapshot chunk off the mesh (issue #1117).
///
/// Called by [`crate::lockstep::apply_mesh_inbox`] for every
/// [`MeshFrame::Snapshot`], whether or not this host is a lockstep participant.
/// A completed transfer stages its text for [`drain_mesh_restore`]; a chunk-level
/// fault is recorded and the buffer is left as it was.
pub fn receive_chunk(
    receiver: &mut MeshSnapshotReceiver,
    chunk: &SnapshotChunk,
    log: &Option<Res<crate::logging::LogFilterConfig>>,
) {
    match receiver.accept_chunk(chunk) {
        Ok(Accepted::More { received, total }) => {
            crate::pdebug!(
                log,
                LogCat::Admit,
                "host-mesh snapshot chunk {}/{} received from {}",
                received,
                total,
                chunk.from.slot_id(),
            );
        }
        Ok(Accepted::Complete(text)) => {
            crate::pinfo!(
                log,
                LogCat::Admit,
                "host-mesh snapshot fully received from {} ({} bytes) — awaiting a \
                 frame ready to restore",
                chunk.from.slot_id(),
                text.len(),
            );
        }
        Err(fault) => {
            crate::pwarn!(
                log,
                LogCat::Admit,
                "host-mesh snapshot chunk from {} refused: {fault}",
                chunk.from.slot_id(),
            );
        }
    }
}

/// Capture this host's authoritative record for transfer (issue #1117).
///
/// Reuses [`crate::snapshot::capture`], [`crate::sim_digest::world_digest`] and
/// [`crate::snapshot::run_for`] exactly as `drain_snapshot_save` does — the same
/// `StoredRun` a local save writes. `scenario` is the world path the host loaded;
/// it rides in the record so a receiver that is NOT already running this world
/// knows which one it belongs to (a fleet member already is).
pub fn capture_run(world: &World, scenario: impl Into<String>) -> StoredRun {
    let payload = snapshot::capture(world);
    let digest = crate::sim_digest::world_digest(world);
    let seed = world
        .get_resource::<crate::sim_rng::SimRng>()
        .map_or(0, |rng| rng.seed());
    snapshot::run_for(
        payload,
        digest,
        seed,
        scenario,
        snapshot::versions(&crate::content_ledger::frozen_or_live()),
    )
}

/// Frame a captured record into the mesh chunks that carry it (issue #1117).
///
/// [`crate::snapshot::export_artifact`] produces the exact RON a file export
/// would; [`transfer::chunk`] frames it. No state is walked here — the bytes are
/// already the whole record.
pub fn frames_for(
    run: &StoredRun,
    from: HostSlot,
    transfer_id: u64,
) -> Result<Vec<MeshFrame>, String> {
    let text = snapshot::export_artifact(run)?;
    let tick = run.snapshot.as_ref().map_or(0, |s| s.tick);
    Ok(transfer::chunk(&text, from, transfer_id, tick)
        .into_iter()
        .map(MeshFrame::Snapshot)
        .collect())
}

/// Capture, frame, and queue a transfer to the fleet in one call (issue #1117).
///
/// The operator-facing sender: everything from "read the world" to "hand the
/// transport its frames" through the seams above. Returns how many chunks were
/// queued, or the reason a capture could not be framed.
pub fn send_snapshot(
    world: &mut World,
    from: HostSlot,
    transfer_id: u64,
    scenario: impl Into<String>,
) -> Result<usize, String> {
    let run = capture_run(world, scenario);
    let frames = frames_for(&run, from, transfer_id)?;
    let count = frames.len();
    let mut outbox = world
        .get_resource_mut::<MeshOutbox>()
        .ok_or_else(|| "this host has no MeshOutbox — register_lockstep has not run".to_string())?;
    for frame in frames {
        outbox.push(frame);
    }
    Ok(count)
}

/// Gate a reassembled record and restore it, or say cleanly why not (issue #1117).
///
/// The whole receiver pipeline after reassembly, and callable directly by the
/// test harness as well as by [`drain_mesh_restore`]:
///
/// 1. **The gate runs first, before any state is written.** `import_artifact`
///    parses the record and runs `Versions::check` against THIS build's current
///    versions — format, simulation rules, and the content digest that folds every
///    authored file the world consumed (#935/#1086). A save from a different
///    build, rules revision or scenario content is refused here, world untouched.
/// 2. **The world must be ready.** A fresh receiver bootstraps the same scenario
///    before it can be overwritten; `reconcile_world_layers` + `ready_to_restore`
///    are the same preconditions `drain_snapshot_restore` waits on. Not ready ⇒
///    [`MeshRestoreOutcome::NotReady`], record retained for a later frame.
/// 3. **Restore, then verify.** `snapshot::restore` overwrites by uuid, and the
///    restored `world_digest` must equal the recorded one — the corruption check.
pub fn gate_and_restore(world: &mut World, text: &str) -> MeshRestoreOutcome {
    let current = snapshot::versions(&crate::content_ledger::frozen_or_live());
    gate_and_restore_against(world, text, &current)
}

/// [`gate_and_restore`] against an explicit `current` version, so a test can prove
/// the gate refuses a record from a different build without loading a second
/// world.
pub fn gate_and_restore_against(
    world: &mut World,
    text: &str,
    current: &Versions,
) -> MeshRestoreOutcome {
    // 1. The gate. Parse and version-check BEFORE touching the world.
    let run = match snapshot::import_artifact(text, current) {
        Ok(run) => run,
        Err(refusal) => return MeshRestoreOutcome::RefusedGate(refusal.to_string()),
    };
    let Some(snap) = run.snapshot.as_ref() else {
        return MeshRestoreOutcome::RefusedGate(
            "the transferred record carries no captured state".to_string(),
        );
    };

    // 2. Readiness. Reconcile the captured layer topology, then check the roster
    //    is far enough along to overwrite — exactly `drain_snapshot_restore`'s
    //    preconditions.
    match snapshot::reconcile_world_layers(world, &snap.state) {
        snapshot::LayerReconcileStatus::Ready => {}
        snapshot::LayerReconcileStatus::Waiting => return MeshRestoreOutcome::NotReady,
        snapshot::LayerReconcileStatus::Failed(path) => {
            return MeshRestoreOutcome::RefusedGate(format!(
                "the record requires world layer '{path}', which could not be reconstructed"
            ))
        }
    }
    if !snapshot::ready_to_restore(world, &snap.state) {
        return MeshRestoreOutcome::NotReady;
    }

    // 3. Restore, then verify the fold. `restore` overwrites by uuid; the recorded
    //    digest is recomputed BY the restored simulation, so a tampered or
    //    truncated record cannot restore to it.
    let report = snapshot::restore(world, &snap.state);
    let restored = crate::sim_digest::world_digest(world);
    if restored != snap.digest {
        return MeshRestoreOutcome::RefusedIntegrity {
            recorded: snap.digest,
            restored,
        };
    }
    if report.is_complete() {
        MeshRestoreOutcome::Committed {
            tick: snap.tick,
            digest: snap.digest,
        }
    } else {
        MeshRestoreOutcome::Incomplete {
            tick: snap.tick,
            gaps: report.gaps.len(),
        }
    }
}

/// Restore a staged record the moment the world is ready (issue #1117).
///
/// Exclusive, in `PreUpdate` after the mesh has drained its inbox, so a record
/// that completed this frame is restored before this frame's fixed steps run and
/// before the digest exchange samples. A [`MeshRestoreOutcome::NotReady`] leaves
/// the record staged and tries again next frame; anything else — committed or
/// refused — clears it, because a refused record is not retried against the same
/// unchanging world.
pub fn drain_mesh_restore(world: &mut World) {
    let staged = world
        .get_resource::<MeshSnapshotReceiver>()
        .and_then(|r| r.staged.clone());
    let Some(text) = staged else {
        return;
    };
    let outcome = gate_and_restore(world, &text);
    if matches!(outcome, MeshRestoreOutcome::NotReady) {
        return;
    }
    if let Some(mut receiver) = world.get_resource_mut::<MeshSnapshotReceiver>() {
        receiver.staged = None;
        receiver.last = Some(outcome);
    }
}

/// Install the receiver resource and the restore system.
///
/// Called from [`crate::lockstep::register_lockstep`]. The restore system is a
/// digest EXCLUSION declaration's concern only in that the receiver resource is —
/// it holds a transient buffer and the last outcome, never simulation state.
pub fn register_snapshot_relay(app: &mut App) {
    {
        use crate::authoritative::{DeclareState, StateClass};
        // The transfer buffer and the last outcome. `ClearedAtFold` in spirit —
        // the buffer is empty between transfers and the outcome is a report about
        // a restore, not state of the world. Declared so the #894 census stays
        // honest that nothing here is folded.
        app.declare_state::<MeshSnapshotReceiver>(
            StateClass::ClearedAtFold,
            "fleet-snapshot-transfer-state",
        );
    }
    // `.after(MeshSet)` so a record completed by this frame's inbox drain is
    // restored the same frame. No explicit `.before(advance_sim_tick)`: that
    // system lives in `FixedLast`, a different schedule, so an ordering edge to it
    // from `PreUpdate` resolves to zero systems and constrains nothing. It is not
    // needed either — Bevy runs `PreUpdate` before `RunFixedMainLoop` by
    // construction, so this restore already precedes this frame's fixed steps and
    // the `FixedLast` digest sample; the ordering holds across schedules, not
    // within one, and cannot be spelled as an edge.
    app.init_resource::<MeshSnapshotReceiver>().add_systems(
        PreUpdate,
        drain_mesh_restore.after(crate::lockstep::MeshSet),
    );
}
