//! A live restore across every simulation peer (issue #1447, PRD #1420
//! stories 10–14 and 16).
//!
//! The crown proof for the multi-peer half of GM M5, and deliberately built the
//! way `tests/lockstep_recovery.rs` is built: three COMPLETE headless
//! simulations in one process with a synchronous mesh between them, a real
//! scenario, real saves in each peer's own storage, the real `snapshot::restore`
//! walk, the real digest fold and the real canonical journal. Nothing here is
//! mocked, because every claim this issue makes is a claim about what happens to
//! three real worlds at once.
//!
//! # Why faults are injected in the FERRY
//!
//! A peer that goes quiet is not a peer that reports an error - it is a peer
//! whose frames stop arriving. So the faults here are delivery faults: stop
//! ferrying to a host, stop ferrying from one, or cut the candidate transfer in
//! half. That is the shape a closed laptop lid, a dropped WebRTC leg and a
//! half-sent snapshot really have, and it is the only injection that can prove
//! the bounded windows do anything at all.
//!
//! # Why its own test binary
//!
//! `--deterministic` pins the scheduler with a one-thread `TaskPoolOptions`, and
//! Bevy's task pools are process-global, fixed by whichever app builds first, so
//! a bit-identical-fold claim in a shared binary is a claim about a race. Same
//! reason as `tests/lockstep_mesh.rs`, `tests/lockstep_snapshot_transfer.rs` and
//! `tests/lockstep_recovery.rs`.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use project_phoenix::command_admission::log::HostSlot;
use project_phoenix::core::messages::StationId;
use project_phoenix::gm_action::{
    sequence_owner_proposal, GmAction, GmActionFrame, GmActionGrant, GmActionId, GmActionJournal,
    GmActionProposal, GmActionRefusalReason, SimulationPaused,
};
use project_phoenix::gm_restore::{GmLiveRestore, GmRestoreFailure, GmRestorePhase};
use project_phoenix::headless::{build_headless_app, world_digest, HeadlessArgs};
use project_phoenix::lockstep::{
    join_fleet, FleetGm, FleetLockstep, FleetRoster, FleetShip, MeshFrame, MeshInbox, MeshOutbox,
};
use project_phoenix::save_slots_store::{install_local_save_store, request_named_manual_save};
use project_phoenix::sim_tick::SimTick;

/// A world with crewed hulls and a hostile, so the fold every peer compares
/// covers a real mission rather than an empty world.
const WORLD: &str = "assets/worlds/probe_fleet_trio.toml";
const SHIP: &str = "assets/entities/alliance_cruiser.toml";
const SEED: u64 = 1_447_2026;

/// Two crewed ship hosts and one stationless Game Master peer. The GM owns a
/// simulation like any other participant, so it holds, rewinds and agrees like
/// any other participant - and because the candidate is a key in ITS private
/// catalogue, the peer coordinating the rewind is deliberately NOT the
/// technical owner that sequences the request.
const SLOT_ONE: HostSlot = HostSlot(1);
const SLOT_TWO: HostSlot = HostSlot(2);
const SLOT_GM: HostSlot = HostSlot(3);
/// Indices into the host vector: the technical owner, the second ship host,
/// and the Game Master's own peer.
const OWNER: usize = 0;
const SHIP_TWO: usize = 1;
const GM_HOST: usize = 2;

const HELM: &str = "helm";
const RATING: &str = "Std";
const GM: &str = "gm-1";

/// One fixed step per frame while the mission runs, so a frame and a tick are
/// the same thing and the mesh can be ferried between them.
const MISSION_FRAME: Duration = Duration::from_nanos(16_666_667);

/// Half a real second per frame once a world is HELD, so the ten-second
/// readiness window of issue #1447 is reached in twenty frames instead of six
/// hundred. The clock really advances: `Time<Real>` is what the window is
/// measured on, and `TimeUpdateStrategy` is the supported way to drive it. A
/// held world spends no fixed ticks, so the larger step changes nothing but
/// how long the test takes.
const HELD_FRAME: Duration = Duration::from_millis(500);

/// One peer's private storage.
#[derive(Clone, Default)]
struct PeerStore {
    slots: Arc<Mutex<BTreeMap<String, String>>>,
}

impl PeerStore {
    fn keys(&self) -> Vec<String> {
        self.slots.lock().unwrap().keys().cloned().collect()
    }
}

impl vellum_save::Store for PeerStore {
    type Error = String;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok(self.slots.lock().unwrap().get(slot).cloned())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        self.slots
            .lock()
            .unwrap()
            .insert(slot.to_string(), contents.to_string());
        Ok(())
    }

    fn remove(&self, slot: &str) -> Result<(), Self::Error> {
        self.slots.lock().unwrap().remove(slot);
        Ok(())
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        Ok(self.slots.lock().unwrap().keys().cloned().collect())
    }
}

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

