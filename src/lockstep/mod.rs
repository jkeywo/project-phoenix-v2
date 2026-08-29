//! Two authoritative hosts, one mission, one agreed order (issue #1116).
//!
//! This module is the Bevy half of the host mesh: it owns the frozen fleet as
//! the *simulation* sees it, the in/out mesh queues a transport fills and
//! drains, the barrier that withholds a tick this host is not entitled to run
//! yet, and the periodic digest exchange that says whether the fleet still
//! agrees.
//!
//! The decisions live next door and are Bevy-free:
//!
//! * [`frame`] — what a host says to another host while a mission is running.
//! * [`session`] — who is in the fleet, and whether tick *T* may run.
//! * [`crate::command_admission::log`] — `CommandOrder`, the peer-independent
//!   total order, and `CommandDelay`, the agreed gap between admitting a
//!   command and applying it.
//!
//! # What this module deliberately does not do
//!
//! * **It does not own a socket.** A transport (the browser's DataChannel via
//!   `gui/host-mesh.js`, or the in-process channel the native harness uses)
//!   pushes into [`MeshInbox`] and drains [`MeshOutbox`]. Every decision that
//!   matters is therefore testable on native with no networking at all, which
//!   is what `tests/lockstep_mesh.rs` does.
//! * **It does not re-run the sending host's own authority check.** That host's
//!   `Sessions` made the human-vs-AI, station-tenure call, and a session token
//!   is a bearer credential that never leaves the host holding it. What a
//!   receiver DOES enforce for itself is ship ownership: a host may drive only
//!   the ship its own fleet slot flies, so [`apply_mesh_inbox`] drops any peer
//!   command whose target `ShipKey` is not the hull the sending slot owns
//!   (another player's, or an NPC's). [`frame::MeshCommand::is_from`] checks the
//!   companion frame-consistency rule (a frame's `from` and its commands' order
//!   origin agree).
//!
//!   The one authority dimension NOT yet enforced is that a frame's declared
//!   `from` slot is the connection that actually delivered it — see the
//!   `TODO(#1118/#1120 mesh hardening)` in [`apply_mesh_inbox`]. That needs a
//!   transport-authenticated sender slot, which the star topology cannot supply
//!   for a relayed sibling frame without the lead tagging it; it is deferred
//!   rather than half-built.
//! * **It does not ship AI decisions.** Every host derives every NPC from the
//!   same ticks, the same seeded streams and the same authored policies
//!   (AGENTS.md rule 6, `p2p-delta-ai-output-is-not-logged`). Only what crosses
//!   a network boundary is logged, and an AI emission never crosses one.
//! * **It does not recover from a divergence.** It *detects* one and names the
//!   tick and the peer. Electing a recovery leader and transferring the
//!   canonical record is #1118's, and [`MeshAgreement`] is the surface it takes
//!   over.
//!
//! # The tick clock is Bevy's, not this module's
//!
//! `p2p-delta-tick-is-fixedupdate` forbids both a second accumulator beside
//! `Time<Fixed>` and "a stall implemented by skipping systems inside a tick
//! rather than withholding the tick". So a stall here pauses `Time<Virtual>`,
//! which starves the fixed accumulator and stops `FixedUpdate` — and with it
//! `SimTick` — from advancing at all. That is the same seam `SimulationPaused`
//! uses (`src/server/bridge.rs`), reused rather than re-invented: a withheld
//! tick simply never starts.

use bevy::prelude::*;

use crate::command_admission::log::{
    CommandDelay, CommandOrder, HostSlot, PendingCommands, ShipKey,
};
use crate::logging::LogCat;

pub mod frame;
pub mod host_loss;
pub mod session;
pub mod snapshot_relay;
pub mod transfer;

pub use frame::{
    DigestFrame, HostLossFrame, MeshCommand, MeshFrame, TickFrame, HOST_MESH_PROTOCOL,
};
pub use host_loss::{agreed_loss_tick, HostLossRecord, PendingHostLoss};
pub use session::{LockstepSession, Stall};
pub use snapshot_relay::{
    capture_run, drain_mesh_restore, frames_for, gate_and_restore, gate_and_restore_against,
    send_snapshot, MeshRestoreOutcome, MeshSnapshotReceiver,
};
pub use transfer::{Accepted, SnapshotChunk, SnapshotReceiver, TransferError};

/// The Bevy adapter for the pure [`LockstepSession`] (AGENTS.md rule 10: the
/// decision module stays Bevy-free and its adapter is a sibling).
///
/// **Absent means "not in a fleet"**, and that is the whole of the solo path:
/// every system below returns immediately without one, so a single-player host
/// carries no barrier, no watermarks and no delay rather than carrying a
/// disabled version of each.
#[derive(Resource, Clone, Debug, PartialEq, Eq, Deref, DerefMut)]
pub struct FleetLockstep(pub LockstepSession);

/// Which fleet slot a player ship belongs to (issue #1116).
///
/// Present on every ship the frozen roster put in the world, absent on every
/// NPC. It is what tells a host that the cruiser over there is *slot 2's ship*
/// — flown by a crew on another machine — rather than one more hull for its own
/// AI to operate, and it is the key a peer's `ShipKey`-routed command is
/// ultimately answered by.
///
/// Deliberately a component and not a lookup table: the roster is frozen, so
/// the ship and its slot are born together and cannot drift apart.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FleetSlotOf(pub HostSlot);

/// One player ship in the frozen fleet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FleetShip {
    /// The host that flies it.
    pub host: HostSlot,
    /// The hull that host chose, as an entity-template path. `None` means "use
    /// whatever this host's own lobby selected", which is what a solo roster
    /// says and is why a solo run is byte-identical to a pre-#1116 one.
    pub ship_path: Option<String>,
    /// Who is aboard, as agreed when the roster froze: each crewed station and
    /// the Station Rating that crew member chose.
    ///
    /// Every host seeds every fleet ship's boot ratings from this — its own
    /// included — so two hosts cannot disagree about whether slot 2's Helm is
    /// crewed, or at what rating. They otherwise would: `Sessions` describes
    /// only the local crew, and a Rating is a per-player complexity choice that
    /// changes which systems a station automates and therefore which of them
    /// the AI operates.
    ///
    /// Empty means nobody is aboard that ship, which is the honest reading of
    /// an NPC-crewed hull. Mid-mission crew changes are #1119's; this is the
    /// frozen picture, and it is frozen because `p2p-delta-backfill-replaces-
    /// auto-crew` requires every peer to make the same transition on the same
    /// tick rather than each following its own view of who is connected.
    pub crew: Vec<(crate::core::messages::StationId, String)>,
}

