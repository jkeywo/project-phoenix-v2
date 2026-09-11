//! Live restore of a running session onto a saved checkpoint, across every
//! simulation peer (issues #1446 and #1447, PRD #1420 stories 8–16).
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
//! # Across peers (issue #1447)
//!
//! #1446 delivered the operation on one simulation peer and refused a session
//! with more BY NAME. #1447 removes that bound by extending the SAME operation
//! rather than building a distributed twin of it: every peer applies the same
//! canonical request off the same replicated journal, holds, takes its OWN
//! recovery checkpoint, and then plays one of two roles it reads off that
//! request.
//!
//! * The peer whose GM asked — the one whose private catalogue holds the
//!   candidate — waits on the room for [`PEER_WINDOW_SECONDS`] REAL seconds
//!   with a visible countdown, transfers the candidate as ordinary #1117 chunks
//!   over #1118's transport, loads its own copy, and decides.
//! * Every other peer answers once, arms #1118's receiver for that peer and no
//!   other, lets the ordinary
//!   [`crate::lockstep::snapshot_relay::drain_mesh_restore`] put the record in,
//!   and reports the fold it got.
//!
//! A peer that does not answer inside the window is DISCONNECTED through
//! canonical membership — #1119's host loss, the same one a closed socket
//! produces — and the remaining peers carry on; it returns through the existing
//! snapshot recovery, never through a hidden automatic resume. Success needs
//! every remaining peer to have loaded and reported the SAME fold; one reported
//! failure or mismatch rolls the whole room back to the recovery checkpoints it
//! took on the way in, held, and says so.
//!
//! Phone consoles are not simulation peers — a peer is a
//! [`crate::lockstep::FleetRoster`] PARTICIPANT — so a crew of phones on one
//! peer is one peer, and they neither answer nor are waited on.
//!
//! # What is NOT decided here
//!
//! Whether anything ELSE may be sequenced while a restore is running is decided
//! by the OWNER in [`crate::gm_action::sequence_owner_proposal`], before a grant
//! exists. It cannot be decided in the reducer: a restore's PROGRESS is
//! peer-local — one peer can have committed the candidate while another is still
//! loading it — and a reducer arm that read it would fold an outcome out of
//! state that legitimately differs between peers.
//!
//! Which of two CONCURRENT requests wins is decided in the reducer, because it
//! has to be: two GMs pressing at the same moment are deterministically
//! scheduled onto the same apply tick, which is behind the owner's gate. That
//! answer is read off the replicated journal
//! ([`crate::gm_action::GmActionJournal::live_restore_admitted`]) and never off
//! this module's state, so every peer folds the same Applied and the same
//! Refused.
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
//! and never crosses the wire. What DOES cross is [`GmRestoreFrame`]: three
//! sentences about liveness and agreement, carrying no world state and no
//! catalogue key. A restore that survived its own snapshot would re-arm itself
//! on resume, which is why this resource is captured nowhere.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use bevy::time::Real;
use serde::{Deserialize, Serialize};

use crate::command_admission::log::HostSlot;
use crate::gm_action::{GmActionJournal, GmActionOrder, SimulationPaused};
use crate::gm_checkpoint::{confirmed_checkpoint, preflight, CandidateBlock, LiveSeating};
use crate::lockstep::snapshot_relay::{MeshRestoreArm, MeshRestoreOutcome, MeshSnapshotReceiver};
use crate::lockstep::{FleetRoster, MeshFrame, MeshOutbox};
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

/// How long a live restore waits on a peer, in REAL seconds (issue #1447).
///
/// Real seconds, not ticks: the world is HELD from the moment the request is
/// accepted, so a tick-based bound would never expire and a peer that says
/// nothing would hold the room for as long as the session lasts. The clock this
/// is measured on is `Time<Real>`, which is the one clock a hold does not stop.
///
/// The same window bounds both waits, because they are the same question asked
/// twice — "is this peer still with us?" — and a facilitator counting down at
/// the desk should not have to learn two numbers. Ten seconds is long enough for
/// a peer to take a whole-world capture of its own and short enough that a room
/// waiting on a laptop somebody closed gets an answer inside one breath.
pub const PEER_WINDOW_SECONDS: f64 = 10.0;

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
    /// This peer's recovery checkpoint is confirmed and the peers that must
    /// take part in the rewind have not all answered yet (issue #1447). Only
    /// the initiating peer waits here; every other peer answers and moves on.
    AwaitingReadiness,
    /// The recovery checkpoint is confirmed; the candidate is being loaded —
    /// out of this peer's own catalogue on the initiating peer, out of the
    /// #1118 recovery transport on every other one.
    Loading,
    /// This peer has put the candidate in its world and reported the fold it
    /// got. Nothing is settled until every remaining peer has (issue #1447).
    AwaitingAgreement,
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
            Self::Accepted
                | Self::CapturingRecovery
                | Self::AwaitingReadiness
                | Self::Loading
                | Self::AwaitingAgreement
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
            Self::AwaitingReadiness => "awaiting-readiness",
            Self::Loading => "loading",
            Self::AwaitingAgreement => "awaiting-agreement",
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
    /// One or more remaining peers reported that they could not load the
    /// candidate, so the whole fleet rolls back (issue #1447).
    ///
    /// A COUNT, never a slot: which machine is which is fleet plumbing, and the
    /// health projection deliberately keeps technical slots off the desk.
    PeerLoadFailed { peers: u32 },
    /// One or more remaining peers loaded the candidate and folded to something
    /// else. Agreement is the whole point, so the fleet rolls back rather than
    /// resuming peers that are no longer running the same world (issue #1447).
    PeerDigestMismatch { peers: u32 },
    /// The candidate never finished arriving over the #1118 recovery transport
    /// within the bounded window (issue #1447).
    TransferIncomplete { detail: String },
    /// The initiating peer stopped speaking before it said whether the rewind
    /// was agreed, so this peer returns to its own recovery checkpoint rather
    /// than waiting on a decision that cannot arrive (issue #1447).
    CoordinatorLost,
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
            Self::PeerLoadFailed { .. } => "server.gm.restore.failed.peer_load",
            Self::PeerDigestMismatch { .. } => "server.gm.restore.failed.peer_digest",
            Self::TransferIncomplete { .. } => "server.gm.restore.failed.transfer",
            Self::CoordinatorLost => "server.gm.restore.failed.coordinator_lost",
        }
    }

    /// How many peers this failure is about, for the `{peers}` its sentence
    /// takes. `None` for every failure that is about this peer alone.
    ///
    /// The COUNT and never the slots: which machine is which is fleet plumbing,
    /// and the health projection keeps technical slots off the desk. The peer
    /// rows carry `restore_waiting`/`restore_excluded` for the naming.
    pub fn peers(&self) -> Option<u32> {
        match self {
            Self::PeerLoadFailed { peers } | Self::PeerDigestMismatch { peers } => Some(*peers),
            _ => None,
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
    /// The peer whose GM asked, and therefore the peer that holds the candidate
    /// in its own catalogue and coordinates the rewind (issue #1447). Every
    /// peer reads it off the same canonical grant, so no peer has to be told
    /// who is in charge.
    pub initiator: HostSlot,
    /// The canonical order of the accepted request, which is this restore's
    /// shared identity on the wire. Two restores can never share one, and a
    /// frame from a restore this peer is not running is dropped by it.
    pub order: GmActionOrder,
}

/// One peer's word in a multi-peer live restore (issue #1447).
///
/// This is the ONLY thing #1447 adds to the host mesh, and it deliberately
/// carries no world state: the candidate itself travels as ordinary #1117
/// snapshot chunks over #1118's recovery transport, and the decision to run the
/// rewind at all is the canonical [`crate::gm_action::GmAction::RequestLiveRestore`]
/// every peer already applies from the same replicated journal. What is left is
/// the three facts a rewind of a LIVE room cannot be done without: "I am ready
/// to be overwritten", "this is the world I ended up with", and "we are agreed".
///
/// Every frame names the restore it belongs to by the canonical order of the
/// accepted request, so a frame from a restore this peer is not running — a
/// late one from an abandoned attempt, say — is dropped rather than folded into
/// the current one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GmRestoreFrame {
    /// This peer has held its world, taken its OWN recovery checkpoint and
    /// armed its receiver: it may be overwritten.
    ///
    /// Only technical simulation participants send this. A phone console is a
    /// leg on a peer, not a peer, so it never speaks here and is never waited
    /// on (PRD #1420: phone consoles do not vote).
    Ready {
        from: HostSlot,
        restore: GmActionOrder,
    },
    /// This peer put the transferred candidate into its world, which folds to
    /// `digest` at `tick`.
    Loaded {
        from: HostSlot,
        restore: GmActionOrder,
        tick: u64,
        digest: u64,
    },
    /// This peer cannot take part, and says which of the bounded reasons it is
    /// rather than going quiet. A quiet peer is handled too — by the window —
    /// but a peer that KNOWS it failed must not cost the room ten seconds.
    Unable {
        from: HostSlot,
        restore: GmActionOrder,
        failure: GmRestoreFailure,
    },
    /// The initiating peer's terminal decision. The one asymmetry in the
    /// protocol, and the reason there is an initiating peer at all: somebody has
    /// to say whether every remaining peer agreed, and that answer has to be the
    /// same sentence everywhere.
    Settle {
        from: HostSlot,
        restore: GmActionOrder,
        commit: bool,
        failure: Option<GmRestoreFailure>,
    },
}

