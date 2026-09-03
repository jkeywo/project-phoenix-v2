//! Two authoritative hosts, one deterministic mission (issue #1116).
//!
//! The crown proof for the whole issue, and the foundation #1090 scales to N.
//! Two complete headless simulations run in one process with a **synchronous
//! in-process mesh** between them — everything the browser's DataChannel would
//! carry, and nothing it would not — while a different crew drives each. After
//! every tick both worlds are folded and compared.
//!
//! # Why an in-process channel proves something
//!
//! It removes exactly one thing: the socket. What it keeps is everything that
//! can actually make two hosts disagree — two independent `App`s, two `Sessions`
//! tables that know nothing of each other, two `SimRng`s, two `WorldIdMint`s,
//! two schedules, `LocalShip` on a different ship in each, and a real one-tick
//! delivery latency that the agreed `CommandDelay` has to cover. A test that
//! could not disagree would prove nothing; each assertion below names what would
//! break it.
//!
//! The transport is deliberately not modelled here at all: `lockstep` owns no
//! socket, a transport fills [`MeshInbox`] and drains [`MeshOutbox`], and the
//! ferry below is that transport in five lines.
//!
//! # Why this is its own test binary
//!
//! `--deterministic` pins the scheduler with a one-thread `TaskPoolOptions`, and
//! Bevy's task pools are process-global, fixed by whichever app builds first. A
//! two-host agreement claim made in a binary where forty other tests build apps
//! is a claim about whoever won that race. Same reason as
//! `tests/local_ship_neutrality.rs`, `tests/archetype_order_determinism.rs` and
//! `tests/entity_id_minting.rs`. Do not add unrelated tests here.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::command_admission::log::{CommandOrder, ShipKey};
use project_phoenix::command_admission::{CommandDelay, CommandLog, HostSlot};
use project_phoenix::core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId};
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lobby::{InboundMessage, Sessions};
use project_phoenix::lockstep::{
    join_fleet, FleetLockstep, FleetRoster, FleetShip, FleetSlotOf, MeshAgreement, MeshCommand,
    MeshDiagnostics, MeshFrame, MeshInbox, MeshOutbox, TickFrame,
};
use project_phoenix::server_app::LocalShip;
use project_phoenix::sim_tick::SimTick;

/// Two crewed hulls and a hostile, so the comparison covers combat: per-victim
/// RNG draws, mid-run projectile mints, and helm decisions that answer to input.
const WORLD: &str = "assets/worlds/probe_fleet_duel.toml";

/// The hull every host in the fleet flies. A single template path, so both
/// slots take the same ship and the only variable between the hosts is which
/// one each projects. (The authored fleet delay is NOT a constant here — it is
/// read back from the loaded config via `authored_delay()` so the test cannot
/// quietly disagree with the world's `[global] command_delay_ticks`.)
const SHIP: &str = "assets/entities/alliance_cruiser.toml";

const SEED: u64 = 1_116_116;

/// Long enough for both crews' orders to have applied, the ships to have closed
/// with the hostile and weapons to have cycled.
const TICKS: u64 = 1200;

const SLOT_ONE: HostSlot = HostSlot(1);
const SLOT_TWO: HostSlot = HostSlot(2);

/// The station each crew sits at. Helm, because a steering order is the
/// shortest path from "a human pressed a key" to a number the digest folds.
const CREWED_STATION: &str = "helm";
/// The Rating the fleet froze that seat at. `Std` automates nothing, so the
/// systems the station owns answer to the human rather than to Backfill AI.
const CREWED_RATING: &str = "Std";

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: SHIP.into(),
        max_ticks: TICKS,
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
    FleetRoster::new(vec![crewed(SLOT_ONE), crewed(SLOT_TWO)], local)
}

/// One authoritative host: its app, its slot, and the token of the one crew
/// member connected to it.
struct Host {
    app: App,
    slot: HostSlot,
    token: String,
}

