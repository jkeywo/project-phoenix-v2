//! Deterministic multiplayer machinery, independent of game rules and I/O.
//! Games admit commands, apply them and capture their own state. This crate
//! owns technical ordering, readiness, continuation and recovery decisions.
#![forbid(unsafe_code)]
pub mod continuation;
pub mod digest;
pub mod identity;
pub mod recovery_plan;
pub mod session;
pub mod transfer;
pub use identity::{CommandOrder, HostSlot};

pub mod commands;