impl GmRestoreFrame {
    /// The slot that sent this frame.
    pub fn from(&self) -> HostSlot {
        match self {
            Self::Ready { from, .. }
            | Self::Loaded { from, .. }
            | Self::Unable { from, .. }
            | Self::Settle { from, .. } => *from,
        }
    }

    /// Which restore this frame belongs to.
    pub fn restore(&self) -> GmActionOrder {
        match self {
            Self::Ready { restore, .. }
            | Self::Loaded { restore, .. }
            | Self::Unable { restore, .. }
            | Self::Settle { restore, .. } => *restore,
        }
    }
}

/// Live-restore frames this peer has been handed and not folded in yet.
///
/// Peeled off the ordinary mesh drain into their own lane for the same reason
/// [`crate::gm_join::GmJoinInbox`] is: the world is HELD for the whole of a
/// restore, so these have to be processable while `FixedUpdate` is starved.
#[derive(Resource, Default, Debug)]
pub struct GmRestoreInbox(Vec<GmRestoreFrame>);

impl GmRestoreInbox {
    pub fn push(&mut self, frame: GmRestoreFrame) {
        self.0.push(frame);
    }

    pub fn drain(&mut self) -> Vec<GmRestoreFrame> {
        std::mem::take(&mut self.0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What one remaining peer reported about its own load.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PeerReport {
    /// The peer loaded the candidate and folded to this digest.
    Loaded(u64),
    /// The peer could not, and said why.
    Unable(GmRestoreFailure),
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
    /// Every peer that has said it is ready to be overwritten (issue #1447).
    /// Recorded whatever phase this peer is in, so a peer that answered while
    /// this one was still capturing is not waited on a second time.
    ready: BTreeSet<HostSlot>,
    /// What each remaining peer reported about its own load.
    reports: BTreeMap<HostSlot, PeerReport>,
    /// Peers disconnected for nonresponse, through canonical membership.
    excluded: BTreeSet<HostSlot>,
    /// The initiating peer's terminal decision, once it has arrived.
    settle: Option<(bool, Option<GmRestoreFailure>)>,
    /// The REAL-clock second the current wait expires at. `None` outside a wait.
    deadline: Option<f64>,
    /// This peer's own world as it stood immediately before the candidate
    /// overwrote it, kept only to put the LIVE seating back afterwards.
    live_world: Option<Box<crate::snapshot::PhoenixSnapshot>>,
    /// Whether this peer has already put the candidate on the wire, so a
    /// coordinator that spends more than one frame loading does not re-send it.
    transferred: bool,
    /// Whether a load actually WALKED this peer's world, as opposed to being
    /// refused in front of it.
    ///
    /// The difference decides whether there is anything to roll back. A gate
    /// refusal, a transfer that never completed and a candidate this session
    /// refused at revalidation all leave the world byte-identical, and running a
    /// whole-world restore over it would turn a clean refusal into a second
    /// chance to fail. A commit, a fold mismatch and an incomplete restore all
    /// touched it, and every one of those goes back through the DURABLE
    /// recovery row - because a rollback nobody verified is not a rollback.
    touched_world: bool,
    /// Answers that arrived before this peer had applied the canonical request
    /// they answer (issue #1447).
    ///
    /// Peers apply the same grant at the same TICK, but a tick is not a frame:
    /// two peers can legitimately sit a whole lockstep delay apart in wall
    /// time, and a follower emits its one and only `Ready` a couple of frames
    /// after IT applies. If the coordinating peer is the last to get there,
    /// every answer that already arrived would otherwise be dropped on the
    /// floor - and a `Ready` is never re-sent, so the room would spend its full
    /// window and then disconnect a peer that answered immediately.
    ///
    /// Bounded, because this buffer fills whenever there is no accepted request
    /// to match against and nothing else would ever empty it. Still peer-local:
    /// neither folded nor captured, so [`GmRestoreInbox`] stays honestly
    /// `ClearedAtFold`.
    early: Vec<GmRestoreFrame>,
    context: RestoreContext,
}

/// How many unmatched live-restore frames one peer buffers while it waits for
/// its own copy of the canonical request.
///
/// Not a designer's number: a restore's whole vocabulary is four frames per
/// peer, so anything above a small multiple of a plausible roster is already
/// answering a question nobody asked. The bound exists so a peer that never
/// applies a request - because the grant was lost, or because these frames
/// belong to a restore that was abandoned - cannot grow this buffer for the
/// rest of the session.
const EARLY_FRAME_BUDGET: usize = 64;

/// The mirrored facts the driver and the reducer need and cannot reach for
/// themselves.
#[derive(Clone, Debug, Default)]
struct RestoreContext {
    simulation_peers: usize,
    live: Option<LiveSeating>,
    /// This peer's own slot and every technical participant, in slot order.
    local: HostSlot,
    participants: Vec<HostSlot>,
    /// `Time<Real>`'s elapsed seconds - the one clock a held world does not
    /// stop, and therefore the only one a ten-second window can be measured on.
    now: f64,
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

    /// Whether THIS peer is the one coordinating the restore in flight - the
    /// peer whose GM asked, and whose catalogue holds the candidate.
    pub fn coordinating(&self) -> bool {
        self.request
            .as_ref()
            .is_some_and(|request| request.initiator == self.context.local)
    }

    /// Whether this peer is still waiting on `slot` to answer.
    ///
    /// The peer-health rows read this so a facilitator can see WHICH public
    /// peers the room is waiting for, in the vocabulary the desk already uses
    /// (PRD #1420 story 10). The technical slot itself never leaves this crate.
    pub fn waiting_on(&self, slot: HostSlot) -> bool {
        if !self.phase.in_flight() || !self.coordinating() || self.excluded.contains(&slot) {
            return false;
        }
        if slot == self.context.local || !self.context.participants.contains(&slot) {
            return false;
        }
        match self.phase {
            GmRestorePhase::AwaitingReadiness => !self.ready.contains(&slot),
            GmRestorePhase::Loading | GmRestorePhase::AwaitingAgreement => {
                !self.reports.contains_key(&slot)
            }
            _ => false,
        }
    }

    /// Whether `slot` was disconnected for nonresponse by this restore.
    pub fn excluded(&self, slot: HostSlot) -> bool {
        self.excluded.contains(&slot)
    }

    /// How many peers this restore stopped waiting for and disconnected.
    pub fn excluded_peers(&self) -> usize {
        self.excluded.len()
    }

    /// Whole seconds left on the current wait, rounded up, or `None` when this
    /// peer is not waiting on anybody. The countdown the desk draws.
    pub fn remaining_seconds(&self) -> Option<u64> {
        let deadline = self.deadline?;
        if !self.phase.in_flight() {
            return None;
        }
        let left = deadline - self.context.now;
        if left <= 0.0 {
            return Some(0);
        }
        Some(left.ceil() as u64)
    }

    /// How many peers this peer is still waiting on.
    pub fn waiting_peers(&self) -> usize {
        self.context
            .participants
            .iter()
            .filter(|slot| self.waiting_on(**slot))
            .count()
    }

    /// Accept one canonical request. Called only from the ordered reducer.
    ///
    /// Unconditional here, and deliberately so: whether this grant may be
    /// accepted at all is decided in the reducer that calls this, off the
    /// replicated journal ([`crate::gm_action::GmActionJournal::live_restore_admitted`]),
    /// and whether anything ELSE may be sequenced while a restore runs is
    /// decided by the owner in
    /// [`crate::gm_action::sequence_owner_proposal`] before a grant exists.
    /// Neither answer is folded out of this peer-local resource.
    ///
    /// Answers that arrived before this grant did are replayed at the end,
    /// because a peer applies the same grant at the same TICK but not in the
    /// same FRAME, and a `Ready` is sent exactly once.
    pub fn accept(&mut self, request: AcceptedRestore) {
        self.changed_tick = request.requested_tick;
        self.request = Some(request);
        self.recovery_slot = None;
        self.failure = None;
        self.restored_tick = None;
        self.restored_digest = None;
        self.fence_generation = None;
        self.not_ready_frames = 0;
        self.ready.clear();
        self.reports.clear();
        self.excluded.clear();
        self.settle = None;
        self.deadline = None;
        self.live_world = None;
        self.transferred = false;
        self.touched_world = false;
        self.phase = GmRestorePhase::Accepted;
        for frame in std::mem::take(&mut self.early) {
            self.fold_frame(frame);
        }
    }

    /// Fold one live-restore frame into this peer's orchestration.
    ///
    /// A frame naming a restore this peer is not running is dropped, which is
    /// what keeps a late frame from an abandoned attempt out of the current one.
    fn fold_frame(&mut self, frame: GmRestoreFrame) {
        let Some(request) = self.request.as_ref() else {
            return;
        };
        if frame.restore() != request.order {
            return;
        }
        let initiator = request.initiator;
        match frame {
            GmRestoreFrame::Ready { from, .. } => {
                self.ready.insert(from);
            }
            GmRestoreFrame::Loaded { from, digest, .. } => {
                self.reports.insert(from, PeerReport::Loaded(digest));
            }
            GmRestoreFrame::Unable { from, failure, .. } => {
                self.reports.insert(from, PeerReport::Unable(failure));
            }
            // Only the initiating peer decides, and only its word is taken. A
            // settle from anybody else is a peer speaking for the room.
            GmRestoreFrame::Settle {
                from,
                commit,
                failure,
                ..
            } => {
                if from == initiator {
                    self.settle = Some((commit, failure));
                }
            }
        }
    }

    /// Keep one answer that arrived before this peer's own copy of the request.
    ///
    /// Bounded by [`EARLY_FRAME_BUDGET`], dropping the OLDEST: the answers that
    /// matter are the ones nearest the grant that is about to arrive.
    fn buffer_early(&mut self, frame: GmRestoreFrame) {
        if self.early.len() >= EARLY_FRAME_BUDGET {
            self.early.remove(0);
        }
        self.early.push(frame);
    }

    /// The peers this restore must hear from: every technical participant bar
    /// this one and the ones already disconnected for nonresponse.
    ///
    /// Phone consoles are absent by construction - they are legs on a peer, not
    /// participants - which is how "phone consoles do not vote" is implemented
    /// rather than asserted.
    fn expected_peers(&self) -> Vec<HostSlot> {
        self.context
            .participants
            .iter()
            .copied()
            .filter(|slot| *slot != self.context.local && !self.excluded.contains(slot))
            .collect()
    }

    /// Open a bounded wait on the ROOM, from now.
    ///
    /// The coordinator's own wait, and only ever that: it is the one peer whose
    /// counterparties are all answering it directly, so a full
    /// [`PEER_WINDOW_SECONDS`] of silence really is a peer that has gone away.
    fn open_window(&mut self) {
        self.deadline = Some(self.context.now + PEER_WINDOW_SECONDS);
    }

    /// Open a wait a peer keeps on the COORDINATOR, which is necessarily longer
    /// than the wait the coordinator keeps on it.
    ///
    /// EVERY such wait, not only the transfer: a peer waiting to be sent the
    /// candidate, a peer that has loaded and is waiting to be told the room
    /// agreed, and a peer that refused and is waiting for the room's answer are
    /// all the same wait on the same machine, and the same arithmetic bounds
    /// all three.
    ///
    /// A peer on this side of the protocol has no idea how much of ITS OWN
    /// window the coordinator still has to burn: the coordinator may be waiting
    /// on a third peer for the full [`PEER_WINDOW_SECONDS`] before it can send
    /// or decide anything at all, and then needs a frame to commit and announce
    /// on top of that. A peer that started a single window at the moment it
    /// spoke would give up at almost exactly the instant the answer arrived —
    /// and a peer that gave up would roll itself back to its own recovery
    /// checkpoint while the room committed, which is a divergent peer nobody
    /// excluded. Two windows is the exact bound, not a margin: the
    /// coordinator's own wait is bounded by the same constant, so the longest a
    /// live coordinator can legitimately stay silent is one window, and this
    /// allows one more for the work that follows it.
    fn open_long_window(&mut self) {
        self.deadline = Some(self.context.now + 2.0 * PEER_WINDOW_SECONDS);
    }

    /// Whether the current wait has run out of real seconds.
    fn window_expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| self.context.now >= deadline)
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
        if !phase.in_flight() {
            self.deadline = None;
            self.live_world = None;
        }
    }

    /// Clear a reported restore once the GM has explicitly resumed.
    fn clear(&mut self, tick: u64) {
        self.phase = GmRestorePhase::Idle;
        self.request = None;
        self.failure = None;
        self.changed_tick = tick;
        self.ready.clear();
        self.reports.clear();
        self.excluded.clear();
        self.settle = None;
        self.deadline = None;
        self.live_world = None;
        self.transferred = false;
        self.touched_world = false;
        // Anything still unmatched belongs to the restore that just ended.
        self.early.clear();
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

/// Mirror the facts the reducer and the driver cannot reach for themselves.
///
/// A plain read of local state: the roster is the replicated one, the scenario
/// is the same [`crate::save_slots_lifecycle::SaveScenario`] every capture
/// stamps, the hull resolves the roster's per-host placeholder exactly as
/// #1445's picker resolves it, and the clock is `Time<Real>`.
///
/// `Time<Real>` deliberately, and it is the only clock in this module. Every
/// other bound in the simulation is counted in ticks, because ticks are what
/// two peers can agree on - but a live restore HOLDS the world, so the tick
/// count is frozen for the whole operation and a tick-based window would never
/// expire. The window here is not a simulation decision (nothing it produces is
/// folded): it decides only when this peer stops waiting for a machine that has
/// gone quiet, which is exactly the kind of liveness question #1118's own
/// recovery driver already answers off a non-folded signal.
pub fn publish_restore_context(
    restore: Option<ResMut<GmLiveRestore>>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    scenario: Option<Res<crate::save_slots_lifecycle::SaveScenario>>,
    selected: Option<Res<crate::lobby::SelectedShipResource>>,
    clock: Option<Res<Time<Real>>>,
) {
    let Some(mut restore) = restore else {
        return;
    };
    let participants = roster
        .as_ref()
        .map_or_else(Vec::new, |roster| roster.participants());
    let peers = participants.len().max(1);
    let local = roster
        .as_ref()
        .map_or(HostSlot::SOLO, |roster| roster.local());
    let live = match (roster.as_ref(), scenario.as_ref()) {
        (Some(roster), Some(scenario)) if !scenario.0.is_empty() => Some(LiveSeating::from_roster(
            scenario.0.clone(),
            roster,
            selected.as_ref().map(|selected| selected.0.as_str()),
        )),
        _ => None,
    };
    let now = clock.as_ref().map_or(0.0, |clock| clock.elapsed_secs_f64());
    let context = &mut restore.bypass_change_detection().context;
    context.simulation_peers = peers;
    context.live = live;
    context.local = local;
    context.participants = participants;
    context.now = now;
}

/// Advance an accepted live restore by exactly one step per frame.
///
/// Exclusive because every step needs the whole world: a capture, a catalogue
/// read, a whole-world overwrite.
///
/// # The shape of a multi-peer rewind (issue #1447)
///
/// Every peer runs THIS function, off the same canonical request, and the two
/// roles differ by one fact each peer reads off that request: whether its own
/// slot is the initiating one.
///
/// | | initiating peer | every other peer |
/// |---|---|---|
/// | hold | canonical, from the applied request | the same |
/// | recovery checkpoint | its own, locally | its own, locally |
/// | readiness | WAITS, bounded, excludes nonresponders | ANSWERS, once |
/// | candidate | read out of its own catalogue | received over #1118's transport |
/// | load | [`crate::lockstep::snapshot_relay::gate_and_restore_rebuilding`] | the same code, through the armed receiver |
/// | fold | reported to nobody; compared against every report | reported |
/// | decision | MADE | applied |
///
/// Nothing above is a second protocol: the candidate travels as ordinary #1117
/// chunks through #1118's armed receiver, the exclusion is #1119's host loss,
/// the fence is #1119's slot-recovery generation, and the only thing added to
/// the mesh is [`GmRestoreFrame`] - three sentences a rewind of a live room
/// cannot be done without.
pub fn drive_live_restore(world: &mut World) {
    ingest_restore_frames(world);
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
        GmRestorePhase::AwaitingReadiness => await_peer_readiness(world),
        GmRestorePhase::Loading => advance_load(world),
        GmRestorePhase::AwaitingAgreement => await_agreement(world),
        GmRestorePhase::Restored | GmRestorePhase::RolledBack | GmRestorePhase::Failed => {
            release_on_explicit_resume(world)
        }
    }
}

/// Fold every live-restore frame this peer has been handed.
///
/// Deliberately phase-INDEPENDENT, and deliberately request-independent too. A
/// peer that is quicker than this one can answer before this one has finished
/// taking its own recovery checkpoint - or before it has even APPLIED the
/// canonical grant, because peers apply the same grant at the same TICK and a
/// tick is not a frame. An answer that arrived early is still an answer, and
/// `Ready` is sent exactly once: dropping one would make the room wait out its
/// whole window and then disconnect a peer that answered immediately.
///
/// So a frame this peer cannot match yet is BUFFERED rather than dropped, and
/// replayed by [`GmLiveRestore::accept`] the moment the request lands. The
/// inbox itself is still emptied every frame, which is what keeps
/// [`GmRestoreInbox`] honestly `ClearedAtFold`.
fn ingest_restore_frames(world: &mut World) {
    let frames = world
        .get_resource_mut::<GmRestoreInbox>()
        .filter(|inbox| !inbox.is_empty())
        .map(|mut inbox| inbox.drain())
        .unwrap_or_default();
    if frames.is_empty() {
        return;
    }
    let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() else {
        return;
    };
    let accepted = restore.request.is_some();
    for frame in frames {
        if accepted {
            restore.fold_frame(frame);
        } else {
            restore.buffer_early(frame);
        }
    }
}

fn tick_of(world: &World) -> u64 {
    world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0)
}

/// Put one live-restore frame on the wire.
fn announce(world: &mut World, frame: GmRestoreFrame) {
    if let Some(mut outbox) = world.get_resource_mut::<MeshOutbox>() {
        outbox.push(MeshFrame::GmRestore(frame));
    }
}

/// This peer's own slot, the restore's identity, and who is coordinating.
fn identity(world: &World) -> Option<(HostSlot, GmActionOrder, bool)> {
    let restore = world.get_resource::<GmLiveRestore>()?;
    let request = restore.request.as_ref()?;
    Some((
        restore.context.local,
        request.order,
        request.initiator == restore.context.local,
    ))
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
/// [`Idle`](GmRestorePhase::Idle) and spend ticks on an unfenced timeline -
/// with the banner's "Do not resume" sentence never seen. On the refusal paths
/// that never unpaused, it is a no-op.
fn fail(world: &mut World, phase: GmRestorePhase, failure: GmRestoreFailure) {
    let tick = tick_of(world);
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.settle(phase, Some(failure), tick);
    }
    disarm_receiver(world);
    hold_session(world);
}

/// Take this peer out of the #1118 receiver arm it set for this restore.
///
/// A peer that stayed armed would let the next record any peer transmits
/// overwrite its whole world - which is exactly the unbidden overwrite the arm
/// exists to prevent. Disarmed on every settled path, success included.
fn disarm_receiver(world: &mut World) {
    if let Some(mut arm) = world.get_resource_mut::<MeshRestoreArm>() {
        arm.disarm();
    }
}

/// Take the recovery checkpoint, through the ORDINARY save write.
///
/// Deliberately synchronous rather than through the fixed-tick scheduler: the
/// request has already HELD the world, and a hold stops `FixedUpdate` - so a
/// capture scheduled for the next fixed tick would never arrive and the restore
/// would wait forever for its own precondition. A held world is a stable
/// boundary by definition, which is exactly what the fixed-tick rule exists to
/// guarantee, so this captures at that boundary and hands the run to the same
/// [`SaveSlotService::persist`] the scheduler uses: same manual routing, same
/// display name, same outcome FIFO, same catalogue row.
///
/// EVERY peer takes one, not just the initiating one (issue #1447). The
/// candidate is one peer's file, but the way back is every peer's own world,
/// and a fleet that could only roll one machine back is a fleet that cannot
/// roll back at all.
fn begin_recovery_capture(world: &mut World) {
    let Some(scenario) = world
        .get_resource::<crate::save_slots_lifecycle::SaveScenario>()
        .map(|scenario| scenario.0.clone())
        .filter(|path| !path.is_empty())
    else {
        return refuse_locally(
            world,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "this peer has no scenario to record a checkpoint against".to_string(),
            },
        );
    };
    if !world.contains_resource::<SaveSlotService>() {
        return refuse_locally(
            world,
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

/// A failure this peer decided BEFORE anything was loaded anywhere.
///
/// Nothing has been overwritten, so there is nothing to roll back - but the
/// room still has to hear it. A non-initiating peer says so and waits for the
/// terminal decision; the initiating peer abandons the whole operation.
fn refuse_locally(world: &mut World, failure: GmRestoreFailure) {
    let Some((from, restore, coordinating)) = identity(world) else {
        return fail(world, GmRestorePhase::RolledBack, failure);
    };
    if coordinating {
        return abandon(world, failure);
    }
    announce(
        world,
        GmRestoreFrame::Unable {
            from,
            restore,
            failure: failure.clone(),
        },
    );
    let tick = tick_of(world);
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.settle(GmRestorePhase::AwaitingAgreement, None, tick);
        // The reason is kept on this peer even though the phase is a WAIT: when
        // the terminal decision arrives it is the room's, and this peer's own
        // sentence is what tells its operator why the room was refused rather
        // than only that it was.
        restore.failure = Some(failure);
        // The LONG window, because this is a wait on the COORDINATOR and not on
        // the room: the coordinator may still owe a third peer a whole window
        // before it can say anything at all. See `open_long_window`.
        restore.open_long_window();
    }
}

/// Confirm the recovery checkpoint before anything is loaded.
///
/// The #1445 rule, unchanged and deliberately kept as its own step: a write the
/// adapter reported as failed, and a write that reported success but left no
/// readable row, are both "no recovery checkpoint" - and either refuses here,
/// with the candidate world untouched.
fn await_recovery_capture(world: &mut World) {
    let Some(slot_id) = world
        .get_resource::<GmLiveRestore>()
        .and_then(|restore| restore.recovery_slot.clone())
    else {
        return refuse_locally(
            world,
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
        return refuse_locally(
            world,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: format!("{error:?}"),
            },
        );
    }
    let Some(entries) = catalogue(world) else {
        return refuse_locally(
            world,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "this peer has no local save catalogue".to_string(),
            },
        );
    };
    if confirmed_checkpoint(&entries, &slot_id).is_none() {
        return refuse_locally(
            world,
            GmRestoreFailure::RecoveryCaptureFailed {
                detail: "the capture reported success but no readable row came back".to_string(),
            },
        );
    }
    let tick = tick_of(world);
    let Some((from, order, coordinating)) = identity(world) else {
        return;
    };
    if coordinating {
        // Wait on the room, bounded (AC2). A session with no other peer has an
        // empty wait set and passes straight through on the next step.
        if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
            restore.settle(GmRestorePhase::AwaitingReadiness, None, tick);
            restore.open_window();
        }
        return;
    }
    // Every other peer: say so once, arm the #1118 receiver to accept this
    // restore's record from the initiating peer, and keep the world it is about
    // to lose so the LIVE seating can be put back over the candidate.
    arm_for_candidate(world, from);
    announce(
        world,
        GmRestoreFrame::Ready {
            from,
            restore: order,
        },
    );
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.settle(GmRestorePhase::Loading, None, tick);
        restore.open_long_window();
    }
}

