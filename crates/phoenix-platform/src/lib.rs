//! Platform mechanisms. Applications supply game policy and UI documents.
#![forbid(unsafe_code)]
#[cfg(not(target_arch = "wasm32"))]
pub mod native_file;

#[cfg(feature = "audio")]
pub mod audio_decode;
pub mod geometry;
pub mod input_routing;
pub mod render_geometry;

#[cfg(feature = "surface")]
pub mod frames;
pub mod monitors;
pub mod surface;
#[cfg(feature = "surface")]
pub mod surface_stats;
#[cfg(feature = "render")]
pub mod upload;

#[cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]
pub mod ultralight;

#[cfg(not(target_arch = "wasm32"))]
pub mod native_capture;
