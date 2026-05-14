pub mod beam_render;
pub mod impulse;
pub mod shield;
pub mod entity_tags;
pub mod region_effects;
pub mod region_shape;
pub mod asteroid_lifecycle;
pub mod phaser;
pub mod torpedo;
pub mod asteroid_spawner;
pub mod asteroid_window;
pub mod breakdown;
pub mod flag_kind;
pub mod config_cache;
pub mod damage;
pub mod entity_config;
pub mod entity_loader;
pub mod entity_override;
pub mod entity_spawner;
pub mod map_config;
pub mod messages;
pub mod stations;
pub mod modifiers;
pub mod power_system;
pub mod codec;
pub mod session;
pub mod ship_state;
pub mod ship_physics;
pub mod lobby_handler;
pub mod lobby;
pub mod simulation;
pub mod radar;
pub mod radar_config;
pub mod region_plugin;
pub mod repair_teams;
pub mod renderer;
pub mod client_lobby;
pub mod client_sim;
pub mod client_helm;
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