impl Host {
    /// Build a host, seat its own crew, and join it to the fleet — all before
    /// the first `update()`, because `headless_auto_start` enters `InProgress`
    /// on the first fixed step and the ships are spawned by that transition.
    fn new(slot: HostSlot) -> Self {
        let args = args();
        let mut app = build_headless_app(&args).expect("app should build");
        // This host's own crew, and ONLY this host's own crew. The other ship's
        // crew is connected to the other machine and is invisible here — which
        // is the property `each_hosts_sessions_stay_its_own` asserts.
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
        assert!(
            delay > 0,
            "the probe world must author a non-zero `[global] command_delay_ticks`: \
             a fleet with no delay has no window in which to receive a peer's \
             input for the tick it is about to simulate"
        );
        join_fleet(app.world_mut(), roster(slot), delay);
        // A short digest cadence, so the exchange is exercised several times
        // inside a probe-length run rather than once. The shipped cadence is a
        // diagnostic interval and moves nothing in the simulation.
        app.insert_resource(MeshAgreement::new(60));
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

    fn log(&self) -> &CommandLog {
        self.app.world().resource::<CommandLog>()
    }

    /// Everything this host wants to say to the fleet since it last spoke.
    fn drain_outbox(&mut self) -> Vec<MeshFrame> {
        self.app.world_mut().resource_mut::<MeshOutbox>().drain()
    }

    /// Deliver frames from the rest of the fleet.
    fn deliver(&mut self, frames: &[MeshFrame]) {
        let mut inbox = self.app.world_mut().resource_mut::<MeshInbox>();
        for frame in frames {
            inbox.push(frame.clone());
        }
    }

    /// This host's own crew presses a key.
    ///
    /// Written as an `InboundMessage`, which is precisely where the browser
    /// bridge's `drain_inbound` writes one — so the command crosses the real
    /// admission boundary, is judged by the real authority predicate against
    /// this host's own `Sessions`, and is stamped and logged by the real seam.
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

    /// The uuid of the ship this host projects to its own crew.
    fn local_ship_uuid(&mut self) -> String {
        let mut q = self
            .app
            .world_mut()
            .query_filtered::<&EntityUuid, With<LocalShip>>();
        q.iter(self.app.world())
            .next()
            .expect("a host projects one ship")
            .0
            .clone()
    }

    /// Every fleet ship's uuid, keyed by slot.
    fn fleet_ships(&mut self) -> std::collections::BTreeMap<HostSlot, String> {
        let mut q = self.app.world_mut().query::<(&EntityUuid, &FleetSlotOf)>();
        q.iter(self.app.world())
            .map(|(uuid, slot)| (slot.0, uuid.0.clone()))
            .collect()
    }
}

/// The mesh: one round of "everything each host said reaches every other host".
///
/// Frames produced during tick *T* are delivered before tick *T + 1*, so
/// delivery costs a full tick of latency — which is the point. The agreed
/// `CommandDelay` is what has to cover it, and a fleet whose delay did not
/// would stall (`a_host_that_has_not_heard_from_its_peer_waits`).
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

/// Step every host one tick, having first delivered the previous tick's traffic.
fn step(hosts: &mut [Host]) {
    ferry(hosts);
    for host in hosts.iter_mut() {
        run(&mut host.app, 1);
    }
}

/// A steering order, as a crew member's key press reaches admission.
fn steer(value: f32) -> SystemControlPayload {
    SystemControlPayload::SetSteering { value }
}

fn thrust(value: f32) -> SystemControlPayload {
    SystemControlPayload::SetThrust { value }
}

/// A two-host fleet, stepped `ticks` ticks, with each crew's orders injected on
/// the ticks named in `orders`.
///
/// Returns the two hosts and the per-tick digests each folded.
fn run_mission(
    ticks: u64,
    orders: &[(u64, HostSlot, &str, SystemControlPayload)],
) -> (Vec<Host>, Vec<Vec<(u64, u64)>>) {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO)];
    let mut digests: Vec<Vec<(u64, u64)>> = vec![Vec::new(), Vec::new()];

    for tick in 0..ticks {
        for (at, slot, target, payload) in orders {
            if *at == tick {
                let host = hosts
                    .iter_mut()
                    .find(|h| h.slot == *slot)
                    .expect("the order names a host in the fleet");
                host.crew_command(target, payload.clone());
            }
        }
        step(&mut hosts);
        for (i, host) in hosts.iter().enumerate() {
            digests[i].push((host.tick(), host.digest()));
        }
    }
    (hosts, digests)
}