impl FleetShip {
    pub fn new(host: HostSlot) -> Self {
        Self {
            host,
            ship_path: None,
            crew: Vec::new(),
        }
    }

    /// The rating a human is holding `station` at, or `None` for an empty seat.
    pub fn rating_at(&self, station: &crate::core::messages::StationId) -> Option<&str> {
        self.crew
            .iter()
            .find(|(id, _)| id == station)
            .map(|(_, rating)| rating.as_str())
    }
}

/// The frozen fleet, as the simulation sees it.
///
/// Present in every app, because "one host, flying its own ship" is a fleet of
/// one and not a special case. The default is exactly that, and it reproduces
/// pre-#1116 behaviour to the byte: one ship, spawned from the first GameStart
/// `[[entity]]` tagged `ship`, tagged [`crate::server_app::LocalShip`], flying
/// whatever the lobby selected.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct FleetRoster {
    ships: Vec<FleetShip>,
    local: HostSlot,
}

impl Default for FleetRoster {
    fn default() -> Self {
        Self {
            ships: vec![FleetShip::new(HostSlot::SOLO)],
            local: HostSlot::SOLO,
        }
    }
}

impl FleetRoster {
    /// Build a roster from the frozen host-mesh slots, in slot order.
    ///
    /// Sorted by slot rather than trusting the caller's order, because the
    /// order is what decides which authored GameStart spawn each ship takes —
    /// and therefore what `WorldIdMint` gives it. Two hosts that walked the
    /// roster differently would mint different ids for the same ship, which is
    /// the exact failure `p2p-delta-identity-is-minted` describes.
    pub fn new(mut ships: Vec<FleetShip>, local: HostSlot) -> Self {
        ships.sort_by_key(|ship| ship.host);
        if ships.is_empty() {
            ships.push(FleetShip::new(local));
        }
        Self { ships, local }
    }

    /// The ships, in the order they take the world's GameStart ship spawns.
    pub fn ships(&self) -> &[FleetShip] {
        &self.ships
    }

    /// How many player ships this world must spawn.
    pub fn len(&self) -> usize {
        self.ships.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ships.is_empty()
    }

    /// This host's own slot.
    pub fn local(&self) -> HostSlot {
        self.local
    }

    /// Whether `host` is the slot this host projects to its own crew.
    pub fn is_local(&self, host: HostSlot) -> bool {
        host == self.local
    }

    /// The nth ship in roster order.
    pub fn ship(&self, index: usize) -> Option<&FleetShip> {
        self.ships.get(index)
    }

    /// The frozen crewing of one slot's ship.
    pub fn crew_of(&self, host: HostSlot) -> &[(crate::core::messages::StationId, String)] {
        self.ships
            .iter()
            .find(|ship| ship.host == host)
            .map(|ship| ship.crew.as_slice())
            .unwrap_or_default()
    }

    /// Empty one slot's frozen crewing because its host has left (issue #1119).
    ///
    /// The ship STAYS in the roster — the fleet is not one host smaller, it is
    /// one crew short — so `is_solo` is unchanged and the survivors keep
    /// answering every ship from the frozen roster rather than falling back to
    /// their own local `Sessions`. What changes is that `crew_of(host)` is now
    /// empty, so [`resolve_human_seeking_hosts`] finds no holder for any of that
    /// ship's seats and keeps its human-seeking systems on AI. Applied at the
    /// agreed tick by [`host_loss::apply_host_loss_backfill`], so every survivor
    /// empties the same crewing on the same tick. Idempotent.
    ///
    /// [`resolve_human_seeking_hosts`]: crate::ship::coordination_systems::resolve_human_seeking_hosts
    pub fn depart_slot(&mut self, host: HostSlot) {
        if let Some(ship) = self.ships.iter_mut().find(|ship| ship.host == host) {
            ship.crew.clear();
        }
    }

    /// Whether this roster describes a lone host — the shipped single-player
    /// case, and the state every fixture is in.
    pub fn is_solo(&self) -> bool {
        self.ships.len() <= 1
    }
}

/// Frames a transport has received and this host has not applied yet.
#[derive(Resource, Default, Debug)]
pub struct MeshInbox {
    frames: Vec<MeshFrame>,
}

impl MeshInbox {
    /// Hand one received frame to the simulation.
    pub fn push(&mut self, frame: MeshFrame) {
        self.frames.push(frame);
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    fn take(&mut self) -> Vec<MeshFrame> {
        std::mem::take(&mut self.frames)
    }
}

/// Order one frame's worth of host-mesh input for [`MeshInbox`] (issue #1119).
///
/// The decoded tick/digest frames go in **before** the self-reported host-loss
/// frames, so a departing host's final watermark is OBSERVED before the loss
/// tick is derived from it. That order is load-bearing for determinism, not
/// cosmetic: when a lost host's last tick frame and its socket close arrive in
/// the same animation frame — the common case, since a host receives roughly one
/// peer frame per drain — [`apply_mesh_inbox`] must see the final frame first,
/// exactly as every OTHER survivor does over the reliable ordered relay (which
/// delivers the relayed final frame before the relayed loss report). Injecting
/// the loss first would derive [`agreed_loss_tick`] from a watermark that omits
/// the co-arriving frame, so the self-observing host would flip its peer's ship
/// to Backfill one tick EARLIER than the members do — and the two folds diverge
/// from that tick on.
///
/// A departed slot is self-reported as a [`HostLossFrame`] with the departing
/// slot as both `from` and `lost` and tick `0`: the bridge cannot know the
/// agreed tick (it never sees the watermark), so it hands over only the fact of
/// the loss and the handler derives the tick. The `from == lost` shape is an
/// advisory marker of a local self-observation; the handler re-broadcasts a
/// normalised report naming the local observer.
pub fn order_mesh_inbound(
    decoded: impl IntoIterator<Item = MeshFrame>,
    departed: impl IntoIterator<Item = HostSlot>,
) -> Vec<MeshFrame> {
    let mut ordered: Vec<MeshFrame> = decoded.into_iter().collect();
    for slot in departed {
        ordered.push(MeshFrame::HostLoss(frame::HostLossFrame {
            from: slot,
            lost: slot,
            tick: 0,
        }));
    }
    ordered
}

/// Frames this host has produced and a transport has not sent yet.
///
/// Also holds the commands admitted since the last tick frame was sealed, so
/// `admit_system_commands` has one place to hand them to and nothing else in
/// the simulation has to know a mesh exists.
#[derive(Resource, Default, Debug)]
pub struct MeshOutbox {
    staged: Vec<MeshCommand>,
    frames: Vec<MeshFrame>,
}

impl MeshOutbox {
    /// Record a command this host just admitted from its own crew.
    ///
    /// Called from admission, on the accepted branch only — a refused command
    /// is not this fleet's business, exactly as it is not the log's.
    pub fn stage(&mut self, command: MeshCommand) {
        self.staged.push(command);
    }

