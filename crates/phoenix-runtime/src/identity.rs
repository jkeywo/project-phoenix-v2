//! Technical participant identity and canonical command ordering.
use serde::{Deserialize, Serialize};

/// Which host in the frozen fleet roster issued a command (issue #1116).
///
/// The fleet owner mints slot ids as `slot-N` (`gui/host-mesh.js`), and the
/// ordinal `N` is what crosses the simulation boundary: it is small, totally
/// ordered, and — because one machine minted every id in one monotonic sequence
/// — it means the same thing on every host. `p2p-delta-identity-is-minted` is
/// the reason the ordinal cannot be self-assigned: "an id is a function of when
/// and in what order a peer minted it".
///
/// A host with no fleet — the single-host case, and every fixture — is
/// [`HostSlot::SOLO`]. That is a real slot rather than an absence, so the
/// ordering key has the same shape whether or not a mesh is running and the
/// solo path is not a second code path.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct HostSlot(pub u32);

impl HostSlot {
    /// The slot a host with no fleet issues from.
    ///
    /// Deliberately `0` and not `1`: `gui/host-mesh.js` numbers fleet slots from
    /// `slot-1`, so a solo host's key can never collide with a fleet member's,
    /// and a log carrying solo entries beside fleet entries (a fleet formed
    /// mid-session would be exactly that) still sorts unambiguously.
    pub const SOLO: HostSlot = HostSlot(0);

    /// Parse the ordinal out of a `slot-N` id from the host-mesh roster.
    ///
    /// `None` for anything that is not one, because a roster id this side
    /// cannot parse is a protocol disagreement rather than a slot to guess at.
    pub fn from_slot_id(id: &str) -> Option<Self> {
        id.strip_prefix("slot-")?.parse().ok().map(HostSlot)
    }

    /// The `slot-N` id this ordinal renders as, for the host-mesh roster.
    pub fn slot_id(&self) -> String {
        format!("slot-{}", self.0)
    }
}

/// The peer-independent tiebreak between commands that apply on the same tick
/// (issue #1116).
///
/// Ordering is the derived field order — origin first, then that origin's own
/// sequence — so the whole fleet's traffic for one tick has exactly one order,
/// and every host computes it from the key alone without knowing what arrived
/// when. The pre-#1116 `arrival` counter was the opposite: a local wire-decode
/// index, which is a fact about the receiver rather than about the command.
///
/// `seq` restarts at zero on the run boundary along with the log
/// (the game's command-log reset), on every host, because a run boundary is a tick
/// every host agrees on. Trap T8 of the #1116 brief is exactly this: an
/// ordering key that survived the reset on one host and not another would
/// tiebreak round two differently from round one.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CommandOrder {
    /// The host that admitted this command from its own crew.
    pub origin: HostSlot,
    /// That host's own monotonic counter, restarted at each run boundary.
    pub seq: u64,
}

impl CommandOrder {
    pub fn new(origin: HostSlot, seq: u64) -> Self {
        Self { origin, seq }
    }
}
