//! Single-simulation-peer live restore (issue #1446, PRD #1420 stories 8–11,
//! 13, 15, 16).
//!
//! An equal Game Master picks a candidate out of the shared preflight picker
//! (#1445) and asks for the live session to be rewound onto it. This module is
//! the whole of that operation, and it is deliberately assembled out of
//! machinery that already exists rather than a second recovery protocol:
//!
//! * the REQUEST is an ordinary typed, attributed
//!   [`crate::gm_action::GmAction::RequestLiveRestore`] on the canonical
//!   journal, so "who asked", "in what order" and "which one won" are answered
//!   by the same ordered lane every other GM decision uses;
//! * the RECOVERY CHECKPOINT is an ordinary named manual save
//!   ([`crate::save_slots_store::SaveSlotService::persist`]) confirmed by
//!   [`crate::gm_checkpoint::confirmed_checkpoint`] — the #1445 read-back rule,
//!   unchanged: no readable row, no checkpoint;
//! * the LOAD, the digest check and the rollback are #1118's
//!   [`crate::lockstep::snapshot_relay::gate_and_restore_rebuilding`], which
//!   already gates before touching the world, captures a rollback checkpoint,
//!   verifies the restored fold against the recorded one and returns the world
//!   on failure — through #863's rebuilding readiness, because a rewind puts
//!   back rows a destruction or a removal took away and a held world will never
//!   bootstrap them for it;
//! * the FENCE is [`crate::gm_action::GmActionJournal::record_slot_recovery`],
//!   the same durable slot-recovery generation #1119 stamps, so an in-flight
//!   grant from before the restore is refused by the rule that already exists.
//!
//! # What is bounded, and why it is visible
//!
//! PRD #1420 ships restore in two paths. This is the first: exactly ONE
//! simulation peer. A session with more is refused BY NAME
//! ([`crate::gm_action::GmActionRefusalReason::MultipleSimulationPeers`]) rather
//! than half-attempted, because the readiness countdown, the nonresponder
//! disconnect and the all-peers digest agreement that a multi-peer restore
//! needs are #1447's and do not exist yet. Phone consoles are not simulation
//! peers — a peer is a [`crate::lockstep::FleetRoster`] PARTICIPANT — so a
//! GM-hosted session with any number of phones on it is squarely inside this
//! bound, which is exactly the demo topology.
//!
//! # Assignments are the live ones, deliberately
//!
//! A restore rewinds the WORLD, not the room. The people at the consoles have
//! not moved, so the candidate's own recorded seating is discarded and the live
//! seating is re-applied over the restored world
//! ([`crate::snapshot::restore_mesh_crew`] against a capture of the live world
//! taken moments before). This is a declared divergence from the candidate's
//! recorded fold, and it happens strictly AFTER the digest check has proved the
//! load itself was faithful: "the load reproduced the save" and "the room kept
//! its seats" are two different claims and are checked in that order.
//!
//! # Nothing here is authoritative across peers
//!
//! [`GmLiveRestore`] is peer-local orchestration. It is not folded into
//! [`crate::sim_digest`], not captured in a [`crate::snapshot::PhoenixSnapshot`]
//! and never crosses the wire. It cannot be: with one simulation peer there is
//! nobody to agree with, and #1447 replaces this driver with the multi-peer one
//! rather than promoting this resource.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::gm_action::{GmActionJournal, GmActionRefusalReason, SimulationPaused};
use crate::gm_checkpoint::{confirmed_checkpoint, preflight, CandidateBlock, LiveSeating};
use crate::lockstep::snapshot_relay::MeshRestoreOutcome;
use crate::save_slots::{ContentCheck, SaveSlotEntry};
use crate::save_slots_store::SaveSlotService;

/// The display name a recovery checkpoint is bookmarked under.
///
/// A String Table id rather than prose: the catalogue row is presentation, and
/// this crate has no locale to write a sentence in.
///
/// The row it names is durable and ordinary — it is listed by the GM checkpoint
/// picker, named in the restore preview, and shown in the landing page's Load
/// list like any other manual save — so every one of those surfaces resolves
/// the name through `slotName` (`gui/save-slots.js`), which is `wireText`: the
/// repo's render-site rule for a field that MAY be an id. Rename the id here
/// and the sentence follows from `assets/strings/strings.csv`; there is no
/// second copy of the name to keep in step.
pub const RECOVERY_CHECKPOINT_NAME: &str = "server.gm.restore.recovery_name";

