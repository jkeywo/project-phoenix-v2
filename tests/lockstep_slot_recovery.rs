//! Recovering a disconnected fixed ship slot on another machine (issue #1120).
//!
//! The crown proof for the final piece of PRD #1093's host mesh, built on
//! `tests/lockstep_recovery.rs`'s divergence-recovery harness and
//! `tests/lockstep_backfill.rs`'s host-loss one. A three-host fleet runs a
//! mission; one host vanishes and its ship backfills; then a FRESH MACHINE claims
//! the disconnected slot, restores the canonical record through #1117's transfer,
//! proves agreement, takes over — and the whole fleet finishes folding
//! bit-identically (AC6). Beside it: the deterministic claim race (AC2), the
//! sender-authentication rejection this issue owns, the #1119 tick-0 self-loss
//! guard regression, and the AC1 refusal to displace a still-connected slot.
//!
//! # Why its own test binary
//!
//! `--deterministic` pins the scheduler with a one-thread `TaskPoolOptions`, and
//! Bevy's task pools are process-global, fixed by whichever app builds first — so a
//! bit-identical-fold claim in a shared binary is a claim about a race. Same reason
//! as its two sibling suites.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::command_admission::log::{CommandOrder, ShipKey};
use project_phoenix::command_admission::{CommandLog, HostSlot};
use project_phoenix::core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId};
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lobby::{InboundMessage, Sessions};
use project_phoenix::lockstep::{
    join_fleet, FleetLockstep, FleetRoster, FleetShip, HostLossFrame, MeshCommand, MeshFrame,
    MeshInbox, MeshOrigin, MeshOutbox, MeshRestoreOutcome, MeshSnapshotReceiver, PendingHostLoss,
    PendingSlotClaims, SlotClaimFrame, SlotRecoveryHold, SlotRecoveryLog, SlotRecoveryResult,
    SlotRecoveryState,
};
use project_phoenix::sim_tick::SimTick;

/// Three crewed hulls and a hostile, so the fold covers combat.
const WORLD: &str = "assets/worlds/probe_fleet_backfill.toml";
const SHIP: &str = "assets/entities/alliance_cruiser.toml";
const SEED: u64 = 1_120_120;

/// A short digest cadence, so the recovery boundary lands inside a probe-length run.
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
        max_ticks: 100_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// A frozen fleet of `slots`, each crewed at Helm except any in `uncrewed`, from
/// `local`'s point of view. The replacement joins with the disconnected slot
/// uncrewed — the same picture the survivors hold after `depart_slot` emptied it —
/// so its human-seeking resolver keeps that ship on Backfill, matching them.
fn roster(local: HostSlot, slots: &[HostSlot], uncrewed: &[HostSlot]) -> FleetRoster {
    let ships = slots
        .iter()
        .map(|&host| FleetShip {
            host,
            ship_path: Some(SHIP.into()),
            crew: if uncrewed.contains(&host) {
                vec![]
            } else {
                vec![(StationId(HELM.into()), RATING.to_string())]
            },
        })
        .collect();
    FleetRoster::new(ships, local)
}

/// One authoritative host.
struct Host {
    app: App,
    slot: HostSlot,
    token: String,
    alive: bool,
}

impl Host {
    fn new(slot: HostSlot, slots: &[HostSlot], uncrewed: &[HostSlot]) -> Self {
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
        join_fleet(app.world_mut(), roster(slot, slots, uncrewed), delay);
        app.insert_resource(project_phoenix::lockstep::MeshAgreement::new(INTERVAL));
        Self {
            app,
            slot,
            token,
            alive: true,
        }
    }

    fn tick(&self) -> u64 {
        self.app.world().resource::<SimTick>().0
    }

