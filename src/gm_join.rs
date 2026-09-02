//! Deterministic GM admission and reconnect during a running mission (issues
//! #1293 and #1294).
//!
//! The transport may discover a candidate at any rendered-frame boundary, but
//! discovery is not roster admission.  This module keeps the authoritative
//! transaction deliberately small:
//!
//! 1. an already-admitted host or GM visibly approves or rejects one first-time
//!    candidate, or a known disconnected GM capability auto-approves reconnect;
//! 2. the technical owner assigns one logical pause boundary;
//! 3. exactly one canonical snapshot/history record is captured at that boundary;
//! 4. the candidate restores it and reports the restored digest; and
//! 5. only a matching digest yields a roster commit.
//!
//! The owner sequences this protocol but does not gain product authority.  The
//! approving peer is retained separately and may be any existing technical
//! participant.  No method below can add the candidate to a [`FleetRoster`]
//! before [`GmJoinCommit`] exists.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command_admission::HostSlot;
use crate::lockstep::{FleetGm, FleetRoster};

/// One owner-minted paused join transaction identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GmJoinId(pub u64);

/// Why the private candidate is traversing the paused transfer transaction.
///
/// Both kinds intentionally share every boundary after validation. A reconnect
/// is not a lighter protocol: its stale world still restores the owner's whole
/// record and proves the same digest before the frozen slot may re-enter the
/// wait-set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmJoinKind {
    FirstTime,
    Reconnect,
}

/// The private identity reserved for a candidate while it is NOT in the roster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmJoinCandidate {
    pub host: HostSlot,
    pub operator_id: String,
}

/// The one owner-sequenced pause/transfer agreement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmJoinApproval {
    pub id: GmJoinId,
    pub kind: GmJoinKind,
    /// Technical owner which assigned the boundary.  This is sequencing only.
    pub owner: HostSlot,
    /// Existing peer that visibly chose Accept, or the technical owner for an
    /// automatically authenticated reconnect.
    pub approved_by: HostSlot,
    pub candidate: GmJoinCandidate,
    /// Exact logical boundary at which every existing peer enters Pause.
    pub apply_tick: u64,
    /// Whole-record transfer identity passed to the #1117 chunker.
    pub transfer_id: u64,
}

/// Proof which permits the private candidate identity to enter the live roster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmJoinCommit {
    pub id: GmJoinId,
    pub kind: GmJoinKind,
    pub owner: HostSlot,
    pub candidate: GmJoinCandidate,
    pub tick: u64,
    pub digest: u64,
}

/// Typed host-mesh protocol for the paused hand-off.
///
/// JavaScript ferries these opaquely like snapshot and GM-action frames.  A
/// joining candidate is not a roster member until `Committed`, so `Restored`,
/// a candidate-originated `RestoreBoundary`, and candidate terminal refusal
/// have their origins checked against the active reservation rather than the
/// live roster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GmJoinFrame {
    Pause(GmJoinApproval),
    Restored {
        from: HostSlot,
        id: GmJoinId,
        digest: u64,
    },
    /// One owner-sequenced liveness boundary while the candidate is waiting for
    /// the snapshot or for a retained `NotReady` restore to become applicable.
    ///
    /// The candidate reports the boundary it has exhausted; the owner grants
    /// exactly the next value. `from` distinguishes those two directions. This
    /// is a protocol clock, not a rendered-frame counter.
    RestoreBoundary {
        from: HostSlot,
        id: GmJoinId,
        boundary: u16,
    },
    Committed(GmJoinCommit),
    Refused {
        from: HostSlot,
        id: GmJoinId,
        reason: GmJoinRefusal,
    },
}

impl GmJoinFrame {
    pub fn wire_from(&self) -> HostSlot {
        match self {
            Self::Pause(approval) => approval.owner,
            Self::Restored { from, .. } => *from,
            Self::RestoreBoundary { from, .. } => *from,
            Self::Committed(commit) => commit.owner,
            Self::Refused { from, .. } => *from,
        }
    }

    pub fn tick(&self) -> u64 {
        match self {
            Self::Pause(approval) => approval.apply_tick,
            Self::Restored { .. } | Self::RestoreBoundary { .. } | Self::Refused { .. } => 0,
            Self::Committed(commit) => commit.tick,
        }
    }
}

/// A terminal, visible answer which never changes the live roster or pause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GmJoinRefusal {
    JoinInProgress,
    UnknownApprover,
    OwnerMismatch,
    CandidateAlreadyAdmitted,
    OperatorAlreadyAdmitted,
    ReconnectIdentityMismatch,
    ReconnectStillConnected,
    InvalidCandidate,
    ConflictingRetry,
    PauseNotApplied,
    WrongCandidate,
    CandidateDisconnected,
    TransferFailed,
    RestoreFailed,
    RestoreTimedOut,
    DigestMismatch { expected: u64, restored: u64 },
}

/// Public progress projected to every existing surface and the candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum GmJoinProgress {
    #[default]
    Idle,
    AwaitingPause {
        approval: GmJoinApproval,
    },
    Transferring {
        approval: GmJoinApproval,
        tick: u64,
        digest: u64,
    },
    Committed {
        commit: GmJoinCommit,
    },
    Refused {
        id: GmJoinId,
        reason: GmJoinRefusal,
    },
}

/// One bounded GM join transaction, shared by first-time admission and reconnect.
///
/// There is intentionally no queue.  Snapshot capture is a whole-world walk and
/// admitting two new barrier members at one pause boundary would make failure of
/// either candidate ambiguous.  A second request receives a visible
/// [`GmJoinRefusal::JoinInProgress`] and may be retried after the terminal result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GmJoinCoordinator {
    progress: GmJoinProgress,
    /// Retained through the terminal projection so an exact browser retry can
    /// re-emit the already-decided transaction instead of minting another
    /// pause. A different id retires the terminal result and replaces this.
    approval: Option<GmJoinApproval>,
}

impl GmJoinCoordinator {
    pub fn progress(&self) -> &GmJoinProgress {
        &self.progress
    }

    pub fn active_candidate(&self) -> Option<HostSlot> {
        match &self.progress {
            GmJoinProgress::AwaitingPause { approval }
            | GmJoinProgress::Transferring { approval, .. } => Some(approval.candidate.host),
            GmJoinProgress::Committed { commit } => Some(commit.candidate.host),
            GmJoinProgress::Idle | GmJoinProgress::Refused { .. } => None,
        }
    }

    pub fn approval(&self) -> Option<&GmJoinApproval> {
        self.approval.as_ref()
    }

    /// Record one accepted request without touching roster or pause state.
    ///
    /// An exact retry returns the existing approval.  That is the load-bearing
    /// "exactly one pause" property: a repeated browser callback cannot mint a
    /// second boundary or transfer id.
    pub fn approve(
        &mut self,
        roster: &FleetRoster,
        approval: GmJoinApproval,
    ) -> Result<GmJoinApproval, GmJoinRefusal> {
        self.approve_candidate(roster, approval, false)
    }

    /// Record an automatically approved reconnect for an exact departed GM
    /// binding. `candidate_departed` comes from the live lockstep wait-set, not
    /// from a browser-connected flag.
    pub fn approve_reconnect(
        &mut self,
        roster: &FleetRoster,
        approval: GmJoinApproval,
        candidate_departed: bool,
    ) -> Result<GmJoinApproval, GmJoinRefusal> {
        self.approve_candidate(roster, approval, candidate_departed)
    }

    fn approve_candidate(
        &mut self,
        roster: &FleetRoster,
        approval: GmJoinApproval,
        candidate_departed: bool,
    ) -> Result<GmJoinApproval, GmJoinRefusal> {
        if let GmJoinProgress::AwaitingPause { approval: existing }
        | GmJoinProgress::Transferring {
            approval: existing, ..
        } = &self.progress
        {
            return if existing == &approval {
                Ok(existing.clone())
            } else if existing.id == approval.id {
                Err(GmJoinRefusal::ConflictingRetry)
            } else {
                Err(GmJoinRefusal::JoinInProgress)
            };
        }
        if let GmJoinProgress::Committed { commit } = &self.progress {
            return if self.approval.as_ref() == Some(&approval)
                && commit.id == approval.id
                && commit.candidate == approval.candidate
            {
                Ok(approval)
            } else if commit.id == approval.id {
                Err(GmJoinRefusal::ConflictingRetry)
            } else {
                self.progress = GmJoinProgress::Idle;
                self.approval = None;
                self.approve_candidate(roster, approval, candidate_departed)
            };
        }
        if let GmJoinProgress::Refused { id, .. } = self.progress {
            if id == approval.id {
                return Err(GmJoinRefusal::ConflictingRetry);
            }
            self.progress = GmJoinProgress::Idle;
            self.approval = None;
        }
        if approval.owner != roster.owner() {
            return Err(GmJoinRefusal::OwnerMismatch);
        }
        if !roster.is_member(approval.approved_by) {
            return Err(GmJoinRefusal::UnknownApprover);
        }
        if approval.candidate.host == HostSlot::SOLO
            || approval.candidate.operator_id.is_empty()
            || approval.candidate.operator_id.chars().count()
                > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
        {
            return Err(GmJoinRefusal::InvalidCandidate);
        }
        match approval.kind {
            GmJoinKind::FirstTime => {
                if roster.is_member(approval.candidate.host) {
                    return Err(GmJoinRefusal::CandidateAlreadyAdmitted);
                }
                if roster
                    .gms()
                    .iter()
                    .any(|gm| gm.operator_id == approval.candidate.operator_id)
                {
                    return Err(GmJoinRefusal::OperatorAlreadyAdmitted);
                }
            }
            GmJoinKind::Reconnect => {
                if roster.gm_operator(approval.candidate.host)
                    != Some(approval.candidate.operator_id.as_str())
                {
                    return Err(GmJoinRefusal::ReconnectIdentityMismatch);
                }
                if !candidate_departed {
                    return Err(GmJoinRefusal::ReconnectStillConnected);
                }
            }
        }
        self.progress = GmJoinProgress::AwaitingPause {
            approval: approval.clone(),
        };
        self.approval = Some(approval.clone());
        Ok(approval)
    }

    /// Reject a request before approval.  The caller owns the visible request
    /// list; this records its terminal answer and has no roster/pause side effect.
    pub fn reject(&mut self, id: GmJoinId, reason: GmJoinRefusal) -> Result<bool, GmJoinRefusal> {
        match &self.progress {
            GmJoinProgress::Committed { commit } if commit.id == id => {
                return Err(GmJoinRefusal::ConflictingRetry);
            }
            GmJoinProgress::Refused {
                id: existing,
                reason: existing_reason,
            } if *existing == id => {
                return if existing_reason == &reason {
                    Ok(false)
                } else {
                    Err(GmJoinRefusal::ConflictingRetry)
                };
            }
            GmJoinProgress::AwaitingPause { approval }
            | GmJoinProgress::Transferring { approval, .. }
                if approval.id != id =>
            {
                return Err(GmJoinRefusal::WrongCandidate);
            }
            _ => {}
        }
        self.progress = GmJoinProgress::Refused { id, reason };
        Ok(true)
    }