/// The orders both crews give: each one steers and throttles its own ship, at
/// staggered ticks so the two hosts' traffic genuinely interleaves within a
/// tick rather than arriving in tidy alternation.
fn mission_orders() -> Vec<(u64, HostSlot, &'static str, SystemControlPayload)> {
    vec![
        (10, SLOT_ONE, "helm-thrust", thrust(0.8)),
        (10, SLOT_TWO, "helm-thrust", thrust(0.6)),
        (10, SLOT_ONE, "helm-steering", steer(0.35)),
        (11, SLOT_TWO, "helm-steering", steer(-0.4)),
        (60, SLOT_ONE, "helm-steering", steer(-0.2)),
        (60, SLOT_TWO, "helm-thrust", thrust(1.0)),
        (140, SLOT_TWO, "helm-steering", steer(0.5)),
        (200, SLOT_ONE, "helm-thrust", thrust(0.3)),
    ]
}

// ── The preconditions ────────────────────────────────────────────────────────

/// Before anything else: the fleet really is two hosts flying two ships, each
/// projecting its own, each carrying only its own crew.
///
/// AC1 and AC5's setup. Every later assertion is vacuous without it.
#[test]
fn each_host_flies_its_own_ship_and_knows_only_its_own_crew() {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO)];
    for _ in 0..30 {
        step(&mut hosts);
    }

    let ships_one = hosts[0].fleet_ships();
    let ships_two = hosts[1].fleet_ships();
    assert_eq!(ships_one.len(), 2, "both fleet ships exist on host one");
    assert_eq!(
        ships_one, ships_two,
        "AC3: both hosts minted the SAME identity for the same ship. They \
         spawned from one frozen roster at one agreed tick, so `WorldIdMint` \
         gave each hull the same (namespace, tick, seq) on both machines — a \
         ship spawned locally rather than from agreed work would carry an id no \
         other host has"
    );

    assert_eq!(hosts[0].local_ship_uuid(), ships_one[&SLOT_ONE]);
    assert_eq!(hosts[1].local_ship_uuid(), ships_two[&SLOT_TWO]);
    assert_ne!(
        hosts[0].local_ship_uuid(),
        hosts[1].local_ship_uuid(),
        "each crew is aboard its OWN ship — if both hosts projected the same \
         hull this would be one crew with two screens, not a fleet"
    );

    // AC5: a host's session table is its own crew and nobody else's. The other
    // ship's crew never identified here, holds no station here, and its token —
    // a bearer credential — has never crossed the mesh.
    for (host, other) in [(0_usize, 1_usize), (1, 0)] {
        let sessions = hosts[host].app.world().resource::<Sessions>();
        let mine = hosts[host].token.clone();
        let theirs = hosts[other].token.clone();
        assert!(
            sessions.0.players().iter().any(|p| p.token == mine),
            "a host must know its own crew"
        );
        assert!(
            !sessions.0.players().iter().any(|p| p.token == theirs),
            "host {host} learned the other crew's session token — it is a bearer \
             credential, and the whole reason a mesh command carries a ShipKey \
             instead of a token is that it must never cross the wire"
        );
    }
}

/// The amendment, observed rather than asserted about: the fleet runs with a
/// non-zero `CommandDelay`, taken from the world's authored value.
///
/// AC2's second half. AGENTS.md rule 7 said a non-zero delay would be the
/// deliberate amendment of "commands apply the tick they are admitted"; this is
/// what that amendment looks like from outside.
#[test]
fn a_fleet_runs_on_the_worlds_authored_delay_and_a_lone_host_does_not() {
    let host = Host::new(SLOT_ONE);
    let authored = project_phoenix::lockstep::authored_delay(host.app.world());
    assert!(authored > 0);
    assert_eq!(
        host.delay(),
        authored,
        "a fleet host stamps its crew's commands `command_delay_ticks` into the \
         future — the number the MISSION authored, not one the engine invented"
    );

    // …and a lone host takes nothing. A fleet of one waits for nobody, so a
    // delay would be latency bought against an empty set of peers.
    let args = args();
    let mut solo = build_headless_app(&args).expect("app should build");
    join_fleet(solo.world_mut(), FleetRoster::default(), authored);
    assert_eq!(
        solo.world().resource::<CommandDelay>().0,
        0,
        "single-player play must not acquire an input delay from a world that \
         authors one for fleets"
    );
}