/// The frozen fleet from `local`'s point of view: two crewed hulls, slot one
/// the technical owner, and the GM operator bound to its own stationless slot.
fn roster(local: HostSlot) -> FleetRoster {
    let ships: Vec<FleetShip> = [SLOT_ONE, SLOT_TWO]
        .into_iter()
        .map(|host| FleetShip {
            host,
            ship_path: Some(SHIP.into()),
            crew: vec![(StationId(HELM.into()), RATING.to_string())],
        })
        .collect();
    FleetRoster::with_participants_and_gms(
        ships,
        vec![SLOT_ONE, SLOT_TWO, SLOT_GM],
        vec![FleetGm {
            host: SLOT_GM,
            operator_id: GM.to_string(),
        }],
        local,
        SLOT_ONE,
    )
    .expect("a three-peer roster with one GM binding is representable")
}

/// Which family of mesh frames an inbound leg is slow for.
///
/// A LEG fault, like every other fault in this file: the peer says exactly what
/// it would have said, and the wire takes longer to carry it. Peers apply the
/// same canonical grant at the same TICK but not in the same FRAME - they may
/// legitimately sit a whole lockstep delay apart - and the coordinator's own
/// bounded waits are measured in real seconds, so both halves of the protocol
/// have to survive a leg that is behind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SlowLeg {
    /// The canonical GM grant itself reaches this host late.
    GmAction,
    /// Readiness, load reports and the room's decision reach this host late.
    GmRestore,
}

/// One authoritative peer, with its own private save storage.
struct Host {
    app: App,
    slot: HostSlot,
    store: PeerStore,
    /// Whether this host's traffic reaches the others this round.
    speaking: bool,
    /// Whether the others' traffic reaches this host this round.
    listening: bool,
    /// This host's inbound leg latency: frames of `slow_leg` spend
    /// `slow_rounds` extra ferry rounds in the air before they arrive.
    slow_leg: Option<SlowLeg>,
    slow_rounds: usize,
    /// Frames still in the air on that leg, with the rounds each has left.
    in_the_air: Vec<(usize, MeshFrame)>,
}

impl Host {
    fn new(slot: HostSlot) -> Self {
        let mut app = build_headless_app(&args()).expect("app should build");
        let store = PeerStore::default();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(MISSION_FRAME));
        install_local_save_store(&mut app, store.clone());
        let delay = project_phoenix::lockstep::authored_delay(app.world());
        assert!(delay > 0, "the probe world must author a non-zero delay");
        join_fleet(app.world_mut(), roster(slot), delay);
        app.finish();
        app.cleanup();
        Self {
            app,
            slot,
            store,
            speaking: true,
            listening: true,
            slow_leg: None,
            slow_rounds: 0,
            in_the_air: Vec::new(),
        }
    }

    /// Put this host behind on one family of frames, for the whole test.
    fn slow_leg(&mut self, leg: SlowLeg, rounds: usize) {
        self.slow_leg = Some(leg);
        self.slow_rounds = rounds;
    }

    fn tick(&self) -> u64 {
        self.app.world().resource::<SimTick>().0
    }

    fn digest(&self) -> u64 {
        world_digest(self.app.world())
    }

    fn paused(&self) -> bool {
        self.app.world().resource::<SimulationPaused>().0
    }

    fn phase(&self) -> GmRestorePhase {
        self.app.world().resource::<GmLiveRestore>().phase()
    }

    fn failure(&self) -> Option<GmRestoreFailure> {
        self.app
            .world()
            .resource::<GmLiveRestore>()
            .failure()
            .cloned()
    }

    fn restore(&self) -> &GmLiveRestore {
        self.app.world().resource::<GmLiveRestore>()
    }

    fn drain_outbox(&mut self) -> Vec<MeshFrame> {
        self.app.world_mut().resource_mut::<MeshOutbox>().drain()
    }

    fn deliver(&mut self, frames: &[MeshFrame]) {
        let rounds = self.slow_rounds;
        let (slow, prompt): (Vec<MeshFrame>, Vec<MeshFrame>) = frames
            .iter()
            .cloned()
            .partition(|frame| rounds > 0 && self.leg_is_slow_for(frame));
        self.in_the_air
            .extend(slow.into_iter().map(|frame| (rounds, frame)));
        let mut inbox = self.app.world_mut().resource_mut::<MeshInbox>();
        for frame in prompt {
            inbox.push(frame);
        }
    }

    fn leg_is_slow_for(&self, frame: &MeshFrame) -> bool {
        matches!(
            (self.slow_leg, frame),
            (Some(SlowLeg::GmAction), MeshFrame::GmAction(_))
                | (Some(SlowLeg::GmRestore), MeshFrame::GmRestore(_))
        )
    }

    /// Land every frame whose flight time has run out, in the order it was sent.
    fn land_arrivals(&mut self) {
        if self.in_the_air.is_empty() {
            return;
        }
        let mut landing = Vec::new();
        let mut still_flying = Vec::new();
        for (rounds, frame) in std::mem::take(&mut self.in_the_air) {
            if rounds <= 1 {
                landing.push(frame);
            } else {
                still_flying.push((rounds - 1, frame));
            }
        }
        self.in_the_air = still_flying;
        let mut inbox = self.app.world_mut().resource_mut::<MeshInbox>();
        for frame in landing {
            inbox.push(frame);
        }
    }

    /// Let this host's clock run at [`HELD_FRAME`], so a real-seconds window can
    /// actually expire inside a test.
    fn hurry_the_clock(&mut self) {
        self.app
            .insert_resource(TimeUpdateStrategy::ManualDuration(HELD_FRAME));
    }

    fn departed(&self, slot: HostSlot) -> bool {
        self.app
            .world()
            .resource::<FleetLockstep>()
            .0
            .has_departed(slot)
    }

    /// The live protocol generation this peer's journal holds for `slot`.
    fn fence_generation(&self, slot: HostSlot) -> u64 {
        self.app
            .world()
            .resource::<GmActionJournal>()
            .current_recovery_generation(slot)
    }

    /// Every Station this peer believes is crewed on its own hull, so "the room
    /// kept its seats" can be asserted against real seating rather than a flag.
    fn seating(&mut self) -> BTreeMap<String, String> {
        let mut query = self.app.world_mut().query::<(
            &project_phoenix::ship::components::ActiveStationRatings,
            &project_phoenix::server_app::LocalShip,
        )>();
        let (ratings, _) = query
            .iter(self.app.world())
            .next()
            .expect("this peer projects exactly one local hull");
        ratings
            .0
            .iter()
            .map(|(station, rating)| (station.0.clone(), rating.clone()))
            .collect()
    }
}

