//! The Bevy adapter that drives divergence recovery (issue #1118).
//!
//! [`crate::lockstep::recovery_plan`] is the pure decision — who recovers, from
//! whom, and at which tick. This module is its adapter into a running host: it
//! reads the periodic digest exchange every frame, and when the shared plan says a
//! recovery is due it holds the fleet at the boundary tick, has the leader transfer
//! its canonical record, arms and restores the divergent host, and resumes the
//! fleet only after the restore has folded to the leader's state — then records a
//! diagnostic artifact of the whole event (AGENTS.md rule 10: the decision stays
//! Bevy-free and its adapter is this sibling).
//!
//! # The six acceptance criteria, and where each lives
//!
//! 1. **Detection** is #1116's, reused verbatim: [`crate::lockstep::MeshAgreement`]
//!    already names the divergent tick. [`crate::lockstep::recovery_plan::decide`]
//!    reads the same ledgers to find the earliest jointly-sampled disagreement.
//! 2. **Deterministic leader** — [`recovery_plan::elect`], the lowest slot in the
//!    strict-majority fold group. A divergent peer is never eligible and never
//!    overwrites the fleet: [`begin_recovery`] arms the receiver only on the
//!    designated recovering host, naming the elected leader, and
//!    [`crate::lockstep::snapshot_relay::drain_mesh_restore`] commits nothing that
//!    did not come from that leader.
//! 3. **Boundary** — [`RecoveryHold`] withholds every tick past the boundary (read
//!    by [`crate::lockstep::gate_lockstep_ticks`]); the leader captures at the
//!    boundary, transfers, and the divergent host resumes only on a restore that
//!    folds to the transferred record.
//! 4. **No reset** — the transfer is #1117's whole-payload record, so the recovered
//!    host resumes the same mission, ship topology, scenario memory and crew.
//! 5. **Diagnostic artifact** — [`RecoveryDiagnostic`], one per host per event,
//!    holding the command window, the per-slot digests, the leader choice and the
//!    result, retained in [`RecoveryLog`] and exportable as RON.
//! 6. **Clean failure** — no strict majority (a two-host split) is refused with a
//!    diagnostic and no transfer; a leader record that will not gate is rolled back
//!    (#1117's checkpoint) and recorded as [`RecoveryResult::NoValidRecord`],
//!    never a half-restore.
//!
//! # Determinism: the same decision on every host, the barrier for the rest
//!
//! Every recovery DECISION is a function of shared state (see
//! [`crate::lockstep::recovery_plan`]'s module docs): the divergence tick, the
//! leader, the boundary. The one thing that is NOT decided from a folded value is
//! *when a canonical host lifts its boundary* — it waits until it has observed the
//! recovering host's watermark advance past the boundary (a monotone liveness
//! signal, [`crate::lockstep::LockstepSession::observed`]). That is deliberately a
//! liveness gate and not a simulation decision: which ticks actually run is still
//! governed by the shared lockstep barrier, so a canonical host that lifts its hold
//! a frame early or late cannot make the fold diverge — it can only wait for the
//! peer it must wait for anyway. The recovering host itself lifts its hold on a
//! folded value: its restored world equals the leader's transferred fold, which is
//! the post-restore agreement AC3 requires.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command_admission::log::{CommandLog, HostSlot, LoggedCommand};
use crate::logging::LogCat;
use crate::sim_tick::SimTick;
use crate::world::server::BridgeWorldSource;

use super::recovery_plan::{self, DivergenceWindow, NoRecovery, RecoveryDecision, RecoveryRole};
use super::snapshot_relay::{
    send_snapshot, MeshRestoreArm, MeshRestoreOutcome, MeshSnapshotReceiver,
};
use super::{FleetLockstep, FleetRoster, MeshAgreement};