// ── The headline ─────────────────────────────────────────────────────────────

/// **AC1, AC4 and AC6.** Two crews, two hosts, one mission — and the
/// authoritative fold agrees on every tick of it.
///
/// Each crew's orders enter its OWN host through the production admission
/// boundary and reach the other host only as agreed, ordered, tick-stamped mesh
/// traffic. Every NPC is derived independently by both hosts from the same
/// ticks, the same seeded streams and the same authored policies — nothing about
/// the hostile crosses the wire, because nothing about it needs to.
///
/// The walk is per tick rather than end-to-end because a fleet that diverged at
/// tick 200 and reconverged by 480 still showed two crews different worlds for
/// four seconds.
#[test]
fn two_crews_on_two_hosts_play_one_mission_and_agree_on_every_tick() {
    let (hosts, digests) = run_mission(TICKS, &mission_orders());

    if let Some((tick, mine, theirs)) = digests[0]
        .iter()
        .zip(digests[1].iter())
        .find(|((_, a), (_, b))| a != b)
        .map(|((tick, a), (_, b))| (*tick, *a, *b))
    {
        panic!(
            "the fleet diverged at tick {tick}: slot 1 folds {mine:#018x}, slot 2 \
             folds {theirs:#018x}.\n\n\
             Compare the two hosts' `CommandLog`s first — they are \
             byte-identical on hosts that agree, so a difference there names the \
             input that split them and a match rules input out. \
             `sim_digest::digest_stages` then names which scope of the fold \
             disagrees."
        );
    }

    // Anti-vacuity, three ways. Without these the equality above could be two
    // empty worlds, two unmanoeuvring hulls, or two hosts that ignored their
    // crews.
    let distinct: std::collections::BTreeSet<u64> = digests[0].iter().map(|(_, d)| *d).collect();
    assert!(
        distinct.len() > TICKS as usize / 2,
        "only {} distinct digests over {TICKS} ticks — the mission is not \
         simulating anything worth agreeing about",
        distinct.len()
    );
    assert_eq!(
        hosts[0].log().len(),
        mission_orders().len(),
        "every crew order must have been admitted and applied on both hosts — \
         a refused order would make this a test of two hosts agreeing to ignore \
         their crews"
    );
    assert!(
        hosts[0].log().ticks_are_monotonic(),
        "the log records commands as they apply, so its ticks cannot go \
         backwards — one that did would be unreplayable"
    );

    // …and the mission had combat in it. Agreement over two hulls coasting is a
    // much weaker claim than agreement over per-victim damage draws and mid-run
    // projectile mints, which are the first two things a peer that ordered a
    // tick differently would diverge on.
    let mut hosts = hosts;
    let projectiles = hosts[0]
        .app
        .world()
        .resource::<project_phoenix::world_id::WorldIdMint>()
        .minted_so_far(project_phoenix::world_id::IdNamespace::Projectile);
    let hurt = {
        let mut q = hosts[0]
            .app
            .world_mut()
            .query::<&project_phoenix::entities::spawner::EntitySystemHull>();
        q.iter(hosts[0].app.world())
            .any(|h| h.0.total_current() < h.0.total_max())
    };
    assert!(
        projectiles > 0 || hurt,
        "the mission never fired a shot or took a hit ({projectiles} projectiles \
         minted), so the agreement above is about ships flying in a straight \
         line — check the probe world still puts a hostile in range"
    );
}

