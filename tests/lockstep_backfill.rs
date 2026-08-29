//! Backfilling a disconnected ship host at an agreed tick (issue #1119).
//!
//! The crown proof for #1119, built on `tests/lockstep_mesh.rs`'s in-process
//! mesh. Three complete headless simulations run in one process with a
//! synchronous in-process mesh between them, a different crew driving each. Then
//! ONE host is dropped mid-mission, and the two survivors must:
//!
//!   * agree the disconnect tick — derived from the lost host's own last
//!     watermark, the same on both without either arbitrating (AC1);
//!   * keep the lost ship's COMPLETE authoritative state while its Station
//!     Ratings flip to ordinary Backfill (AC2);
//!   * re-resolve its human-seeking systems away from the crew that vanished
//!     (AC3);
//!   * carry on without a global reset, running the lost ship's AI from the
//!     same ticks so neither survivor double-drives it and neither abandons it
//!     (AC4);
//!   * converge on ONE transition at ONE tick however the loss is reported —
//!     reordered, duplicated, or delayed (AC5); and
//!   * fold the SAME authoritative digest on every tick, through the transition
//!     and past it (AC6).
//!
//! # Why three hosts, and why its own world
//!
//! A two-host fleet dropped to one survivor has nobody left to agree WITH, so
//! AC6's "both surviving views agree" needs a third ship. `probe_fleet_backfill.
//! toml` is the smallest world that is a genuine three-crew fleet; dropping slot
//! 3 leaves slots 1 and 2 to hold the agreement against each other, tick by tick.
//!
//! # Why this is its own test binary
//!
//! `--deterministic` pins the scheduler with a one-thread `TaskPoolOptions`, and
//! Bevy's task pools are process-global, fixed by whichever app builds first — so
//! a two-survivor agreement claim made in a binary where other tests build apps
//! is a claim about whoever won that race. Same reason as `tests/lockstep_mesh.
//! rs`. Do not add unrelated tests here.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::command_admission::{CommandDelay, HostSlot};
use project_phoenix::core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId};
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lobby::{InboundMessage, Sessions};
use project_phoenix::lockstep::{
    agreed_loss_tick, join_fleet, order_mesh_inbound, FleetLockstep, FleetRoster, FleetShip,
    FleetSlotOf, HostLossFrame, HostLossRecord, MeshAgreement, MeshFrame, MeshInbox, MeshOutbox,
    PendingHostLoss,
};
use project_phoenix::ship::control_source::ControlSource;
use project_phoenix::ship::state::ShipPhysics;
use project_phoenix::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use project_phoenix::sim_tick::SimTick;

/// Three crewed hulls and a hostile.
const WORLD: &str = "assets/worlds/probe_fleet_backfill.toml";
/// The hull every host in the fleet flies.
const SHIP: &str = "assets/entities/alliance_cruiser.toml";

const SEED: u64 = 1_119_119;

const SLOT_ONE: HostSlot = HostSlot(1);
const SLOT_TWO: HostSlot = HostSlot(2);
const SLOT_THREE: HostSlot = HostSlot(3);

/// The station each crew sits at.
const CREWED_STATION: &str = "helm";
/// The Rating the fleet froze that seat at. `Std` automates nothing, so its
/// systems answer to the human rather than to Backfill AI — which is what makes
/// "the lost ship had a live crew before the drop" a real precondition.
const CREWED_RATING: &str = "Std";

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: SHIP.into(),
        // The harness drives ticks by hand; this is only an upper bound so the
        // headless auto-stop never fires mid-test.
        max_ticks: 100_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// The frozen fleet, identical on every host except for which slot is local.
fn roster(local: HostSlot) -> FleetRoster {
    let crewed = |host| FleetShip {
        host,
        ship_path: Some(SHIP.into()),
        crew: vec![(StationId(CREWED_STATION.into()), CREWED_RATING.to_string())],
    };
    FleetRoster::new(
        vec![crewed(SLOT_ONE), crewed(SLOT_TWO), crewed(SLOT_THREE)],
        local,
    )
}

/// One authoritative host.
struct Host {
    app: App,
    slot: HostSlot,
    token: String,
    /// Whether this host is still in the mission. A dropped host stops stepping
    /// and stops ferrying; the survivors reach it only as a host-loss report.
    alive: bool,
}

