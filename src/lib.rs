pub mod core;
pub mod ai;
pub mod ai_plugin;
pub mod captain_plugin;
pub mod faction;
pub mod weapons;
pub use weapons::beam_render;
pub mod delegation;
pub use ship::impulse;
pub use weapons::shield;
pub mod entity_tags;
pub mod region_effects;
pub mod region_shape;
pub mod asteroid_lifecycle;
pub use weapons::phaser;
pub use weapons::torpedo;
pub mod asteroid_spawner;
pub mod asteroid_window;
pub mod breakdown;
pub mod flag_kind;
pub mod complexity;
pub mod console_ai;
pub mod console_ai_plugin;
pub mod config_cache;
pub mod ship;
pub use ship::damage;
pub mod entity_config;
pub mod entity_loader;
pub mod entity_override;
pub mod entity_spawner;
pub mod map_config;
pub mod objectives;
pub mod comms_inbox;
pub mod messages;
pub mod stations_config;
pub mod stations_policy;
pub mod stations;
pub mod modifiers;
pub mod modifier_coordination;
pub mod ship_view;
pub mod power_system;
pub mod codec;
pub mod session;
pub use ship::state as ship_state;
pub use ship::physics as ship_physics;
pub mod lobby_handler;
pub mod lobby;
pub mod simulation;
pub mod world;
pub mod radar;
pub mod radar_config;
pub mod region_plugin;
pub mod repair_teams;
pub mod renderer;
pub mod client_lobby;
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