/// **AC2 and AC6.** The two hosts' command logs are byte-identical.
///
/// A far stronger claim than "they agree", and the one that makes the digest
/// comparison diagnosable: each host accepted its own crew's orders locally and
/// the other's off the mesh, at different moments and in different arrival
/// orders, and still wrote the same record. That can only be true if the total
/// order is peer-independent — it is `(origin slot, that slot's own sequence)`,
/// computable from the key alone — and if the log is written when a command
/// APPLIES rather than when it is accepted.
///
/// It is also the first thing to look at when a fleet does diverge: identical
/// logs rule input out and point at the simulation; different logs name the
/// command.
#[test]
fn both_hosts_write_the_same_command_log() {
    let (hosts, _) = run_mission(TICKS, &mission_orders());

    assert_eq!(
        hosts[0].log(),
        hosts[1].log(),
        "two hosts of one mission must record the same commands in the same \
         order, with the same fleet order on each"
    );

    let entries = hosts[0].log().entries();
    assert!(!entries.is_empty(), "precondition: the crews gave orders");

    // Both crews are represented, and each order is stamped under its OWN
    // host's slot — a receiver that renumbered a sender's traffic would show
    // every entry under one origin.
    let origins: std::collections::BTreeSet<HostSlot> =
        entries.iter().map(|e| e.order.origin).collect();
    assert_eq!(
        origins,
        [SLOT_ONE, SLOT_TWO].into_iter().collect(),
        "both crews' orders must be in the record, each under its own slot"
    );

    // And each order landed on the ship its own crew flies — the ShipKey route,
    // not "whatever this host calls its local ship".
    let mut ships = hosts;
    let fleet = ships[0].fleet_ships();
    for entry in ships[0].log().entries() {
        assert_eq!(
            entry.ship.0, fleet[&entry.order.origin],
            "an order from {:?} applied to a ship that is not that slot's — a \
             fleet in which one crew can fly another's hull is not a fleet",
            entry.order.origin
        );
    }
}

/// **AC2's first half, observed.** A crew order applies on the agreed future
/// tick — the same tick on both hosts — rather than on the tick it was admitted.
///
/// The delay is what makes lockstep possible, so it is asserted as a fact about
/// the run rather than inferred from the digests agreeing.
#[test]
fn a_crew_order_applies_on_the_agreed_future_tick_on_both_hosts() {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO)];
    for _ in 0..20 {
        step(&mut hosts);
    }

    // The tick admission will actually stamp against: `SimTick` read between
    // updates is the number of completed steps, which is the index of the step
    // about to run. Taken rather than assumed, because the first `update()` of
    // a headless app establishes the time baseline and steps nothing, so a
    // loop count is one ahead of the clock.
    let admitted_on = hosts[0].tick();
    let delay = hosts[0].delay();
    hosts[0].crew_command("helm-steering", steer(0.5));
    for _ in 0..(delay + 30) {
        step(&mut hosts);
    }

    for host in &hosts {
        let entries = host.log().entries();
        assert_eq!(entries.len(), 1, "one order, one record");
        assert_eq!(
            entries[0].tick,
            admitted_on + delay,
            "the order was admitted on tick {admitted_on} and must apply \
             {delay} ticks later — on BOTH hosts, which is the whole of what \
             the fleet agreed a delay for"
        );
    }
    assert!(
        delay > 0,
        "…and the gap has to be a real one: with a zero delay this test would \
         be asserting the pre-#1116 behaviour"
    );
}

/// **AC6.** The periodic digest exchange stays equal through a normal run, and
/// both hosts say so.
#[test]
fn the_periodic_digest_exchange_agrees_throughout() {
    let (hosts, _) = run_mission(TICKS, &mission_orders());

    for host in &hosts {
        let agreement = host.app.world().resource::<MeshAgreement>();
        assert!(
            agreement.local.checkpoints.len() > 3,
            "precondition: the exchange must have sampled several times, or \
             `agreed()` is a claim about nothing (got {})",
            agreement.local.checkpoints.len()
        );
        assert!(
            !agreement.peers.is_empty(),
            "each host must have HEARD its peer's digests — an exchange nobody \
             receives cannot detect anything"
        );
        assert!(
            agreement.agreed(),
            "the fleet reported a divergence in a run that agreed on every \
             tick: {:?}",
            agreement.first_disagreement()
        );
    }
}

