//! Shared bounded resources and atomic next-stage claims.
pub mod stock;
pub use stock::*;
pub mod energy;
pub mod energy_ai;
pub mod energy_network;
pub mod logistics;
pub mod logistics_ai;
pub mod logistics_profile;
pub mod ports;
pub mod ports_profile;
pub mod production;
pub mod production_ai;
pub mod production_profile;
pub mod recovery;
pub mod recovery_ai;
pub mod recovery_profile;
pub mod service;
