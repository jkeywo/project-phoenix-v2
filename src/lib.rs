//! Project Phoenix host composition and compatibility paths.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
#![recursion_limit = "256"]
pub use phoenix_simulation::*;
pub mod boot;
#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
pub mod headless;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod native_host;
#[cfg(feature = "server")]
pub mod presentation_adapters;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
pub use phoenix_presentation::gui;
#[cfg(feature = "capture")]
pub use phoenix_presentation::render_capture;
#[cfg(feature = "viewer")]
pub use phoenix_presentation::viewer;
pub use phoenix_presentation::{debug_overlay, entities, perf, render_setup, server_app_render};
pub mod server_app;
pub mod workshop;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use phoenix_platform::native_file;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod layer_tests;