/// **AC6's other half.** An injected mismatch is *caught* — the exchange is not
/// a comparison that always passes.
///
/// The perturbation is applied to one host's ledger rather than to its world,
/// deliberately: what is under test is the detector, and corrupting a world
/// would test the simulation's sensitivity instead. Recovering from the
/// mismatch is #1118's; naming it is this issue's.
#[test]
fn an_injected_mismatch_is_named_with_its_tick_and_its_peer() {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO)];
    for host in hosts.iter_mut() {
        host.app.insert_resource(MeshAgreement::new(10));
    }
    for _ in 0..30 {
        step(&mut hosts);
    }

    // One host's recorded sample for a tick both have passed, quietly wrong.
    // Edited in place rather than re-recorded: `DigestLedger::record` refuses a
    // duplicate for a tick it already holds, which is the guard that keeps a
    // frame running several fixed steps from double-sampling.
    let sampled = {
        let agreement = hosts[0].app.world().resource::<MeshAgreement>();
        *agreement
            .local
            .checkpoints
            .last()
            .expect("the exchange has sampled")
    };
    {
        let mut agreement = hosts[1].app.world_mut().resource_mut::<MeshAgreement>();
        let corrupted = agreement
            .local
            .checkpoints
            .iter_mut()
            .find(|c| c.tick == sampled.tick)
            .expect("both hosts sample the same ticks — they share an interval");
        corrupted.digest ^= 0xff;
    }
    // Host one re-states that sample; host two compares and must object.
    hosts[1].deliver(&[MeshFrame::Digest(project_phoenix::lockstep::DigestFrame {
        from: SLOT_ONE,
        tick: sampled.tick,
        digest: sampled.digest,
    })]);
    run(&mut hosts[1].app, 1);

    let found = hosts[1]
        .app
        .world()
        .resource::<MeshAgreement>()
        .first_disagreement()
        .expect("a mismatched digest must be reported, not absorbed");
    assert_eq!(found.tick, sampled.tick, "the report names the tick");
    assert_eq!(found.peer, SLOT_ONE, "…and the host that disagreed");
    assert_ne!(found.local_digest, found.peer_digest);
}

// ── The barrier ──────────────────────────────────────────────────────────────

/// **The honest wait.** A host that has not heard from its peer withholds the
/// tick rather than speculating, and says which peer it is waiting for.
///
/// `p2p-delta-tick-is-fixedupdate` rules out both a second accumulator beside
/// `Time<Fixed>` and a stall that skips systems inside a tick. This one pauses
/// `Time<Virtual>`, which starves the fixed accumulator — so the withheld tick
/// never begins, and `SimTick` simply does not advance.
#[test]
fn a_host_that_has_not_heard_from_its_peer_waits() {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO)];
    // A few ticks of ordinary lockstep first, so the stall below is a change of
    // behaviour rather than the state it started in.
    for _ in 0..20 {
        step(&mut hosts);
    }
    let running_tick = hosts[0].tick();
    assert!(running_tick > 0, "precondition: the fleet was running");
    assert!(
        !hosts[0]
            .app
            .world()
            .resource::<MeshDiagnostics>()
            .is_stalled(),
        "precondition: it was not already stalled"
    );

    // Slot 2 falls silent. Slot 1 may run on for exactly the delay window — the
    // ticks slot 2 has already promised it will never speak for again — and then
    // must stop.
    let delay = hosts[0].delay();
    for _ in 0..(delay + 6) {
        hosts[0].drain_outbox();
        run(&mut hosts[0].app, 1);
    }
    let stalled_tick = hosts[0].tick();

    assert!(
        stalled_tick <= running_tick + delay + 1,
        "slot 1 ran to tick {stalled_tick} from {running_tick} with a silent \
         peer and a delay of {delay}: it speculated past the input it does not \
         have"
    );
    let diagnostics = hosts[0].app.world().resource::<MeshDiagnostics>().clone();
    assert!(
        diagnostics.is_stalled(),
        "a host withholding a tick has to SAY so — a silent stall is \
         indistinguishable from a crash"
    );
    let stall = diagnostics.last_stall.expect("the stall is described");
    assert!(
        stall.waiting_on.iter().any(|(slot, _)| *slot == SLOT_TWO),
        "…and name the peer it is waiting for: {stall}"
    );

    // And it resumes exactly when the missing input arrives, with no lost work.
    for _ in 0..(delay + 8) {
        step(&mut hosts);
    }
    assert!(
        hosts[0].tick() > stalled_tick,
        "the fleet must resume once the peer speaks again"
    );
    assert!(!hosts[0]
        .app
        .world()
        .resource::<MeshDiagnostics>()
        .is_stalled());
}