/// Arm this peer to accept the initiating peer's record, and only its record.
///
/// #1118's own receiver arm, used for exactly what it is for: condition (a) is
/// set LOCALLY, from the canonical request every peer applied, and condition
/// (b) names the initiating peer that request identifies - so a record from any
/// other slot is dropped without touching the world. `permit_rebuild` is the
/// same choice #1446 made for a live rewind and for the same reason: a held
/// world has stopped becoming readier, and the rows a candidate names that are
/// no longer standing were destroyed, not un-bootstrapped.
fn arm_for_candidate(world: &mut World, local: HostSlot) {
    let leader = world
        .get_resource::<GmLiveRestore>()
        .and_then(|restore| restore.request.as_ref().map(|request| request.initiator))
        .unwrap_or(local);
    if let Some(mut receiver) = world.get_resource_mut::<MeshSnapshotReceiver>() {
        // Act on THIS restore's transfer, never on a stale outcome left behind
        // by a join or an earlier recovery.
        receiver.clear_outcome();
    }
    if let Some(mut arm) = world.get_resource_mut::<MeshRestoreArm>() {
        arm.arm(leader);
        arm.permit_rebuild(leader);
    }
    let live_world = crate::snapshot::capture(world);
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.live_world = Some(Box::new(live_world));
    }
}