impl Host {
    fn new(slot: HostSlot) -> Self {
        let args = args();
        let mut app = build_headless_app(&args).expect("app should build");
        let token = format!("crew-{}", slot.slot_id());
        {
            let mut sessions = app.world_mut().resource_mut::<Sessions>();
            sessions
                .0
                .register(token.clone(), format!("Helm {}", slot.0))
                .expect("a fresh token");
            sessions
                .0
                .set_station(&token, Some(StationId(CREWED_STATION.into())));
        }
        let delay = project_phoenix::lockstep::authored_delay(app.world());
        assert!(delay > 0, "the probe world must author a non-zero delay");
        join_fleet(app.world_mut(), roster(slot), delay);
        app.insert_resource(MeshAgreement::new(30));
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

    /// Report to THIS host that `lost`'s ship host has vanished, exactly as the
    /// bridge's `wasm_host_departed` → `drain_mesh_inbound` path does: a
    /// self-addressed `HostLoss` with tick 0, whose real agreed tick the
    /// simulation derives from the lost host's own watermark.
    fn observe_host_loss(&mut self, lost: HostSlot) {
        self.deliver(&[MeshFrame::HostLoss(HostLossFrame {
            from: lost,
            lost,
            tick: 0,
        })]);
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

    /// The host-loss log this host has applied.
    fn host_loss_records(&self) -> Vec<HostLossRecord> {
        self.app
            .world()
            .resource::<PendingHostLoss>()
            .records()
            .to_vec()
    }

    /// Every fleet ship's uuid, keyed by slot.
    fn fleet_ships(&mut self) -> std::collections::BTreeMap<HostSlot, String> {
        let mut q = self.app.world_mut().query::<(&EntityUuid, &FleetSlotOf)>();
        q.iter(self.app.world())
            .map(|(uuid, slot)| (slot.0, uuid.0.clone()))
            .collect()
    }

    /// How many of `slot`'s ship's systems are under HUMAN control, or `None`
    /// when this host has no ship for that slot. A crewed ship has at least one;
    /// a fully backfilled one has zero.
    fn human_systems_of(&mut self, slot: HostSlot) -> Option<usize> {
        let mut q = self
            .app
            .world_mut()
            .query::<(&ShipConfigComponent, &ShipSystemControlSources, &FleetSlotOf)>();
        for (config, sources, slot_of) in q.iter(self.app.world()) {
            if slot_of.0 != slot {
                continue;
            }
            let human = config
                .0
                .systems
                .iter()
                .filter(|s| sources.0.source_for(&s.id) == ControlSource::Human)
                .count();
            return Some(human);
        }
        None
    }

    /// The authoritative world-space position of `slot`'s ship (the folded
    /// `ShipPhysics`, not the presentation `Transform`), or `None`.
    fn ship_position_of(&mut self, slot: HostSlot) -> Option<(f32, f32, f32)> {
        let mut q = self.app.world_mut().query::<(&ShipPhysics, &FleetSlotOf)>();
        for (physics, slot_of) in q.iter(self.app.world()) {
            if slot_of.0 == slot {
                return Some((physics.x, physics.y, physics.z));
            }
        }
        None
    }
}

/// Ferry one round of "everything each LIVE host said reaches every other LIVE
/// host". A dropped host neither sends nor receives.
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

/// Step every LIVE host one frame, having first ferried the previous round.
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

/// Build a three-host fleet and run it in lockstep for `warmup` ticks with each
/// crew steering and throttling its own ship, so all three ships are genuinely
/// under way — and slot 3's under a HUMAN — before it is dropped.
fn warmed_fleet(warmup: u64) -> Vec<Host> {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO), Host::new(SLOT_THREE)];
    let orders: &[(u64, HostSlot, &str, SystemControlPayload)] = &[
        (10, SLOT_ONE, "helm-thrust", thrust(0.8)),
        (10, SLOT_TWO, "helm-thrust", thrust(0.6)),
        (10, SLOT_THREE, "helm-thrust", thrust(0.7)),
        (12, SLOT_ONE, "helm-steering", steer(0.3)),
        (12, SLOT_TWO, "helm-steering", steer(-0.35)),
        (12, SLOT_THREE, "helm-steering", steer(0.2)),
    ];
    for tick in 0..warmup {
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
    }
    // Deliberately NO trailing ferry: every host has PROCESSED every other's
    // frames up to the last delivered tick, so `watermark_of(slot 3)` reads the
    // settled value the host-loss path will itself derive from. A trailing ferry
    // would leave slot 3's most recent frame buffered and unprocessed, so
    // `apply_mesh_inbox` would bump the watermark by one before deriving the
    // disconnect tick — and the test's prediction would be one behind the flip.
    hosts
}

