//! Host adapters and disposable live Test control. Offline authoring is preparation-owned.
pub use phoenix_content_preparation::authoring::*;
#[cfg(target_arch = "wasm32")]
pub mod captured_source;
#[cfg(not(target_arch = "wasm32"))]
pub mod provider;
pub mod rehearsal;
pub mod rehearsal_request;
pub mod test_breakpoint;
pub mod test_clock;
pub mod test_protocol;
pub mod test_source;
pub mod test_trace;
pub mod test_view;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(test)]
mod authoring_integration_tests;
