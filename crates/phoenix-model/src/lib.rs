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

pub mod damage;
pub mod energy;
pub mod equipment_recovery_ui;
pub mod field;
pub mod fitting;
pub mod focused_enquiry;
pub mod interior;
pub mod job;
pub mod logistics;
pub mod ports;
pub mod ports_ui;
pub mod production;
pub mod production_ui;
pub mod recovery;
pub mod recovery_ui;
pub mod sensing;
pub mod sensing_forecast;
pub mod sensing_references;
pub mod sensing_sharing;
pub mod sensing_tracks;

pub mod logistics_ui;
pub mod umbilical;