// ── Preconditions ────────────────────────────────────────────────────────────

/// The fleet really is three hosts flying three ships, and slot 3's is crewed —
/// so "flip it to Backfill" is a real change of control, not a no-op.
#[test]
fn three_hosts_fly_three_ships_and_slot_three_is_crewed() {
    let mut hosts = warmed_fleet(40);

    let ships_one = hosts[0].fleet_ships();
    let ships_two = hosts[1].fleet_ships();
    let ships_three = hosts[2].fleet_ships();
    assert_eq!(ships_one.len(), 3, "three fleet ships exist");
    assert_eq!(ships_one, ships_two, "AC-shared identity across hosts");
    assert_eq!(ships_two, ships_three);

    for (host, h) in hosts.iter_mut().enumerate() {
        assert!(
            h.human_systems_of(SLOT_THREE).unwrap() > 0,
            "slot 3's ship must have a live human crew before the drop, or \
             flipping it to Backfill proves nothing (host {host})"
        );
    }
}

// ── The headline ─────────────────────────────────────────────────────────────

/// **AC1–AC4, AC6.** Drop slot 3 mid-mission; the two survivors agree the
/// disconnect tick, flip slot 3's ship to Backfill at exactly that tick, keep
/// its full state, and fold the same authoritative digest on every tick after.
#[test]
fn a_dropped_host_backfills_at_an_agreed_tick_and_survivors_stay_in_lockstep() {
    const WARMUP: u64 = 80;
    const AFTER: u64 = 260;

    let mut hosts = warmed_fleet(WARMUP);

    // Slot 3's last watermark, as the survivors hold it, is the observation the
    // disconnect tick is a function of. Both survivors derive the same tick from
    // it because reliable delivery gave them the same watermark.
    let delay = hosts[0].delay();
    let expected_loss_tick = {
        let session = hosts[0].app.world().resource::<FleetLockstep>();
        agreed_loss_tick(
            session
                .watermark_of(SLOT_THREE)
                .expect("host one heard slot 3"),
        )
    };
    assert_eq!(
        expected_loss_tick,
        {
            let session = hosts[1].app.world().resource::<FleetLockstep>();
            agreed_loss_tick(session.watermark_of(SLOT_THREE).expect("host two heard slot 3"))
        },
        "both survivors must derive the SAME disconnect tick from slot 3's own \
         watermark — the agreement is peer-independent"
    );
    assert!(
        expected_loss_tick > hosts[0].tick(),
        "the disconnect tick is in the future — slot 3's input covers up to \
         watermark {}, so the flip lands seamlessly on the next tick",
        expected_loss_tick - 1
    );

    // Drop slot 3. The survivors will stall on it until they hear the loss.
    hosts[2].alive = false;
    hosts[0].observe_host_loss(SLOT_THREE);
    hosts[1].observe_host_loss(SLOT_THREE);

    // Sanity: a stalled survivor cannot advance until it processes the loss, so
    // one step frees the barrier and the flip lands at the agreed tick.
    let mut digests: [Vec<(u64, u64)>; 2] = [Vec::new(), Vec::new()];
    for _ in 0..AFTER {
        step(&mut hosts);
        digests[0].push((hosts[0].tick(), hosts[0].digest()));
        digests[1].push((hosts[1].tick(), hosts[1].digest()));
    }

    // AC1 + AC3: exactly one host-loss transition, at the agreed tick, on both.
    for (host, h) in hosts.iter().enumerate().take(2) {
        assert_eq!(
            h.host_loss_records(),
            vec![HostLossRecord {
                slot: SLOT_THREE,
                tick: expected_loss_tick,
            }],
            "survivor {host} must record ONE host-loss transition, for slot 3, \
             at the agreed tick {expected_loss_tick}"
        );
    }

    // AC2 + AC3: slot 3's ship still exists, keeps a full crew of systems, and
    // every one of them is now AI — its human-seeking systems re-resolved away
    // from the crew that vanished, the rest flipped by ordinary Backfill.
    for (host, h) in hosts.iter_mut().enumerate().take(2) {
        assert_eq!(
            h.human_systems_of(SLOT_THREE),
            Some(0),
            "survivor {host}: every system on the lost ship must be AI after \
             Backfill — a human system would mean a console nobody is at"
        );
    }

    // AC6: per-tick digest equality holds through the transition and past it.
    let paired: Vec<((u64, u64), (u64, u64))> = digests[0]
        .iter()
        .copied()
        .zip(digests[1].iter().copied())
        .collect();
    for ((tick_a, a), (tick_b, b)) in &paired {
        assert_eq!(tick_a, tick_b, "the survivors step in lockstep");
        assert_eq!(
            a, b,
            "the survivors diverged at tick {tick_a}: {a:#018x} vs {b:#018x} — \
             a mis-timed or asymmetric Backfill flip would show here first"
        );
    }
    assert!(
        paired.iter().any(|((t, _), _)| *t >= expected_loss_tick),
        "the run must continue past the disconnect tick, or the equality above \
         never covers the transition"
    );

    // AC4: the lost ship is not abandoned — under Backfill AI it keeps moving,
    // and identically on both survivors (which the digest equality already
    // proves; this makes the AI activity explicit rather than implied).
    let before = hosts[0].ship_position_of(SLOT_THREE).unwrap();
    for _ in 0..40 {
        step(&mut hosts);
    }
    let after = hosts[0].ship_position_of(SLOT_THREE).unwrap();
    assert_ne!(
        before, after,
        "the backfilled ship stopped moving — an abandoned NPC, not one its AI \
         is flying"
    );
    assert_eq!(
        hosts[0].ship_position_of(SLOT_THREE),
        hosts[1].ship_position_of(SLOT_THREE),
        "both survivors must derive the lost ship's AI to the SAME place — one \
         driving it and one not, or two driving it differently, would diverge"
    );

    // AC6's exchange half: the survivors' periodic digest exchange still agrees.
    for (host, h) in hosts.iter().enumerate().take(2) {
        let agreement = h.app.world().resource::<MeshAgreement>();
        assert!(
            agreement.agreed(),
            "survivor {host} reported a divergence after the drop: {:?}",
            agreement.first_disagreement()
        );
    }

    // Anti-vacuity: the run actually simulated something worth agreeing about.
    let distinct: std::collections::BTreeSet<u64> =
        digests[0].iter().map(|(_, d)| *d).collect();
    assert!(
        distinct.len() > AFTER as usize / 2,
        "only {} distinct digests — the mission is coasting, not simulating",
        distinct.len()
    );
    let _ = delay;
}

