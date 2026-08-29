//! Automatic recovery from a real host divergence (issue #1118).
//!
//! The crown proof for the whole issue, built on `tests/lockstep_mesh.rs`'s
//! in-process mesh and `tests/lockstep_snapshot_transfer.rs`'s transfer harness.
//! Three complete headless simulations run in one process with a synchronous mesh
//! between them; one host's WORLD is then deliberately corrupted mid-mission, and
//! the test proves the fleet detects the split, elects the same leader on every
//! host, holds at the boundary, transfers the canonical record, restores the
//! divergent host, and reconverges bit-for-bit — then that a two-host split and a
//! non-leader snapshot both fail cleanly without ever overwriting a world.
//!
//! # Why a real world corruption, not a doctored ledger
//!
//! `tests/lockstep_mesh.rs` injects a mismatch into a host's digest LEDGER,
//! because it is testing the DETECTOR — it must not perturb the simulation. This
//! file is testing the CURE, so it perturbs the thing the cure heals: it nudges one
//! host's ship state, which diverges that host's authoritative fold from the tick
//! of the nudge onward exactly as a real nondeterminism bug would, and then proves
//! the transfer-and-restore puts it back.
//!
//! # Why its own test binary
//!
//! `--deterministic` pins the scheduler with a one-thread `TaskPoolOptions`, and
//! Bevy's task pools are process-global, fixed by whichever app builds first, so a
//! bit-identical-fold claim in a shared binary is a claim about a race. Same reason
//! as `tests/lockstep_mesh.rs` and `tests/lockstep_snapshot_transfer.rs`.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::command_admission::{CommandDelay, CommandLog, HostSlot};
use project_phoenix::core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId};
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lobby::{InboundMessage, Sessions};
use project_phoenix::lockstep::recovery::{RecoveryLog, RecoveryResult, RecoveryState};
use project_phoenix::lockstep::{
    capture_run, drain_mesh_restore, frames_for, join_fleet, FleetRoster, FleetShip, MeshAgreement,
    MeshFrame, MeshInbox, MeshOutbox, MeshRestoreArm, MeshRestoreOutcome, MeshSnapshotReceiver,
};
use project_phoenix::server_app::LocalShip;
use project_phoenix::ship::state::ShipPhysics;
use project_phoenix::sim_tick::SimTick;

/// Three crewed hulls and a hostile, so the fold the hosts compare covers combat.
const WORLD: &str = "assets/worlds/probe_fleet_trio.toml";
const SHIP: &str = "assets/entities/alliance_cruiser.toml";
const SEED: u64 = 1_118_118;

/// A short digest cadence, so a divergence is named a few ticks after it happens
/// and the recovery boundary lands inside a probe-length run.
const INTERVAL: u64 = 20;

const SLOT_ONE: HostSlot = HostSlot(1);
const SLOT_TWO: HostSlot = HostSlot(2);
const SLOT_THREE: HostSlot = HostSlot(3);

const HELM: &str = "helm";
const RATING: &str = "Std";

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: SHIP.into(),
        max_ticks: 5_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// A frozen fleet of `slots`, each crewed at Helm, from `local`'s point of view.
fn roster(local: HostSlot, slots: &[HostSlot]) -> FleetRoster {
    let ships = slots
        .iter()
        .map(|&host| FleetShip {
            host,
            ship_path: Some(SHIP.into()),
            crew: vec![(StationId(HELM.into()), RATING.to_string())],
        })
        .collect();
    FleetRoster::new(ships, local)
}

/// One authoritative host.
struct Host {
    app: App,
    slot: HostSlot,
    token: String,
}

impl Host {
    fn new(slot: HostSlot, slots: &[HostSlot]) -> Self {
        let mut app = build_headless_app(&args()).expect("app should build");
        let token = format!("crew-{}", slot.slot_id());
        {
            let mut sessions = app.world_mut().resource_mut::<Sessions>();
            sessions
                .0
                .register(token.clone(), format!("Helm {}", slot.0))
                .expect("a fresh token");
            sessions.0.set_station(&token, Some(StationId(HELM.into())));
        }
        let delay = project_phoenix::lockstep::authored_delay(app.world());
        assert!(delay > 0, "the probe world must author a non-zero delay");
        join_fleet(app.world_mut(), roster(slot, slots), delay);
        app.insert_resource(MeshAgreement::new(INTERVAL));
        Self { app, slot, token }
    }

