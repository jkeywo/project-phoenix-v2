use super::*;

/// Drive the actual adapter from each local viewpoint: four ship hosts and
/// two GM-only hosts share the same six folds, with only GM slot 5 divergent.
fn gm_divergence_plans(departed: Option<HostSlot>) -> Vec<recovery_plan::RecoveryPlan> {
    let participants: Vec<_> = (1..=6).map(HostSlot).collect();
    let mut plans = Vec::new();
    for &local in participants.iter().filter(|slot| Some(**slot) != departed) {
        let mut world = World::new();
        world.insert_resource(
            FleetRoster::with_participants(
                (1..=4)
                    .map(|slot| super::super::FleetShip::new(HostSlot(slot)))
                    .collect(),
                participants.clone(),
                local,
                HostSlot(1),
            )
            .expect("four ships and two GM-only participants"),
        );
        let mut session = super::super::LockstepSession::new(local, participants.clone(), 6);
        if let Some(slot) = departed {
            session.depart(slot);
        }
        world.insert_resource(FleetLockstep(session));
        world.init_resource::<RecoveryState>();
        world.init_resource::<MeshRestoreArm>();
        let mut agreement = MeshAgreement::new(300);
        for &slot in participants.iter().filter(|slot| Some(**slot) != departed) {
            let mut ledger = crate::sim_digest::DigestLedger::new(300);
            ledger.record(300, 0xAA);
            ledger.record(600, if slot == HostSlot(5) { 0xBB } else { 0xAA });
            if slot == local {
                agreement.local = ledger;
            } else {
                agreement.peers.insert(slot, ledger);
            }
        }
        world.insert_resource(agreement);
        begin_recovery(&mut world, local, 6);
        let Some(ActiveRecovery::InProgress { plan, role, .. }) =
            world.resource::<RecoveryState>().active.as_ref()
        else {
            panic!("GM divergence must open recovery on {local:?}");
        };
        assert_eq!(*role, plan.role_of(local));
        assert_eq!(plan.leader, HostSlot(1));
        assert_eq!(plan.recovering, vec![HostSlot(5)]);
        assert_eq!(plan.divergence_tick, 600);
        assert_eq!(plan.last_agreed_tick, Some(300));
        assert_eq!(plan.digests.len(), if departed.is_some() { 5 } else { 6 });
        if let Some(slot) = departed {
            assert!(!plan.digests.contains_key(&slot));
        }
        plans.push(plan.clone());
    }
    plans
}

