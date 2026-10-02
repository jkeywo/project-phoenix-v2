//! Scheduled adapter for the pure owner continuation transaction.
use super::{continuation::*, FleetLockstep, FleetRoster, PendingHostLoss};
use bevy::prelude::*;

#[derive(Resource, Default, Debug)]
pub struct OwnerContinuation {
    pub state: Continuation,
    pending: Option<ContinuationRequest>,
    deferred_proposals: Vec<crate::gm_action::GmActionProposal>,
}
impl OwnerContinuation {
    pub fn held(&self) -> bool {
        self.state.held()
    }
    pub fn status(&self) -> &ContinuationStatus {
        &self.state.status
    }
}
pub fn not_held(continuation: Option<Res<OwnerContinuation>>) -> bool {
    continuation.is_none_or(|continuation| !continuation.held())
}

/// Called only by the authenticated transport adapter, never by a crew message.
/// Begin establishes the hold immediately; replay/commit are acknowledged only
/// after the ordinary mesh admission pass has ingested the retained tails.
pub fn enqueue(world: &mut World, request: ContinuationRequest) -> ContinuationStatus {
    world.init_resource::<OwnerContinuation>();
    let mut lane = world.remove_resource::<OwnerContinuation>().unwrap();
    lane.state.status.generation = lane.state.status.generation.saturating_add(1);
    let result = if lane.pending.is_some() {
        Err("continuation-operation-pending")
    } else if let ContinuationRequest::Begin {
        epoch,
        previous_owner,
        next_owner,
        participants,
    } = &request
    {
        match (
            world.get_resource::<FleetRoster>(),
            world.get_resource::<FleetLockstep>(),
        ) {
            (Some(roster), Some(session)) => {
                let live = roster
                    .participants()
                    .into_iter()
                    .filter(|slot| !session.has_departed(*slot))
                    .collect();
                lane.state.begin(
                    *epoch,
                    *previous_owner,
                    *next_owner,
                    participants.clone(),
                    roster.owner(),
                    roster.local(),
                    live,
                )
            }
            _ => Err("not-in-fleet"),
        }
    } else {
        lane.pending = Some(request);
        // Keep the phase Refused latched: a later replay must not bless bad tails.
        if lane.state.status.status != ContinuationPhase::Refused {
            lane.state.status.status = ContinuationPhase::Pending;
        }
        Ok(())
    };
    if let Err(reason) = result {
        lane.state.refuse(reason);
    }
    let status = lane.status().clone();
    world.insert_resource(lane);
    status
}

/// Replay a row retained from an authenticated transport stream. The adapter
/// supplies its ORIGINAL authenticated source, as ordinary mesh ingress does.
/// Unlike ordinary ingress, this lane also requires the active epoch and only
/// accepts tick/digest/host-loss/GM rows. Proposals wait until commit. It never grants relay authority.
/// The authenticated successor's commit acknowledgement set is checked by the
/// transport before enqueue; Rust additionally checks the exact frozen set.
pub fn enqueue_frame(
    world: &mut World,
    epoch: u64,
    source: crate::command_admission::HostSlot,
    frame: super::MeshFrame,
) -> bool {
    let active = world
        .get_resource::<OwnerContinuation>()
        .is_some_and(|lane| {
            lane.state.status.status != ContinuationPhase::Refused
                && lane
                    .state
                    .transaction
                    .as_ref()
                    .is_some_and(|tx| tx.epoch == epoch)
        });
    let supported = matches!(
        &frame,
        super::MeshFrame::Tick(_)
            | super::MeshFrame::Digest(_)
            | super::MeshFrame::HostLoss(_)
            | super::MeshFrame::GmAction(_)
    );
    let valid = active
        && supported
        && source == frame.from()
        && world
            .get_resource::<FleetRoster>()
            .is_some_and(|roster| roster.is_member(source));
    if !valid {
        refuse(world, "invalid-retained-frame");
        return false;
    }
    if let super::MeshFrame::GmAction(crate::gm_action::GmActionFrame::Proposal(proposal)) = frame {
        let mut lane = world.resource_mut::<OwnerContinuation>();
        if lane.deferred_proposals.len() >= 4096 {
            lane.state.refuse("retained-proposal-capacity");
            return false;
        }
        lane.deferred_proposals.push(proposal);
    } else {
        world
            .resource_mut::<super::MeshInbox>()
            .push_from(frame, super::MeshOrigin::Peer(source));
    }
    true
}
/// Preserve the hold when a retained row cannot be decoded or admitted.
pub fn refuse(world: &mut World, reason: &str) {
    if let Some(mut lane) = world
        .get_resource_mut::<OwnerContinuation>()
        .filter(|lane| lane.held())
    {
        lane.state.refuse(reason);
    }
}