fn catalogue(world: &World) -> Option<Vec<SaveSlotEntry>> {
    let service = world.get_resource::<SaveSlotService>()?;
    let current = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());
    service.list(&current, ContentCheck::Full).ok()
}

/// Wait on the room, for at most [`PEER_WINDOW_SECONDS`] real seconds (AC2).
///
/// A peer that has not answered when the window closes is DISCONNECTED through
/// canonical membership - #1119's host loss, the same one a closed socket
/// produces - and the remaining peers carry on. It is not skipped, not assumed
/// willing and not quietly left behind: it leaves the session, and it comes
/// back the way any departed peer comes back, through the existing snapshot
/// recovery, onto whatever state the room ends up in (AC5).
fn await_peer_readiness(world: &mut World) {
    let Some(restore) = world.get_resource::<GmLiveRestore>() else {
        return;
    };
    let expected = restore.expected_peers();
    // A peer that SAYS it cannot take part has answered, and the answer is no.
    // Waiting out its window and then disconnecting it would spend ten seconds
    // to reach a worse conclusion: a peer with no recovery checkpoint of its
    // own is a peer the fleet could not roll back, which is exactly the case
    // the operation must abandon rather than route around.
    let refusing = expected
        .iter()
        .filter(|slot| {
            matches!(
                restore.reports.get(slot),
                Some(PeerReport::Unable(_)) | Some(PeerReport::Loaded(_))
            )
        })
        .count();
    if refusing > 0 {
        return abandon(
            world,
            GmRestoreFailure::PeerLoadFailed {
                peers: u32::try_from(refusing).unwrap_or(u32::MAX),
            },
        );
    }
    let outstanding: Vec<HostSlot> = expected
        .iter()
        .copied()
        .filter(|slot| !restore.ready.contains(slot))
        .collect();
    if outstanding.is_empty() {
        return begin_transfer_and_load(world);
    }
    if !restore.window_expired() {
        return;
    }
    for slot in outstanding {
        exclude_peer(world, slot);
    }
    begin_transfer_and_load(world);
}