/// The diagnostic-artifact format revision (issue #1118).
///
/// Bumped when [`RecoveryDiagnostic`]'s shape changes, so a stored artifact from a
/// different build is refused by [`parse_recovery_artifact`] rather than
/// mis-parsed — the same discipline `headless::replay`'s `ARTIFACT_VERSION` keeps
/// for the replay artifact.
pub const RECOVERY_ARTIFACT_VERSION: u32 = 1;

/// How far past the boundary this host must withhold ticks while a recovery it has
/// not yet resolved is in flight (issue #1118).
///
/// Read by [`crate::lockstep::gate_lockstep_ticks`], written by [`drive_recovery`].
/// `None` means no hold. Kept a separate resource, rather than reaching into the
/// barrier, so the barrier's own decision stays exactly #1116's and this is a
/// clearly-separable addition beside it.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct RecoveryHold {
    /// Ticks strictly greater than this are withheld; `None` = run freely.
    pub withhold_beyond: Option<u64>,
}

/// This host's view of the recovery in flight, if any.
#[derive(Resource, Default)]
pub struct RecoveryState {
    active: Option<ActiveRecovery>,
}

impl RecoveryState {
    /// Whether a recovery (recoverable or failed) is currently tracked.
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The tick this host must not run past yet, or `None` if it is free.
    fn hold_boundary(&self) -> Option<u64> {
        match &self.active {
            Some(ActiveRecovery::InProgress {
                plan, resolved, ..
            }) if !*resolved => Some(plan.boundary_tick),
            _ => None,
        }
    }
}

/// What this host is doing about a divergence.
enum ActiveRecovery {
    /// A recoverable divergence: the shared plan, this host's role, and its
    /// progress through the boundary.
    InProgress {
        plan: recovery_plan::RecoveryPlan,
        role: RecoveryRole,
        /// The tick the leader captured and queued its record at, once it has —
        /// the same tick the recovering host restores to. `None` until sent.
        record_tick: Option<u64>,
        /// This host has done its part and lifted its boundary hold.
        resolved: bool,
    },
    /// A clean failure — no safe leader, or a leader record that would not gate.
    /// Terminal: recorded once (the tick and reason live in the [`RecoveryLog`]
    /// diagnostic), holds nothing, and is not re-decided, so a persistent
    /// unrecoverable split is reported without a re-attempt storm.
    Failed,
}

/// Every recovery event this host has recorded (issue #1118, AC5).
#[derive(Resource, Default)]
pub struct RecoveryLog {
    entries: Vec<RecoveryDiagnostic>,
}

impl RecoveryLog {
    /// The recorded diagnostics, oldest first.
    pub fn entries(&self) -> &[RecoveryDiagnostic] {
        &self.entries
    }

    /// The most recent diagnostic, if any.
    pub fn last(&self) -> Option<&RecoveryDiagnostic> {
        self.entries.last()
    }
}

/// The replay/diagnostic artifact a recovery leaves behind (issue #1118, AC5).
///
/// One per host per event: it records the command window the divergence fell in,
/// every fleet slot's fold at the divergence tick, the leader the fleet elected,
/// the recovery boundary, and how the event resolved. Serialisable to RON so it can
/// be exported and replayed, guarded by [`RECOVERY_ARTIFACT_VERSION`].
///
/// Not `Eq`: the command window carries `LoggedCommand`s whose payloads hold
/// floats, so equality is `PartialEq` only — enough for the tests, and a diagnostic
/// is never a map key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecoveryDiagnostic {
    /// The artifact format revision.
    pub version: u32,
    /// The slot that recorded this artifact.
    pub observer: HostSlot,
    /// The earliest checkpoint tick the fleet disagreed on.
    pub divergence_tick: u64,
    /// The last checkpoint the fleet agreed on before it, if any.
    pub last_agreed_tick: Option<u64>,
    /// The tick the fleet paused at for the transfer (0 for a no-leader failure,
    /// which sets no boundary).
    pub boundary_tick: u64,
    /// Every fleet slot's fold at the divergence tick, in slot order.
    pub digests: Vec<(HostSlot, u64)>,
    /// The leader the fleet elected, or `None` when none commanded a majority.
    pub leader: Option<HostSlot>,
    /// The canonical fold the leader's record carried at the divergence tick.
    pub canonical_digest: Option<u64>,
    /// The slots that diverged and were to restore, in slot order.
    pub recovering: Vec<HostSlot>,
    /// Every command that applied in the window between the last agreement and the
    /// divergence — the input a replay reads to explain the split.
    pub command_window: Vec<LoggedCommand>,
    /// How the event resolved, from this host's vantage.
    pub result: RecoveryResult,
}

