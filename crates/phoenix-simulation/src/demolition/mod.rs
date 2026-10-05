//! Controlled demolition of an obstruction (issue #1350, PRD #1337).
//!
//! Clearing a Falling Skyway obstruction is a four-stage act — Tactical
//! dispatches Security to place charges, Engineering holds the mass with tractor
//! control where it needs it, Security withdraws, Tactical detonates — and the
//! first three stages are the existing Security team-dispatch (`src/security/`)
//! and tractor-hold (`src/tractor/`) machinery unchanged. This module is the
//! fourth stage and the authoritative operation state that ties the others
//! together: the one-shot detonation, its refusal gate, and the four-outcome
//! decision (safe / unsupported / premature / — the fourth, ordinary weapons
//! fire, being a scenario `on_destroyed` handler that needs no engine support).
//!
//! Split the same way Security and the tractor are (rule 10):
//!
//! * [`ops`] — the pure, Bevy-free half: the authored `[demolition_target]`
//!   config and its validation, the refusal and outcome vocabularies, the
//!   detonation verdict and the four-outcome decision. Unit-tested in isolation.
//! * [`server`] — the Bevy adapter: the per-target [`server::DemolitionTarget`]
//!   and per-ship [`server::DemolitionControl`] components, the fixed-tick handler
//!   that takes `DetonateCharges` (routed on the `security` target it borrows) and
//!   raises the authored outcome flag, the backfill host, and the blackboard
//!   publisher.
//!
//! # What the engine does NOT know
//!
//! What a safe demolition leaves behind, what an unsupported one costs, who a
//! premature one kills and what ordinary weapons fire scatters are the scenario's
//! business, authored entirely in TOML off the flags this module raises. Nothing
//! here branches on a world entity's name (rule 11).

/// The pure, Bevy-free authored config, refusal/outcome vocabularies, detonation
/// verdict and four-outcome decision.
pub mod ops;

/// The Bevy adapter: the components, the fixed-tick handler, the backfill host
/// and the blackboard publisher.
pub mod server;

pub use ops::{
    detonation_status, resolve_outcome, DemolitionConfig, DemolitionOutcome, DemolitionRefusal,
};
pub use server::{
    demolition_blackboard_key, handle_demolition_commands, operate_demolition_ai,
    publish_demolition_blackboard, DemolitionControl, DemolitionPlugin, DemolitionTarget,
    DEMOLITION_BLACKBOARD_KEY,
};
