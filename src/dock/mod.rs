//! Helm docking (issue #1159) — the second slice of PRD #1143's coupling family.
//!
//! Helm gains a contextual dock/undock control that appears only while a hull
//! carrying dock markers sits inside the authored range, and running it flies an
//! automatic manoeuvre that mates the two hulls' nearest viable dock-marker pair.
//! Undock backs the ship clear and returns ordinary flight. The docked
//! relationship between the two hulls is a real, published, folded state that
//! survives a snapshot resume and is what the umbilical (#1160) gates on.
//!
//! Split the same way the tractor is (rule 10):
//!
//! * [`mating`] — the pure, Bevy-free half: the marker-mating module (nearest
//!   viable pair + the pose that mates it), the authored `[dock]` config and its
//!   validation, and the refusal vocabulary. Unit-tested in isolation.
//! * [`server`] — the Bevy adapter: the per-ship [`server::DockControl`], the
//!   per-hull [`server::DockMarkers`] resolved from the rig sidecar at spawn, and
//!   the fixed-tick systems that take the commands, decide the dock, fly the own
//!   ship onto its mate, and publish the blackboard.
//!
//! This copies the shape the tractor slice (#1156) established and hands the
//! docked relationship to the umbilical (#1160).

/// The pure, Bevy-free marker-mating module, authored config and refusal
/// vocabulary.
pub mod mating;
/// The Bevy adapter: the components, the fixed-tick systems and the blackboard
/// publisher.
pub mod server;

pub use mating::{nearest_viable_pair, DockConfig, DockMarker, DockRefusal, MatingSolution, Pose};
pub use server::{
    dock_blackboard_key, handle_dock_commands, publish_dock_blackboard, resolve_dock_markers,
    tick_dock, DockControl, DockMarkers, DockPlugin, DockSaveState, DOCK_MARKER_PREFIX,
};