    /// Adopt the owner's agreement on the candidate itself.
    ///
    /// The candidate may carry a provisional bootstrap roster so the ordinary
    /// world loader can create every ship the incoming snapshot overwrites.  It
    /// is not a live admission: existing peers have not added it to their roster
    /// or wait-set, and no commit exists.  This method therefore validates the
    /// owner/approver against the supplied topology but deliberately permits the
    /// local candidate row which [`approve`](Self::approve) must refuse on a live
    /// peer.
    pub fn adopt_candidate(
        &mut self,
        roster: &FleetRoster,
        approval: GmJoinApproval,
    ) -> Result<GmJoinApproval, GmJoinRefusal> {
        if let GmJoinProgress::AwaitingPause { approval: existing }
        | GmJoinProgress::Transferring {
            approval: existing, ..
        } = &self.progress
        {
            return if existing == &approval {
                Ok(existing.clone())
            } else {
                Err(GmJoinRefusal::ConflictingRetry)
            };
        }
        if let GmJoinProgress::Committed { commit } = &self.progress {
            return if self.approval.as_ref() == Some(&approval)
                && commit.id == approval.id
                && commit.candidate == approval.candidate
            {
                Ok(approval)
            } else {
                Err(GmJoinRefusal::ConflictingRetry)
            };
        }
        if let GmJoinProgress::Refused { id, .. } = self.progress {
            if id == approval.id {
                return Err(GmJoinRefusal::ConflictingRetry);
            }
            self.progress = GmJoinProgress::Idle;
            self.approval = None;
        }
        if roster.local() != approval.candidate.host
            || roster.owner() != approval.owner
            || !roster.is_member(approval.approved_by)
            || roster.gm_operator(approval.candidate.host)
                != Some(approval.candidate.operator_id.as_str())
        {
            return Err(GmJoinRefusal::InvalidCandidate);
        }
        self.progress = GmJoinProgress::AwaitingPause {
            approval: approval.clone(),
        };
        self.approval = Some(approval.clone());
        Ok(approval)
    }

    /// Seal the one snapshot/history capture made after the agreed pause landed.
    ///
    /// Returns `true` only for the first seal.  Exact repeats are inert; a
    /// different tick or digest is a conflicting retry and cannot replace the
    /// record already being transferred.
    pub fn pause_applied(
        &mut self,
        id: GmJoinId,
        tick: u64,
        digest: u64,
    ) -> Result<bool, GmJoinRefusal> {
        match &self.progress {
            GmJoinProgress::AwaitingPause { approval } if approval.id == id => {
                if tick < approval.apply_tick {
                    return Err(GmJoinRefusal::PauseNotApplied);
                }
                let approval = approval.clone();
                self.progress = GmJoinProgress::Transferring {
                    approval,
                    tick,
                    digest,
                };
                Ok(true)
            }
            GmJoinProgress::Transferring {
                approval,
                tick: expected_tick,
                digest: expected_digest,
            } if approval.id == id => {
                if *expected_tick == tick && *expected_digest == digest {
                    Ok(false)
                } else {
                    Err(GmJoinRefusal::ConflictingRetry)
                }
            }
            _ => Err(GmJoinRefusal::PauseNotApplied),
        }
    }

    /// Compare the candidate's post-restore fold and produce the only roster
    /// admission proof.  A mismatch is terminal and leaves the roster untouched.
    pub fn restored(
        &mut self,
        id: GmJoinId,
        from: HostSlot,
        restored: u64,
    ) -> Result<GmJoinCommit, GmJoinRefusal> {
        if let GmJoinProgress::Committed { commit } = &self.progress {
            return if commit.id == id && commit.candidate.host == from && commit.digest == restored
            {
                Ok(commit.clone())
            } else {
                Err(GmJoinRefusal::ConflictingRetry)
            };
        }
        let GmJoinProgress::Transferring {
            approval,
            tick,
            digest,
        } = &self.progress
        else {
            return Err(GmJoinRefusal::PauseNotApplied);
        };
        if approval.id != id || approval.candidate.host != from {
            return Err(GmJoinRefusal::WrongCandidate);
        }
        if *digest != restored {
            let refusal = GmJoinRefusal::DigestMismatch {
                expected: *digest,
                restored,
            };
            self.progress = GmJoinProgress::Refused {
                id,
                reason: refusal.clone(),
            };
            return Err(refusal);
        }
        let commit = GmJoinCommit {
            id,
            kind: approval.kind,
            owner: approval.owner,
            candidate: approval.candidate.clone(),
            tick: *tick,
            digest: restored,
        };
        self.progress = GmJoinProgress::Committed {
            commit: commit.clone(),
        };
        Ok(commit)
    }

    /// Start accepting a later request after the terminal result was projected.
    pub fn clear_terminal(&mut self) {
        if matches!(
            self.progress,
            GmJoinProgress::Committed { .. } | GmJoinProgress::Refused { .. }
        ) {
            self.progress = GmJoinProgress::Idle;
            self.approval = None;
        }
    }
}

/// Candidate-private topology used to construct the incoming world without
/// admitting the candidate to the live roster or lockstep wait-set.
///
/// The browser's provisional payload contains the reserved candidate row only
/// so the ordinary world bootstrap knows which local profile it is building.
/// It is stored under this distinct resource type; neither [`FleetRoster`] nor
/// [`crate::lockstep::FleetLockstep`] exists on the candidate until Commit.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct GmJoinBootstrap {
    provisional: FleetRoster,
    candidate: GmJoinCandidate,
}

/// Authenticated topology changes received while a joining GM is still private
/// and therefore has no authoritative roster or lockstep wait-set.
///
/// This resource is deliberately outside the authoritative state registry: a
/// loss can arrive after the owner's snapshot was captured, so folding it into
/// [`crate::lockstep::PendingHostLoss`] before restore would change the
/// candidate's digest and reject an otherwise valid record. [`commit_roster`]
/// is the single transition which drains these records into authoritative state
/// and removes the corresponding slots from the newly installed wait-set.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct GmJoinPendingHostLoss {
    pending: std::collections::BTreeMap<HostSlot, u64>,
}

impl GmJoinPendingHostLoss {
    pub fn observe(&mut self, slot: HostSlot, tick: u64) -> bool {
        let agreed = self.pending.entry(slot).or_default();
        if tick > *agreed {
            *agreed = tick;
            true
        } else {
            false
        }
    }

    pub fn agreed_tick(&self, slot: HostSlot) -> Option<u64> {
        self.pending.get(&slot).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn take(&mut self) -> Vec<crate::lockstep::HostLossRecord> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|(slot, tick)| crate::lockstep::HostLossRecord { slot, tick })
            .collect()
    }
}

impl GmJoinBootstrap {
    pub fn from_provisional(provisional: FleetRoster) -> Result<Self, GmJoinRefusal> {
        let local = provisional.local();
        let Some(operator_id) = provisional.gm_operator(local) else {
            return Err(GmJoinRefusal::InvalidCandidate);
        };
        if provisional.ships().iter().any(|ship| ship.host == local) {
            return Err(GmJoinRefusal::InvalidCandidate);
        }
        Ok(Self {
            candidate: GmJoinCandidate {
                host: local,
                operator_id: operator_id.to_string(),
            },
            provisional,
        })
    }

    pub fn topology(&self) -> &FleetRoster {
        &self.provisional
    }

    fn validates(&self, approval: &GmJoinApproval) -> bool {
        self.candidate == approval.candidate
            && self.provisional.owner() == approval.owner
            && self.provisional.is_member(approval.approved_by)
    }

    fn base_roster(&self, kind: GmJoinKind) -> Result<FleetRoster, GmJoinRefusal> {
        if kind == GmJoinKind::Reconnect {
            return Ok(self.provisional.clone());
        }
        let participants = self
            .provisional
            .participants()
            .into_iter()
            .filter(|host| *host != self.candidate.host)
            .collect();
        let gms = self
            .provisional
            .gms()
            .iter()
            .filter(|gm| gm.host != self.candidate.host)
            .cloned()
            .collect();
        FleetRoster::with_participants_and_gms(
            self.provisional.ships().to_vec(),
            participants,
            gms,
            self.provisional.owner(),
            self.provisional.owner(),
        )
        .ok_or(GmJoinRefusal::InvalidCandidate)
    }
}

/// Install only the candidate-private world-bootstrap topology.
pub fn prepare_candidate_bootstrap(
    world: &mut World,
    provisional: FleetRoster,
) -> Result<(), GmJoinRefusal> {
    if world.contains_resource::<crate::lockstep::FleetLockstep>()
        || world
            .get_resource::<FleetRoster>()
            .is_some_and(|roster| roster != &FleetRoster::default())
    {
        return Err(GmJoinRefusal::CandidateAlreadyAdmitted);
    }
    let bootstrap = GmJoinBootstrap::from_provisional(provisional)?;
    if let Some(existing) = world.get_resource::<GmJoinBootstrap>() {
        return if existing == &bootstrap {
            Ok(())
        } else {
            Err(GmJoinRefusal::ConflictingRetry)
        };
    }
    // `register_lockstep` installs a harmless solo placeholder in every App.
    // A candidate is no longer a solo authority once it starts this private
    // bootstrap, so remove that placeholder rather than letting downstream code
    // mistake it for an admitted roster.
    world.remove_resource::<FleetRoster>();
    world.insert_resource(bootstrap);
    Ok(())
}

/// Add the digest-proven candidate to a cloned roster.  This is the sole roster
/// mutation helper and therefore cannot be called with an approval alone.
pub fn roster_after_commit(
    roster: &FleetRoster,
    commit: &GmJoinCommit,
    local: HostSlot,
) -> Result<FleetRoster, GmJoinRefusal> {
    if commit.owner != roster.owner() {
        return Err(GmJoinRefusal::OwnerMismatch);
    }
    if commit.kind == GmJoinKind::Reconnect {
        return if roster.gm_operator(commit.candidate.host)
            == Some(commit.candidate.operator_id.as_str())
        {
            Ok(roster.clone())
        } else {
            Err(GmJoinRefusal::ReconnectIdentityMismatch)
        };
    }
    if roster.is_member(commit.candidate.host) {
        return if local == commit.candidate.host
            && roster.gm_operator(commit.candidate.host)
                == Some(commit.candidate.operator_id.as_str())
        {
            // A first-time candidate's private provisional topology already
            // contains its own reserved row. Commit publishes that exact row;
            // it must not append a second one.
            Ok(roster.clone())
        } else {
            Err(GmJoinRefusal::CandidateAlreadyAdmitted)
        };
    }
    if roster
        .gms()
        .iter()
        .any(|gm| gm.operator_id == commit.candidate.operator_id)
    {
        return Err(GmJoinRefusal::OperatorAlreadyAdmitted);
    }
    let mut participants = roster.participants();
    participants.push(commit.candidate.host);
    let mut gms = roster.gms().to_vec();
    gms.push(FleetGm {
        host: commit.candidate.host,
        operator_id: commit.candidate.operator_id.clone(),
    });
    FleetRoster::with_participants_and_gms(
        roster.ships().to_vec(),
        participants,
        gms,
        local,
        roster.owner(),
    )
    .ok_or(GmJoinRefusal::InvalidCandidate)
}

