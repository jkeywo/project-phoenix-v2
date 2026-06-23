// Structural lints we allow at the crate level because the
// affected functions consume a fixed set of Bevy system parameters
// common in game-development patterns.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod ai;
pub mod core;
pub use ai::core as ai_core;
pub use ai::faction;
pub use ai::server as ai_plugin;
pub mod weapons;
pub use weapons::beam_render;
pub mod console_ai;
pub use ship::boost;
pub use ship::config as ship_config;
pub use ship::control_source;
pub use ship::impulse;
pub use weapons::shield;
pub mod regions;
pub use regions::effects as region_effects;
pub use regions::server as region_plugin;
pub use regions::shape as region_shape;
pub mod entities;
pub use entities::tags as entity_tags;
pub mod asteroids;
pub use asteroids::lifecycle as asteroid_lifecycle;
pub use asteroids::spawner as asteroid_spawner;
pub use asteroids::window as asteroid_window;
pub use console_ai::core as console_ai_core;
pub use console_ai::server as console_ai_plugin;
pub use core::flag_kind;
pub use entities::config_cache;
pub use weapons::phaser;
pub use weapons::torpedo;
pub mod ship;
pub use entities::config as entity_config;
pub use entities::entity_override;
pub use entities::loader as entity_loader;
pub use entities::model_rig;
pub use entities::spawner as entity_spawner;
pub use entities::star as entity_star;
pub use entities::target as entity_target;
pub use ship::damage;
pub mod objectives;
pub use core::messages;
pub mod lobby;
pub use lobby::stations_config;
pub use lobby::stations_policy;
pub mod modifiers;
pub use core::codec;
pub use lobby::handler as lobby_handler;
pub use lobby::session;
pub use modifiers::coordination as modifier_coordination;
pub use modifiers::power_system;
pub use ship::physics as ship_physics;
pub use ship::state as ship_state;
pub use ship::system_registry;
pub mod server_app;
// Backward-compat alias: all `crate::simulation::*` imports continue to resolve.
pub use server_app as simulation;
pub mod console_bridge;
pub mod radar;
pub mod radar_config;
pub mod ship_plugin;
pub mod sim_sets;
pub mod world;
pub use modifiers::repair_teams;
pub mod comms;

// ── Console module ─────────────────────────────────────────────────────────

pub mod console;

// Backwards-compat aliases so old paths still resolve.
pub use console::captain::server as captain_plugin;
pub use console::comms::inbox as comms_inbox;
pub use console::comms::server as comms_plugin;
pub use console::helm::server as helm_plugin;
pub use console::navigation as navigation_plugin;
pub use console::repair::server as repair_plugin;
pub use console::weapons::server as weapons_plugin;
pub use ship::power as power_plugin;
pub use ship::sensors as sensors_plugin;
pub use ship::shields as shields_plugin;

// Server-only grouped module (bridge, renderer, viewscreen_border, debug_overlay).
#[cfg(feature = "server")]
pub mod server;

// Backwards-compat re-exports so any code using the old top-level names still
// compiles without modification.
#[cfg(feature = "server")]
pub use server::bridge;

#[cfg(feature = "server")]
pub mod renderer {
    pub use crate::server::renderer::*;
}

#[cfg(feature = "server")]
pub mod viewscreen_border {
    pub use crate::server::viewscreen_border::*;
}

pub mod debug_overlay;

// Generic GUI widget library — needed by the server viewscreen radar
// (ServerViewscreenRadarPlugin).
#[cfg(feature = "server")]
pub mod gui;