/// Stop waiting for one peer, through canonical membership.
///
/// Exactly what a closed socket does (#1119): the loss is agreed at the tick
/// derived from that peer's own last watermark, the barrier stops waiting for
/// it, and the report is broadcast so every survivor converges on the same
/// membership. Nothing here is a restore-private idea of who is in the room.
fn exclude_peer(world: &mut World, slot: HostSlot) {
    let local = world
        .get_resource::<GmLiveRestore>()
        .map_or(HostSlot::SOLO, |restore| restore.context.local);
    let agreed = world
        .get_resource::<crate::lockstep::FleetLockstep>()
        .and_then(|session| session.0.watermark_of(slot))
        .map_or(0, crate::lockstep::host_loss::agreed_loss_tick);
    if let Some(mut pending) =
        world.get_resource_mut::<crate::lockstep::host_loss::PendingHostLoss>()
    {
        pending.observe(slot, agreed);
    }
    if let Some(mut session) = world.get_resource_mut::<crate::lockstep::FleetLockstep>() {
        session.0.depart(slot);
    }
    if let Some(mut outbox) = world.get_resource_mut::<MeshOutbox>() {
        outbox.push(MeshFrame::HostLoss(crate::lockstep::frame::HostLossFrame {
            from: local,
            lost: slot,
            tick: agreed,
        }));
    }
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.excluded.insert(slot);
        restore.ready.remove(&slot);
        restore.reports.remove(&slot);
    }
}

/// Put the candidate on the wire for every remaining peer, then load it here.
fn begin_transfer_and_load(world: &mut World) {
    let tick = tick_of(world);
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.settle(GmRestorePhase::Loading, None, tick);
        restore.open_window();
    }
}

/// One step of "this peer takes the candidate world".
fn advance_load(world: &mut World) {
    match identity(world) {
        Some((_, _, true)) | None => execute_restore(world),
        Some((_, _, false)) => await_transferred_candidate(world),
    }
}