    /// Queue a frame for the transport.
    pub fn push(&mut self, frame: MeshFrame) {
        self.frames.push(frame);
    }

    /// Take everything waiting to go out.
    pub fn drain(&mut self) -> Vec<MeshFrame> {
        std::mem::take(&mut self.frames)
    }

    pub fn pending_frames(&self) -> &[MeshFrame] {
        &self.frames
    }
}

/// What the fleet's periodic digest exchange has found.
///
/// The surface #1118 takes over: today it *names* a divergence, and recovering
/// from one is that issue's. The comparator is deliberately not re-invented —
/// [`crate::sim_digest::DigestLedger`] already pairs samples **by tick** and
/// already produces "they agreed at tick 240 and disagreed by tick 250", so
/// this holds one ledger per peer and lets that code answer.
#[derive(Resource, Debug)]
pub struct MeshAgreement {
    /// This host's own sampled digests.
    pub local: crate::sim_digest::DigestLedger,
    /// Every peer's, as reported.
    pub peers: std::collections::BTreeMap<HostSlot, crate::sim_digest::DigestLedger>,
    /// Every tick a peer reported a digest this host disagreed with, in the
    /// order they were found.
    pub disagreements: Vec<MeshDisagreement>,
}

/// One tick two hosts folded differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshDisagreement {
    pub tick: u64,
    pub peer: HostSlot,
    pub local_digest: u64,
    pub peer_digest: u64,
}

impl std::fmt::Display for MeshDisagreement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tick {}: this host folds {:#018x}, {} folds {:#018x}",
            self.tick,
            self.local_digest,
            self.peer.slot_id(),
            self.peer_digest
        )
    }
}

impl MeshAgreement {
    /// A ledger sampling every `interval` ticks. `0` disables sampling and
    /// costs nothing, which is what a solo host wants.
    pub fn new(interval: u64) -> Self {
        Self {
            local: crate::sim_digest::DigestLedger::new(interval),
            peers: std::collections::BTreeMap::new(),
            disagreements: Vec::new(),
        }
    }

    /// Whether every digest the fleet has exchanged so far agreed.
    pub fn agreed(&self) -> bool {
        self.disagreements.is_empty()
    }

    /// The first tick the fleet is known to have disagreed on.
    pub fn first_disagreement(&self) -> Option<MeshDisagreement> {
        self.disagreements.first().copied()
    }
}

impl Default for MeshAgreement {
    fn default() -> Self {
        Self::new(0)
    }
}

/// How often, in logical ticks, a host publishes its digest to the fleet.
///
/// Not a gameplay number and not authored: it is the fleet's own diagnostic
/// cadence, and its only effect is how quickly a divergence is *named* — never
/// on what the simulation does. [ai] 300 ticks is five seconds at the default
/// 60 Hz, which is short enough that a crew notices the report in the same
/// engagement the split happened in and long enough that the exchange is
/// invisible beside the per-tick input traffic.
pub const DIGEST_INTERVAL_TICKS: u64 = 300;

/// What the barrier has had to do, for the operator and for the tests.
#[derive(Resource, Default, Debug, Clone)]
pub struct MeshDiagnostics {
    /// How many frames this host has withheld a tick on.
    pub stalled_frames: u64,
    /// The most recent stall, if there has been one.
    pub last_stall: Option<Stall>,
    /// The longest run of consecutive withheld frames.
    pub longest_stall: u64,
    current_run: u64,
}

impl MeshDiagnostics {
    fn stalled(&mut self, stall: Stall) {
        self.stalled_frames = self.stalled_frames.saturating_add(1);
        self.current_run = self.current_run.saturating_add(1);
        self.longest_stall = self.longest_stall.max(self.current_run);
        self.last_stall = Some(stall);
    }

    fn running(&mut self) {
        self.current_run = 0;
    }

    /// Whether this host is withholding a tick right now.
    pub fn is_stalled(&self) -> bool {
        self.current_run > 0
    }
}

/// System set for the frame-driven mesh work: applying what arrived and
/// deciding whether the next tick may run.
///
/// Frame-driven and not fixed-driven on purpose, and for the same reason
/// `drain_inbound` is: the decision is *whether a fixed step happens at all*,
/// so it cannot live inside the schedule it gates.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MeshSet;

