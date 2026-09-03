//! Transfer and restore the portable authoritative record between two hosts, and
//! prove post-restore agreement (issue #1117).
//!
//! The crown proof for #1117, and the transport-half companion to
//! `tests/snapshot_resume.rs` (which proves a save round-trips through a `Store`)
//! and `tests/lockstep_mesh.rs` (which proves two hosts lockstep). Here one host
//! captures the ONE canonical `PhoenixSnapshot`, frames and chunks it, ferries the
//! chunks over an in-process mesh, and a second, genuinely fresh host reassembles
//! them, runs the SAME version and content gate a local load runs, restores
//! through the SAME `snapshot::restore` walk, and then steps forward in lockstep
//! digest-agreement with the first.
//!
//! # What the mesh adds, and what it must not
//!
//! Everything here rides `crate::lockstep::transfer` (pure chunk/reassemble/
//! integrity) and `crate::lockstep::snapshot_relay` (capture, gate, restore). The
//! bytes chunked are `snapshot::export_artifact`'s and the bytes restored go
//! through `snapshot::import_artifact` + `snapshot::restore` — no second
//! serializer, which is AC5, and is true by construction because there is no state
//! walk in the transfer path to be a second one.
//!
//! # Why its own test binary
//!
//! `deterministic` pins the scheduler with a one-thread `TaskPoolOptions`, and
//! Bevy's task pools are process-global, fixed by whichever app builds first — so
//! a digest-agreement claim in a binary shared with other app-building tests is a
//! claim about whoever won that race. Same reason as `tests/snapshot_resume.rs`
//! and `tests/lockstep_mesh.rs`; building a headless app also populates the
//! process-global template cache, which AGENTS.md confines to integration tests.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::{App, Entity, Fixed, FixedUpdate, ResMut, Resource, Time, With};

use project_phoenix::command_admission::{
    CommandLog, CommandOrder, HostSlot, LoggedCommand, ShipKey,
};
use project_phoenix::content_ledger;
use project_phoenix::core::codec::{decode_mesh_frame, encode_mesh_frame};
use project_phoenix::core::messages::{SystemControlPayload, SystemId};
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::lockstep::snapshot_relay::{capture_join_run, drain_mesh_restore, frames_for};
use project_phoenix::lockstep::transfer::{Accepted, SnapshotChunk, TransferError};
use project_phoenix::lockstep::{
    gate_and_restore_against, send_snapshot, FleetGm, FleetLockstep, FleetRoster, FleetShip,
    HostLossFrame, LockstepSession, MeshFrame, MeshInbox, MeshOrigin, MeshOutbox, MeshRestoreArm,
    MeshRestoreOutcome, MeshSnapshotReceiver, PendingHostLoss,
};
use project_phoenix::server_app::LocalShip;
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_tick::SimTick;
use project_phoenix::snapshot::{
    self, capture, ready_to_restore, reconcile_world_layers, run_for, versions,
    LayerReconcileStatus, PhoenixSnapshot,
};
use vellum_save::Versions;

/// The duel arena: a fixed roster spawned at t=0, no streaming. The narrow world,
/// where a divergence is readable — two ships and nothing between the capture and
/// the restore but the ships themselves.
const DUEL: &str = "assets/worlds/duel.toml";

/// `tests/snapshot_resume.rs`'s seed and roster, on purpose: this file proves the
/// same restore the resume suite proves, reached over the mesh instead of a
/// `Store`, so it must not re-find a different world's quirks under a new seed.
const SEED: u64 = 862_2026;

/// Frames before the capture — past the auto-start and far enough that the ships
/// have closed, acquired and traded fire. A capture of a world at rest would
/// round-trip trivially; [`assert_capture_is_alive`] refuses one.
const CAPTURE_AT: u64 = 400;

/// Frames the two worlds are stepped together after the restore. The number
/// `tests/snapshot_resume.rs` measured for this world; the mesh path reaches it
/// too, because the payload it carries is byte-identical.
const CONTINUE_FOR: u64 = 120;

const SLOT_SENDER: HostSlot = HostSlot(1);
const TRANSFER_ID: u64 = 0x1117_0000_0000_0001;

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: DUEL.into(),
        side_a: vec!["cruiser".into()],
        side_b: vec!["destroyer".into()],
        max_ticks: 4_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn current_versions() -> Versions {
    versions(&content_ledger::frozen_or_live())
}

fn boot() -> App {
    let mut app = build_headless_app(&args()).expect("the world builds");
    app.finish();
    app.cleanup();
    app
}

fn step(app: &mut App, frames: u64) {
    for _ in 0..frames {
        app.update();
    }
}

/// A fresh receiving host, brought up until the captured world's layers exist and
/// its authored entities are spawned — the restore point. The same loop
/// `tests/snapshot_resume.rs` and `tests/native_host_snapshot.rs` use.
fn boot_to_restore_point(snapshot: &PhoenixSnapshot) -> App {
    let mut app = boot();
    for _ in 0..1_000 {
        app.update();
        match reconcile_world_layers(app.world_mut(), snapshot) {
            LayerReconcileStatus::Ready if ready_to_restore(app.world(), snapshot) => return app,
            LayerReconcileStatus::Failed(path) => {
                panic!("world-layer reconciliation failed at {path}")
            }
            LayerReconcileStatus::Ready | LayerReconcileStatus::Waiting => {}
        }
    }
    panic!("the fresh receiving host never reached the restore point");
}

/// The liveness guard: refuse a capture of a world at rest, or the agreement
/// below is agreement about two hulls coasting.
fn assert_capture_is_alive(payload: &PhoenixSnapshot) {
    assert!(!payload.entities.is_empty(), "the capture found no ships");
    let moving = payload.entities.iter().any(|e| {
        e.physics
            .is_some_and(|p| p[4] != 0.0 || p[6] != 0.0 || p[7] != 0.0)
    });
    assert!(moving, "no captured ship has any velocity");
    let damaged = payload.entities.iter().any(|e| {
        e.hull
            .as_ref()
            .is_some_and(|rows| rows.iter().any(|(_, current, max)| current < max))
    });
    assert!(
        damaged,
        "no captured ship has taken damage — nothing is shooting"
    );
}

/// Capture a live host and build the transfer frames for it, returning the
/// payload and its recorded digest alongside so a receiver can be booted to the
/// restore point and the agreement checked.
///
/// `versions_override` lets a fault test forge a record from a "different build"
/// without loading a second world; `None` uses this build's real versions.
fn capture_and_frame(
    live: &App,
    versions_override: Option<Versions>,
) -> (PhoenixSnapshot, u64, Vec<MeshFrame>) {
    let payload = capture(live.world());
    let digest = world_digest(live.world());
    let run = run_for(
        payload.clone(),
        digest,
        SEED,
        DUEL,
        versions_override.unwrap_or_else(current_versions),
    );
    let frames = frames_for(&run, SLOT_SENDER, TRANSFER_ID).expect("the record frames");
    (payload, digest, frames)
}

/// The [`SnapshotChunk`]s a frame vec carries, each re-encoded and decoded through
/// the REAL host-mesh wire codec on the way out — so the bytes a receiver feeds
/// its reassembler are the bytes the wire would actually deliver, not the
/// in-memory struct.
fn chunks_over_the_wire(frames: &[MeshFrame]) -> Vec<SnapshotChunk> {
    frames
        .iter()
        .map(|frame| {
            let encoded = encode_mesh_frame(frame).expect("a snapshot frame encodes");
            match decode_mesh_frame(&encoded).expect("and decodes") {
                MeshFrame::Snapshot(chunk) => chunk,
                other => panic!("a snapshot frame decoded as {other:?}"),
            }
        })
        .collect()
}

/// The uuid of the single ship a host projects its private per-console state to.
fn local_ship_uuid(app: &mut App) -> String {
    let mut q = app
        .world_mut()
        .query_filtered::<&EntityUuid, With<LocalShip>>();
    let uuids: Vec<String> = q.iter(app.world()).map(|u| u.0.clone()).collect();
    assert_eq!(
        uuids.len(),
        1,
        "a host projects exactly one ship's private consoles — {} carried LocalShip",
        uuids.len()
    );
    uuids.into_iter().next().unwrap()
}

/// Arm a receiving host to accept a restore from `leader` (issue #1118, AC2).
///
/// `drain_mesh_restore` commits a staged record only for an armed recovering host,
/// and only from the leader the recovery plan named — so every test that expects a
/// commit (or that expects the version/content gate to run at all) arms first, the
/// way the recovery driver arms the real recovering host.
fn arm_receiver(app: &mut App, leader: HostSlot) {
    app.world_mut().resource_mut::<MeshRestoreArm>().arm(leader);
}

/// A jumbled-but-complete delivery order that still covers every chunk once.
fn jumbled(len: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).rev().collect();
    if len > 2 {
        order.swap(0, len / 2);
    }
    order
}

fn historical_command(tick: u64, sequence: u64, target: &str) -> LoggedCommand {
    LoggedCommand {
        tick,
        order: CommandOrder::new(HostSlot(1), sequence),
        ship: ShipKey("history-ship".into()),
        target: SystemId(target.into()),
        payload: SystemControlPayload::SetRedAlert { active: true },
    }
}