    fn tick(&self) -> u64 {
        self.app.world().resource::<SimTick>().0
    }

    fn digest(&self) -> u64 {
        world_digest(self.app.world())
    }

    fn delay(&self) -> u64 {
        self.app.world().resource::<CommandDelay>().0
    }

    fn drain_outbox(&mut self) -> Vec<MeshFrame> {
        self.app.world_mut().resource_mut::<MeshOutbox>().drain()
    }

    fn deliver(&mut self, frames: &[MeshFrame]) {
        let mut inbox = self.app.world_mut().resource_mut::<MeshInbox>();
        for frame in frames {
            inbox.push(frame.clone());
        }
    }

    fn crew_command(&mut self, target: &str, payload: SystemControlPayload) {
        self.app
            .world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: self.token.clone(),
                msg: ClientMessage::ControlSystem {
                    target: SystemId(target.into()),
                    payload,
                },
            });
    }

    fn recovery_log(&self) -> &RecoveryLog {
        self.app.world().resource::<RecoveryLog>()
    }

    fn recovery_active(&self) -> bool {
        self.app.world().resource::<RecoveryState>().is_active()
    }

    fn last_restore(&self) -> Option<MeshRestoreOutcome> {
        self.app
            .world()
            .resource::<MeshSnapshotReceiver>()
            .last_outcome()
            .cloned()
    }

    /// Corrupt this host's authoritative fold: nudge its own ship's position by
    /// `delta`. `ShipPhysics` is folded field-by-field and captured/restored, so
    /// this diverges the fold from this tick on and is exactly what the transfer
    /// heals.
    fn inject_position_divergence(&mut self, delta: f32) {
        let mut q = self
            .app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<LocalShip>>();
        let mut physics = q
            .single_mut(self.app.world_mut())
            .expect("this host projects exactly one ship");
        physics.x += delta;
    }
}

/// One round of "everything each host said reaches every other host".
fn ferry(hosts: &mut [Host]) {
    let outgoing: Vec<Vec<MeshFrame>> = hosts.iter_mut().map(|h| h.drain_outbox()).collect();
    for (i, host) in hosts.iter_mut().enumerate() {
        for (j, frames) in outgoing.iter().enumerate() {
            if i != j {
                host.deliver(frames);
            }
        }
    }
}

/// Step every host one tick, delivering the previous tick's traffic first.
fn step(hosts: &mut [Host]) {
    ferry(hosts);
    for host in hosts.iter_mut() {
        run(&mut host.app, 1);
    }
}

fn steer(value: f32) -> SystemControlPayload {
    SystemControlPayload::SetSteering { value }
}

fn thrust(value: f32) -> SystemControlPayload {
    SystemControlPayload::SetThrust { value }
}

// ── The headline: detect, elect, hold, transfer, restore, reconverge ─────────