/// Install the lockstep resources and the frame-driven mesh systems.
///
/// Idempotent, and safe in an app with no fleet: [`FleetRoster`]'s default is a
/// fleet of one, [`LockstepSession`] is absent until a fleet forms, and every
/// system below is inert without one.
pub fn register_lockstep(app: &mut App) {
    // The #894 digest-boundary declarations (issue #1220's registry). Every type
    // this module registers is a digest EXCLUSION, and each for a different
    // reason — see `authoritative::StateClass` for the classes and
    // `tests/authoritative_state_enumeration.rs` for the census that enforces
    // them. Nothing here is folded, and nothing here is a second copy of
    // anything that is.
    {
        use crate::authoritative::{DeclareState, StateClass};
        app
            // Who is in the fleet, which slot is this host's, and how far each
            // peer has declared itself ready. `Timer` — it is transport
            // bookkeeping about a SESSION, not state of the world: it says who
            // is connected where, on which machine, which is exactly what
            // `world_id`'s own docs exclude from simulation state. What crosses
            // from it INTO the world is the ships it spawns and the ratings it
            // seeds, and those are folded like anything else.
            .declare_state::<FleetLockstep>(StateClass::Timer, "fleet-lockstep-state")
            .declare_state::<FleetRoster>(StateClass::Timer, "fleet-lockstep-state")
            // Which host flies this hull. `Timer` for the same reason and one
            // more: it is written once at spawn from the frozen roster and never
            // again, so it carries no run state to fold. Every host holds the
            // identical value for the identical hull, which is the whole point
            // of it.
            .declare_state::<FleetSlotOf>(StateClass::Timer, "fleet-lockstep-state")
            // What a transport has delivered and not yet handed over, and what
            // the simulation has said and the transport has not yet sent.
            // `ClearedAtFold`: `apply_mesh_inbox` empties the first every frame
            // and a transport drains the second, so both are structurally empty
            // at the fold point on any correctly-running host.
            .declare_state::<MeshInbox>(StateClass::ClearedAtFold, "fleet-lockstep-state")
            .declare_state::<MeshOutbox>(StateClass::ClearedAtFold, "fleet-lockstep-state")
            // The digest exchange's own records. `Derived` — they are folds OF
            // the authoritative state and a count of the barrier's decisions, so
            // folding them would fold their inputs a second time, and a peer's
            // reported digest is not this host's state at all.
            .declare_state::<MeshAgreement>(StateClass::Derived, "fleet-agreement-state")
            .declare_state::<MeshDiagnostics>(StateClass::Derived, "fleet-agreement-state")
            // The host-loss queue and its log (issue #1119). `Timer` — like the
            // roster it drives, it is transport bookkeeping about who is
            // connected where, not state of the world: what crosses from it INTO
            // the world is the Backfill flip it triggers, and the control sources
            // and ratings that flip are classified where they already live. The
            // queue is empty on any host that has lost nobody, which is every
            // host in a healthy fleet and every solo run.
            .declare_state::<host_loss::PendingHostLoss>(
                StateClass::Timer,
                "fleet-lockstep-state",
            );
    }
    app.init_resource::<FleetRoster>()
        .init_resource::<MeshInbox>()
        .init_resource::<MeshOutbox>()
        .init_resource::<MeshDiagnostics>()
        .init_resource::<MeshAgreement>()
        .init_resource::<host_loss::PendingHostLoss>()
        .add_systems(
            PreUpdate,
            (apply_mesh_inbox, gate_lockstep_ticks)
                .chain()
                .in_set(MeshSet),
        )
        .add_systems(
            FixedLast,
            seal_tick_frame.before(crate::sim_tick::advance_sim_tick),
        )
        // Sampled from INSIDE the fixed schedule, once per fixed step, so a
        // frame that runs several steps across a checkpoint boundary cannot skip
        // the checkpoint (issue #1116). `.before(advance_sim_tick)` so `SimTick`
        // still reads the step that just committed. Gated to a running fleet so a
        // solo host takes no per-step exclusive sync point for a digest exchange
        // it has no peer to hold.
        .add_systems(
            FixedLast,
            sample_and_publish_digest
                .before(crate::sim_tick::advance_sim_tick)
                .run_if(fleet_is_running),
        )
        // The host-loss Backfill flip (issue #1119). In `SimSet::Input`, at the
        // agreed tick, on the lost ship — the same phase the ordinary rating
        // change and the human-seeking resolver run in. Ordered
        // `.after(handle_station_rating_change)` and
        // `.before(resolve_human_seeking_hosts)`: all three write
        // `ShipSystemControlSources`, and the flip must land before the resolver
        // re-reads the (now uncrewed) roster and keeps the lost ship's
        // human-seeking systems on AI. In an app without `ShipPlugin` — a bare
        // fixture — both edges are vacuous and the flip is a no-op with an empty
        // ship query.
        .add_systems(
            FixedUpdate,
            host_loss::apply_host_loss_backfill
                .in_set(crate::sim_sets::SimSet::Input)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::ship_plugin::handle_station_rating_change)
                .before(crate::ship_plugin::resolve_human_seeking_hosts),
        );
    // The portable-record transfer (issue #1117): the receiver resource and the
    // frame-driven restore that commits a fully-arrived record. Kept in its own
    // sibling so #1119's parallel work on this module does not collide with it.
    snapshot_relay::register_snapshot_relay(app);
}

/// Join a fleet: adopt the slot, the peers and the agreed delay.
///
/// The **one** writer of [`CommandDelay`], which is what makes AGENTS.md rule
/// 7's amendment checkable rather than a hope — a delay cannot appear by
/// accident anywhere else in the plumbing.
///
/// `delay` comes from the world's authored `[global] command_delay_ticks`; see
/// [`crate::lockstep::authored_delay`].
pub fn join_fleet(world: &mut World, roster: FleetRoster, delay: u64) {
    let local = roster.local();
    let peers: Vec<HostSlot> = roster.ships().iter().map(|ship| ship.host).collect();
    let alone = roster.is_solo();
    world.insert_resource(roster);
    world.insert_resource(FleetLockstep(LockstepSession::new(local, peers, delay)));
    world.insert_resource(MeshAgreement::new(if alone {
        0
    } else {
        DIGEST_INTERVAL_TICKS
    }));
    // A fleet of one waits for nobody, so it also takes no delay: a lone host
    // that stamped its own input six ticks into the future would be adding
    // latency to buy agreement with an empty set of peers.
    world.insert_resource(CommandDelay(if alone { 0 } else { delay }));
    if let Some(mut pending) = world.get_resource_mut::<PendingCommands>() {
        pending.set_origin(local);
    }
}

/// The delay this world authored, in logical ticks.
///
/// Reads `[global] command_delay_ticks`, which `world::config::parse_world`
/// has already validated. Falls back to the schema default when no world is
/// loaded, which only a bare-`App` fixture is.
pub fn authored_delay(world: &World) -> u64 {
    world
        .get_resource::<crate::world::config::WorldConfig>()
        .map(|config| u64::from(config.global.command_delay_ticks))
        .unwrap_or_else(|| u64::from(crate::entities::config::default_command_delay_ticks()))
}

