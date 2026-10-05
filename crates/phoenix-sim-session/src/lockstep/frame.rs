use crate::command_admission::log::{CommandOrder, HostSlot, ShipKey};
use phoenix_model::messages::{SystemControlPayload, SystemId};
use serde::{Deserialize, Serialize};

/// One command a host admitted from its own crew, as it crosses to the fleet.
///
/// This is the non-secret projection of the `LoggedCommand` that host recorded:
/// the tick it applies on, the fleet-wide order it holds within that tick, the
/// ship it applies to, and what it asks for. Everything a receiving host needs
/// to queue it in exactly the slot the sender queued it in.
///
/// The receiver does **not** re-run the authority check. It cannot: the check
/// is against the sending host's `Sessions`, and a session token is a bearer
/// credential that deliberately never leaves the host that holds it. So the
/// contract is the one a replay already keeps — *a command on this wire is one
/// an authority check already accepted* — narrowed by the one rule a receiver
/// can enforce for itself, in [`MeshCommand::is_from`]: a host may only speak
/// for the ship its own fleet slot flies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshCommand {
    /// The `SimTick` this applies on, on every host.
    pub tick: u64,
    /// Where it sits in the fleet's agreed order for that tick. The origin
    /// inside it is the sending host's own slot, which is what makes the order
    /// peer-independent.
    pub order: CommandOrder,
    /// The ship whose `AdmittedCommands` it lands in.
    pub ship: ShipKey,
    pub target: SystemId,
    pub payload: SystemControlPayload,
}

impl MeshCommand {
    /// Whether this command claims to come from `slot`.
    ///
    /// One of the two authority checks a receiver runs over peer traffic — the
    /// frame-consistency half. It is a check about the *frame*, not the command:
    /// a `tick` frame from slot 2 carrying a command ordered under slot 3 is a
    /// host speaking for somebody else, and the answer is to drop it rather than
    /// to guess which half is true.
    ///
    /// It does **not** on its own establish that the command targets a ship the
    /// slot is entitled to drive — `apply_mesh_inbox` enforces that separately,
    /// dropping a command whose `ship` is not the hull the sending slot owns.
    /// And it trusts the frame's declared `from`: binding that to the connection
    /// that delivered it is deferred (see `apply_mesh_inbox`'s
    /// `TODO(#1118/#1120 mesh hardening)`), so a receiver currently verifies
    /// origin-slot *consistency* and ship *ownership*, not transport-level
    /// origin authenticity.
    pub fn is_from(&self, slot: HostSlot) -> bool {
        self.order.origin == slot
    }
}

/// One host's complete statement about its own input, up to a tick.
///
/// The frame that makes lockstep work, and the reason it carries
/// [`TickFrame::ready_through`] as well as the commands: a host cannot simulate
/// tick *T* until it knows every peer's input for *T*, and "no input" is an
/// answer it has to receive rather than assume. A fleet where hosts only spoke
/// when they had something to say would either stall forever or speculate, and
/// speculation is what a deterministic lockstep exists to avoid.
///
/// `ready_through` is the sender's `SimTick` plus the agreed `CommandDelay`:
/// every command that host will ever issue for a tick at or below it has
/// already been said. That is why the delay is the fleet's tolerance for
/// latency — a peer may be up to `delay` ticks behind before anyone waits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TickFrame {
    /// The slot that is speaking.
    pub from: HostSlot,
    /// The sender's own `SimTick` when it spoke. Diagnostic — the barrier reads
    /// `ready_through` — but it is what turns "we stalled" into "we stalled
    /// because slot 2 was eleven ticks behind".
    pub tick: u64,
    /// Every tick at or below this one has all of this host's input.
    pub ready_through: u64,
    /// The commands this host admitted from its own crew since it last spoke,
    /// in its own order.
    pub commands: Vec<MeshCommand>,
    /// One immutable fleet-wide start decision, carried only by the technical
    /// owner's authenticated frame. Its `apply_tick` lies beyond this frame's
    /// watermark, so receiving this frame becomes part of the same barrier that
    /// must open before any participant can reach the decision tick.
    pub start_grant: Option<crate::lobby::start_policy::StartGrant>,
}

