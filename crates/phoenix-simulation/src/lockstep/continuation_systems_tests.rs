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
