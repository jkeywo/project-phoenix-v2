//! Shared Phoenix data vocabulary; no simulation, I/O or renderer.
#![forbid(unsafe_code)]
pub mod console;
pub mod debug_surface;
pub mod entity;
pub mod identity;
pub mod tags;

pub mod texture;

pub mod rig;

pub mod messages;
pub mod wire;

pub mod system_status;

pub mod observation;