/// How many frames a request may watch a world that will not accept it before
/// the attempt is reported as a failure and rolled back.
///
/// Not a designer's number and deliberately not authored: this is the last
/// resort behind [`crate::lockstep::snapshot_relay::gate_and_restore_rebuilding`],
/// which already converts the reachable "this candidate names a row this world
/// cannot produce" case into a reported [`GmRestoreFailure::LoadIncomplete`].
/// What is left is the unforeseen readiness answer — a world layer that will not
/// reconcile, say — and the only requirement on the bound is that a GM can never
/// be stranded in a phase whose Resume is refused by name. Two seconds of frames
/// is long enough that nothing legitimate is cut short (every step of this
/// driver is synchronous once the world is held) and short enough that the desk
/// gets a sentence rather than a spinner.
pub const READINESS_FRAME_BUDGET: u32 = 120;

/// Where a live restore has got to.
///
/// Every non-[`Idle`](Self::Idle) phase holds the session paused. That is the
/// point: a restore is not a thing that happens to a running world, and
/// success is not a resume (PRD #1420 story 13 — the GM resumes explicitly).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmRestorePhase {
    /// No restore is in flight and none has been reported since the last resume.
    #[default]
    Idle,
    /// The canonical journal accepted one request. The world is held; nothing
    /// has been captured or loaded yet.
    Accepted,
    /// A recovery checkpoint has been asked for and not yet confirmed.
    CapturingRecovery,
    /// The recovery checkpoint is confirmed; the candidate is being loaded.
    Loading,
    /// The candidate world is live, its fold matched, and the session is held
    /// for an explicit GM resume.
    Restored,
    /// The candidate was refused or failed and the world is back at the
    /// recovery checkpoint. Held.
    RolledBack,
    /// The candidate failed AND the rollback failed. Held, and honestly said
    /// so: this is the one state where the world is neither the old session nor
    /// the new one.
    Failed,
}

impl GmRestorePhase {
    /// Whether a further request must be refused as concurrent.
    pub fn in_flight(self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::CapturingRecovery | Self::Loading
        )
    }

    /// Whether this phase holds the session against a resume that is not the
    /// GM's own deliberate one.
    pub fn holds_session(self) -> bool {
        self.in_flight()
    }

    /// Whether this phase leaves the world stopped at all.
    ///
    /// Every non-[`Idle`](Self::Idle) phase does, in flight or reported: a
    /// restore is never a resume, so the hold outlives the outcome until the GM
    /// lifts it. This is the fact [`crate::gm_action::submit_local`] passes to
    /// the sequencer, which must schedule that Resume at the stopped boundary
    /// rather than one tick past it — the restored journal is the candidate's
    /// own and carries no canonical record of this hold.
    pub fn holds_world(self) -> bool {
        self != Self::Idle
    }

    /// The wire spelling, which is also the `data-phase` the page draws with.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Accepted => "accepted",
            Self::CapturingRecovery => "capturing-recovery",
            Self::Loading => "loading",
            Self::Restored => "restored",
            Self::RolledBack => "rolled-back",
            Self::Failed => "failed",
        }
    }
}

/// Why a live restore did not put the candidate world on screen.
///
/// Append-only, and each variant carries the concrete values its sentence
/// needs. A restore that failed is not "a restore that failed": a GM has to
/// know whether the recovery capture never landed, the candidate is no longer
/// eligible, the file would not load, or the world it loaded did not reproduce.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum GmRestoreFailure {
    /// The recovery checkpoint could not be taken, so nothing was loaded. The
    /// world is untouched apart from the hold the request itself placed.
    RecoveryCaptureFailed { detail: String },
    /// Revalidation at execution refused the candidate. Every applicable block
    /// is carried, exactly as the picker's advisory answer carries them.
    CandidateIneligible { blocks: Vec<CandidateBlock> },
    /// The candidate row could not be read back out of this peer's catalogue.
    CandidateUnreadable { detail: String },
    /// The version/content gate refused the candidate at execution.
    LoadRefused { detail: String },
    /// The candidate loaded but did not fold to the digest it recorded.
    DigestMismatch { recorded: u64, restored: u64 },
    /// The candidate loaded but the world could not house every captured row.
    LoadIncomplete { gaps: u32 },
    /// The load succeeded and the protocol fence could not be stamped, so
    /// pre-restore work would still be admissible. Rolled back rather than
    /// left unfenced.
    FenceFailed { detail: String },
    /// The candidate failed and the recovery checkpoint would not come back.
    /// The world is in neither state and this build says so.
    RollbackFailed { detail: String },
}