/// **AC1–AC5.** A real world divergence on one of three hosts is detected, all
/// three elect the same leader, the fleet holds at the boundary, the divergent
/// host restores the canonical record, and every host folds identically again —
/// with a diagnostic artifact on each.
#[test]
fn a_diverged_host_is_healed_and_the_whole_fleet_reconverges() {
    let slots = [SLOT_ONE, SLOT_TWO, SLOT_THREE];
    let mut hosts = vec![
        Host::new(SLOT_ONE, &slots),
        Host::new(SLOT_TWO, &slots),
        Host::new(SLOT_THREE, &slots),
    ];
    let delay = hosts[0].delay();

    // A real mission: each crew steers and throttles its own ship.
    let orders: &[(u64, HostSlot, &str, SystemControlPayload)] = &[
        (6, SLOT_ONE, "helm-thrust", thrust(0.8)),
        (6, SLOT_TWO, "helm-thrust", thrust(0.6)),
        (6, SLOT_THREE, "helm-thrust", thrust(0.7)),
        (10, SLOT_ONE, "helm-steering", steer(0.3)),
        (12, SLOT_THREE, "helm-steering", steer(-0.25)),
    ];

    // Run in agreement first, so the split below is a change of behaviour.
    for tick in 0..30 {
        for (at, slot, target, payload) in orders {
            if *at == tick {
                hosts
                    .iter_mut()
                    .find(|h| h.slot == *slot)
                    .unwrap()
                    .crew_command(target, payload.clone());
            }
        }
        step(&mut hosts);
        let d: Vec<u64> = hosts.iter().map(|h| h.digest()).collect();
        assert!(
            d[0] == d[1] && d[1] == d[2],
            "the fleet disagreed BEFORE any fault was injected, at tick {}: {d:?}",
            hosts[0].tick()
        );
    }

    // Corrupt slot 3's authoritative fold. Slots 1 and 2 are the majority.
    hosts[2].inject_position_divergence(200.0);
    assert_ne!(
        hosts[2].digest(),
        hosts[0].digest(),
        "the injection must actually diverge slot 3's fold"
    );

    // Run through detection, the boundary hold, the transfer, the restore and the
    // reconvergence. Capture the tick slot 3 was at when it committed the restore:
    // if the boundary held, slot 3 cannot have run its divergent world past it.
    let mut slot3_tick_at_commit: Option<u64> = None;
    for _ in 0..400 {
        step(&mut hosts);
        if slot3_tick_at_commit.is_none()
            && matches!(hosts[2].last_restore(), Some(MeshRestoreOutcome::Committed { .. }))
        {
            slot3_tick_at_commit = Some(hosts[2].tick());
        }
        // Stop once every host has recorded its recovery outcome and stepped well
        // past the boundary, so the tail below is genuine post-recovery running.
        if hosts.iter().all(|h| !h.recovery_log().entries().is_empty())
            && hosts.iter().all(|h| !h.recovery_active())
            && hosts.iter().all(|h| h.tick() > 200)
        {
            break;
        }
    }

    // 1. Every host recorded a diagnostic, and they agree on the shared decision.
    let diags: Vec<_> = hosts
        .iter()
        .map(|h| h.recovery_log().last().cloned().expect("a diagnostic"))
        .collect();
    let boundary = diags[0].boundary_tick;
    for (i, d) in diags.iter().enumerate() {
        assert_eq!(
            d.leader,
            Some(SLOT_ONE),
            "host {i} elected a different leader — leader choice must be a \
             deterministic function of the shared digests, not who-noticed-first"
        );
        assert_eq!(d.recovering, vec![SLOT_THREE], "host {i} recovering set");
        assert_eq!(d.divergence_tick, diags[0].divergence_tick, "host {i} tick");
        assert_eq!(d.boundary_tick, boundary, "host {i} boundary");
        assert_eq!(d.digests.len(), 3, "host {i} folds all three slots");
        assert!(d.canonical_digest.is_some(), "host {i} canonical fold");
    }

    // 2. Each host recorded the result its role implies. The leader captured its
    //    record at the SAME tick the recovering host restored to — the boundary
    //    the fleet held at — proving they resume aligned.
    let led_tick = match diags[0].result {
        RecoveryResult::Led { record_tick } => record_tick,
        ref other => panic!("slot 1 (leader) result: {other:?}"),
    };
    assert_eq!(
        diags[1].result,
        RecoveryResult::Witnessed,
        "slot 2 (canonical bystander) result"
    );
    let recovered_tick = match diags[2].result {
        RecoveryResult::Recovered { record_tick, .. } => record_tick,
        ref other => panic!("slot 3 (recovering) result: {other:?}"),
    };
    assert_eq!(
        led_tick, recovered_tick,
        "the leader captured at tick {led_tick} but the recovering host restored to \
         {recovered_tick} — the fleet must hold and resume at ONE agreed tick"
    );
    assert!(
        recovered_tick > boundary && recovered_tick <= boundary + INTERVAL,
        "the recovery tick {recovered_tick} is not just past the boundary {boundary} — \
         the fleet must hold AT the boundary, not run on to the next checkpoint"
    );

    // 3. The divergent host actually committed the transferred record — and it did
    //    so WHILE held at the boundary, so it never ran its divergent world onward
    //    (AC3: resume only after the transfer).
    assert!(
        matches!(hosts[2].last_restore(), Some(MeshRestoreOutcome::Committed { .. })),
        "slot 3 must have committed the leader's record, got {:?}",
        hosts[2].last_restore()
    );
    assert_eq!(
        slot3_tick_at_commit,
        Some(recovered_tick),
        "slot 3 committed the restore at a different tick than the record's — the \
         boundary must withhold the divergent host until it restores"
    );

    // 5. Reconvergence: the fold is bit-identical on every host, every tick, for
    //    the tail of the run. A photograph at the instant of restore is not
    //    enough — a cold state machine is invisible in one, so this is a
    //    continuation.
    let mut agreed_ticks = 0;
    for _ in 0..120 {
        step(&mut hosts);
        let d: Vec<u64> = hosts.iter().map(|h| h.digest()).collect();
        assert!(
            d[0] == d[1] && d[1] == d[2],
            "the fleet diverged AGAIN {} tick(s) after recovery, at tick {}: {d:?}",
            agreed_ticks,
            hosts[0].tick()
        );
        agreed_ticks += 1;
    }
    assert_eq!(agreed_ticks, 120, "the tail must have run");

    // Anti-vacuity: the run was doing something worth agreeing about, and the
    // boundary was a real number past the divergence.
    assert!(boundary > diags[0].divergence_tick, "boundary past divergence");
    assert!(
        !hosts[0].app.world().resource::<CommandLog>().is_empty(),
        "the crews gave orders — the recovery healed a live mission, not an idle one"
    );
    // The command window in the artifact is the input a replay reads.
    assert_eq!(
        diags[2].last_agreed_tick,
        diags[0].last_agreed_tick,
        "every host names the same last-agreed edge for the command window"
    );
    let _ = delay;
}