/// One round of "everything each host said reaches every other host".
fn ferry(hosts: &mut [Host]) {
    let outgoing: Vec<(bool, Vec<MeshFrame>)> = hosts
        .iter_mut()
        .map(|host| (host.speaking, host.drain_outbox()))
        .collect();
    for (i, host) in hosts.iter_mut().enumerate() {
        // A frame already in the air lands whether or not anything new is said
        // this round, and before it, so one leg's order is preserved.
        host.land_arrivals();
        if !host.listening {
            continue;
        }
        for (j, (speaking, frames)) in outgoing.iter().enumerate() {
            if i != j && *speaking {
                host.deliver(frames);
            }
        }
    }
}

/// Step every host one FRAME, delivering the previous round's traffic.
///
/// One frame is one fixed step while the mission is running and none at all
/// while it is held, which is why everything here counts frames: a held world
/// spends no ticks, so a tick-counting loop would never return during a
/// restore.
fn frame(hosts: &mut [Host]) {
    ferry(hosts);
    for host in hosts.iter_mut() {
        host.app.update();
    }
}

/// Run the mission forward.
fn step(hosts: &mut [Host]) {
    frame(hosts);
}

/// Run frames until every host has REPORTED an outcome, or `frames` go by.
///
/// "Reported" and not merely "not working": a restore that has not started yet
/// is idle too, and a loop that took idleness for a settled answer would pass
/// every test in this file without a restore ever happening.
fn settle(hosts: &mut [Host], frames: usize) {
    for _ in 0..frames {
        if hosts
            .iter()
            .all(|host| !host.phase().in_flight() && host.phase() != GmRestorePhase::Idle)
        {
            return;
        }
        frame(hosts);
    }
}

fn fleet() -> Vec<Host> {
    let mut hosts = vec![Host::new(SLOT_ONE), Host::new(SLOT_TWO), Host::new(SLOT_GM)];
    // A real mission first, so the rewind below has somewhere to rewind FROM.
    for _ in 0..90 {
        step(&mut hosts);
    }
    assert!(
        hosts.iter().all(|host| host.tick() > 10),
        "the fleet never got a mission underway: {:?}",
        hosts.iter().map(Host::tick).collect::<Vec<_>>(),
    );
    let digests: Vec<u64> = hosts.iter().map(Host::digest).collect();
    assert!(
        digests[0] == digests[1] && digests[1] == digests[2],
        "the fleet disagreed BEFORE any restore was asked for: {digests:?}",
    );
    hosts
}

/// Bookmark a checkpoint in the OWNER's own catalogue - the only catalogue a
/// candidate can come from, because the candidate is one GM's private file.
fn bookmark(hosts: &mut [Host], name: &str) -> String {
    let slot_id = request_named_manual_save(hosts[GM_HOST].app.world_mut(), name)
        .expect("the GM peer has a Store installed");
    for _ in 0..30 {
        step(hosts);
    }
    slot_id
}

/// Sequence one GM decision through the REAL owner path and put it on the wire.
///
/// Deliberately `sequence_owner_proposal` with the arguments `submit_local`
/// computes for this topology, rather than a hand-built grant: WHERE a decision
/// lands, and whether it is allowed to land at all while a restore is running,
/// are decided there and nowhere else (issue #1447).
fn gm_decision(
    hosts: &mut [Host],
    correlation: &str,
    action: GmAction,
) -> Result<GmActionGrant, GmActionRefusalReason> {
    let owner = &mut hosts[0];
    let now = owner.tick();
    let paused = owner.paused();
    let phase = owner.phase();
    let ready_through = owner
        .app
        .world()
        .resource::<FleetLockstep>()
        .0
        .ready_through(now);
    let proposal = GmActionProposal {
        // The GM's own peer proposes; the technical OWNER sequences. They are
        // deliberately different machines here, because that is the topology a
        // restore has to work in: the peer that holds the candidate is the peer
        // whose desk asked, not the peer that orders the fleet's decisions.
        from: SLOT_GM,
        operator_id: GM.to_string(),
        correlation: GmActionId::new(correlation.to_string()).unwrap(),
        action,
    };
    let mut journal = owner.app.world_mut().resource_mut::<GmActionJournal>();
    let grant = sequence_owner_proposal(
        &mut journal,
        &proposal,
        SLOT_ONE,
        now,
        ready_through,
        paused,
        phase.holds_world(),
        phase.in_flight(),
    )?;
    owner
        .app
        .world_mut()
        .resource_mut::<MeshOutbox>()
        .push(MeshFrame::GmAction(GmActionFrame::Granted(grant.clone())));
    Ok(grant)
}