/// How a recovery resolved, from the recording host's vantage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RecoveryResult {
    /// This host led: it transferred its canonical record at `record_tick`.
    Led { record_tick: u64 },
    /// This host was canonical but not the leader; it waited and resumed.
    Witnessed,
    /// This host was divergent and restored the leader's record, folding to it.
    Recovered { record_tick: u64, digest: u64 },
    /// No fold commanded a strict majority — the fleet refused to overwrite one
    /// half from the other. No record was transferred and the world is untouched.
    NoSafeLeader { largest_group: usize, fleet: usize },
    /// The leader's record was refused (a bad build, a corrupt payload, an
    /// incomplete restore). The world was rolled back to its checkpoint, never
    /// left half-restored, and the divergence stands, honestly reported.
    NoValidRecord { refusal: String },
}

/// Serialise a diagnostic to RON for export (issue #1118, AC5).
pub fn export_recovery_artifact(diagnostic: &RecoveryDiagnostic) -> Result<String, String> {
    ron::ser::to_string(diagnostic).map_err(|e| e.to_string())
}

/// Parse a diagnostic from RON, refusing one whose format revision this build does
/// not speak.
pub fn parse_recovery_artifact(text: &str) -> Result<RecoveryDiagnostic, String> {
    let diagnostic: RecoveryDiagnostic = ron::from_str(text).map_err(|e| e.to_string())?;
    if diagnostic.version != RECOVERY_ARTIFACT_VERSION {
        return Err(format!(
            "recovery artifact is version {}, this build speaks {RECOVERY_ARTIFACT_VERSION}",
            diagnostic.version
        ));
    }
    Ok(diagnostic)
}

/// Read the periodic digest exchange, drive the recovery it implies, and publish
/// the boundary hold (issue #1118).
///
/// Exclusive because the leader's transfer captures the whole world and the
/// recovering host's restore overwrites it. Runs in `PreUpdate` after
/// [`crate::lockstep::apply_mesh_inbox`] (so it reads the freshest peer digests and
/// watermarks) and before [`crate::lockstep::gate_lockstep_ticks`] (so the hold it
/// sets is honoured this same frame) and before
/// [`crate::lockstep::snapshot_relay::drain_mesh_restore`] (so an arm it sets takes
/// effect this frame).
pub fn drive_recovery(world: &mut World) {
    let Some(session) = world.get_resource::<FleetLockstep>() else {
        return;
    };
    if session.is_alone() {
        // A lone host has no fleet to diverge from; make sure no stale hold lingers.
        if let Some(mut hold) = world.get_resource_mut::<RecoveryHold>() {
            hold.withhold_beyond = None;
        }
        return;
    }
    let local = session.local();
    let delay = session.delay();
    let sim_tick = world.get_resource::<SimTick>().map_or(0, |t| t.0);

    // Begin a recovery if one is due and none is tracked.
    if world.resource::<RecoveryState>().active.is_none() {
        begin_recovery(world, local, delay);
    }

    // Advance whatever is now tracked.
    advance_recovery(world, local, delay, sim_tick);

    // Publish the hold for the barrier.
    let hold = world.resource::<RecoveryState>().hold_boundary();
    world.resource_mut::<RecoveryHold>().withhold_beyond = hold;
}

