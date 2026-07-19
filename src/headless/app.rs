//! Builds and drives the headless Bevy app.
//!
//! Mirrors `wasm_init` in `src/server/bridge.rs`, minus everything that needs a
//! window, a GPU, or a JS host. The plugin list is derived from that function's
//! `is_automation` branch, which is the already-proven inventory of what the
//! simulation needs when `RenderPlugin` is absent.

use bevy::app::{PanicHandlerPlugin, TaskPoolPlugin};
use bevy::asset::{AssetApp, AssetPlugin};
use bevy::diagnostic::{DiagnosticsPlugin, FrameCountPlugin};
use bevy::image::Image;
use bevy::log::LogPlugin;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::scene::ScenePlugin;
use bevy::shader::{Shader, ShaderLoader};
use bevy::state::app::StatesPlugin;
use bevy::time::{TimePlugin, TimeUpdateStrategy};
use bevy::transform::TransformPlugin;
use bevy_rapier3d::plugin::TimestepMode;

use crate::asteroid_lifecycle::AsteroidLifecyclePlugin;
use crate::console_bridge::{AiChatterEvent, HudStateChanged, LobbyStateChanged};
use crate::entity_config::EntityConfig;
use crate::lobby::{LobbyOutbox, LobbyPlugin, SelectedShipResource, Target};
use crate::logging::LoggingPlugin;
use crate::messages::{GamePhase, ServerMessage};
use crate::modifier_coordination::ModifierCoordinationPlugin;
use crate::server_app::{add_simulation_plugins_with, SimPluginOptions};
use crate::ship_plugin::PendingShipConfig;
use crate::world::WorldPlugin;

use super::args::HeadlessArgs;

/// Anything that stopped the app being built.
#[derive(Debug)]
pub struct BuildError(pub String);

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BuildError {}

fn read_toml(path: &str, what: &str) -> Result<String, BuildError> {
    std::fs::read_to_string(path)
        .map_err(|e| BuildError(format!("could not read {what} {path:?}: {e}")))
}

/// Load every `assets/entities/*.toml` into the native template cache.
///
/// The browser fills this cache from a JS-driven preload before the app starts.
/// Several simulation paths (`asteroids::lifecycle`, the spawn helpers in
/// `server_app`, `world::server::setup_world`) read the cache with no
/// filesystem fallback of their own, so headless has to do the equivalent up
/// front or those paths quietly find nothing.
///
/// Templates that fail to parse are reported and skipped rather than aborting
/// the run — `assets/entities/` holds a lot of files and one bad cosmetic
/// asteroid should not stop a combat test.
fn preload_entity_templates(dir: &str) -> Result<usize, BuildError> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| BuildError(format!("could not list {dir:?}: {e}")))?;
    let mut loaded = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        // Key on the repo-relative path the world TOML uses, with forward
        // slashes — on Windows `Path::display` would emit backslashes and every
        // lookup would miss.
        let key = format!(
            "{}/{}",
            dir.trim_end_matches('/'),
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let Ok(toml) = std::fs::read_to_string(&path) else {
            warn!(target: "config", "template unreadable, skipping: {key}");
            continue;
        };
        match EntityConfig::from_toml(&toml) {
            Ok(cfg) => {
                crate::config_cache::insert_native_config(key, cfg);
                loaded += 1;
            }
            Err(e) => warn!(target: "config", "template failed to parse, skipping: {key}: {e}"),
        }
    }
    Ok(loaded)
}