impl GmRestoreFailure {
    /// The String Table id whose sentence explains this failure.
    pub fn label_id(&self) -> &'static str {
        match self {
            Self::RecoveryCaptureFailed { .. } => "server.gm.restore.failed.recovery_capture",
            Self::CandidateIneligible { .. } => "server.gm.restore.failed.ineligible",
            Self::CandidateUnreadable { .. } => "server.gm.restore.failed.unreadable",
            Self::LoadRefused { .. } => "server.gm.restore.failed.load_refused",
            Self::DigestMismatch { .. } => "server.gm.restore.failed.digest",
            Self::LoadIncomplete { .. } => "server.gm.restore.failed.incomplete",
            Self::FenceFailed { .. } => "server.gm.restore.failed.fence",
            Self::RollbackFailed { .. } => "server.gm.restore.failed.rollback",
        }
    }
}

/// The attributed request the canonical journal accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedRestore {
    /// The crew-public GM operator id. Never a session token.
    pub operator_id: String,
    /// The requesting page's own correlation, so its feedback lifecycle settles.
    pub correlation: String,
    /// The peer-local catalogue row asked for.
    pub candidate_slot: String,
    /// The tick the request was applied at.
    pub requested_tick: u64,
}

/// One peer's live-restore orchestration.
///
/// Peer-local: never digest-folded, never captured, never on the wire (see the
/// module note). `context` is mirrored each frame by [`publish_restore_context`]
/// so the canonical reducer — which is at Bevy's parameter ceiling and cannot
/// take four more resources — can still make the two refusals that must be
/// canonical, in the ordered lane, at the apply tick.
#[derive(Resource, Clone, Debug, Default)]
pub struct GmLiveRestore {
    phase: GmRestorePhase,
    request: Option<AcceptedRestore>,
    recovery_slot: Option<String>,
    failure: Option<GmRestoreFailure>,
    restored_tick: Option<u64>,
    restored_digest: Option<u64>,
    fence_generation: Option<u64>,
    /// How many frames this request has spent watching a world that is not yet
    /// ready to be overwritten. Bounded by [`READINESS_FRAME_BUDGET`].
    not_ready_frames: u32,
    /// The tick this phase was entered at, for the health banner's own
    /// first-seen field.
    changed_tick: u64,
    context: RestoreContext,
}

/// The mirrored facts the reducer needs and cannot reach for itself.
#[derive(Clone, Debug, Default)]
struct RestoreContext {
    simulation_peers: usize,
    live: Option<LiveSeating>,
}

impl GmLiveRestore {
    pub fn phase(&self) -> GmRestorePhase {
        self.phase
    }

    pub fn request(&self) -> Option<&AcceptedRestore> {
        self.request.as_ref()
    }

    pub fn failure(&self) -> Option<&GmRestoreFailure> {
        self.failure.as_ref()
    }

    /// The confirmed recovery checkpoint this restore may be rolled back to.
    pub fn recovery_slot(&self) -> Option<&str> {
        self.recovery_slot.as_deref()
    }

    pub fn restored_tick(&self) -> Option<u64> {
        self.restored_tick
    }

    pub fn restored_digest(&self) -> Option<u64> {
        self.restored_digest
    }

    /// The fresh live protocol generation stamped on the local slot, which is
    /// what makes pre-restore in-flight work stale.
    pub fn fence_generation(&self) -> Option<u64> {
        self.fence_generation
    }

    pub fn changed_tick(&self) -> u64 {
        self.changed_tick
    }

    /// How many technical simulation participants this peer last observed.
    pub fn simulation_peers(&self) -> usize {
        self.context.simulation_peers
    }

    /// This session's live seating, as last mirrored.
    pub fn live_seating(&self) -> Option<&LiveSeating> {
        self.context.live.as_ref()
    }

    /// The canonical admission the reducer performs at the apply tick.
    ///
    /// Only the two answers that MUST be ordered live here. Candidate
    /// eligibility is deliberately not one of them: PRD #1420 requires the
    /// candidate to be revalidated against current content and current
    /// assignments AT EXECUTION, which is later than this and needs the
    /// catalogue.
    pub fn admit_request(&self) -> Result<(), GmActionRefusalReason> {
        if self.context.simulation_peers > 1 {
            return Err(GmActionRefusalReason::MultipleSimulationPeers);
        }
        if self.phase.in_flight() {
            return Err(GmActionRefusalReason::LiveRestoreInProgress);
        }
        Ok(())
    }

    /// Accept one canonical request. Called only from the ordered reducer.
    pub fn accept(&mut self, request: AcceptedRestore) {
        self.changed_tick = request.requested_tick;
        self.request = Some(request);
        self.recovery_slot = None;
        self.failure = None;
        self.restored_tick = None;
        self.restored_digest = None;
        self.fence_generation = None;
        self.not_ready_frames = 0;
        self.phase = GmRestorePhase::Accepted;
    }

    /// Count one frame in which the world refused to be overwritten yet, and
    /// say whether this request has now waited longer than a held world can
    /// justify ([`READINESS_FRAME_BUDGET`]).
    fn note_not_ready(&mut self) -> bool {
        self.not_ready_frames = self.not_ready_frames.saturating_add(1);
        self.not_ready_frames > READINESS_FRAME_BUDGET
    }