fn request_restore(hosts: &mut [Host], correlation: &str, candidate: &str) {
    gm_decision(
        hosts,
        correlation,
        GmAction::RequestLiveRestore {
            candidate: candidate.to_string(),
        },
    )
    .expect("the owner sequences its own restore request");
}

/// Run frames until the request has been applied everywhere and the world is
/// held, then let the clock run in half-second steps so a ten-second window can
/// close inside a test.
///
/// Deliberately after the hold and never before it: at half a second a frame a
/// RUNNING world would take thirty fixed steps between ferries, and the fleet
/// would spend the test waiting on the barrier instead of on the restore.
fn hold_and_hurry(hosts: &mut [Host]) {
    hold_and_hurry_within(hosts, 12);
}

/// The same, for a topology where one peer's copy of the grant is deliberately
/// behind and needs longer to arrive.
fn hold_and_hurry_within(hosts: &mut [Host], frames: usize) {
    for _ in 0..frames {
        frame(hosts);
        if hosts.iter().all(|host| host.phase().in_flight()) {
            break;
        }
    }
    assert!(
        hosts.iter().all(|host| host.phase().in_flight()),
        "the canonical request never reached every peer: {:?}",
        hosts.iter().map(Host::phase).collect::<Vec<_>>(),
    );
    for host in hosts.iter_mut() {
        host.hurry_the_clock();
    }
}

// ── The headline: one rewind, three worlds, one fold ─────────────────────────

/// **AC1, AC3, AC4, AC7.** A GM on one peer rewinds a three-peer session onto a
/// checkpoint that lives only in that peer's catalogue: every peer holds, takes
/// its own recovery checkpoint, loads the transferred candidate, reports the
/// same fold, and is fenced and held for one explicit resume.
#[test]
fn a_live_rewind_lands_on_every_peer_and_the_fleet_folds_identically() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Before the pass");
    // Run on, so a successful rewind is visibly a rewind rather than a no-op.
    for _ in 0..20 {
        step(&mut hosts);
    }
    let live_tick = hosts[0].tick();
    let seating_before: Vec<BTreeMap<String, String>> =
        hosts[..GM_HOST].iter_mut().map(Host::seating).collect();
    let fences_before: Vec<u64> = hosts
        .iter()
        .map(|host| host.fence_generation(SLOT_TWO))
        .collect();

    request_restore(&mut hosts, "restore-1", &candidate);
    settle(&mut hosts, 80);

    for host in &hosts {
        assert_eq!(
            host.phase(),
            GmRestorePhase::Restored,
            "peer {} did not land the rewind: {:?}",
            host.slot.slot_id(),
            host.failure(),
        );
        assert!(
            host.paused(),
            "a restore is never a resume (PRD #1420 story 13)",
        );
        assert!(
            host.tick() < live_tick,
            "peer {} is still standing where the session was, not where the \
             checkpoint is",
            host.slot.slot_id(),
        );
    }

    // One world on three machines. This is the claim the whole operation exists
    // to make, and the one a peer-by-peer restore could quietly break.
    let digests: Vec<u64> = hosts.iter().map(Host::digest).collect();
    assert_eq!(
        digests[0], digests[1],
        "peers 1 and 2 restored different worlds: {digests:?}",
    );
    assert_eq!(
        digests[1], digests[2],
        "peers 2 and 3 restored different worlds: {digests:?}",
    );
    let ticks: Vec<u64> = hosts.iter().map(Host::tick).collect();
    assert!(
        ticks[0] == ticks[1] && ticks[1] == ticks[2],
        "the fleet is held at three different ticks: {ticks:?}",
    );

    // The world rewound; the ROOM did not. Every peer keeps the seating it had.
    let seating_after: Vec<BTreeMap<String, String>> =
        hosts[..GM_HOST].iter_mut().map(Host::seating).collect();
    assert_eq!(
        seating_after, seating_before,
        "a rewind of the world moved somebody out of their seat",
    );

    // AC4: the fence is stamped for EVERY participant, identically everywhere -
    // a peer that fenced only itself would hold a different journal and split
    // the fold on the very next sample.
    for slot in [SLOT_ONE, SLOT_TWO, SLOT_GM] {
        let generations: Vec<u64> = hosts
            .iter()
            .map(|host| host.fence_generation(slot))
            .collect();
        assert!(
            generations[0] == generations[1] && generations[1] == generations[2],
            "the fence for {} differs between peers: {generations:?}",
            slot.slot_id(),
        );
        assert!(generations[0] > 0, "{} was never fenced", slot.slot_id());
    }
    assert!(
        hosts[0].fence_generation(SLOT_TWO) > fences_before[0],
        "the fence did not advance, so pre-restore work would still be admitted",
    );
}