/// Assemble the headless app. Does not run it — see [`run`].
pub fn build_headless_app(args: &HeadlessArgs) -> Result<App, BuildError> {
    // Templates first: `update_session_with_config` reads the cache during
    // `Startup`, so it has to be populated before the app is built.
    let template_dir = std::path::Path::new(&args.ship_path)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "assets/entities".to_string());
    let loaded = preload_entity_templates(&template_dir)?;

    let mut app = App::new();

    app.add_plugins((
        PanicHandlerPlugin,
        LogPlugin {
            // Our own categories gate the `plog!` call sites; this filter only
            // governs bevy-internal events. Keep it quiet by default so the
            // report is the loudest thing on stdout.
            filter: if args.log_spec.is_empty() {
                "warn".to_string()
            } else {
                format!("warn,{}", args.log_spec)
            },
            ..default()
        },
        // Single-threaded when reproducibility is asked for: with the default
        // multithreaded executor, system *execution* order varies run to run
        // even though the schedule graph is fixed.
        //
        // This is necessary but not sufficient for reproducibility — damage
        // distribution and region effects still call
        // `SmallRng::from_os_rng()`. Runs that take damage may diverge.
        if args.deterministic {
            TaskPoolPlugin {
                task_pool_options: bevy::app::TaskPoolOptions::with_num_threads(1),
            }
        } else {
            TaskPoolPlugin::default()
        },
        FrameCountPlugin,
        TimePlugin,
        TransformPlugin,
        DiagnosticsPlugin,
        AssetPlugin::default(),
        ScenePlugin,
        StatesPlugin,
    ));

    // Asset types and messages that `RenderPlugin` / `ViewscreenBorderPlugin`
    // would otherwise register. Simulation systems name these types even when
    // nothing is drawn, so they have to exist. Kept in step with the
    // `is_automation` branch of `wasm_init`.
    app.init_asset::<Shader>()
        .init_asset_loader::<ShaderLoader>()
        .init_asset::<Image>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .add_message::<HudStateChanged>()
        .add_message::<LobbyStateChanged>()
        .add_message::<AiChatterEvent>();

    app.insert_resource(args.log.clone())
        .add_plugins(LoggingPlugin);

    // World config. `insert_world_config_resource` (a `Startup` system in
    // `WorldPlugin`) sources this from the JS bridge, which has no native
    // equivalent — it no-ops off-browser. Inserting it here pre-empts that.
    let world_toml = read_toml(&args.world_path, "world")?;
    let world_config = crate::world::config::parse_world(&world_toml)
        .map_err(|e| BuildError(format!("world {:?} failed to parse: {e}", args.world_path)))?;
    app.insert_resource(world_config);

    // Ship config, before `LobbyPlugin`: the native twin of
    // `wasm_validate_stations`. Without it `update_session_with_config` falls
    // back to `load_ship_config_from_disk`, which returns the *battleship*
    // roster regardless of `--ship` — so every station, and therefore every
    // backfilled AI system, would belong to the wrong hull.
    let ship_toml = read_toml(&args.ship_path, "ship")?;
    let ship_entity_config = EntityConfig::from_toml(&ship_toml)
        .map_err(|e| BuildError(format!("ship {:?} failed to parse: {e}", args.ship_path)))?;
    let ship_config = ship_entity_config.ship_config.ok_or_else(|| {
        BuildError(format!(
            "ship {:?} has no [[station]] blocks",
            args.ship_path
        ))
    })?;
    app.insert_resource(PendingShipConfig(ship_config));
    app.insert_resource(SelectedShipResource(args.ship_path.clone()));

    // `ConfigCachePlugin` is wasm-only; its two jobs are the template cache
    // (done above) and the faction registry, which `add_simulation_plugins`
    // already inserts from the `include_str!`ed native registry.
    app.add_plugins(AsteroidLifecyclePlugin)
        .add_plugins(ModifierCoordinationPlugin)
        .add_plugins(LobbyPlugin)
        .add_plugins(crate::lobby::lobby_outbox_broadcaster());

    add_simulation_plugins_with(&mut app, SimPluginOptions { render: false });
    app.add_plugins(WorldPlugin);

    // Fixed timestep. `ManualDuration` makes every `Time` clock advance by
    // exactly `dt` per `update()` regardless of wall clock, which is what makes
    // the run rate-independent; rapier needs telling separately, since its
    // default `TimestepMode::Variable` would otherwise reintroduce the coupling.
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(args.dt),
    ));
    app.insert_resource(TimestepMode::Fixed {
        dt: args.dt as f32,
        substeps: 1,
    });

    app.add_systems(PreUpdate, headless_auto_start);

    // Telemetry. `count_tick` in `First` and `collect_outbound` in `Last` so
    // every message is attributed to the tick that produced it.
    app.insert_resource(super::report::RunTelemetry {
        capture_stream: args.report_format == super::args::ReportFormat::Ndjson,
        ..Default::default()
    })
    .add_systems(First, super::report::count_tick)
    .add_systems(Last, super::report::collect_outbound);

    info!(
        target: "config",
        "headless: world={} ship={} templates={} dt={:.5}s ({:.1} Hz) ticks={}",
        args.world_path, args.ship_path, loaded, args.dt, args.hz(), args.max_ticks
    );

    Ok(app)
}

/// Start the game with nobody connected.
///
/// The native twin of `drain_force_start` in `bridge.rs`, which is wasm-gated
/// and reads a JS thread-local. It skips that function's asset-preload check
/// because headless never registers the preloader (it lives behind
/// `SimPluginOptions::render`).
///
/// Going straight to `InProgress` with an empty `Sessions` is exactly what
/// makes the player ship AI-driven: `spawn_game_start_entities` assigns
/// `BACKFILL_RATING` to every station not in the manned set, and with no
/// sessions that set is empty.
fn headless_auto_start(
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut started: Local<bool>,
) {
    if *started || state.get() != &GamePhase::Lobby {
        return;
    }
    next_state.set(GamePhase::InProgress);
    outbox.0.push((Target::All, ServerMessage::GameStarted));
    *started = true;
}

/// Pump the app for `args.max_ticks` fixed steps.
///
/// Deliberately not `App::run()`: with no `WinitPlugin` and no
/// `ScheduleRunnerPlugin` the default runner calls `update()` exactly once.
/// Driving the loop by hand also gives the tick budget and the exit condition
/// for free.
pub fn run(app: &mut App, max_ticks: u64) -> u64 {
    app.finish();
    app.cleanup();
    let mut ticks = 0;
    while ticks < max_ticks {
        app.update();
        ticks += 1;
        if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
            break;
        }
    }
    ticks
}