    fn settle(&mut self, phase: GmRestorePhase, failure: Option<GmRestoreFailure>, tick: u64) {
        self.phase = phase;
        self.failure = failure;
        self.changed_tick = tick;
    }

    /// Clear a reported restore once the GM has explicitly resumed.
    fn clear(&mut self, tick: u64) {
        self.phase = GmRestorePhase::Idle;
        self.request = None;
        self.failure = None;
        self.changed_tick = tick;
    }
}

/// Decide whether one catalogue row may be restored onto this session RIGHT NOW.
///
/// This is the authoritative revalidation PRD #1420 asks for at execution, and
/// it is deliberately the SAME pure function the picker showed the GM
/// ([`crate::gm_checkpoint::preflight`]) rather than a second compatibility
/// rule: a candidate that stopped being eligible between the preview and the
/// press is refused with the concrete blocks, in the vocabulary the GM was
/// already reading.
pub fn revalidate(
    live: &LiveSeating,
    candidate: Option<&SaveSlotEntry>,
) -> Result<(), GmRestoreFailure> {
    let Some(entry) = candidate else {
        return Err(GmRestoreFailure::CandidateIneligible {
            blocks: vec![CandidateBlock::Unreadable],
        });
    };
    let answer = preflight(live, entry);
    if answer.eligible {
        Ok(())
    } else {
        Err(GmRestoreFailure::CandidateIneligible {
            blocks: answer.blocks,
        })
    }
}

/// Mirror the two facts the canonical reducer cannot reach for itself.
///
/// A plain read of local state, published for a local decision: the roster is
/// the replicated one, the scenario is the same [`crate::save_slots_lifecycle::SaveScenario`]
/// every capture stamps, and the hull resolves the roster's per-host
/// placeholder exactly as #1445's picker resolves it.
pub fn publish_restore_context(
    restore: Option<ResMut<GmLiveRestore>>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    scenario: Option<Res<crate::save_slots_lifecycle::SaveScenario>>,
    selected: Option<Res<crate::lobby::SelectedShipResource>>,
) {
    let Some(mut restore) = restore else {
        return;
    };
    let peers = roster
        .as_ref()
        .map_or(1, |roster| roster.participants().len().max(1));
    let live = match (roster.as_ref(), scenario.as_ref()) {
        (Some(roster), Some(scenario)) if !scenario.0.is_empty() => Some(LiveSeating::from_roster(
            scenario.0.clone(),
            roster,
            selected.as_ref().map(|selected| selected.0.as_str()),
        )),
        _ => None,
    };
    let context = &mut restore.bypass_change_detection().context;
    context.simulation_peers = peers;
    context.live = live;
}

/// Advance an accepted live restore by exactly one step per frame.
///
/// Exclusive because every step needs the whole world: a capture, a catalogue
/// read, a whole-world overwrite. One step per frame is deliberate — the
/// recovery capture is scheduled for the next FIXED tick by the ordinary save
/// lifecycle, so this must be able to wait for it rather than assume it.
pub fn drive_live_restore(world: &mut World) {
    let Some(phase) = world
        .get_resource::<GmLiveRestore>()
        .map(|restore| restore.phase)
    else {
        return;
    };
    match phase {
        GmRestorePhase::Idle => {}
        GmRestorePhase::Accepted => begin_recovery_capture(world),
        GmRestorePhase::CapturingRecovery => await_recovery_capture(world),
        GmRestorePhase::Loading => execute_restore(world),
        GmRestorePhase::Restored | GmRestorePhase::RolledBack | GmRestorePhase::Failed => {
            release_on_explicit_resume(world)
        }
    }
}

fn tick_of(world: &World) -> u64 {
    world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0)
}

/// Settle on a failure, held.
///
/// The hold is UNCONDITIONAL because [`GmRestorePhase::holds_world`] promises
/// that every non-[`Idle`](GmRestorePhase::Idle) phase leaves the world
/// stopped, and only this can keep that promise on a path that failed AFTER the
/// candidate was committed: `snapshot::restore` re-installs the CANDIDATE's own
/// `SimulationPaused`, which for an ordinary bookmark taken while the session
/// was running is `false`. A settle that left it there would hand the very next
/// frame to [`release_on_explicit_resume`], which would read the running world
/// as the GM's own resume, clear the phase back to
/// [`Idle`](GmRestorePhase::Idle) and spend ticks on an unfenced timeline —
/// with the banner's "Do not resume" sentence never seen. On the refusal paths
/// that never unpaused, it is a no-op.
fn fail(world: &mut World, phase: GmRestorePhase, failure: GmRestoreFailure) {
    let tick = tick_of(world);
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.settle(phase, Some(failure), tick);
    }
    hold_session(world);
}

