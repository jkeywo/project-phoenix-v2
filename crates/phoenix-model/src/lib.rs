//! Shared Phoenix data vocabulary; no simulation, I/O or renderer.
#![forbid(unsafe_code)]
pub mod comms_channel;
pub mod comms_history;
pub mod comms_request;
pub mod console;
pub mod debug_surface;
pub mod entity;
pub mod identity;
pub mod tags;
pub mod weapon_targets;

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
pub mod guided_guidance;
pub mod guided_orders;
pub mod guided_round;
pub mod helm_control;
pub mod interior;
pub mod job;
pub mod logistics;
pub mod navigation_sharing;
pub mod personnel;
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
pub mod shield_enquiry;

pub mod emp;
pub mod logistics_ui;
pub mod turret;
pub mod umbilical;
pub mod weapon_operation;

pub mod activity;
pub mod craft;
pub mod illumination_ui;
pub mod jamming_ui;
pub mod transit;
pub mod weapon_targets_view;

pub mod identity_ui;
pub mod protection_ui;
