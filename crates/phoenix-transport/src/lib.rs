//! Nonblocking game transport and physical connection ownership.
#![forbid(unsafe_code)]
pub mod connections;
pub mod rendezvous;
pub mod transport;
pub use transport::*;

pub mod codec;
pub mod relay;
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod relay_socket;
pub mod shared_connections;
pub mod socket;