/// Apply everything a transport has delivered: peer input into the future-tick
/// queue, peer digests into the agreement ledger.
///
/// Peer commands go in with **no route resolved**. That is not laziness: a
/// command stamped for tick *T + delay* may name a ship this host has not
/// spawned yet (a frame that arrives during the spawn burst) or one that will
/// be despawned and rebuilt before it lands, and a route resolved now would be
/// stale by then. `admit_system_commands` resolves every due command by its
/// [`ShipKey`] at the instant it applies, which is the only instant the answer
/// is knowable — and is the per-ship routing the replay path recorded as a
/// named gap (`src/headless/replay.rs`: "reading `ShipKey` back out and
/// resolving *it* to a destination … is issue #854's work").
pub fn apply_mesh_inbox(
    mut inbox: ResMut<MeshInbox>,
    mut session: Option<ResMut<FleetLockstep>>,
    mut pending: ResMut<PendingCommands>,
    mut pending_loss: ResMut<host_loss::PendingHostLoss>,
    mut agreement: ResMut<MeshAgreement>,
    mut snapshot_rx: ResMut<MeshSnapshotReceiver>,
    mut outbox: ResMut<MeshOutbox>,
    fleet_ships: Query<(&FleetSlotOf, &crate::entities::spawner::EntityUuid)>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    if inbox.is_empty() {
        return;
    }
    let frames = inbox.take();
    // Snapshot chunks (issue #1117) are handled whether or not this host is a
    // lockstep participant: RECEIVING the record is how a host becomes one (a
    // join, a #1120 slot recovery), so a chunk must not be dropped by the
    // fleet-session gate below. Peeled off first; the buffer they accumulate is
    // bounded, and nothing is committed until the whole record has arrived, gated
    // and restored. `snapshot_relay::drain_mesh_restore` does the committing.
    let mut sim_frames = Vec::with_capacity(frames.len());
    for frame in frames {
        match frame {
            MeshFrame::Snapshot(chunk) => {
                snapshot_relay::receive_chunk(&mut snapshot_rx, &chunk, &log);
            }
            other => sim_frames.push(other),
        }
    }
    // The hull each fleet slot actually flies, keyed by slot. A host may speak
    // only for the ship ITS OWN slot owns (`frame::MeshCommand`'s contract), so
    // a peer command naming any other hull — another player's, or an NPC's — is
    // refused before it is queued. The roster froze which slot flies which hull;
    // this is that same fact read off the spawned ships, where the minted uuid
    // (the `ShipKey` a command routes by) actually lives.
    let owned_ship: std::collections::HashMap<HostSlot, String> = fleet_ships
        .iter()
        .map(|(slot, uuid)| (slot.0, uuid.0.clone()))
        .collect();
    let Some(session) = session.as_deref_mut() else {
        // No fleet: a tick or digest frame that arrives before this host has
        // joined one is dropped rather than queued, because there is no agreed
        // order to put it in and applying it would be exactly the unilateral
        // admission lockstep exists to prevent. Snapshot chunks were already
        // handled above — they are how a host joins in the first place.
        if !sim_frames.is_empty() {
            crate::pwarn!(
                log,
                LogCat::Admit,
                "dropping {} host-mesh frame(s): this host is not in a fleet",
                sim_frames.len()
            );
        }
        return;
    };
    for frame in sim_frames {
        match frame {
            MeshFrame::Tick(tick) => {
                if tick.from == session.local() {
                    continue;
                }
                // TODO(#1118/#1120 mesh hardening): `tick.from` is trusted from
                // the frame, not authenticated to the connection that delivered
                // it. A transport-authenticated sender slot (JS binds conn->slot
                // and passes it across `wasm_receive_mesh_frame`) would let this
                // reject a frame whose `from` disagrees with its origin. It is
                // deferred because the star relay means a member cannot
                // authenticate a sibling frame the lead forwarded without the
                // lead tagging it — a change to #1114's verbatim-relay envelope.
                // The ship-ownership check below still holds regardless: a
                // spoofed `from` can only speak for the hull that slot owns.
                for command in tick.commands {
                    if !command.is_from(tick.from) {
                        crate::pwarn!(
                            log,
                            LogCat::Admit,
                            "{} sent a command ordered under {} — dropped",
                            tick.from.slot_id(),
                            command.order.origin.slot_id(),
                        );
                        continue;
                    }
                    // A host may drive only the ship its own slot flies. Without
                    // this a peer at slot 2 could order a command under slot 2
                    // (passing `is_from`) yet name slot 1's hull — or an NPC — in
                    // `command.ship`, and the `ShipKey` apply route would deliver
                    // it there on every host. The command is dropped unless the
                    // ship it targets is the one the sending slot owns. A slot
                    // whose ship this host has not spawned (or has despawned) owns
                    // nothing here, so its claim cannot be verified and is
                    // refused — the fleet's player ships are all spawned at
                    // GameStart, before any crew command can be issued for them.
                    if owned_ship.get(&tick.from).map(String::as_str)
                        != Some(command.ship.0.as_str())
                    {
                        crate::pwarn!(
                            log,
                            LogCat::Admit,
                            "{} sent a command for ship {:?}, which its slot does \
                             not fly — dropped",
                            tick.from.slot_id(),
                            command.ship.0,
                        );
                        continue;
                    }
                    let admitted = crate::core::messages::AdmittedCommand {
                        target: command.target.clone(),
                        payload: command.payload.clone(),
                        // No reply address: the crew that asked is on another
                        // host, and answering them is that host's job.
                        response_token: None,
                    };
                    crate::command_admission::log::stamp_accepted_command(
                        &mut pending,
                        command.tick,
                        Some(command.order),
                        Entity::PLACEHOLDER,
                        command.ship.clone(),
                        admitted,
                    );
                }
                session.observe(tick.from, tick.ready_through);
            }
            MeshFrame::Digest(digest) => {
                if digest.from == session.local() {
                    continue;
                }
                if let Some(local) = agreement.local.digest_at(digest.tick) {
                    if local != digest.digest {
                        let found = MeshDisagreement {
                            tick: digest.tick,
                            peer: digest.from,
                            local_digest: local,
                            peer_digest: digest.digest,
                        };
                        if !agreement.disagreements.contains(&found) {
                            crate::perror!(
                                log,
                                LogCat::Admit,
                                "host-mesh divergence — {found}. The fleet's \
                                 command logs are the first place to look: they \
                                 are byte-identical on hosts that agree.",
                            );
                            agreement.disagreements.push(found);
                        }
                    }
                }
                agreement
                    .peers
                    .entry(digest.from)
                    .or_insert_with(|| crate::sim_digest::DigestLedger::new(DIGEST_INTERVAL_TICKS))
                    .record(digest.tick, digest.digest);
            }
            MeshFrame::HostLoss(hl) => {
                let lost = hl.lost;
                // This host cannot be told it is itself lost, and a host does not
                // report its own loss — either is somebody else's confusion.
                if lost == session.local() {
                    continue;
                }
                // The flip has already applied; it cannot be re-agreed, so a late
                // duplicate report is inert (AC5).
                if pending_loss.is_applied(lost) {
                    continue;
                }
                let observed = session.watermark_of(lost);
                if observed.is_none() && !pending_loss.is_known(lost) {
                    // A report about a slot this host has never known as a peer,
                    // and has no pending loss queued for: there is nothing to lose,
                    // so it is dropped rather than scheduling a flip for a ship
                    // that is not here.
                    crate::pwarn!(
                        log,
                        LogCat::Admit,
                        "dropping a host-loss report for {}: this host has no \
                         such peer",
                        lost.slot_id(),
                    );
                    continue;
                }
                // The agreed disconnect tick, and why it is NEVER re-derived from a
                // local watermark on a relayed report (issue #1119 determinism).
                //
                // A lost slot's final watermark is shared across survivors only for
                // a genuinely-departed slot whose last frame preceded the loss
                // report in the one reliable ordered stream. A LIVE slot — or one
                // whose loss is still propagating — has a skew-prone watermark: the
                // relay lead sees a peer's frame before it forwards it, so at any
                // instant honest survivors legitimately hold DIFFERENT
                // `watermark_of(lost)` values for that slot. Deciding the tick — or
                // an accept/refuse — against that raw local watermark let two
                // honest survivors disagree whenever a report's tick fell between
                // their watermarks: one flipped the ship to Backfill and the other
                // did not, a permanent fold divergence with no reconciliation path.
                //
                // So the derivation has ONE authority per loss. In the star relay
                // (`p2p-delta-transport-is-a-star-today`) exactly one host — the
                // connection holder — sees a ship host's socket close; the bridge
                // hands it a self-observation carrying tick `0`. THAT host derives
                // the tick from the lost slot's own last watermark, which
                // `order_mesh_inbound` guarantees already includes the co-arriving
                // final frame, and stamps the concrete tick into the report it
                // re-broadcasts. Every OTHER survivor receives that report with a
                // non-zero carried tick and honours it VERBATIM — adopting the one
                // authority's figure rather than re-deriving from its own skewed
                // watermark — so all survivors converge on the identical tick under
                // any frame-arrival interleaving.
                //
                // TODO(#1118/#1120 mesh hardening): a relayed report is honoured
                // whether or not it is genuine. Honouring it is CONVERGENT — every
                // survivor that receives it agrees the same tick, so the fold never
                // diverges — but a forged or replayed `HostLoss` for a live slot is
                // still acted on (its ship flips to Backfill on every host that
                // hears it). REJECTING a forged report cannot be done
                // deterministically here: the only ground-truth signal that a slot
                // is alive is the transport connection this host does not hold (only
                // the relay lead does), and a bounded liveness check against a raw
                // watermark is exactly the skew-prone decision that diverged the
                // fold. So reporter authenticity is deferred to sender
                // authentication (#1118/#1120), parity with the Tick arm's
                // unauthenticated `from`; until then an unauthenticated loss is
                // honoured convergently.
                let agreed = if hl.tick == 0 {
                    // Self-observed local close: this host's own transport saw the
                    // socket close, so it is the authority that derives the tick.
                    // `map_or(0, …)` covers a close for a peer this host is already
                    // departing (watermark cleared, loss still pending): `observe`
                    // below keeps the higher tick already queued.
                    observed.map_or(0, host_loss::agreed_loss_tick)
                } else {
                    // Relayed, already-agreed report: honour the carried tick
                    // VERBATIM. Not re-derived, not merged with a local watermark —
                    // that is the whole of the convergence argument above.
                    hl.tick
                };
                let changed = pending_loss.observe(lost, agreed);
                // Stop the barrier waiting for the departed host so the fleet
                // resumes at once — the ship's Backfill flip is tick-stamped for
                // `agreed` and applied in the fixed schedule, so this frame-driven
                // half moves no folded state.
                session.depart(lost);
                if changed {
                    // Re-broadcast so the rest of the fleet converges — the
                    // propagation that lets a relay member learn a loss it did not
                    // see the socket close for. The report is normalised to name
                    // THIS host as the observer and to carry the agreed tick.
                    outbox.push(MeshFrame::HostLoss(frame::HostLossFrame {
                        from: session.local(),
                        lost,
                        tick: agreed,
                    }));
                    crate::pinfo!(
                        log,
                        LogCat::Admit,
                        "host-mesh: {} left; agreeing its ship flips to Backfill \
                         at tick {}",
                        lost.slot_id(),
                        agreed,
                    );
                }
            }
            // Peeled off above, before the fleet-session gate, so it never reaches
            // this loop — but the match stays exhaustive rather than resting on
            // that being remembered.
            MeshFrame::Snapshot(_) => {}
        }
    }
}