fn existing_join_roster(local: HostSlot) -> FleetRoster {
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

fn candidate_join_roster() -> FleetRoster {
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

fn install_existing_join_peer(app: &mut App, local: HostSlot, tick: u64, delay: u64) {
    app.world_mut().insert_resource(existing_join_roster(local));
    app.world_mut().insert_resource(FleetLockstep(
        LockstepSession::new_at(local, [HostSlot(1), HostSlot(2)], delay, tick).unwrap(),
    ));
    app.world_mut()
        .insert_resource(project_phoenix::command_admission::CommandDelay(delay));
}

fn deliver(app: &mut App, frame: MeshFrame, authenticated: HostSlot) {
    app.world_mut()
        .resource_mut::<MeshInbox>()
        .push_from(frame, MeshOrigin::Peer(authenticated));
}

fn drain_mesh(app: &mut App) -> Vec<MeshFrame> {
    app.world_mut().resource_mut::<MeshOutbox>().drain()
}

// ── The headline ─────────────────────────────────────────────────────────────

/// #1293 history contract: the candidate receives both input families, not
/// merely a current-state snapshot. Command history rides `Run.commands`; GM
/// history rides `PhoenixSnapshot::gm_actions`. Both pass through the same
/// #1117 whole-record gate before the candidate can report its restored digest.
#[test]
fn a_join_record_restores_full_command_and_gm_history() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);

    let commands = vec![
        historical_command(CAPTURE_AT - 20, 1, "helm"),
        historical_command(CAPTURE_AT - 10, 2, "red-alert"),
    ];
    live.world_mut()
        .resource_mut::<CommandLog>()
        .replace_from_transfer(commands.clone());

    let mut gm_history = project_phoenix::gm_action::GmActionJournal::default();
    for (sequence, active) in [(1, true), (2, false)] {
        gm_history
            .insert(project_phoenix::gm_action::GmActionGrant {
                from: HostSlot(2),
                sequenced_by: HostSlot(1),
                operator_id: "gm-1".into(),
                correlation: project_phoenix::gm_action::GmActionId::new(format!(
                    "history-{sequence}"
                ))
                .unwrap(),
                recovery_generation: 0,
                apply_tick: CAPTURE_AT - 3 + sequence,
                order: project_phoenix::gm_action::GmActionOrder::new(HostSlot(2), sequence),
                action: project_phoenix::gm_action::GmAction::SetSessionPaused { active },
            })
            .unwrap();
    }
    gm_history.restore_applied_frontier(2).unwrap();
    live.world_mut().insert_resource(gm_history.clone());
    live.world_mut().insert_resource(gm_history.applied_log());
    live.world_mut()
        .insert_resource(project_phoenix::gm_action::SimulationPaused(false));

    let run = capture_join_run(live.world(), DUEL);
    assert_eq!(
        run.commands, commands,
        "the whole envelope carries commands"
    );
    assert_eq!(
        run.snapshot.as_ref().unwrap().state.gm_actions,
        gm_history,
        "the snapshot half carries the durable GM journal"
    );
    let payload = run.snapshot.as_ref().unwrap().state.clone();
    let frames = frames_for(&run, SLOT_SENDER, TRANSFER_ID + 93).unwrap();
    let chunks = chunks_over_the_wire(&frames);

    let mut candidate = boot_to_restore_point(&payload);
    candidate
        .world_mut()
        .resource_mut::<CommandLog>()
        .replace_from_transfer(vec![historical_command(1, 99, "sentinel")]);
    for chunk in &chunks {
        candidate
            .world_mut()
            .resource_mut::<MeshSnapshotReceiver>()
            .accept_chunk(chunk)
            .unwrap();
    }
    arm_receiver(&mut candidate, SLOT_SENDER);
    drain_mesh_restore(candidate.world_mut());

    assert!(matches!(
        candidate
            .world()
            .resource::<MeshSnapshotReceiver>()
            .last_outcome(),
        Some(MeshRestoreOutcome::Committed { .. })
    ));
    assert_eq!(
        candidate.world().resource::<CommandLog>().entries(),
        commands
    );
    assert_eq!(
        candidate
            .world()
            .resource::<project_phoenix::gm_action::GmActionJournal>(),
        &gm_history
    );
    assert_eq!(
        candidate
            .world()
            .resource::<project_phoenix::gm_action::GmActionLog>(),
        &gm_history.applied_log()
    );
}

/// #1293 end-to-end mesh transaction. Three independent Apps exchange only
/// typed `MeshFrame`s: visible acceptance schedules one pause, the existing
/// peers remain the sole roster members while the candidate restores the whole
/// record and histories, Commit installs the same third wait-set member, and a
/// separately admitted absolute Resume is the only release.
#[test]
fn first_time_gm_join_commits_three_apps_only_after_digest_then_typed_resume() {
    use project_phoenix::gm_action::{
        GmAction, GmActionId, GmActionJournal, GmActionRequest, GmActionSubmission,
        SimulationPaused,
    };
    use project_phoenix::gm_join::{
        begin_join, prepare_candidate_bootstrap, refuse_join, GmJoinCandidate, GmJoinFrame,
        GmJoinId, GmJoinPauseHold, GmJoinProgress, GmJoinRefusal, GmJoinRuntime,
    };
    use project_phoenix::gm_roster::{GmOperator, GmRoster};

    let mut owner = boot();
    let mut member = boot();
    let mut candidate = boot();
    step(&mut owner, 80);
    step(&mut member, 80);
    step(&mut candidate, 5);
    assert_eq!(world_digest(owner.world()), world_digest(member.world()));

    let boundary_base = owner.world().resource::<SimTick>().0;
    let delay = project_phoenix::lockstep::authored_delay(owner.world());
    install_existing_join_peer(&mut owner, HostSlot(1), boundary_base, delay);
    install_existing_join_peer(&mut member, HostSlot(2), boundary_base, delay);
    prepare_candidate_bootstrap(candidate.world_mut(), candidate_join_roster()).unwrap();
    assert!(candidate.world().get_resource::<FleetRoster>().is_none());
    assert!(candidate.world().get_resource::<FleetLockstep>().is_none());

    let commands = vec![
        historical_command(boundary_base - 20, 1, "helm"),
        historical_command(boundary_base - 10, 2, "red-alert"),
    ];
    let mut history = GmActionJournal::default();
    for (sequence, active) in [(1, true), (2, false)] {
        history
            .insert(project_phoenix::gm_action::GmActionGrant {
                from: HostSlot(2),
                sequenced_by: HostSlot(1),
                operator_id: "gm-1".into(),
                correlation: GmActionId::new(format!("join-history-{sequence}")).unwrap(),
                recovery_generation: 0,
                apply_tick: boundary_base - 4 + sequence,
                order: project_phoenix::gm_action::GmActionOrder::new(HostSlot(2), sequence),
                action: GmAction::SetSessionPaused { active },
            })
            .unwrap();
    }
    history.restore_applied_frontier(2).unwrap();
    for app in [&mut owner, &mut member] {
        app.world_mut()
            .resource_mut::<CommandLog>()
            .replace_from_transfer(commands.clone());
        app.world_mut().insert_resource(history.clone());
        app.world_mut().insert_resource(history.applied_log());
        app.world_mut().insert_resource(SimulationPaused(false));
        drain_mesh(app);
    }
    drain_mesh(&mut candidate);

    let approval = begin_join(
        owner.world_mut(),
        GmJoinId(1293),
        HostSlot(2),
        GmJoinCandidate {
            host: HostSlot(3),
            operator_id: "gm-2".into(),
        },
        DUEL,
    )
    .unwrap();
    let pause = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Pause(_))))
        .expect("acceptance emits one typed Pause");
    deliver(&mut member, pause.clone(), HostSlot(1));
    deliver(&mut candidate, pause, HostSlot(1));

    for app in [&mut owner, &mut member] {
        app.world_mut().resource_mut::<SimTick>().0 = approval.apply_tick;
        app.update();
        assert_eq!(app.world().resource::<SimTick>().0, approval.apply_tick);
        assert!(app.world().resource::<SimulationPaused>().0);
        assert!(app.world().resource::<GmJoinPauseHold>().active());
    }
    let candidate_before_restore = candidate.world().resource::<SimTick>().0;
    candidate.update();
    assert!(
        candidate.world().resource::<SimTick>().0 < approval.apply_tick,
        "the candidate must genuinely remain behind until the record restores"
    );
    assert!(candidate.world().resource::<SimTick>().0 >= candidate_before_restore);
    assert!(
        !candidate.world().resource::<GmJoinPauseHold>().active(),
        "a behind candidate has not crossed the owner's pause boundary locally"
    );
    assert!(!owner
        .world()
        .resource::<FleetRoster>()
        .is_member(HostSlot(3)));
    assert!(!member
        .world()
        .resource::<FleetRoster>()
        .is_member(HostSlot(3)));
    assert!(candidate.world().get_resource::<FleetRoster>().is_none());

    let owner_transfer = drain_mesh(&mut owner);
    assert_eq!(
        owner_transfer
            .iter()
            .filter(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Pause(_))))
            .count(),
        0,
        "the due boundary does not mint a second Pause"
    );
    let chunks: Vec<_> = owner_transfer
        .into_iter()
        .filter(|frame| matches!(frame, MeshFrame::Snapshot(_)))
        .collect();
    assert!(chunks.len() > 1, "the real record traverses chunk framing");
    for frame in chunks {
        deliver(&mut candidate, frame, HostSlot(1));
    }
    candidate.update();
    assert_eq!(
        candidate.world().resource::<SimTick>().0,
        approval.apply_tick
    );
    assert!(candidate.world().resource::<SimulationPaused>().0);
    assert!(
        candidate.world().resource::<GmJoinPauseHold>().active(),
        "successful restore engages the hold even without local tick traversal"
    );

    let restored = drain_mesh(&mut candidate)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Restored { .. })))
        .expect("matching restored digest is reported");
    deliver(&mut owner, restored.clone(), HostSlot(3));
    deliver(&mut member, restored, HostSlot(3));
    owner.update();
    member.update();
    assert!(!member
        .world()
        .resource::<FleetRoster>()
        .is_member(HostSlot(3)));

    // Rust has committed, but the browser has not polled that terminal status
    // yet. A transport-close callback queued in this gap must preserve and
    // re-project Commit instead of removing GmJoinRuntime through an early `?`.
    assert_eq!(
        refuse_join(
            owner.world_mut(),
            approval.id,
            GmJoinRefusal::CandidateDisconnected,
        ),
        Err(GmJoinRefusal::ConflictingRetry)
    );
    assert!(matches!(
        owner.world().resource::<GmJoinRuntime>().progress(),
        GmJoinProgress::Committed { commit }
            if commit.id == approval.id && commit.candidate.host == HostSlot(3)
    ));
    let terminal_frames = drain_mesh(&mut owner);
    assert_eq!(
        terminal_frames
            .iter()
            .filter(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Committed(_))))
            .count(),
        2,
        "the original Commit and the disconnect-race retry both preserve the same proof"
    );
    let commit = terminal_frames
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Committed(_))))
        .expect("owner emits digest-proven Commit");
    deliver(&mut member, commit.clone(), HostSlot(1));
    deliver(&mut candidate, commit, HostSlot(1));
    member.update();
    candidate.update();

    for app in [&owner, &member, &candidate] {
        let roster = app.world().resource::<FleetRoster>();
        assert_eq!(
            roster.participants(),
            vec![HostSlot(1), HostSlot(2), HostSlot(3)]
        );
        assert_eq!(roster.gm_operator(HostSlot(3)), Some("gm-2"));
        assert!(app
            .world()
            .resource::<FleetLockstep>()
            .peers()
            .any(|slot| slot == HostSlot(3) || roster.local() == HostSlot(3)));
        assert!(app.world().resource::<SimulationPaused>().0);
    }
    assert_eq!(
        candidate.world().resource::<CommandLog>().entries(),
        commands
    );
    assert_eq!(candidate.world().resource::<GmActionJournal>(), &history);

    for app in [&mut owner, &mut member, &mut candidate] {
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<SimTick>().0,
            approval.apply_tick,
            "no peer may spend a tick while the committed join hold is paused"
        );
    }

    let public_gms = GmRoster::try_new(vec![
        GmOperator::new("gm-1".into(), "One".into(), true),
        GmOperator::new("gm-2".into(), "Two".into(), true),
    ])
    .unwrap();
    for app in [&mut owner, &mut member, &mut candidate] {
        app.world_mut().insert_resource(public_gms.clone());
    }
    assert_eq!(
        project_phoenix::gm_action::submit_local(
            candidate.world_mut(),
            GmActionRequest {
                operator_id: "gm-2".into(),
                correlation: GmActionId::new("resume-after-gm-join").unwrap(),
                action: GmAction::SetSessionPaused { active: false },
            },
        ),
        Ok(GmActionSubmission::Pending)
    );
    let proposal = drain_mesh(&mut candidate)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmAction(_)))
        .expect("candidate emits a typed Resume proposal");
    deliver(&mut owner, proposal.clone(), HostSlot(3));
    deliver(&mut member, proposal, HostSlot(3));
    owner.update();
    member.update();
    assert!(member.world().resource::<SimulationPaused>().0);

    let grant = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmAction(_)))
        .expect("owner sequences the typed Resume grant");
    deliver(&mut member, grant.clone(), HostSlot(1));
    deliver(&mut candidate, grant, HostSlot(1));
    member.update();
    candidate.update();
    for (index, app) in [&owner, &member, &candidate].into_iter().enumerate() {
        assert!(
            !app.world().resource::<SimulationPaused>().0,
            "peer {index} did not apply the typed Resume"
        );
        assert!(!app.world().resource::<GmJoinPauseHold>().active());
        assert!(matches!(
            app.world()
                .resource::<GmActionJournal>()
                .applied_prefix()
                .last()
                .map(|grant| &grant.action),
            Some(GmAction::SetSessionPaused { active: false })
        ));
    }
}

