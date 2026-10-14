//! Shared bounded resources and atomic next-stage claims.
pub mod stock;
pub use stock::*;
pub mod energy;
pub mod energy_ai;
pub mod energy_network;
pub mod service;
