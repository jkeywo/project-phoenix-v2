//! Civilian **traffic**: authored routes, crew orders, and compliance
//! (issue #1028, Falling Skyway foundation).
//!
//! Traffic control is only real work for Navigation if the traffic has somewhere
//! to be going and can be told otherwise. `traffic` is the pure, Bevy-free half
//! — the `[[route]]` and `[civilian]` vocabularies, the three order verbs, and
//! the compliance state machine that turns an order plus an authored disposition
//! into `received → acknowledged → complying`, or `refused`, or `non_compliant`.
//! `server` is the thin adapter that runs it against the live world and installs
//! the result as the entity's own doctrine objective, so an ordered civilian is
//! flown by exactly the same NPC helm that flies every other authored directive.

/// The Bevy adapter: the per-entity components, the fixed-tick system, and the
/// directive it installs.
pub mod server;
/// The pure vocabulary and state machine: routes, orders, dispositions,
/// compliance.
pub mod traffic;

pub use server::{
    tick_civilian_traffic, CivilianPlugin, CivilianSection, CivilianTraffic, PendingCivilianOrder,
    CIVILIAN_ROUTE_OBJECTIVE_ID,
};
pub use traffic::{
    CivilianConfig, CivilianOrder, CivilianState, CivilianTravel, ComplianceDisposition,
    ComplianceState, ComplianceTransition, OrderKind, OrderResponse, RouteCompletion, RouteConfig,
    RouteLeg, REASON_UNABLE,
};