/// #1294 reconnects a known departed GM through the same owner-canonical
/// transfer as #1293. There are only two technical slots here: the sole
/// survivor and a deliberately stale returning peer, so no majority exists to
/// elect a record. The private candidate cannot enter agreement or the wait-set;
/// the survivor's typed transaction is the only canonical source.
#[test]
fn departed_gm_reconnect_restores_owner_history_and_digest_before_rejoin_and_resume() {
    use project_phoenix::ai::cadence::{AiBaseInterval, AiSnapshotReady, AiTickReady};
    use project_phoenix::gm_action::{
        GmAction, GmActionId, GmActionJournal, GmActionRequest, GmActionSubmission,
        SimulationPaused,
    };
    use project_phoenix::gm_join::{
        begin_reconnect, prepare_candidate_bootstrap, GmJoinCandidate, GmJoinFrame, GmJoinId,
        GmJoinKind, GmJoinPauseHold,
    };
    use project_phoenix::gm_roster::{GmOperator, GmRoster};

    let mut owner = boot();
    let mut returning = boot();
    step(&mut owner, 90);
    step(&mut returning, 7);
    // `boot()` is the ordinary headless ship profile. A real returning GM uses
    // BrowserGameMaster and therefore has no host-local ship projection; remove
    // that presentation/ownership marker while retaining the same authored
    // world that the private bootstrap and canonical restore overwrite.
    let stale_local_ships: Vec<Entity> = {
        let mut query = returning
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>();
        query.iter(returning.world()).collect()
    };
    for entity in stale_local_ships {
        returning
            .world_mut()
            .entity_mut(entity)
            .remove::<LocalShip>();
    }
    let stale_tick = returning.world().resource::<SimTick>().0;
    let stale_digest = world_digest(returning.world());
    assert_ne!(
        stale_digest,
        world_digest(owner.world()),
        "the returning process must genuinely begin from unequal stale state"
    );

    let boundary_base = owner.world().resource::<SimTick>().0;
    let delay = project_phoenix::lockstep::authored_delay(owner.world());
    install_existing_join_peer(&mut owner, HostSlot(1), boundary_base, delay);
    install_existing_join_peer(&mut returning, HostSlot(2), stale_tick, delay);
    owner
        .world_mut()
        .resource_mut::<FleetLockstep>()
        .depart(HostSlot(2));
    assert!(owner
        .world()
        .resource::<FleetLockstep>()
        .has_departed(HostSlot(2)));

    // A crashed/reloaded GM page may carry a stale local world, but it is not a
    // live mesh peer. Strip the obsolete wait-set and install only the private
    // bootstrap topology; no digest/election lane can see slot 2 before Commit.
    returning.world_mut().remove_resource::<FleetLockstep>();
    returning.world_mut().remove_resource::<FleetRoster>();
    prepare_candidate_bootstrap(returning.world_mut(), existing_join_roster(HostSlot(2))).unwrap();
    assert!(returning.world().get_resource::<FleetRoster>().is_none());
    assert!(returning.world().get_resource::<FleetLockstep>().is_none());

    let commands = vec![
        historical_command(boundary_base - 20, 1, "helm"),
        historical_command(boundary_base - 10, 2, "red-alert"),
    ];
    owner
        .world_mut()
        .resource_mut::<CommandLog>()
        .replace_from_transfer(commands.clone());
    returning
        .world_mut()
        .resource_mut::<CommandLog>()
        .replace_from_transfer(vec![historical_command(1, 99, "stale")]);

    let mut history = GmActionJournal::default();
    history
        .insert(project_phoenix::gm_action::GmActionGrant {
            from: HostSlot(2),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("reconnect-history-pause").unwrap(),
            recovery_generation: 0,
            apply_tick: boundary_base - 3,
            order: project_phoenix::gm_action::GmActionOrder::new(HostSlot(2), 1),
            action: GmAction::SetSessionPaused { active: true },
        })
        .unwrap();
    history
        .insert(project_phoenix::gm_action::GmActionGrant {
            from: HostSlot(2),
            sequenced_by: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("reconnect-history-resume").unwrap(),
            recovery_generation: 0,
            apply_tick: boundary_base - 2,
            order: project_phoenix::gm_action::GmActionOrder::new(HostSlot(2), 2),
            action: GmAction::SetSessionPaused { active: false },
        })
        .unwrap();
    history.restore_applied_frontier(2).unwrap();
    owner.world_mut().insert_resource(history.clone());
    owner.world_mut().insert_resource(history.applied_log());
    owner.world_mut().insert_resource(SimulationPaused(false));
    drain_mesh(&mut owner);
    drain_mesh(&mut returning);

    let approval = begin_reconnect(
        owner.world_mut(),
        GmJoinId(1294),
        GmJoinCandidate {
            host: HostSlot(2),
            operator_id: "gm-1".into(),
        },
        DUEL,
    )
    .unwrap();
    assert_eq!(approval.kind, GmJoinKind::Reconnect);
    assert_eq!(approval.owner, HostSlot(1));
    assert_eq!(approval.approved_by, HostSlot(1));
    assert_eq!(approval.transfer_id >> 48, 0x1294);
    let pause = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Pause(_))))
        .expect("the owner emits the reconnect Pause");
    deliver(&mut returning, pause, HostSlot(1));

    owner.world_mut().resource_mut::<SimTick>().0 = approval.apply_tick;
    owner.update();
    returning.update();
    assert!(owner.world().resource::<GmJoinPauseHold>().active());
    assert!(returning.world().get_resource::<FleetRoster>().is_none());
    assert!(returning.world().get_resource::<FleetLockstep>().is_none());
    assert_ne!(
        world_digest(returning.world()),
        world_digest(owner.world()),
        "the private stale peer cannot become canonical merely by reaching Pause"
    );

    let transfer = drain_mesh(&mut owner);
    let chunks: Vec<_> = transfer
        .into_iter()
        .filter(|frame| matches!(frame, MeshFrame::Snapshot(_)))
        .collect();
    assert!(
        chunks.len() > 1,
        "the real whole record crosses the chunker"
    );
    for frame in chunks {
        deliver(&mut returning, frame, HostSlot(1));
    }
    returning.update();
    let restored = drain_mesh(&mut returning)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Restored { .. })))
        .expect("the restored stale peer proves the canonical digest");
    deliver(&mut owner, restored, HostSlot(2));
    owner.update();
    let commit = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Committed(_))))
        .expect("matching digest yields the reconnect Commit");

    let frozen_before = existing_join_roster(HostSlot(1));
    assert_eq!(owner.world().resource::<FleetRoster>(), &frozen_before);
    assert!(!owner
        .world()
        .resource::<FleetLockstep>()
        .has_departed(HostSlot(2)));
    assert_eq!(
        owner
            .world()
            .resource::<FleetLockstep>()
            .watermark_of(HostSlot(2)),
        Some(approval.apply_tick),
        "reconnect uses LockstepSession::rejoin at the proven boundary"
    );

    deliver(&mut returning, commit, HostSlot(1));
    returning.update();
    assert!(
        !returning.world().resource::<MeshRestoreArm>().is_armed(),
        "Commit must close the transaction-scoped whole-record permission"
    );
    assert_eq!(
        returning.world().resource::<FleetRoster>(),
        &existing_join_roster(HostSlot(2)),
        "the same row is restored rather than appended"
    );
    assert_eq!(
        returning.world().resource::<CommandLog>().entries(),
        commands
    );
    assert_eq!(returning.world().resource::<GmActionJournal>(), &history);
    assert_eq!(world_digest(owner.world()), world_digest(returning.world()));
    let cadence_state = |app: &App| {
        let snapshot = capture(app.world());
        (
            app.world().resource::<SimTick>().0,
            app.world().resource::<AiTickReady>().0,
            app.world().resource::<AiSnapshotReady>().0,
            app.world().resource::<AiBaseInterval>().0,
            snapshot.ai_policy_clock,
            snapshot.fixed_overstep_nanos,
            app.world().resource::<Time<Fixed>>().overstep().as_nanos(),
        )
    };
    assert_eq!(
        cadence_state(&owner),
        cadence_state(&returning),
        "Commit must leave the restored peer on the owner's exact next-decision boundary",
    );
    for app in [&owner, &returning] {
        assert!(app.world().resource::<SimulationPaused>().0);
        assert!(app.world().resource::<GmJoinPauseHold>().active());
    }

    let public_gms = GmRoster::try_new(vec![GmOperator::new(
        "gm-1".into(),
        "Returning".into(),
        true,
    )])
    .unwrap();
    for app in [&mut owner, &mut returning] {
        app.world_mut().insert_resource(public_gms.clone());
    }
    assert_eq!(
        project_phoenix::gm_action::submit_local(
            returning.world_mut(),
            GmActionRequest {
                operator_id: "gm-1".into(),
                correlation: GmActionId::new("resume-after-gm-reconnect").unwrap(),
                action: GmAction::SetSessionPaused { active: false },
            },
        ),
        Ok(GmActionSubmission::Pending)
    );
    let proposal = drain_mesh(&mut returning)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmAction(_)))
        .expect("the reconnected identity submits an explicit typed Resume");
    deliver(&mut owner, proposal, HostSlot(2));
    owner.update();
    let grant = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmAction(_)))
        .expect("the owner sequences Resume after reconnect Commit");
    deliver(&mut returning, grant, HostSlot(1));
    returning.update();
    for app in [&owner, &returning] {
        assert!(!app.world().resource::<SimulationPaused>().0);
        assert!(!app.world().resource::<GmJoinPauseHold>().active());
    }
    assert_eq!(
        cadence_state(&owner),
        cadence_state(&returning),
        "the explicit Resume must not re-phase the restored peer's AI cadence",
    );

    // Exchange genuine post-restore watermarks while both peers continue. The
    // two worlds must advance beyond the recovered boundary and remain folded
    // to the same digest.
    for round in 0..24 {
        let owner_frames = drain_mesh(&mut owner);
        let returning_frames = drain_mesh(&mut returning);
        for frame in owner_frames {
            deliver(&mut returning, frame, HostSlot(1));
        }
        for frame in returning_frames {
            deliver(&mut owner, frame, HostSlot(2));
        }
        owner.update();
        returning.update();
        if world_digest(owner.world()) != world_digest(returning.world()) {
            let owner_entities = capture(owner.world()).entities;
            let returning_entities = capture(returning.world()).entities;
            let (owner_entity, returning_entity) = owner_entities
                .iter()
                .zip(&returning_entities)
                .find(|(owner_entity, returning_entity)| owner_entity != returning_entity)
                .expect("an entity-scope digest split names an entity row");
            assert_eq!(
                owner_entity.physics, returning_entity.physics,
                "first divergent entity {} differs in physics",
                owner_entity.uuid,
            );
            assert_eq!(
                owner_entity.control, returning_entity.control,
                "first divergent entity {} differs in helm control",
                owner_entity.uuid,
            );
            assert_eq!(
                owner_entity.drive, returning_entity.drive,
                "first divergent entity {} differs in drive state",
                owner_entity.uuid,
            );
            assert_eq!(
                owner_entity.hull, returning_entity.hull,
                "first divergent entity {} differs in hull state",
                owner_entity.uuid,
            );
            assert_eq!(
                owner_entity.weapons, returning_entity.weapons,
                "first divergent entity {} differs in weapon state",
                owner_entity.uuid,
            );
            panic!(
                "first divergent entity {}:\nowner={owner_entity:#?}\nreturning={returning_entity:#?}",
                owner_entity.uuid,
            );
        }
        let returning_stages = project_phoenix::sim_digest::digest_stages(returning.world());
        assert_eq!(
            world_digest(owner.world()),
            world_digest(returning.world()),
            "post-reconnect continuation diverged in round {round} at {:?}",
            project_phoenix::sim_digest::first_divergent_scope(owner.world(), &returning_stages,),
        );
    }
    assert!(owner.world().resource::<SimTick>().0 > approval.apply_tick);
    assert_eq!(
        owner.world().resource::<SimTick>().0,
        returning.world().resource::<SimTick>().0
    );
}