/// Withhold the next tick until every peer's input for it is in hand.
///
/// A host that is ahead of the fleet **waits honestly**: `Time<Virtual>` is
/// paused, so the fixed accumulator stops and the tick never begins. Nothing is
/// speculated and nothing is rolled back — `p2p-delta-tick-is-fixedupdate`
/// rules out both a second accumulator and a stall that skips systems inside a
/// tick, and this is neither.
///
/// `SimTick` read outside a fixed step is the number of completed steps, which
/// is the index of the step about to run — so that is the tick the barrier asks
/// about.
pub fn gate_lockstep_ticks(
    session: Option<Res<FleetLockstep>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    paused: Option<Res<crate::debug_overlay::SimulationPaused>>,
    virtual_time: Option<ResMut<Time<Virtual>>>,
    mut diagnostics: ResMut<MeshDiagnostics>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    let Some(session) = session else {
        return;
    };
    // `Option` for the same reason `SimTick` is taken as one in admission: a
    // bare-`App` fixture with no `TimePlugin` would otherwise fail Bevy's
    // parameter validation and skip this system entirely, which is a silent
    // way to lose the barrier.
    let Some(mut virtual_time) = virtual_time else {
        return;
    };
    if session.is_alone() {
        return;
    }
    // The operator's own pause owns the clock while it is on; the barrier must
    // not un-pause it out from under them.
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let next_tick = sim_tick.map_or(0, |t| t.0);
    match session.stall_at(next_tick) {
        Some(stall) => {
            if !virtual_time.is_paused() {
                crate::pwarn!(log, LogCat::Admit, "host-mesh stall: {stall}");
                virtual_time.pause();
            }
            diagnostics.stalled(stall);
        }
        None => {
            if virtual_time.is_paused() {
                crate::pinfo!(log, LogCat::Admit, "host-mesh resumed at tick {next_tick}");
                virtual_time.unpause();
            }
            diagnostics.running();
        }
    }
}