/// Take the recovery checkpoint, through the ORDINARY save write.
///
/// Deliberately synchronous rather than through the fixed-tick scheduler: the
/// request has already HELD the world, and a hold stops `FixedUpdate` — so a
/// capture scheduled for the next fixed tick would never arrive and the restore
/// would wait forever for its own precondition. A held world is a stable
/// boundary by definition, which is exactly what the fixed-tick rule exists to
/// guarantee, so this captures at that boundary and hands the run to the same
/// [`SaveSlotService::persist`] the scheduler uses: same manual routing, same
/// display name, same outcome FIFO, same catalogue row.
fn begin_recovery_capture(world: &mut World) {
    let Some(scenario) = world
        .get_resource::<crate::save_slots_lifecycle::SaveScenario>()
        .map(|scenario| scenario.0.clone())
        .filter(|path| !path.is_empty())
    else {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "this peer has no scenario to record a checkpoint against".to_string(),
            },
        );
    };
    if !world.contains_resource::<SaveSlotService>() {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "no local save Store is installed".to_string(),
            },
        );
    }
    let tick = tick_of(world);
    let slot_id = world
        .resource_mut::<SaveSlotService>()
        .reserve_manual(RECOVERY_CHECKPOINT_NAME);
    let run = crate::lockstep::snapshot_relay::capture_run(world, scenario);
    world.resource_mut::<SaveSlotService>().persist(
        crate::save_slots_lifecycle::PendingStoredRun {
            decision: crate::save_slots::CaptureDecision {
                tick,
                slot: crate::save_slots::CaptureSlot::Manual(slot_id.clone()),
                reason: crate::save_slots::CaptureReason::Manual,
            },
            run,
        },
    );
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.recovery_slot = Some(slot_id);
        restore.settle(GmRestorePhase::CapturingRecovery, None, tick);
    }
}

/// Confirm the recovery checkpoint before anything is loaded.
///
/// The #1445 rule, unchanged and deliberately kept as its own step: a write the
/// adapter reported as failed, and a write that reported success but left no
/// readable row, are both "no recovery checkpoint" — and either refuses here,
/// with the candidate world untouched.
fn await_recovery_capture(world: &mut World) {
    let Some(slot_id) = world
        .get_resource::<GmLiveRestore>()
        .and_then(|restore| restore.recovery_slot.clone())
    else {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "the recovery checkpoint was never requested".to_string(),
            },
        );
    };
    let reported_failure = world
        .get_resource::<SaveSlotService>()
        .and_then(|service| {
            service
                .outcomes()
                .find(|outcome| {
                    outcome.decision.slot == crate::save_slots::CaptureSlot::Manual(slot_id.clone())
                })
                .map(|outcome| outcome.result.clone())
        })
        .and_then(|result| result.err());
    if let Some(error) = reported_failure {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: format!("{error:?}"),
            },
        );
    }
    let Some(entries) = catalogue(world) else {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "this peer has no local save catalogue".to_string(),
            },
        );
    };
    if confirmed_checkpoint(&entries, &slot_id).is_none() {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "the capture reported success but no readable row came back".to_string(),
            },
        );
    }
    let tick = tick_of(world);
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.settle(GmRestorePhase::Loading, None, tick);
    }
}

fn catalogue(world: &World) -> Option<Vec<SaveSlotEntry>> {
    let service = world.get_resource::<SaveSlotService>()?;
    let current = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());
    service.list(&current, ContentCheck::Full).ok()
}

