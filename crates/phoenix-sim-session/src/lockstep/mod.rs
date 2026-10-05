//! Session-owned fleet membership, admission records and agreement state.
use bevy::prelude::*;
pub use phoenix_runtime::session::{LockstepSession, Stall};
use phoenix_runtime::HostSlot;
pub mod crew;
pub mod frame;
pub mod host_loss;
pub mod slot_recovery;

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

/// One player ship in the frozen fleet.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FleetShip {
    /// The host that flies it.
    pub host: HostSlot,
    /// The hull that host chose, as an entity-template path. `None` means "use
    /// whatever this host's own lobby selected", which is what a solo roster
    /// says and is why a solo run is byte-identical to a pre-#1116 one.
    pub ship_path: Option<String>,
    /// Scenario-authored player-ship slot identity. Separate from numeric
    /// `host`, which is transport authority and may change on recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authored_slot_id: Option<String>,
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
    pub crew: Vec<(phoenix_model::messages::StationId, String)>,
}

impl FleetShip {
    pub fn new(host: HostSlot) -> Self {
        Self {
            host,
            ship_path: None,
            authored_slot_id: None,
            crew: Vec::new(),
        }
    }

    /// The rating a human is holding `station` at, or `None` for an empty seat.
    pub fn rating_at(&self, station: &phoenix_model::messages::StationId) -> Option<&str> {
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

    /// The one-peer roster of a STANDALONE game master — the landing's `host_gm`
    /// route, where the game master IS the session.
    ///
    /// That peer runs the only simulation, owns the authored hull its operator
    /// picked on the landing (flown by AI backfill — see
    /// `crate::lobby::server::update_session_with_config`), and holds the public
    /// operator identity its own privileged actions are attributed to. Binding
    /// that identity here is what lets the ordinary admission path in
    /// [`crate::gm_action::submit_local`] resolve
    /// `gm_operator(local)`; without it a game-master-only session could look at
    /// its world but never act on it.
    ///
    /// This remains a convenience for the one-peer landing route. Fleet rosters
    /// use [`Self::with_participants_and_gms`], which can bind the same technical
    /// participant to a ship and an equal GM operator without duplicating the
    /// lockstep wait-set.
    ///
    /// `operator_id` is bounded exactly as a mesh binding is, so an unusable id
    /// fails closed here rather than at the first refused action.
    pub fn solo_game_master(operator_id: impl Into<String>) -> Option<Self> {
        let operator_id = operator_id.into();
        if operator_id.is_empty()
            || operator_id.chars().count() > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
        {
            return None;
        }
        Some(Self {
            ships: vec![FleetShip::new(HostSlot::SOLO)],
            participants: vec![HostSlot::SOLO],
            gms: vec![FleetGm {
                host: HostSlot::SOLO,
                operator_id,
            }],
            local: HostSlot::SOLO,
            owner: HostSlot::SOLO,
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

    /// The public operator identity authenticated to `host`, when that technical
    /// participant advertises GM capability. It may also own a player ship.
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

    /// Transfer only the coordinator identity after survivor reconciliation.
    /// Hulls, crew, participant identities and local ownership remain frozen.
    pub fn transfer_owner(&mut self, previous: HostSlot, next: HostSlot) -> bool {
        if self.owner() != previous || next == previous || !self.is_member(next) {
            return false;
        }
        self.owner = next;
        true
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
    pub fn crew_of(&self, host: HostSlot) -> &[(phoenix_model::messages::StationId, String)] {
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
pub fn uses_live_sessions(
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

/// What the fleet's periodic digest exchange has found.
///
/// The surface #1118 takes over: today it *names* a divergence, and recovering
/// from one is that issue's. The comparator is deliberately not re-invented —
/// [`phoenix_runtime::digest::DigestLedger`] already pairs samples **by tick** and
/// already produces "they agreed at tick 240 and disagreed by tick 250", so
/// this holds one ledger per peer and lets that code answer.
#[derive(Resource, Debug)]
pub struct MeshAgreement {
    /// This host's own sampled digests.
    pub local: phoenix_runtime::digest::DigestLedger,
    /// Every peer's, as reported.
    pub peers: std::collections::BTreeMap<HostSlot, phoenix_runtime::digest::DigestLedger>,
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
            local: phoenix_runtime::digest::DigestLedger::new(interval),
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

    /// Retain an authenticated peer checkpoint and compare if local arrived first.
    pub fn record_peer(
        &mut self,
        peer: HostSlot,
        tick: u64,
        digest: u64,
    ) -> Option<MeshDisagreement> {
        let found = self.compare_sample(peer, tick, digest);
        self.peers
            .entry(peer)
            .or_insert_with(|| phoenix_runtime::digest::DigestLedger::new(self.local.interval))
            .record(tick, digest);
        found
    }

    /// Retain our checkpoint and reconcile all peers that arrived before it.
    pub fn record_local(&mut self, tick: u64, digest: u64) -> Vec<MeshDisagreement> {
        self.local.record(tick, digest);
        let early: Vec<_> = self
            .peers
            .iter()
            .filter_map(|(peer, ledger)| ledger.digest_at(tick).map(|digest| (*peer, digest)))
            .collect();
        early
            .into_iter()
            .filter_map(|(peer, digest)| self.compare_sample(peer, tick, digest))
            .collect()
    }

    /// Compare when either half of a checkpoint arrives last. Discovery order
    /// and duplicate suppression are shared by inbound and local sampling.
    pub fn compare_sample(
        &mut self,
        peer: HostSlot,
        tick: u64,
        peer_digest: u64,
    ) -> Option<MeshDisagreement> {
        let local_digest = self.local.digest_at(tick)?;
        if local_digest == peer_digest {
            return None;
        }
        let found = MeshDisagreement {
            tick,
            peer,
            local_digest,
            peer_digest,
        };
        if self.disagreements.contains(&found) {
            return None;
        }
        self.disagreements.push(found);
        Some(found)
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
    pub fn stalled(&mut self, stall: Stall) {
        self.stalled_frames = self.stalled_frames.saturating_add(1);
        self.current_run = self.current_run.saturating_add(1);
        self.longest_stall = self.longest_stall.max(self.current_run);
        self.last_stall = Some(stall);
    }

    pub fn running(&mut self) {
        self.current_run = 0;
    }

    /// Whether this host is withholding a tick right now.
    pub fn is_stalled(&self) -> bool {
        self.current_run > 0
    }
}

#[cfg(test)]
#[path = "agreement_tests.rs"]
mod agreement_tests;
