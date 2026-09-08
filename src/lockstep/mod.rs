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
//!   The frame's declared `from` slot IS now authenticated to the connection
//!   that delivered it (issue #1120): [`MeshInbox`] carries a [`MeshOrigin`]
//!   beside each frame, and [`apply_mesh_inbox`] drops one whose declared origin
//!   disagrees with the connection's authenticated slot or names a non-roster
//!   peer. The star's own accommodation — a member trusts the lead's verbatim
//!   relay of a sibling frame it cannot itself re-authenticate — is documented on
//!   [`MeshOrigin`], along with the byzantine-lead residue the rendezvous cutover
//!   would close.
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

#[cfg(test)]
use crate::sim_rng::InstallSimRng;
#[cfg(test)]
use crate::world_id::InstallWorldIdMint;
use bevy::prelude::*;

use crate::command_admission::log::{
    CommandDelay, CommandOrder, HostSlot, PendingCommands, ShipKey,
};
use crate::logging::LogCat;

pub mod crew;
pub mod frame;
pub mod host_loss;
pub mod recovery;
pub mod recovery_plan;
pub mod session;
pub mod slot_recovery;
pub mod snapshot_relay;
pub mod transfer;

pub use frame::{
    DigestFrame, HostLossFrame, MeshCommand, MeshFrame, SlotClaimFrame, TickFrame,
    HOST_MESH_PROTOCOL,
};
pub use host_loss::{agreed_loss_tick, HostLossRecord, PendingHostLoss};
pub use session::{LockstepSession, Stall};
pub use slot_recovery::{
    leader_for, PendingSlotClaims, SlotRecoveryHold, SlotRecoveryLog, SlotRecoveryRecord,
    SlotRecoveryResult, SlotRecoveryState,
};
pub use snapshot_relay::{
    capture_join_run, capture_run, drain_mesh_restore, frames_for, gate_and_restore,
    gate_and_restore_against, send_snapshot, MeshRestoreArm, MeshRestoreOutcome,
    MeshSnapshotReceiver,
};
pub use transfer::{Accepted, SnapshotChunk, SnapshotReceiver, TransferError};

/// Shared logical epoch at which a browser-booted participant mesh activates.
///
/// Lobby pages are free-running before adoption and therefore do not share a
/// useful `SimTick`. Tick zero is already occupied by Startup's immediate world
/// entities, while pre-game callbacks, deadlines and GameStart entities are all
/// gated until `InProgress`. Tick one is therefore the first common, unused
/// simulation boundary: it avoids reusing tick-zero `WorldId`s without jumping
/// a fresh lobby clock far enough to expire absolute-tick state.
pub const FLEET_ACTIVATION_TICK: u64 = 1;

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
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Private binding between one authenticated technical slot and the public GM
/// identity it may use for privileged actions. This never enters the crew
/// roster projection; every simulation peer needs it to reject a ship host
/// claiming an operator id it does not own.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FleetGm {
    pub host: HostSlot,
    pub operator_id: String,
}

/// The frozen fleet, as the simulation sees it.
///
/// Present in every app, because "one host, flying its own ship" is a fleet of
/// one and not a special case. The default is exactly that, and it reproduces
/// pre-#1116 behaviour to the byte: one ship, spawned from the first GameStart
/// `[[entity]]` tagged `ship`, tagged [`crate::server_app::LocalShip`], flying
/// whatever the lobby selected.
#[derive(Resource, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FleetRoster {
    ships: Vec<FleetShip>,
    /// Every technical simulation participant, including stationless GM hosts.
    ///
    /// This is deliberately separate from `ships`: a GM-only participant still
    /// owns a Rust simulation and therefore must contribute a lockstep watermark,
    /// while a fleet may contain an authored ship whose original host has left.
    #[serde(default)]
    participants: Vec<HostSlot>,
    /// Privileged operator bindings, sorted by technical slot.
    #[serde(default)]
    gms: Vec<FleetGm>,
    local: HostSlot,
    /// The technical owner whose authenticated tick frame may order fleet-wide
    /// control decisions such as a coordinated start.
    ///
    /// `SOLO` is also the backward-compatible sentinel for records written before
    /// this field existed; [`Self::owner`] then derives the old lowest-slot lead.
    #[serde(default)]
    owner: HostSlot,
}

/// Result of asynchronously adopting a browser-supplied technical roster.
///
/// `wasm_join_fleet` can validate and enqueue while no Bevy `World` is
/// available; only the next `PreUpdate` can actually install the participant
/// wait-set and canonical activation state. This generation-stamped status is
/// the acknowledgement boundary that keeps a stale acceptance from blessing a
/// newer queued roster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FleetJoinStatusKind {
    #[default]
    Idle,
    Pending,
    Accepted,
    Refused,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct FleetJoinStatus {
    pub generation: u64,
    pub status: FleetJoinStatusKind,
    pub reason: Option<String>,
}

