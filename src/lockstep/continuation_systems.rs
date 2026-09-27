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
mod tests {
    use super::*;
    use crate::command_admission::HostSlot;
    fn world() -> World {
        let mut world = World::new();
        let slots = vec![HostSlot(1), HostSlot(2), HostSlot(3)];
        world.insert_resource(
            FleetRoster::with_participants(
                vec![super::super::FleetShip::new(HostSlot(1))],
                slots.clone(),
                HostSlot(2),
                HostSlot(1),
            )
            .unwrap(),
        );
        world.insert_resource(FleetLockstep(super::super::LockstepSession::new(
            HostSlot(2),
            slots,
            6,
        )));
        world.insert_resource(PendingHostLoss::default());
        world.insert_resource(crate::sim_tick::SimTick(5));
        world
    }
    fn begin() -> ContinuationRequest {
        ContinuationRequest::Begin {
            epoch: 2,
            previous_owner: HostSlot(1),
            next_owner: HostSlot(2),
            participants: vec![HostSlot(2), HostSlot(3)],
        }
    }
    fn commit(acked: Vec<HostSlot>) -> ContinuationRequest {
        ContinuationRequest::Commit {
            epoch: 2,
            previous_owner: HostSlot(1),
            next_owner: HostSlot(2),
            loss_tick: 11,
            acked,
        }
    }
    #[test]
    fn continuation_preserves_clock_roster_delay_and_recovered_frontier() {
        let mut world = world();
        let ships = world.resource::<FleetRoster>().ships().to_vec();
        assert_eq!(enqueue(&mut world, begin()).status, ContinuationPhase::Held);
        assert_eq!(world.resource::<FleetRoster>().owner(), HostSlot(1));
        world
            .resource_mut::<FleetLockstep>()
            .observe(HostSlot(1), 10);
        enqueue(&mut world, ContinuationRequest::Replayed { epoch: 2 });
        drain_after_mesh(&mut world);
        assert_eq!(
            world.resource::<OwnerContinuation>().status().status,
            ContinuationPhase::Replayed
        );
        enqueue(&mut world, commit(vec![HostSlot(3), HostSlot(2)]));
        drain_after_mesh(&mut world);
        assert!(!world.resource::<OwnerContinuation>().held());
        assert_eq!(world.resource::<FleetRoster>().owner(), HostSlot(2));
        assert_eq!(world.resource::<FleetRoster>().ships(), ships);
        assert_eq!(world.resource::<FleetLockstep>().delay(), 6);
        assert_eq!(world.resource::<crate::sim_tick::SimTick>().0, 5);
        assert!(world.resource::<FleetLockstep>().has_departed(HostSlot(1)));
        assert_eq!(
            world.resource::<PendingHostLoss>().agreed_tick(HostSlot(1)),
            Some(11)
        );
        enqueue(&mut world, commit(vec![HostSlot(2), HostSlot(3)]));
        drain_after_mesh(&mut world);
        assert_eq!(
            world.resource::<OwnerContinuation>().status().status,
            ContinuationPhase::Committed
        );
    }
    #[test]
    fn missing_survivor_ack_keeps_old_authority_and_hold() {
        let mut world = world();
        enqueue(&mut world, begin());
        world
            .resource_mut::<FleetLockstep>()
            .observe(HostSlot(1), 10);
        enqueue(&mut world, ContinuationRequest::Replayed { epoch: 2 });
        drain_after_mesh(&mut world);
        enqueue(&mut world, commit(vec![HostSlot(2)]));
        drain_after_mesh(&mut world);
        assert!(world.resource::<OwnerContinuation>().held());
        assert_eq!(world.resource::<FleetRoster>().owner(), HostSlot(1));
        assert!(!world.resource::<PendingHostLoss>().is_known(HostSlot(1)));
    }
    #[test]
    fn replacing_roster_or_committing_before_replay_is_refused() {
        let mut world = world();
        let mut bad = begin();
        if let ContinuationRequest::Begin { participants, .. } = &mut bad {
            participants.pop();
        }
        assert_eq!(enqueue(&mut world, bad).status, ContinuationPhase::Refused);
        enqueue(&mut world, begin());
        enqueue(&mut world, commit(vec![HostSlot(2), HostSlot(3)]));
        drain_after_mesh(&mut world);
        assert!(world.resource::<OwnerContinuation>().held());
        assert_eq!(world.resource::<FleetRoster>().owner(), HostSlot(1));
    }
    #[test]
    fn old_grants_ingest_before_commit_and_successor_keeps_the_journal_sequence() {
        use super::super::{FleetGm, MeshFrame, MeshInbox, MeshOrigin, TickFrame};
        use crate::gm_action::*;
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        super::super::register_lockstep(&mut app);
        app.init_resource::<crate::command_admission::log::PendingCommands>();
        let world = app.world_mut();
        world.insert_resource(
            FleetRoster::with_participants_and_gms(
                vec![],
                vec![HostSlot(1), HostSlot(2), HostSlot(3)],
                vec![FleetGm {
                    host: HostSlot(2),
                    operator_id: "survivor-gm".into(),
                }],
                HostSlot(2),
                HostSlot(1),
            )
            .unwrap(),
        );
        world.insert_resource(FleetLockstep(super::super::LockstepSession::new(
            HostSlot(2),
            vec![HostSlot(1), HostSlot(2), HostSlot(3)],
            6,
        )));
        world.insert_resource(crate::sim_tick::SimTick(5));
        enqueue(world, begin());
        let proposal = GmActionProposal {
            from: HostSlot(2),
            operator_id: "survivor-gm".into(),
            correlation: GmActionId::new("retained-pause").unwrap(),
            action: GmAction::SetSessionPaused { active: true },
        };
        let mut old_journal = GmActionJournal::default();
        let old_grant = sequence_owner_proposal(
            &mut old_journal,
            &proposal,
            HostSlot(1),
            5,
            11,
            false,
            false,
            false,
        )
        .unwrap();
        // The reliable replay authenticates through the previous owner's relay.
        for _ in 0..2 {
            assert!(enqueue_frame(
                world,
                2,
                HostSlot(1),
                MeshFrame::GmAction(GmActionFrame::Granted(old_grant.clone()))
            ));
        }
        world.resource_mut::<MeshInbox>().push_from(
            MeshFrame::Tick(TickFrame {
                from: HostSlot(1),
                tick: 4,
                ready_through: 10,
                commands: vec![],
                start_grant: None,
            }),
            MeshOrigin::Peer(HostSlot(1)),
        );
        enqueue(world, ContinuationRequest::Replayed { epoch: 2 });
        world
            .run_system_once(super::super::apply_mesh_inbox)
            .unwrap();
        drain_after_mesh(world);
        assert_eq!(
            world.resource::<OwnerContinuation>().status().loss_tick,
            Some(11)
        );
        assert_eq!(world.resource::<GmActionJournal>().next_sequence(), 2);
        let mut second = proposal.clone();
        second.correlation = GmActionId::new("successor-resume").unwrap();
        second.action = GmAction::SetSessionPaused { active: false };
        // Both a grant-covered and an ungranted proposal may be in the tail.
        assert!(enqueue_frame(
            world,
            2,
            HostSlot(2),
            MeshFrame::GmAction(GmActionFrame::Proposal(proposal.clone()))
        ));
        assert!(enqueue_frame(
            world,
            2,
            HostSlot(2),
            MeshFrame::GmAction(GmActionFrame::Proposal(second.clone()))
        ));
        assert_eq!(world.resource::<GmActionJournal>().next_sequence(), 2);
        enqueue(world, commit(vec![HostSlot(2), HostSlot(3)]));
        drain_after_mesh(world);
        assert_eq!(
            world.resource::<MeshInbox>().len(),
            1,
            "only the ungranted proposal is retried"
        );
        let old_late = sequence_owner_proposal(
            &mut old_journal,
            &second,
            HostSlot(1),
            5,
            11,
            false,
            false,
            false,
        )
        .unwrap();
        world.resource_mut::<MeshInbox>().push_from(
            MeshFrame::GmAction(GmActionFrame::Granted(old_late)),
            MeshOrigin::Peer(HostSlot(1)),
        );
        world
            .run_system_once(super::super::apply_mesh_inbox)
            .unwrap();
        assert_eq!(
            world.resource::<GmActionJournal>().next_sequence(),
            3,
            "successor sequences the deferred proposal once; old owner is fenced"
        );
        let next = sequence_owner_proposal(
            &mut world.resource_mut::<GmActionJournal>(),
            &second,
            HostSlot(2),
            5,
            11,
            false,
            false,
            false,
        )
        .unwrap();
        assert_eq!(next.order.sequence, 2);
        assert_eq!(next.sequenced_by, HostSlot(2));
        assert_eq!(world.resource::<GmActionJournal>().next_sequence(), 3);
    }
    #[test]
    fn continuation_hold_withholds_the_current_fixed_frame_without_reset() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = world();
        world.insert_resource(Time::<Virtual>::default());
        world.insert_resource(Time::<Fixed>::default());
        world.insert_resource(super::super::MeshDiagnostics::default());
        enqueue(&mut world, begin());
        world
            .run_system_once(super::super::gate_lockstep_ticks)
            .unwrap();
        assert!(world.resource::<Time<Virtual>>().is_paused());
        assert_eq!(world.resource::<crate::sim_tick::SimTick>().0, 5);
        assert_eq!(
            world.resource::<FleetLockstep>().watermark_of(HostSlot(1)),
            Some(6)
        );
    }
    #[test]
    fn an_invalid_retained_grant_cannot_be_blessed_by_a_replay_ack() {
        let mut world = world();
        enqueue(&mut world, begin());
        world
            .resource_mut::<OwnerContinuation>()
            .state
            .refuse("retained-gm-grant-refused");
        enqueue(&mut world, ContinuationRequest::Replayed { epoch: 2 });
        drain_after_mesh(&mut world);
        assert!(world.resource::<OwnerContinuation>().held());
        assert_eq!(
            world.resource::<OwnerContinuation>().status().status,
            ContinuationPhase::Refused
        );
        assert_eq!(world.resource::<FleetRoster>().owner(), HostSlot(1));
    }
    #[test]
    fn replay_ingress_requires_active_epoch_and_original_source() {
        use super::super::{MeshFrame, MeshInbox, TickFrame};
        let frame = || {
            MeshFrame::Tick(TickFrame {
                from: HostSlot(1),
                tick: 4,
                ready_through: 10,
                commands: vec![],
                start_grant: None,
            })
        };
        let mut world = world();
        world.init_resource::<MeshInbox>();
        assert!(!enqueue_frame(&mut world, 2, HostSlot(1), frame()));
        enqueue(&mut world, begin());
        assert!(!enqueue_frame(&mut world, 3, HostSlot(1), frame()));
        assert!(world.resource::<OwnerContinuation>().held());
        assert!(world.resource::<MeshInbox>().is_empty());
        assert!(!enqueue_frame(&mut world, 2, HostSlot(2), frame()));
        assert!(world.resource::<MeshInbox>().is_empty());
    }
}