/// A reconnect uses #1118's rollback-safe receiver, not a lighter in-place
/// overwrite. A terminal chunk fault therefore leaves the stale private world
/// untouched, keeps the frozen public row departed, and can never manufacture
/// either a roster or Commit for the candidate.
#[test]
fn corrupted_reconnect_record_rolls_back_without_admitting_the_candidate() {
    use project_phoenix::gm_join::{
        begin_reconnect, prepare_candidate_bootstrap, GmJoinCandidate, GmJoinFrame, GmJoinId,
        GmJoinRefusal,
    };

    let mut owner = boot();
    let mut returning = boot();
    step(&mut owner, 40);
    step(&mut returning, 5);
    let boundary_base = owner.world().resource::<SimTick>().0;
    let delay = project_phoenix::lockstep::authored_delay(owner.world());
    install_existing_join_peer(&mut owner, HostSlot(1), boundary_base, delay);
    owner
        .world_mut()
        .resource_mut::<FleetLockstep>()
        .depart(HostSlot(2));
    returning.world_mut().remove_resource::<FleetLockstep>();
    returning.world_mut().remove_resource::<FleetRoster>();
    prepare_candidate_bootstrap(returning.world_mut(), existing_join_roster(HostSlot(2))).unwrap();
    drain_mesh(&mut owner);
    drain_mesh(&mut returning);

    let approval = begin_reconnect(
        owner.world_mut(),
        GmJoinId(12_940),
        GmJoinCandidate {
            host: HostSlot(2),
            operator_id: "gm-1".into(),
        },
        DUEL,
    )
    .unwrap();
    let pause = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Pause(_))))
        .expect("reconnect emits Pause");
    deliver(&mut returning, pause, HostSlot(1));
    owner.world_mut().resource_mut::<SimTick>().0 = approval.apply_tick;
    owner.update();
    returning.update();

    // Hold the private stale process still while the receiver processes the
    // terminal fault, so any digest movement below can only be a partial
    // restore rather than an unrelated stale-world fixed step.
    returning
        .world_mut()
        .resource_mut::<Time<bevy::time::Virtual>>()
        .pause();
    let stale_before = world_digest(returning.world());
    let mut chunks: Vec<SnapshotChunk> = drain_mesh(&mut owner)
        .into_iter()
        .filter_map(|frame| match frame {
            MeshFrame::Snapshot(chunk) => Some(chunk),
            _ => None,
        })
        .collect();
    assert!(chunks.len() > 1);
    let victim = 1;
    let mut corrupted = chunks.remove(victim);
    let mut bytes = corrupted.text.into_bytes();
    let at = bytes.len() / 2;
    bytes[at] = bytes[at].wrapping_add(1);
    corrupted.text = String::from_utf8(bytes).expect("duel RON is ascii");
    for chunk in chunks {
        deliver(&mut returning, MeshFrame::Snapshot(chunk), HostSlot(1));
    }
    deliver(&mut returning, MeshFrame::Snapshot(corrupted), HostSlot(1));
    returning.update();

    let terminal = drain_mesh(&mut returning);
    assert!(terminal.iter().any(|frame| matches!(
        frame,
        MeshFrame::GmJoin(GmJoinFrame::Refused {
            id: GmJoinId(12_940),
            reason: GmJoinRefusal::TransferFailed,
            ..
        })
    )));
    assert!(!terminal
        .iter()
        .any(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Committed(_)))));
    assert_eq!(
        world_digest(returning.world()),
        stale_before,
        "the rollback-safe receiver must not partially overwrite stale state",
    );
    assert!(returning.world().get_resource::<FleetRoster>().is_none());
    assert!(returning.world().get_resource::<FleetLockstep>().is_none());
    assert!(owner
        .world()
        .resource::<FleetLockstep>()
        .has_departed(HostSlot(2)));
    assert_eq!(
        owner.world().resource::<FleetRoster>(),
        &existing_join_roster(HostSlot(1)),
    );
}

