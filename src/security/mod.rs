//! The Security System (issue #1346, PRD #1337).
//!
//! A hull musters Security teams and sends them across to do dangerous, authored
//! work at a named target — secure or contain it, assist an evacuation already
//! under way there, board it, place charges on it. Security is a generic
//! station-owned `[[system]]`, assigned through ship configuration: on the
//! Alliance Destroyer, Tactical owns it and musters two teams, each dispatched
//! and recalled independently, with no Duty Officer standing between the seat and
//! the order.
//!
//! Split the same way the tractor, dock and umbilical are (rule 10):
//!
//! * [`teams`] — the pure, Bevy-free half: the action vocabulary and priority
//!   classes, the authored `[security]` and `[security_target]` configs and their
//!   validation, the refusal vocabulary, the per-team state machine, the dispatch
//!   verdict and the backfill selection with its capacity reservation. Unit-tested
//!   in isolation.
//! * [`server`] — the Bevy adapter: the per-ship [`server::ShipSecurityTeams`] and
//!   per-target [`server::SecurityTargetActions`] components, the fixed-tick
//!   systems that take the dispatch/recall commands, walk each team through
//!   deploy → work → withdraw, raise the authored consequence flag on success, run
//!   the backfill host, and publish the console's readout.
//!
//! # What the engine does NOT know
//!
//! Which compartment is on fire, which platform is being evacuated, and what any
//! of it means, are the scenario's business: a target authors what may be done to
//! it, how long it takes, how risky it is, how urgent it is, and the world flag
//! its success raises. Nothing here branches on a world entity's name, so the
//! Falling Skyway's containment path and a later mission's boarding action are the
//! same code with different TOML (AGENTS.md rule 11).

/// The pure, Bevy-free vocabulary, authored configs, team state machine, dispatch
/// verdict and backfill selection.
pub mod teams;

/// The Bevy adapter: the components, the fixed-tick systems, the backfill host
/// and the blackboard publisher.
pub mod server;

pub use server::{
    handle_security_commands, operate_security_ai, publish_security_blackboard,
    security_blackboard_key, tick_security_teams, SecurityAiDispatched, SecurityPlugin,
    SecuritySaveState, SecurityTargetActions, SecurityTeamSaveState, ShipSecurityTeams,
    TASK_VERB_SECURITY_TEAM,
};
pub use teams::{
    dispatch_status, select_assignments, SecurityAction, SecurityActionConfig, SecurityCandidate,
    SecurityConfig, SecurityPriority, SecurityRefusal, SecurityTargetConfig, SecurityTeam,
    SecurityTeamState,
};
