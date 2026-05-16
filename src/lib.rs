pub mod core;
pub mod ai;
pub use ai::core as ai_core;
pub use ai::server as ai_plugin;
pub mod captain_plugin;
pub use ai::faction;
pub mod weapons;
pub use weapons::beam_render;
pub mod console_ai;
pub use console_ai::delegation;
pub use ship::impulse;
pub use weapons::shield;
pub mod regions;
pub use regions::effects as region_effects;
pub use regions::shape as region_shape;
pub use regions::server as region_plugin;
pub mod entities;
pub use entities::tags as entity_tags;
pub mod asteroids;
pub use asteroids::lifecycle as asteroid_lifecycle;
pub use asteroids::spawner as asteroid_spawner;
pub use asteroids::window as asteroid_window;
pub use weapons::phaser;
pub use weapons::torpedo;
pub use modifiers::breakdown;
pub use core::flag_kind;
pub use console_ai::complexity;
pub use console_ai::core as console_ai_core;
pub use console_ai::server as console_ai_plugin;
pub use entities::config_cache;
pub mod ship;
pub use ship::damage;
pub use entities::config as entity_config;
pub use entities::loader as entity_loader;
pub use entities::entity_override;
pub use entities::spawner as entity_spawner;
pub use entities::map_config;
pub mod objectives;
pub mod comms_inbox;
pub use core::messages;
pub mod lobby;
pub use lobby::stations_config;
pub use lobby::stations_policy;
pub mod modifiers;
pub use modifiers::coordination as modifier_coordination;
pub mod ship_view;
pub use modifiers::power_system;
pub use core::codec;
pub use lobby::session;
pub use ship::state as ship_state;
pub use ship::physics as ship_physics;
pub use lobby::handler as lobby_handler;
pub mod server_app;
// Backward-compat alias: all `crate::simulation::*` imports continue to resolve.
pub use server_app as simulation;
pub mod ship_plugin;
pub mod weapons_plugin;
pub mod repair_plugin;
pub mod power_plugin;
pub mod science_plugin;
pub mod world;
pub mod radar;
pub mod radar_config;
pub use modifiers::repair_teams;
pub use lobby::client_panel as client_lobby;
pub mod client_sim;
pub mod client_comms;
pub mod client_complexity;

// ── New folder layout ────────────────────────────────────────────────────────

pub mod console;

// Keep flat-file modules in place for now — callers still use their old paths.
// New code should import from console::helm::joystick directly.
pub mod client_helm;

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

#[cfg(feature = "server")]
pub mod debug_overlay {
    pub use crate::server::debug_overlay::*;
}

// Client-only grouped module (app, bridge, elements, phone_border).
#[cfg(feature = "client")]
pub mod helm_panel;

#[cfg(feature = "client")]
pub mod weapons_panel;

#[cfg(feature = "client")]
pub mod repair_panel;

#[cfg(feature = "client")]
pub mod power_panel;

#[cfg(feature = "client")]
pub mod science_panel;

#[cfg(feature = "client")]
pub mod comms_panel;

#[cfg(feature = "client")]
pub mod client;

// Backwards-compat flat modules for client.
#[cfg(feature = "client")]
pub mod client_app {
    pub use crate::client::app::*;
}

#[cfg(feature = "client")]
pub mod client_bridge {
    pub use crate::client::bridge::*;
}

#[cfg(feature = "client")]
pub mod client_elements {
    pub use crate::client::elements::*;
}

#[cfg(feature = "client")]
pub mod phone_border {
    pub use crate::client::phone_border::*;
}
