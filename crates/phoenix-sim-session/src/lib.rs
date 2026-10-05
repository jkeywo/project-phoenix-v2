//! Crew Sessions, lobby decisions and authenticated fleet protocol state.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod command_admission;
pub mod gm_roster;
pub mod lobby;
pub mod session_connections;
pub use phoenix_sim_contracts::sim_tick;
pub mod lockstep;