/// A peer can disappear after the owner's canonical record was captured but
/// before digest Commit. The private candidate must receive and authenticate
/// that already-agreed loss without gaining a roster early, then install a
/// wait-set with the lost peer already departed so typed Resume cannot stall.
#[test]
fn host_loss_during_join_transfer_is_staged_until_commit_and_does_not_stall_resume() {
    use project_phoenix::gm_action::{
        GmAction, GmActionId, GmActionJournal, GmActionRequest, GmActionSubmission,
        SimulationPaused,
    };
    use project_phoenix::gm_join::{
        begin_join, prepare_candidate_bootstrap, GmJoinCandidate, GmJoinFrame, GmJoinId,
        GmJoinPauseHold, GmJoinPendingHostLoss,
    };
    use project_phoenix::gm_roster::{GmOperator, GmRoster};

    let mut owner = boot();
    let mut lost_member = boot();
    let mut candidate = boot();
    step(&mut owner, 80);
    step(&mut lost_member, 80);
    step(&mut candidate, 5);
    let boundary_base = owner.world().resource::<SimTick>().0;
    let delay = project_phoenix::lockstep::authored_delay(owner.world());
    install_existing_join_peer(&mut owner, HostSlot(1), boundary_base, delay);
    install_existing_join_peer(&mut lost_member, HostSlot(2), boundary_base, delay);
    prepare_candidate_bootstrap(candidate.world_mut(), candidate_join_roster()).unwrap();
    let commands = vec![
        historical_command(boundary_base - 20, 1, "helm"),
        historical_command(boundary_base - 10, 2, "red-alert"),
    ];
    let mut history = GmActionJournal::default();
    for (sequence, active) in [(1, true), (2, false)] {
        history
            .insert(project_phoenix::gm_action::GmActionGrant {
                from: HostSlot(2),
                sequenced_by: HostSlot(1),
                operator_id: "gm-1".into(),
                correlation: GmActionId::new(format!("loss-history-{sequence}")).unwrap(),
                recovery_generation: 0,
                apply_tick: boundary_base - 4 + sequence,
                order: project_phoenix::gm_action::GmActionOrder::new(HostSlot(2), sequence),
                action: GmAction::SetSessionPaused { active },
            })
            .unwrap();
    }
    history.restore_applied_frontier(2).unwrap();
    for app in [&mut owner, &mut lost_member] {
        app.world_mut()
            .resource_mut::<CommandLog>()
            .replace_from_transfer(commands.clone());
        app.world_mut().insert_resource(history.clone());
        app.world_mut().insert_resource(history.applied_log());
        app.world_mut().insert_resource(SimulationPaused(false));
        drain_mesh(app);
    }
    drain_mesh(&mut candidate);

    let approval = begin_join(
        owner.world_mut(),
        GmJoinId(12_930),
        HostSlot(2),
        GmJoinCandidate {
            host: HostSlot(3),
            operator_id: "gm-2".into(),
        },
        DUEL,
    )
    .unwrap();
    let pause = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Pause(_))))
        .expect("acceptance emits Pause");
    deliver(&mut lost_member, pause.clone(), HostSlot(1));
    deliver(&mut candidate, pause, HostSlot(1));
    for app in [&mut owner, &mut lost_member] {
        app.world_mut().resource_mut::<SimTick>().0 = approval.apply_tick;
        app.update();
        assert!(app.world().resource::<SimulationPaused>().0);
    }
    let candidate_before_pause = candidate.world().resource::<SimTick>().0;
    candidate.update();
    assert!(candidate.world().resource::<SimTick>().0 < approval.apply_tick);
    assert!(candidate.world().resource::<SimTick>().0 >= candidate_before_pause);
    assert!(candidate.world().get_resource::<FleetRoster>().is_none());
    assert!(candidate.world().get_resource::<FleetLockstep>().is_none());

    // Capture has happened: the snapshot chunks are already in the owner's
    // outbox. Keep them aside while slot 2's transport loss is agreed.
    let chunks: Vec<_> = drain_mesh(&mut owner)
        .into_iter()
        .filter(|frame| matches!(frame, MeshFrame::Snapshot(_)))
        .collect();
    assert!(chunks.len() > 1);
    owner.world_mut().resource_mut::<MeshInbox>().push_from(
        MeshFrame::HostLoss(HostLossFrame {
            from: HostSlot(1),
            lost: HostSlot(2),
            tick: 0,
        }),
        MeshOrigin::LocalObservation,
    );
    owner.update();
    let agreed_loss = drain_mesh(&mut owner)
        .into_iter()
        .find_map(|frame| match frame {
            MeshFrame::HostLoss(loss) if loss.lost == HostSlot(2) => Some(loss),
            _ => None,
        })
        .expect("the owner normalises and relays the member loss");
    assert!(agreed_loss.tick > 0);
    assert!(owner
        .world()
        .resource::<FleetLockstep>()
        .has_departed(HostSlot(2)));

    // Reliable ordered relay delivers the already-enqueued snapshot chunks
    // before the later host-loss frame. Restore proves the captured boundary,
    // but the candidate remains private until the owner sees this proof.
    for frame in chunks {
        deliver(&mut candidate, frame, HostSlot(1));
    }
    candidate.update();
    let candidate_frames = drain_mesh(&mut candidate);
    let restored = candidate_frames
        .iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Restored { .. })))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "the candidate reports matching restored digest; frames={candidate_frames:?}; outcome={:?}",
                candidate
                    .world()
                    .resource::<project_phoenix::lockstep::MeshSnapshotReceiver>()
                    .last_outcome()
            )
        });
    let restored_digest = match &restored {
        MeshFrame::GmJoin(GmJoinFrame::Restored { digest, .. }) => *digest,
        _ => unreachable!(),
    };
    assert_eq!(
        world_digest(candidate.world()),
        restored_digest,
        "candidate-private loss staging must not contaminate snapshot digest proof"
    );
    let digest_before_loss = world_digest(candidate.world());

    // The candidate receives the later loss through the owner's authenticated
    // pending link. It stages the carried tick but remains absent from every
    // roster and wait-set until Commit.
    deliver(
        &mut candidate,
        MeshFrame::HostLoss(agreed_loss),
        HostSlot(1),
    );
    candidate.update();
    assert!(candidate.world().get_resource::<FleetRoster>().is_none());
    assert!(candidate.world().get_resource::<FleetLockstep>().is_none());
    assert_eq!(
        candidate
            .world()
            .resource::<GmJoinPendingHostLoss>()
            .agreed_tick(HostSlot(2)),
        Some(agreed_loss.tick),
        "the post-capture loss survives restore privately until Commit"
    );
    assert_eq!(
        candidate
            .world()
            .resource::<PendingHostLoss>()
            .agreed_tick(HostSlot(2)),
        None,
        "pre-Commit topology must remain outside authoritative state"
    );
    assert_eq!(
        world_digest(candidate.world()),
        digest_before_loss,
        "candidate-private staging cannot mutate the proven authoritative fold"
    );
    deliver(&mut owner, restored, HostSlot(3));
    owner.update();
    let commit = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Committed(_))))
        .expect("matching digest commits admission");
    deliver(&mut candidate, commit.clone(), HostSlot(1));
    deliver(&mut candidate, commit, HostSlot(1));
    candidate.update();

    assert!(candidate
        .world()
        .resource::<GmJoinPendingHostLoss>()
        .is_empty());
    assert_eq!(
        candidate
            .world()
            .resource::<PendingHostLoss>()
            .agreed_tick(HostSlot(2)),
        Some(agreed_loss.tick),
        "Commit drains the carried loss exactly once despite an exact Commit retry"
    );
    assert!(candidate
        .world()
        .resource::<FleetRoster>()
        .is_member(HostSlot(2)));
    assert!(candidate
        .world()
        .resource::<FleetLockstep>()
        .has_departed(HostSlot(2)));
    assert!(!candidate
        .world()
        .resource::<FleetLockstep>()
        .peers()
        .any(|slot| slot == HostSlot(2)));
    assert!(candidate.world().resource::<GmJoinPauseHold>().active());

    let public_gms = GmRoster::try_new(vec![
        GmOperator::new("gm-1".into(), "One".into(), false),
        GmOperator::new("gm-2".into(), "Two".into(), true),
    ])
    .unwrap();
    for app in [&mut owner, &mut candidate] {
        app.world_mut().insert_resource(public_gms.clone());
    }
    assert_eq!(
        project_phoenix::gm_action::submit_local(
            candidate.world_mut(),
            GmActionRequest {
                operator_id: "gm-2".into(),
                correlation: GmActionId::new("resume-after-transfer-loss").unwrap(),
                action: GmAction::SetSessionPaused { active: false },
            },
        ),
        Ok(GmActionSubmission::Pending)
    );
    let proposal = drain_mesh(&mut candidate)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmAction(_)))
        .expect("candidate emits typed Resume proposal");
    deliver(&mut owner, proposal, HostSlot(3));
    owner.update();
    let grant = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmAction(_)))
        .expect("owner sequences typed Resume");
    deliver(&mut candidate, grant, HostSlot(1));
    candidate.update();
    assert!(!owner.world().resource::<SimulationPaused>().0);
    assert!(!candidate.world().resource::<SimulationPaused>().0);

    // Keep only the two live peers exchanging their real typed mesh output. If
    // Commit accidentally retained slot 2 in the candidate's wait-set, these
    // Apps stop at its old watermark instead of reaching the carried loss tick.
    for _ in 0..16 {
        owner.update();
        candidate.update();
        let owner_frames = drain_mesh(&mut owner);
        let candidate_frames = drain_mesh(&mut candidate);
        for frame in owner_frames {
            deliver(&mut candidate, frame, HostSlot(1));
        }
        for frame in candidate_frames {
            deliver(&mut owner, frame, HostSlot(3));
        }
    }
    owner.update();
    candidate.update();
    for app in [&owner, &candidate] {
        assert!(
            app.world().resource::<SimTick>().0 > agreed_loss.tick,
            "a live peer stalled on the member lost during transfer"
        );
        let losses = app.world().resource::<PendingHostLoss>();
        assert_eq!(losses.records().len(), 1);
        assert_eq!(losses.records()[0].slot, HostSlot(2));
    }
    assert_eq!(
        owner.world().resource::<SimTick>().0,
        candidate.world().resource::<SimTick>().0
    );
    assert_eq!(
        owner.world().resource::<PendingHostLoss>().records(),
        candidate.world().resource::<PendingHostLoss>().records(),
        "both live wait-set members apply the same carried loss exactly once"
    );
}