/// **AC6, concurrency.** A second GM pressing while a restore is running is
/// answered by the owner, by name, before a grant exists - never folded out of
/// peer-local progress that legitimately differs between peers.
#[test]
fn a_second_request_under_a_working_restore_is_refused_by_the_owner() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "First");
    request_restore(&mut hosts, "restore-1", &candidate);
    // One frame is enough: the request is applied on the owner and the world is
    // held from that moment.
    for _ in 0..4 {
        frame(&mut hosts);
        if hosts[OWNER].phase().in_flight() {
            break;
        }
    }
    assert!(hosts[OWNER].phase().in_flight());

    let refused = gm_decision(
        &mut hosts,
        "restore-2",
        GmAction::RequestLiveRestore {
            candidate: candidate.clone(),
        },
    );
    assert_eq!(
        refused.err(),
        Some(GmActionRefusalReason::LiveRestoreInProgress),
    );
    // And so is anything else: a grant minted during the flight would land on
    // the held boundary and be applied before the rewind on one peer and after
    // it on another. Refusing the whole window is the only answer that is the
    // same everywhere (AC4).
    let unrelated = gm_decision(
        &mut hosts,
        "pause-1",
        GmAction::SetSessionPaused { active: false },
    );
    assert_eq!(
        unrelated.err(),
        Some(GmActionRefusalReason::LiveRestoreInProgress),
    );

    settle(&mut hosts, 80);
    for host in &hosts {
        assert_eq!(
            host.phase(),
            GmRestorePhase::Restored,
            "{:?}",
            host.failure()
        );
    }
}

// ── Faults ───────────────────────────────────────────────────────────────────

/// **AC2, AC5.** A peer that never answers the readiness ask is disconnected
/// through canonical membership after ten real seconds, and the remaining peers
/// rewind without it. It is never resumed, and it is never assumed willing.
#[test]
fn a_peer_that_never_answers_is_disconnected_and_the_rest_restore() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Without the second ship");

    request_restore(&mut hosts, "restore-1", &candidate);
    hold_and_hurry(&mut hosts);
    // Slot two's lid closes: nothing it says arrives, and nothing reaches it.
    // It keeps running its own frames, which is exactly what a machine that has
    // not noticed it is alone does.
    hosts[SHIP_TWO].speaking = false;
    hosts[SHIP_TWO].listening = false;

    // A few frames in, the room is visibly waiting for it - with a countdown.
    for _ in 0..8 {
        frame(&mut hosts);
        if hosts[GM_HOST].phase() == GmRestorePhase::AwaitingReadiness {
            break;
        }
    }
    assert_eq!(hosts[GM_HOST].phase(), GmRestorePhase::AwaitingReadiness);
    assert!(
        hosts[GM_HOST].restore().waiting_on(SLOT_TWO),
        "the desk must be able to name the peer the room is waiting for",
    );
    assert!(
        hosts[GM_HOST]
            .restore()
            .remaining_seconds()
            .is_some_and(|left| left <= 10),
        "a wait with no countdown is a wait nobody can plan around",
    );

    settle(&mut hosts, 160);

    assert!(
        hosts[GM_HOST].restore().excluded(SLOT_TWO),
        "the nonresponder was waited on forever instead of being excluded",
    );
    for index in [OWNER, GM_HOST] {
        let host = &hosts[index];
        assert_eq!(
            host.phase(),
            GmRestorePhase::Restored,
            "the remaining peers must continue: {:?}",
            host.failure(),
        );
        assert!(host.paused());
        assert!(
            host.departed(SLOT_TWO),
            "the exclusion is canonical membership, not a restore-private idea \
             of who is in the room",
        );
    }
    assert_eq!(
        hosts[OWNER].digest(),
        hosts[GM_HOST].digest(),
        "the two peers that stayed did not end up in the same world",
    );
    // AC5/AC6: the excluded peer is NOT resumed onto the restored timeline, and
    // nothing brought it back automatically.
    assert_ne!(
        hosts[SHIP_TWO].phase(),
        GmRestorePhase::Restored,
        "an excluded peer must never be resumed on a world it never loaded",
    );
    assert!(
        hosts[SHIP_TWO].paused(),
        "and it is left stopped, not running on",
    );
}