impl Default for FleetRoster {
    fn default() -> Self {
        Self {
            ships: vec![FleetShip::new(HostSlot::SOLO)],
            participants: vec![HostSlot::SOLO],
            gms: Vec::new(),
            local: HostSlot::SOLO,
            owner: HostSlot::SOLO,
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
        let mut participants: Vec<_> = ships.iter().map(|ship| ship.host).collect();
        participants.push(local);
        participants.sort_unstable();
        participants.dedup();
        let owner = participants.first().copied().unwrap_or(local);
        Self {
            ships,
            participants,
            gms: Vec::new(),
            local,
            owner,
        }
    }

    /// Build the exact private technical roster supplied by the host mesh.
    ///
    /// Unlike [`Self::new`], this constructor never invents a ship. That makes an
    /// explicitly participant-only GM simulation representable without changing
    /// the default solo path used by existing fixtures. Every ship host must also
    /// be a technical participant, and both the local slot and owner must be in
    /// the bounded, unique participant set.
    pub fn with_participants(
        ships: Vec<FleetShip>,
        participants: Vec<HostSlot>,
        local: HostSlot,
        owner: HostSlot,
    ) -> Option<Self> {
        Self::with_participants_and_gms(ships, participants, Vec::new(), local, owner)
    }

    /// Build the frozen topology plus its private GM-slot bindings.
    pub fn with_participants_and_gms(
        mut ships: Vec<FleetShip>,
        mut participants: Vec<HostSlot>,
        mut gms: Vec<FleetGm>,
        local: HostSlot,
        owner: HostSlot,
    ) -> Option<Self> {
        ships.sort_by_key(|ship| ship.host);
        if ships.windows(2).any(|pair| pair[0].host == pair[1].host) {
            return None;
        }
        participants.sort_unstable();
        gms.sort_by(|left, right| {
            (left.host, left.operator_id.as_str()).cmp(&(right.host, right.operator_id.as_str()))
        });
        if participants.is_empty()
            || participants.windows(2).any(|pair| pair[0] == pair[1])
            || !participants.contains(&local)
            || !participants.contains(&owner)
            || ships.iter().any(|ship| !participants.contains(&ship.host))
            || gms.iter().any(|gm| {
                !participants.contains(&gm.host)
                    || gm.operator_id.is_empty()
                    || gm.operator_id.chars().count() > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
                    || ships.iter().any(|ship| ship.host == gm.host)
            })
            || gms.windows(2).any(|pair| {
                pair[0].host == pair[1].host || pair[0].operator_id == pair[1].operator_id
            })
        {
            return None;
        }
        Some(Self {
            ships,
            participants,
            gms,
            local,
            owner,
        })
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

    /// Technical participants in deterministic slot order.
    ///
    /// The fallback covers old serialized rosters whose new private field was
    /// absent. New rosters always carry an explicit non-empty participant set.
    pub fn participants(&self) -> Vec<HostSlot> {
        if self.participants.is_empty() {
            let mut participants: Vec<_> = self.ships.iter().map(|ship| ship.host).collect();
            participants.push(self.local);
            participants.sort_unstable();
            participants.dedup();
            participants
        } else {
            self.participants.clone()
        }
    }

    /// The public operator identity authenticated to `host`, if that technical
    /// participant is a GM rather than a ship host.
    pub fn gm_operator(&self, host: HostSlot) -> Option<&str> {
        self.gms
            .iter()
            .find(|gm| gm.host == host)
            .map(|gm| gm.operator_id.as_str())
    }

    pub fn gms(&self) -> &[FleetGm] {
        &self.gms
    }

    /// Technical owner of the participant mesh.
    pub fn owner(&self) -> HostSlot {
        if self.owner == HostSlot::SOLO && self.local != HostSlot::SOLO {
            self.participants().into_iter().next().unwrap_or(self.local)
        } else {
            self.owner
        }
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

    /// Keep the saved ship topology while releasing every old crew assignment.
    ///
    /// A peer-local save starts a new independent session, not a reconstruction
    /// of the host mesh that happened to be connected when it was captured.
    /// The hulls and this peer's local slot still decide which authored ships
    /// bootstrap, but nobody from the old session is considered connected to
    /// them. The local ship can therefore be claimed through this new App's
    /// ordinary [`crate::lobby::Sessions`], while the other ships begin on AI
    /// backfill.
    pub fn into_uncrewed(mut self) -> Self {
        for ship in &mut self.ships {
            ship.crew.clear();
        }
        // A saved fleet is starting as one new independent simulation. Preserve
        // its authored ships, but do not retain technical participants whose
        // transports are deliberately not recreated.
        self.participants.clear();
        self.participants.push(self.local);
        self.gms.retain(|gm| gm.host == self.local);
        self.owner = self.local;
        self
    }

    /// Whether this roster describes a lone host — the shipped single-player
    /// case, and the state every fixture is in.
    pub fn is_solo(&self) -> bool {
        self.participants().len() <= 1
    }

    /// The fleet lead: the lowest slot, which `gui/host-mesh.js` always mints as
    /// the owner (`slot-1`) — the star centre through which every member frame
    /// passes (issue #1120). Used by [`apply_mesh_inbox`]'s sender-auth check as
    /// the one slot allowed to relay a sibling's frame. `SOLO` for a solo roster,
    /// which has no peers to authenticate.
    pub fn lead(&self) -> HostSlot {
        self.owner()
    }

    /// Whether `host` is a slot in this frozen roster — a member of the fleet,
    /// whether currently connected, departed, or being recovered (issue #1120).
    ///
    /// Read off the frozen roster rather than the live barrier wait-set on
    /// purpose: a departed or recovering slot (#1119/#1120) is no longer a peer
    /// the barrier waits for, yet it is still a roster member whose frames the
    /// sender-auth check must recognise.
    pub fn is_member(&self, host: HostSlot) -> bool {
        self.participants().contains(&host)
    }
}

/// Whether `host` takes its crew state from this App's live [`Sessions`].
///
/// Ship count is not the authority boundary: a saved multi-ship roster can run
/// without its old peer mesh. In that standalone state only the local ship can
/// have newly connected crew, while every saved remote ship remains frozen on
/// its now-empty roster crew (AI Backfill). Once a [`FleetLockstep`] session is
/// active, every ship — the local one included — must use the identically frozen
/// roster so peers cannot derive different control sources.
pub(crate) fn uses_live_sessions(
    roster: &FleetRoster,
    fleet_lockstep_active: bool,
    host: HostSlot,
) -> bool {
    !fleet_lockstep_active && roster.is_local(host)
}

/// Who a transport says delivered a frame — the mesh-boundary authentication
/// this issue (#1120) owns, deferred here by #1117/#1118/#1119.
///
/// Every mesh frame declares its own `from` slot, but that field is peer-supplied
/// and forgeable. A transport that binds each connection to the fleet slot it was
/// admitted as (JS: `conn -> slot` at join, `gui/fleet-session.js`) can hand the
/// simulation the AUTHENTICATED delivering slot beside the frame, and
/// [`apply_mesh_inbox`] rejects a frame whose declared `from` disagrees with it.
///
/// # Why the check is a pure function of the frame and this origin — determinism
///
/// The rejection is at ingress and is identical on every honest host, because the
/// authenticated origin is a stable transport fact (which connection carried the
/// frame), never an arrival-order or watermark value that skews across peers — the
/// #1119 lesson that two fix rounds failed for by deciding against a raw local
/// watermark. Every honest host that receives a given frame sees the same
/// authenticated origin over the reliable relay, so all make the same accept/reject
/// call and the fold cannot diverge from the check.
///
/// # The star accommodation, and its honest limit
///
/// The transport is a star with the fleet lead (the owner, always the lowest slot)
/// at the centre (`p2p-delta-transport-is-a-star-today`). The lead authenticates
/// every member frame directly against the connection it arrived on. A member,
/// though, has ONE connection — to the lead — over which BOTH the lead's own frames
/// and the lead's verbatim relay of a sibling's frame arrive; it cannot itself
/// re-authenticate a relayed sibling. So the rule [`Self::refuses`] enforces is
/// `from == authenticated` OR `authenticated == lead`: a peer may speak only for
/// itself, and the lead may relay for any roster peer (it caught a forged `from` at
/// its own ingress and never relays it). A byzantine LEAD is still trusted, exactly
/// as the star already trusts it to relay verbatim; removing that trust needs
/// per-hop authentication the Phoenix rendezvous worker would carry, which is the
/// real cutover this flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshOrigin {
    /// A connection authenticated to this fleet slot delivered the frame.
    Peer(HostSlot),
    /// This host's OWN local observation — a socket close it saw itself — injected
    /// by the bridge, authentic by construction and never received from a peer.
    /// The one legitimate source of a `tick == 0` self-reported [`HostLossFrame`].
    LocalObservation,
    /// No transport authentication is available: a native fixture, or a caller that
    /// used [`MeshInbox::push`]. The frame's declared `from` is trusted, preserving
    /// pre-#1120 behaviour so every existing determinism guard is unaffected.
    Unauthenticated,
}

impl MeshOrigin {
    /// Whether a frame whose declared origin is `from` must be REFUSED at ingress,
    /// given the star `lead` and whether a slot is a known roster peer.
    ///
    /// See the type docs for the determinism argument and the star `== lead`
    /// accommodation. [`Self::Unauthenticated`] and [`Self::LocalObservation`] never
    /// refuse here (the former trusts `from`; the latter is a self-observation
    /// judged by its own guard in [`apply_mesh_inbox`]).
    pub fn refuses(
        &self,
        from: HostSlot,
        lead: HostSlot,
        is_roster_peer: impl Fn(HostSlot) -> bool,
    ) -> bool {
        match self {
            MeshOrigin::Unauthenticated | MeshOrigin::LocalObservation => false,
            MeshOrigin::Peer(authenticated) => {
                if !is_roster_peer(*authenticated) || !is_roster_peer(from) {
                    return true;
                }
                !(from == *authenticated || *authenticated == lead)
            }
        }
    }
}

/// Owner-local sequence used when minting ordered slot claims. The browser
/// queues slot ordinals; only the scheduled owner mints the tie-break value.
#[derive(Resource, Default, Debug)]
pub struct SlotClaimSequence(u64);

impl SlotClaimSequence {
    pub fn next_claim(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    pub fn reset(&mut self) {
        self.0 = 0;
    }
}

/// Frames a transport has received and this host has not applied yet, each with
/// the slot the delivering connection authenticated it to (issue #1120).
#[derive(Resource, Default, Debug)]
pub struct MeshInbox {
    frames: Vec<(MeshFrame, MeshOrigin)>,
}

impl MeshInbox {
    /// Hand one received frame to the simulation with no transport authentication.
    ///
    /// Kept for native fixtures and every pre-#1120 caller: the frame's declared
    /// `from` is trusted ([`MeshOrigin::Unauthenticated`]), so the auth check is a
    /// no-op and existing behaviour is unchanged.
    pub fn push(&mut self, frame: MeshFrame) {
        self.frames.push((frame, MeshOrigin::Unauthenticated));
    }

    /// Hand one received frame over with the origin the transport authenticated it
    /// to (issue #1120) — the bridge's path, and the sender-auth tests'.
    pub fn push_from(&mut self, frame: MeshFrame, origin: MeshOrigin) {
        self.frames.push((frame, origin));
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    fn take(&mut self) -> Vec<(MeshFrame, MeshOrigin)> {
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
    staged_start_grant: Option<crate::lobby::start_policy::StartGrant>,
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

    /// Attach a coordinated start to the next local tick frame.
    ///
    /// Only the lobby's owner-admission path calls this. Refusing a second,
    /// different grant prevents arrival order from deciding which proposal is
    /// made authoritative inside the frame.
    pub fn stage_start_grant(&mut self, grant: crate::lobby::start_policy::StartGrant) -> bool {
        match self.staged_start_grant.as_ref() {
            None => {
                self.staged_start_grant = Some(grant);
                true
            }
            Some(staged) => staged == &grant,
        }
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

    /// Forget every sampled checkpoint at or before `tick`, across this host's own
    /// ledger and every peer's, and clear the recorded disagreements at or before
    /// it (issue #1118).
    ///
    /// Called on every host when a recovery resolves at the boundary `tick`: the
    /// divergent samples the recovery just healed are dropped so the same stale
    /// split cannot re-trigger recovery, while later samples — which now agree —
    /// are kept. A disagreement past the boundary (there should be none) is
    /// retained rather than hidden.
    pub fn forget_through(&mut self, tick: u64) {
        self.local.forget_through(tick);
        for ledger in self.peers.values_mut() {
            ledger.forget_through(tick);
        }
        self.disagreements.retain(|d| d.tick > tick);
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
    // The #894 digest-boundary declarations (issue #1220's registry). Transport
    // and diagnostic types remain explicit exclusions; the typed GM journal is
    // the deliberate exception because it is durable simulation input,
    // captured and folded in full. See `authoritative::StateClass` for the
    // classes and `tests/authoritative_state_enumeration.rs` for the census that
    // enforces them.
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
            // The full canonical journal, including future grants and their
            // attribution/idempotency keys, is captured for snapshot/replay.
            // `state_digest` folds only the durable applied prefix: receipt time
            // for an owner commit at a future/exact-next boundary is not current
            // state.
            .declare_state::<crate::gm_action::GmActionJournal>(
                StateClass::Folded,
                "gm-action-state",
            )
            // The shared reducer registers this optional private transport
            // capability even on hosts that have no native surface installed.
            .declare_state::<crate::gm_action::NativeGmAuthority>(
                StateClass::Timer,
                "native-local-gm-workspace",
            )
            .declare_state::<crate::gm_puppet::StationPuppets>(
                StateClass::Folded,
                "gm-action-state",
            )
            .declare_state::<crate::gm_puppet::PreviousStationPuppetTargets>(
                StateClass::Derived,
                "gm-action-state",
            )
            .declare_state::<crate::gm_puppet::capability::NpcStationConfig>(
                StateClass::Derived,
                "gm-action-state",
            )
            .declare_state::<crate::gm_puppet::PendingGmStationCommands>(
                StateClass::Folded,
                "gm-action-state",
            )
            .declare_state::<crate::gm_puppet::StationPuppetActivity>(
                StateClass::Presentation,
                "gm-action-state",
            )
            // Directed world effects resolved at a canonical apply boundary and
            // not yet landed by the damage phase (issue #1310). `Folded` for
            // `PendingGmStationCommands`' reason: the grant that authorised it
            // is already applied, so a peer that lost the arm across a snapshot
            // would keep a hull nobody else kept.
            .declare_state::<crate::gm_effect::PendingGmDirectEffects>(
                StateClass::Folded,
                "gm-action-state",
            )
            .declare_state::<crate::gm_action::GmActionLog>(StateClass::Derived, "gm-action-state")
            .declare_state::<crate::gm_action::LocalGmActionRefusals>(
                StateClass::Presentation,
                "gm-action-state",
            )
            .declare_state::<crate::gm_action::LastGmSessionProjection>(
                StateClass::Presentation,
                "gm-action-state",
            )
            // The GM mission panel's last published page-local projection
            // (issue #1301). `Presentation` for its Session twin's exact
            // reason: it is a de-duplication cache for a Host Channel push,
            // derived entirely from the authored trigger table and the GM
            // action log, both of which are already classified.
            .declare_state::<crate::gm_event::LastGmMissionProjection>(
                StateClass::Presentation,
                "gm-action-state",
            )
            // The GM placement panel's last published page-local projection
            // (issue #1305). `Presentation` for its Session and Mission twins'
            // exact reason: it is a de-duplication cache for a Host Channel
            // push, derived entirely from the authored palette and the GM
            // action log, both of which are already classified.
            .declare_state::<crate::gm_comms::LastGmCommsProjection>(
                StateClass::Presentation,
                "gm-action-state",
            )
            .declare_state::<crate::gm_spawn::LastGmSpawnProjection>(
                StateClass::Presentation,
                "gm-action-state",
            )
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
            .declare_state::<host_loss::PendingHostLoss>(StateClass::Timer, "fleet-lockstep-state")
            .declare_state::<SlotClaimSequence>(StateClass::Timer, "fleet-lockstep-state");
    }
    app.init_resource::<FleetRoster>()
        .init_resource::<MeshInbox>()
        .init_resource::<SlotClaimSequence>()
        .init_resource::<MeshOutbox>()
        .init_resource::<MeshDiagnostics>()
        .init_resource::<MeshAgreement>()
        .init_resource::<host_loss::PendingHostLoss>()
        .init_resource::<crate::gm_action::SimulationPaused>()
        .init_resource::<crate::gm_action::GmActionJournal>()
        .init_resource::<crate::gm_puppet::StationPuppets>()
        .init_resource::<crate::gm_puppet::PendingGmStationCommands>()
        .init_resource::<crate::gm_effect::PendingGmDirectEffects>()
        .init_resource::<crate::gm_puppet::StationPuppetActivity>()
        .init_resource::<crate::gm_action::GmActionLog>()
        .init_resource::<crate::gm_action::LocalGmActionRefusals>()
        .init_resource::<crate::gm_action::LastGmSessionProjection>()
        .init_resource::<crate::gm_event::LastGmMissionProjection>()
        .init_resource::<crate::gm_spawn::LastGmSpawnProjection>()
        .init_resource::<crate::gm_comms::LastGmCommsProjection>()
        .add_message::<crate::console_bridge::GmCommsChanged>()
        .add_message::<crate::console_bridge::GmSessionChanged>()
        .add_message::<crate::console_bridge::GmMissionChanged>()
        .add_message::<crate::console_bridge::GmSpawnChanged>()
        .add_systems(
            PreUpdate,
            // `drive_recovery` (issue #1118) sits between applying the inbox and
            // the barrier: it reads the freshest peer digests and watermarks, and
            // publishes the boundary hold `gate_lockstep_ticks` then honours this
            // same frame. It is a clearly-separable addition beside the barrier
            // rather than a change to it.
            (
                apply_mesh_inbox,
                crate::gm_contact::prune,
                crate::gm_action::apply_due_actions,
                recovery::drive_recovery,
                slot_recovery::drive_slot_recovery,
                gate_lockstep_ticks,
            )
                .chain()
                .in_set(MeshSet),
        )
        .add_systems(
            FixedLast,
            (crate::gm_contact::prune, seal_tick_frame)
                .chain()
                .before(crate::sim_tick::advance_sim_tick)
                .before(sample_and_publish_digest),
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
        // Bevy normally spends every accumulated fixed step before returning to
        // PostUpdate. A mesh cannot do that safely: the first step seals its
        // TickFrame, but the browser transport does not flush that frame until
        // PostUpdate. Running a second step first would let this host cross a
        // barrier (including a scheduled start) its peers have not had any
        // opportunity to observe. After one complete step has committed, discard
        // only the residual accumulator so the bearing frame leaves before the
        // next step. No tick is partially skipped or rolled back.
        .add_systems(
            FixedLast,
            (
                discard_gm_boundary_overstep,
                discard_multi_participant_overstep,
            )
                .chain()
                .after(crate::sim_tick::advance_sim_tick),
        )
        .add_systems(
            PostUpdate,
            (
                crate::gm_action::publish_session_projection,
                crate::gm_event::publish_mission_projection,
                crate::gm_spawn::publish_spawn_projection,
                crate::gm_comms::publish_comms_projection,
            ),
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
                .in_set(crate::sim_sets::FixedStep::ApplyHostLossBackfill)
                .in_set(crate::sim_sets::SimSet::Input)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::ship_plugin::handle_station_rating_change)
                .before(crate::ship_plugin::resolve_human_seeking_hosts),
        );
    // The portable-record transfer (issue #1117): the receiver resource and the
    // frame-driven restore that commits a fully-arrived record. Kept in its own
    // sibling so #1119's parallel work on this module does not collide with it.
    snapshot_relay::register_snapshot_relay(app);
    crate::gm_join::register_join_driver(app);
    // Divergence recovery (issue #1118): the recovery resources and the diagnostic
    // log. The `drive_recovery` system itself is wired into the mesh chain above.
    recovery::register_recovery(app);
    // Slot recovery (issue #1120): the claim resolver, the recovery bookkeeping and
    // its hold. `drive_slot_recovery` is wired into the mesh chain above, beside
    // `drive_recovery` and before the barrier.
    slot_recovery::register_slot_recovery(app);
}

/// Join a fleet: adopt the slot, the peers and the agreed delay.
///
/// The **one** writer of [`CommandDelay`], which is what makes AGENTS.md rule
/// 7's amendment checkable rather than a hope — a delay cannot appear by
/// accident anywhere else in the plumbing.
///
/// `delay` comes from the world's authored `[global] command_delay_ticks`; see
/// [`crate::lockstep::authored_delay`].
pub fn join_fleet(world: &mut World, roster: FleetRoster, delay: u64) -> bool {
    let local = roster.local();
    // The barrier follows simulations, not ships. A stationless GM host still
    // advances a Rust world and must publish a watermark before any peer may
    // cross a scheduled control boundary.
    let peers = roster.participants();
    let alone = roster.is_solo();
    let activation_tick = if alone { 0 } else { FLEET_ACTIVATION_TICK };
    let Some(session) = LockstepSession::new_at(local, peers, delay, activation_tick) else {
        return false;
    };
    if let Some(installed) = world.get_resource::<FleetLockstep>() {
        // A frozen reconnect can replay the same private roster after activation.
        // Treat that as an acknowledgement, not a second activation that resets
        // tick/RNG. A different topology or delay is a new fleet generation and
        // cannot be grafted onto the running wait-set.
        return installed.local() == local
            && installed.delay() == delay
            && world
                .get_resource::<FleetRoster>()
                .is_some_and(|current| current == &roster);
    }
    if !crew::roster_crew_matches_hulls(world, &roster) {
        return false;
    }
    crate::gm_action::reset(world);
    if !alone {
        // Browser pages can finish booting their lobby at different frame rates,
        // so their pre-adoption SimTicks are not a shared clock. Installing a
        // wait-set at those unequal values would either deadlock immediately
        // (peers start ready only through `delay`) or preserve divergent start
        // ticks. Fleet adoption therefore has one explicit activation boundary:
        // while still in Lobby, every technical participant rebases to the
        // reserved fleet epoch before the barrier exists. A faster adopter may
        // run only to the delay
        // frontier and then waits for the later adopter's first frames.
        let lobby = world
            .get_resource::<State<crate::core::messages::GamePhase>>()
            .is_none_or(|state| state.get() == &crate::core::messages::GamePhase::Lobby);
        let no_pending_start = world
            .get_resource::<NextState<crate::core::messages::GamePhase>>()
            .is_none_or(|next| {
                !matches!(
                    next,
                    NextState::Pending(phase)
                        if phase != &crate::core::messages::GamePhase::Lobby
                )
            });
        // A returned-to-lobby or resume-staged App contains authoritative state
        // from another run. Rebasing only its clock/RNG would not canonicalise
        // that world, so participant adoption is startup-only and fails closed.
        let restore_staged = crate::startup_restore::is_pending(world);
        let fresh_lobby = !world.contains_resource::<crate::server_app::GameStartEntityUuids>()
            && !world.contains_resource::<crate::server_app::ResumeGameStartEntityUuids>()
            && !crate::save_slots_lifecycle::startup_restore_pending(world)
            && !restore_staged;
        let authored_seed = world
            .get_resource::<crate::world::config::WorldConfig>()
            .and_then(|config| config.global.seed);
        let Some(timestep) = world
            .get_resource::<Time<Fixed>>()
            .map(|fixed| fixed.timestep())
        else {
            return false;
        };
        let elapsed_nanos = timestep
            .as_nanos()
            .checked_mul(u128::from(FLEET_ACTIVATION_TICK))
            .and_then(|nanos| u64::try_from(nanos).ok());
        if !lobby
            || !no_pending_start
            || !fresh_lobby
            || authored_seed.is_none()
            || elapsed_nanos.is_none()
        {
            return false;
        }

        // Validate every fallible input before mutating anything: refusal must
        // leave a standalone lobby exactly as it was.
        let elapsed_nanos = elapsed_nanos.expect("checked above");
        if let Some(mut tick) = world.get_resource_mut::<crate::sim_tick::SimTick>() {
            tick.0 = FLEET_ACTIVATION_TICK;
        } else {
            world.insert_resource(crate::sim_tick::SimTick(FLEET_ACTIVATION_TICK));
        }
        // Replace the whole fixed clock: elapsed is exactly tick*timestep and
        // overstep is exactly zero on every participant. Keeping the pre-
        // adoption clock would make context-sensitive `Time` reads and mission
        // anchors differ even though `SimTick` agreed.
        let mut canonical = Time::<Fixed>::from_duration(timestep);
        canonical.advance_to(std::time::Duration::from_nanos(elapsed_nanos));
        world.insert_resource(canonical);
        // Browser registration starts from OS entropy. That is correct for an
        // independent host but immediately divergent in a participant mesh,
        // whose digest folds every stream position. Fleet activation therefore
        // requires the world's canonical authored seed and replaces the random
        // resource before any participant frame can be sealed.
        crate::sim_rng::install(
            world,
            crate::sim_rng::SimRng::new(
                authored_seed.expect("checked above"),
                crate::sim_rng::SeedSource::World,
            ),
        );
        if let Some(mint) = world.get_resource::<crate::world_id::WorldIdMint>() {
            mint.begin_tick(FLEET_ACTIVATION_TICK);
        }
        // Host-only clock and damage cheats are not replicated inputs. A fleet
        // adopts from their neutral values, and their drains refuse later raw
        // mutations while the wait-set is installed.
        if let Some(mut paused) = world.get_resource_mut::<crate::debug_overlay::SimulationPaused>()
        {
            paused.0 = false;
        }
        if let Some(mut virtual_time) = world.get_resource_mut::<Time<bevy::time::Virtual>>() {
            virtual_time.unpause();
        }
        if let Some(mut instagib) = world.get_resource_mut::<crate::server_app::Instagib>() {
            instagib.0 = false;
        }
    }
    world.insert_resource(roster);
    world.insert_resource(FleetLockstep(session));
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
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FleetLeaveError {
    NotFreshLobby,
}

/// Tear down one accepted fleet generation without leaving its wait-set behind.
///
/// This is deliberately a Lobby-only lifecycle edge. Once GameStart state has
/// existed, removing only transport/session resources would turn one continuing
/// authoritative world into an unrelated solo simulation. A fresh Lobby may
/// leave and reopen freely: all mesh queues, agreement state, delayed commands,
/// host-loss bookkeeping and coordinated-start state return to their standalone
/// baselines, and a barrier-paused virtual clock is released.
pub fn leave_fleet(world: &mut World) -> Result<(), FleetLeaveError> {
    let lobby = world
        .get_resource::<State<crate::core::messages::GamePhase>>()
        .is_none_or(|state| state.get() == &crate::core::messages::GamePhase::Lobby);
    let no_pending_start = world
        .get_resource::<NextState<crate::core::messages::GamePhase>>()
        .is_none_or(|next| {
            !matches!(
                next,
                NextState::Pending(phase) if phase != &crate::core::messages::GamePhase::Lobby
            )
        });
    let fresh = !world.contains_resource::<crate::server_app::GameStartEntityUuids>()
        && !world.contains_resource::<crate::server_app::ResumeGameStartEntityUuids>()
        && !crate::save_slots_lifecycle::startup_restore_pending(world);
    let restore_staged = crate::startup_restore::is_pending(world);
    if !lobby || !no_pending_start || !fresh || restore_staged {
        return Err(FleetLeaveError::NotFreshLobby);
    }

    world.remove_resource::<FleetLockstep>();
    world.insert_resource(FleetRoster::default());
    world.insert_resource(MeshInbox::default());
    world.insert_resource(MeshOutbox::default());
    world.insert_resource(MeshAgreement::new(0));
    world.insert_resource(MeshDiagnostics::default());
    world.insert_resource(host_loss::PendingHostLoss::default());
    world.insert_resource(recovery::RecoveryState::default());
    world.insert_resource(recovery::RecoveryHold::default());
    world.insert_resource(recovery::RecoveryLog::default());
    world.insert_resource(slot_recovery::PendingSlotClaims::default());
    world.insert_resource(slot_recovery::SlotRecoveryState::default());
    world.insert_resource(slot_recovery::SlotRecoveryHold::default());
    world.insert_resource(slot_recovery::SlotRecoveryLog::default());
    world.insert_resource(snapshot_relay::MeshSnapshotReceiver::default());
    world.insert_resource(snapshot_relay::MeshRestoreArm::default());
    world.insert_resource(PendingCommands::default());
    world.insert_resource(CommandDelay(0));
    if let Some(mut virtual_time) = world.get_resource_mut::<Time<bevy::time::Virtual>>() {
        virtual_time.unpause();
    }
    if let Some(mut managed) = world.get_resource_mut::<crate::lobby::server::FleetManagedLobby>() {
        managed.set_enabled(false);
    }
    if let Some(mut grants) = world.get_resource_mut::<crate::lobby::server::PendingStartGrants>() {
        grants.clear();
    }
    if let Some(mut tracker) = world.get_resource_mut::<crate::lobby::server::StartGrantTracker>() {
        tracker.reset();
    }
    if let Some(mut results) = world.get_resource_mut::<crate::lobby::server::StartGrantResults>() {
        results.clear();
    }
    Ok(())
}

/// Bootstrap a saved fleet as one peer's new independent local session.
///
/// Save-slot resume is startup-only, but its boot identity may describe a real
/// multi-host fleet. Reusing [`join_fleet`] here would recreate the old wait set
/// without recreating its transports: every remote watermark would remain at
/// the opening delay and the new session would stop forever a few ticks after
/// restore. Instead, preserve the authored ship topology and this peer's local
/// slot, release the old crew, and deliberately install no [`FleetLockstep`].
/// The local ship follows the new App's live Sessions; every other saved fleet
/// ship begins on AI backfill.
pub fn start_saved_fleet_standalone(world: &mut World, roster: FleetRoster) {
    let roster = roster.into_uncrewed();
    let local = roster.local();
    world.insert_resource(roster);
    world.remove_resource::<FleetLockstep>();
    world.insert_resource(MeshAgreement::new(0));
    world.insert_resource(CommandDelay(0));
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

#[derive(bevy::ecs::system::SystemParam)]
pub struct StartGrantAdmission<'w, 's> {
    phase: Option<Res<'w, State<crate::core::messages::GamePhase>>>,
    next_phase: Option<Res<'w, NextState<crate::core::messages::GamePhase>>>,
    managed: Option<Res<'w, crate::lobby::server::FleetManagedLobby>>,
    gm_journal: ResMut<'w, crate::gm_action::GmActionJournal>,
    gm_paused: Res<'w, crate::gm_action::SimulationPaused>,
    gm_join_hold: Option<Res<'w, crate::gm_join::GmJoinPauseHold>>,
    gm_results: ResMut<'w, crate::gm_action::LocalGmActionRefusals>,
    gm_puppets: Res<'w, crate::gm_puppet::StationPuppets>,
    gm_station_ships: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static crate::ship_plugin::ShipConfigComponent,
            &'static crate::ship_plugin::ActiveStationRatings,
        ),
        With<crate::server_app::Ship>,
    >,
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
    mut pending_claims: ResMut<slot_recovery::PendingSlotClaims>,
    mut agreement: ResMut<MeshAgreement>,
    mut join_lane: crate::gm_join::GmJoinMeshLane,
    mut outbox: ResMut<MeshOutbox>,
    roster: Option<Res<FleetRoster>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    mut pending_starts: Option<ResMut<crate::lobby::PendingStartGrants>>,
    mut start_tracker: Option<ResMut<crate::lobby::server::StartGrantTracker>>,
    mut start_results: Option<ResMut<crate::lobby::server::StartGrantResults>>,
    mut start_admission: StartGrantAdmission,
    fleet_ships: Query<(&FleetSlotOf, &crate::entities::spawner::EntityUuid)>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    if inbox.is_empty() {
        return;
    }
    let frames = inbox.take();
    // The mesh-boundary sender authentication (issue #1120). `lead` is the star
    // centre (the owner, lowest slot); `is_member` recognises any roster slot,
    // departed or recovering ones included, since those are still fleet members
    // whose frames are legitimate even when the barrier no longer waits for them.
    // Absent roster ⇒ no fleet: nothing to authenticate against, so trust the
    // frame exactly as the pre-#1120 path did (a bare fixture).
    // A joining GM candidate intentionally has no authoritative FleetRoster
    // before digest proof. Its private bootstrap topology may authenticate the
    // owner's relay connection, but it is never used below as admission or as a
    // simulation wait-set.
    let authentication_roster = roster.as_deref().cloned().or_else(|| {
        join_lane
            .bootstrap
            .as_deref()
            .map(crate::gm_join::GmJoinBootstrap::topology)
            .cloned()
    });
    let lead = authentication_roster
        .as_ref()
        .map_or(HostSlot::SOLO, FleetRoster::lead);
    let is_member = |slot: HostSlot| {
        authentication_roster
            .as_ref()
            .is_none_or(|r| r.is_member(slot))
    };
    let private_candidate = session.is_none() && roster.is_none() && join_lane.bootstrap.is_some();
    // Snapshot chunks (issue #1117) are handled whether or not this host is a
    // lockstep participant: RECEIVING the record is how a host becomes one (a
    // join, a #1120 slot recovery), so a chunk must not be dropped by the
    // fleet-session gate below. Peeled off first; the buffer they accumulate is
    // bounded, and nothing is committed until the whole record has arrived, gated
    // and restored. `snapshot_relay::drain_mesh_restore` does the committing.
    let mut sim_frames = Vec::with_capacity(frames.len());
    for (frame, origin) in frames {
        // Reject a frame whose declared origin disagrees with the connection that
        // delivered it, or that names a slot no roster peer owns (issue #1120).
        // Identical on every honest host, so it cannot diverge the fold — see
        // `MeshOrigin`'s docs. A `LocalObservation` and an `Unauthenticated` push
        // are trusted here; the former is judged by the self-observation guard in
        // the HostLoss arm below.
        let proven_candidate_report = match &frame {
            MeshFrame::GmJoin(crate::gm_join::GmJoinFrame::Restored { from, .. })
            | MeshFrame::GmJoin(crate::gm_join::GmJoinFrame::RestoreBoundary { from, .. })
            | MeshFrame::GmJoin(crate::gm_join::GmJoinFrame::Refused { from, .. }) => {
                join_lane.runtime.active_candidate() == Some(*from)
            }
            _ => false,
        };
        if origin.refuses(frame.from(), lead, is_member) && !proven_candidate_report {
            crate::pwarn!(
                log,
                LogCat::Admit,
                "host-mesh: dropping a {} frame whose declared origin {} is not \
                 authenticated by the delivering connection ({origin:?})",
                frame.type_name(),
                frame.from().slot_id(),
            );
            continue;
        }
        match frame {
            MeshFrame::Snapshot(chunk) => {
                snapshot_relay::receive_chunk(&mut join_lane.snapshot_rx, &chunk, &log);
            }
            MeshFrame::GmJoin(frame) => join_lane.inbox.push(frame),
            MeshFrame::HostLoss(loss) if private_candidate => {
                let topology = authentication_roster
                    .as_ref()
                    .expect("a private candidate has bootstrap topology");
                if loss.tick == 0 && matches!(origin, MeshOrigin::Peer(_)) {
                    crate::pwarn!(
                        log,
                        LogCat::Admit,
                        "host-mesh: dropping a forged self-observed loss for {} on a private GM candidate",
                        loss.lost.slot_id(),
                    );
                    continue;
                }
                if loss.lost == topology.local() || !topology.is_member(loss.lost) {
                    crate::pwarn!(
                        log,
                        LogCat::Admit,
                        "host-mesh: dropping a pre-admission loss for unknown/private slot {}",
                        loss.lost.slot_id(),
                    );
                    continue;
                }
                // Retain the already-agreed carried tick in candidate-private,
                // non-authoritative state. Commit is the sole transition into
                // PendingHostLoss and the new wait-set, so this post-capture
                // topology change cannot contaminate snapshot digest proof.
                join_lane.pending_host_loss.observe(loss.lost, loss.tick);
            }
            other => sim_frames.push((other, origin)),
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
    for (frame, origin) in sim_frames {
        match frame {
            MeshFrame::Tick(tick) => {
                if tick.from == session.local() {
                    continue;
                }
                if let Some(grant) = tick.start_grant.as_ref() {
                    let expected_owner = roster.as_deref().map_or(lead, FleetRoster::owner);
                    let now = sim_tick.as_deref().map_or(0, |tick| tick.0);
                    let reason =
                        if tick.from != expected_owner {
                            Some(crate::lobby::start_policy::StartGrantReason::UnauthorizedGrant)
                        } else if grant.validate().is_err() {
                            Some(crate::lobby::start_policy::StartGrantReason::InvalidGrant)
                        } else if tick.ready_through != session.ready_through(tick.tick)
                            || grant.apply_tick <= tick.ready_through
                        {
                            Some(crate::lobby::start_policy::StartGrantReason::UnsafeApplyTick)
                        } else if now > grant.apply_tick {
                            Some(crate::lobby::start_policy::StartGrantReason::MissedApplyTick)
                        } else if start_admission
                            .managed
                            .as_deref()
                            .is_none_or(|managed| !managed.enabled)
                        {
                            Some(crate::lobby::start_policy::StartGrantReason::FleetNotManaged)
                        } else if start_admission.phase.as_deref().is_none_or(|phase| {
                            phase.get() != &crate::core::messages::GamePhase::Lobby
                        }) {
                            Some(crate::lobby::start_policy::StartGrantReason::AlreadyStarted)
                        } else if start_admission
                            .managed
                            .as_deref()
                            .is_none_or(|managed| !managed.validation_passed)
                        {
                            Some(crate::lobby::start_policy::StartGrantReason::ValidationFailed)
                        } else if start_tracker
                            .as_deref_mut()
                            .is_none_or(|tracker| tracker.adopt_canonical(grant).is_err())
                        {
                            Some(crate::lobby::start_policy::StartGrantReason::ConflictingGrant)
                        } else {
                            None
                        };

                    if let Some(reason) = reason {
                        if let Some(tracker) = start_tracker.as_deref_mut() {
                            // An invalid authoritative boundary is not recoverable
                            // by observing a later watermark: that would allow one
                            // peer to run past a decision another accepted. Keep the
                            // barrier closed until this fleet generation is torn
                            // down explicitly.
                            tracker.fail_closed();
                        }
                        if let Some(results) = start_results.as_deref_mut() {
                            results.push(crate::lobby::server::start_result(
                                grant,
                                crate::lobby::start_policy::StartGrantStatus::Refused,
                                Some(reason),
                                now,
                            ));
                        }
                    } else if let Some(pending) = pending_starts.as_deref_mut() {
                        pending.adopt_canonical(grant.clone());
                    }
                }
                // `tick.from` is now authenticated to the delivering connection at
                // ingress above (issue #1120 closed the #1118 `TODO`): a frame
                // whose declared origin disagrees with its connection's slot, or
                // whose origin is not a roster peer, never reaches this loop. The
                // ship-ownership check below is the second, orthogonal authority a
                // receiver enforces — a slot may speak only for the hull it flies.
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
                        feedback_correlation: None,
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
                // The #1119 self-observation guard, carried into #1120. A `tick ==
                // 0` report is a SELF-observation — "my own transport saw this
                // socket close" — and the tick is then derived from THIS host's own
                // watermark. That derivation is only sound for a loss this host
                // genuinely observed locally; a peer that forged a tick-0 report
                // over the mesh would make this host derive a flip tick from a
                // watermark other survivors do not share, the exact skew-derive
                // divergence #1119 fought. So a tick-0 report is honoured only from
                // a local observation (or an unauthenticated fixture); one that
                // arrived over an authenticated peer connection is dropped.
                if hl.tick == 0 && matches!(origin, MeshOrigin::Peer(_)) {
                    crate::pwarn!(
                        log,
                        LogCat::Admit,
                        "host-mesh: dropping a forged self-observed loss for {} — a \
                         tick-0 report may only originate from a local socket close, \
                         never over a peer connection",
                        lost.slot_id(),
                    );
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
                // Sender authentication (issue #1120) now closes two of the three
                // gaps this arm once carried a `TODO` for. The `from` of a relayed
                // report is authenticated to the connection that delivered it, so a
                // survivor can no longer forge a report UNDER ANOTHER SURVIVOR'S
                // slot, and the tick-0 self-observation guard above rejects a forged
                // self-observation injected over a peer connection. What is NOT
                // closed, and is deliberately left, is reporter TRUTHFULNESS: a
                // survivor can still honestly send (under its own authenticated
                // slot) a loss report for a slot that is in fact still LIVE.
                // Honouring it stays CONVERGENT — every survivor that receives it
                // agrees the same carried tick, so the fold never diverges — and
                // rejecting it cannot be done deterministically here, because the
                // only ground-truth liveness signal is the transport connection this
                // host does not hold, and a bounded liveness check against a raw
                // watermark is exactly the skew-prone decision that diverged the
                // fold. That residue is a transport-trust question (a byzantine peer
                // asserting a false fact under its real identity), for the Phoenix
                // rendezvous cutover, not a forged-origin one.
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
            MeshFrame::SlotClaim(claim) => {
                // A replacement machine's granted claim on a disconnected fixed
                // slot (issue #1120). The owner (star centre) resolved any race and
                // stamped `claim_seq` in arrival order; every host records it, and
                // the deterministic winner for a slot is the lowest seq — so two
                // hosts that hear two claims agree the same winner from the shared
                // value, never from who-processed-first locally. `drive_slot_recovery`
                // reads this and opens the recovery.
                pending_claims.observe(claim.slot, claim.claim_seq, claim.tick);
            }
            MeshFrame::GmAction(action_frame) => {
                if roster.as_deref().is_some_and(|roster| {
                    crate::gm_action::validate_fleet_frame(&action_frame, roster).is_err()
                }) {
                    crate::pwarn!(
                        log,
                        LogCat::Admit,
                        "refused GM frame whose proposal/owner authority did not match the frozen roster",
                    );
                    continue;
                }
                let gm_run_active =
                    start_admission.phase.as_deref().is_none_or(|phase| {
                        phase.get() == &crate::core::messages::GamePhase::InProgress
                    }) && start_admission.next_phase.as_deref().is_none_or(|next| {
                        !matches!(
                            next,
                            NextState::Pending(phase)
                                if phase != &crate::core::messages::GamePhase::InProgress
                        )
                    });
                match action_frame {
                    crate::gm_action::GmActionFrame::Proposal(proposal) => {
                        // A run-exit reset is authoritative for this frame. Drop
                        // every GM lane variant symmetrically once that boundary
                        // is pending so a co-arriving Proposal cannot recreate a
                        // refusal projection or outbound decision after reset.
                        if !gm_run_active {
                            continue;
                        }
                        let bound = roster
                            .as_deref()
                            .and_then(|roster| roster.gm_operator(proposal.from));
                        if bound != Some(proposal.operator_id.as_str()) {
                            crate::pwarn!(
                                log,
                                LogCat::Admit,
                                "{} proposed GM action as {:?}; frozen binding is {:?} — dropped",
                                proposal.from.slot_id(),
                                proposal.operator_id,
                                bound,
                            );
                            continue;
                        }
                        let owner = roster.as_deref().map_or(lead, FleetRoster::owner);
                        // The star relays opaque proposals to every peer. Only the
                        // technical owner turns one into a canonical decision.
                        if session.local() != owner {
                            continue;
                        }
                        let now = sim_tick.as_deref().map_or(0, |tick| tick.0);
                        let found = proposal.action.ship_key().and_then(|ship| {
                            start_admission
                                .gm_station_ships
                                .iter()
                                .find(|(uuid, ..)| uuid.0 == ship.0)
                        });
                        let sequenced = crate::gm_puppet::validate_station_action(
                            &proposal.action,
                            &proposal.operator_id,
                            &start_admission.gm_puppets,
                            found.map(|(_, config, _)| &config.0),
                            found.map(|(_, _, ratings)| ratings),
                        )
                        .and_then(|()| {
                            crate::gm_action::sequence_owner_proposal(
                                &mut start_admission.gm_journal,
                                &proposal,
                                owner,
                                now,
                                session.ready_through(now),
                                start_admission.gm_paused.0,
                                start_admission
                                    .gm_join_hold
                                    .as_deref()
                                    .is_some_and(crate::gm_join::GmJoinPauseHold::active),
                            )
                        });
                        let decision = match sequenced {
                            Ok(grant) => crate::gm_action::GmActionFrame::Granted(grant),
                            Err(reason) => {
                                // Built by the shared constructor, so a remote
                                // proposal's refusal names the same identity —
                                // including the action's stable target — that a
                                // locally-submitted one does.
                                let refusal =
                                    crate::gm_action::refusal_for(owner, &proposal, now, reason);
                                start_admission.gm_results.push(refusal.logged());
                                crate::gm_action::GmActionFrame::Refused(refusal)
                            }
                        };
                        outbox.push(MeshFrame::GmAction(decision));
                    }
                    crate::gm_action::GmActionFrame::Granted(grant) => {
                        if !gm_run_active {
                            continue;
                        }
                        let owner = roster.as_deref().map_or(lead, FleetRoster::owner);
                        let bound = roster
                            .as_deref()
                            .and_then(|roster| roster.gm_operator(grant.from));
                        if grant.sequenced_by != owner || bound != Some(grant.operator_id.as_str())
                        {
                            crate::pwarn!(
                            log,
                            LogCat::Admit,
                            "refused GM grant sequenced by {} for {} as {:?}; owner/binding are {}/{:?}",
                            grant.sequenced_by.slot_id(),
                            grant.from.slot_id(),
                            grant.operator_id,
                            owner.slot_id(),
                            bound,
                        );
                            continue;
                        }
                        // The owner adopts the live pre-journal pause before it
                        // sequences the first typed grant. Every receiver must
                        // make the same one-time adoption before inserting that
                        // grant, otherwise a technical join hold (or a restored
                        // standalone pause) makes the owner's Resume `Applied`
                        // while peers derive `NoOp` from an empty false baseline.
                        // Capture/restore can therefore preserve the exact folded
                        // journal instead of smuggling the technical hold into it.
                        if let Err(reason) = crate::gm_action::insert_replicated_grant(
                            &mut start_admission.gm_journal,
                            start_admission.gm_paused.0,
                            grant,
                        ) {
                            crate::pwarn!(
                                log,
                                LogCat::Admit,
                                "refused replicated GM action: {reason:?}",
                            );
                        }
                    }
                    crate::gm_action::GmActionFrame::Refused(refusal) => {
                        if !gm_run_active {
                            continue;
                        }
                        let owner = roster.as_deref().map_or(lead, FleetRoster::owner);
                        let bound = roster
                            .as_deref()
                            .and_then(|roster| roster.gm_operator(refusal.requester));
                        if refusal.sequenced_by != owner
                            || bound != Some(refusal.operator_id.as_str())
                        {
                            crate::pwarn!(
                                log,
                                LogCat::Admit,
                                "refused unauthenticated GM refusal from {}",
                                refusal.sequenced_by.slot_id(),
                            );
                            continue;
                        }
                        start_admission.gm_results.push(refusal.logged());
                    }
                }
            }
            // Peeled off above, before the fleet-session gate, so it never reaches
            // this loop — but the match stays exhaustive rather than resting on
            // that being remembered.
            MeshFrame::Snapshot(_) => {}
            // The deterministic admission driver consumes this in the bounded
            // join lane; keep the main command/digest switch exhaustive while
            // that lane is registered beside the snapshot relay.
            MeshFrame::GmJoin(_) => {}
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
    model_rigs: Option<Res<crate::entities::model_markers::ModelRigReadiness>>,
    virtual_time: Option<ResMut<Time<Virtual>>>,
    mut diagnostics: ResMut<MeshDiagnostics>,
    recovery_hold: Option<Res<recovery::RecoveryHold>>,
    slot_recovery_hold: Option<Res<slot_recovery::SlotRecoveryHold>>,
    start_tracker: Option<Res<crate::lobby::server::StartGrantTracker>>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    let model_rig_hold = model_rigs.is_some_and(|rigs| rigs.blocks_simulation());
    // `Option` for the same reason `SimTick` is taken as one in admission: a
    // bare-`App` fixture with no `TimePlugin` would otherwise fail Bevy's
    // parameter validation and skip this system entirely, which is a silent
    // way to lose the barrier.
    let Some(mut virtual_time) = virtual_time else {
        return;
    };
    // The operator's own pause owns the clock while it is on; the barrier must
    // not un-pause it out from under them.
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let next_tick = sim_tick.map_or(0, |t| t.0);

    // Canonical marker geometry is required even outside a multi-peer fleet.
    // A solo/browser-GM app has no peer barrier, but it must still wait rather
    // than make sidecar delivery timing authoritative. This branch owns only
    // that hold; the operator-pause guard above remains stronger.
    let Some(session) = session.filter(|session| !session.is_alone()) else {
        if model_rig_hold {
            if !virtual_time.is_paused() {
                crate::pinfo!(
                    log,
                    LogCat::Assets,
                    "authoritative model-rig hold at tick {next_tick}: waiting for primary sidecar"
                );
                virtual_time.pause();
            }
        } else if virtual_time.is_paused() {
            crate::pinfo!(
                log,
                LogCat::Assets,
                "authoritative model-rig hold released at tick {next_tick}"
            );
            virtual_time.unpause();
        }
        return;
    };

    // A peer stall (this host is ahead of the fleet) and a recovery boundary hold
    // (issue #1118, this host must not run past the tick a divergence is being
    // healed at) are two reasons to withhold the same tick. Both pause the same
    // clock, so one system owns it: the barrier's own peer-stall decision is
    // exactly #1116's, and the recovery hold is an additional withhold reason read
    // from `RecoveryHold` beside it. The stall diagnostics track only the peer
    // stall, so a boundary hold with no peer behind it is not miscounted as one.
    let stall = session.stall_at(next_tick);
    match &stall {
        Some(stall) => diagnostics.stalled(stall.clone()),
        None => diagnostics.running(),
    }
    // A slot-recovery boundary hold (issue #1120, this host must not run past the
    // tick a disconnected slot is being recovered at) is a third withhold reason,
    // read from its own `SlotRecoveryHold` beside the peer-stall and the #1118 hold
    // for the same clearly-separable reason. All three pause the same clock.
    let held = recovery_hold
        .and_then(|hold| hold.withhold_beyond)
        .is_some_and(|boundary| next_tick > boundary)
        || slot_recovery_hold
            .and_then(|hold| hold.withhold_beyond)
            .is_some_and(|boundary| next_tick > boundary)
        || start_tracker.is_some_and(|tracker| tracker.is_failed_closed())
        || model_rig_hold;

    if stall.is_some() || held {
        if !virtual_time.is_paused() {
            match &stall {
                Some(stall) => crate::pwarn!(log, LogCat::Admit, "host-mesh stall: {stall}"),
                None => crate::pinfo!(
                    log,
                    LogCat::Admit,
                    "host-mesh recovery hold at tick {next_tick}: withholding until the \
                     divergence is healed"
                ),
            }
            virtual_time.pause();
        }
    } else if virtual_time.is_paused() {
        crate::pinfo!(log, LogCat::Admit, "host-mesh resumed at tick {next_tick}");
        virtual_time.unpause();
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
    slot_recovery: Option<Res<slot_recovery::SlotRecoveryState>>,
    mut outbox: ResMut<MeshOutbox>,
) {
    let Some(session) = session else {
        outbox.staged.clear();
        outbox.staged_start_grant = None;
        return;
    };
    if session.is_alone() {
        outbox.staged.clear();
        outbox.staged_start_grant = None;
        return;
    }
    // A replacement bootstrapping and awaiting the transfer (issue #1120) must not
    // seal a frame: its throwaway pre-restore world would declare a premature
    // watermark that walks the survivors past the boundary. Drop the staged
    // commands with it — they are pre-restore noise that must never cross.
    if slot_recovery.is_some_and(|s| s.suppresses_egress()) {
        outbox.staged.clear();
        outbox.staged_start_grant = None;
        return;
    }
    let tick = sim_tick.0;
    let commands = std::mem::take(&mut outbox.staged);
    let start_grant = outbox.staged_start_grant.take();
    outbox.push(MeshFrame::Tick(TickFrame {
        from: session.local(),
        tick,
        ready_through: session.ready_through(tick),
        commands,
        start_grant,
    }));
}

/// Whether this host is in a fleet with at least one peer — the only state in
/// which the digest exchange has anybody to compare with. A solo host (no
/// session, or a fleet of one) skips it, so it takes no per-step exclusive sync
/// point for an exchange that would fold nothing anyone receives.
fn fleet_is_running(session: Option<Res<FleetLockstep>>) -> bool {
    session.is_some_and(|s| !s.is_alone())
}

/// Limit a multi-participant mesh to one complete fixed step per render frame.
///
/// Bevy's fixed runner calls `Time<Fixed>::expend()` before each step and tests
/// the resource again before the next. Clearing the remaining overstep here,
/// in `FixedLast` after the completed step's frame/digest have been sealed and
/// its [`crate::sim_tick::SimTick`] advanced, makes that next test fail. This is
/// a transport boundary, not a simulation skip: the discarded duration is
/// frame-time catch-up debt and no authoritative tick was begun from it.
pub fn discard_multi_participant_overstep(
    session: Option<Res<FleetLockstep>>,
    mut fixed: Option<ResMut<Time<Fixed>>>,
) {
    if session.is_none_or(|session| session.is_alone()) {
        return;
    }
    let Some(fixed) = fixed.as_deref_mut() else {
        return;
    };
    discard_whole_fixed_overstep(fixed);
}

/// Stop a rendered frame exactly when it reaches the next unapplied GM
/// boundary, including in a one-participant GM fleet. `apply_due_actions` runs
/// in PreUpdate, so spending another catch-up step here would skip over a Pause
/// before the reducer had any frame in which to close the clock.
pub fn discard_gm_boundary_overstep(
    journal: Option<Res<crate::gm_action::GmActionJournal>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut fixed: Option<ResMut<Time<Fixed>>>,
) {
    let Some(journal) = journal else {
        return;
    };
    let Some(next) = journal.grants().get(journal.applied_grants()) else {
        return;
    };
    let now = tick.as_deref().map_or(0, |tick| tick.0);
    if next.apply_tick > now {
        return;
    }
    let Some(fixed) = fixed.as_deref_mut() else {
        return;
    };
    discard_whole_fixed_overstep(fixed);
}

fn discard_whole_fixed_overstep(fixed: &mut Time<Fixed>) {
    let remaining = fixed.overstep();
    let timestep = fixed.timestep();
    // Preserve the sub-step remainder: render interpolation legitimately reads
    // it as alpha. Only whole additional simulation steps are unsafe to carry
    // through this frame before the transport has flushed.
    let remainder_nanos = remaining.as_nanos() % timestep.as_nanos();
    let remainder = std::time::Duration::new(
        u64::try_from(remainder_nanos / 1_000_000_000).unwrap_or(u64::MAX),
        (remainder_nanos % 1_000_000_000) as u32,
    );
    let whole_step_debt = remaining - remainder;
    fixed.discard_overstep(whole_step_debt);
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
    // A replacement bootstrapping toward the transfer (issue #1120) neither samples
    // nor publishes: its throwaway pre-restore fold is not the fleet's state, and a
    // survivor that compared against it would report a divergence recovery must not
    // chase. Its canonical digests resume the moment it commits the restore.
    if world
        .get_resource::<slot_recovery::SlotRecoveryState>()
        .is_some_and(slot_recovery::SlotRecoveryState::suppresses_egress)
    {
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
    #[test]
    fn slot_claim_sequence_is_owned_by_the_world_and_resets_for_a_new_fleet() {
        let mut world = bevy::prelude::World::new();
        world.init_resource::<super::SlotClaimSequence>();
        assert_eq!(
            world
                .resource_mut::<super::SlotClaimSequence>()
                .next_claim(),
            1
        );
        assert_eq!(
            world
                .resource_mut::<super::SlotClaimSequence>()
                .next_claim(),
            2
        );
        world.resource_mut::<super::SlotClaimSequence>().reset();
        assert_eq!(
            world
                .resource_mut::<super::SlotClaimSequence>()
                .next_claim(),
            1
        );
        let mut other = bevy::prelude::World::new();
        other.init_resource::<super::SlotClaimSequence>();
        assert_eq!(
            other
                .resource_mut::<super::SlotClaimSequence>()
                .next_claim(),
            1
        );
    }
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

    /// The adoption boundary validates the frozen Helm rating against selected
    /// content. Use a world-local selected hull, avoiding the process-global
    /// native template cache in these unit fixtures.
    fn selected_hull_roster(world: &mut World) -> FleetRoster {
        let hull = crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "helm"
name = "Helm"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []
[[system]]
id = "helm"
kind = "helm_thrust"
station = "helm"
"#,
            &["helm_thrust"],
        )
        .expect("the selected hull supports the frozen Helm rating");
        world.insert_resource(crate::ship_plugin::PendingShipConfig(hull));
        let mut roster = roster();
        roster
            .ships
            .iter_mut()
            .find(|ship| ship.host == HostSlot(2))
            .unwrap()
            .ship_path = None;
        assert!(crew::roster_crew_matches_hulls(world, &roster));
        roster
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

    #[test]
    fn private_gm_bindings_are_bounded_unique_and_stationless() {
        let roster = FleetRoster::with_participants_and_gms(
            Vec::new(),
            vec![HostSlot(1), HostSlot(2)],
            vec![FleetGm {
                host: HostSlot(2),
                operator_id: "gm-1".into(),
            }],
            HostSlot(2),
            HostSlot(1),
        )
        .expect("GM-only participants are valid");
        assert_eq!(roster.gm_operator(HostSlot(2)), Some("gm-1"));
        assert!(roster.ships().is_empty());

        let ship_and_gm_same_slot = FleetRoster::with_participants_and_gms(
            vec![FleetShip::new(HostSlot(2))],
            vec![HostSlot(1), HostSlot(2)],
            vec![FleetGm {
                host: HostSlot(2),
                operator_id: "gm-1".into(),
            }],
            HostSlot(2),
            HostSlot(1),
        );
        assert!(ship_and_gm_same_slot.is_none());

        let duplicate_operator = FleetRoster::with_participants_and_gms(
            Vec::new(),
            vec![HostSlot(1), HostSlot(2)],
            vec![
                FleetGm {
                    host: HostSlot(1),
                    operator_id: "gm-1".into(),
                },
                FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-1".into(),
                },
            ],
            HostSlot(2),
            HostSlot(1),
        );
        assert!(duplicate_operator.is_none());
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

    /// A peer-local save preserves the ships it booted, but it is not a ticket
    /// back into the old mesh. Starting it must release the old wait set and
    /// crew assignments so the new App can advance by itself.
    #[test]
    fn a_saved_fleet_starts_as_an_uncrewed_standalone_session() {
        use crate::command_admission::log::PendingCommands;

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.init_resource::<PendingCommands>();
        register_lockstep(&mut app);
        let mut config = crate::world::config::WorldConfig::default();
        config.global.seed = Some(7);
        app.insert_resource(config);
        let saved = selected_hull_roster(app.world_mut());
        assert!(join_fleet(app.world_mut(), saved.clone(), 6));
        assert!(app.world().contains_resource::<FleetLockstep>());
        assert_eq!(app.world().resource::<CommandDelay>().0, 6);

        start_saved_fleet_standalone(app.world_mut(), saved);

        assert!(!app.world().contains_resource::<FleetLockstep>());
        assert_eq!(app.world().resource::<CommandDelay>().0, 0);
        let restored = app.world().resource::<FleetRoster>();
        assert_eq!(restored.len(), 2);
        assert!(restored.is_local(HostSlot(2)));
        assert!(restored.ships().iter().all(|ship| ship.crew.is_empty()));
    }

    #[test]
    fn fresh_lobby_leave_clears_the_wait_set_and_can_reopen() {
        use crate::command_admission::log::PendingCommands;

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.init_resource::<PendingCommands>();
        app.init_resource::<crate::lobby::server::FleetManagedLobby>();
        app.init_resource::<crate::lobby::server::PendingStartGrants>();
        app.init_resource::<crate::lobby::server::StartGrantTracker>();
        app.init_resource::<crate::lobby::server::StartGrantResults>();
        register_lockstep(&mut app);
        let mut config = crate::world::config::WorldConfig::default();
        config.global.seed = Some(11);
        app.insert_resource(config);
        let frozen = selected_hull_roster(app.world_mut());
        assert!(join_fleet(app.world_mut(), frozen.clone(), 6));
        app.world_mut()
            .resource_mut::<Time<bevy::time::Virtual>>()
            .pause();
        app.world_mut()
            .resource_mut::<crate::lobby::server::FleetManagedLobby>()
            .set_enabled(true);

        assert_eq!(leave_fleet(app.world_mut()), Ok(()));
        assert!(!app.world().contains_resource::<FleetLockstep>());
        assert!(app.world().resource::<FleetRoster>().is_solo());
        assert_eq!(app.world().resource::<CommandDelay>().0, 0);
        assert!(
            !app.world()
                .resource::<crate::lobby::server::FleetManagedLobby>()
                .enabled
        );
        assert!(!app
            .world()
            .resource::<Time<bevy::time::Virtual>>()
            .is_paused());

        assert!(
            join_fleet(app.world_mut(), frozen, 6),
            "the new generation must not inherit the old wait-set identity"
        );
        assert!(app.world().contains_resource::<FleetLockstep>());
    }

    #[test]
    fn unequal_gm_only_bootstraps_adopt_one_tick_clock_rng_and_mint_epoch() {
        use crate::command_admission::log::PendingCommands;
        use crate::sim_rng::{SeedSource, SimRng};
        use crate::world_id::{IdNamespace, WorldIdMint};

        let period = std::time::Duration::from_millis(20);
        let mut apps = Vec::new();
        let mut immediate_ids = Vec::new();
        for (local, bootstrap_tick, bootstrap_rng) in
            [(HostSlot(1), 17, 101), (HostSlot(2), 93, 202)]
        {
            let mut app = App::new();
            app.add_plugins(bevy::time::TimePlugin);
            app.init_resource::<PendingCommands>();
            register_lockstep(&mut app);
            let mut config = crate::world::config::WorldConfig::default();
            config.global.seed = Some(77);
            app.insert_resource(config);
            app.insert_resource(crate::sim_tick::SimTick(bootstrap_tick));
            app.insert_sim_rng(SimRng::new(bootstrap_rng, SeedSource::World));
            app.insert_world_id_mint(WorldIdMint::default());
            let immediate = app
                .world()
                .resource::<WorldIdMint>()
                .mint(IdNamespace::Entity);
            immediate_ids.push(immediate);
            let mut fixed = Time::<Fixed>::from_duration(period);
            fixed.advance_to(period * u32::try_from(bootstrap_tick).unwrap());
            app.insert_resource(fixed);

            let roster = FleetRoster::with_participants(
                Vec::new(),
                vec![HostSlot(1), HostSlot(2)],
                local,
                HostSlot(1),
            )
            .unwrap();
            assert!(join_fleet(app.world_mut(), roster, 6));
            apps.push(app);
        }

        for (index, app) in apps.iter_mut().enumerate() {
            assert_eq!(
                app.world().resource::<crate::sim_tick::SimTick>().0,
                FLEET_ACTIVATION_TICK
            );
            let fixed = app.world().resource::<Time<Fixed>>();
            assert_eq!(fixed.timestep(), period);
            assert_eq!(fixed.elapsed(), period);
            assert_eq!(fixed.overstep(), std::time::Duration::ZERO);
            let mint = app.world().resource::<WorldIdMint>();
            assert_eq!(mint.tick(), FLEET_ACTIVATION_TICK);
            let game_start = mint.mint(IdNamespace::Entity);
            assert_ne!(
                game_start, immediate_ids[index],
                "the tick-1 fleet epoch must not reuse a live immediate tick-0 id"
            );
            assert_eq!(
                app.world()
                    .resource::<FleetLockstep>()
                    .watermark_of(if index == 0 { HostSlot(2) } else { HostSlot(1) }),
                Some(FLEET_ACTIVATION_TICK + 6),
                "the first wait-set frontier is based at the shared activation epoch"
            );
        }
        assert_eq!(
            apps[0].world().resource::<SimRng>().state(),
            apps[1].world().resource::<SimRng>().state(),
            "browser-equivalent hosts discard their different entropy and use the authored seed"
        );
        assert_eq!(
            apps[0].world().resource::<WorldIdMint>().state(),
            apps[1].world().resource::<WorldIdMint>().state(),
            "the same post-activation mint state produces identical GameStart ids"
        );

        use bevy::ecs::system::RunSystemOnce;
        fn first_live_draw(
            rng: crate::sim_rng::LiveStream<
                { crate::sim_rng::SimStream::BeamCycleJitter as usize },
            >,
        ) -> u32 {
            crate::sim_rng::with_live_stream(rng.as_deref(), |stream| stream.next_u32())
        }
        for app in &mut apps {
            let reference = SimRng::new(77, SeedSource::World);
            assert_eq!(
                app.world_mut().run_system_once(first_live_draw).unwrap(),
                reference
                    .stream(crate::sim_rng::SimStream::BeamCycleJitter)
                    .next_u32(),
                "first live draw must use the adopted authored seed, not bootstrap handles"
            );
            let continued = app.world().resource::<SimRng>().state();
            let same_roster = app.world().resource::<FleetRoster>().clone();
            assert!(join_fleet(app.world_mut(), same_roster, 6));
            assert_eq!(
                app.world().resource::<SimRng>().state(),
                continued,
                "same-roster adoption must not reset any stream"
            );
            assert_eq!(
                app.world_mut().run_system_once(first_live_draw).unwrap(),
                reference
                    .stream(crate::sim_rng::SimStream::BeamCycleJitter)
                    .next_u32(),
                "repeated adoption continues the existing live cell"
            );
            assert_eq!(app.world().resource::<SimRng>().state(), reference.state());
        }
    }

    #[test]
    fn fleet_leave_refuses_at_the_start_boundary_without_clearing_live_state() {
        use crate::command_admission::log::PendingCommands;

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.init_resource::<PendingCommands>();
        register_lockstep(&mut app);
        let mut config = crate::world::config::WorldConfig::default();
        config.global.seed = Some(13);
        app.insert_resource(config);
        let frozen = selected_hull_roster(app.world_mut());
        assert!(join_fleet(app.world_mut(), frozen, 6));
        let retained = MeshFrame::Digest(DigestFrame {
            from: HostSlot(1),
            tick: 1,
            digest: 13,
        });
        app.world_mut()
            .resource_mut::<MeshInbox>()
            .push(retained.clone());
        app.world_mut().resource_mut::<MeshOutbox>().push(retained);
        app.insert_resource(NextState::Pending(
            crate::core::messages::GamePhase::InProgress,
        ));

        assert_eq!(
            leave_fleet(app.world_mut()),
            Err(FleetLeaveError::NotFreshLobby)
        );
        assert!(app.world().contains_resource::<FleetLockstep>());
        assert_eq!(app.world().resource::<CommandDelay>().0, 6);
        assert_eq!(app.world().resource::<MeshInbox>().len(), 1);
        assert_eq!(
            app.world().resource::<MeshOutbox>().pending_frames().len(),
            1
        );

        app.insert_resource(NextState::<crate::core::messages::GamePhase>::Unchanged);
        app.insert_resource(crate::server_app::GameStartEntityUuids::default());

        assert_eq!(
            leave_fleet(app.world_mut()),
            Err(FleetLeaveError::NotFreshLobby)
        );
        assert!(app.world().contains_resource::<FleetLockstep>());
        assert_eq!(app.world().resource::<MeshInbox>().len(), 1);
        assert_eq!(
            app.world().resource::<MeshOutbox>().pending_frames().len(),
            1
        );
    }

    #[test]
    fn only_a_standalone_rosters_local_ship_uses_live_sessions() {
        let restored = roster().into_uncrewed();

        assert!(uses_live_sessions(&restored, false, HostSlot(2)));
        assert!(
            !uses_live_sessions(&restored, false, HostSlot(1)),
            "a saved remote ship has no crew in this independent App"
        );
        assert!(
            !uses_live_sessions(&restored, true, HostSlot(2)),
            "active lockstep keeps even the local ship on frozen roster crew"
        );
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

    /// Canonical marker geometry is an authority prerequisite even when no
    /// fleet session exists. A rendererless GM therefore uses the same virtual
    /// clock hold as a rendered host, and releases it as soon as its live
    /// primary rig resolves.
    #[test]
    fn model_rig_hold_pauses_and_resumes_a_solo_clock() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<crate::entities::model_markers::ModelRigReadiness>()
            .init_resource::<MeshDiagnostics>()
            .add_systems(PreUpdate, gate_lockstep_ticks);

        app.world_mut()
            .resource_mut::<crate::entities::model_markers::ModelRigReadiness>()
            .set_live_blocked_for_test(true);
        app.update();
        assert!(
            app.world().resource::<Time<Virtual>>().is_paused(),
            "a solo authoritative profile must not tick without live marker geometry"
        );

        app.world_mut()
            .resource_mut::<crate::entities::model_markers::ModelRigReadiness>()
            .set_live_blocked_for_test(false);
        app.update();
        assert!(
            !app.world().resource::<Time<Virtual>>().is_paused(),
            "the narrow marker hold releases immediately once geometry resolves"
        );
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
            feedback_correlation: None,
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

    /// A participant mesh commits at most one fixed step before transport
    /// egress, while a solo simulation retains Bevy's ordinary catch-up loop.
    ///
    /// The oversized frame is the exact race a scheduled start exposed: without
    /// the FixedLast overstep cap, its first step could seal the owner's grant
    /// and four later steps could reach `apply_tick` before PostUpdate had any
    /// chance to send the bearing TickFrame. The fractional remainder is kept
    /// for render interpolation; only whole unstarted steps are discarded.
    #[test]
    fn a_multi_participant_frame_commits_one_step_before_egress() {
        use crate::sim_tick::{register_sim_tick, SimTick};

        let period = std::time::Duration::from_millis(10);

        let mut fleet = App::new();
        fleet.add_plugins(bevy::time::TimePlugin);
        register_sim_tick(&mut fleet);
        fleet.init_resource::<crate::command_admission::log::PendingCommands>();
        register_lockstep(&mut fleet);
        fleet.insert_resource(FleetLockstep(
            LockstepSession::new_at(
                HostSlot(1),
                vec![HostSlot(1), HostSlot(2)],
                6,
                FLEET_ACTIVATION_TICK,
            )
            .unwrap(),
        ));
        fleet
            .world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        // Prime Bevy's first frame, which intentionally carries zero delta.
        fleet.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        fleet.update();
        fleet.world_mut().resource_mut::<MeshOutbox>().drain();

        fleet.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            period * 5 + period / 2,
        ));
        fleet.update();
        assert_eq!(fleet.world().resource::<SimTick>().0, 1);
        assert_eq!(
            fleet.world().resource::<Time<Fixed>>().overstep(),
            period / 2,
            "fractional interpolation remainder survives the whole-step cap"
        );
        let ticks: Vec<_> = fleet
            .world()
            .resource::<MeshOutbox>()
            .pending_frames()
            .iter()
            .filter_map(|frame| match frame {
                MeshFrame::Tick(frame) => Some(frame.tick),
                _ => None,
            })
            .collect();
        assert_eq!(ticks, vec![0], "one committed step seals one bearing frame");

        let mut solo = App::new();
        solo.add_plugins(bevy::time::TimePlugin);
        register_sim_tick(&mut solo);
        solo.init_resource::<crate::command_admission::log::PendingCommands>();
        register_lockstep(&mut solo);
        solo.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        solo.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        solo.update();
        solo.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 5));
        solo.update();
        assert_eq!(
            solo.world().resource::<SimTick>().0,
            5,
            "a standalone simulation retains ordinary fixed-step catch-up"
        );
    }

    #[test]
    fn a_solo_gm_pause_boundary_cannot_be_skipped_by_fixed_catch_up() {
        use crate::sim_tick::{register_sim_tick, SimTick};

        let period = std::time::Duration::from_millis(10);
        let local = HostSlot(1);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        register_sim_tick(&mut app);
        app.init_resource::<crate::command_admission::log::PendingCommands>();
        register_lockstep(&mut app);
        app.insert_resource(FleetLockstep(LockstepSession::new(local, [local], 6)));
        app.world_mut()
            .resource_mut::<crate::gm_action::GmActionJournal>()
            .insert(crate::gm_action::GmActionGrant {
                from: local,
                sequenced_by: local,
                operator_id: "solo-gm".into(),
                correlation: crate::gm_action::GmActionId::new("catch-up-pause").unwrap(),
                recovery_generation: 0,
                apply_tick: 1,
                order: crate::gm_action::GmActionOrder::new(local, 1),
                action: crate::gm_action::GmAction::SetSessionPaused { active: true },
            })
            .unwrap();
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();

        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 5));
        app.update();
        assert_eq!(
            app.world().resource::<SimTick>().0,
            1,
            "the oversized frame stops on the unapplied GM boundary"
        );
        assert_eq!(
            app.world()
                .resource::<crate::gm_action::GmActionJournal>()
                .applied_grants(),
            0,
            "the boundary is not misreported as applied before its next PreUpdate"
        );

        app.update();
        assert_eq!(app.world().resource::<SimTick>().0, 1);
        assert!(
            app.world()
                .resource::<crate::gm_action::SimulationPaused>()
                .0
        );
        assert_eq!(
            app.world()
                .resource::<crate::gm_action::GmActionJournal>()
                .applied_grants(),
            1
        );
        assert_eq!(
            app.world()
                .resource::<crate::gm_action::GmActionLog>()
                .entries()[0]
                .outcome,
            crate::gm_action::GmActionOutcome::Applied
        );
    }

    #[test]
    fn applying_pause_consumes_the_current_frame_delta_standalone_and_in_a_fleet() {
        use crate::sim_tick::{register_sim_tick, SimTick};

        let period = std::time::Duration::from_millis(10);
        for fleet in [false, true] {
            let local = HostSlot(1);
            let mut app = App::new();
            app.add_plugins(bevy::time::TimePlugin);
            register_sim_tick(&mut app);
            app.init_resource::<crate::command_admission::log::PendingCommands>();
            register_lockstep(&mut app);
            if fleet {
                let mut session = LockstepSession::new(local, [local, HostSlot(2)], 6);
                session.observe(HostSlot(2), 100);
                app.insert_resource(FleetLockstep(session));
            }
            app.world_mut()
                .resource_mut::<Time<Fixed>>()
                .set_timestep(period);
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::ZERO,
            ));
            app.update();

            app.world_mut()
                .resource_mut::<crate::gm_action::GmActionJournal>()
                .insert(crate::gm_action::GmActionGrant {
                    from: local,
                    sequenced_by: local,
                    operator_id: "gm".into(),
                    correlation: crate::gm_action::GmActionId::new(if fleet {
                        "fleet-now-pause"
                    } else {
                        "standalone-now-pause"
                    })
                    .unwrap(),
                    recovery_generation: 0,
                    apply_tick: 0,
                    order: crate::gm_action::GmActionOrder::new(local, 1),
                    action: crate::gm_action::GmAction::SetSessionPaused { active: true },
                })
                .unwrap();
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 5));
            app.update();

            assert_eq!(
                app.world().resource::<SimTick>().0,
                0,
                "an apply-at-now Pause leaked fixed work (fleet={fleet})"
            );
            assert!(
                app.world()
                    .resource::<crate::gm_action::SimulationPaused>()
                    .0
            );
            assert_eq!(
                app.world()
                    .resource::<crate::gm_action::GmActionJournal>()
                    .applied_grants(),
                1
            );
        }
    }
}