fn stalled_join_timeout_boundary(candidate_updates_per_boundary: usize) -> u16 {
    use project_phoenix::gm_action::SimulationPaused;
    use project_phoenix::gm_join::{
        begin_join, prepare_candidate_bootstrap, GmJoinCandidate, GmJoinFrame, GmJoinId,
        GmJoinPauseHold, GmJoinProgress,
    };

    assert!(candidate_updates_per_boundary > 0);
    let mut owner = boot();
    let mut candidate = boot();
    step(&mut owner, 40);
    step(&mut candidate, 3);
    let base = owner.world().resource::<SimTick>().0;
    let delay = project_phoenix::lockstep::authored_delay(owner.world());
    install_existing_join_peer(&mut owner, HostSlot(1), base, delay);
    prepare_candidate_bootstrap(candidate.world_mut(), candidate_join_roster()).unwrap();
    drain_mesh(&mut owner);
    drain_mesh(&mut candidate);

    let approval = begin_join(
        owner.world_mut(),
        GmJoinId(9000 + u64::try_from(candidate_updates_per_boundary).unwrap()),
        HostSlot(2),
        GmJoinCandidate {
            host: HostSlot(3),
            operator_id: "gm-2".into(),
        },
        DUEL,
    )
    .unwrap();
    let pause = drain_mesh(&mut owner)
        .into_iter()
        .find(|frame| matches!(frame, MeshFrame::GmJoin(GmJoinFrame::Pause(_))))
        .unwrap();
    deliver(&mut candidate, pause, HostSlot(1));

    owner.world_mut().resource_mut::<SimTick>().0 = approval.apply_tick;
    owner.update();
    candidate.update();
    assert!(candidate.world().resource::<SimTick>().0 < approval.apply_tick);

    let chunks: Vec<_> = drain_mesh(&mut owner)
        .into_iter()
        .filter(|frame| matches!(frame, MeshFrame::Snapshot(_)))
        .collect();
    assert!(chunks.len() > 1);
    deliver(&mut candidate, chunks[0].clone(), HostSlot(1));
    candidate.update();

    for expected in 0..=project_phoenix::gm_join::GM_JOIN_RESTORE_TIMEOUT_BOUNDARY {
        for _ in 1..candidate_updates_per_boundary {
            candidate.update();
        }
        let requests: Vec<_> = drain_mesh(&mut candidate)
            .into_iter()
            .filter(|frame| {
                matches!(
                    frame,
                    MeshFrame::GmJoin(GmJoinFrame::RestoreBoundary { .. })
                )
            })
            .collect();
        assert_eq!(
            requests.len(),
            1,
            "render cadence must not mint extra protocol boundaries"
        );
        assert!(matches!(
            &requests[0],
            MeshFrame::GmJoin(GmJoinFrame::RestoreBoundary {
                from: HostSlot(3),
                id,
                boundary,
            }) if *id == approval.id && *boundary == expected
        ));
        deliver(&mut owner, requests[0].clone(), HostSlot(3));
        owner.update();

        let replies = drain_mesh(&mut owner);
        if expected == project_phoenix::gm_join::GM_JOIN_RESTORE_TIMEOUT_BOUNDARY {
            let refusal = replies
                .into_iter()
                .find(|frame| {
                    matches!(
                        frame,
                        MeshFrame::GmJoin(GmJoinFrame::Refused {
                            reason: project_phoenix::gm_join::GmJoinRefusal::RestoreTimedOut,
                            ..
                        })
                    )
                })
                .expect("the owner terminates at the authored protocol boundary");
            assert!(matches!(
                owner
                    .world()
                    .resource::<project_phoenix::gm_join::GmJoinRuntime>()
                    .progress(),
                GmJoinProgress::Refused {
                    reason: project_phoenix::gm_join::GmJoinRefusal::RestoreTimedOut,
                    ..
                }
            ));
            assert!(owner.world().resource::<SimulationPaused>().0);
            assert!(owner.world().resource::<GmJoinPauseHold>().active());
            deliver(&mut candidate, refusal, HostSlot(1));
            candidate.update();
            assert!(matches!(
                candidate
                    .world()
                    .resource::<project_phoenix::gm_join::GmJoinRuntime>()
                    .progress(),
                GmJoinProgress::Refused {
                    reason: project_phoenix::gm_join::GmJoinRefusal::RestoreTimedOut,
                    ..
                }
            ));
            return expected;
        }

        let grant = replies
            .into_iter()
            .find(|frame| {
                matches!(
                    frame,
                    MeshFrame::GmJoin(GmJoinFrame::RestoreBoundary {
                        from: HostSlot(1),
                        id,
                        boundary,
                    }) if *id == approval.id && *boundary == expected + 1
                )
            })
            .expect("owner grants exactly the next protocol boundary");
        deliver(&mut candidate, grant, HostSlot(1));
        candidate.update();
    }
    unreachable!("the bounded restore protocol must terminate")
}

#[test]
fn stalled_join_timeout_is_the_same_protocol_boundary_at_unequal_render_cadence() {
    let one_update = stalled_join_timeout_boundary(1);
    let seven_updates = stalled_join_timeout_boundary(7);
    assert_eq!(one_update, seven_updates);
    assert_eq!(
        one_update,
        project_phoenix::gm_join::GM_JOIN_RESTORE_TIMEOUT_BOUNDARY
    );
}

/// **AC1, AC3 and AC6.** One host captures, frames and chunks the record; a fresh
/// host reassembles it over the mesh, gates it, restores it, and folds to the same
/// digest — then both step forward and agree on every tick.
#[test]
fn a_record_transfers_between_hosts_and_the_two_agree_after_restore() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);
    assert_capture_is_alive(&capture(live.world()));

    let (payload, captured_digest, frames) = capture_and_frame(&live, None);
    assert!(
        frames.len() > 1,
        "the record must have CHUNKED — a single frame does not exercise \
         reassembly (got {} frame(s))",
        frames.len()
    );
    let chunks = chunks_over_the_wire(&frames);

    // A fresh receiving host, nothing shared with `live` but the scenario and the
    // seed. It must not already stand where the capture does, or the agreement
    // below would prove nothing about the transfer.
    let mut receiver = boot_to_restore_point(&payload);
    assert_ne!(
        world_digest(receiver.world()),
        captured_digest,
        "the bootstrapped receiver already folds to the capture's digest"
    );

    // Feed the chunks in a jumbled order through the real receiver resource. The
    // record is staged only once EVERY chunk has arrived and the whole payload
    // verified — a partial transfer commits nothing.
    for (i, &c) in jumbled(chunks.len()).iter().enumerate() {
        let mut rx = receiver.world_mut().resource_mut::<MeshSnapshotReceiver>();
        let outcome = rx.accept_chunk(&chunks[c]).expect("each chunk is accepted");
        let complete = matches!(outcome, Accepted::Complete(_));
        assert_eq!(
            complete,
            i == chunks.len() - 1,
            "the record must stage on the LAST chunk and no earlier"
        );
    }
    assert!(
        receiver
            .world()
            .resource::<MeshSnapshotReceiver>()
            .has_staged_record(),
        "a fully-arrived record must be staged for restore"
    );

    // The real restore system commits it: gate, reconcile, restore, and the
    // post-restore digest check. The receiver is armed for the sender, as the
    // recovery driver arms a designated recovering host (issue #1118, AC2).
    arm_receiver(&mut receiver, SLOT_SENDER);
    drain_mesh_restore(receiver.world_mut());
    let outcome = receiver
        .world()
        .resource::<MeshSnapshotReceiver>()
        .last_outcome()
        .cloned()
        .expect("the staged record was acted on");
    assert_eq!(
        outcome,
        MeshRestoreOutcome::Committed {
            tick: payload.tick,
            digest: captured_digest,
        },
        "the transferred record must gate and restore cleanly, folding to the \
         digest the capture recorded"
    );

    // AC6, the photograph: at the instant of restore the two hosts agree.
    assert_eq!(
        world_digest(receiver.world()),
        captured_digest,
        "the restored host stands exactly where the capture did"
    );

    // AC6, the moving picture: they step forward together and stay equal. An
    // equal digest at the instant of restore is a photograph; a cold state
    // machine is invisible in one, so the claim is a CONTINUATION.
    for frame in 1..=CONTINUE_FOR {
        live.update();
        receiver.update();
        assert_eq!(
            world_digest(receiver.world()),
            world_digest(live.world()),
            "the two hosts diverged {frame} frame(s) after the mesh restore"
        );
    }

    // Anti-vacuity: the run was doing something worth agreeing about.
    let ticks = receiver.world().resource::<SimTick>().0;
    assert!(
        ticks > CAPTURE_AT,
        "the receiver continued past the restore tick"
    );
}