// ── AC5: convergence under reordered, duplicate and delayed reports ──────────

/// **AC5.** Two survivors that hear the loss at different times, one that hears
/// it twice, and a report that arrives late all converge on the one transition
/// at the one tick.
///
/// Survivor 1 hears the loss immediately and duplicated; survivor 2 hears it
/// many frames later. They still flip slot 3 at the identical tick and fold the
/// identical digest on every shared tick — because the tick is a function of
/// slot 3's watermark, not of when either survivor noticed.
#[test]
fn reordered_duplicate_and_delayed_loss_reports_converge() {
    const WARMUP: u64 = 80;

    let mut hosts = warmed_fleet(WARMUP);
    let expected_loss_tick = {
        let session = hosts[0].app.world().resource::<FleetLockstep>();
        agreed_loss_tick(session.watermark_of(SLOT_THREE).unwrap())
    };

    hosts[2].alive = false;

    // Survivor 1 hears it at once — and again, a duplicate in the same batch.
    hosts[0].observe_host_loss(SLOT_THREE);
    hosts[0].observe_host_loss(SLOT_THREE);

    // Run a while. Survivor 1 proceeds; survivor 2 is stalled on slot 3, but
    // keeps hearing survivor 1's frames so it can catch up once it is freed.
    let mut digests: [std::collections::BTreeMap<u64, u64>; 2] =
        [Default::default(), Default::default()];
    for _ in 0..60 {
        step(&mut hosts);
        digests[0].insert(hosts[0].tick(), hosts[0].digest());
        digests[1].insert(hosts[1].tick(), hosts[1].digest());
    }
    assert!(
        hosts[0].tick() > expected_loss_tick,
        "survivor 1 should have run past the disconnect tick while survivor 2 \
         waited: {} vs loss {expected_loss_tick}",
        hosts[0].tick()
    );

    // Survivor 2 finally hears it — late, and after slot 3's own last frame has
    // long since arrived. It must derive the SAME tick and catch up.
    hosts[1].observe_host_loss(SLOT_THREE);
    // A third, redundant report to survivor 1 for a slot already backfilled: a
    // pure no-op.
    hosts[0].observe_host_loss(SLOT_THREE);
    for _ in 0..120 {
        step(&mut hosts);
        digests[0].insert(hosts[0].tick(), hosts[0].digest());
        digests[1].insert(hosts[1].tick(), hosts[1].digest());
    }

    // One transition, same tick, on both — whatever the report history.
    for (host, h) in hosts.iter().enumerate().take(2) {
        assert_eq!(
            h.host_loss_records(),
            vec![HostLossRecord {
                slot: SLOT_THREE,
                tick: expected_loss_tick,
            }],
            "survivor {host}: reordered/duplicate/delayed reports must still \
             yield exactly one transition at the agreed tick"
        );
    }

    // And every tick both survivors folded, they folded the same — the delayed
    // survivor caught up to a bit-identical history, not a plausible-looking
    // parallel one.
    let mut shared = 0usize;
    for (tick, a) in &digests[0] {
        if let Some(b) = digests[1].get(tick) {
            shared += 1;
            assert_eq!(
                a, b,
                "the survivors disagree at tick {tick} despite converging on \
                 the same transition: {a:#018x} vs {b:#018x}"
            );
        }
    }
    assert!(
        shared > 40,
        "the survivors barely overlapped ({shared} shared ticks) — the \
         comparison is too thin to prove convergence"
    );
    assert!(
        digests[1].keys().any(|t| *t >= expected_loss_tick),
        "survivor 2 must have caught up past the disconnect tick"
    );
}