/// Revalidate, load, verify, retain the live seating and fence — or roll back.
fn execute_restore(world: &mut World) {
    let Some((candidate_slot, recovery_slot)) =
        world.get_resource::<GmLiveRestore>().and_then(|restore| {
            Some((
                restore.request.as_ref()?.candidate_slot.clone(),
                restore.recovery_slot.clone()?,
            ))
        })
    else {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::CandidateUnreadable {
                detail: "the accepted request is no longer readable".to_string(),
            },
        );
    };

    // Authoritative revalidation at execution, against the CURRENT catalogue
    // and the CURRENT seating — not the advisory answer the picker showed.
    let entries = catalogue(world).unwrap_or_default();
    let live = world
        .get_resource::<GmLiveRestore>()
        .and_then(|restore| restore.context.live.clone());
    let Some(live) = live else {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::CandidateIneligible {
                blocks: vec![CandidateBlock::NoFleetRecord],
            },
        );
    };
    if let Err(failure) = revalidate(
        &live,
        entries.iter().find(|entry| entry.slot_id == candidate_slot),
    ) {
        return fail(world, GmRestorePhase::RolledBack, failure);
    }

    let Some(text) = world
        .get_resource::<SaveSlotService>()
        .map(|service| service.export(&candidate_slot))
    else {
        return fail(
            world,
            GmRestorePhase::RolledBack,
            GmRestoreFailure::CandidateUnreadable {
                detail: "this peer has no local save catalogue".to_string(),
            },
        );
    };
    let text = match text {
        Ok(text) => text,
        Err(refusal) => {
            return fail(
                world,
                GmRestorePhase::RolledBack,
                GmRestoreFailure::CandidateUnreadable {
                    detail: refusal.to_string(),
                },
            )
        }
    };

    // The live seating, captured BEFORE the overwrite: the people at the
    // consoles have not moved, and this is what puts them back.
    let live_world = crate::snapshot::capture(world);

    // The REBUILDING gate, deliberately: this world is held, so it has stopped
    // becoming readier, and the rows the candidate names that are no longer
    // standing were destroyed or removed rather than not yet bootstrapped.
    let outcome = crate::lockstep::snapshot_relay::gate_and_restore_rebuilding(world, &text);
    let failure = match outcome {
        MeshRestoreOutcome::Committed { tick, digest } => {
            // Retain the LIVE assignments over the restored world. Strictly
            // after the fold check above, which is what proves the load itself
            // was faithful; this divergence is the deliberate one.
            crate::snapshot::restore_mesh_crew(world, &live_world);
            match fence_local_slot(world, tick) {
                Ok(generation) => {
                    let now = tick_of(world);
                    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
                        restore.restored_tick = Some(tick);
                        restore.restored_digest = Some(digest);
                        restore.fence_generation = Some(generation);
                        restore.settle(GmRestorePhase::Restored, None, now);
                    }
                    // A restore is never a resume (PRD #1420 story 13).
                    hold_session(world);
                    return;
                }
                Err(detail) => GmRestoreFailure::FenceFailed { detail },
            }
        }
        MeshRestoreOutcome::RefusedGate(detail) => GmRestoreFailure::LoadRefused { detail },
        MeshRestoreOutcome::RefusedIntegrity { recorded, restored } => {
            GmRestoreFailure::DigestMismatch { recorded, restored }
        }
        MeshRestoreOutcome::Incomplete { gaps, .. } => GmRestoreFailure::LoadIncomplete {
            gaps: u32::try_from(gaps).unwrap_or(u32::MAX),
        },
        MeshRestoreOutcome::NotReady => {
            // The world is not far enough along to be overwritten yet. Nothing
            // has been touched, so try again next frame — but a HELD world
            // spends no fixed ticks, so nothing it is waiting for can arrive on
            // its own. Bound the wait: a restore that cannot proceed is a
            // reported failure the GM can act on, never a phase whose own
            // Resume is refused by name for as long as the session lasts.
            let exhausted = world
                .get_resource_mut::<GmLiveRestore>()
                .is_some_and(|mut restore| restore.note_not_ready());
            if !exhausted {
                return;
            }
            GmRestoreFailure::LoadRefused {
                detail: "the world never became ready to accept this candidate".to_string(),
            }
        }
        other => GmRestoreFailure::LoadRefused {
            detail: format!("{other:?}"),
        },
    };

    roll_back(world, &recovery_slot, failure);
}

/// Return the world to the recovery checkpoint, through the same gate.
///
/// Deliberately the DURABLE row rather than an in-memory buffer, and
/// deliberately re-run even though [`gate_and_restore`] has already rolled its
/// own checkpoint back: a rollback nobody verified is not a rollback, and a
/// recovery checkpoint that cannot be read back is the persistent-failure case
/// PRD #1420 requires to stay honestly failed rather than be papered over.
fn roll_back(world: &mut World, recovery_slot: &str, failure: GmRestoreFailure) {
    let text = world
        .get_resource::<SaveSlotService>()
        .map(|service| service.export(recovery_slot));
    let text = match text {
        Some(Ok(text)) => text,
        Some(Err(refusal)) => {
            return fail(
                world,
                GmRestorePhase::Failed,
                GmRestoreFailure::RollbackFailed {
                    detail: format!("{failure:?}: {refusal}"),
                },
            )
        }
        None => {
            return fail(
                world,
                GmRestorePhase::Failed,
                GmRestoreFailure::RollbackFailed {
                    detail: format!("{failure:?}: this peer has no local save catalogue"),
                },
            )
        }
    };
    // The rebuilding gate here too: a committed candidate may have despawned
    // rows the recovery checkpoint holds, and a rollback that could not put
    // them back would report "the world is in neither state" for a world this
    // build can in fact rebuild.
    match crate::lockstep::snapshot_relay::gate_and_restore_rebuilding(world, &text) {
        // `fail` holds: the recovery checkpoint carries its own recorded
        // `SimulationPaused`, and a rollback is no more a resume than a
        // restore is.
        MeshRestoreOutcome::Committed { .. } => fail(world, GmRestorePhase::RolledBack, failure),
        other => fail(
            world,
            GmRestorePhase::Failed,
            GmRestoreFailure::RollbackFailed {
                detail: format!("{failure:?}: {other:?}"),
            },
        ),
    }
}

