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
/// **Every refusal now leaves the receiving world byte-identical to what it was**
/// — issue #1118 closed the asymmetry #1117 flagged here. The version/content gate
/// still runs BEFORE a single component is written, so a [`Self::RefusedGate`] on a
/// build, rules or content mismatch is untouched by construction. And
/// [`Self::RefusedIntegrity`] and [`Self::Incomplete`], which are decided only
/// AFTER `snapshot::restore` has walked the world, now restore into a
/// **rollback-able checkpoint** ([`gate_and_restore_against`]): the world's live
/// state is captured first, the record is written over it, and on a fold mismatch
/// or an incomplete report the checkpoint is restored back — so a refused host is
/// left exactly where it began rather than holding the very record the check judged
/// bad. (A `RefusedGate` naming a world layer that could not be reconstructed
/// remains the one gate-side exception: `reconcile_world_layers` mutates layer
/// state before it returns, so that sub-case is not byte-clean; it is a gate
/// refusal, not a half-restore.)
///
/// The two arm refusals sit in front of all of the above: a completed record that
/// arrives when this host is not the designated recovering host, or from a slot
/// that is not the recovery leader, is DROPPED without touching the world at all.
/// That is the receiver arm (issue #1118, AC2) — the transfer path may reassemble
/// any peer's record, but only a recovery this host is party to may commit one.
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
    /// and gate yet does not reproduce, which the corruption check names. The
    /// rollback checkpoint has been restored, so the world is back where it was.
    RefusedIntegrity { recorded: u64, restored: u64 },
    /// The restore ran but the world could not house every captured row (missing
    /// entities, an unreconciled layer). Reported rather than hidden, and the
    /// rollback checkpoint has been restored so the world is left clean.
    Incomplete { tick: u64, gaps: usize },
    /// A completed record arrived, but this host is not armed to restore one — it
    /// is not a designated recovering host (issue #1118, AC2). Dropped without
    /// touching the world: a peer must never overwrite another host's whole world
    /// simply by sending it a snapshot.
    RefusedUnarmed,
    /// This host is armed to restore, but the record came from a slot that is not
    /// the recovery leader the plan named. Dropped, world untouched — a divergent
    /// peer cannot inject its own state by racing the leader's transfer.
    RefusedWrongSender {
        armed_from: HostSlot,
        from: Option<HostSlot>,
    },
    /// Reassembly refused a corrupt, conflicting, or oversized chunk. The
    /// in-flight buffer is discarded so a join cannot remain paused behind a
    /// transfer which can no longer complete.
    RefusedChunk(String),
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
    /// The slot that sent the staged record. The reassembled text does not name
    /// its sender, so the receiver arm (issue #1118) remembers it here: a restore
    /// is committed only when this matches the recovery leader the plan named.
    staged_from: Option<HostSlot>,
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

    /// Forget the last transfer's outcome.
    ///
    /// The recovery driver (#1118) calls this when it arms a recovering host, so
    /// the host acts on THIS recovery's restore result and never on a stale one
    /// left over from an earlier transfer (a join, a prior recovery).
    pub fn clear_outcome(&mut self) {
        self.last = None;
    }

    /// The most recent chunk-level fault, if the last chunk was refused.
    pub fn last_fault(&self) -> Option<&TransferError> {
        self.last_fault.as_ref()
    }

    /// Whether a record has fully arrived and is waiting to be restored.
    pub fn has_staged_record(&self) -> bool {
        self.staged.is_some()
    }

    /// The slot that sent the staged record, if one is staged.
    pub fn staged_from(&self) -> Option<HostSlot> {
        self.staged_from
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
                self.staged_from = Some(chunk.from);
            }
            Ok(Accepted::More { .. }) => self.last_fault = None,
            Err(fault) => {
                self.last_fault = Some(fault.clone());
                self.last = Some(MeshRestoreOutcome::RefusedChunk(fault.to_string()));
                self.rx = SnapshotReceiver::default();
                self.staged = None;
                self.staged_from = None;
            }
        }
        outcome
    }
}

