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
pub mod stations;
pub mod modifiers;
pub use modifiers::coordination as modifier_coordination;
pub mod ship_view;
pub use modifiers::power_system;
pub use core::codec;
pub use lobby::session;
pub use ship::state as ship_state;
pub use ship::physics as ship_physics;
pub use lobby::handler as lobby_handler;
pub mod simulation;
pub mod world;
pub mod radar;
pub mod radar_config;
pub use modifiers::repair_teams;
pub mod renderer;
pub use lobby::client_panel as client_lobby;
pub mod client_sim;
pub mod client_helm;
pub mod client_comms;
pub mod comms_plugin;
pub mod client_complexity;
pub mod client_elements;
pub mod client_app;

#[cfg(feature = "server")]
pub mod bridge;

#[cfg(feature = "server")]
pub mod viewscreen_border;

#[cfg(feature = "server")]
pub mod debug_overlay;

#[cfg(feature = "client")]
pub mod client_bridge;

#[cfg(feature = "client")]
pub mod phone_border;
