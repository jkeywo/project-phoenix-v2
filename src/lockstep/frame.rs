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
//! * **A snapshot.** Transferring the portable record is #1117's.

use serde::{Deserialize, Serialize};

use crate::command_admission::log::{CommandOrder, HostSlot, ShipKey};
use crate::core::messages::{SystemControlPayload, SystemId};

/// Host-mesh vocabulary revision.
///
/// `1` was #1114's fleet lobby (hello / welcome / refused / slot / roster /
/// admission). `2` adds the running-mission frames below. Bumped rather than
/// extended-in-place because #1114's decoder refuses a frame whose `m` it does
/// not recognise, which is precisely the behaviour that makes a mixed-build
/// fleet fail loudly instead of half-understanding each other.
pub const HOST_MESH_PROTOCOL: u32 = 2;

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
}

impl MeshFrame {
    /// The slot that sent this frame.
    pub fn from(&self) -> HostSlot {
        match self {
            MeshFrame::Tick(f) => f.from,
            MeshFrame::Digest(f) => f.from,
        }
    }

    /// The frame type's wire name, matching `gui/host-mesh.js`'s `t` field.
    pub fn type_name(&self) -> &'static str {
        match self {
            MeshFrame::Tick(_) => TYPE_TICK,
            MeshFrame::Digest(_) => TYPE_DIGEST,
        }
    }
}

/// The `t` value a [`MeshFrame::Tick`] carries on the JS wire.
pub const TYPE_TICK: &str = "tick";
/// The `t` value a [`MeshFrame::Digest`] carries on the JS wire.
pub const TYPE_DIGEST: &str = "digest";

#[cfg(test)]
mod tests {
    use super::*;

    fn command(origin: u32, seq: u64) -> MeshCommand {
        MeshCommand {
            tick: 12,
            order: CommandOrder::new(HostSlot(origin), seq),
            ship: ShipKey("uuid-ship".into()),
            target: SystemId("helm".into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        }
    }

    /// The one rule a receiver can enforce about a peer's traffic: a host may
    /// speak only for its own slot.
    #[test]
    fn a_command_belongs_to_the_slot_that_ordered_it() {
        let cmd = command(2, 0);
        assert!(cmd.is_from(HostSlot(2)));
        assert!(
            !cmd.is_from(HostSlot(3)),
            "a frame from slot 3 carrying slot 2's order is one host speaking \
             for another, and the receiver must be able to say so"
        );
    }

    /// The frames round-trip through the serde shape the codec and the RON
    /// diagnostics both use. Asserted here rather than in the codec because the
    /// property belongs to the vocabulary: a frame that cannot survive being
    /// written down cannot be replayed by #1118 either.
    #[test]
    fn every_frame_round_trips() {
        let frames = vec![
            MeshFrame::Tick(TickFrame {
                from: HostSlot(1),
                tick: 10,
                ready_through: 16,
                commands: vec![command(1, 0), command(1, 1)],
            }),
            MeshFrame::Digest(DigestFrame {
                from: HostSlot(2),
                tick: 400,
                digest: 0xdead_beef,
            }),
        ];
        for frame in frames {
            let text = ron::ser::to_string(&frame).expect("a frame serialises");
            let back: MeshFrame = ron::from_str(&text).expect("and comes back");
            assert_eq!(back, frame);
            assert!(
                !text.contains("response_token"),
                "a mesh frame must never carry a session token:\n{text}"
            );
        }
    }

    /// The two halves of the host mesh agree on the revision they speak.
    ///
    /// Pinned as a value rather than compared to the JS constant, because the
    /// JS half is not compiled here — the Vitest suite asserts the same number
    /// from its side, and the pair of pins is what catches a one-sided bump.
    #[test]
    fn the_protocol_revision_is_pinned() {
        assert_eq!(
            HOST_MESH_PROTOCOL, 2,
            "bumping this is a fleet-wide incompatible change: gui/host-mesh.js \
             refuses a frame whose `m` it does not know, so both halves and the \
             Vitest pin move together or a mixed fleet fails to agree a tick"
        );
    }
}