/// Whether this host will let a staged record overwrite its world, and from whom
/// (issue #1118, AC2 — the receiver arm).
///
/// Absent-or-disarmed by default, which is the safe state: [`drain_mesh_restore`]
/// drops a completed record rather than committing it. The divergence-recovery
/// driver ([`crate::lockstep::recovery`]) arms this on the ONE host the shared
/// recovery plan designates as recovering, naming the leader the same plan
/// elected. It is disarmed again the moment the recovery resolves.
///
/// # What the two conditions actually guarantee — and what condition (b) does not
///
/// A staged record is committed only when (a) this host is armed as a designated
/// recovering host AND (b) [`drain_mesh_restore`] does not refuse it as
/// [`MeshRestoreOutcome::RefusedWrongSender`] — i.e. its `from` slot equals the
/// leader the arm names. These two are NOT equally strong:
///
/// * **Condition (a) is the primary unbidden-overwrite protection, and it has ZERO
///   dependence on `from`.** The arm is set LOCALLY, from the shared recovery plan
///   every honest host computes identically — never from anything a peer sends. A
///   host that is not the plan's designated recovering host is never armed, so no
///   record any peer transmits can overwrite its world. This is what fully covers
///   the honest-but-diverged threat model issue #1118 targets: a peer cannot
///   overwrite another host's whole world simply by sending it a snapshot.
/// * **Condition (b) refuses an HONEST non-leader's record, but does NOT
///   authenticate the sender.** [`MeshRestoreOutcome::RefusedWrongSender`] stops a
///   divergent peer from injecting its state by racing the leader's transfer — but
///   only for honest peers. The `from` it checks is `staged_from`, copied from
///   `chunk.from` (an ordinary serialized wire field — see
///   [`crate::lockstep::transfer::SnapshotChunk::from`]), NOT an authenticated
///   delivering connection. This is the same honest-`from` trust the whole mesh
///   already rests on (see the `TODO(#1118/#1120 mesh hardening)` in
///   [`crate::lockstep::apply_mesh_inbox`]). So a BYZANTINE peer that forges
///   `from = leader` DURING an active recovery already targeting this host could
///   get a record past the arm. It would still have to pass the version/content
///   gate and fold to its OWN recorded digest — a self-consistent record, not
///   arbitrary bytes — but the arm alone does not prove the record came from the
///   elected leader.
///
/// Transport-authenticated sender identity (JS binding conn->slot so a forged
/// `from` can be rejected at the mesh boundary) is the deferred issue #1120 work.
/// Until then condition (b) is an honest-peer guard, not a byzantine one; condition
/// (a) is what makes an unarmed host uncommittable regardless of `from`.
///
/// Slot recovery and the paused GM-join transaction are the other legitimate
/// armers. They set this locally from their already-authenticated transaction;
/// an unarmed host still commits nothing.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct MeshRestoreArm {
    armed_from: Option<HostSlot>,
    /// A returning GM is a fresh private world, not an already-booted recovery
    /// target. Its accepted canonical record must stage the saved GameStart
    /// identities before that world enters `InProgress`; otherwise the normal
    /// GameStart mint creates different UUIDs and a by-UUID restore can never
    /// become ready. Ordinary divergence and slot recovery never use this mode.
    bootstrap_reconnect: bool,
    /// The bounded join bootstrap exhausted its owner-granted wait boundary,
    /// so snapshot restore may build captured authored rows which the fresh
    /// world still has not produced. Ordinary divergence/slot recovery keeps
    /// the stricter fully-bootstrapped gate.
    allow_rebuild: bool,
}

impl MeshRestoreArm {
    /// Arm this host to restore a record from `leader`.
    pub fn arm(&mut self, leader: HostSlot) {
        self.armed_from = Some(leader);
        self.bootstrap_reconnect = false;
        self.allow_rebuild = false;
    }