/// Revalidate, transfer, load, verify and retain the live seating - or abandon.
///
/// The initiating peer's half. Everything down to the digest check is #1446's
/// single-peer path, unchanged; what #1447 adds is the transfer to the room in
/// front of it and, after the fold check, a WAIT instead of a success: this
/// peer's own load proves nothing about anybody else's.
fn execute_restore(world: &mut World) {
    // The recovery checkpoint is read back as a precondition, not used here: a
    // rollback goes through `roll_back_to_recovery`, which reads the confirmed
    // slot itself. Requiring it present keeps "no way back, nothing loaded"
    // true on this path too.
    let Some((candidate_slot, _recovery_slot)) =
        world.get_resource::<GmLiveRestore>().and_then(|restore| {
            Some((
                restore.request.as_ref()?.candidate_slot.clone(),
                restore.recovery_slot.clone()?,
            ))
        })
    else {
        return abandon(
            world,
            GmRestoreFailure::CandidateUnreadable {
                detail: "the accepted request is no longer readable".to_string(),
            },
        );
    };

    // Authoritative revalidation at execution, against the CURRENT catalogue
    // and the CURRENT seating - not the advisory answer the picker showed.
    let entries = catalogue(world).unwrap_or_default();
    let live = world
        .get_resource::<GmLiveRestore>()
        .and_then(|restore| restore.context.live.clone());
    let Some(live) = live else {
        return abandon(
            world,
            GmRestoreFailure::CandidateIneligible {
                blocks: vec![CandidateBlock::NoFleetRecord],
            },
        );
    };
    if let Err(failure) = revalidate(
        &live,
        entries.iter().find(|entry| entry.slot_id == candidate_slot),
    ) {
        return abandon(world, failure);
    }

    let Some(text) = world
        .get_resource::<SaveSlotService>()
        .map(|service| service.export(&candidate_slot))
    else {
        return abandon(
            world,
            GmRestoreFailure::CandidateUnreadable {
                detail: "this peer has no local save catalogue".to_string(),
            },
        );
    };
    let text = match text {
        Ok(text) => text,
        Err(refusal) => {
            return abandon(
                world,
                GmRestoreFailure::CandidateUnreadable {
                    detail: refusal.to_string(),
                },
            )
        }
    };

    // The candidate reaches every remaining peer as ordinary #1117 chunks over
    // #1118's transport - the same bytes this peer is about to load, so no peer
    // can end up restoring a different record from the one that was agreed.
    // Sent exactly once, and only after revalidation has passed: a room must
    // not be handed a candidate this session has already refused.
    send_candidate(world, &text);

    // The live seating, captured BEFORE the overwrite: the people at the
    // consoles have not moved, and this is what puts them back.
    let live_world = crate::snapshot::capture(world);

    // The REBUILDING gate, deliberately: this world is held, so it has stopped
    // becoming readier, and the rows the candidate names that are no longer
    // standing were destroyed or removed rather than not yet bootstrapped.
    let outcome = crate::lockstep::snapshot_relay::gate_and_restore_rebuilding(world, &text);
    note_world_walked(world, &outcome);
    let failure = match outcome {
        MeshRestoreOutcome::Committed { tick, digest } => {
            // Retain the LIVE assignments over the restored world. Strictly
            // after the fold check above, which is what proves the load itself
            // was faithful; this divergence is the deliberate one.
            crate::snapshot::restore_mesh_crew(world, &live_world);
            let now = tick_of(world);
            if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
                restore.restored_tick = Some(tick);
                restore.restored_digest = Some(digest);
                restore.settle(GmRestorePhase::AwaitingAgreement, None, now);
                restore.open_window();
            }
            // A restore is never a resume (PRD #1420 story 13), and it is not a
            // success either until the room agrees.
            hold_session(world);
            return;
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
            // has been touched, so try again next frame - but a HELD world
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

    abandon(world, failure);
}

/// Record whether this outcome walked the world, so a later failure knows
/// whether there is anything to put back.
fn note_world_walked(world: &mut World, outcome: &MeshRestoreOutcome) {
    let walked = matches!(
        outcome,
        MeshRestoreOutcome::Committed { .. }
            | MeshRestoreOutcome::RefusedIntegrity { .. }
            | MeshRestoreOutcome::Incomplete { .. }
    );
    if !walked {
        return;
    }
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.touched_world = true;
    }
}

/// Frame the candidate onto the mesh, once.
fn send_candidate(world: &mut World, text: &str) {
    let already = world
        .get_resource::<GmLiveRestore>()
        .is_some_and(|restore| restore.transferred);
    if already {
        return;
    }
    let Some((from, order, _)) = identity(world) else {
        return;
    };
    // The transfer id is the restore's own canonical order, so a chunk from an
    // abandoned attempt can never be reassembled into this one.
    let transfer_id = order
        .sequence
        .wrapping_mul(0x100)
        .wrapping_add(u64::from(order.origin.0));
    let tick = tick_of(world);
    let frames: Vec<MeshFrame> = crate::lockstep::transfer::chunk(text, from, transfer_id, tick)
        .into_iter()
        .map(MeshFrame::Snapshot)
        .collect();
    if let Some(mut outbox) = world.get_resource_mut::<MeshOutbox>() {
        for frame in frames {
            outbox.push(frame);
        }
    }
    if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
        restore.transferred = true;
    }
}

/// Wait for the candidate to arrive and be committed by the armed receiver.
///
/// The receiving half is entirely #1118's: `drain_mesh_restore` runs in
/// `PreUpdate`, sees this peer armed for the initiating one, and runs the very
/// same gate-checkpoint-restore-verify walk with the rebuilding readiness this
/// arm permitted. All this does is READ the outcome it left and turn it into
/// this peer's sentence - and bound the wait, because a held world will never
/// become readier and a transfer that stopped halfway would otherwise hold the
/// room for the rest of the session.
fn await_transferred_candidate(world: &mut World) {
    // The room may already have been answered. A coordinator that abandoned
    // before it ever transferred - because its own revalidation refused the
    // candidate, or because another peer said it could not take part - has
    // nothing left to send, so a peer that kept waiting for a transfer would
    // spend its whole window on a decision it has already been given.
    if world
        .get_resource::<GmLiveRestore>()
        .is_some_and(|restore| restore.settle.is_some())
    {
        return apply_agreement(world);
    }
    let outcome = world
        .get_resource::<MeshSnapshotReceiver>()
        .and_then(|receiver| receiver.last_outcome().cloned());
    if let Some(outcome) = outcome.as_ref() {
        note_world_walked(world, outcome);
    }
    let failure = match outcome {
        None => {
            // Chunks are still landing, so the coordinator is demonstrably
            // alive and this is a slow transfer rather than a silent peer. The
            // window measures SILENCE; refresh it while the record is moving.
            if world
                .get_resource::<MeshSnapshotReceiver>()
                .is_some_and(MeshSnapshotReceiver::is_receiving)
            {
                if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
                    restore.open_window();
                }
                return;
            }
            let expired = world
                .get_resource::<GmLiveRestore>()
                .is_some_and(GmLiveRestore::window_expired);
            if !expired {
                return;
            }
            let missing = world
                .get_resource::<MeshSnapshotReceiver>()
                .map_or(0, |receiver| receiver.missing().len());
            GmRestoreFailure::TransferIncomplete {
                detail: format!(
                    "the candidate never finished arriving ({missing} pieces outstanding)"
                ),
            }
        }
        Some(MeshRestoreOutcome::Committed { tick, digest }) => {
            let live_world = world
                .get_resource_mut::<GmLiveRestore>()
                .and_then(|mut restore| restore.live_world.take());
            if let Some(live_world) = live_world {
                // The same declared divergence the initiating peer makes: the
                // world rewinds, the room does not.
                crate::snapshot::restore_mesh_crew(world, &live_world);
            }
            let Some((from, order, _)) = identity(world) else {
                return;
            };
            announce(
                world,
                GmRestoreFrame::Loaded {
                    from,
                    restore: order,
                    tick,
                    digest,
                },
            );
            let now = tick_of(world);
            if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
                restore.restored_tick = Some(tick);
                restore.restored_digest = Some(digest);
                restore.settle(GmRestorePhase::AwaitingAgreement, None, now);
                // The LONG window, for the reason `open_long_window` gives and
                // which applies to this wait EXACTLY as it applies to the
                // transfer: a peer that loaded quickly has no idea how much of
                // its own window the coordinator still has to burn. A third
                // peer going silent costs the coordinator a full window before
                // `decide_agreement` excludes it, and only then can it commit
                // and announce. A single window here would expire a frame or
                // two before that `Settle` arrived, and this peer would take
                // the `CoordinatorLost` branch and roll itself back while the
                // room committed — a divergent peer that is still a fleet
                // member and that nobody excluded, which is precisely what AC3
                // and AC6 say must never be reachable.
                restore.open_long_window();
            }
            hold_session(world);
            return;
        }
        // #1118 has already rolled this peer's world back to where it began, so
        // there is nothing to undo here - only something to say.
        Some(MeshRestoreOutcome::NotReady) => return,
        Some(MeshRestoreOutcome::RefusedGate(detail)) => GmRestoreFailure::LoadRefused { detail },
        Some(MeshRestoreOutcome::RefusedIntegrity { recorded, restored }) => {
            GmRestoreFailure::DigestMismatch { recorded, restored }
        }
        Some(MeshRestoreOutcome::Incomplete { gaps, .. }) => GmRestoreFailure::LoadIncomplete {
            gaps: u32::try_from(gaps).unwrap_or(u32::MAX),
        },
        Some(other) => GmRestoreFailure::LoadRefused {
            detail: format!("{other:?}"),
        },
    };
    refuse_locally(world, failure);
}