/// **AC3, AC6.** A remaining peer that reports it cannot take part rolls the
/// WHOLE fleet back to the recovery checkpoints it took on the way in, held,
/// with the reason named. Nobody is left running the candidate.
#[test]
fn a_peer_that_cannot_take_part_rolls_the_whole_fleet_back() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Rolled back");
    let ticks_before: Vec<u64> = hosts.iter().map(Host::tick).collect();

    // The second ship host loses its storage between the request and its own
    // recovery capture: no Store, no recovery checkpoint, and #1446's unchanged
    // rule is that no recovery checkpoint means no restore.
    hosts[SHIP_TWO]
        .app
        .world_mut()
        .remove_resource::<project_phoenix::save_slots_store::SaveSlotService>();

    request_restore(&mut hosts, "restore-1", &candidate);
    hold_and_hurry(&mut hosts);
    settle(&mut hosts, 200);

    for host in &hosts {
        assert!(
            matches!(
                host.phase(),
                GmRestorePhase::RolledBack | GmRestorePhase::Failed
            ),
            "peer {} settled on a rewind the room did not agree: {:?}",
            host.slot.slot_id(),
            host.phase(),
        );
        assert!(host.paused(), "a failed restore never resumes");
    }
    assert!(
        matches!(
            hosts[GM_HOST].failure(),
            Some(GmRestoreFailure::PeerLoadFailed { .. })
        ),
        "the coordinating peer must say WHY it abandoned: {:?}",
        hosts[GM_HOST].failure(),
    );
    // Nothing rewound: every peer is at least where the accepted request held
    // it, never back at the candidate's tick.
    for (host, before) in hosts.iter().zip(ticks_before.iter()) {
        assert!(
            host.tick() >= *before,
            "peer {} rewound anyway after the room refused",
            host.slot.slot_id(),
        );
    }
    assert_eq!(
        hosts[OWNER].digest(),
        hosts[GM_HOST].digest(),
        "the peers that could have loaded did not agree after the rollback",
    );
}

/// **AC3.** A peer that goes quiet DURING the transfer is disconnected through
/// the same canonical membership rather than waited on forever - and the
/// restore is not reported as a success on its behalf either.
#[test]
fn a_peer_silent_during_the_transfer_is_disconnected_rather_than_waited_on() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Silent mid-transfer");

    request_restore(&mut hosts, "restore-1", &candidate);
    hold_and_hurry(&mut hosts);
    // Let every peer answer the readiness ask, so the room really is agreed
    // before the fault; then cut the second ship off mid-transfer.
    for _ in 0..10 {
        frame(&mut hosts);
        if matches!(
            hosts[GM_HOST].phase(),
            GmRestorePhase::Loading | GmRestorePhase::AwaitingAgreement
        ) {
            break;
        }
    }
    assert!(
        !hosts[GM_HOST].restore().excluded(SLOT_TWO),
        "the fault must land AFTER the peer answered, or this proves nothing \
         the readiness case does not already prove",
    );
    hosts[SHIP_TWO].speaking = false;
    hosts[SHIP_TWO].listening = false;

    settle(&mut hosts, 200);

    for index in [OWNER, GM_HOST] {
        assert!(
            !hosts[index].phase().in_flight(),
            "peer {} is still waiting on a machine that stopped speaking",
            hosts[index].slot.slot_id(),
        );
        assert!(hosts[index].paused());
    }
    assert!(
        hosts[GM_HOST].restore().excluded(SLOT_TWO)
            || hosts[GM_HOST].phase() == GmRestorePhase::RolledBack,
        "a peer that went quiet mid-transfer was neither excluded nor reported: \
         {:?}",
        hosts[GM_HOST].phase(),
    );
    assert_eq!(
        hosts[OWNER].digest(),
        hosts[GM_HOST].digest(),
        "the peers that stayed did not agree",
    );
    assert!(
        hosts[SHIP_TWO].paused(),
        "a cut-off peer must never be left running a world the room did not \
         agree",
    );
}

/// **AC3, AC6.** The initiating peer vanishing is not a reason to wait forever
/// or to commit on a guess: every other peer returns to its own recovery
/// checkpoint and says so.
#[test]
fn the_initiating_peer_vanishing_returns_every_other_peer_to_its_checkpoint() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Coordinator lost");

    request_restore(&mut hosts, "restore-1", &candidate);
    // Let the request reach every peer, then take the coordinating peer away
    // before it can decide anything.
    hold_and_hurry(&mut hosts);
    hosts[GM_HOST].speaking = false;
    hosts[GM_HOST].listening = false;

    settle(&mut hosts, 200);

    for index in [OWNER, SHIP_TWO] {
        let host = &hosts[index];
        assert!(
            !host.phase().in_flight(),
            "peer {} is still waiting on a decision that cannot arrive",
            host.slot.slot_id(),
        );
        assert!(
            host.paused(),
            "a peer with no coordinator must not resume itself",
        );
        assert_ne!(
            host.phase(),
            GmRestorePhase::Restored,
            "peer {} committed a rewind nobody agreed",
            host.slot.slot_id(),
        );
    }
}