    fn digest(&self) -> u64 {
        world_digest(self.app.world())
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

    fn deliver_from(&mut self, frames: &[MeshFrame], origin: MeshOrigin) {
        let mut inbox = self.app.world_mut().resource_mut::<MeshInbox>();
        for frame in frames {
            inbox.push_from(frame.clone(), origin);
        }
    }

    /// Declare a peer ready far ahead, so a single-host fixture does not stall on
    /// peers that are not being simulated in this focused test.
    fn observe_peer(&mut self, slot: HostSlot, watermark: u64) {
        self.app
            .world_mut()
            .resource_mut::<FleetLockstep>()
            .observe(slot, watermark);
    }

    /// Report to THIS host that `lost`'s ship host has vanished, as a genuine
    /// tick-0 self-observation (the bridge's `LocalObservation`).
    fn observe_host_loss(&mut self, lost: HostSlot) {
        self.deliver_from(
            &[MeshFrame::HostLoss(HostLossFrame {
                from: lost,
                lost,
                tick: 0,
            })],
            MeshOrigin::LocalObservation,
        );
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

    fn last_restore(&self) -> Option<MeshRestoreOutcome> {
        self.app
            .world()
            .resource::<MeshSnapshotReceiver>()
            .last_outcome()
            .cloned()
    }

    fn slot_recovery_log(&self) -> Vec<SlotRecoveryResult> {
        self.app
            .world()
            .resource::<SlotRecoveryLog>()
            .entries()
            .iter()
            .map(|r| r.result.clone())
            .collect()
    }

    fn slot_recovery_active(&self) -> bool {
        self.app.world().resource::<SlotRecoveryState>().is_active()
    }

    fn command_log_len(&self) -> usize {
        self.app.world().resource::<CommandLog>().len()
    }

    fn is_departed(&self, slot: HostSlot) -> bool {
        self.app
            .world()
            .resource::<FleetLockstep>()
            .has_departed(slot)
    }
}

/// One round of "everything each LIVE host said reaches every other LIVE host".
fn ferry(hosts: &mut [Host]) {
    let outgoing: Vec<(usize, Vec<MeshFrame>)> = hosts
        .iter_mut()
        .enumerate()
        .filter(|(_, h)| h.alive)
        .map(|(i, h)| (i, h.drain_outbox()))
        .collect();
    for (i, host) in hosts.iter_mut().enumerate() {
        if !host.alive {
            continue;
        }
        for (j, frames) in &outgoing {
            if i != *j {
                host.deliver(frames);
            }
        }
    }
}

/// Step every LIVE host one tick, delivering the previous round first.
fn step(hosts: &mut [Host]) {
    ferry(hosts);
    for host in hosts.iter_mut() {
        if host.alive {
            run(&mut host.app, 1);
        }
    }
}

fn steer(value: f32) -> SystemControlPayload {
    SystemControlPayload::SetSteering { value }
}

fn thrust(value: f32) -> SystemControlPayload {
    SystemControlPayload::SetThrust { value }
}

// ── The headline: claim, restore, prove, take over, reconverge (AC6) ─────────

/// **AC1–AC6.** A three-host fleet loses a host mid-mission; a FRESH MACHINE claims
/// the disconnected slot, restores the canonical record through #1117's transfer,
/// proves agreement (its restored world folds to the leader's), takes over the
/// slot, and the whole fleet finishes folding bit-identically on every shared tick.
#[test]
fn a_replacement_recovers_a_disconnected_slot_and_the_fleet_reconverges() {
    let all = [SLOT_ONE, SLOT_TWO, SLOT_THREE];
    let mut hosts = vec![
        Host::new(SLOT_ONE, &all, &[]),
        Host::new(SLOT_TWO, &all, &[]),
        Host::new(SLOT_THREE, &all, &[]),
    ];

    // A real mission first, so the recovery heals a live world.
    let orders: &[(u64, HostSlot, &str, SystemControlPayload)] = &[
        (6, SLOT_ONE, "helm-thrust", thrust(0.8)),
        (6, SLOT_TWO, "helm-thrust", thrust(0.6)),
        (6, SLOT_THREE, "helm-thrust", thrust(0.7)),
        (10, SLOT_ONE, "helm-steering", steer(0.3)),
        (12, SLOT_THREE, "helm-steering", steer(-0.25)),
    ];
    for tick in 0..40 {
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
            "the fleet disagreed BEFORE the loss, at tick {}: {d:?}",
            hosts[0].tick()
        );
    }

    // Slot 3's host vanishes. Its ship flips to Backfill on the survivors.
    hosts[2].alive = false;
    hosts[0].observe_host_loss(SLOT_THREE);
    hosts[1].observe_host_loss(SLOT_THREE);
    for _ in 0..12 {
        step(&mut hosts);
    }
    assert!(
        hosts[0].is_departed(SLOT_THREE) && hosts[1].is_departed(SLOT_THREE),
        "the survivors must have departed slot 3 before the claim"
    );

    // A FRESH MACHINE boots as slot 3, with the slot uncrewed — the same picture the
    // survivors hold — and takes the vanished host's place in the ferry.
    hosts[2] = Host::new(SLOT_THREE, &all, &[SLOT_THREE]);
    assert_eq!(hosts[2].tick(), 0, "the replacement starts from a fresh world");

    // The owner (slot 1) grants the claim: it stamps the current tick and the first
    // sequence and broadcasts one SlotClaimFrame to the whole fleet.
    let claim_tick = hosts[0].tick();
    let claim = MeshFrame::SlotClaim(SlotClaimFrame {
        from: SLOT_ONE,
        slot: SLOT_THREE,
        claim_seq: 1,
        tick: claim_tick,
    });
    for host in hosts.iter_mut() {
        host.deliver(std::slice::from_ref(&claim));
    }

    // Drive through the boundary hold, the transfer, the restore and the takeover.
    let mut committed_tick: Option<u64> = None;
    for _ in 0..600 {
        step(&mut hosts);
        if committed_tick.is_none() {
            if let Some(MeshRestoreOutcome::Committed { tick, .. }) = hosts[2].last_restore() {
                committed_tick = Some(tick);
            }
        }
        let resolved = hosts.iter().all(|h| !h.slot_recovery_active())
            && hosts.iter().all(|h| !h.slot_recovery_log().is_empty());
        if resolved && committed_tick.is_some() && hosts.iter().all(|h| h.tick() > claim_tick + 80) {
            break;
        }
    }

    // AC3: the replacement committed the transferred record, proving agreement.
    assert!(
        matches!(
            hosts[2].last_restore(),
            Some(MeshRestoreOutcome::Committed { .. })
        ),
        "the replacement must have committed the leader's record, got {:?}",
        hosts[2].last_restore()
    );
    let committed_tick = committed_tick.expect("a commit tick");

    // Every host recorded the event its role implies.
    assert_eq!(
        hosts[0].slot_recovery_log(),
        vec![SlotRecoveryResult::Led {
            record_tick: committed_tick
        }],
        "slot 1 led and captured at the tick the replacement restored to"
    );
    assert_eq!(
        hosts[1].slot_recovery_log(),
        vec![SlotRecoveryResult::Witnessed],
        "slot 2 was a surviving bystander"
    );
    assert!(
        matches!(
            hosts[2].slot_recovery_log().as_slice(),
            [SlotRecoveryResult::Recovered { record_tick, .. }] if *record_tick == committed_tick
        ),
        "the replacement recorded a Recovered result at the boundary tick, got {:?}",
        hosts[2].slot_recovery_log()
    );

    // AC6: the fold is bit-identical on every host, every shared tick, for the tail
    // — the replacement is now a full authoritative member of the fleet.
    let mut digests: [std::collections::BTreeMap<u64, u64>; 3] =
        [Default::default(), Default::default(), Default::default()];
    for _ in 0..120 {
        step(&mut hosts);
        for (i, h) in hosts.iter().enumerate() {
            digests[i].insert(h.tick(), h.digest());
        }
    }
    let mut shared = 0usize;
    for (tick, a) in &digests[0] {
        if let (Some(b), Some(c)) = (digests[1].get(tick), digests[2].get(tick)) {
            shared += 1;
            assert!(
                a == b && b == c,
                "the fleet diverged after recovery at tick {tick}: {a:#018x} / \
                 {b:#018x} / {c:#018x} — the replacement is not folding the same \
                 world as the survivors"
            );
        }
    }
    assert!(
        shared > 60,
        "the three hosts barely overlapped ({shared} shared ticks) — the tail is \
         too thin to prove the takeover reconverged the fleet"
    );

    // Anti-vacuity: the mission gave orders the recovery had to preserve, and the
    // replacement genuinely restored to a tick past the claim.
    assert!(
        hosts[0].command_log_len() >= orders.len(),
        "the crews' orders must be in the record the recovery healed"
    );
    assert!(
        committed_tick > claim_tick,
        "the replacement restored to a tick past the claim ({committed_tick} > {claim_tick})"
    );
}

// ── AC2: the deterministic claim race ────────────────────────────────────────

/// **AC2.** Two machines claim the same vacant slot; every host elects the same
/// winner — the lowest owner-minted `claim_seq` — from the shared claim frames,
/// whatever order it heard them in.
#[test]
fn two_claims_for_one_slot_elect_the_same_winner_on_every_host() {
    // Host A hears the higher claim first; host B hears the lower first.
    let mut a = PendingSlotClaims::default();
    a.observe(SLOT_THREE, 9, 200);
    a.observe(SLOT_THREE, 4, 201);

    let mut b = PendingSlotClaims::default();
    b.observe(SLOT_THREE, 4, 201);
    b.observe(SLOT_THREE, 9, 200);

    assert_eq!(a.winning_seq(SLOT_THREE), Some(4));
    assert_eq!(
        a.winning_seq(SLOT_THREE),
        b.winning_seq(SLOT_THREE),
        "two hosts that heard the same two claims in opposite orders must agree the \
         same winner — the resolution is a function of the seq value, not arrival"
    );
    assert!(a.wins(SLOT_THREE, 4) && !a.wins(SLOT_THREE, 9));
}

// ── The carried sender authentication (#1117/#1118/#1119, owned here) ─────────

/// **Sender authentication.** A frame whose declared `from` disagrees with the
/// connection that delivered it is REFUSED at the mesh boundary, so a peer cannot
/// speak under another slot's identity. The lead's verbatim relay is the one
/// accommodation: a frame it delivers on behalf of a sibling is accepted.
#[test]
fn a_forged_sender_is_refused_at_the_mesh_boundary() {
    let all = [SLOT_ONE, SLOT_TWO, SLOT_THREE];
    // Host 1 is the LEAD, which authenticates every member frame against the
    // connection it arrived on. Its peers are declared far ahead so this focused
    // fixture runs freely rather than stalling on hosts it does not simulate.
    let mut hosts = vec![Host::new(SLOT_ONE, &all, &[])];
    hosts[0].observe_peer(SLOT_TWO, u64::MAX / 2);
    hosts[0].observe_peer(SLOT_THREE, u64::MAX / 2);
    for _ in 0..30 {
        step(&mut hosts);
        hosts[0].observe_peer(SLOT_TWO, u64::MAX / 2);
        hosts[0].observe_peer(SLOT_THREE, u64::MAX / 2);
    }
    let lead = &mut hosts[0];
    let apply_tick = lead.tick() + 20;

    let ships: std::collections::BTreeMap<HostSlot, String> = {
        let mut q = lead
            .app
            .world_mut()
            .query::<(&project_phoenix::entities::spawner::EntityUuid, &project_phoenix::lockstep::FleetSlotOf)>();
        q.iter(lead.app.world())
            .map(|(uuid, slot)| (slot.0, uuid.0.clone()))
            .collect()
    };
    let command = |origin: HostSlot, ship: HostSlot, seq: u64| {
        MeshFrame::Tick(project_phoenix::lockstep::TickFrame {
            from: origin,
            tick: apply_tick,
            ready_through: apply_tick,
            commands: vec![MeshCommand {
                tick: apply_tick,
                order: CommandOrder::new(origin, seq),
                ship: ShipKey(ships[&ship].clone()),
                target: SystemId("helm-steering".into()),
                payload: steer(0.5),
            }],
        })
    };

    // A FORGED frame: slot 2's connection carries a frame claiming to be from slot
    // 3 (impersonation). from(3) != authenticated(2), and 2 is not the lead, so it
    // is dropped at ingress before a command can be queued.
    lead.deliver_from(&[command(SLOT_THREE, SLOT_THREE, 0)], MeshOrigin::Peer(SLOT_TWO));
    // An HONEST frame: slot 2's connection carries slot 2's own command for its own
    // ship. from(2) == authenticated(2), so it is admitted.
    lead.deliver_from(&[command(SLOT_TWO, SLOT_TWO, 1)], MeshOrigin::Peer(SLOT_TWO));

    for _ in 0..(apply_tick + 40) {
        step(&mut hosts);
    }

    let origins: std::collections::BTreeSet<HostSlot> = hosts[0]
        .app
        .world()
        .resource::<CommandLog>()
        .entries()
        .iter()
        .map(|e| e.order.origin)
        .collect();
    assert!(
        origins.contains(&SLOT_TWO),
        "the honest slot-2 command must have applied"
    );
    assert!(
        !origins.contains(&SLOT_THREE),
        "the forged slot-3 command (delivered over slot 2's connection) must have \
         been refused at the mesh boundary, not applied — a peer cannot speak under \
         another slot's identity"
    );
}

/// **The #1119 guard, carried.** A `tick == 0` self-observed host-loss is a claim
/// about THIS host's own transport; a peer that forges one over the mesh would make
/// this host derive a flip tick from a watermark other survivors do not share — the
/// skew-derive divergence #1119 fought. So a tick-0 report over a peer connection is
/// refused; one from a genuine local observation is honoured.
#[test]
fn a_forged_tick_zero_self_loss_over_a_peer_connection_is_refused() {
    let all = [SLOT_ONE, SLOT_TWO, SLOT_THREE];
    let mut hosts = vec![Host::new(SLOT_ONE, &all, &[])];
    for _ in 0..30 {
        step(&mut hosts);
    }

    // FORGED: slot 2 (over its own authenticated connection, so sender-auth passes)
    // claims to self-observe slot 3's loss with tick 0. The guard drops it.
    hosts[0].deliver_from(
        &[MeshFrame::HostLoss(HostLossFrame {
            from: SLOT_TWO,
            lost: SLOT_THREE,
            tick: 0,
        })],
        MeshOrigin::Peer(SLOT_TWO),
    );
    for _ in 0..8 {
        step(&mut hosts);
    }
    assert!(
        !hosts[0].is_departed(SLOT_THREE),
        "a forged tick-0 self-loss injected over a peer connection must be refused — \
         only a genuine local observation may derive a loss tick from a local watermark"
    );
    assert!(
        hosts[0]
            .app
            .world()
            .resource::<PendingHostLoss>()
            .agreed_tick(SLOT_THREE)
            .is_none(),
        "no loss transition may be queued from a forged self-observation"
    );

    // GENUINE: a local observation (the bridge's `LocalObservation`) for slot 3 IS
    // honoured — this host's own transport saw the close.
    hosts[0].observe_host_loss(SLOT_THREE);
    for _ in 0..8 {
        step(&mut hosts);
    }
    assert!(
        hosts[0].is_departed(SLOT_THREE),
        "a genuine local self-observation must still be honoured — the guard rejects \
         only a forged one arriving over a peer connection"
    );
}

// ── AC1: cannot displace a still-connected slot ──────────────────────────────

/// **AC1.** A claim naming a slot whose host is still connected recovers nothing —
/// a survivor opens a recovery only for a slot it has genuinely departed, so
/// server-code entry cannot displace a live host.
#[test]
fn a_survivor_will_not_recover_a_still_connected_slot() {
    let all = [SLOT_ONE, SLOT_TWO, SLOT_THREE];
    let mut hosts = vec![
        Host::new(SLOT_ONE, &all, &[]),
        Host::new(SLOT_TWO, &all, &[]),
        Host::new(SLOT_THREE, &all, &[]),
    ];
    for _ in 0..30 {
        step(&mut hosts);
    }

    // A claim on slot 2, whose host is still connected. Delivered to slot 1.
    let claim = MeshFrame::SlotClaim(SlotClaimFrame {
        from: SLOT_ONE,
        slot: SLOT_TWO,
        claim_seq: 1,
        tick: hosts[0].tick(),
    });
    hosts[0].deliver(std::slice::from_ref(&claim));
    for _ in 0..20 {
        step(&mut hosts);
    }

    assert!(
        !hosts[0].slot_recovery_active(),
        "no recovery may open for a slot whose host is still connected — a live host \
         cannot be displaced"
    );
    assert!(
        hosts[0]
            .app
            .world()
            .resource::<SlotRecoveryHold>()
            .withhold_beyond
            .is_none(),
        "and the fleet must not be held at a boundary for a claim it refused"
    );
    // The fleet is still whole and agreeing.
    let d: Vec<u64> = hosts.iter().map(|h| h.digest()).collect();
    assert!(d[0] == d[1] && d[1] == d[2], "the fleet stayed in agreement: {d:?}");
}
