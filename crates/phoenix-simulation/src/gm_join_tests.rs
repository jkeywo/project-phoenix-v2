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
    let mut agreement = crate::lockstep::MeshAgreement::new(60);
    agreement.local.record(60, 123);
    agreement
        .disagreements
        .push(crate::lockstep::MeshDisagreement {
            tick: 60,
            peer: HostSlot(2),
            local_digest: 123,
            peer_digest: 456,
        });
    world.insert_resource(agreement);
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
    let agreement = world.resource::<crate::lockstep::MeshAgreement>();
    assert_eq!(agreement.local.interval, 60);
    assert_eq!(agreement.local.digest_at(60), Some(123));
    assert_eq!(
        agreement.disagreements.len(),
        1,
        "joining cannot erase an existing disagreement"
    );
}

#[test]
fn retained_candidate_commit_rebases_live_frontier_without_resurrecting_departed_peers() {
    let retained = first_time_candidate_roster();
    let mut session =
        crate::lockstep::LockstepSession::new_at(HostSlot(3), retained.participants(), 6, 7)
            .unwrap();
    session.observe(HostSlot(1), 1200);
    session.depart(HostSlot(2));
    let mut world = World::new();
    world.insert_resource(retained.clone());
    world.insert_resource(crate::lockstep::FleetLockstep(session));
    world.insert_resource(GmJoinPauseHold::default());
    let commit = GmJoinCommit {
        id: GmJoinId(8),
        kind: GmJoinKind::Reconnect,
        owner: HostSlot(1),
        candidate: GmJoinCandidate {
            host: HostSlot(3),
            operator_id: "gm-2".into(),
        },
        tick: 84,
        digest: 0x1294,
    };
    commit_roster(&mut world, &commit).unwrap();
    assert_eq!(world.resource::<FleetRoster>(), &retained);
    let session = world.resource::<crate::lockstep::FleetLockstep>();
    assert_eq!(session.local(), HostSlot(3));
    assert_eq!(session.delay(), 6);
    assert_eq!(session.watermark_of(HostSlot(1)), Some(90));
    assert_eq!(session.watermark_of(HostSlot(2)), None);
    assert!(session.has_departed(HostSlot(2)));
    assert_eq!(session.peers().collect::<Vec<_>>(), vec![HostSlot(1)]);
}

#[test]
fn reconnect_candidate_bootstrap_stays_private_and_keeps_the_existing_row() {
    let provisional = roster(HostSlot(2));
    let mut world = World::new();
    world.insert_resource(FleetRoster::default());
    world.insert_resource(crate::lockstep::MeshOutbox::default());
    world.insert_resource(GmJoinPendingHostLoss::default());
    world.insert_resource(GmJoinPauseHold::default());
    world.init_resource::<crate::lockstep::MeshAgreement>();

    prepare_candidate_bootstrap(&mut world, provisional.clone()).unwrap();
    assert!(!world.contains_resource::<FleetRoster>());
    assert!(!world.contains_resource::<crate::lockstep::FleetLockstep>());
    assert_eq!(
        world
            .resource::<crate::lockstep::MeshAgreement>()
            .local
            .interval,
        0
    );

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
    assert_eq!(
        world
            .resource::<crate::lockstep::MeshAgreement>()
            .local
            .interval,
        crate::lockstep::DIGEST_INTERVAL_TICKS
    );
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
fn admitted_reconnect_pause_rejects_wrong_identity_owner_and_first_time() {
    for invalid in ["operator", "owner", "approver", "first-time"] {
        let mut world = World::new();
        let retained = roster(HostSlot(2));
        world.insert_resource(retained.clone());
        world.insert_resource(crate::lockstep::FleetLockstep(
            crate::lockstep::LockstepSession::new_at(HostSlot(2), retained.participants(), 2, 7)
                .unwrap(),
        ));
        world.insert_resource(crate::lockstep::MeshRestoreArm::default());
        world.insert_resource(GmJoinPauseHold::default());
        let mut approval = reconnect_approval();
        match invalid {
            "operator" => approval.candidate.operator_id = "other-gm".into(),
            "owner" => approval.owner = HostSlot(9),
            "approver" => approval.approved_by = HostSlot(9),
            "first-time" => approval.kind = GmJoinKind::FirstTime,
            _ => unreachable!(),
        }
        let mut inbox = GmJoinInbox::default();
        inbox.push(GmJoinFrame::Pause(approval));
        world.insert_resource(inbox);
        drive_join(&mut world);
        assert!(
            !world
                .resource::<crate::lockstep::MeshRestoreArm>()
                .is_armed(),
            "{invalid}"
        );
        assert!(!world.resource::<GmJoinPauseHold>().active(), "{invalid}");
        assert_eq!(world.resource::<FleetRoster>(), &retained);
        assert!(world.contains_resource::<crate::lockstep::FleetLockstep>());
        assert!(!world.contains_resource::<GmJoinBootstrap>());
    }
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
    assert_eq!(
        world
            .resource::<crate::lockstep::MeshAgreement>()
            .local
            .interval,
        crate::lockstep::DIGEST_INTERVAL_TICKS
    );
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