/// Ask the shared exchange whether a recovery is due, and if so, open it.
fn begin_recovery(world: &mut World, local: HostSlot, delay: u64) {
    let fleet: Vec<HostSlot> = world
        .resource::<FleetRoster>()
        .ships()
        .iter()
        .map(|ship| ship.host)
        .collect();
    let decision = {
        let agreement = world.resource::<MeshAgreement>();
        recovery_plan::decide(
            &fleet,
            local,
            &agreement.local,
            &agreement.peers,
            agreement.local.interval,
            delay,
        )
    };

    match decision {
        RecoveryDecision::Pending => {}
        RecoveryDecision::Recover(plan) => {
            let role = plan.role_of(local);
            if role == RecoveryRole::Recovering {
                // Arm to accept a record from the elected leader, and only that
                // leader (AC2). Clear any stale outcome so this host acts on THIS
                // recovery's restore.
                world.resource_mut::<MeshRestoreArm>().arm(plan.leader);
                if let Some(mut rx) = world.get_resource_mut::<MeshSnapshotReceiver>() {
                    rx.clear_outcome();
                }
            }
            let log = world.get_resource::<crate::logging::LogFilterConfig>().cloned();
            crate::pwarn!(
                log,
                LogCat::Admit,
                "host-mesh divergence recovery: tick {}, leader {}, recovering {:?}, \
                 boundary tick {} (this host is {:?})",
                plan.divergence_tick,
                plan.leader.slot_id(),
                plan.recovering,
                plan.boundary_tick,
                role,
            );
            world.resource_mut::<RecoveryState>().active = Some(ActiveRecovery::InProgress {
                plan,
                role,
                record_tick: None,
                resolved: false,
            });
        }
        RecoveryDecision::Unrecoverable { window, reason } => {
            let diagnostic = failure_diagnostic(world, local, &window, &reason);
            let log = world.get_resource::<crate::logging::LogFilterConfig>().cloned();
            crate::perror!(
                log,
                LogCat::Admit,
                "host-mesh divergence at tick {} cannot be recovered: {reason}. A \
                 diagnostic artifact was recorded; the world was not touched.",
                window.tick,
            );
            world.resource_mut::<RecoveryLog>().entries.push(diagnostic);
            world.resource_mut::<RecoveryState>().active = Some(ActiveRecovery::Failed);
        }
    }
}

/// Advance the tracked recovery: do this host's role work and, when it is done,
/// record the artifact and clear (or fail) the tracking.
fn advance_recovery(world: &mut World, local: HostSlot, delay: u64, sim_tick: u64) {
    let Some(active) = world.resource_mut::<RecoveryState>().active.take() else {
        return;
    };
    let ActiveRecovery::InProgress {
        plan,
        role,
        mut record_tick,
        mut resolved,
    } = active
    else {
        // A terminal failure stays put, holding nothing.
        world.resource_mut::<RecoveryState>().active = Some(active);
        return;
    };

    let boundary = plan.boundary_tick;
    let reached = sim_tick > boundary;

    // The result this host will record when it resolves, if it has resolved.
    let mut resolution: Option<RecoveryResult> = None;

    match role {
        RecoveryRole::Leader => {
            if reached && record_tick.is_none() {
                record_tick = try_send_record(world, local, &plan);
            }
            if let Some(tick) = record_tick {
                if recovering_all_resumed(world, &plan, boundary, delay) {
                    resolved = true;
                    resolution = Some(RecoveryResult::Led { record_tick: tick });
                }
            }
        }
        RecoveryRole::Bystander => {
            if recovering_all_resumed(world, &plan, boundary, delay) {
                resolved = true;
                resolution = Some(RecoveryResult::Witnessed);
            }
        }
        RecoveryRole::Recovering => {
            match restore_result(world) {
                RestoreResolution::Pending => {}
                RestoreResolution::Recovered { tick, digest } => {
                    resolved = true;
                    resolution = Some(RecoveryResult::Recovered {
                        record_tick: tick,
                        digest,
                    });
                }
                RestoreResolution::Failed(refusal) => {
                    resolved = true;
                    resolution = Some(RecoveryResult::NoValidRecord { refusal });
                }
            }
        }
    }

    if let Some(result) = resolution {
        finish_recovery(world, local, &plan, boundary, result);
        return;
    }

    // Not resolved yet — keep tracking.
    world.resource_mut::<RecoveryState>().active = Some(ActiveRecovery::InProgress {
        plan,
        role,
        record_tick,
        resolved,
    });
}