/// Seal this tick's admitted commands into a frame for the fleet.
///
/// Runs in `FixedLast` **before** `advance_sim_tick`, so `SimTick` still reads
/// the index of the step that just ran: the frame reports the tick this host
/// has completed and the watermark that follows from it. A frame is emitted
/// every tick even when there is nothing to say, because "no input" is an
/// answer a peer has to *receive* — a fleet whose hosts only spoke when they
/// had something to say would stall until somebody pressed a key.
pub fn seal_tick_frame(
    session: Option<Res<FleetLockstep>>,
    sim_tick: Res<crate::sim_tick::SimTick>,
    mut outbox: ResMut<MeshOutbox>,
) {
    let Some(session) = session else {
        outbox.staged.clear();
        return;
    };
    if session.is_alone() {
        outbox.staged.clear();
        return;
    }
    let tick = sim_tick.0;
    let commands = std::mem::take(&mut outbox.staged);
    outbox.push(MeshFrame::Tick(TickFrame {
        from: session.local(),
        tick,
        ready_through: session.ready_through(tick),
        commands,
    }));
}

/// Whether this host is in a fleet with at least one peer — the only state in
/// which the digest exchange has anybody to compare with. A solo host (no
/// session, or a fleet of one) skips it, so it takes no per-step exclusive sync
/// point for an exchange that would fold nothing anyone receives.
fn fleet_is_running(session: Option<Res<FleetLockstep>>) -> bool {
    session.is_some_and(|s| !s.is_alone())
}

/// Fold this host's authoritative state at a sampled tick and publish it.
///
/// In `FixedLast`, **before** `advance_sim_tick` — the same fold point
/// `seal_tick_frame` uses: `SimSet` has fully committed this step's tick (and
/// so has the fixed-schedule `StateTransition` that follows it), and `SimTick`
/// still reads the tick that just ran. `sim_digest`'s fold-point rule — after a
/// tick has committed, before frame-time interpolation — holds here for the
/// same reason it held in `Last`: render interpolation writes `Transform` in
/// the frame schedules, which no fixed step touches and the fold does not read.
///
/// Sampling **per fixed step** rather than once per frame is what closes the
/// mid-frame-checkpoint hole (issue #1116): a frame that catches up several
/// fixed steps can advance `SimTick` past a checkpoint multiple (e.g. 299 → 301
/// across interval 300), and a once-per-frame sampler that only saw the final
/// `SimTick` would never fold or publish that checkpoint — so a real divergence
/// at exactly that tick would go undetected. Driven from inside the fixed loop,
/// every checkpoint tick the frame crosses is observed.
/// [`crate::sim_digest::DigestLedger::record`] still refuses a duplicate for a
/// tick already at the head of the ledger.
pub fn sample_and_publish_digest(world: &mut World) {
    let Some(session) = world.get_resource::<FleetLockstep>() else {
        return;
    };
    if session.is_alone() {
        return;
    }
    let from = session.local();
    let tick = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |t| t.0);
    let samples = world
        .get_resource::<MeshAgreement>()
        .is_some_and(|a| a.local.samples(tick) && a.local.digest_at(tick).is_none());
    if !samples {
        return;
    }
    let digest = crate::sim_digest::world_digest(world);
    if let Some(mut agreement) = world.get_resource_mut::<MeshAgreement>() {
        agreement.local.record(tick, digest);
    }
    if let Some(mut outbox) = world.get_resource_mut::<MeshOutbox>() {
        outbox.push(MeshFrame::Digest(DigestFrame { from, tick, digest }));
    }
}