/// Nobody resumes until every remaining peer has said what it got (AC3).
fn await_agreement(world: &mut World) {
    match identity(world) {
        Some((_, _, true)) | None => decide_agreement(world),
        Some((_, _, false)) => apply_agreement(world),
    }
}

/// The initiating peer's decision, and the only asymmetry in the protocol.
fn decide_agreement(world: &mut World) {
    let Some(restore) = world.get_resource::<GmLiveRestore>() else {
        return;
    };
    let expected = restore.expected_peers();
    let silent: Vec<HostSlot> = expected
        .iter()
        .copied()
        .filter(|slot| !restore.reports.contains_key(slot))
        .collect();
    if !silent.is_empty() {
        if !restore.window_expired() {
            return;
        }
        // A peer that went quiet mid-transfer is not guessed about and not
        // waited on forever: it leaves the session through the same canonical
        // membership a nonresponder at the readiness step does, and it comes
        // back through the existing snapshot recovery. It is never RESUMED on
        // whatever half-state it holds, which is the whole of AC6's last line.
        for slot in silent {
            exclude_peer(world, slot);
        }
        return;
    }

    let expected_digest = restore.restored_digest;
    let mut failed = 0u32;
    let mut mismatched = 0u32;
    for slot in &expected {
        match restore.reports.get(slot) {
            Some(PeerReport::Loaded(digest)) if Some(*digest) == expected_digest => {}
            Some(PeerReport::Loaded(_)) => mismatched += 1,
            Some(PeerReport::Unable(_)) | None => failed += 1,
        }
    }
    // This peer's own load may itself have failed before the room was asked.
    let local_failure = restore.failure.clone();
    let local_loaded = restore.restored_digest.is_some();

    if let Some(failure) = local_failure {
        return abandon(world, failure);
    }
    if !local_loaded {
        return abandon(
            world,
            GmRestoreFailure::LoadRefused {
                detail: "this peer never loaded the candidate".to_string(),
            },
        );
    }
    if mismatched > 0 {
        return abandon(
            world,
            GmRestoreFailure::PeerDigestMismatch { peers: mismatched },
        );
    }
    if failed > 0 {
        return abandon(world, GmRestoreFailure::PeerLoadFailed { peers: failed });
    }

    let Some((from, order, _)) = identity(world) else {
        return;
    };
    announce(
        world,
        GmRestoreFrame::Settle {
            from,
            restore: order,
            commit: true,
            failure: None,
        },
    );
    commit_restore(world);
}

/// Every other peer applies the decision it was given, and only that decision.
fn apply_agreement(world: &mut World) {
    let settle = world
        .get_resource::<GmLiveRestore>()
        .and_then(|restore| restore.settle.clone());
    match settle {
        Some((true, _)) => commit_restore(world),
        Some((false, failure)) => {
            let failure = failure.unwrap_or(GmRestoreFailure::PeerLoadFailed { peers: 0 });
            roll_back_to_recovery(world, failure)
        }
        None => {
            let expired = world
                .get_resource::<GmLiveRestore>()
                .is_some_and(GmLiveRestore::window_expired);
            if expired {
                roll_back_to_recovery(world, GmRestoreFailure::CoordinatorLost);
            }
        }
    }
}

/// Commit the rewind on this peer: fence, then hold for an explicit resume.
fn commit_restore(world: &mut World) {
    let restored_tick = world
        .get_resource::<GmLiveRestore>()
        .and_then(|restore| restore.restored_tick);
    let Some(restored_tick) = restored_tick else {
        // Told to commit a world this peer never got. Its own recovery
        // checkpoint is the only honest place to be.
        return roll_back_to_recovery(
            world,
            GmRestoreFailure::LoadRefused {
                detail: "this peer was told to commit a candidate it never loaded".to_string(),
            },
        );
    };
    match fence_slots(world, restored_tick) {
        Ok(generation) => {
            let now = tick_of(world);
            if let Some(mut restore) = world.get_resource_mut::<GmLiveRestore>() {
                restore.fence_generation = Some(generation);
                restore.settle(GmRestorePhase::Restored, None, now);
            }
            disarm_receiver(world);
            hold_session(world);
        }
        Err(detail) => roll_back_to_recovery(world, GmRestoreFailure::FenceFailed { detail }),
    }
}

/// Abandon the whole operation from the initiating peer, and say so once.
///
/// The room is told BEFORE this peer rolls itself back, so a peer holding a
/// committed candidate starts returning at the same moment rather than after a
/// whole-world restore has finished here.
fn abandon(world: &mut World, failure: GmRestoreFailure) {
    if let Some((from, order, true)) = identity(world) {
        announce(
            world,
            GmRestoreFrame::Settle {
                from,
                restore: order,
                commit: false,
                failure: Some(failure.clone()),
            },
        );
    }
    roll_back_to_recovery(world, failure);
}

/// Return this peer to its own recovery checkpoint, through the same gate.
///
/// Deliberately the DURABLE row rather than an in-memory buffer, and
/// deliberately re-run even though [`crate::lockstep::snapshot_relay::gate_and_restore_rebuilding`]
/// has already rolled its own checkpoint back: a rollback nobody verified is not
/// a rollback, and a recovery checkpoint that cannot be read back is the
/// persistent-failure case PRD #1420 requires to stay honestly failed rather
/// than be papered over.
fn roll_back_to_recovery(world: &mut World, failure: GmRestoreFailure) {
    let recovery_slot = world.get_resource::<GmLiveRestore>().and_then(|restore| {
        restore
            .touched_world
            .then(|| restore.recovery_slot.clone())
            .flatten()
    });
    let Some(recovery_slot) = recovery_slot else {
        // Nothing walked this peer's world, so there is nothing to put back:
        // this is a refusal with the world untouched, not a failed rollback.
        return fail(world, GmRestorePhase::RolledBack, failure);
    };
    roll_back(world, &recovery_slot, failure);
}

/// Return the world to the recovery checkpoint, through the same gate.
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