#[test]
fn gm_only_divergence_opens_the_same_recovery_on_all_six_participants() {
    let plans = gm_divergence_plans(None);
    assert_eq!(plans.len(), 6);
    assert!(plans.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn divergence_electorate_excludes_a_departed_frozen_ship_without_its_digest() {
    let plans = gm_divergence_plans(Some(HostSlot(4)));
    assert_eq!(plans.len(), 5);
    assert!(plans.windows(2).all(|pair| pair[0] == pair[1]));
}

fn gm_grant(sequence: u64, apply_tick: u64) -> GmActionGrant {
    let from = HostSlot(2);
    GmActionGrant {
        from,
        sequenced_by: HostSlot(1),
        operator_id: "gm-recovery".into(),
        correlation: crate::gm_action::GmActionId::new(format!("gm-{sequence}"))
            .expect("valid correlation"),
        recovery_generation: 0,
        apply_tick,
        order: crate::gm_action::GmActionOrder::new(from, sequence),
        action: crate::gm_action::GmAction::SetSessionPaused {
            active: sequence % 2 == 1,
        },
    }
}

/// What an observer may be told about a recovery (issue #1437): that the
/// fleet is holding, where, and how many peers are restoring — never the
/// canonical digest, the elected leader or the command window.
#[test]
fn the_public_status_reports_the_hold_without_the_plan() {
    let mut state = RecoveryState::default();
    assert_eq!(state.status(), None);

    let plan = recovery_plan::RecoveryPlan {
        divergence_tick: 240,
        last_agreed_tick: Some(220),
        boundary_tick: 260,
        canonical_digest: 0xDEAD_BEEF,
        leader: HostSlot(1),
        recovering: vec![HostSlot(3)],
        digests: std::collections::BTreeMap::new(),
    };
    state.active = Some(ActiveRecovery::InProgress {
        plan: plan.clone(),
        role: RecoveryRole::Bystander,
        record_tick: None,
        resolved: false,
    });
    let status = state.status().expect("a recovery is in flight");
    assert_eq!(status.divergence_tick(), 240);
    assert_eq!(status.boundary_tick(), Some(260));
    assert_eq!(status.recovering(), [HostSlot(3)]);
    assert!(!status.resolved());
    assert!(!status.failed());

    // A leader record that would not gate: `finish_recovery` becomes terminal,
    // and the public status must still name the REAL divergence and boundary —
    // driven through the production path, not hand-built, because a hand-built
    // status cannot catch a placeholder written at the assignment site.
    let mut world = World::new();
    world.init_resource::<RecoveryState>();
    world.init_resource::<RecoveryLog>();
    world.init_resource::<MeshRestoreArm>();
    world.init_resource::<MeshAgreement>();
    world.init_resource::<CommandLog>();
    finish_recovery(
        &mut world,
        HostSlot(2),
        &plan,
        plan.boundary_tick,
        RecoveryResult::NoValidRecord {
            refusal: "the leader record would not gate".to_string(),
        },
    );
    let failed = world
        .resource::<RecoveryState>()
        .status()
        .expect("a failed recovery is still tracked");
    assert!(failed.failed());
    assert_eq!(
        failed.divergence_tick(),
        240,
        "the terminal warning names the divergence it is about, never tick 0"
    );
    assert_eq!(failed.boundary_tick(), Some(260));
    assert!(failed.recovering().is_empty());
}

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
        digests: vec![
            (HostSlot(1), 0xAA),
            (HostSlot(2), 0xBB),
            (HostSlot(3), 0xAA),
        ],
        leader: Some(HostSlot(1)),
        canonical_digest: Some(0xAA),
        recovering: vec![HostSlot(2)],
        command_window: Vec::new(),
        gm_action_window: Vec::new(),
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

    let mut pre_gm = future;
    pre_gm.version = 1;
    let text = export_recovery_artifact(&pre_gm).expect("serialises");
    assert!(
        parse_recovery_artifact(&text).is_err(),
        "a v1 diagnostic has no typed GM action window"
    );
}

#[test]
fn the_recovery_window_carries_canonical_gm_replay_input() {
    let mut world = World::new();
    let before = gm_grant(1, 120);
    let inside = gm_grant(2, 240);
    let after = gm_grant(3, 300);
    let mut journal = GmActionJournal::default();
    for grant in [before, inside.clone(), after] {
        journal.insert(grant).expect("canonical fixture");
    }
    journal.restore_applied_frontier(2).unwrap();
    world.insert_resource(journal);

    assert_eq!(gm_action_window(&world, Some(180), 240), vec![inside]);
}

#[test]
fn the_recovery_window_excludes_a_due_but_still_unapplied_gm_grant() {
    let mut world = World::new();
    let applied = gm_grant(1, 239);
    let exact_boundary_unapplied = gm_grant(2, 240);
    let mut journal = GmActionJournal::default();
    journal.insert(applied.clone()).expect("canonical fixture");
    journal
        .insert(exact_boundary_unapplied)
        .expect("canonical exact-boundary fixture");
    journal.restore_applied_frontier(1).unwrap();
    world.insert_resource(journal);

    assert_eq!(
        gm_action_window(&world, Some(180), 240),
        vec![applied],
        "recovery replay input is the durable applied prefix, not every due receipt"
    );
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
    assert_eq!(
        id,
        recovery_transfer_id(&plan.clone()),
        "same plan, same id"
    );
    let mut other = plan;
    other.divergence_tick = 250;
    assert_ne!(
        id,
        recovery_transfer_id(&other),
        "a different divergence is a different transfer"
    );
}