/// Frames peeled from the ordinary command/digest lane.  A joining candidate is
/// not yet a [`FleetLockstep`](crate::lockstep::FleetLockstep) member, so these
/// must remain processable beside snapshot chunks before the session gate.
#[derive(Resource, Default, Debug)]
pub struct GmJoinInbox(Vec<GmJoinFrame>);

impl GmJoinInbox {
    pub fn push(&mut self, frame: GmJoinFrame) {
        self.0.push(frame);
    }

    fn take(&mut self) -> Vec<GmJoinFrame> {
        std::mem::take(&mut self.0)
    }
}

/// The three resources the shared mesh drain needs for the pre-admission lane.
///
/// Keeping these in one [`SystemParam`] preserves Bevy's supported function
/// parameter arity without hiding any of the ordinary command-lane resources.
#[derive(SystemParam)]
pub struct GmJoinMeshLane<'w> {
    pub snapshot_rx: ResMut<'w, crate::lockstep::MeshSnapshotReceiver>,
    pub inbox: ResMut<'w, GmJoinInbox>,
    pub runtime: Res<'w, GmJoinRuntime>,
    pub pending_host_loss: ResMut<'w, GmJoinPendingHostLoss>,
    /// Candidate-private topology, used only to authenticate pre-Commit relay
    /// traffic such as a host loss. It never installs a public roster/wait-set.
    pub bootstrap: Option<Res<'w, GmJoinBootstrap>>,
}

/// Peer-local adapter state around the pure coordinator.
#[derive(Resource, Debug, Default)]
pub struct GmJoinRuntime {
    coordinator: GmJoinCoordinator,
    scenario: String,
    restored_reported: bool,
    /// Latest owner-granted liveness boundary for the active restore.
    restore_boundary: u16,
    /// Candidate has already reported waiting at `restore_boundary`.
    restore_boundary_reported: bool,
}

/// Protocol pause which survives the ordinary GM-action reducer.
///
/// The join boundary is not itself a GM action: an existing ship host may
/// approve the candidate, and inventing an attributed GM command for that
/// technical decision would corrupt the durable action history.  The hold is
/// therefore a separate transport source which is ORed into
/// [`SimulationPaused`](crate::gm_action::SimulationPaused).  Once the
/// transaction has a terminal answer, only a newly-applied explicit
/// `SetSessionPaused(false)` may release it.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct GmJoinPauseHold {
    active: bool,
    resolved: bool,
    resume_frontier: usize,
}

impl GmJoinPauseHold {
    pub fn active(&self) -> bool {
        self.active
    }

    fn engage(&mut self, applied_actions: usize) {
        if self.active {
            return;
        }
        self.active = true;
        self.resolved = false;
        self.resume_frontier = applied_actions;
    }

    fn resolve(&mut self, applied_actions: usize) {
        if !self.active || self.resolved {
            return;
        }
        self.resolved = true;
        // A Resume applied while admission was still unproven cannot silently
        // release the boundary the moment Commit arrives.
        self.resume_frontier = applied_actions;
    }

    /// Fold this technical hold with the newly-applied typed action prefix.
    pub fn retain_until_explicit_resume(
        &mut self,
        journal: &crate::gm_action::GmActionJournal,
    ) -> bool {
        if self.active
            && self.resolved
            && journal
                .applied_prefix()
                .iter()
                .skip(self.resume_frontier)
                .any(|grant| grant.action.requested_pause() == Some(false))
        {
            self.active = false;
        }
        self.active
    }
}

impl GmJoinRuntime {
    pub fn progress(&self) -> &GmJoinProgress {
        self.coordinator.progress()
    }

    pub fn active_candidate(&self) -> Option<HostSlot> {
        self.coordinator.active_candidate()
    }

    pub fn refuse(&mut self, id: GmJoinId, reason: GmJoinRefusal) {
        let _ = self.coordinator.reject(id, reason);
    }
}

/// Owner-only entry from the visible browser decision.
///
/// `approved_by` may name any current technical participant.  This function
/// checks it against the frozen roster, while only requiring the caller to be
/// the technical owner which assigns the boundary.
pub fn begin_join(
    world: &mut World,
    id: GmJoinId,
    approved_by: HostSlot,
    candidate: GmJoinCandidate,
    scenario: impl Into<String>,
) -> Result<GmJoinApproval, GmJoinRefusal> {
    begin_transaction(
        world,
        id,
        GmJoinKind::FirstTime,
        approved_by,
        candidate,
        scenario,
    )
}

/// Owner-only automatic entry for a known disconnected GM identity.
///
/// The browser capability selects the private candidate, but this Rust boundary
/// is authoritative: the frozen topology must still bind that exact technical
/// slot to that exact operator id, and lockstep must already mark the slot
/// departed.
pub fn begin_reconnect(
    world: &mut World,
    id: GmJoinId,
    candidate: GmJoinCandidate,
    scenario: impl Into<String>,
) -> Result<GmJoinApproval, GmJoinRefusal> {
    let owner = world
        .get_resource::<FleetRoster>()
        .map(FleetRoster::owner)
        .ok_or(GmJoinRefusal::OwnerMismatch)?;
    begin_transaction(world, id, GmJoinKind::Reconnect, owner, candidate, scenario)
}

fn begin_transaction(
    world: &mut World,
    id: GmJoinId,
    kind: GmJoinKind,
    approved_by: HostSlot,
    candidate: GmJoinCandidate,
    scenario: impl Into<String>,
) -> Result<GmJoinApproval, GmJoinRefusal> {
    let roster = world
        .get_resource::<FleetRoster>()
        .cloned()
        .ok_or(GmJoinRefusal::OwnerMismatch)?;
    let local = roster.local();
    if local != roster.owner() {
        return Err(GmJoinRefusal::OwnerMismatch);
    }
    if world
        .get_resource::<State<crate::core::messages::GamePhase>>()
        .is_some_and(|phase| phase.get() != &crate::core::messages::GamePhase::InProgress)
    {
        return Err(GmJoinRefusal::PauseNotApplied);
    }
    if world.get_resource::<GmJoinRuntime>().is_none() {
        world.insert_resource(GmJoinRuntime::default());
    }

    // Resolve an exact browser retry against the retained approval before
    // calculating a new boundary. The current tick may have moved since the
    // first callback, but that must not turn the same request into a conflicting
    // approval, mint a second pause, or replenish the restore retry budget.
    let retained = world
        .resource::<GmJoinRuntime>()
        .coordinator
        .approval()
        .filter(|approval| approval.id == id)
        .cloned();
    if let Some(approval) = retained {
        if approval.owner != roster.owner()
            || approval.kind != kind
            || approval.approved_by != approved_by
            || approval.candidate != candidate
        {
            return Err(GmJoinRefusal::ConflictingRetry);
        }
        let frame = match world.resource::<GmJoinRuntime>().progress() {
            GmJoinProgress::Committed { commit } => GmJoinFrame::Committed(commit.clone()),
            GmJoinProgress::Refused { id, reason } => GmJoinFrame::Refused {
                from: roster.owner(),
                id: *id,
                reason: reason.clone(),
            },
            GmJoinProgress::AwaitingPause { .. } | GmJoinProgress::Transferring { .. } => {
                GmJoinFrame::Pause(approval.clone())
            }
            GmJoinProgress::Idle => return Err(GmJoinRefusal::ConflictingRetry),
        };
        world
            .resource_mut::<crate::lockstep::MeshOutbox>()
            .push(crate::lockstep::MeshFrame::GmJoin(frame));
        return Ok(approval);
    }

    let now = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    let ready = world
        .get_resource::<crate::lockstep::FleetLockstep>()
        .map_or(now, |session| session.ready_through(now));
    let already_paused = world
        .get_resource::<crate::gm_action::SimulationPaused>()
        .is_some_and(|paused| paused.0);
    let apply_tick = if already_paused {
        now
    } else {
        ready.saturating_add(1).max(now.saturating_add(1))
    };
    let approval = GmJoinApproval {
        id,
        kind,
        owner: roster.owner(),
        approved_by,
        candidate,
        apply_tick,
        // Domain-separated owner-minted id; bounded transfer reassembly still
        // supplies the hard memory ceiling.
        transfer_id: (match kind {
            GmJoinKind::FirstTime => 0x1293_u64,
            GmJoinKind::Reconnect => 0x1294_u64,
        } << 48)
            | (id.0 & 0x0000_ffff_ffff_ffff),
    };
    let candidate_departed = world
        .get_resource::<crate::lockstep::FleetLockstep>()
        .is_some_and(|session| session.has_departed(approval.candidate.host));
    let approved = match kind {
        GmJoinKind::FirstTime => world
            .resource_mut::<GmJoinRuntime>()
            .coordinator
            .approve(&roster, approval)?,
        GmJoinKind::Reconnect => world
            .resource_mut::<GmJoinRuntime>()
            .coordinator
            .approve_reconnect(&roster, approval, candidate_departed)?,
    };
    {
        let mut runtime = world.resource_mut::<GmJoinRuntime>();
        runtime.scenario = scenario.into();
        runtime.restored_reported = false;
        runtime.restore_boundary = 0;
        runtime.restore_boundary_reported = false;
    }
    let terminal = world.resource::<GmJoinRuntime>().progress().clone();
    let frame = match terminal {
        GmJoinProgress::Committed { commit } => GmJoinFrame::Committed(commit),
        GmJoinProgress::Refused { id, reason } => GmJoinFrame::Refused {
            from: roster.owner(),
            id,
            reason,
        },
        _ => GmJoinFrame::Pause(approved.clone()),
    };
    world
        .resource_mut::<crate::lockstep::MeshOutbox>()
        .push(crate::lockstep::MeshFrame::GmJoin(frame));
    Ok(approved)
}