/// **AC5.** A peer excluded for nonresponse comes back through the EXISTING
/// snapshot recovery, onto the state the room actually ended up in - not
/// through a hidden automatic resume of the timeline it was cut off from.
#[test]
fn an_excluded_peer_rejoins_the_restored_state_through_snapshot_recovery() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Rejoin after exclusion");

    request_restore(&mut hosts, "restore-1", &candidate);
    hold_and_hurry(&mut hosts);
    hosts[SHIP_TWO].speaking = false;
    hosts[SHIP_TWO].listening = false;
    settle(&mut hosts, 200);
    assert_eq!(
        hosts[GM_HOST].phase(),
        GmRestorePhase::Restored,
        "{:?}",
        hosts[GM_HOST].failure(),
    );
    assert!(hosts[GM_HOST].restore().excluded(SLOT_TWO));

    let restored_digest = hosts[OWNER].digest();
    assert_ne!(
        hosts[SHIP_TWO].digest(),
        restored_digest,
        "the excluded peer is on the abandoned timeline, which is the whole \
         reason it has to recover",
    );

    // The ordinary #1117/#1118 transfer, unchanged: a surviving peer captures
    // its canonical record, the excluded one arms for it and restores. Nothing
    // about a live restore is special here - which is the point of AC5.
    let record = project_phoenix::lockstep::capture_run(hosts[OWNER].app.world(), WORLD);
    let text =
        project_phoenix::snapshot::export_artifact(&record).expect("a canonical record exports");
    hosts[SHIP_TWO]
        .app
        .world_mut()
        .resource_mut::<project_phoenix::lockstep::MeshRestoreArm>()
        .arm(SLOT_ONE);
    let outcome = project_phoenix::lockstep::snapshot_relay::gate_and_restore_rebuilding(
        hosts[SHIP_TWO].app.world_mut(),
        &text,
    );
    assert!(
        matches!(
            outcome,
            project_phoenix::lockstep::MeshRestoreOutcome::Committed { .. }
        ),
        "the excluded peer could not recover the restored state: {outcome:?}",
    );
    assert_eq!(
        hosts[SHIP_TWO].digest(),
        restored_digest,
        "recovery put the excluded peer somewhere other than where the room is",
    );
    // And it is still held: rejoining a rewound session is not a resume.
    assert!(hosts[SHIP_TWO].paused());
}

/// A peer's own storage is private, and a restore does not reach into it: every
/// peer's recovery checkpoint is written to its OWN catalogue, and the
/// candidate's slot id is a key in one peer's storage that no other peer has.
#[test]
fn every_peer_takes_its_own_recovery_checkpoint_in_its_own_storage() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Own checkpoints");
    assert!(hosts[GM_HOST].store.keys().contains(&candidate));
    assert!(
        !hosts[OWNER].store.keys().contains(&candidate),
        "the candidate is one GM's private file; no other peer holds it",
    );
    let before: Vec<usize> = hosts.iter().map(|host| host.store.keys().len()).collect();

    request_restore(&mut hosts, "restore-1", &candidate);
    settle(&mut hosts, 120);

    for (index, host) in hosts.iter().enumerate() {
        assert_eq!(
            host.phase(),
            GmRestorePhase::Restored,
            "{:?}",
            host.failure(),
        );
        assert!(
            host.store.keys().len() > before[index],
            "peer {} wrote nothing to its own storage, so it cannot have taken \
             a recovery checkpoint",
            host.slot.slot_id(),
        );
        let recovery = host
            .restore()
            .recovery_slot()
            .unwrap_or_else(|| panic!("peer {} has no way back", host.slot.slot_id()))
            .to_string();
        assert!(
            host.store.keys().contains(&recovery),
            "peer {}'s recovery checkpoint is not in its OWN catalogue",
            host.slot.slot_id(),
        );
    }
}

/// **AC3, AC6.** A remaining peer that ends up with a DIFFERENT world is the
/// case agreement exists for: the fold it reports is compared against the one
/// the initiating peer got, and a mismatch rolls the whole room back rather
/// than resuming two peers running two worlds.
///
/// Injected the way it could really happen: a stale transfer from an abandoned
/// attempt reaches one peer ahead of this one's. The receiver arm cannot tell
/// them apart - both carry the initiating peer's slot and both are
/// self-consistent records that gate and fold correctly - which is precisely
/// why the fold has to be compared ACROSS peers and not only against the record
/// each peer loaded.
#[test]
fn a_peer_that_ends_up_with_a_different_world_rolls_the_whole_fleet_back() {
    let mut hosts = fleet();
    let wanted = bookmark(&mut hosts, "The one that was asked for");
    for _ in 0..20 {
        step(&mut hosts);
    }
    let stale = bookmark(&mut hosts, "A different moment entirely");
    let other = hosts[GM_HOST]
        .app
        .world()
        .resource::<project_phoenix::save_slots_store::SaveSlotService>()
        .export(&stale)
        .expect("the other bookmark reads back");

    request_restore(&mut hosts, "restore-1", &wanted);
    hold_and_hurry(&mut hosts);
    // Let the room answer, so the coordinator is past readiness and really is
    // comparing folds rather than refusing an unready peer.
    for _ in 0..10 {
        frame(&mut hosts);
        if matches!(
            hosts[GM_HOST].phase(),
            GmRestorePhase::Loading | GmRestorePhase::AwaitingAgreement
        ) {
            break;
        }
    }
    // The stale record wins the race to slot two, and the real one never
    // arrives there.
    hosts[SHIP_TWO].listening = false;
    let chunks = project_phoenix::lockstep::transfer::chunk(&other, SLOT_GM, 0x1447, 0);
    for chunk in chunks {
        hosts[SHIP_TWO]
            .app
            .world_mut()
            .resource_mut::<MeshInbox>()
            .push(MeshFrame::Snapshot(chunk));
    }
    hosts[SHIP_TWO].app.update();
    // Slot two must speak, or this would be the silent-peer case over again.
    hosts[SHIP_TWO].listening = true;

    settle(&mut hosts, 200);

    assert!(
        matches!(
            hosts[GM_HOST].failure(),
            Some(GmRestoreFailure::PeerDigestMismatch { .. })
        ),
        "a peer running a different world must be named as exactly that: {:?}",
        hosts[GM_HOST].failure(),
    );
    for index in [OWNER, GM_HOST] {
        let host = &hosts[index];
        assert_ne!(
            host.phase(),
            GmRestorePhase::Restored,
            "peer {} resumed a rewind the room did not agree on",
            host.slot.slot_id(),
        );
        assert!(host.paused(), "a disagreed rewind never resumes");
    }
    assert!(
        hosts[SHIP_TWO].paused(),
        "and the peer that diverged is stopped rather than running on",
    );
}