#[derive(Resource, Default)]
struct RestoreFrameFixedSteps(u64);

fn count_restore_frame_fixed_steps(mut count: ResMut<RestoreFrameFixedSteps>) {
    count.0 += 1;
}

#[test]
fn an_armed_paused_mesh_restore_cannot_spend_the_restore_frames_delta() {
    use bevy::time::{Fixed, Time, TimeUpdateStrategy, Virtual};

    let mut live = boot();
    step(&mut live, 40);
    live.world_mut()
        .insert_resource(project_phoenix::gm_action::SimulationPaused(true));
    let mut gm_actions = project_phoenix::gm_action::GmActionJournal::default();
    gm_actions.adopt_initial_pause(true);
    live.world_mut().insert_resource(gm_actions);
    live.world_mut().resource_mut::<Time<Virtual>>().pause();
    let (payload, captured_digest, frames) = capture_and_frame(&live, None);
    assert!(payload.paused);
    let stored_overstep = std::time::Duration::from_nanos(
        payload
            .fixed_overstep_nanos
            .expect("a real host captures its fixed interpolation remainder"),
    );

    let mut receiver = boot_to_restore_point(&payload);
    receiver
        .init_resource::<RestoreFrameFixedSteps>()
        .add_systems(FixedUpdate, count_restore_frame_fixed_steps);
    for chunk in chunks_over_the_wire(&frames) {
        receiver
            .world_mut()
            .resource_mut::<MeshSnapshotReceiver>()
            .accept_chunk(&chunk)
            .expect("the paused record chunk is accepted");
    }
    assert!(receiver
        .world()
        .resource::<MeshSnapshotReceiver>()
        .has_staged_record());
    arm_receiver(&mut receiver, SLOT_SENDER);

    let period = receiver.world().resource::<Time<Fixed>>().timestep();
    receiver.insert_resource(TimeUpdateStrategy::ManualDuration(period * 5));
    receiver.update();

    assert_eq!(
        receiver
            .world()
            .resource::<MeshSnapshotReceiver>()
            .last_outcome(),
        Some(&MeshRestoreOutcome::Committed {
            tick: payload.tick,
            digest: captured_digest,
        })
    );
    assert_eq!(receiver.world().resource::<SimTick>().0, payload.tick);
    assert_eq!(
        receiver.world().resource::<RestoreFrameFixedSteps>().0,
        0,
        "the oversized current-frame delta must not enter FixedUpdate after restore"
    );
    assert_eq!(
        receiver.world().resource::<Time<Fixed>>().overstep(),
        {
            let remainder_nanos = stored_overstep.as_nanos() % period.as_nanos();
            std::time::Duration::new(
                u64::try_from(remainder_nanos / 1_000_000_000)
                    .expect("a fixed-step remainder fits Duration seconds"),
                (remainder_nanos % 1_000_000_000) as u32,
            )
        },
        "restore preserves only the captured interpolation remainder"
    );
}

/// **AC3, spelled out.** Every persisted dimension the issue names survives the
/// framing and chunking — the transfer drops nothing.
///
/// Read off the reassembled payload rather than trusting the round-trip: RNG
/// stream positions, minted-identity continuation, Rhai scheduled work and flags,
/// the mission clock, Comms state, and every ship's evidence store are all present
/// after a full transfer, exactly as they were in the capture.
#[test]
fn the_transfer_carries_every_persisted_dimension() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);

    let (payload, _digest, frames) = capture_and_frame(&live, None);
    let chunks = chunks_over_the_wire(&frames);

    // Reassemble through the pure receiver and re-parse the record, so the
    // assertions below are about the bytes that crossed the wire.
    let mut rx = project_phoenix::lockstep::SnapshotReceiver::new();
    let mut text = None;
    for &c in &jumbled(chunks.len()) {
        if let Accepted::Complete(t) = rx.accept(&chunks[c]).expect("accepts") {
            text = Some(t);
        }
    }
    let text = text.expect("the record reassembles");
    let reassembled = snapshot::import_artifact(&text, &current_versions())
        .expect("the reassembled record gates")
        .snapshot
        .expect("carries a snapshot")
        .state;

    assert_eq!(
        reassembled, payload,
        "the reassembled payload must equal the capture — the framing changed \
         nothing"
    );
    // And each named dimension is actually PRESENT (not merely equal to an empty
    // capture), so the equality above is a claim about a full record.
    assert!(
        reassembled.rng.is_some(),
        "RNG stream positions ride the transfer"
    );
    assert!(
        reassembled.mint.is_some(),
        "minted-identity continuation rides it"
    );
    // ScenarioState carries the Rhai scheduled work and flags, the mission clock,
    // and every ship's evidence store, so its presence is those dimensions
    // crossing. Comms rides its own field where the scenario opened a channel.
    assert!(
        reassembled.scenario.is_some(),
        "Rhai scheduled work, flags, the mission clock and the evidence store ride \
         it (ScenarioState)"
    );
    assert_eq!(
        reassembled.comms, payload.comms,
        "Comms state crosses the transfer exactly as captured"
    );
    assert!(
        reassembled.entities.iter().any(|e| e.hull.is_some()),
        "every ship's authoritative state rides the transfer"
    );
}

/// **AC1 and AC5, the sender seam.** `send_snapshot` captures, frames and queues
/// the whole record to the fleet outbox in one call — reusing `snapshot::capture`
/// and `export_artifact`, with no second serializer — and what it queues
/// reassembles and gates into the same record.
#[test]
fn send_snapshot_captures_and_queues_the_record_to_the_fleet_outbox() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);
    let payload = capture(live.world());

    let count = send_snapshot(live.world_mut(), SLOT_SENDER, TRANSFER_ID, DUEL)
        .expect("the record captures and frames");
    assert!(count > 0, "the sender queued no frames");

    // Everything the sender put on the wire, drained the way the transport polls
    // it, is snapshot chunks and nothing else.
    let frames = live.world_mut().resource_mut::<MeshOutbox>().drain();
    assert_eq!(
        frames.len(),
        count,
        "the outbox holds exactly what was queued"
    );
    let chunks: Vec<SnapshotChunk> = frames
        .iter()
        .map(|f| match f {
            MeshFrame::Snapshot(c) => c.clone(),
            other => panic!("send_snapshot queued a non-snapshot frame: {other:?}"),
        })
        .collect();

    // And it reassembles into the record the live world would have exported.
    let mut rx = project_phoenix::lockstep::SnapshotReceiver::new();
    let mut text = None;
    for chunk in &chunks {
        if let Accepted::Complete(t) = rx.accept(chunk).expect("accepts") {
            text = Some(t);
        }
    }
    let restored = snapshot::import_artifact(&text.expect("reassembles"), &current_versions())
        .expect("gates")
        .snapshot
        .expect("carries a snapshot")
        .state;
    assert_eq!(
        restored, payload,
        "what send_snapshot queued must reassemble to the captured record"
    );
}

// ── Fault injection ──────────────────────────────────────────────────────────

/// **AC1, the gap refusal.** A dropped chunk is named as a gap, not silently
/// awaited — and the record is never staged from an incomplete transfer.
#[test]
fn a_dropped_chunk_leaves_the_record_uncommitted_and_named() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);
    let (payload, _digest, frames) = capture_and_frame(&live, None);
    let chunks = chunks_over_the_wire(&frames);
    assert!(chunks.len() >= 2, "need at least two chunks to drop one");

    let mut receiver = boot_to_restore_point(&payload);
    let before = world_digest(receiver.world());

    // Deliver everything except the middle chunk.
    let dropped = chunks.len() / 2;
    for chunk in chunks.iter().filter(|c| c.seq as usize != dropped) {
        let mut rx = receiver.world_mut().resource_mut::<MeshSnapshotReceiver>();
        assert!(matches!(rx.accept_chunk(chunk), Ok(Accepted::More { .. })));
    }
    {
        let rx = receiver.world().resource::<MeshSnapshotReceiver>();
        assert!(
            !rx.has_staged_record(),
            "an incomplete transfer must not stage"
        );
        assert_eq!(
            rx.missing(),
            vec![dropped as u32],
            "the receiver names the missing chunk rather than hanging silently"
        );
    }
    // The restore system finds nothing to commit, and the world is untouched.
    drain_mesh_restore(receiver.world_mut());
    assert_eq!(
        world_digest(receiver.world()),
        before,
        "a gap must leave the receiving world exactly as it was"
    );

    // …and the transfer recovers: the missing chunk completes it.
    {
        let mut rx = receiver.world_mut().resource_mut::<MeshSnapshotReceiver>();
        let late = chunks.iter().find(|c| c.seq as usize == dropped).unwrap();
        assert!(matches!(rx.accept_chunk(late), Ok(Accepted::Complete(_))));
    }
    assert!(receiver
        .world()
        .resource::<MeshSnapshotReceiver>()
        .has_staged_record());
}