/// End the active transaction from the technical owner's ordered ingress.
///
/// Used when the candidate transport disappears after visible acceptance but
/// before Commit. Exact retries re-emit the same refusal; a refusal racing an
/// already committed transaction is rejected so it cannot regress the roster.
pub fn refuse_join(
    world: &mut World,
    id: GmJoinId,
    reason: GmJoinRefusal,
) -> Result<bool, GmJoinRefusal> {
    let roster = world
        .get_resource::<FleetRoster>()
        .cloned()
        .ok_or(GmJoinRefusal::OwnerMismatch)?;
    if roster.local() != roster.owner() {
        return Err(GmJoinRefusal::OwnerMismatch);
    }
    let mut runtime = world.remove_resource::<GmJoinRuntime>().unwrap_or_default();
    let result = runtime.coordinator.reject(id, reason.clone());
    let first = match result {
        Ok(first) => first,
        Err(error) => {
            // `remove_resource` is necessary while this adapter mutates other
            // world resources, but it must never turn a rejected race into the
            // loss of the durable transaction. In particular, the browser can
            // queue candidate-disconnected after Rust committed and before its
            // next status poll. Preserve and re-project that Commit so JS still
            // promotes the roster and then emits ordinary typed host loss.
            let terminal = match runtime.coordinator.progress() {
                GmJoinProgress::Committed { commit } if commit.id == id => {
                    Some(GmJoinFrame::Committed(commit.clone()))
                }
                GmJoinProgress::Refused {
                    id: refused_id,
                    reason,
                } if *refused_id == id => Some(GmJoinFrame::Refused {
                    from: roster.owner(),
                    id: *refused_id,
                    reason: reason.clone(),
                }),
                _ => None,
            };
            world.insert_resource(runtime);
            if let Some(frame) = terminal {
                world
                    .resource_mut::<crate::lockstep::MeshOutbox>()
                    .push(crate::lockstep::MeshFrame::GmJoin(frame));
            }
            return Err(error);
        }
    };
    resolve_join_pause(world);
    world
        .resource_mut::<crate::lockstep::MeshOutbox>()
        .push(crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Refused {
            from: roster.owner(),
            id,
            reason,
        }));
    world.insert_resource(runtime);
    Ok(first)
}

fn set_join_pause(world: &mut World) {
    let applied_actions = world
        .get_resource::<crate::gm_action::GmActionJournal>()
        .map_or(0, |journal| journal.applied_grants());
    if let Some(mut hold) = world.get_resource_mut::<GmJoinPauseHold>() {
        hold.engage(applied_actions);
    } else {
        let mut hold = GmJoinPauseHold::default();
        hold.engage(applied_actions);
        world.insert_resource(hold);
    }
    if let Some(mut paused) = world.get_resource_mut::<crate::gm_action::SimulationPaused>() {
        paused.0 = true;
    } else {
        world.insert_resource(crate::gm_action::SimulationPaused(true));
    }
    if let Some(mut time) = world.get_resource_mut::<Time<bevy::time::Virtual>>() {
        time.pause();
        time.advance_by(std::time::Duration::ZERO);
    }
    if let Some(mut fixed) = world.get_resource_mut::<Time<Fixed>>() {
        let overstep = fixed.overstep();
        let timestep = fixed.timestep();
        let remainder_nanos = overstep.as_nanos() % timestep.as_nanos();
        let remainder = std::time::Duration::new(
            u64::try_from(remainder_nanos / 1_000_000_000).unwrap_or(u64::MAX),
            (remainder_nanos % 1_000_000_000) as u32,
        );
        fixed.discard_overstep(overstep - remainder);
    }
    // The canonical peer may enter the transfer hold before another fixed
    // step re-arms these derived latches, while the returning peer necessarily
    // derives them from the restored SimTick. Align the live side through the
    // same #1118 restore seam so the first explicit post-Commit Resume cannot
    // give either peer an extra AI decision.
    crate::ai::cadence::rederive_ai_cadence(world);
}

fn commit_roster(world: &mut World, commit: &GmJoinCommit) -> Result<(), GmJoinRefusal> {
    let (before, local) = if let Some(roster) = world.get_resource::<FleetRoster>().cloned() {
        let local = roster.local();
        (roster, local)
    } else {
        let bootstrap = world
            .get_resource::<GmJoinBootstrap>()
            .cloned()
            .ok_or(GmJoinRefusal::InvalidCandidate)?;
        (
            bootstrap.base_roster(commit.kind)?,
            bootstrap.candidate.host,
        )
    };
    let after = roster_after_commit(&before, commit, local)?;
    let delay = world
        .get_resource::<crate::lockstep::FleetLockstep>()
        .map_or_else(
            || crate::lockstep::authored_delay(world),
            |session| session.delay(),
        );
    world.insert_resource(after.clone());
    if local == commit.candidate.host
        && world
            .get_resource::<crate::lockstep::FleetLockstep>()
            .is_none()
    {
        let session = crate::lockstep::LockstepSession::new_at(
            local,
            after.participants(),
            delay,
            commit.tick,
        )
        .ok_or(GmJoinRefusal::InvalidCandidate)?;
        world.insert_resource(crate::lockstep::FleetLockstep(session));
        world.insert_resource(crate::command_admission::CommandDelay(delay));
        if let Some(mut pending) =
            world.get_resource_mut::<crate::command_admission::PendingCommands>()
        {
            pending.set_origin(local);
        }
    } else if let Some(mut session) = world.get_resource_mut::<crate::lockstep::FleetLockstep>() {
        match commit.kind {
            GmJoinKind::FirstTime => session.admit_peer(commit.candidate.host, commit.tick),
            GmJoinKind::Reconnect => session.rejoin(commit.candidate.host, commit.tick),
        }
    }
    // The owner can relay a peer loss after capturing the join record but before
    // digest Commit. A private candidate staged that authenticated carried tick
    // without a roster; mark every such slot departed in the newly installed
    // wait-set now, before Resume can ask the barrier to advance. Existing peers
    // already departed these slots, so the same loop is idempotent there.
    let staged_losses = world
        .get_resource_mut::<GmJoinPendingHostLoss>()
        .map(|mut pending| pending.take())
        .unwrap_or_default();
    if let Some(mut pending) = world.get_resource_mut::<crate::lockstep::PendingHostLoss>() {
        for loss in &staged_losses {
            pending.observe(loss.slot, loss.tick);
        }
    }
    if let Some(mut session) = world.get_resource_mut::<crate::lockstep::FleetLockstep>() {
        for loss in &staged_losses {
            session.depart(loss.slot);
        }
    }
    // The whole-record permission is transaction-scoped. Once Commit installs
    // the proven wait-set, no later snapshot frame may reuse this join arm to
    // overwrite an admitted host.
    if local == commit.candidate.host {
        if let Some(mut arm) = world.get_resource_mut::<crate::lockstep::MeshRestoreArm>() {
            arm.disarm();
        }
    }
    world.remove_resource::<GmJoinBootstrap>();
    resolve_join_pause(world);
    Ok(())
}

fn resolve_join_pause(world: &mut World) {
    let applied_actions = world
        .get_resource::<crate::gm_action::GmActionJournal>()
        .map_or(0, |journal| journal.applied_grants());
    if let Some(mut hold) = world.get_resource_mut::<GmJoinPauseHold>() {
        hold.resolve(applied_actions);
    }
}

