//! Phoenix viewscreen, visuals and local presentation over simulation projections.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
pub use phoenix_simulation::*;
pub mod debug_overlay;
pub mod entities;
#[cfg(feature = "server")]
pub mod gui;
pub mod perf;
pub mod registration;
#[cfg(all(feature = "capture", not(target_arch = "wasm32")))]
pub mod render_capture;
pub mod render_setup;
#[cfg(feature = "server")]
pub mod server;
pub mod server_app_render;
#[cfg(feature = "viewer")]
pub mod viewer;

#[cfg(test)]
mod content_compatibility_tests;