/// **AC1, the corruption refusal.** A chunk damaged in flight fails its own
/// checksum on arrival — distinct from a gap — and does not stage the record.
#[test]
fn a_corrupted_chunk_is_refused_and_does_not_commit() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);
    let (payload, _digest, frames) = capture_and_frame(&live, None);
    let mut chunks = chunks_over_the_wire(&frames);
    assert!(chunks.len() >= 2);

    // Flip a byte in a chunk WITHOUT updating its crc: the wire changed it.
    let victim = 1;
    let mut bytes = chunks[victim].text.clone().into_bytes();
    let at = bytes.len() / 2;
    bytes[at] = bytes[at].wrapping_add(1);
    chunks[victim].text = String::from_utf8(bytes).expect("duel RON is ascii");

    let mut receiver = boot_to_restore_point(&payload);
    let before = world_digest(receiver.world());

    // Deliver every intact chunk first, then the corrupt one, so it is the last
    // fault the receiver saw. Each intact chunk buffers (`More`); the corrupt one
    // is refused at its own checksum and never buffered, so the transfer stays a
    // chunk short and never stages.
    for chunk in chunks.iter().filter(|c| c.seq as usize != victim) {
        let mut rx = receiver.world_mut().resource_mut::<MeshSnapshotReceiver>();
        assert!(matches!(rx.accept_chunk(chunk), Ok(Accepted::More { .. })));
    }
    {
        let mut rx = receiver.world_mut().resource_mut::<MeshSnapshotReceiver>();
        assert_eq!(
            rx.accept_chunk(&chunks[victim]),
            Err(TransferError::ChunkCorrupt { seq: victim as u32 }),
            "a chunk damaged in flight is refused with a distinct error, not \
             mistaken for a gap"
        );
    }
    let rx = receiver.world().resource::<MeshSnapshotReceiver>();
    assert!(!rx.has_staged_record(), "a corrupt transfer must not stage");
    assert!(matches!(
        rx.last_fault(),
        Some(TransferError::ChunkCorrupt { .. })
    ));
    assert!(matches!(
        rx.last_outcome(),
        Some(MeshRestoreOutcome::RefusedChunk(_))
    ));
    assert!(
        !rx.is_receiving() && rx.missing().is_empty(),
        "a terminal chunk fault retires the poisoned partial transfer"
    );
    drain_mesh_restore(receiver.world_mut());
    assert_eq!(
        world_digest(receiver.world()),
        before,
        "a corrupt transfer must leave the receiving world untouched"
    );
}

/// **AC2.** A record from a different build/content is refused by the version and
/// content gate BEFORE any state is committed — the world is untouched.
///
/// The record here is captured from a live world exactly as the happy path is,
/// then stamped with a rules revision this build does not speak. The receiver
/// gates it against its own real versions and refuses, naming the dimension that
/// moved — never a half-restore.
#[test]
fn a_record_from_a_different_build_is_refused_before_it_commits() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);

    // A record whose rules dimension does not match this build's.
    let content = snapshot::content_digest(&content_ledger::frozen_or_live());
    let wrong = Versions::new(snapshot::SNAPSHOT_FORMAT, "0.0-not-this-build", content);
    let (payload, _digest, frames) = capture_and_frame(&live, Some(wrong));
    let chunks = chunks_over_the_wire(&frames);

    let mut receiver = boot_to_restore_point(&payload);
    let before = world_digest(receiver.world());

    let mut text = None;
    for chunk in &chunks {
        let mut rx = receiver.world_mut().resource_mut::<MeshSnapshotReceiver>();
        if let Ok(Accepted::Complete(t)) = rx.accept_chunk(chunk) {
            text = Some(t);
        }
    }
    assert!(
        text.is_some(),
        "the transfer itself is intact — only the gate refuses"
    );

    // Armed for the sender, so the drop is the version/content gate's doing rather
    // than the receiver arm's: this test is about the gate, not the arm.
    arm_receiver(&mut receiver, SLOT_SENDER);
    drain_mesh_restore(receiver.world_mut());
    let outcome = receiver
        .world()
        .resource::<MeshSnapshotReceiver>()
        .last_outcome()
        .cloned()
        .expect("the gate ran");
    match outcome {
        MeshRestoreOutcome::RefusedGate(why) => {
            assert!(
                why.contains("rule")
                    || why.contains("0.0-not-this-build")
                    || why.contains("simulation"),
                "the refusal must name the dimension that moved: {why}"
            );
        }
        other => panic!("a mismatched-build record must be refused, got {other:?}"),
    }
    assert_eq!(
        world_digest(receiver.world()),
        before,
        "a refused record must leave the receiving world exactly as it was — never \
         a half-restore"
    );
}

// ── Per-crew projection isolation (AC4) ──────────────────────────────────────

/// **AC4, honestly scoped to today's model.** Restoring every ship's knowledge
/// does not project one ship's private view to another crew.
///
/// The projection layer, not the record, is what gates what a crew sees: the
/// dossier and console builders run `With<LocalShip>`, and the projection-SCOPE
/// marker `LocalShip` — a host-local marker of "the ship this machine's crew is
/// aboard" — is *never* in the record (not a field of `PhoenixSnapshot` or
/// `EntityState`). The materialized dossier blackboard DOES cross, though:
/// `publish_dossier_blackboard` writes the `dossiers` channel into
/// `ShipSystemBlackboards`, which capture takes unfiltered. It is inert on the
/// receiver all the same — restore is by-uuid, so that channel lands only on the
/// same hull it left (which is not the receiver's local ship), the blackboards are
/// not folded into `world_digest`, and only `LocalShip`'s blackboards render. So a
/// restore cannot re-point what a crew sees — the guarantee #1117 makes is exactly
/// "restore does not itself widen projection".
///
/// The evidence STORE the transfer carries is world-global today (keyed by the
/// observed subject, not the observing ship): `EvidenceLog` on
/// `WorldContentRuntime`, restored whole as `ScenarioState::evidence`. Truly
/// per-ship evidence stores — keyed by the observer, crossing ships only through
/// Comms — are open PRD #1016's (`p2p-delta-per-ship-epistemics`,
/// `fields-epistemics.yaml:evidence-stores-per-ship`), not this issue's. So the
/// honest claims here are: the world-global store crosses the transfer verbatim,
/// and the restore leaves the projection scope exactly where it was.
#[test]
fn restoring_a_transferred_record_does_not_widen_which_crew_sees_what() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);
    let (payload, _digest, frames) = capture_and_frame(&live, None);
    let chunks = chunks_over_the_wire(&frames);

    let mut receiver = boot_to_restore_point(&payload);
    // The projection scope BEFORE the restore: the ship this receiving host's own
    // crew is aboard.
    let scope_before = local_ship_uuid(&mut receiver);

    // Reassemble and restore through the real path.
    for chunk in &chunks {
        let mut rx = receiver.world_mut().resource_mut::<MeshSnapshotReceiver>();
        let _ = rx.accept_chunk(chunk);
    }
    arm_receiver(&mut receiver, SLOT_SENDER);
    drain_mesh_restore(receiver.world_mut());
    assert!(matches!(
        receiver
            .world()
            .resource::<MeshSnapshotReceiver>()
            .last_outcome(),
        Some(MeshRestoreOutcome::Committed { .. })
    ));

    // 1. The projection scope did not move. The restore overwrote every ship's
    //    authoritative state by uuid, but `LocalShip` — which decides whose
    //    private consoles this host paints — is host-local and not in the record,
    //    so this host still projects the same ship it did before. A record that
    //    carried an audience marker could have re-pointed it at another crew's
    //    ship; this one cannot.
    let scope_after = local_ship_uuid(&mut receiver);
    assert_eq!(
        scope_before, scope_after,
        "the restore moved which ship this host projects to its crew — a record \
         must not be able to widen projection onto another crew's hull"
    );

    // 2. The world-global evidence store crossed the transfer verbatim: what any
    //    crew can be shown about a subject is neither dropped nor duplicated by
    //    the framing. (Per-ship KEYING of that store is #1016's; #1117 carries
    //    today's store faithfully and does not itself widen it.)
    let reassembled = {
        let mut rx = project_phoenix::lockstep::SnapshotReceiver::new();
        let mut text = None;
        for chunk in &chunks {
            if let Ok(Accepted::Complete(t)) = rx.accept(chunk) {
                text = Some(t);
            }
        }
        snapshot::import_artifact(&text.expect("reassembles"), &current_versions())
            .expect("gates")
            .snapshot
            .expect("carries a snapshot")
            .state
    };
    assert_eq!(
        reassembled.scenario.as_ref().map(|s| &s.evidence),
        payload.scenario.as_ref().map(|s| &s.evidence),
        "every ship's evidence store must cross the transfer exactly as captured"
    );
}

/// **AC2, the content dimension explicitly.** The gate refuses a record whose
/// scenario/content digest differs, checked directly through the gate seam.
#[test]
fn the_content_identity_gate_refuses_mismatched_content() {
    let mut live = boot();
    step(&mut live, CAPTURE_AT);
    let (payload, _digest, frames) = capture_and_frame(&live, None);
    let chunks = chunks_over_the_wire(&frames);

    // Reassemble the (valid) record.
    let mut rx = project_phoenix::lockstep::SnapshotReceiver::new();
    let mut text = None;
    for chunk in &chunks {
        if let Accepted::Complete(t) = rx.accept(chunk).expect("accepts") {
            text = Some(t);
        }
    }
    let text = text.expect("reassembles");

    let mut receiver = boot_to_restore_point(&payload);
    let before = world_digest(receiver.world());

    // Gate the intact record against a version whose CONTENT digest differs — a
    // save from a world with different authored files.
    let real = current_versions();
    let content = snapshot::content_digest(&content_ledger::frozen_or_live());
    let wrong_content = Versions::new(
        snapshot::SNAPSHOT_FORMAT,
        snapshot::SIMULATION_RULES,
        content ^ 0xffff,
    );
    assert_ne!(
        real, wrong_content,
        "the forged versions must actually differ"
    );

    let outcome = gate_and_restore_against(receiver.world_mut(), &text, &wrong_content);
    assert!(
        matches!(outcome, MeshRestoreOutcome::RefusedGate(_)),
        "a content mismatch must be refused at the boundary, got {outcome:?}"
    );
    assert_eq!(
        world_digest(receiver.world()),
        before,
        "the content gate must run before any state is written"
    );
}