/// Build the [`MeshCommand`] a locally-admitted command crosses the wire as.
///
/// Lives here rather than in admission so that admission needs to know nothing
/// about the mesh beyond "hand the accepted command over"; the projection from
/// an accepted command to a fleet-visible one is this module's vocabulary.
pub fn mesh_command(
    tick: u64,
    order: CommandOrder,
    ship: ShipKey,
    command: &crate::core::messages::AdmittedCommand,
) -> MeshCommand {
    MeshCommand {
        tick,
        order,
        ship,
        target: command.target.clone(),
        payload: command.payload.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{StationId, SystemControlPayload, SystemId};

    fn roster() -> FleetRoster {
        FleetRoster::new(
            vec![
                FleetShip {
                    host: HostSlot(2),
                    ship_path: Some("b.toml".into()),
                    crew: vec![(StationId("helm".into()), "Std".into())],
                },
                FleetShip {
                    host: HostSlot(1),
                    ship_path: Some("a.toml".into()),
                    crew: vec![],
                },
            ],
            HostSlot(2),
        )
    }

    /// The roster walks in slot order whatever order it was handed in, because
    /// that order decides which authored spawn each ship takes — and therefore
    /// what the mint gives it.
    #[test]
    fn a_roster_is_ordered_by_slot_not_by_arrival() {
        let roster = roster();
        let slots: Vec<HostSlot> = roster.ships().iter().map(|s| s.host).collect();
        assert_eq!(slots, vec![HostSlot(1), HostSlot(2)]);
        assert_eq!(roster.ship(0).unwrap().ship_path.as_deref(), Some("a.toml"));
        assert!(roster.is_local(HostSlot(2)));
        assert!(!roster.is_local(HostSlot(1)));
        assert!(!roster.is_solo());
    }

    /// The default roster is the shipped single-player case, spelled out rather
    /// than left implicit: one ship, this host's, flying the lobby's choice.
    #[test]
    fn the_default_roster_is_a_fleet_of_one() {
        let roster = FleetRoster::default();
        assert!(roster.is_solo());
        assert_eq!(roster.len(), 1);
        assert!(roster.is_local(HostSlot::SOLO));
        assert_eq!(roster.ship(0).unwrap().ship_path, None);
    }

    /// The frozen crewing is per slot, so a host can answer "is slot 2's Helm
    /// crewed, and at what rating?" without a session for slot 2's crew — which
    /// it never has, and never will.
    #[test]
    fn crewing_is_answerable_for_a_ship_this_host_has_no_sessions_for() {
        let roster = roster();
        assert_eq!(
            roster.crew_of(HostSlot(2)),
            &[(StationId("helm".into()), "Std".to_string())]
        );
        assert_eq!(
            roster.ship(1).unwrap().rating_at(&StationId("helm".into())),
            Some("Std"),
            "the RATING travels with the seat: it decides which systems the              station automates, so two hosts holding different ones would run              different AI on the same ship"
        );
        assert!(roster.crew_of(HostSlot(1)).is_empty());
        assert!(roster.crew_of(HostSlot(9)).is_empty());
    }

    /// A disagreement renders both digests and names the peer, because "the
    /// fleet diverged" without a tick and a slot is not actionable.
    #[test]
    fn a_disagreement_names_the_tick_the_peer_and_both_digests() {
        let found = MeshDisagreement {
            tick: 240,
            peer: HostSlot(2),
            local_digest: 1,
            peer_digest: 2,
        };
        let text = found.to_string();
        assert!(text.contains("240"), "{text}");
        assert!(text.contains("slot-2"), "{text}");
    }

    /// The diagnostics count a run of withheld frames, not just the fact of
    /// one: a fleet that stalls for a frame is normal jitter and a fleet that
    /// stalls for a thousand is broken, and the resource has to tell them apart.
    #[test]
    fn the_diagnostics_measure_the_longest_stall_not_just_the_last() {
        let mut diagnostics = MeshDiagnostics::default();
        let stall = |tick| Stall {
            tick,
            waiting_on: vec![(HostSlot(2), 0)],
        };
        diagnostics.stalled(stall(1));
        diagnostics.running();
        for tick in 0..3 {
            diagnostics.stalled(stall(tick));
        }
        assert_eq!(diagnostics.stalled_frames, 4);
        assert_eq!(diagnostics.longest_stall, 3);
        assert!(diagnostics.is_stalled());
        diagnostics.running();
        assert!(!diagnostics.is_stalled());
        assert_eq!(diagnostics.longest_stall, 3, "the peak is remembered");
    }

    /// The projection an accepted command crosses the wire as keeps everything
    /// a peer needs and nothing it must not have.
    #[test]
    fn a_mesh_command_carries_no_session_token() {
        let admitted = crate::core::messages::AdmittedCommand {
            target: SystemId("helm".into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
            response_token: Some("session-token-aaaa".into()),
        };
        let crossed = mesh_command(
            9,
            CommandOrder::new(HostSlot(1), 3),
            ShipKey("uuid-ship".into()),
            &admitted,
        );
        assert_eq!(crossed.tick, 9);
        assert_eq!(crossed.ship, ShipKey("uuid-ship".into()));
        assert!(
            !format!("{crossed:?}").contains("session-token"),
            "the token is a bearer credential and the wire is exactly where it \
             must not go"
        );
    }

    /// A frame that catches up several fixed steps across a checkpoint boundary
    /// still folds AND publishes that checkpoint (issue #1116).
    ///
    /// The digest exchange samples from inside the fixed schedule, so a
    /// `SimTick` that jumps past a checkpoint multiple mid-frame — a browser
    /// under load, a post-stall unpause — cannot skip it. A once-per-frame
    /// sampler that only read the end-of-frame `SimTick` would never observe
    /// tick 5 here, and a real divergence at exactly that tick would then go
    /// unreported until a later checkpoint, if ever.
    #[test]
    fn a_multi_step_frame_samples_every_checkpoint_it_crosses() {
        use crate::command_admission::log::PendingCommands;
        use crate::sim_tick::{register_sim_tick, SimTick};

        const INTERVAL: u64 = 5;
        let period = std::time::Duration::from_millis(10);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.init_resource::<PendingCommands>();
        register_sim_tick(&mut app);
        // The PRODUCTION wiring: `register_lockstep` is what places the sampler
        // in the fixed schedule, so this guards that registration, not a stand-in.
        register_lockstep(&mut app);

        // A running two-host fleet whose peer is already known to be ready far
        // ahead, so the barrier never withholds a step and this stays a test of
        // the sampler rather than of the wait.
        let roster = FleetRoster::new(
            vec![FleetShip::new(HostSlot(1)), FleetShip::new(HostSlot(2))],
            HostSlot(1),
        );
        join_fleet(app.world_mut(), roster, 0);
        app.insert_resource(MeshAgreement::new(INTERVAL));
        app.world_mut()
            .resource_mut::<FleetLockstep>()
            .observe(HostSlot(2), u64::MAX);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);

        // A single fixed step per frame up to just below the checkpoint: the
        // first update carries a zero delta and steps nothing, then four one-step
        // frames leave `SimTick` at 4 with only tick 0 sampled.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
        app.update();
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<SimTick>().0,
            4,
            "precondition: at tick 4"
        );
        assert!(
            app.world()
                .resource::<MeshAgreement>()
                .local
                .digest_at(INTERVAL)
                .is_none(),
            "precondition: the checkpoint tick has not been reached yet"
        );

        // One frame worth two fixed steps: `SimTick` 4 -> 6, crossing checkpoint
        // 5 in the MIDDLE of the frame. A sampler reading only the end-of-frame
        // `SimTick` (6) would never see 5.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 2));
        app.update();
        assert_eq!(
            app.world().resource::<SimTick>().0,
            6,
            "the frame ran two steps"
        );

        assert!(
            app.world()
                .resource::<MeshAgreement>()
                .local
                .digest_at(INTERVAL)
                .is_some(),
            "the mid-frame checkpoint at tick {INTERVAL} was skipped — the \
             sampler is not folding every checkpoint the frame crosses"
        );
        let published = app
            .world()
            .resource::<MeshOutbox>()
            .pending_frames()
            .iter()
            .any(|f| matches!(f, MeshFrame::Digest(d) if d.tick == INTERVAL));
        assert!(
            published,
            "the crossed checkpoint must be PUBLISHED to the fleet, not just \
             recorded — a peer that never hears it cannot compare against it"
        );
    }
}