// ── The production STAR, not a symmetric mesh ────────────────────────────────

/// **HIGH determinism guard (issue #1119).** The other cases model a symmetric
/// mesh: every survivor both directly observes slot 3's frames and self-reports
/// the loss, and slot 3's frames are all processed *before* the loss. Production
/// is a STAR — only the connection-holder (here the "lead", slot 1) sees the
/// socket close, and it sees slot 3's FINAL tick frame in the very same inbox
/// batch as that close; every other host (the "member", slot 2) learns the loss
/// only from the lead's relayed report, which the reliable ordered relay
/// delivers *after* slot 3's relayed final frame.
///
/// The lead must therefore observe slot 3's final watermark BEFORE it derives
/// the loss tick, or it flips the ship to Backfill one tick earlier than the
/// member does and the two folds diverge. `order_mesh_inbound` is where the
/// bridge guarantees that order (decoded frames before the self-loss); this test
/// drives the lead's batch through it, so reversing that order — the pre-fix
/// bug — makes the lead derive `watermark_prev + 1` while the member derives
/// `watermark_final + 1`, the asserts below fail, and the divergence surfaces.
#[test]
fn a_star_lead_and_member_agree_the_same_tick_when_the_final_frame_co_arrives() {
    const WARMUP: u64 = 80;
    const AFTER: u64 = 220;

    let mut hosts = warmed_fleet(WARMUP);

    // Slot 3's un-ferried FINAL batch: produced by its last step in
    // `warmed_fleet` and not yet delivered (there is no trailing ferry). This is
    // the frame that co-arrives with the socket close on the lead and is relayed
    // to the member ahead of the loss report.
    let slot3_final = hosts[2].drain_outbox();
    let slot3_final_watermark = slot3_final
        .iter()
        .find_map(|f| match f {
            MeshFrame::Tick(t) => Some(t.ready_through),
            _ => None,
        })
        .expect("slot 3's final batch must carry a tick frame with its watermark");
    // The one true agreed tick: the first tick past slot 3's LAST watermark —
    // the one that includes the co-arriving final frame. The pre-fix lead would
    // derive one lower (from the watermark before that frame).
    let expected_loss_tick = agreed_loss_tick(slot3_final_watermark);

    hosts[2].alive = false;

    // LEAD (slot 1): the self-observer. Its relay of slot 3's final frame and its
    // own socket close land in ONE inbox batch, in the bridge's push order.
    {
        let mut inbox = hosts[0].app.world_mut().resource_mut::<MeshInbox>();
        for frame in order_mesh_inbound(slot3_final.clone(), [SLOT_THREE]) {
            inbox.push(frame);
        }
    }

    // MEMBER (slot 2): sees slot 3's final frame first (reliable-ordered relay);
    // it will learn the loss only from the lead's re-broadcast, which reaches it
    // through the ordinary ferry below — strictly after this frame.
    hosts[1].deliver(&slot3_final);

    let mut digests: [std::collections::BTreeMap<u64, u64>; 2] =
        [Default::default(), Default::default()];
    for _ in 0..AFTER {
        step(&mut hosts);
        digests[0].insert(hosts[0].tick(), hosts[0].digest());
        digests[1].insert(hosts[1].tick(), hosts[1].digest());
    }

    // Both derive the SAME agreed tick — the one past slot 3's final watermark.
    let one = HostLossRecord {
        slot: SLOT_THREE,
        tick: expected_loss_tick,
    };
    assert_eq!(
        hosts[0].host_loss_records(),
        vec![one],
        "the LEAD must derive the tick past slot 3's FINAL watermark — observing \
         the co-arriving frame before deriving the loss, not one tick early"
    );
    assert_eq!(
        hosts[1].host_loss_records(),
        vec![one],
        "the MEMBER, learning the loss from the lead's relayed report, must derive \
         the identical tick"
    );

    // Slot 3's ship is Backfill on both, and folds bit-identically every shared
    // tick through the transition — a one-tick-early flip on the lead would show
    // here as the first divergence.
    for (host, h) in hosts.iter_mut().enumerate().take(2) {
        assert_eq!(
            h.human_systems_of(SLOT_THREE),
            Some(0),
            "host {host}: slot 3's ship must be fully Backfill after the flip"
        );
    }
    let mut shared = 0usize;
    for (tick, a) in &digests[0] {
        if let Some(b) = digests[1].get(tick) {
            shared += 1;
            assert_eq!(
                a, b,
                "lead and member diverged at tick {tick}: {a:#018x} vs {b:#018x} \
                 — the star's asymmetric observation flipped Backfill on different \
                 ticks"
            );
        }
    }
    assert!(
        digests[0].keys().any(|t| *t >= expected_loss_tick) && shared > 40,
        "the run must cover the transition on both hosts ({shared} shared ticks)"
    );
}

