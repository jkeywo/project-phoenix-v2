//! Infrastructure **condition** and **capacity** on authored world furniture
//! (issue #1025, Falling Skyway foundation).
//!
//! A skyhook or a fuel depot is not a ship and not an asteroid: it degrades over
//! a mission, it gets patched up, and how much it can move or hold is a number
//! the scenario asks for rather than one the call site knows. `condition` is the
//! pure, Bevy-free arithmetic and edge detection; `server` is the thin adapter
//! that ticks it against the live world, folds in whatever damage the entity
//! took, and mirrors the resulting operational flags into the world flag store
//! where script conditions and `on_flag_set` / `on_flag_cleared` hooks can see
//! them.

/// The pure condition/capacity track: authored TOML shape, degradation and
/// repair arithmetic, and hysteresis-guarded threshold crossing.
pub mod condition;
/// The Bevy adapter: the per-entity component, the fixed-tick system, and the
/// world-flag mirror.
pub mod server;

pub use condition::{
    CapacityAdjustment, CapacityConfig, ConditionAdjustment, FlagChange, InfrastructureConfig,
    InfrastructureState, ResolvedCapacity, ResolvedThreshold, ThresholdConfig,
};
pub use server::{tick_infrastructure_condition, InfrastructureCondition, InfrastructurePlugin};
