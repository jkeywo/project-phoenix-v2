//! Shared resource contracts with the game's endpoint adapter.
pub use phoenix_sim_gameplay::resources::*;
mod ports_equipment;
pub mod ports_server;
pub mod server;

pub mod equipment_recovery_server;
pub mod fitting_server;
mod logistics_mission;
pub mod logistics_server;
pub mod production_server;
pub mod recovery_server;