// ── AC6: clean failure when there is no safe leader ──────────────────────────

/// **AC6.** Two hosts that diverge have no majority, so recovery refuses cleanly:
/// a diagnostic is recorded, no record is transferred, and NEITHER world is
/// touched — the canonical host is not overwritten and the divergent host is not
/// "healed" to a guess.
#[test]
fn a_two_host_split_fails_cleanly_without_overwriting_either_world() {
    let slots = [SLOT_ONE, SLOT_TWO];
    let mut hosts = vec![Host::new(SLOT_ONE, &slots), Host::new(SLOT_TWO, &slots)];

    for _ in 0..25 {
        step(&mut hosts);
    }
    let canonical_before = hosts[0].digest();
    hosts[1].inject_position_divergence(200.0);
    let diverged_after_inject = hosts[1].digest();
    assert_ne!(canonical_before, diverged_after_inject, "the split is real");

    // Run past detection. Neither host may attempt a restore.
    for _ in 0..120 {
        step(&mut hosts);
        assert!(
            hosts
                .iter()
                .all(|h| !matches!(h.last_restore(), Some(MeshRestoreOutcome::Committed { .. }))),
            "a two-host split has no safe leader — nothing may be committed"
        );
        if hosts.iter().all(|h| !h.recovery_log().entries().is_empty()) {
            break;
        }
    }

    // Both hosts recorded a no-safe-leader diagnostic.
    for (i, host) in hosts.iter().enumerate() {
        let diag = host.recovery_log().last().expect("a diagnostic on host");
        assert!(
            matches!(
                diag.result,
                RecoveryResult::NoSafeLeader {
                    largest_group: 1,
                    fleet: 2
                }
            ),
            "host {i} must record a clean no-safe-leader failure, got {:?}",
            diag.result
        );
        assert_eq!(diag.leader, None, "host {i}: no leader was elected");
    }

    // Step both a little further, in lockstep, and confirm the fleet is STILL
    // split — recovery declined to make it agree by overwriting one from the
    // other. Neither world was half-restored (no commit ever happened), so both
    // are intact, valid, and simply different.
    for _ in 0..10 {
        step(&mut hosts);
    }
    assert_eq!(
        hosts[0].tick(),
        hosts[1].tick(),
        "the two hosts stay in tick-lockstep even while diverged"
    );
    assert_ne!(
        hosts[0].digest(),
        hosts[1].digest(),
        "recovery must NOT have silently healed a two-host split — with no majority \
         there is no canonical record, so the honest state is still-divergent, not \
         a coin-flip agreement"
    );
    assert!(
        hosts
            .iter()
            .all(|h| !matches!(h.last_restore(), Some(MeshRestoreOutcome::Committed { .. }))),
        "no restore was ever committed on either host"
    );
}