/// Capture and queue this leader's canonical record. Returns the tick it was
/// captured at (the tick the recovering host restores to), or `None` if the
/// capture could not be framed.
fn try_send_record(
    world: &mut World,
    local: HostSlot,
    plan: &recovery_plan::RecoveryPlan,
) -> Option<u64> {
    let scenario = world
        .get_resource::<BridgeWorldSource>()
        .map(|source| source.path.clone())
        .unwrap_or_default();
    let captured_at = world.get_resource::<SimTick>().map_or(0, |t| t.0);
    let transfer_id = recovery_transfer_id(plan);
    match send_snapshot(world, local, transfer_id, scenario) {
        Ok(chunks) => {
            let log = world.get_resource::<crate::logging::LogFilterConfig>().cloned();
            crate::pinfo!(
                log,
                LogCat::Admit,
                "host-mesh recovery leader {} transferred its canonical record in {chunks} \
                 chunk(s) at tick {captured_at}",
                local.slot_id(),
            );
            Some(captured_at)
        }
        Err(why) => {
            let log = world.get_resource::<crate::logging::LogFilterConfig>().cloned();
            crate::perror!(
                log,
                LogCat::Admit,
                "host-mesh recovery leader {} could not capture its record: {why}",
                local.slot_id(),
            );
            None
        }
    }
}

/// A recovering host's read of its restore outcome.
enum RestoreResolution {
    /// No terminal outcome yet — the record has not arrived or is not ready.
    Pending,
    /// The record gated, restored and folded to itself.
    Recovered { tick: u64, digest: u64 },
    /// The record was refused; the world was rolled back, not left half-restored.
    Failed(String),
}

/// Classify the receiver's latest restore outcome for the recovering host.
fn restore_result(world: &World) -> RestoreResolution {
    let Some(outcome) = world
        .get_resource::<MeshSnapshotReceiver>()
        .and_then(|rx| rx.last_outcome().cloned())
    else {
        return RestoreResolution::Pending;
    };
    match outcome {
        MeshRestoreOutcome::Committed { tick, digest } => {
            RestoreResolution::Recovered { tick, digest }
        }
        MeshRestoreOutcome::NotReady => RestoreResolution::Pending,
        MeshRestoreOutcome::RefusedGate(why) => RestoreResolution::Failed(why),
        MeshRestoreOutcome::RefusedIntegrity { recorded, restored } => {
            RestoreResolution::Failed(format!(
                "the restored world folds to {restored:#018x}, not the {recorded:#018x} the \
                 record recorded"
            ))
        }
        MeshRestoreOutcome::Incomplete { tick, gaps } => RestoreResolution::Failed(format!(
            "the restore left {gaps} gap(s) at tick {tick}"
        )),
        MeshRestoreOutcome::RefusedWrongSender { armed_from, from } => {
            RestoreResolution::Failed(format!(
                "the record came from {from:?}, not the leader {}",
                armed_from.slot_id()
            ))
        }
        MeshRestoreOutcome::RefusedUnarmed => {
            RestoreResolution::Failed("this host was not armed to restore".to_string())
        }
    }
}

