//! Shared simulation contracts; domain state belongs to its owning branch.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod authoritative;
pub mod authority;
pub mod catalogue;
pub mod control;
pub mod debug_schema;
pub mod directive;
pub mod doctrine;
pub mod effect_queue;
pub mod flags;
pub mod fleet;
pub mod identity;
pub mod logging;
pub mod messages;
pub mod modifiers;
pub mod objective_utility;
pub mod presentation_contracts;
pub mod routes;
pub mod scene;
pub mod session_io;
pub mod sim_rng;
pub mod sim_sets;
pub mod sim_tick;
pub mod stance;
pub mod world_id;

pub mod lifecycle;
pub mod outcome;

pub mod power;
