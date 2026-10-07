//! Host preparation and compatibility facade for delivery.
pub use phoenix_delivery::*;
#[cfg(not(target_arch = "wasm32"))]
pub mod args;
pub mod payload;
#[cfg(not(target_arch = "wasm32"))]
pub mod serve;