/// Record this host's diagnostic, forget the healed history, disarm, and clear (or
/// on a bad record, become terminal so it does not re-arm on the same split).
fn finish_recovery(
    world: &mut World,
    local: HostSlot,
    plan: &recovery_plan::RecoveryPlan,
    boundary: u64,
    result: RecoveryResult,
) {
    let clean_failure = matches!(result, RecoveryResult::NoValidRecord { .. });
    let diagnostic = plan_diagnostic(world, local, plan, result);
    world.resource_mut::<RecoveryLog>().entries.push(diagnostic);
    world.resource_mut::<MeshRestoreArm>().disarm();

    if clean_failure {
        // The divergence stands; leave the ledger so it is not silently forgotten,
        // and become terminal so this host does not keep re-arming for a record
        // that will not gate.
        world.resource_mut::<RecoveryState>().active = Some(ActiveRecovery::Failed);
        return;
    }

    // A resolved success: forget the divergent samples the recovery healed so the
    // same stale split cannot re-trigger, and clear the tracking.
    world.resource_mut::<MeshAgreement>().forget_through(boundary);
    world.resource_mut::<RecoveryState>().active = None;

    let log = world.get_resource::<crate::logging::LogFilterConfig>().cloned();
    crate::pinfo!(
        log,
        LogCat::Admit,
        "host-mesh recovery for the tick-{} divergence resolved on {}; the fleet resumes \
         past boundary tick {boundary}",
        plan.divergence_tick,
        local.slot_id(),
    );
}

/// Whether every recovering host has resumed past the boundary — its post-restore
/// watermark has advanced beyond the value a host paused at the boundary holds.
///
/// A monotone liveness signal, not a fold: a host that has completed the boundary
/// declares `boundary + delay`, and the first tick it runs after restoring pushes
/// that to `boundary + delay + 1`. The barrier, not this, decides which ticks run.
fn recovering_all_resumed(
    world: &World,
    plan: &recovery_plan::RecoveryPlan,
    boundary: u64,
    delay: u64,
) -> bool {
    let Some(session) = world.get_resource::<FleetLockstep>() else {
        return false;
    };
    let threshold = boundary.saturating_add(delay);
    plan.recovering
        .iter()
        .all(|slot| session.observed(*slot).is_some_and(|watermark| watermark > threshold))
}

/// A stable transfer id for a recovery, so a re-sent chunk is never mistaken for a
/// different transfer.
fn recovery_transfer_id(plan: &recovery_plan::RecoveryPlan) -> u64 {
    // The boundary tick in the high bits, the divergence tick in the low: unique
    // per recovery event and identical on every host that computes the same plan.
    (plan.boundary_tick << 32) ^ plan.divergence_tick
}

/// Build the diagnostic for a recovery that ran (recovered, led, witnessed, or a
/// bad-record failure).
fn plan_diagnostic(
    world: &World,
    observer: HostSlot,
    plan: &recovery_plan::RecoveryPlan,
    result: RecoveryResult,
) -> RecoveryDiagnostic {
    RecoveryDiagnostic {
        version: RECOVERY_ARTIFACT_VERSION,
        observer,
        divergence_tick: plan.divergence_tick,
        last_agreed_tick: plan.last_agreed_tick,
        boundary_tick: plan.boundary_tick,
        digests: plan.digests.iter().map(|(&slot, &d)| (slot, d)).collect(),
        leader: Some(plan.leader),
        canonical_digest: Some(plan.canonical_digest),
        recovering: plan.recovering.clone(),
        command_window: command_window(world, plan.last_agreed_tick, plan.divergence_tick),
        result,
    }
}

/// Build the diagnostic for a divergence with no safe leader.
fn failure_diagnostic(
    world: &World,
    observer: HostSlot,
    window: &DivergenceWindow,
    reason: &NoRecovery,
) -> RecoveryDiagnostic {
    let NoRecovery::NoSafeLeader {
        largest_group,
        fleet,
    } = reason;
    RecoveryDiagnostic {
        version: RECOVERY_ARTIFACT_VERSION,
        observer,
        divergence_tick: window.tick,
        last_agreed_tick: window.last_agreed,
        boundary_tick: 0,
        digests: window.digests.iter().map(|(&slot, &d)| (slot, d)).collect(),
        leader: None,
        canonical_digest: None,
        recovering: Vec::new(),
        command_window: command_window(world, window.last_agreed, window.tick),
        result: RecoveryResult::NoSafeLeader {
            largest_group: *largest_group,
            fleet: *fleet,
        },
    }
}

