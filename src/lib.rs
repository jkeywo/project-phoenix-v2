// Structural lints we allow at the crate level because the
// affected functions consume a fixed set of Bevy system parameters
// common in game-development patterns.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

// Declared early: the `plog!` family is `#[macro_export]`ed, and the helper
// macros they expand to must be defined before any module that uses them.
pub mod logging;

// Headless runner. Native only: it drives the app with a manual fixed-timestep
// loop, which has no meaning under requestAnimationFrame.
#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
pub mod headless;

// Performance measurement (issue #868). Not behind the headless feature: the
// asset inventory needs no simulation, and the browser collector runs in the
// shipped wasm build.
pub mod perf;

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
pub use entities::config_cache;
pub use weapons::blaster;
pub use weapons::pattern;
pub use weapons::phaser;
pub use weapons::torpedo;
pub mod ship;
pub use entities::config as entity_config;
pub use entities::entity_override;
pub use entities::include_resolve as entity_includes;
pub use entities::loader as entity_loader;
pub use entities::marker_validate;
pub use entities::model_rig;
pub use entities::planet as entity_planet;
pub use entities::spawner as entity_spawner;
pub use entities::star as entity_star;
pub use entities::target as entity_target;
pub use ship::damage;
pub mod objectives;
pub use core::balance;
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
pub mod command_admission;
pub mod server_app;
// Backward-compat alias: all `crate::simulation::*` imports continue to resolve.
pub use server_app as simulation;
pub mod audio_config;
/// Fixed-capacity history window (issue #788). Pure, Bevy-free, domain-neutral.
pub mod bounded_history;
/// Compile-time build flags readable at runtime (issue #939) — currently just
/// `PHOENIX_DEMO_BUILD`, which gates the host settings menu's Debug/Cheat tab.
pub mod build_flags;
/// Composite-key deterministic value derivation (issue #788). Pure, Bevy-free,
/// domain-neutral.
pub mod composite_rng;
pub mod console_bridge;
/// The loaded-content ledger (issue #935): every authored file the world/entity
/// loader actually reads, folded into `snapshot::content_digest` so an edit to
/// a hull, fragment, or sidecar moves a save's content version exactly as
/// reliably as an edit to the scenario TOML does. Compiles on both targets —
/// native and wasm converge on the same recording shape so the digest is
/// target-independent for identical bytes.
pub mod content_ledger;
/// The seeded cross-target determinism probe (issue #904): one minimal sim
/// world both a native test and the browser drive, under deliberately
/// different frame pacing, folding the canonical digest at shared ticks.
pub mod cross_target_probe;
pub mod radar;
pub mod radar_config;
pub mod ship_plugin;
/// The canonical authoritative-state digest (issue #901). At the crate root
/// rather than under `headless` since issue #904: a digest that only compiles
/// on native cannot make a native↔wasm claim. `headless::digest` aliases it.
pub mod sim_digest;
pub mod sim_rng;
pub mod sim_sets;
pub mod sim_tick;
/// Shared pure-Rust libm wrappers — the only sanctioned transcendental float
/// math in simulation code (issue #908; enforced via clippy.toml).
pub mod simmath;
/// Cross-target vector battery proving native and wasm agree, bit for bit,
/// on every `simmath` function (issue #909).
pub mod simmath_vectors;
/// The authoritative world snapshot (issue #862): phoenix's save *payload*,
/// stored inside `vellum-save`'s envelope. Compiles on both targets — a browser
/// host is the thing that saves.
pub mod snapshot;
pub mod world;
/// Deterministic tick-scoped world-id minting (issue #907) — the single
/// chokepoint every simulation entity, message and projectile id comes from.
pub mod world_id;
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
pub use console::weapons as weapons_plugin;
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

/// Shared 3D render setup (skybox, camera optics, ambient fill) — used by both
/// the game renderer and the standalone model viewer.
pub mod render_setup;

/// Standalone model/shader viewer (`viewer.html`), a dev tool built as its own
/// Trunk target. Not part of the game binary.
#[cfg(feature = "viewer")]
pub mod viewer;

// Generic GUI widget library — needed by the server viewscreen radar
// (ServerViewscreenRadarPlugin).
#[cfg(feature = "server")]
pub mod gui;