/// Consume join frames, apply the one pause boundary, and queue the one canonical
/// snapshot/history transfer from the technical owner.
pub fn drive_join(world: &mut World) {
    let frames = world
        .get_resource_mut::<GmJoinInbox>()
        .map(|mut inbox| inbox.take())
        .unwrap_or_default();
    let mut runtime = world.remove_resource::<GmJoinRuntime>().unwrap_or_default();
    for frame in frames {
        match frame {
            GmJoinFrame::Pause(approval) => {
                let local = world
                    .get_resource::<FleetRoster>()
                    .map(FleetRoster::local)
                    .or_else(|| {
                        world
                            .get_resource::<GmJoinBootstrap>()
                            .map(|bootstrap| bootstrap.candidate.host)
                    });
                let adopted = if local == Some(approval.candidate.host) {
                    let Some(bootstrap) = world.get_resource::<GmJoinBootstrap>() else {
                        continue;
                    };
                    if bootstrap.validates(&approval) {
                        runtime
                            .coordinator
                            .adopt_candidate(bootstrap.topology(), approval.clone())
                    } else {
                        Err(GmJoinRefusal::InvalidCandidate)
                    }
                } else {
                    let Some(roster) = world.get_resource::<FleetRoster>() else {
                        continue;
                    };
                    match approval.kind {
                        GmJoinKind::FirstTime => {
                            runtime.coordinator.approve(roster, approval.clone())
                        }
                        GmJoinKind::Reconnect => {
                            let departed = world
                                .get_resource::<crate::lockstep::FleetLockstep>()
                                .is_some_and(|session| {
                                    session.has_departed(approval.candidate.host)
                                });
                            runtime.coordinator.approve_reconnect(
                                roster,
                                approval.clone(),
                                departed,
                            )
                        }
                    }
                };
                if let Err(reason) = adopted {
                    let _ = runtime.coordinator.reject(approval.id, reason);
                    continue;
                }
                runtime.restored_reported = false;
                runtime.restore_boundary = 0;
                runtime.restore_boundary_reported = false;
                if local == Some(approval.candidate.host) {
                    if approval.kind == GmJoinKind::Reconnect {
                        // A newly opened returning-GM page is still in Lobby and
                        // therefore cannot enter GameStart until the accepted
                        // canonical record stages its exact saved UUID map. The
                        // authenticated Pause freezes the candidate now; the
                        // reconnect restore arm requests that private bootstrap
                        // only after the record passes its version/content gate.
                        set_join_pause(world);
                    }
                    // Both join kinds arrive as private Lobby worlds. Their
                    // canonical record must stage the original GameStart UUIDs
                    // before requesting InProgress; otherwise a named authored
                    // row can mint a different identity and a by-UUID restore
                    // can never become ready. Reconnect engages its pause above
                    // immediately because its local clock may be arbitrarily
                    // stale; a first-time candidate still reaches the one
                    // owner-authored pause boundary normally.
                    world
                        .resource_mut::<crate::lockstep::MeshRestoreArm>()
                        .arm_join_candidate(approval.owner);
                    world
                        .resource_mut::<crate::lockstep::MeshSnapshotReceiver>()
                        .clear_outcome();
                }
            }
            GmJoinFrame::Restored { from, id, digest } => {
                let local_is_owner = world
                    .get_resource::<FleetRoster>()
                    .is_some_and(|roster| roster.local() == roster.owner());
                if !local_is_owner {
                    continue;
                }
                match runtime.coordinator.restored(id, from, digest) {
                    Ok(commit) => {
                        if commit_roster(world, &commit).is_ok() {
                            world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                                crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Committed(commit)),
                            );
                        }
                    }
                    Err(_) => match runtime.coordinator.progress().clone() {
                        GmJoinProgress::Committed { commit } if commit.id == id => {
                            world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                                crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Committed(commit)),
                            );
                        }
                        GmJoinProgress::Refused {
                            id: refused_id,
                            reason,
                        } if refused_id == id => {
                            resolve_join_pause(world);
                            let owner = world.resource::<FleetRoster>().owner();
                            world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                                crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Refused {
                                    from: owner,
                                    id,
                                    reason,
                                }),
                            );
                        }
                        _ => {}
                    },
                }
            }
            GmJoinFrame::RestoreBoundary { from, id, boundary } => {
                let approval = match runtime.coordinator.progress() {
                    GmJoinProgress::AwaitingPause { approval }
                    | GmJoinProgress::Transferring { approval, .. }
                        if approval.id == id =>
                    {
                        approval.clone()
                    }
                    _ => continue,
                };
                let local = world
                    .get_resource::<FleetRoster>()
                    .map(FleetRoster::local)
                    .or_else(|| {
                        world
                            .get_resource::<GmJoinBootstrap>()
                            .map(|bootstrap| bootstrap.candidate.host)
                    });
                if local == Some(approval.owner) && from == approval.candidate.host {
                    if boundary > runtime.restore_boundary {
                        // A candidate cannot mint the owner's protocol clock.
                        continue;
                    }
                    if boundary < runtime.restore_boundary {
                        // Exact/stale request retry: re-project the one boundary
                        // already granted without advancing it.
                        world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                            crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::RestoreBoundary {
                                from: approval.owner,
                                id,
                                boundary: runtime.restore_boundary,
                            }),
                        );
                        continue;
                    }
                    if boundary >= GM_JOIN_RESTORE_TIMEOUT_BOUNDARY {
                        let reason = GmJoinRefusal::RestoreTimedOut;
                        if runtime.coordinator.reject(id, reason.clone()).is_ok() {
                            resolve_join_pause(world);
                            world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                                crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Refused {
                                    from: approval.owner,
                                    id,
                                    reason,
                                }),
                            );
                        }
                    } else {
                        runtime.restore_boundary = boundary.saturating_add(1);
                        world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                            crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::RestoreBoundary {
                                from: approval.owner,
                                id,
                                boundary: runtime.restore_boundary,
                            }),
                        );
                    }
                } else if local == Some(approval.candidate.host) && from == approval.owner {
                    if boundary == runtime.restore_boundary.saturating_add(1) {
                        runtime.restore_boundary = boundary;
                        runtime.restore_boundary_reported = false;
                        if boundary == GM_JOIN_RESTORE_TIMEOUT_BOUNDARY {
                            // The deterministic owner-granted wait budget is
                            // exhausted. On this final attempt, use the same
                            // #863 snapshot builder as save resume for authored
                            // rows which the paused fresh bootstrap did not
                            // produce. The receiver stays armed to this exact
                            // owner and the ordinary integrity/rollback proof is
                            // unchanged; a non-rebuildable world still reports
                            // boundary 8 and receives the typed timeout.
                            world
                                .resource_mut::<crate::lockstep::MeshRestoreArm>()
                                .permit_rebuild(approval.owner);
                        }
                    } else if boundary > runtime.restore_boundary {
                        candidate_refusal(
                            world,
                            &mut runtime,
                            &approval,
                            GmJoinRefusal::ConflictingRetry,
                        );
                    }
                    // Exact/stale owner retransmissions are inert.
                }
            }
            GmJoinFrame::Committed(commit) => {
                if runtime
                    .coordinator
                    .restored(commit.id, commit.candidate.host, commit.digest)
                    .as_ref()
                    == Ok(&commit)
                {
                    let _ = commit_roster(world, &commit);
                }
            }
            GmJoinFrame::Refused { from, id, reason } => {
                if runtime.coordinator.reject(id, reason.clone()).is_ok() {
                    resolve_join_pause(world);
                    let owner_receiving_candidate =
                        world.get_resource::<FleetRoster>().is_some_and(|roster| {
                            roster.local() == roster.owner() && from != roster.owner()
                        });
                    if owner_receiving_candidate {
                        let owner = world.resource::<FleetRoster>().owner();
                        world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                            crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Refused {
                                from: owner,
                                id,
                                reason,
                            }),
                        );
                    }
                } else if let GmJoinProgress::Committed { commit } = runtime.coordinator.progress()
                {
                    let local_is_owner = world
                        .get_resource::<FleetRoster>()
                        .is_some_and(|roster| roster.local() == roster.owner());
                    if commit.id == id && local_is_owner {
                        world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                            crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Committed(
                                commit.clone(),
                            )),
                        );
                    }
                }
            }
        }
    }

    let now = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    let due = match runtime.coordinator.progress() {
        GmJoinProgress::AwaitingPause { approval } if now >= approval.apply_tick => {
            Some(approval.clone())
        }
        _ => None,
    };
    if let Some(approval) = due {
        set_join_pause(world);
        let local = world
            .get_resource::<FleetRoster>()
            .map(FleetRoster::local)
            .or_else(|| {
                world
                    .get_resource::<GmJoinBootstrap>()
                    .map(|bootstrap| bootstrap.candidate.host)
            });
        if local != Some(approval.candidate.host) {
            let digest = crate::sim_digest::world_digest(world);
            if runtime
                .coordinator
                .pause_applied(approval.id, now, digest)
                .unwrap_or(false)
                && local == Some(approval.owner)
            {
                let run = crate::lockstep::snapshot_relay::capture_join_run(
                    world,
                    runtime.scenario.clone(),
                );
                if let Ok(frames) = crate::lockstep::snapshot_relay::frames_for(
                    &run,
                    approval.owner,
                    approval.transfer_id,
                ) {
                    let mut outbox = world.resource_mut::<crate::lockstep::MeshOutbox>();
                    for frame in frames {
                        outbox.push(frame);
                    }
                } else {
                    let reason = GmJoinRefusal::TransferFailed;
                    let _ = runtime.coordinator.reject(approval.id, reason.clone());
                    resolve_join_pause(world);
                    world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                        crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Refused {
                            from: approval.owner,
                            id: approval.id,
                            reason,
                        }),
                    );
                }
            }
        }
    }
    world.insert_resource(runtime);
}

/// Terminal owner-sequenced restore boundary for an accepted transfer.
///
/// Render-only updates cannot advance this value. The candidate reports one
/// exhausted boundary, the authenticated owner grants exactly the next, and
/// exact retransmissions re-project rather than increment it.
pub const GM_JOIN_RESTORE_TIMEOUT_BOUNDARY: u16 = 8;

fn restore_refusal(outcome: &crate::lockstep::MeshRestoreOutcome) -> Option<GmJoinRefusal> {
    match outcome {
        crate::lockstep::MeshRestoreOutcome::Committed { .. }
        | crate::lockstep::MeshRestoreOutcome::NotReady => None,
        crate::lockstep::MeshRestoreOutcome::RefusedChunk(_) => Some(GmJoinRefusal::TransferFailed),
        crate::lockstep::MeshRestoreOutcome::RefusedGate(_)
        | crate::lockstep::MeshRestoreOutcome::RefusedIntegrity { .. }
        | crate::lockstep::MeshRestoreOutcome::Incomplete { .. }
        | crate::lockstep::MeshRestoreOutcome::RefusedUnarmed
        | crate::lockstep::MeshRestoreOutcome::RefusedWrongSender { .. } => {
            Some(GmJoinRefusal::RestoreFailed)
        }
    }
}

fn candidate_refusal(
    world: &mut World,
    runtime: &mut GmJoinRuntime,
    approval: &GmJoinApproval,
    reason: GmJoinRefusal,
) {
    if runtime
        .coordinator
        .reject(approval.id, reason.clone())
        .is_err()
    {
        return;
    }
    resolve_join_pause(world);
    if let Some(mut arm) = world.get_resource_mut::<crate::lockstep::MeshRestoreArm>() {
        arm.disarm();
    }
    world
        .resource_mut::<crate::lockstep::MeshOutbox>()
        .push(crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Refused {
            from: approval.candidate.host,
            id: approval.id,
            reason,
        }));
    runtime.restored_reported = true;
}

fn report_restore_wait_boundary(
    world: &mut World,
    runtime: &mut GmJoinRuntime,
    approval: &GmJoinApproval,
) {
    if runtime.restore_boundary_reported {
        return;
    }
    world
        .resource_mut::<crate::lockstep::MeshOutbox>()
        .push(crate::lockstep::MeshFrame::GmJoin(
            GmJoinFrame::RestoreBoundary {
                from: approval.candidate.host,
                id: approval.id,
                boundary: runtime.restore_boundary,
            },
        ));
    runtime.restore_boundary_reported = true;
}

/// A fresh joining-GM page cannot spend its bounded restore clock before the
/// canonical record has actually produced the authored GameStart roster.
///
/// `GameStartEntityUuids` is inserted only after the spawn system has walked
/// every authored row (including an empty roster). This is the same durable
/// prerequisite used by browser save resume: phase alone is too early because
/// its `OnEnter` commands are deferred. It applies equally to a first-time
/// mid-session candidate and a returning one: both remain private Lobby worlds
/// until the accepted record stages the original UUID map.
fn join_restore_clock_started(world: &World) -> bool {
    world.contains_resource::<crate::server_app::GameStartEntityUuids>()
}

/// After the #1117 relay resolves, report either the proven fold or a terminal
/// refusal exactly once from the candidate. The owner alone turns a successful
/// proof into `Committed`; neither success nor failure admits a roster here.
pub fn report_restored_join(world: &mut World) {
    let (outcome, restore_started) = world
        .get_resource::<crate::lockstep::MeshSnapshotReceiver>()
        .map(|receiver| {
            (
                receiver.last_outcome().cloned(),
                receiver.is_receiving()
                    || receiver.has_staged_record()
                    || receiver.last_outcome().is_some(),
            )
        })
        .unwrap_or((None, false));
    let mut runtime = world.remove_resource::<GmJoinRuntime>().unwrap_or_default();
    if runtime.restored_reported {
        world.insert_resource(runtime);
        return;
    }
    let approval = match runtime.coordinator.progress() {
        GmJoinProgress::AwaitingPause { approval }
        | GmJoinProgress::Transferring { approval, .. } => Some(approval.clone()),
        _ => None,
    };
    if let Some(approval) = approval {
        let local = world
            .get_resource::<FleetRoster>()
            .map(FleetRoster::local)
            .or_else(|| {
                world
                    .get_resource::<GmJoinBootstrap>()
                    .map(|bootstrap| bootstrap.candidate.host)
            });
        if local == Some(approval.candidate.host) {
            match outcome {
                Some(crate::lockstep::MeshRestoreOutcome::Committed { tick, digest }) => {
                    // A behind candidate may restore directly onto the owner's
                    // boundary without ever locally crossing `apply_tick`. Engage
                    // the same technical hold before projecting proof so Commit
                    // cannot expose the snapshot's durable Running state.
                    set_join_pause(world);
                    match runtime.coordinator.pause_applied(approval.id, tick, digest) {
                        Ok(_) => {
                            world.resource_mut::<crate::lockstep::MeshOutbox>().push(
                                crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Restored {
                                    from: approval.candidate.host,
                                    id: approval.id,
                                    digest,
                                }),
                            );
                            runtime.restored_reported = true;
                        }
                        Err(reason) => {
                            candidate_refusal(world, &mut runtime, &approval, reason);
                        }
                    }
                }
                Some(outcome) => {
                    if let Some(reason) = restore_refusal(&outcome) {
                        candidate_refusal(world, &mut runtime, &approval, reason);
                    } else if join_restore_clock_started(world) {
                        report_restore_wait_boundary(world, &mut runtime, &approval);
                    }
                }
                None => {
                    if join_restore_clock_started(world)
                        && (restore_started
                            || world
                                .get_resource::<GmJoinPauseHold>()
                                .is_some_and(GmJoinPauseHold::active))
                    {
                        report_restore_wait_boundary(world, &mut runtime, &approval);
                    }
                }
            }
        }
    }
    world.insert_resource(runtime);
}