/// Stamp a fresh live protocol generation on this peer's own slot.
///
/// The restored journal is the CANDIDATE's, so a grant that was in flight
/// against the abandoned timeline carries a generation that no longer names the
/// current incarnation. This is #1119's durable slot-recovery machinery used
/// for exactly what it is for; no second fencing concept is introduced, and the
/// generation is not exposed anywhere as fictional history.
///
/// # The boundary is the restored tick itself
///
/// #1119 fences a rejoining slot at a FUTURE tick, because that session keeps
/// running and will reach it. This one never will: the restore holds the world
/// at `restored_tick`, and a held world does not spend ticks. A boundary at
/// `restored_tick + 1` would therefore be unreachable, and — because
/// [`crate::gm_action::sequence_owner_proposal`] forces every later grant to at
/// least its slot's current recovery boundary — it would strand the GM's own
/// explicit Resume, and every action after it, one tick past a stopped clock.
/// So the fence lands exactly where the world is standing. That is the same
/// boundary rule #1119 already documents: both adjacent incarnations are valid
/// ON the boundary tick, and from the next tick only the restored one is.
///
/// A boundary the candidate's own journal already recorded at or after
/// `restored_tick` would be an INERT repeat — [`GmActionJournal::record_slot_recovery`]
/// hands back the generation that is already there — which would leave
/// pre-restore work admissible while reporting a fence. That is refused here
/// (and rolled back by the caller) rather than reported, because an unfenced
/// restore is not a restore that succeeded.
fn fence_local_slot(world: &mut World, restored_tick: u64) -> Result<u64, String> {
    let slot = world
        .get_resource::<crate::lockstep::FleetRoster>()
        .map_or(crate::command_admission::log::HostSlot::SOLO, |roster| {
            roster.local()
        });
    if !world.contains_resource::<GmActionJournal>() {
        world.insert_resource(GmActionJournal::default());
    }
    let mut journal = world.resource_mut::<GmActionJournal>();
    let previous = journal.current_recovery_generation(slot);
    let generation = journal
        .record_slot_recovery(slot, restored_tick)
        .map_err(str::to_string)?;
    if generation <= previous {
        return Err(format!(
            "the candidate already recorded generation {generation} at tick {restored_tick}"
        ));
    }
    Ok(generation)
}

/// Hold the session. A restore leaves the world stopped, always.
fn hold_session(world: &mut World) {
    if let Some(mut paused) = world.get_resource_mut::<SimulationPaused>() {
        paused.0 = true;
    }
}

