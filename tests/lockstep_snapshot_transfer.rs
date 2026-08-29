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

use bevy::prelude::{App, With};

use project_phoenix::command_admission::HostSlot;
use project_phoenix::content_ledger;
use project_phoenix::core::codec::{decode_mesh_frame, encode_mesh_frame};
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::lockstep::snapshot_relay::{drain_mesh_restore, frames_for};
use project_phoenix::lockstep::transfer::{Accepted, SnapshotChunk, TransferError};
use project_phoenix::lockstep::{
    gate_and_restore_against, send_snapshot, MeshFrame, MeshOutbox, MeshRestoreOutcome,
    MeshSnapshotReceiver,
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
const SLOT_RECEIVER: HostSlot = HostSlot(2);
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

/// A jumbled-but-complete delivery order that still covers every chunk once.
fn jumbled(len: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).rev().collect();
    if len > 2 {
        order.swap(0, len / 2);
    }
    order
}

// ── The headline ─────────────────────────────────────────────────────────────

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
    // post-restore digest check.
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
/// dossier and console builders run `With<LocalShip>`, and `LocalShip` is a
/// host-local marker of "the ship this machine's crew is aboard" that is *never*
/// captured or restored (it is not a field of `PhoenixSnapshot` or `EntityState`).
/// So a restore cannot move which ship a host projects — the guarantee #1117
/// makes is exactly "restore does not itself widen projection".
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