/// **AC2, AC3.** An answer that arrived before this peer's own copy of the
/// grant is still an answer.
///
/// Every peer applies the same canonical request at the same TICK, but a tick
/// is not a frame: two peers may legitimately sit a whole lockstep delay apart
/// in wall time, and a follower says `Ready` exactly once, a couple of frames
/// after IT applies. Here the coordinating peer's copy of the grant is the last
/// to land, so both answers reach it before it has a request to match them
/// against. Nobody may be excluded for having spoken too early.
#[test]
fn an_answer_that_arrives_before_the_grant_still_counts() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Early answers");
    let live_tick = hosts[0].tick();

    // The GM peer - the one that coordinates - is behind on canonical grants.
    // Nothing else about it is slow: it hears every answer on time.
    hosts[GM_HOST].slow_leg(SlowLeg::GmAction, 8);

    request_restore(&mut hosts, "restore-1", &candidate);
    hold_and_hurry_within(&mut hosts, 40);
    settle(&mut hosts, 200);

    for host in &hosts {
        assert_eq!(
            host.phase(),
            GmRestorePhase::Restored,
            "peer {} did not land the rewind: {:?}",
            host.slot.slot_id(),
            host.failure(),
        );
        assert!(host.paused(), "a restore is never a resume");
        assert!(
            !hosts[GM_HOST].restore().excluded(host.slot),
            "peer {} answered immediately and was disconnected anyway",
            host.slot.slot_id(),
        );
        assert!(
            host.tick() < live_tick,
            "peer {} never rewound",
            host.slot.slot_id(),
        );
    }
    assert_eq!(
        hosts[GM_HOST].restore().excluded_peers(),
        0,
        "a peer was excluded for being quicker than the coordinator",
    );
    let digests: Vec<u64> = hosts.iter().map(Host::digest).collect();
    assert!(
        digests[0] == digests[1] && digests[1] == digests[2],
        "the fleet did not fold identically after the rewind: {digests:?}",
    );
}

/// **AC3, AC6.** A peer that loaded quickly must still be there when the room's
/// decision arrives, however long the coordinator's own bounded wait takes.
///
/// A third peer goes silent between "ready" and "loaded", so the coordinator
/// spends its whole window before it can exclude it, decide and announce - and
/// the leg carrying that decision is itself behind. A follower that timed its
/// own wait as if the coordinator owed it an answer inside one window would
/// give up first and roll ITSELF back to its recovery checkpoint while the room
/// committed: a divergent peer that is still a fleet member and that nobody
/// excluded, which is exactly the state AC3 and AC6 say must never be
/// reachable.
#[test]
fn a_peer_that_loaded_early_waits_out_a_silent_peers_whole_window() {
    let mut hosts = fleet();
    let candidate = bookmark(&mut hosts, "Slow agreement");

    // The owner's leg is behind on the restore lane, so the room's decision
    // takes real seconds to reach it after the coordinator finally makes one.
    hosts[OWNER].slow_leg(SlowLeg::GmRestore, 8);

    request_restore(&mut hosts, "restore-1", &candidate);
    hold_and_hurry(&mut hosts);
    // Let every peer answer the readiness ask, so the fault lands AFTER the
    // room was agreed; then the second ship goes quiet before it can report
    // what it loaded.
    for _ in 0..10 {
        frame(&mut hosts);
        if matches!(
            hosts[GM_HOST].phase(),
            GmRestorePhase::Loading | GmRestorePhase::AwaitingAgreement
        ) {
            break;
        }
    }
    assert!(
        !hosts[GM_HOST].restore().excluded(SLOT_TWO),
        "the fault must land AFTER the peer answered readiness",
    );
    hosts[SHIP_TWO].speaking = false;
    hosts[SHIP_TWO].listening = false;

    settle(&mut hosts, 260);

    for index in [OWNER, GM_HOST] {
        let host = &hosts[index];
        assert_eq!(
            host.phase(),
            GmRestorePhase::Restored,
            "peer {} did not end on the world the room agreed: {:?}",
            host.slot.slot_id(),
            host.failure(),
        );
        assert!(host.paused(), "a restore is never a resume");
    }
    assert_eq!(
        hosts[OWNER].digest(),
        hosts[GM_HOST].digest(),
        "a peer gave up on the coordinator and diverged from the room",
    );
    assert!(
        hosts[GM_HOST].restore().excluded(SLOT_TWO),
        "the silent peer was never excluded",
    );
    assert!(
        hosts[OWNER].departed(SLOT_TWO),
        "the exclusion is canonical membership, not a restore-private idea of \
         who is in the room",
    );
}