// ── Determinism under async relay watermark skew ─────────────────────────────

/// **HIGH determinism guard (issue #1119).** Honest survivors legitimately hold
/// DIFFERENT watermarks for a slot at any instant — the relay lead sees a peer's
/// frame before it forwards it, so `watermark_of(lost)` is skew-prone across
/// survivors while the slot is live or its loss is still propagating. The first
/// fix round decided accept/refuse AND derived the flip tick EAGERLY against that
/// raw local watermark, so a `HostLoss` whose tick fell BETWEEN two survivors'
/// watermarks made one survivor refuse it (`watermark >= tick`) and the other
/// accept it (`watermark < tick`): one flipped slot 3's ship to Backfill and the
/// other kept it human — a permanent fold divergence with no reconciliation path.
/// The same flaw bit a genuine relayed departure: a survivor one frame ahead of
/// the detector refused the real loss the detector agreed.
///
/// The fix removes that skew-prone eager decision. A relayed report's tick is
/// honoured VERBATIM (only the single self-observing connection holder derives a
/// tick, from its own watermark), so both survivors adopt the identical tick
/// whatever their own watermark, and both flip at exactly it. This asserts they
/// reach the SAME decision AND fold bit-identically afterwards. On the pre-fix
/// code they diverge — one refuses and stays human, the other flips to Backfill —
/// and both the record and the per-tick digest assertions below fail.
///
/// The report is deliberately for a slot the survivors have NOT genuinely lost at
/// that tick (its watermark straddles the claimed tick): rejecting such a report
/// is a security question deferred to reporter authentication (#1118/#1120) and
/// cannot be answered deterministically here. #1119's guarantee is the one this
/// pins — whatever the survivors do with a report, they do it alike.
#[test]
fn a_relayed_loss_whose_tick_straddles_two_survivors_watermarks_converges() {
    const WARMUP: u64 = 80;
    const AFTER: u64 = 160;

    let mut hosts = warmed_fleet(WARMUP);

    // Slot 3 stops ferrying so the two survivors are the only comparison; the
    // report about it is therefore premature, not a genuine departure.
    hosts[2].alive = false;

    // Survivor 1 (host index 1) holds slot 3 at its settled post-warmup
    // watermark. Manufacture the async relay skew: survivor 0 (host index 0) has
    // heard slot 3 through a much HIGHER watermark — exactly as a relay lead holds
    // a higher watermark than a member it has not yet forwarded the frame to.
    let w_low = hosts[1]
        .app
        .world()
        .resource::<FleetLockstep>()
        .watermark_of(SLOT_THREE)
        .expect("survivor 1 heard slot 3");
    // Comfortably past the whole run, so pre-fix survivor 0 (which refuses and so
    // never departs slot 3) still never stalls waiting for it.
    let w_high = w_low + (WARMUP + AFTER);
    hosts[0]
        .app
        .world_mut()
        .resource_mut::<FleetLockstep>()
        .observe(SLOT_THREE, w_high);

    // The straddling tick: above survivor 1's watermark (pre-fix it ACCEPTS) and
    // at/below survivor 0's (pre-fix it REFUSES).
    let straddle_tick = w_low + 2;
    assert!(
        straddle_tick > w_low && straddle_tick <= w_high,
        "the report tick must straddle the survivors' watermarks: \
         {w_low} < {straddle_tick} <= {w_high}"
    );

    // A relayed report (a survivor's re-broadcast, tick > 0 — NOT a tick-0
    // self-observation), naming slot 3 lost at the straddling tick.
    let relayed = MeshFrame::HostLoss(HostLossFrame {
        from: SLOT_ONE,
        lost: SLOT_THREE,
        tick: straddle_tick,
    });
    hosts[0].deliver(std::slice::from_ref(&relayed));
    hosts[1].deliver(std::slice::from_ref(&relayed));

    let mut digests: [std::collections::BTreeMap<u64, u64>; 2] =
        [Default::default(), Default::default()];
    for _ in 0..AFTER {
        step(&mut hosts);
        digests[0].insert(hosts[0].tick(), hosts[0].digest());
        digests[1].insert(hosts[1].tick(), hosts[1].digest());
    }

    // Same decision: BOTH survivors flip slot 3 to Backfill at the identical
    // carried tick — never one refusing and one accepting.
    let one = HostLossRecord {
        slot: SLOT_THREE,
        tick: straddle_tick,
    };
    for (host, h) in hosts.iter().enumerate().take(2) {
        assert_eq!(
            h.host_loss_records(),
            vec![one],
            "survivor {host} must honour the relayed tick verbatim — a skew-prone \
             refuse/accept split is exactly the divergence this guards"
        );
    }
    for (host, h) in hosts.iter_mut().enumerate().take(2) {
        assert_eq!(
            h.human_systems_of(SLOT_THREE),
            Some(0),
            "survivor {host}: slot 3's ship must be fully Backfill after the flip"
        );
    }

    // Bit-identical folds: every tick both survivors folded, they folded the
    // same. Pre-fix one keeps slot 3 human and the other flips it, so the two
    // digests diverge from the flip tick on and this fails.
    let mut shared = 0usize;
    for (tick, a) in &digests[0] {
        if let Some(b) = digests[1].get(tick) {
            shared += 1;
            assert_eq!(
                a, b,
                "survivors diverged at tick {tick}: {a:#018x} vs {b:#018x} — a \
                 skew-prone accept/refuse split flipped Backfill on one host only"
            );
        }
    }
    assert!(
        digests[0].keys().any(|t| *t >= straddle_tick) && shared > AFTER as usize / 2,
        "the run must cover the transition on both survivors ({shared} shared ticks)"
    );
}
