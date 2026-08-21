//! The transfer umbilical (issue #1160) — the third slice of PRD #1143's
//! coupling family.
//!
//! Engineering gets a transfer umbilical: a first-class engineering-owned
//! `[[system]]` that declares its own power group, carries a damage entry, is
//! admission-gated, registers as an admitted consumer and publishes its own
//! blackboard. Running it moves an authored capacity per second between the two
//! docked hulls' capacity ledgers — so resupply is gated on Helm having achieved
//! the dock (#1159): two seats on two ships, or two seats on one. Undocking,
//! losing the power allocation or damaging the umbilical stops the flow where it
//! stands; what has already moved has moved.
//!
//! Split the same way the tractor and dock are (rule 10):
//!
//! * [`flow`] — the pure, Bevy-free half: the flow-arithmetic module (how much
//!   crosses this tick, clamped by source depletion and destination headroom in
//!   both directions), the authored `[umbilical]` config and its validation, and
//!   the refusal vocabulary. Unit-tested in isolation.
//! * [`server`] — the Bevy adapter: the per-ship [`server::TransferUmbilical`]
//!   component, the fixed-tick systems that take the start/stop commands, gate on
//!   #1159's docked state, move the capacity through the infrastructure queue, and
//!   publish the blackboard.
//!
//! This copies the shape the tractor slice (#1156) established and the dock slice
//! (#1159) it gates on.

/// The pure, Bevy-free flow-arithmetic module, authored config and refusal
/// vocabulary.
pub mod flow;
/// The Bevy adapter: the component, the fixed-tick systems and the blackboard
/// publisher.
pub mod server;

pub use flow::{
    plan_flow, CapacityEnd, FlowContext, FlowEnds, FlowVerdict, UmbilicalConfig,
    UmbilicalDirection, UmbilicalRefusal,
};
pub use server::{
    handle_umbilical_commands, publish_umbilical_blackboard, tick_umbilical,
    umbilical_blackboard_key, TransferUmbilical, UmbilicalPlugin, UmbilicalSaveState,
};