    /// Arm a fresh returning-GM candidate whose private GameStart identities
    /// must come from the accepted canonical record.
    pub(crate) fn arm_reconnect(&mut self, leader: HostSlot) {
        self.armed_from = Some(leader);
        self.bootstrap_reconnect = true;
        self.allow_rebuild = false;
    }

    /// At the terminal owner-granted bootstrap boundary, permit the existing
    /// #863 restore builder to supply captured rows which never appeared.
    pub fn permit_rebuild(&mut self, leader: HostSlot) {
        if self.armed_from == Some(leader) {
            self.allow_rebuild = true;
        }
    }

    /// Disarm: a staged record must not overwrite the world.
    pub fn disarm(&mut self) {
        self.armed_from = None;
        self.bootstrap_reconnect = false;
        self.allow_rebuild = false;
    }

    /// The slot this host will accept a restore from, or `None` when disarmed.
    pub fn armed_from(&self) -> Option<HostSlot> {
        self.armed_from
    }

    /// Whether this host is expecting to restore a record at all.
    pub fn is_armed(&self) -> bool {
        self.armed_from.is_some()
    }

    pub(crate) fn allows_rebuild(&self) -> bool {
        self.allow_rebuild
    }

    pub(crate) fn bootstraps_reconnect(&self) -> bool {
        self.bootstrap_reconnect
    }
}

/// Take one snapshot chunk off the mesh (issue #1117).
///
/// Called by [`crate::lockstep::apply_mesh_inbox`] for every
/// [`MeshFrame::Snapshot`], whether or not this host is a lockstep participant.
/// A completed transfer stages its text for [`drain_mesh_restore`]; a chunk-level
/// fault records a terminal outcome and clears the partial buffer so a join
/// driver can refuse visibly instead of waiting forever on a poisoned transfer.
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

/// Capture the canonical record plus the applied command history required by a
/// first-time mid-session participant (issue #1293).
///
/// [`capture_run`] intentionally preserves save-slot semantics (a snapshot with
/// an empty continuation log).  Join transfer needs the other existing half of
/// `vellum_save::Run`: the commands which led to this boundary.  The state walk
/// remains [`snapshot::capture`]; this only fills the envelope field the
/// canonical record already owns.
pub fn capture_join_run(world: &World, scenario: impl Into<String>) -> StoredRun {
    let mut run = capture_run(world, scenario);
    run.commands = world
        .get_resource::<crate::command_admission::CommandLog>()
        .map(|log| log.entries().to_vec())
        .unwrap_or_default();
    run
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
/// 3. **Checkpoint, restore, verify, and roll back on failure.** The world's live
///    state is captured first; `snapshot::restore` then overwrites by uuid; the
///    restored `world_digest` must equal the recorded one — the corruption check —
///    and the restore must be complete. On either failure the captured checkpoint
///    is restored back, so a refused host is left byte-identical to where it began
///    (issue #1118, AC3) rather than holding the record the check judged bad.
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
    gate_and_restore_against_with_readiness(world, text, current, false, false)
}

