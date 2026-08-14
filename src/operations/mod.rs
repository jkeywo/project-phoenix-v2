//! External **operations** — what a crewed ship does to something outside its
//! own hull (issue #1026, Falling Skyway foundation).
//!
//! Stabilising a failing skyhook, towing a crippled freighter, escorting a
//! convoy: verbs that are neither weapons fire nor navigation, and that the PRD
//! deliberately builds out of **eligibility plus a timed hold** rather than out
//! of a minigame. The hull authors which verbs it can perform; the pure sibling
//! [`hold`] decides whether it may right now and counts the ticks it managed;
//! [`server`] gathers the live inputs, applies the verdict, and pays a completed
//! operation off into the target's [`crate::infrastructure`] condition track.
//!
//! An operation is **not a ship system**. It has no `[[system]]` block, no
//! station owns it, and it takes no damage. It reaches the client through a
//! blackboard of its own under a reserved channel key — see
//! [`server::OPERATIONS_BLACKBOARD_KEY`] — the same way a console-level
//! blackboard is carried under a station id rather than a system id.

/// The pure, Bevy-free half: the authored capability vocabulary, the
/// eligibility verdict, and the timed hold's progress arithmetic.
pub mod hold;
/// The Bevy adapter: the per-ship components, the fixed-tick system that
/// gathers real inputs and applies the pure verdict, the admitted start/abort
/// commands, and the blackboard publisher.
pub mod server;

pub use hold::{
    eligibility, CapabilityConfig, HoldState, Ineligibility, OperationConditions, OperationHold,
    OperationVerb, OperationsConfig, Settlement,
};
pub use server::{
    handle_operation_commands, operations_blackboard_key, publish_operations_blackboard,
    tick_operations, verb_label, OperationsPlugin, PendingOperationStart, ShipOperations,
    OPERATIONS_BLACKBOARD_KEY,
};
