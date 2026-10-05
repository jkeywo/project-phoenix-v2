//! The host-to-host lockstep vocabulary (issue #1116).
//!
//! Pure and Bevy-free. These are the frames a ship host says to the other ship
//! hosts in its fleet once the roster has frozen and the mission is running —
//! the running-simulation half of the vocabulary `gui/host-mesh.js` opened for
//! the fleet lobby.
//!
//! # Why these types are not `ClientMessage`/`ServerMessage`
//!
//! #1114 AC3 and `p2p-delta-transport-is-a-star-today` both require the
//! host-to-host message set to stay separate from the crew protocol, and the
//! reason is the same one #1114 gave: a ship host is not a console. None of
//! what a host says here — "here is my crew's input for tick 412", "I am ready
//! through tick 418", "my world folds to this at tick 400" — has any meaning on
//! a phone, and a frame that could be decoded by both wires would make "each
//! crew star stays attached to its own host" a filtering rule rather than a
//! fact about the vocabulary.
//!
//! # The envelope
//!
//! [`HOST_MESH_PROTOCOL`] is the same revision number `gui/host-mesh.js` carries
//! in its `m` field, and it is declared here as well as there because the two
//! halves must move together: a host whose Rust half speaks revision 2 and whose
//! JS half speaks revision 1 would assemble a fleet and then silently fail to
//! agree a tick. The JS side has a test that reads this constant's value out of
//! the shared build flags for exactly that reason.
//!
//! # What is deliberately absent
//!
//! * **Session tokens.** A [`MeshCommand`] is the projection of a
//!   `LoggedCommand`, which already omits them (they are bearer credentials —
//!   `command_admission::log`'s module docs say why), so the property is
//!   inherited rather than restated.
//! * **Any AI emission.** AGENTS.md rule 6 and
//!   `p2p-delta-ai-output-is-not-logged`: what crosses a network boundary is
//!   logged, and an AI decision never crosses one. Every host derives every NPC
//!   from the same ticks and the same RNG streams, so shipping AI output would
//!   double-apply it.
//! * **A snapshot chunk.** Transferring the portable record is #1117's, and it
//!   rides here as [`MeshFrame::Snapshot`] — one framed piece of the whole-payload
//!   RON export. The framing, chunking and integrity live in
//!   [`crate::lockstep::transfer`], which is pure; this enum only carries a chunk
//!   across the wire beside the tick and digest frames.

use serde::{Deserialize, Serialize};

use crate::command_admission::log::HostSlot;
use crate::lockstep::transfer::SnapshotChunk;

/// Host-mesh vocabulary revision.
///
/// `1` was #1114's fleet lobby (hello / welcome / refused / slot / roster /
/// admission). `2` added the running-mission frames below (tick / digest). `3`
/// adds [`HostLossFrame`] — one host telling the fleet that a ship host has
/// vanished (issue #1119). `4` adds [`SlotClaimFrame`] — the owner announcing
/// that a replacement machine has claimed a disconnected fixed slot, so the whole
/// fleet resolves the same recovery (issue #1120). `5` adds the host-mesh GM
/// role and its public operator roster (issue #1289); a GM consumes no ship slot,
/// so a revision-4 peer would otherwise silently disagree about membership.
/// `6` added collective GM/crew start policy. `7` makes its immutable start
/// grant carry the exact logical tick it applies on; a revision-6 peer would
/// otherwise apply the grant on its next locally observed fixed step.
/// `8` adds the paused-safe, typed and attributed GM action grant. `9` adds the
/// visible first-time GM admission controls and Rust-owned paused transfer frame.
/// `10` added the owner-sequenced restore-boundary clock. `11` distinguishes a
/// known departed GM reconnect from a first-time admission while keeping both on
/// that same paused transfer transaction.
/// Bumped rather than
/// extended-in-place because #1114's decoder refuses a frame whose `m` it does
/// not recognise, which is precisely the behaviour that makes a mixed-build fleet
/// fail loudly instead of half-understanding each other: a revision-3 build that
/// silently DROPPED a slot-claim frame would keep the recovered ship on Backfill
/// while the revision-4 hosts handed it back to the replacement — a split with no
/// symptom but a divergence.
// Revision 12 also carries the frozen Station ratings through the lobby mesh;
// revision 11's JavaScript would discard them before Rust could seed a ship.
// Revision 13 adds [`crate::gm_restore::GmRestoreFrame`] — the readiness,
// per-peer load report and terminal settle of a multi-peer live restore
// (issue #1447). A revision-12 host would hold its world on the canonical
// request, never answer the readiness ask, and be excluded as a nonresponder
// while the revision-13 hosts rewound without it: a fleet split with no
// symptom, which is exactly the class of change this revision refuses whole.
// Revision 14 adds one technical peer carrying both ship and GM capabilities.
// Revision 13 peers would collapse it to one role or count two simulations.
// Revision 16 requires unified authored ship initialization. The wire fields
// are unchanged, but older hosts would simulate different equipment/defaults.
pub const HOST_MESH_PROTOCOL: u32 = 16;