/// Stamp a fresh live protocol generation on EVERY participant's slot.
///
/// The restored journal is the CANDIDATE's, so a grant that was in flight
/// against the abandoned timeline carries a generation that no longer names the
/// current incarnation. This is #1119's durable slot-recovery machinery used
/// for exactly what it is for; no second fencing concept is introduced, and the
/// generation is not exposed anywhere as fictional history.
///
/// # Why every slot, and why that is the same fence on every peer
///
/// #1446 stamped the local slot, because in a one-peer session that IS every
/// slot. A rewind of a fleet is one event for the whole fleet, and the journal
/// it stamps into is DIGEST-FOLDED: a peer that fenced only itself and a peer
/// that fenced only itself would hold two different journals and split the fold
/// on the very next sample. So each peer stamps the whole participant set, in
/// slot order, from the replicated roster - the same input, the same order, the
/// same result everywhere.
///
/// A slot excluded for nonresponse is stamped too. It is out of the room, but
/// it is still in the roster every peer folds, and leaving a hole in the fence
/// would make the fold depend on which peers happened to answer.
///
/// # The boundary is the restored tick itself
///
/// #1119 fences a rejoining slot at a FUTURE tick, because that session keeps
/// running and will reach it. This one never will: the restore holds the world
/// at `restored_tick`, and a held world does not spend ticks. A boundary at
/// `restored_tick + 1` would therefore be unreachable, and - because
/// [`crate::gm_action::sequence_owner_proposal`] forces every later grant to at
/// least its slot's current recovery boundary - it would strand the GM's own
/// explicit Resume, and every action after it, one tick past a stopped clock.
/// So the fence lands exactly where the world is standing. That is the same
/// boundary rule #1119 already documents: both adjacent incarnations are valid
/// ON the boundary tick, and from the next tick only the restored one is.
///
/// A boundary the candidate's own journal already recorded at or after
/// `restored_tick` would be an INERT repeat - [`GmActionJournal::record_slot_recovery`]
/// hands back the generation that is already there - which would leave
/// pre-restore work admissible while reporting a fence. That is refused here
/// (and rolled back by the caller) rather than reported, because an unfenced
/// restore is not a restore that succeeded.
fn fence_slots(world: &mut World, restored_tick: u64) -> Result<u64, String> {
    let mut slots = world
        .get_resource::<FleetRoster>()
        .map(|roster| roster.participants())
        .unwrap_or_default();
    if slots.is_empty() {
        slots.push(
            world
                .get_resource::<FleetRoster>()
                .map_or(HostSlot::SOLO, |roster| roster.local()),
        );
    }
    slots.sort_unstable();
    slots.dedup();
    if !world.contains_resource::<GmActionJournal>() {
        world.insert_resource(GmActionJournal::default());
    }
    let mut journal = world.resource_mut::<GmActionJournal>();
    let mut local_generation = 0;
    for slot in slots {
        let previous = journal.current_recovery_generation(slot);
        let generation = journal
            .record_slot_recovery(slot, restored_tick)
            .map_err(str::to_string)?;
        if generation <= previous {
            return Err(format!(
                "the candidate already recorded generation {generation} at tick {restored_tick}"
            ));
        }
        local_generation = local_generation.max(generation);
    }
    Ok(local_generation)
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
/// every session control already uses - there is no second resume - and this
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

    const LOCAL: HostSlot = HostSlot(1);
    const PEER: HostSlot = HostSlot(2);
    const ELSEWHERE: HostSlot = HostSlot(3);

    /// A peer holding `participants`, sitting at `LOCAL`.
    fn with_peers(participants: &[HostSlot]) -> GmLiveRestore {
        let mut restore = GmLiveRestore::default();
        restore.context.simulation_peers = participants.len().max(1);
        restore.context.live = Some(seating());
        restore.context.local = LOCAL;
        restore.context.participants = participants.to_vec();
        restore
    }

    fn request(initiator: HostSlot) -> AcceptedRestore {
        AcceptedRestore {
            operator_id: "gm-1".to_string(),
            correlation: "one".to_string(),
            candidate_slot: "slot-a".to_string(),
            requested_tick: 10,
            initiator,
            order: GmActionOrder::new(initiator, 1),
        }
    }

    /// Phone consoles are legs on a peer, not participants, so a peer holding a
    /// whole crew of them is one simulation peer and is never waited on twice.
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

    /// A restore in flight is exactly what the owner's sequencing gate refuses
    /// on, and a settled one is exactly what it does not: the two states the
    /// sequencer reads have to be distinguishable from the phase alone.
    #[test]
    fn every_working_phase_is_in_flight_and_every_reported_one_is_not() {
        for phase in [
            GmRestorePhase::Accepted,
            GmRestorePhase::CapturingRecovery,
            GmRestorePhase::AwaitingReadiness,
            GmRestorePhase::Loading,
            GmRestorePhase::AwaitingAgreement,
        ] {
            assert!(phase.in_flight(), "{phase:?} is work in progress");
            assert!(phase.holds_world());
        }
        for phase in [
            GmRestorePhase::Restored,
            GmRestorePhase::RolledBack,
            GmRestorePhase::Failed,
        ] {
            assert!(!phase.in_flight(), "{phase:?} is a reported outcome");
            assert!(
                phase.holds_world(),
                "{phase:?} still leaves the world stopped",
            );
        }
        assert!(!GmRestorePhase::Idle.in_flight());
        assert!(!GmRestorePhase::Idle.holds_world());
    }

    /// Which peers the room is waiting for, and which it has stopped waiting
    /// for, are the two facts the desk draws. Only the coordinating peer waits:
    /// a peer that merely answered is not waiting on anybody.
    #[test]
    fn the_coordinating_peer_names_the_peers_it_is_waiting_for() {
        let mut restore = with_peers(&[LOCAL, PEER, ELSEWHERE]);
        restore.accept(request(LOCAL));
        assert!(restore.coordinating());
        restore.settle(GmRestorePhase::AwaitingReadiness, None, 10);
        assert!(restore.waiting_on(PEER));
        assert!(restore.waiting_on(ELSEWHERE));
        assert!(!restore.waiting_on(LOCAL), "a peer never waits for itself");
        assert_eq!(restore.waiting_peers(), 2);

        restore.ready.insert(PEER);
        assert!(!restore.waiting_on(PEER));
        assert_eq!(restore.waiting_peers(), 1);

        restore.excluded.insert(ELSEWHERE);
        assert!(!restore.waiting_on(ELSEWHERE));
        assert!(restore.excluded(ELSEWHERE));
        assert_eq!(restore.waiting_peers(), 0);
        assert_eq!(restore.excluded_peers(), 1);
        assert_eq!(restore.expected_peers(), vec![PEER]);
    }

    /// A peer that is not coordinating answers and gets on with it; it must
    /// never draw a countdown of its own over the room.
    #[test]
    fn a_peer_that_is_not_coordinating_waits_on_nobody() {
        let mut restore = with_peers(&[LOCAL, PEER]);
        restore.accept(request(PEER));
        assert!(!restore.coordinating());
        restore.settle(GmRestorePhase::Loading, None, 10);
        assert!(!restore.waiting_on(PEER));
        assert_eq!(restore.waiting_peers(), 0);
    }

    /// The countdown is REAL seconds and it runs out. A held world spends no
    /// ticks, so this is the only clock that can end a wait at all.
    #[test]
    fn the_readiness_window_is_ten_real_seconds_and_expires() {
        let mut restore = with_peers(&[LOCAL, PEER]);
        restore.accept(request(LOCAL));
        restore.settle(GmRestorePhase::AwaitingReadiness, None, 10);
        restore.open_window();
        assert_eq!(restore.remaining_seconds(), Some(10));
        assert!(!restore.window_expired());

        restore.context.now += 4.5;
        assert_eq!(
            restore.remaining_seconds(),
            Some(6),
            "a countdown rounds UP, so the last whole second is shown as one",
        );
        assert!(!restore.window_expired());

        restore.context.now += PEER_WINDOW_SECONDS;
        assert!(restore.window_expired());
        assert_eq!(restore.remaining_seconds(), Some(0));

        // A reported outcome is not a wait, whatever the clock says.
        restore.settle(GmRestorePhase::Restored, None, 10);
        assert_eq!(restore.remaining_seconds(), None);
    }

    /// A frame naming a restore this peer is not running is dropped rather than
    /// folded into the current one - the whole point of carrying the canonical
    /// order on every frame.
    #[test]
    fn a_frame_names_the_restore_it_belongs_to() {
        let order = GmActionOrder::new(LOCAL, 7);
        let frame = GmRestoreFrame::Loaded {
            from: PEER,
            restore: order,
            tick: 40,
            digest: 0xfeed,
        };
        assert_eq!(frame.from(), PEER);
        assert_eq!(frame.restore(), order);
        assert_ne!(frame.restore(), GmActionOrder::new(LOCAL, 8));
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