// ── AC2: the receiver arm — no blind overwrite ───────────────────────────────

/// **AC2 (carried).** A fully-arrived snapshot is NOT applied unless this host is
/// armed to restore it, and unless it came from the leader the plan named. A peer
/// cannot overwrite another host's world simply by sending it a record.
#[test]
fn a_snapshot_is_refused_outside_a_recovery_and_from_a_non_leader() {
    let slots = [SLOT_ONE, SLOT_TWO];
    let mut hosts = vec![Host::new(SLOT_ONE, &slots), Host::new(SLOT_TWO, &slots)];
    for _ in 0..25 {
        step(&mut hosts);
    }

    // A genuine, gate-passing record captured from host 1's live world.
    let record = capture_run(hosts[0].app.world(), WORLD);

    // 1. Delivered to host 2, which is NOT in a recovery (disarmed): dropped. The
    //    chunks are staged and the restore driven directly, WITHOUT advancing a
    //    sim tick, so any digest change would be the restore's doing and nothing
    //    else — the fold is untouched.
    let before = hosts[1].digest();
    let from_leader = frames_for(&record, SLOT_ONE, 0x1118_0001).expect("frames");
    stage_and_drain(&mut hosts[1], &from_leader);
    assert_eq!(
        hosts[1].last_restore(),
        Some(MeshRestoreOutcome::RefusedUnarmed),
        "an unarmed host must refuse a record, not install it"
    );
    assert_eq!(
        hosts[1].digest(),
        before,
        "an unarmed host's world must be untouched by a delivered snapshot"
    );

    // 2. Now arm host 2 to accept a record from the LEADER (slot 1), but deliver
    //    one that came from slot 2: refused as the wrong sender, world untouched.
    hosts[1]
        .app
        .world_mut()
        .resource_mut::<MeshRestoreArm>()
        .arm(SLOT_ONE);
    let before = hosts[1].digest();
    let from_non_leader = frames_for(&record, SLOT_TWO, 0x1118_0002).expect("frames");
    stage_and_drain(&mut hosts[1], &from_non_leader);
    assert!(
        matches!(
            hosts[1].last_restore(),
            Some(MeshRestoreOutcome::RefusedWrongSender { .. })
        ),
        "a record from a slot other than the leader must be refused, got {:?}",
        hosts[1].last_restore()
    );
    assert_eq!(
        hosts[1].digest(),
        before,
        "a wrong-sender record must leave the world untouched — never a blind \
         overwrite by an arbitrary peer"
    );
}

/// Feed a record's chunks straight into a host's receiver and drive the restore,
/// without advancing a sim tick — so a digest change can only be the restore's.
fn stage_and_drain(host: &mut Host, frames: &[MeshFrame]) {
    for frame in frames {
        if let MeshFrame::Snapshot(chunk) = frame {
            host.app
                .world_mut()
                .resource_mut::<MeshSnapshotReceiver>()
                .accept_chunk(chunk)
                .ok();
        }
    }
    drain_mesh_restore(host.app.world_mut());
}