fn gate_and_restore_against_with_readiness(
    world: &mut World,
    text: &str,
    current: &Versions,
    allow_rebuild: bool,
    bootstrap_reconnect: bool,
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

    // A fresh reconnect candidate cannot mint its GameStart identities before
    // seeing the canonical record: those UUIDs are part of the by-UUID restore
    // contract. `import_artifact` above has already version- and
    // semantic-gated `boot_identity`, so it is safe to stage only that private
    // boot metadata now. The candidate remains technically paused, outside the
    // public roster and lockstep wait-set, and this record stays staged for the
    // ordinary restore retry after `OnEnter(InProgress)` finishes.
    //
    // This intentionally precedes the rollback checkpoint. Staging boot UUIDs
    // and requesting the candidate-private GameStart transition is bootstrap,
    // not a partial authoritative restore; a later corrupt-record rollback must
    // return to the now-booted private world so a terminal refusal can be
    // reported without trying to undo a state transition.
    if bootstrap_reconnect && !world.contains_resource::<crate::server_app::GameStartEntityUuids>()
    {
        let Some(phase) = world
            .get_resource::<State<crate::core::messages::GamePhase>>()
            .map(|phase| phase.get().clone())
        else {
            return MeshRestoreOutcome::RefusedGate(
                "the reconnect candidate has no GamePhase bootstrap state".to_string(),
            );
        };
        if phase == crate::core::messages::GamePhase::InProgress {
            return MeshRestoreOutcome::RefusedGate(
                "the reconnect candidate entered GameStart before canonical identities were staged"
                    .to_string(),
            );
        }
        if !world.contains_resource::<NextState<crate::core::messages::GamePhase>>() {
            return MeshRestoreOutcome::RefusedGate(
                "the reconnect candidate cannot request its GameStart bootstrap".to_string(),
            );
        }
        let boot = snap
            .state
            .boot_identity
            .as_ref()
            .expect("import_artifact gates the current snapshot's boot identity");
        let Some(world_config) = world.get_resource::<crate::world::config::WorldConfig>() else {
            return MeshRestoreOutcome::NotReady;
        };
        if let Err(refusal) = snapshot::validate_boot_identity_for_world(boot, world_config) {
            return MeshRestoreOutcome::RefusedGate(refusal.to_string());
        }
        crate::server_app::stage_resume_game_start_entity_uuids(world, boot);
        world
            .resource_mut::<NextState<crate::core::messages::GamePhase>>()
            .set(crate::core::messages::GamePhase::InProgress);
        return MeshRestoreOutcome::NotReady;
    }

    // 2. Checkpoint the live world BEFORE anything below can mutate it, so an
    //    integrity or completeness failure can be rolled back to the TRUE pre-gate
    //    original (issue #1118, AC3). This is captured ahead of
    //    `reconcile_world_layers` deliberately: reconcile can spawn or despawn layer
    //    entities, so capturing after it would let a later rollback return to a
    //    post-reconcile intermediate rather than to where this host actually began.
    //    Capture is a pure read of the current world state — nothing below needs to
    //    have run first for it to be correct — so moving it here changes nothing on
    //    the common (folded-value) path where reconcile is a no-op, and closes the
    //    topology-divergence case where reconcile makes real changes yet returns
    //    `Ready`. The checkpoint is the same `snapshot::capture` a save takes and
    //    restores through the same walk, so rolling back reconciles the world —
    //    despawning anything reconcile or the leader's record spawned, rebuilding
    //    anything they despawned — exactly as a resume would. `pre_digest` is the
    //    fold of THIS captured state, so the rollback can verify it actually
    //    returned the world to where it started.
    let checkpoint = snapshot::capture(world);
    let pre_digest = crate::sim_digest::world_digest(world);

    // 3. Readiness. Reconcile the captured layer topology, then check the roster
    //    is far enough along to overwrite — exactly `drain_snapshot_restore`'s
    //    preconditions. A not-ready or failed-layer return simply discards the
    //    checkpoint above (a pure read); as before #1118's reorder, neither path
    //    rolls back reconcile's own partial layer work — a `Failed` return is a
    //    gate refusal, not a half-restore.
    match snapshot::reconcile_world_layers(world, &snap.state) {
        snapshot::LayerReconcileStatus::Ready => {}
        snapshot::LayerReconcileStatus::Waiting => return MeshRestoreOutcome::NotReady,
        snapshot::LayerReconcileStatus::Failed(path) => {
            return MeshRestoreOutcome::RefusedGate(format!(
                "the record requires world layer '{path}', which could not be reconstructed"
            ))
        }
    }
    if !snapshot::ready_to_restore(world, &snap.state)
        && !(allow_rebuild && snapshot::ready_to_rebuild(world, &snap.state))
    {
        return MeshRestoreOutcome::NotReady;
    }

    // Restore, then verify the fold. `restore` overwrites by uuid; the recorded
    // digest is recomputed BY the restored simulation, so a tampered or truncated
    // record cannot restore to it.
    let report = snapshot::restore(world, &snap.state);
    let restored = crate::sim_digest::world_digest(world);
    if restored == snap.digest && report.is_complete() {
        // The transfer's whole-payload checksum already covered this history,
        // and `import_artifact` parsed it through the canonical Run gate.  Install
        // it only after the snapshot itself proved its digest so a refused or
        // partial restore cannot overwrite the receiver's local record.
        if let Some(mut log) = world.get_resource_mut::<crate::command_admission::CommandLog>() {
            log.replace_from_transfer(run.commands.clone());
        }
        return MeshRestoreOutcome::Committed {
            tick: snap.tick,
            digest: snap.digest,
        };
    }

    // A refused restore must not be left half-applied. Roll back to the checkpoint
    // and report the refusal — the world is now exactly what it was.
    //
    // The `rollback.is_complete()` half of the guard below is a `debug_assert`, so
    // it fires only in test/debug builds, never in release — and its partial-
    // rollback case cannot even arise in a same-roster recovery: the checkpoint is
    // this host's OWN live world, so restoring it back is a by-uuid overwrite of
    // entities that already exist, always complete for the homogeneous fleet this
    // path recovers. It stays asserted to catch a heterogeneous-roster regression.
    let rollback = snapshot::restore(world, &checkpoint);
    debug_assert!(
        crate::sim_digest::world_digest(world) == pre_digest && rollback.is_complete(),
        "the recovery rollback did not return the world to its pre-restore fold"
    );
    if restored != snap.digest {
        MeshRestoreOutcome::RefusedIntegrity {
            recorded: snap.digest,
            restored,
        }
    } else {
        MeshRestoreOutcome::Incomplete {
            tick: snap.tick,
            gaps: report.gaps.len(),
        }
    }
}