/// A reported restore stays on the banner until the GM explicitly resumes.
///
/// The resume is the ORDINARY [`crate::gm_action::GmAction::SetSessionPaused`]
/// every session control already uses — there is no second resume — and this
/// only observes it, so nothing here can resume a held world on its own.
fn release_on_explicit_resume(world: &mut World) {
    let running = world
        .get_resource::<SimulationPaused>()
        .is_some_and(|paused| !paused.0);
    if !running {
        return;
    }
    let tick = tick_of(world);
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.clear(tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_admission::log::HostSlot;
    use crate::core::messages::StationId;
    use crate::gm_checkpoint::SeatedShip;
    use crate::lockstep::{FleetRoster, FleetShip};

    fn seating() -> LiveSeating {
        LiveSeating {
            scenario: "assets/worlds/duel.toml".to_string(),
            ships: vec![SeatedShip {
                slot: 0,
                hull: Some("assets/entities/alliance_cruiser.toml".to_string()),
                stations: vec!["helm".to_string()],
            }],
        }
    }

    fn with_peers(peers: usize) -> GmLiveRestore {
        let mut restore = GmLiveRestore::default();
        restore.context.simulation_peers = peers;
        restore.context.live = Some(seating());
        restore
    }

    #[test]
    fn one_simulation_peer_is_admitted_and_more_are_refused_by_name() {
        assert_eq!(with_peers(1).admit_request(), Ok(()));
        assert_eq!(
            with_peers(2).admit_request(),
            Err(GmActionRefusalReason::MultipleSimulationPeers),
        );
    }

    /// The bounded delivery refuses the TOPOLOGY, never the desk. Phone
    /// consoles are legs on a peer, not participants, so a peer holding a whole
    /// crew of them is still one simulation peer.
    #[test]
    fn many_phone_consoles_on_one_peer_are_still_one_simulation_peer() {
        let roster = FleetRoster::new(
            vec![FleetShip {
                host: HostSlot::SOLO,
                ship_path: Some("assets/entities/alliance_cruiser.toml".to_string()),
                crew: vec![
                    (StationId("helm".to_string()), "commander".to_string()),
                    (StationId("tactical".to_string()), "commander".to_string()),
                    (
                        StationId("engineering".to_string()),
                        "commander".to_string(),
                    ),
                ],
            }],
            HostSlot::SOLO,
        );
        assert_eq!(roster.participants().len(), 1);
        let live = LiveSeating::from_roster("assets/worlds/duel.toml", &roster, None);
        assert_eq!(live.ships.len(), 1);
        assert_eq!(live.ships[0].stations.len(), 3);
    }

    #[test]
    fn a_second_request_while_one_is_in_flight_is_refused_as_concurrent() {
        let mut restore = with_peers(1);
        restore.accept(AcceptedRestore {
            operator_id: "gm-1".to_string(),
            correlation: "one".to_string(),
            candidate_slot: "slot-a".to_string(),
            requested_tick: 10,
        });
        assert_eq!(restore.phase(), GmRestorePhase::Accepted);
        assert_eq!(
            restore.admit_request(),
            Err(GmActionRefusalReason::LiveRestoreInProgress),
        );
        // The first request is the one that is still named: a refused
        // concurrent press must not overwrite the attribution of the accepted
        // one.
        assert_eq!(restore.request().unwrap().correlation, "one");
    }

    /// A reported outcome is not in flight. The world is held, and a GM may ask
    /// again — which is what makes a failed restore recoverable rather than a
    /// dead end.
    #[test]
    fn a_settled_restore_admits_a_fresh_request() {
        let mut restore = with_peers(1);
        restore.accept(AcceptedRestore {
            operator_id: "gm-1".to_string(),
            correlation: "one".to_string(),
            candidate_slot: "slot-a".to_string(),
            requested_tick: 10,
        });
        restore.settle(
            GmRestorePhase::RolledBack,
            Some(GmRestoreFailure::LoadRefused {
                detail: "moved".to_string(),
            }),
            12,
        );
        assert_eq!(restore.admit_request(), Ok(()));
        assert!(!GmRestorePhase::RolledBack.in_flight());
        assert!(!GmRestorePhase::Restored.in_flight());
    }

    #[test]
    fn revalidation_reports_the_blocks_the_picker_would_have_shown() {
        let entry = SaveSlotEntry {
            slot_id: "slot-a".to_string(),
            display_name: "Before the ambush".to_string(),
            kind: crate::save_slots::SaveSlotKind::Manual,
            record: None,
            start: crate::save_slots::StartState::Refused(crate::snapshot::LoadRefusal::Empty),
            metadata: crate::save_slots::MetadataStatus::Present,
        };
        let failure =
            revalidate(&seating(), Some(&entry)).expect_err("an empty row is no candidate");
        assert_eq!(
            failure,
            GmRestoreFailure::CandidateIneligible {
                blocks: vec![CandidateBlock::Unreadable],
            },
        );
        assert_eq!(failure.label_id(), "server.gm.restore.failed.ineligible");
    }

    #[test]
    fn an_absent_candidate_is_refused_rather_than_treated_as_eligible() {
        assert!(revalidate(&seating(), None).is_err());
    }

    /// Every phase except Idle holds the world, and only the three working ones
    /// refuse a concurrent request. A reported outcome must not hold the GM's
    /// own resume hostage.
    #[test]
    fn phase_wire_spellings_are_stable_and_distinct() {
        let phases = [
            GmRestorePhase::Idle,
            GmRestorePhase::Accepted,
            GmRestorePhase::CapturingRecovery,
            GmRestorePhase::Loading,
            GmRestorePhase::Restored,
            GmRestorePhase::RolledBack,
            GmRestorePhase::Failed,
        ];
        let wires: std::collections::BTreeSet<_> =
            phases.iter().map(|phase| phase.as_wire()).collect();
        assert_eq!(wires.len(), phases.len());
        assert!(GmRestorePhase::Loading.holds_session());
        assert!(!GmRestorePhase::Idle.holds_session());
        // A reported restore no longer refuses a resume — that IS the resume —
        // but the world it reported on is still stopped, and the sequencer has
        // to schedule that resume at the boundary the hold stopped.
        assert!(!GmRestorePhase::Restored.holds_session());
        for phase in phases {
            assert_eq!(phase.holds_world(), phase != GmRestorePhase::Idle);
        }
    }
}