/// The commands that applied in the window between the last agreement and the
/// divergence — the input a replay reads to explain the split.
fn command_window(
    world: &World,
    last_agreed: Option<u64>,
    divergence_tick: u64,
) -> Vec<LoggedCommand> {
    let Some(log) = world.get_resource::<CommandLog>() else {
        return Vec::new();
    };
    log.entries()
        .iter()
        .filter(|entry| {
            entry.tick <= divergence_tick && last_agreed.is_none_or(|edge| entry.tick > edge)
        })
        .cloned()
        .collect()
}

/// Install the recovery resources. The [`drive_recovery`] system is wired into the
/// mesh chain by [`crate::lockstep::register_lockstep`].
pub fn register_recovery(app: &mut App) {
    {
        use crate::authoritative::{DeclareState, StateClass};
        // The recovery bookkeeping. `Timer` — it is session state about whether a
        // recovery is in flight and how far past a boundary to withhold, never
        // folded, and every honest host derives it from the same shared plan.
        app.declare_state::<RecoveryState>(StateClass::Timer, "fleet-recovery-state")
            .declare_state::<RecoveryHold>(StateClass::Timer, "fleet-recovery-state")
            // The diagnostic log. `Derived` — a report ABOUT folds and the barrier's
            // decisions, never simulation state and never folded back in.
            .declare_state::<RecoveryLog>(StateClass::Derived, "fleet-recovery-state");
    }
    app.init_resource::<RecoveryState>()
        .init_resource::<RecoveryHold>()
        .init_resource::<RecoveryLog>();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The diagnostic artifact round-trips through RON and refuses a foreign
    /// version — the AC5 record is a real, replayable artifact, not a debug print.
    #[test]
    fn a_diagnostic_artifact_round_trips_and_guards_its_version() {
        let diagnostic = RecoveryDiagnostic {
            version: RECOVERY_ARTIFACT_VERSION,
            observer: HostSlot(2),
            divergence_tick: 240,
            last_agreed_tick: Some(180),
            boundary_tick: 360,
            digests: vec![(HostSlot(1), 0xAA), (HostSlot(2), 0xBB), (HostSlot(3), 0xAA)],
            leader: Some(HostSlot(1)),
            canonical_digest: Some(0xAA),
            recovering: vec![HostSlot(2)],
            command_window: Vec::new(),
            result: RecoveryResult::Recovered {
                record_tick: 361,
                digest: 0xAA,
            },
        };
        let text = export_recovery_artifact(&diagnostic).expect("serialises");
        assert_eq!(parse_recovery_artifact(&text).unwrap(), diagnostic);

        // A record from a future format revision is refused, not mis-read.
        let mut future = diagnostic;
        future.version = RECOVERY_ARTIFACT_VERSION + 1;
        let text = export_recovery_artifact(&future).expect("serialises");
        assert!(parse_recovery_artifact(&text).is_err());
    }

    /// The transfer id is a stable function of the shared plan, so every host that
    /// computes the same plan tags the transfer identically.
    #[test]
    fn the_transfer_id_is_a_function_of_the_shared_plan() {
        use std::collections::BTreeMap;
        let plan = recovery_plan::RecoveryPlan {
            divergence_tick: 240,
            last_agreed_tick: Some(180),
            boundary_tick: 360,
            canonical_digest: 0xAA,
            leader: HostSlot(1),
            recovering: vec![HostSlot(3)],
            digests: BTreeMap::new(),
        };
        let id = recovery_transfer_id(&plan);
        assert_eq!(id, recovery_transfer_id(&plan.clone()), "same plan, same id");
        let mut other = plan;
        other.divergence_tick = 250;
        assert_ne!(
            id,
            recovery_transfer_id(&other),
            "a different divergence is a different transfer"
        );
    }
}