/// One host's authoritative-state digest at one tick.
///
/// Exchanged periodically so a divergence is named at the tick it happened
/// rather than discovered when two crews see different worlds. #1116 detects
/// and reports; recovering is #1118's, which is why this frame carries the
/// sampled tick rather than a window: `sim_digest::DigestLedger::first_divergence`
/// already pairs samples *by tick*, and #1118 should use it verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestFrame {
    pub from: HostSlot,
    pub tick: u64,
    pub digest: u64,
}

/// One host's report that a ship host has left the fleet (issue #1119).
///
/// This is the observation, not the transition. It names the slot whose host
/// vanished and the tick the fleet agrees that ship's crew stops speaking —
/// `agreed_loss_tick`, the first tick past the lost host's last declared
/// watermark. Every survivor derives that tick from the SAME input (the lost
/// slot's own last [`TickFrame::ready_through`], which reliable delivery gave
/// every survivor identically), so the tick is a function of the observation
/// and not of who noticed first — which is what makes two survivors' reports,
/// or one survivor's repeated report, converge on one transition at one tick
/// (`p2p-delta-backfill-replaces-auto-crew`, #1119 AC5).
///
/// `tick` is carried as well as derived so a survivor that has not itself seen
/// the close — a member behind the star relay — adopts the highest tick anyone
/// derived rather than acting on a stale watermark; the receiver takes the max
/// of its own derivation and this field, so a duplicate or reordered report can
/// only ever agree or raise, never walk the transition backwards.
///
/// It carries no crew, no session token and no ship state: the lost ship keeps
/// its complete authoritative state on every surviving host, and only its
/// control SOURCE flips, through the ordinary Station Rating machinery, at the
/// agreed tick (#1119 AC2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostLossFrame {
    /// The slot reporting the loss. **Advisory**, not authenticated: the handler
    /// ([`apply_mesh_inbox`](crate::lockstep::apply_mesh_inbox)) ignores it when
    /// deriving the agreed tick and normalises it to the local observer on
    /// re-broadcast. On a self-observed local socket close the bridge injects it
    /// EQUAL to `lost` (with tick `0`) — the frame is the fact of the loss, not a
    /// claim about who noticed — so a survivor is the usual reporter but the field
    /// may name the lost slot itself on that first, self-reported observation.
    pub from: HostSlot,
    /// The slot whose host has vanished.
    pub lost: HostSlot,
    /// The tick the reporter agrees the lost ship flips to Backfill on.
    pub tick: u64,
}

/// One machine's granted claim on a disconnected fixed slot (issue #1120).
///
/// After the mission has frozen, a member host's socket close leaves its slot in
/// the roster marked disconnected rather than removed (`gui/host-mesh.js`'s
/// `dropHost`), because that disconnected slot is exactly what a replacement
/// machine recovers. When a replacement types the fleet code and claims that
/// slot, the owner — the one machine every claim passes through, the star centre
/// (`p2p-delta-transport-is-a-star-today`) — stamps the claim with a monotonic
/// `claim_seq` in the order it received it and broadcasts THIS frame to the whole
/// fleet. Every host records it and derives the same recovery from it:
///
/// * the winner of a race between two claims for one slot is the LOWEST
///   `claim_seq` — the first the owner minted, so every host that hears both
///   agrees the same winner from the shared value rather than from arrival order;
/// * the recovery boundary and the leader that transfers the canonical record are
///   pure functions of `slot`, `tick` and the frozen roster, identical everywhere.
///
/// It carries no snapshot and no crew: the recovery restores the whole
/// authoritative record through #1117's transfer, and the replacement's own crew
/// reconnects through their ordinary Session identities. Like every other
/// running-mission frame this is minted and read only by Rust; `gui/host-mesh.js`
/// ferries it opaquely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotClaimFrame {
    /// The slot that announced the granted claim — always the fleet owner (the
    /// star centre), which is the sole minter of `claim_seq`. Authenticated at the
    /// mesh boundary like every other frame's `from`.
    pub from: HostSlot,
    /// The disconnected fixed slot being reclaimed — the identity the replacement
    /// machine assumes, and the ship whose canonical record it restores.
    pub slot: HostSlot,
    /// The owner-minted monotonic order of this claim among all claims for `slot`.
    /// The deterministic tiebreak: the lowest wins, so a race resolves the same on
    /// every host without depending on who processed a claim first.
    pub claim_seq: u64,
    /// The owner's `SimTick` when it stamped the claim — the recovery boundary is
    /// derived from it (the same on every host), and it dates the claim.
    pub tick: u64,
}