pub fn drain_after_mesh(world: &mut World) {
    let Some(mut lane) = world.remove_resource::<OwnerContinuation>() else {
        return;
    };
    let Some(request) = lane.pending.take() else {
        world.insert_resource(lane);
        return;
    };
    let result = match request {
        ContinuationRequest::Replayed { epoch } => {
            let loss_tick = lane.state.transaction.as_ref().and_then(|tx| {
                world
                    .get_resource::<FleetLockstep>()?
                    .watermark_of(tx.previous_owner)?
                    .checked_add(1)
            });
            if let Some(loss_tick) = loss_tick {
                let result = lane.state.replayed(epoch);
                if result.is_ok() {
                    lane.state.status.loss_tick = Some(loss_tick);
                }
                result
            } else {
                Err("missing-owner-frontier")
            }
        }
        ContinuationRequest::Commit {
            epoch,
            previous_owner,
            next_owner,
            loss_tick,
            acked,
        } => {
            let duplicate = lane.state.committed.as_ref().is_some_and(|tx| {
                tx.epoch == epoch
                    && tx.previous_owner == previous_owner
                    && tx.next_owner == next_owner
                    && lane.state.committed_loss_tick == Some(loss_tick)
            });
            if duplicate && !lane.held() {
                lane.state.status.status = ContinuationPhase::Committed;
                Ok(())
            } else {
                let now = world
                    .get_resource::<crate::sim_tick::SimTick>()
                    .map_or(0, |tick| tick.0);
                let watermark = world
                    .get_resource::<FleetLockstep>()
                    .and_then(|session| session.watermark_of(previous_owner));
                let valid = lane.state.validate_commit(
                    epoch,
                    previous_owner,
                    next_owner,
                    loss_tick,
                    &acked,
                    watermark,
                    now,
                );
                if valid.is_ok() {
                    let roster = world.get_resource::<FleetRoster>();
                    if roster.is_none_or(|roster| {
                        roster.owner() != previous_owner || !roster.is_member(next_owner)
                    }) {
                        lane.state.refuse("owner-changed-during-continuation");
                    } else {
                        world
                            .resource_mut::<FleetRoster>()
                            .transfer_owner(previous_owner, next_owner);
                        world.resource_mut::<FleetLockstep>().depart(previous_owner);
                        world
                            .resource_mut::<PendingHostLoss>()
                            .observe(previous_owner, loss_tick);
                        lane.state.commit(loss_tick);
                        // All old-owner grants are now in the journal. Retry only
                        // proposals still ungranted, under the successor's normal
                        // authorization and sequencing on the next inbox pass.
                        for proposal in lane.deferred_proposals.drain(..) {
                            let granted = world
                                .get_resource::<crate::gm_action::GmActionJournal>()
                                .is_some_and(|journal| {
                                    journal
                                        .grant_for(&proposal.operator_id, &proposal.correlation)
                                        .is_some()
                                });
                            if !granted {
                                let source = proposal.from;
                                world.resource_mut::<super::MeshInbox>().push_from(
                                    super::MeshFrame::GmAction(
                                        crate::gm_action::GmActionFrame::Proposal(proposal),
                                    ),
                                    super::MeshOrigin::Peer(source),
                                );
                            }
                        }
                    }
                }
                valid
            }
        }
        ContinuationRequest::Begin { .. } => unreachable!("begin is synchronous"),
    };
    if let Err(reason) = result {
        lane.state.refuse(reason);
    }
    world.insert_resource(lane);
}

#[cfg(test)]
#[path = "continuation_systems_tests.rs"]
mod tests;