/// A fleet of one is not a fleet: nothing waits, nothing stalls, and the delay
/// is zero. The solo path must not acquire a barrier it can never clear.
#[test]
fn a_fleet_of_one_never_waits() {
    let args = args();
    let mut app = build_headless_app(&args).expect("app should build");
    join_fleet(app.world_mut(), FleetRoster::default(), 6);
    run(&mut app, 60);

    // 59 rather than 60: a headless app's first `update()` establishes the time
    // baseline and steps nothing. What is under test is that NOTHING was
    // withheld, so the claim is against the free-running clock rather than
    // against the loop count.
    assert_eq!(
        app.world().resource::<SimTick>().0,
        59,
        "a host with no peers must run every tick its clock offers it"
    );
    assert!(app.world().resource::<FleetLockstep>().is_alone());
    assert_eq!(
        app.world().resource::<MeshDiagnostics>().stalled_frames,
        0,
        "a host with no peers must never withhold a tick"
    );
    assert!(
        app.world()
            .resource::<MeshOutbox>()
            .pending_frames()
            .is_empty(),
        "…and must say nothing to a fleet that does not exist"
    );
}

// ── Receiver authority ─────────────────────────────────────────────────────────

/// **The ship-ownership check (issue #1116).** A host may drive only the ship
/// its own fleet slot flies. A peer frame ordered honestly under its own slot,
/// but naming ANOTHER slot's hull — or an NPC — in its `ShipKey`, is DROPPED,
/// not applied.
///
/// Without it a peer could inject authoritative commands onto another player's
/// ship or onto any NPC: `is_from` passes (the frame's `from` and its command's
/// order origin agree), and the `ShipKey` apply route would then deliver the
/// command to whatever hull that uuid names — identically, and wrongly, on every
/// host. The forged commands here are the only ones the fleet ever admits; no
/// crew presses a key, so an empty log is proof they were refused rather than
/// merely lost in the noise of legitimate traffic.
#[test]
fn a_peer_may_not_drive_a_ship_its_slot_does_not_fly() {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO)];
    // Spawn the fleet and let it settle. No crew order is ever issued.
    for _ in 0..30 {
        step(&mut hosts);
    }

    let fleet = hosts[0].fleet_ships();
    let slot_one_hull = fleet
        .get(&SLOT_ONE)
        .expect("slot 1's ship exists on host one")
        .clone();

    // Two forged slot-2 frames delivered to host ONE (which applies slot 2's
    // traffic): one names host one's OWN hull, one names a hull no slot flies.
    // Both are ordered honestly under slot 2, so only the ownership check can
    // stop them.
    let apply_tick = hosts[0].tick() + 20;
    let forge = |ship: &str, seq: u64| {
        MeshFrame::Tick(TickFrame {
            from: SLOT_TWO,
            tick: apply_tick,
            ready_through: apply_tick,
            commands: vec![MeshCommand {
                tick: apply_tick,
                order: CommandOrder::new(SLOT_TWO, seq),
                ship: ShipKey(ship.to_string()),
                target: SystemId("helm-steering".into()),
                payload: steer(0.5),
            }],
            start_grant: None,
        })
    };
    hosts[0].deliver(&[forge(&slot_one_hull, 0), forge("npc-no-slot-flies-this", 1)]);

    // Run well past the forged apply tick, so a command that WAS queued would
    // have drained into the log by now.
    for _ in 0..80 {
        step(&mut hosts);
    }

    assert!(
        hosts[0].log().entries().is_empty(),
        "a forged peer command for a ship slot 2 does not fly reached the log — \
         the receiver must drop a command whose ShipKey is not the sending \
         slot's own hull, or one host can drive another's ship or an NPC"
    );
}