/// Restore a staged record the moment the world is ready AND this host is armed
/// to accept it (issues #1117, #1118).
///
/// Exclusive, in `PreUpdate` after the mesh has drained its inbox, so a record
/// that completed this frame is restored before this frame's fixed steps run and
/// before the digest exchange samples. A [`MeshRestoreOutcome::NotReady`] leaves
/// the record staged and tries again next frame; anything else — committed or
/// refused — clears it, because a refused record is not retried against the same
/// unchanging world.
///
/// # The receiver arm (issue #1118, AC2)
///
/// Reassembling a record is not consent to install it. Before #1118 this committed
/// any completed record unconditionally, so any peer could overwrite another host's
/// whole world simply by sending it a snapshot. Now a staged record is committed
/// only when [`MeshRestoreArm`] says this host is a designated recovering host
/// **and** the record came from the leader that arm names. An unarmed host drops
/// the record ([`MeshRestoreOutcome::RefusedUnarmed`]); an armed host presented a
/// record from the wrong slot drops it too ([`MeshRestoreOutcome::RefusedWrongSender`]).
/// Both leave the world untouched — the drop happens before [`gate_and_restore`] is
/// even called.
pub fn drain_mesh_restore(world: &mut World) {
    let staged = world
        .get_resource::<MeshSnapshotReceiver>()
        .and_then(|r| r.staged.clone());
    let Some(text) = staged else {
        return;
    };
    let staged_from = world
        .get_resource::<MeshSnapshotReceiver>()
        .and_then(|r| r.staged_from);
    let (armed_from, allow_rebuild, bootstrap_reconnect) = world
        .get_resource::<MeshRestoreArm>()
        .map_or((None, false, false), |arm| {
            (
                arm.armed_from(),
                arm.allows_rebuild(),
                arm.bootstraps_reconnect(),
            )
        });

    let outcome = match armed_from {
        // Not a designated recovering host: a peer must never overwrite this
        // world by sending it a snapshot. Drop it, world untouched.
        None => MeshRestoreOutcome::RefusedUnarmed,
        // Armed, but from a slot other than the leader the plan named: a divergent
        // peer cannot inject its state by racing the leader's transfer.
        Some(leader) if staged_from != Some(leader) => MeshRestoreOutcome::RefusedWrongSender {
            armed_from: leader,
            from: staged_from,
        },
        // Armed, and from the leader: run the rollback-able gate and restore.
        Some(_leader) => {
            let current = snapshot::versions(&crate::content_ledger::frozen_or_live());
            gate_and_restore_against_with_readiness(
                world,
                &text,
                &current,
                allow_rebuild,
                bootstrap_reconnect,
            )
        }
    };

    if matches!(outcome, MeshRestoreOutcome::NotReady) {
        return;
    }
    if let Some(mut receiver) = world.get_resource_mut::<MeshSnapshotReceiver>() {
        receiver.staged = None;
        receiver.staged_from = None;
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
        // The receiver arm (issue #1118). `Timer` — it is transport/session
        // bookkeeping about whether this host is mid-recovery, not state of the
        // world: nothing it holds is folded, and every honest host derives it from
        // the same shared recovery plan.
        app.declare_state::<MeshRestoreArm>(StateClass::Timer, "fleet-snapshot-transfer-state");
    }
    // `.after(MeshSet)` so a record completed by this frame's inbox drain is
    // restored the same frame. No explicit `.before(advance_sim_tick)`: that
    // system lives in `FixedLast`, a different schedule, so an ordering edge to it
    // from `PreUpdate` resolves to zero systems and constrains nothing. It is not
    // needed either — Bevy runs `PreUpdate` before `RunFixedMainLoop` by
    // construction, so this restore already precedes this frame's fixed steps and
    // the `FixedLast` digest sample; the ordering holds across schedules, not
    // within one, and cannot be spelled as an edge.
    app.init_resource::<MeshSnapshotReceiver>()
        .init_resource::<MeshRestoreArm>()
        .add_systems(
            PreUpdate,
            drain_mesh_restore.after(crate::lockstep::MeshSet),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_record_stages_exact_game_start_ids_before_requesting_the_phase() {
        let entity_uuid =
            crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 17, 3).render();
        let mut source = World::new();
        source.insert_resource(crate::lobby::SelectedShipResource(
            "assets/entities/alliance_cruiser.toml".into(),
        ));
        source.insert_resource(crate::lockstep::FleetRoster::default());
        source.insert_resource(crate::server_app::GameStartEntityUuids(vec![
            crate::snapshot::GameStartEntityUuid {
                authored_index: 0,
                entity_uuid: entity_uuid.clone(),
            },
        ]));
        let run = capture_run(&source, "assets/worlds/default.toml");
        let boot = run
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.state.boot_identity.clone())
            .expect("the source carries a complete boot identity");
        let text = crate::snapshot::export_artifact(&run).unwrap();
        let current = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());

        let mut candidate = World::new();
        candidate.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
        candidate.insert_resource(NextState::<crate::core::messages::GamePhase>::default());
        let mut world_config = crate::world::config::WorldConfig::default();
        world_config
            .entities
            .push(crate::world::config::WorldEntity {
                spawn_on: crate::world::config::WorldEntitySpawnOn::GameStart,
                ..Default::default()
            });
        candidate.insert_resource(world_config);

        assert_eq!(
            gate_and_restore_against_with_readiness(&mut candidate, &text, &current, false, true,),
            MeshRestoreOutcome::NotReady,
        );
        assert!(matches!(
            candidate.resource::<NextState<crate::core::messages::GamePhase>>(),
            NextState::Pending(crate::core::messages::GamePhase::InProgress)
        ));
        assert!(!candidate.contains_resource::<crate::server_app::GameStartEntityUuids>());

        let mut expected = World::new();
        crate::server_app::stage_resume_game_start_entity_uuids(&mut expected, &boot);
        assert_eq!(
            candidate.get_resource::<crate::server_app::ResumeGameStartEntityUuids>(),
            expected.get_resource::<crate::server_app::ResumeGameStartEntityUuids>(),
            "the candidate must stage the canonical authored-index/UUID map exactly"
        );
        assert_eq!(
            boot.game_start_entity_uuids,
            vec![crate::snapshot::GameStartEntityUuid {
                authored_index: 0,
                entity_uuid,
            }]
        );
    }
}