pub fn register_join_driver(app: &mut App) {
    {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<GmJoinInbox>(StateClass::Timer, "gm-join-state")
            .declare_state::<GmJoinRuntime>(StateClass::Timer, "gm-join-state")
            .declare_state::<GmJoinPauseHold>(StateClass::Timer, "gm-join-state")
            // Same join-transaction transport bookkeeping as their siblings
            // above: the candidate's private pre-admission staging and the
            // owner's pending host-loss watch never enter the folded world.
            .declare_state::<GmJoinBootstrap>(StateClass::Timer, "gm-join-state")
            .declare_state::<GmJoinPendingHostLoss>(StateClass::Timer, "gm-join-state");
    }
    app.init_resource::<GmJoinInbox>()
        .init_resource::<GmJoinRuntime>()
        .init_resource::<GmJoinPauseHold>()
        .init_resource::<GmJoinPendingHostLoss>()
        .add_systems(
            PreUpdate,
            drive_join
                .after(crate::lockstep::MeshSet)
                .before(crate::lockstep::snapshot_relay::drain_mesh_restore),
        )
        .add_systems(
            PreUpdate,
            report_restored_join.after(crate::lockstep::snapshot_relay::drain_mesh_restore),
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockstep::{FleetGm, FleetRoster, FleetShip};

    fn roster(local: HostSlot) -> FleetRoster {
        FleetRoster::with_participants_and_gms(
            vec![FleetShip::new(HostSlot(1))],
            vec![HostSlot(1), HostSlot(2)],
            vec![FleetGm {
                host: HostSlot(2),
                operator_id: "gm-1".into(),
            }],
            local,
            HostSlot(1),
        )
        .unwrap()
    }

    fn first_time_candidate_roster() -> FleetRoster {
        FleetRoster::with_participants_and_gms(
            vec![FleetShip::new(HostSlot(1))],
            vec![HostSlot(1), HostSlot(2), HostSlot(3)],
            vec![
                FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-1".into(),
                },
                FleetGm {
                    host: HostSlot(3),
                    operator_id: "gm-2".into(),
                },
            ],
            HostSlot(3),
            HostSlot(1),
        )
        .unwrap()
    }

    fn approval() -> GmJoinApproval {
        GmJoinApproval {
            id: GmJoinId(7),
            kind: GmJoinKind::FirstTime,
            owner: HostSlot(1),
            approved_by: HostSlot(2),
            candidate: GmJoinCandidate {
                host: HostSlot(3),
                operator_id: "gm-2".into(),
            },
            apply_tick: 42,
            transfer_id: 700,
        }
    }

    fn reconnect_approval() -> GmJoinApproval {
        GmJoinApproval {
            id: GmJoinId(8),
            kind: GmJoinKind::Reconnect,
            owner: HostSlot(1),
            approved_by: HostSlot(1),
            candidate: GmJoinCandidate {
                host: HostSlot(2),
                operator_id: "gm-1".into(),
            },
            apply_tick: 84,
            transfer_id: 0x1294_0000_0000_0008,
        }
    }

    #[test]
    fn reconnect_requires_the_exact_departed_frozen_gm_binding() {
        let roster = roster(HostSlot(1));

        let mut connected = GmJoinCoordinator::default();
        assert_eq!(
            connected.approve_reconnect(&roster, reconnect_approval(), false),
            Err(GmJoinRefusal::ReconnectStillConnected)
        );

        let mut wrong_slot = reconnect_approval();
        wrong_slot.candidate.host = HostSlot(3);
        let mut coordinator = GmJoinCoordinator::default();
        assert_eq!(
            coordinator.approve_reconnect(&roster, wrong_slot, true),
            Err(GmJoinRefusal::ReconnectIdentityMismatch)
        );

        let mut wrong_operator = reconnect_approval();
        wrong_operator.candidate.operator_id = "gm-other".into();
        assert_eq!(
            coordinator.approve_reconnect(&roster, wrong_operator, true),
            Err(GmJoinRefusal::ReconnectIdentityMismatch)
        );

        let exact = reconnect_approval();
        assert_eq!(
            coordinator.approve_reconnect(&roster, exact.clone(), true),
            Ok(exact.clone())
        );
        assert_eq!(
            coordinator.approve_reconnect(&roster, exact.clone(), true),
            Ok(exact),
            "an exact retry reuses the one transaction"
        );
    }

    #[test]
    fn reconnect_can_follow_a_completed_join_without_reusing_its_kind() {
        let roster = roster(HostSlot(1));
        let first = approval();
        let mut coordinator = GmJoinCoordinator::default();
        coordinator.approve(&roster, first.clone()).unwrap();
        coordinator
            .pause_applied(first.id, first.apply_tick, 0xaaaa)
            .unwrap();
        coordinator
            .restored(first.id, first.candidate.host, 0xaaaa)
            .unwrap();

        let reconnect = reconnect_approval();
        assert_eq!(
            coordinator.approve_reconnect(&roster, reconnect.clone(), true),
            Ok(reconnect),
            "a later reconnect retains its departed-slot validation"
        );
    }

    #[test]
    fn reconnect_commit_preserves_roster_and_rejoins_the_departed_wait_set() {
        let frozen = roster(HostSlot(1));
        let mut session =
            crate::lockstep::LockstepSession::new_at(HostSlot(1), frozen.participants(), 6, 40)
                .unwrap();
        session.depart(HostSlot(2));

        let mut world = World::new();
        world.insert_resource(frozen.clone());
        world.insert_resource(crate::lockstep::FleetLockstep(session));
        world.insert_resource(GmJoinPendingHostLoss::default());
        world.insert_resource(GmJoinPauseHold::default());
        let commit = GmJoinCommit {
            id: GmJoinId(8),
            kind: GmJoinKind::Reconnect,
            owner: HostSlot(1),
            candidate: reconnect_approval().candidate,
            tick: 84,
            digest: 0x1294,
        };

        commit_roster(&mut world, &commit).unwrap();

        assert_eq!(world.resource::<FleetRoster>(), &frozen);
        let session = world.resource::<crate::lockstep::FleetLockstep>();
        assert!(!session.has_departed(HostSlot(2)));
        assert_eq!(session.watermark_of(HostSlot(2)), Some(84));
        assert_eq!(session.peers().collect::<Vec<_>>(), vec![HostSlot(2)]);
    }

    #[test]
    fn reconnect_candidate_bootstrap_stays_private_and_keeps_the_existing_row() {
        let provisional = roster(HostSlot(2));
        let mut world = World::new();
        world.insert_resource(FleetRoster::default());
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        world.insert_resource(GmJoinPendingHostLoss::default());
        world.insert_resource(GmJoinPauseHold::default());

        prepare_candidate_bootstrap(&mut world, provisional.clone()).unwrap();
        assert!(!world.contains_resource::<FleetRoster>());
        assert!(!world.contains_resource::<crate::lockstep::FleetLockstep>());

        let commit = GmJoinCommit {
            id: GmJoinId(8),
            kind: GmJoinKind::Reconnect,
            owner: HostSlot(1),
            candidate: reconnect_approval().candidate,
            tick: 84,
            digest: 0x1294,
        };
        commit_roster(&mut world, &commit).unwrap();

        assert_eq!(world.resource::<FleetRoster>(), &provisional);
        assert_eq!(world.resource::<FleetRoster>().gms().len(), 1);
        assert_eq!(
            world.resource::<FleetRoster>().gm_operator(HostSlot(2)),
            Some("gm-1")
        );
    }

    #[test]
    fn reconnect_pause_waits_for_the_canonical_boot_identity_without_admitting_it() {
        let provisional = roster(HostSlot(2));
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.init_state::<crate::core::messages::GamePhase>();
        let world = app.world_mut();
        world.insert_resource(FleetRoster::default());
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        world.insert_resource(crate::lockstep::MeshRestoreArm::default());
        world.insert_resource(crate::lockstep::MeshSnapshotReceiver::default());
        world.insert_resource(GmJoinInbox::default());
        world.insert_resource(GmJoinRuntime::default());
        world.insert_resource(GmJoinPauseHold::default());
        world.insert_resource(GmJoinPendingHostLoss::default());
        prepare_candidate_bootstrap(world, provisional).unwrap();
        world
            .resource_mut::<GmJoinInbox>()
            .push(GmJoinFrame::Pause(reconnect_approval()));

        drive_join(world);

        assert!(matches!(
            world.resource::<NextState<crate::core::messages::GamePhase>>(),
            NextState::Unchanged
        ));
        assert!(world.resource::<GmJoinPauseHold>().active());
        let arm = world.resource::<crate::lockstep::MeshRestoreArm>();
        assert!(arm.is_armed());
        assert!(arm.bootstraps_join_candidate());
        assert!(!world.contains_resource::<FleetRoster>());
        assert!(!world.contains_resource::<crate::lockstep::FleetLockstep>());
    }

    #[test]
    fn first_time_pause_also_arms_the_canonical_game_start_bootstrap() {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.init_state::<crate::core::messages::GamePhase>();
        let world = app.world_mut();
        world.insert_resource(FleetRoster::default());
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        world.insert_resource(crate::lockstep::MeshRestoreArm::default());
        world.insert_resource(crate::lockstep::MeshSnapshotReceiver::default());
        world.insert_resource(GmJoinInbox::default());
        world.insert_resource(GmJoinRuntime::default());
        world.insert_resource(GmJoinPauseHold::default());
        world.insert_resource(GmJoinPendingHostLoss::default());
        prepare_candidate_bootstrap(world, first_time_candidate_roster()).unwrap();
        world
            .resource_mut::<GmJoinInbox>()
            .push(GmJoinFrame::Pause(approval()));

        drive_join(world);

        assert!(matches!(
            world.resource::<NextState<crate::core::messages::GamePhase>>(),
            NextState::Unchanged
        ));
        let arm = world.resource::<crate::lockstep::MeshRestoreArm>();
        assert!(arm.is_armed());
        assert!(arm.bootstraps_join_candidate());
        assert!(!world.contains_resource::<FleetRoster>());
        assert!(!world.contains_resource::<crate::lockstep::FleetLockstep>());
        assert!(
            !join_restore_clock_started(world),
            "the private candidate cannot spend its wait budget before GameStart is built"
        );
    }

    #[test]
    fn rejection_does_not_change_roster_or_pause_transaction() {
        let before = roster(HostSlot(1));
        let mut joins = GmJoinCoordinator::default();
        let _ = joins.reject(GmJoinId(7), GmJoinRefusal::UnknownApprover);

        assert_eq!(before.participants(), vec![HostSlot(1), HostSlot(2)]);
        assert_eq!(before.gms().len(), 1);
        assert!(matches!(
            joins.progress(),
            GmJoinProgress::Refused {
                id: GmJoinId(7),
                ..
            }
        ));
    }

    #[test]
    fn exact_approval_retry_schedules_one_pause_and_one_capture() {
        let roster = roster(HostSlot(1));
        let mut joins = GmJoinCoordinator::default();
        let approved = approval();

        assert_eq!(
            joins.approve(&roster, approved.clone()),
            Ok(approved.clone())
        );
        assert_eq!(joins.approve(&roster, approved.clone()), Ok(approved));
        assert_eq!(joins.pause_applied(GmJoinId(7), 42, 0xaaaa), Ok(true));
        assert_eq!(joins.pause_applied(GmJoinId(7), 42, 0xaaaa), Ok(false));
        assert_eq!(
            joins.pause_applied(GmJoinId(7), 42, 0xbbbb),
            Err(GmJoinRefusal::ConflictingRetry)
        );
    }

    #[test]
    fn begin_join_retry_reuses_original_pause_and_restore_boundary() {
        let mut world = World::new();
        world.insert_resource(roster(HostSlot(1)));
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        world.insert_resource(crate::sim_tick::SimTick(10));

        let first = begin_join(
            &mut world,
            GmJoinId(7),
            HostSlot(2),
            approval().candidate,
            "scenario-a",
        )
        .unwrap();
        assert_eq!(first.apply_tick, 11);
        assert!(matches!(
            world
                .resource_mut::<crate::lockstep::MeshOutbox>()
                .drain()
                .as_slice(),
            [crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Pause(_))]
        ));

        world.resource_mut::<crate::sim_tick::SimTick>().0 = 40;
        {
            let mut runtime = world.resource_mut::<GmJoinRuntime>();
            runtime.restore_boundary = 3;
            runtime.restore_boundary_reported = true;
        }
        let retry = begin_join(
            &mut world,
            GmJoinId(7),
            HostSlot(2),
            first.candidate.clone(),
            "scenario-b",
        )
        .unwrap();
        assert_eq!(retry, first, "the original pause boundary is retained");
        let runtime = world.resource::<GmJoinRuntime>();
        assert_eq!(runtime.restore_boundary, 3);
        assert!(runtime.restore_boundary_reported);
        assert_eq!(runtime.scenario, "scenario-a");
        assert!(matches!(
            world
                .resource_mut::<crate::lockstep::MeshOutbox>()
                .drain()
                .as_slice(),
            [crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Pause(approval))]
                if approval == &first
        ));

        let digest = 0xaaaa;
        let commit = {
            let mut runtime = world.resource_mut::<GmJoinRuntime>();
            runtime
                .coordinator
                .pause_applied(first.id, first.apply_tick, digest)
                .unwrap();
            runtime
                .coordinator
                .restored(first.id, first.candidate.host, digest)
                .unwrap()
        };
        world.resource_mut::<crate::sim_tick::SimTick>().0 = 80;
        assert_eq!(
            begin_join(
                &mut world,
                GmJoinId(7),
                HostSlot(2),
                first.candidate.clone(),
                "scenario-c",
            ),
            Ok(first.clone())
        );
        assert!(matches!(
            world
                .resource_mut::<crate::lockstep::MeshOutbox>()
                .drain()
                .as_slice(),
            [crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Committed(actual))]
                if actual == &commit
        ));
    }

    #[test]
    fn candidate_bootstrap_is_private_until_commit_installs_roster_and_wait_set() {
        let provisional = FleetRoster::with_participants_and_gms(
            vec![FleetShip::new(HostSlot(1))],
            vec![HostSlot(1), HostSlot(2), HostSlot(3)],
            vec![
                FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-1".into(),
                },
                FleetGm {
                    host: HostSlot(3),
                    operator_id: "gm-2".into(),
                },
            ],
            HostSlot(3),
            HostSlot(1),
        )
        .unwrap();
        let mut world = World::new();
        world.insert_resource(FleetRoster::default());
        world.insert_resource(crate::lockstep::MeshOutbox::default());

        prepare_candidate_bootstrap(&mut world, provisional.clone()).unwrap();
        prepare_candidate_bootstrap(&mut world, provisional.clone())
            .expect("an exact private bootstrap retry is inert");
        let conflicting = FleetRoster::with_participants_and_gms(
            provisional.ships().to_vec(),
            provisional.participants(),
            provisional.gms().to_vec(),
            provisional.local(),
            HostSlot(2),
        )
        .unwrap();
        assert_eq!(
            prepare_candidate_bootstrap(&mut world, conflicting),
            Err(GmJoinRefusal::ConflictingRetry),
            "a later same-candidate topology must not replace the accepted private bootstrap"
        );
        assert!(!world.contains_resource::<FleetRoster>());
        assert!(!world.contains_resource::<crate::lockstep::FleetLockstep>());
        assert!(world.contains_resource::<GmJoinBootstrap>());

        let commit = GmJoinCommit {
            id: GmJoinId(7),
            kind: GmJoinKind::FirstTime,
            owner: HostSlot(1),
            candidate: approval().candidate,
            tick: 42,
            digest: 0xaaaa,
        };
        commit_roster(&mut world, &commit).unwrap();
        assert_eq!(world.resource::<FleetRoster>().local(), HostSlot(3));
        assert!(world.resource::<FleetRoster>().is_member(HostSlot(3)));
        assert!(world.contains_resource::<crate::lockstep::FleetLockstep>());
        assert!(!world.contains_resource::<GmJoinBootstrap>());
    }

    #[test]
    fn authenticated_restore_failures_have_terminal_visible_reasons() {
        use crate::lockstep::MeshRestoreOutcome;

        assert_eq!(
            restore_refusal(&MeshRestoreOutcome::RefusedChunk("crc".into())),
            Some(GmJoinRefusal::TransferFailed)
        );
        for outcome in [
            MeshRestoreOutcome::RefusedGate("build".into()),
            MeshRestoreOutcome::RefusedIntegrity {
                recorded: 1,
                restored: 2,
            },
            MeshRestoreOutcome::Incomplete { tick: 42, gaps: 1 },
            MeshRestoreOutcome::RefusedUnarmed,
            MeshRestoreOutcome::RefusedWrongSender {
                armed_from: HostSlot(1),
                from: Some(HostSlot(2)),
            },
        ] {
            assert_eq!(
                restore_refusal(&outcome),
                Some(GmJoinRefusal::RestoreFailed)
            );
        }
        assert_eq!(restore_refusal(&MeshRestoreOutcome::NotReady), None);
    }

    #[test]
    fn render_updates_and_duplicate_reports_cannot_advance_restore_boundary() {
        let provisional = FleetRoster::with_participants_and_gms(
            vec![FleetShip::new(HostSlot(1))],
            vec![HostSlot(1), HostSlot(2), HostSlot(3)],
            vec![
                FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-1".into(),
                },
                FleetGm {
                    host: HostSlot(3),
                    operator_id: "gm-2".into(),
                },
            ],
            HostSlot(3),
            HostSlot(1),
        )
        .unwrap();
        let mut world = World::new();
        world.insert_resource(FleetRoster::default());
        prepare_candidate_bootstrap(&mut world, provisional.clone()).unwrap();
        world.insert_resource(crate::lockstep::MeshSnapshotReceiver::default());
        world.insert_resource(crate::lockstep::MeshRestoreArm::default());
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        // The candidate may not spend its wait boundary before the authored
        // GameStart walk completes, so this fixture starts past that gate.
        world.insert_resource(crate::server_app::GameStartEntityUuids::default());
        world.insert_resource(GmJoinPauseHold {
            active: true,
            resolved: false,
            resume_frontier: 0,
        });
        let mut runtime = GmJoinRuntime::default();
        runtime
            .coordinator
            .adopt_candidate(&provisional, approval())
            .unwrap();
        world.insert_resource(runtime);

        for _ in 0..1_000 {
            report_restored_join(&mut world);
        }
        let request = match world
            .resource_mut::<crate::lockstep::MeshOutbox>()
            .drain()
            .as_slice()
        {
            [crate::lockstep::MeshFrame::GmJoin(
                frame @ GmJoinFrame::RestoreBoundary {
                    from: HostSlot(3),
                    id: GmJoinId(7),
                    boundary: 0,
                },
            )] => frame.clone(),
            frames => panic!("render-only updates emitted {frames:?}"),
        };
        assert_eq!(world.resource::<GmJoinRuntime>().restore_boundary, 0);

        let mut owner = World::new();
        owner.insert_resource(roster(HostSlot(1)));
        owner.insert_resource(crate::lockstep::MeshOutbox::default());
        owner.insert_resource(GmJoinInbox::default());
        owner.insert_resource(GmJoinPauseHold {
            active: true,
            resolved: false,
            resume_frontier: 0,
        });
        let mut owner_runtime = GmJoinRuntime::default();
        owner_runtime
            .coordinator
            .approve(owner.resource::<FleetRoster>(), approval())
            .unwrap();
        owner_runtime
            .coordinator
            .pause_applied(GmJoinId(7), 42, 0xaaaa)
            .unwrap();
        owner.insert_resource(owner_runtime);

        for _ in 0..2 {
            owner.resource_mut::<GmJoinInbox>().push(request.clone());
            drive_join(&mut owner);
            assert!(matches!(
                owner
                    .resource_mut::<crate::lockstep::MeshOutbox>()
                    .drain()
                    .as_slice(),
                [crate::lockstep::MeshFrame::GmJoin(
                    GmJoinFrame::RestoreBoundary {
                        from: HostSlot(1),
                        id: GmJoinId(7),
                        boundary: 1,
                    }
                )]
            ));
        }
        assert_eq!(owner.resource::<GmJoinRuntime>().restore_boundary, 1);
    }

    #[test]
    fn only_the_terminal_owner_grant_permits_the_existing_rebuild_restore_path() {
        let provisional = FleetRoster::with_participants_and_gms(
            vec![FleetShip::new(HostSlot(1))],
            vec![HostSlot(1), HostSlot(2)],
            vec![FleetGm {
                host: HostSlot(2),
                operator_id: "gm-1".into(),
            }],
            HostSlot(2),
            HostSlot(1),
        )
        .unwrap();
        let mut world = World::new();
        world.insert_resource(FleetRoster::default());
        prepare_candidate_bootstrap(&mut world, provisional.clone()).unwrap();
        world.insert_resource(crate::lockstep::MeshSnapshotReceiver::default());
        let mut arm = crate::lockstep::MeshRestoreArm::default();
        arm.arm(HostSlot(1));
        world.insert_resource(arm);
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        world.insert_resource(GmJoinInbox::default());
        world.insert_resource(GmJoinPauseHold {
            active: true,
            resolved: false,
            resume_frontier: 0,
        });
        let mut runtime = GmJoinRuntime::default();
        runtime
            .coordinator
            .adopt_candidate(&provisional, reconnect_approval())
            .unwrap();
        runtime.restore_boundary = GM_JOIN_RESTORE_TIMEOUT_BOUNDARY - 1;
        world.insert_resource(runtime);

        assert!(!world
            .resource::<crate::lockstep::MeshRestoreArm>()
            .allows_rebuild());
        world
            .resource_mut::<GmJoinInbox>()
            .push(GmJoinFrame::RestoreBoundary {
                from: HostSlot(1),
                id: GmJoinId(8),
                boundary: GM_JOIN_RESTORE_TIMEOUT_BOUNDARY,
            });
        drive_join(&mut world);

        assert!(world
            .resource::<crate::lockstep::MeshRestoreArm>()
            .allows_rebuild());
        assert_eq!(
            world.resource::<GmJoinRuntime>().restore_boundary,
            GM_JOIN_RESTORE_TIMEOUT_BOUNDARY
        );
    }

    #[test]
    fn join_restore_clock_waits_for_the_authored_game_start_roster_walk() {
        let mut world = World::new();
        let reconnect = reconnect_approval();
        let first_time = approval();

        assert!(!join_restore_clock_started(&world));

        world.insert_resource(crate::server_app::GameStartEntityUuids::default());
        assert!(
            join_restore_clock_started(&world),
            "both join kinds start their budget only after every authored row was walked"
        );
        assert_eq!(reconnect.kind, GmJoinKind::Reconnect);
        assert_eq!(first_time.kind, GmJoinKind::FirstTime);
    }

    #[test]
    fn committed_retries_reemit_the_same_proof_and_a_later_join_can_start() {
        let roster = roster(HostSlot(1));
        let mut joins = GmJoinCoordinator::default();
        let first = approval();
        joins.approve(&roster, first.clone()).unwrap();
        joins.pause_applied(first.id, 42, 0xaaaa).unwrap();
        let commit = joins.restored(first.id, HostSlot(3), 0xaaaa).unwrap();

        assert_eq!(
            joins.restored(first.id, HostSlot(3), 0xaaaa),
            Ok(commit.clone())
        );
        assert_eq!(
            joins.restored(first.id, HostSlot(3), 0xbbbb),
            Err(GmJoinRefusal::ConflictingRetry)
        );
        assert_eq!(
            joins.reject(first.id, GmJoinRefusal::CandidateDisconnected),
            Err(GmJoinRefusal::ConflictingRetry)
        );
        assert_eq!(
            joins.progress(),
            &GmJoinProgress::Committed { commit },
            "a stale retry cannot regress a committed peer"
        );

        let mut second = first;
        second.id = GmJoinId(8);
        second.candidate.host = HostSlot(4);
        second.candidate.operator_id = "gm-3".into();
        second.transfer_id = 800;
        assert_eq!(joins.approve(&roster, second.clone()), Ok(second));
        assert!(matches!(
            joins.progress(),
            GmJoinProgress::AwaitingPause { .. }
        ));
    }

    #[test]
    fn owner_disconnect_refusal_resolves_once_and_cannot_regress_commit() {
        let mut world = World::new();
        world.insert_resource(roster(HostSlot(1)));
        world.insert_resource(crate::lockstep::MeshOutbox::default());
        world.insert_resource(GmJoinPauseHold {
            active: true,
            resolved: false,
            resume_frontier: 0,
        });
        let mut runtime = GmJoinRuntime::default();
        runtime
            .coordinator
            .approve(world.resource::<FleetRoster>(), approval())
            .unwrap();
        world.insert_resource(runtime);

        assert_eq!(
            refuse_join(
                &mut world,
                GmJoinId(7),
                GmJoinRefusal::CandidateDisconnected
            ),
            Ok(true)
        );
        assert!(world.resource::<GmJoinPauseHold>().resolved);
        assert!(matches!(
            world
                .resource::<crate::lockstep::MeshOutbox>()
                .pending_frames(),
            [crate::lockstep::MeshFrame::GmJoin(GmJoinFrame::Refused {
                reason: GmJoinRefusal::CandidateDisconnected,
                ..
            })]
        ));
        assert_eq!(
            refuse_join(
                &mut world,
                GmJoinId(7),
                GmJoinRefusal::CandidateDisconnected
            ),
            Ok(false)
        );
    }

    #[test]
    fn digest_mismatch_is_terminal_and_cannot_admit_the_candidate() {
        let before = roster(HostSlot(1));
        let mut joins = GmJoinCoordinator::default();
        joins.approve(&before, approval()).unwrap();
        joins.pause_applied(GmJoinId(7), 42, 0xaaaa).unwrap();

        assert_eq!(
            joins.restored(GmJoinId(7), HostSlot(3), 0xbbbb),
            Err(GmJoinRefusal::DigestMismatch {
                expected: 0xaaaa,
                restored: 0xbbbb,
            })
        );
        assert_eq!(before.participants(), vec![HostSlot(1), HostSlot(2)]);
        assert_eq!(before.gms().len(), 1);
    }

    #[test]
    fn matching_digest_is_the_only_path_to_roster_admission_on_every_peer() {
        let owner_before = roster(HostSlot(1));
        let member_before = roster(HostSlot(2));
        let mut joins = GmJoinCoordinator::default();
        joins.approve(&owner_before, approval()).unwrap();
        joins.pause_applied(GmJoinId(7), 42, 0xaaaa).unwrap();
        let commit = joins.restored(GmJoinId(7), HostSlot(3), 0xaaaa).unwrap();

        let owner_after = roster_after_commit(&owner_before, &commit, HostSlot(1)).unwrap();
        let member_after = roster_after_commit(&member_before, &commit, HostSlot(2)).unwrap();
        let candidate_after = roster_after_commit(&owner_before, &commit, HostSlot(3)).unwrap();
        for roster in [&owner_after, &member_after, &candidate_after] {
            assert_eq!(
                roster.participants(),
                vec![HostSlot(1), HostSlot(2), HostSlot(3)]
            );
            assert_eq!(roster.gm_operator(HostSlot(3)), Some("gm-2"));
            assert_eq!(roster.owner(), HostSlot(1));
        }
        assert_eq!(owner_after.local(), HostSlot(1));
        assert_eq!(member_after.local(), HostSlot(2));
        assert_eq!(candidate_after.local(), HostSlot(3));
    }

    #[test]
    fn technical_owner_does_not_have_to_be_the_peer_who_accepted() {
        let roster = roster(HostSlot(1));
        let mut joins = GmJoinCoordinator::default();
        let approved = joins.approve(&roster, approval()).unwrap();
        assert_eq!(approved.owner, HostSlot(1));
        assert_eq!(approved.approved_by, HostSlot(2));
    }

    fn resume_grant(sequence: u64, tick: u64) -> crate::gm_action::GmActionGrant {
        crate::gm_action::GmActionGrant {
            from: HostSlot(2),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: crate::gm_action::GmActionId::new(format!("resume-{sequence}")).unwrap(),
            recovery_generation: 0,
            apply_tick: tick,
            order: crate::gm_action::GmActionOrder::new(HostSlot(2), sequence),
            action: crate::gm_action::GmAction::SetSessionPaused { active: false },
        }
    }

    #[test]
    fn join_pause_ignores_an_early_resume_and_releases_only_on_a_new_explicit_resume() {
        let mut journal = crate::gm_action::GmActionJournal::default();
        let mut hold = GmJoinPauseHold::default();
        hold.engage(0);

        journal.insert(resume_grant(1, 42)).unwrap();
        journal.restore_applied_frontier(1).unwrap();
        assert!(
            hold.retain_until_explicit_resume(&journal),
            "an action applied before the admission verdict cannot release it"
        );

        hold.resolve(journal.applied_grants());
        assert!(hold.retain_until_explicit_resume(&journal));

        journal.insert(resume_grant(2, 43)).unwrap();
        journal.restore_applied_frontier(2).unwrap();
        assert!(
            !hold.retain_until_explicit_resume(&journal),
            "the first explicit Resume after the terminal admission result releases the hold"
        );
    }

    #[test]
    fn three_peers_admit_only_after_matching_digest_and_all_keep_the_join_pause() {
        let owner_before = roster(HostSlot(1));
        let member_before = roster(HostSlot(2));
        let candidate_before = FleetRoster::with_participants_and_gms(
            vec![FleetShip::new(HostSlot(1))],
            vec![HostSlot(1), HostSlot(2), HostSlot(3)],
            vec![
                FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-1".into(),
                },
                FleetGm {
                    host: HostSlot(3),
                    operator_id: "gm-2".into(),
                },
            ],
            HostSlot(3),
            HostSlot(1),
        )
        .unwrap();
        let approval = approval();
        let digest = 0x1293_aaaa;

        let mut owner = GmJoinCoordinator::default();
        let mut member = GmJoinCoordinator::default();
        let mut candidate = GmJoinCoordinator::default();
        owner.approve(&owner_before, approval.clone()).unwrap();
        member.approve(&member_before, approval.clone()).unwrap();
        candidate
            .adopt_candidate(&candidate_before, approval.clone())
            .unwrap();

        for joins in [&mut owner, &mut member, &mut candidate] {
            joins
                .pause_applied(approval.id, approval.apply_tick, digest)
                .unwrap();
        }
        // Pause/transfer progress is explicitly not public admission.
        assert!(!owner_before.is_member(HostSlot(3)));
        assert!(!member_before.is_member(HostSlot(3)));

        let owner_commit = owner.restored(approval.id, HostSlot(3), digest).unwrap();
        let member_commit = member.restored(approval.id, HostSlot(3), digest).unwrap();
        let candidate_commit = candidate
            .restored(approval.id, HostSlot(3), digest)
            .unwrap();
        assert_eq!(owner_commit, member_commit);
        assert_eq!(owner_commit, candidate_commit);

        let owner_after = roster_after_commit(&owner_before, &owner_commit, HostSlot(1)).unwrap();
        let member_after = roster_after_commit(&member_before, &owner_commit, HostSlot(2)).unwrap();
        let candidate_after =
            roster_after_commit(&candidate_before, &owner_commit, HostSlot(3)).unwrap();
        for after in [&owner_after, &member_after, &candidate_after] {
            assert!(after.is_member(HostSlot(3)));
            assert_eq!(after.gm_operator(HostSlot(3)), Some("gm-2"));
            assert_eq!(
                after.owner(),
                HostSlot(1),
                "owner sequences but does not lead"
            );
        }

        let mut holds = [
            GmJoinPauseHold::default(),
            GmJoinPauseHold::default(),
            GmJoinPauseHold::default(),
        ];
        let mut history = crate::gm_action::GmActionJournal::default();
        for hold in &mut holds {
            hold.engage(0);
            hold.resolve(0);
            assert!(hold.retain_until_explicit_resume(&history));
        }
        history
            .insert(resume_grant(1, approval.apply_tick))
            .unwrap();
        history.restore_applied_frontier(1).unwrap();
        for hold in &mut holds {
            assert!(
                !hold.retain_until_explicit_resume(&history),
                "every peer stays paused until the same explicit typed Resume"
            );
        }
    }
}