/// Everything the running half of the host mesh says.
///
/// A closed enum rather than a string tag, so a receiver that compiles has
/// handled every frame; the JS half's `t` strings are the same set, and the
/// codec is where the two spellings meet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MeshFrame {
    /// Input, and the watermark that says there is no more of it for those ticks.
    Tick(TickFrame),
    /// A sampled digest, for agreement checking.
    Digest(DigestFrame),
    /// One framed piece of a portable snapshot in transit (issue #1117). The
    /// chunking and integrity are [`crate::lockstep::transfer`]'s; this variant
    /// only carries a piece across the mesh.
    Snapshot(SnapshotChunk),
    /// A ship host has left; its ship flips to Backfill at the agreed tick
    /// (issue #1119).
    HostLoss(HostLossFrame),
    /// A replacement machine has claimed a disconnected fixed slot; the fleet
    /// resolves the same recovery from it (issue #1120).
    SlotClaim(SlotClaimFrame),
    /// A GM proposal or owner-sequenced terminal decision, independently
    /// deliverable while `FixedUpdate` is starved by Pause (issue #1292).
    GmAction(crate::gm_action::GmActionFrame),
    /// First-time mid-session GM pause/transfer/admission protocol (#1293).
    GmJoin(crate::gm_join::GmJoinFrame),
    /// Multi-peer live-restore readiness, load reports and the terminal settle
    /// (issue #1447). Like every other running-mission frame it is minted and
    /// read only by Rust; `gui/host-mesh.js` ferries it opaquely.
    GmRestore(crate::gm_restore::GmRestoreFrame),
}

impl MeshFrame {
    /// The slot that sent this frame.
    pub fn from(&self) -> HostSlot {
        match self {
            MeshFrame::Tick(f) => f.from,
            MeshFrame::Digest(f) => f.from,
            MeshFrame::Snapshot(f) => f.from,
            MeshFrame::HostLoss(f) => f.from,
            MeshFrame::SlotClaim(f) => f.from,
            MeshFrame::GmAction(f) => f.wire_from(),
            MeshFrame::GmJoin(f) => f.wire_from(),
            MeshFrame::GmRestore(f) => f.from(),
        }
    }

    /// The frame type's wire name, matching `gui/host-mesh.js`'s `t` field.
    pub fn type_name(&self) -> &'static str {
        match self {
            MeshFrame::Tick(_) => TYPE_TICK,
            MeshFrame::Digest(_) => TYPE_DIGEST,
            MeshFrame::Snapshot(_) => TYPE_SNAPSHOT,
            MeshFrame::HostLoss(_) => TYPE_HOST_LOSS,
            MeshFrame::SlotClaim(_) => TYPE_SLOT_CLAIM,
            MeshFrame::GmAction(_) => TYPE_GM_ACTION,
            MeshFrame::GmJoin(_) => TYPE_GM_JOIN,
            MeshFrame::GmRestore(_) => TYPE_GM_RESTORE,
        }
    }
}

/// The `t` value a [`MeshFrame::Tick`] carries on the JS wire.
pub const TYPE_TICK: &str = "tick";
/// The `t` value a [`MeshFrame::Digest`] carries on the JS wire.
pub const TYPE_DIGEST: &str = "digest";
/// The `t` value a [`MeshFrame::Snapshot`] carries on the JS wire (issue #1117).
pub const TYPE_SNAPSHOT: &str = "snapshot";
/// The `t` value a [`MeshFrame::HostLoss`] carries on the JS wire.
pub const TYPE_HOST_LOSS: &str = "host-loss";
/// The `t` value a [`MeshFrame::SlotClaim`] carries on the JS wire (issue #1120).
pub const TYPE_SLOT_CLAIM: &str = "slot-claim";
/// The `t` value a [`MeshFrame::GmAction`] carries on the JS wire.
pub const TYPE_GM_ACTION: &str = "gm-action";
/// The `t` value a [`MeshFrame::GmJoin`] carries on the JS wire.
pub const TYPE_GM_JOIN: &str = "gm-join";
/// The `t` value a [`MeshFrame::GmRestore`] carries on the JS wire (issue #1447).
pub const TYPE_GM_RESTORE: &str = "gm-restore";

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;

pub use phoenix_sim_session::lockstep::frame::*;
