//! Assembling the native windowed authoritative host (issue #1121).
//!
//! This is the fourth app builder in the repo, and it is a *thin* one on
//! purpose: everything shared with the other three comes from
//! [`crate::boot::build`] under [`BootProfile::NativeHost`], and the simulation
//! itself is [`add_simulation_plugins_with`], unchanged and in its unchanged
//! registration order (that order is a digest input — see
//! `server_app::registration`'s module note).
//!
//! Read it beside [`crate::headless::app::build_headless_app_with`]: the two
//! run the *same* sequence, and the differences are the whole content of this
//! issue.
//!
//! | headless | native host |
//! |---|---|
//! | `BootProfile::Headless`, render surrogate | `BootProfile::NativeHost`, real Bevy/wgpu through winit |
//! | `add_simulation_plugins_with(render: false)` | `render: true` — the viewscreen radar, reference grid, star/planet renderers, LOD swap and asset preloader |
//! | `TimeUpdateStrategy::ManualDuration` + a hand-rolled `while` loop | `App::run()` under winit, `WinitSettings` pinned to `Continuous` so an unfocused bridge machine keeps simulating |
//! | ship chosen by `--ship`/`--side-a` | ship chosen by `--ship`, defaulting to the world's first `available_ships` entry |
//! | template preload derived from the ship's directory | template preload over the whole content tree, done by the caller and required by [`crate::boot`] |
//! | nobody can ever connect | a [`NativeTransportPlugin`] seam a transport plugs into |
//!
//! # The deferred acceptance criterion
//!
//! Issue #1121's third acceptance criterion — browser clients joining the
//! native host through the same protocol, admission and projection contracts —
//! is **deferred to issue #1112**, the Phoenix WebRTC transport that replaces
//! PeerJS. It has to be: PeerJS is browser JavaScript, so it cannot run in a
//! native process at all, and #1121 is filed as blocked by #1112 for exactly
//! this reason. What is built here instead is the seam it plugs into — see
//! [`crate::native_host::transport`] — which is wired, ordered and gated
//! already, and exercised end to end by a loopback participant in
//! `tests/native_host_sim.rs`.

use bevy::prelude::*;

use crate::asteroids::lifecycle::AsteroidLifecyclePlugin;
use crate::boot::{BootError, BootPlan, BootProfile, NativeRenderSurface, WorldIngest};
use crate::core::messages::{GamePhase, ServerMessage};
use crate::entities::loader::TemplateLoader;
use crate::entities::template_preload::{preload_entity_templates, TemplatePreload};
use crate::lobby::{LobbyOutbox, LobbyPlugin, SelectedShipResource, Target};
use crate::logging::{LogFilterConfig, LoggingPlugin};
use crate::modifiers::coordination::ModifierCoordinationPlugin;
use crate::native_host::transport::NativeTransportPlugin;
use crate::server_app::{add_simulation_plugins_with, SimPluginOptions};
use crate::ship_plugin::PendingShipConfig;
use crate::sim_rng::{SeedSource, SimRng};
use crate::world::WorldPlugin;

/// The OS window caption.
///
/// Not a `strings.csv` id, and deliberately so: there is no Rust-side string
/// table in this repo (ids are resolved client-side by `gui/strings.js`), and
/// this is the window caption of an operator-launched process, on the same
/// footing as `phoenix-host`'s `HELP` text and its operator log. It is also a
/// proper noun rather than prose.
pub const WINDOW_TITLE: &str = "Project Phoenix";

/// Anything that stopped a native host being assembled.
#[derive(Debug)]
pub enum NativeHostError {
    /// The content tree could not be preloaded — see
    /// [`preload_entity_templates`]. Includes the deliberately loud "loaded
    /// nothing" case.
    Content(String),
    /// The model-marker contract is violated somewhere in the content tree
    /// (issue #758), so nothing may spawn.
    Markers(String),
    /// [`crate::boot::build`] refused.
    Boot(BootError),
    /// The world names no playable hull and none was given.
    NoShip(String),
    /// The chosen hull could not be read, parsed, or carries no `[[station]]`
    /// blocks.
    Ship(String),
}

impl std::fmt::Display for NativeHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeHostError::Content(m) => write!(f, "content preload: {m}"),
            NativeHostError::Markers(m) => write!(f, "{m}"),
            NativeHostError::Boot(e) => write!(f, "{e}"),
            NativeHostError::NoShip(m) => write!(f, "no playable hull: {m}"),
            NativeHostError::Ship(m) => write!(f, "ship: {m}"),
        }
    }
}

impl std::error::Error for NativeHostError {}

/// What to assemble.
///
/// Filled from `delivery::args::HostArgs` by the `phoenix-host` binary and by
/// hand in tests. Every path is authored the way a world TOML authors one —
/// repo-relative with forward slashes — and resolved against the process
/// working directory, which [`crate::native_host::pin_content_root`] has
/// already pinned to `--content-dir`.
pub struct NativeHostConfig {
    /// The root world TOML.
    pub world_path: String,
    /// The player's hull. `None` takes the world's first `available_ships`
    /// entry, which is what the browser's ship picker defaults to.
    pub ship_path: Option<String>,
    /// Overrides the world's `[global] seed`; both lose to nothing, and an
    /// absent pair draws from the OS.
    pub seed: Option<u64>,
    /// `plog!` filtering, already parsed.
    pub log: LogFilterConfig,
    /// The raw `--log` spec, forwarded to `LogPlugin`'s own `EnvFilter` so
    /// bevy-internal events roughly agree with our categories.
    pub log_spec: String,
    /// How this host presents — a winit window in production; see
    /// [`NativeRenderSurface`].
    pub surface: NativeRenderSurface,
    /// Start the mission with nobody connected, every station on `Backfill`.
    ///
    /// The native twin of headless's auto-start and of the host page's
    /// force-start button. Without it the host boots to the lobby and waits for
    /// participants to ready up, which is what a crewed session wants.
    pub solo: bool,
}

impl NativeHostConfig {
    /// A configuration for `world_path` with everything else defaulted: no
    /// hull override, no seed override, warn-level logging, no window, and a
    /// lobby that waits. The shape tests start from.
    pub fn new(world_path: impl Into<String>) -> Self {
        Self {
            world_path: world_path.into(),
            ship_path: None,
            seed: None,
            log: LogFilterConfig::default(),
            log_spec: String::new(),
            surface: NativeRenderSurface::Contract,
            solo: false,
        }
    }
}

/// Populate the native entity-template cache for a content tree.
///
/// The strict walk — [`preload_entity_templates`] over `<content_dir>/assets/
/// entities` — not `delivery::serve::preload_templates`. Both write the same
/// process-global cache and a host must call **one** of them, once; the strict
/// one is the one a simulation needs (sorted, marker-validating, and it refuses
/// to succeed having cached nothing), and its side effect of enriching the
/// published catalogue is exactly what the delivery walk was there for.
///
/// `content_dir` is expected to be the process working directory — in practice
/// `"."`, because [`crate::native_host::pin_content_root`] has already made it
/// so. That is not incidental: the cache is keyed by the path a *world*
/// authors (`assets/entities/…`), so a root spelled any other way would key
/// entries nothing ever looks up.
///
/// **Process-global.** Callers belong in a binary or an integration test.
pub fn preload_content_templates(content_dir: &str) -> Result<TemplatePreload, NativeHostError> {
    let root = content_dir.trim_end_matches(['/', '\\']);
    let dir = if root.is_empty() || root == "." {
        "assets/entities".to_string()
    } else {
        format!("{root}/assets/entities")
    };
    preload_entity_templates(&dir).map_err(NativeHostError::Content)
}

/// Assemble the native host's `App`. Does not run it — see [`run`].
///
/// `preload` is the receipt from [`preload_content_templates`], taken by
/// reference rather than performed here so that the caller (`phoenix-host`)
/// can populate the cache *before* it binds its HTTP listener and builds the
/// published catalogue from the same templates. It cannot be forged: boot
/// re-checks the cache against the world's declared set regardless, and refuses
/// with [`BootError::NativeTemplatesMissing`] on a miss.
pub fn build_native_host_app(
    cfg: &NativeHostConfig,
    preload: &TemplatePreload,
) -> Result<App, NativeHostError> {
    // The model-marker contract gate (issue #758), before an `App` exists. An
    // unresolved marker attaches a beam, exhaust plume or camera to the ship's
    // centre and produces a plausible-looking mission with the wrong numbers —
    // exactly the class of silent failure a windowed host is worst at showing.
    preload.marker_gate().map_err(NativeHostError::Markers)?;

    // The boot seam (issue #1217, fourth profile #1121). It composes the plugin
    // core and the real render stack, and owns the whole world-ingestion order:
    // Rhai hashing seed → content-ledger reset → read/validate/compile →
    // abort on a broken world (NativeHost is authoritative, like headless) →
    // the native template-cache check → ledger apply + eager record → freeze →
    // insert the `WorldConfig` and its `PreCompiledScripts`.
    let plan = BootPlan {
        profile: BootProfile::NativeHost,
        world_ingest: WorldIngest::FromReader,
        // Keep bevy-internal events quiet by default; a `--log` spec is folded
        // in after the `warn` floor, exactly as headless does it.
        log_filter: if cfg.log_spec.is_empty() {
            "warn".to_string()
        } else {
            format!("warn,{}", cfg.log_spec)
        },
        world_path: cfg.world_path.clone(),
        reader: Box::new(crate::world::load::FsReader),
        script_resolver: Box::new(crate::entities::config_cache::production_script_resolver()),
        // A rendered host is not reproduced tick-for-tick, so it keeps the
        // multithreaded pool — the same answer both browser profiles give.
        single_threaded: false,
        raw_transform: None,
        native_surface: cfg.surface,
    };
    let mut app = crate::boot::build(plan).map_err(NativeHostError::Boot)?;

    // Everything the preload gathered before a `tracing` subscriber existed.
    // Emitted HERE, after boot's `LogPlugin::build` installed one — before it,
    // every line goes nowhere.
    preload.report();

    // Crate-side `plog!` filtering, separate from boot's bevy `LogPlugin`.
    app.insert_resource(cfg.log.clone())
        .add_plugins(LoggingPlugin);

    // Seed precedence: `--seed`, then the world's `[global] seed`, then the OS.
    // Read back from the `WorldConfig` boot parsed and inserted — the first
    // point at which both the config and the parsed world are in scope.
    // Inserted into the app AFTER `add_simulation_plugins_with`'s
    // `init_resource` below, so it overrides the OS-seeded default.
    let world_config = app
        .world()
        .resource::<crate::world::config::WorldConfig>()
        .clone();
    let sim_rng = match (cfg.seed, world_config.global.seed) {
        (Some(seed), _) => SimRng::new(seed, SeedSource::Cli),
        (None, Some(seed)) => SimRng::new(seed, SeedSource::World),
        (None, None) => SimRng::random(),
    };

    // The hull. `--ship` wins; otherwise the world's first `available_ships`
    // entry, which is what the browser's ship picker pre-selects.
    let ship_path = match &cfg.ship_path {
        Some(path) => path.clone(),
        None => world_config
            .available_ships
            .first()
            .map(|s| s.template_path.clone())
            .ok_or_else(|| {
                NativeHostError::NoShip(format!(
                    "{} authors no [[available_ships]] and no --ship was given",
                    cfg.world_path
                ))
            })?,
    };

    // Ship config, BEFORE `LobbyPlugin`: the native twin of
    // `wasm_validate_stations`. Without it `update_session_with_config` falls
    // back to `load_ship_config_from_disk`, which returns the *battleship*
    // roster regardless of the hull chosen — so every station, and therefore
    // every backfilled AI system, would belong to the wrong ship.
    let ship_entity_config = crate::entities::include_resolve::load_entity_config(&ship_path)
        .map_err(|e| NativeHostError::Ship(format!("{ship_path:?} failed to parse: {e}")))?;
    // Issue #935: the player's own hull is authored content too and need not be
    // among the world's declared entities, so re-record and re-freeze. The
    // ledger fold is path-sorted and order-independent, so the frozen digest is
    // byte-identical whichever route the hull rode in on.
    let _ = crate::entities::loader::FsTemplateLoader.load_template(&ship_path);
    crate::content_ledger::freeze();
    let ship_config = ship_entity_config.ship_config.ok_or_else(|| {
        NativeHostError::Ship(format!("{ship_path:?} has no [[station]] blocks"))
    })?;
    app.insert_resource(PendingShipConfig(ship_config));
    app.insert_resource(SelectedShipResource(ship_path.clone()));

    // `ConfigCachePlugin` is wasm-only; its two jobs are the template cache
    // (done by the preload) and the faction registry, which
    // `add_simulation_plugins` already inserts from the `include_str!`ed native
    // registry.
    app.add_plugins(AsteroidLifecyclePlugin)
        .add_plugins(ModifierCoordinationPlugin)
        .add_plugins(LobbyPlugin)
        .add_plugins(crate::lobby::lobby_outbox_broadcaster());

    // `render: true` — the same option the browser host takes, and the reason
    // this host draws anything: the star and planet renderers, the procedural
    // mesh cache and LOD swap, the viewscreen radar, the reference grid and the
    // asset preloader that gates `Loading → InProgress`. Every type it adds is
    // declared `Presentation`/`DeferredFold` in the authoritative census, which
    // is the claim `tests/native_host_sim.rs` pins by digest against a headless
    // run of the same world and seed.
    add_simulation_plugins_with(
        &mut app,
        SimPluginOptions {
            render: true,
            ..Default::default()
        },
    );
    // After the plugins, so it overrides their OS-seeded `init_resource`.
    app.insert_resource(sim_rng);
    app.add_plugins(WorldPlugin);

    // The transport seam. Installed unconditionally and inert until something
    // inserts a `NativeTransportLink` — see the module docs for why it is here
    // before the transport is.
    app.add_plugins(NativeTransportPlugin);

    // An unfocused bridge machine must keep simulating: the browser host
    // inserts exactly this, and a native window's default is to throttle when
    // it loses focus.
    if cfg.surface == NativeRenderSurface::Window {
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        });
    }

    if cfg.solo {
        app.add_systems(
            FixedUpdate,
            solo_auto_start.before(crate::sim_sets::SimSet::Input),
        );
    }

    Ok(app)
}

/// Start the mission with nobody connected.
///
/// Byte-for-byte the policy `headless::app::headless_auto_start` applies, and
/// for the same reason: going straight to `InProgress` with an empty `Sessions`
/// is what makes the player ship AI-driven, because
/// `spawn_game_start_entities` assigns `BACKFILL_RATING` to every station not
/// in the manned set and with no sessions that set is empty.
///
/// **Registered in `FixedUpdate`, not `PreUpdate` (issue #907).** A
/// `NextState<GamePhase>` write from `PreUpdate` applies at the FRAME-level
/// `StateTransition`, so `OnEnter(GamePhase::InProgress)` — and the player-ship
/// mint inside it — would fire at a point whose relationship to `SimTick`
/// depends on frame pacing. `FixedUpdate` puts the write on the same
/// tick-scoped transition site every other phase writer uses.
///
/// It deliberately does NOT wait for the asset preloader the way the host
/// page's force-start does. A solo run is the AI flying the mission; the
/// viewscreen catches up as models stream in, and gating the simulation on a
/// GPU upload would make the run's tick sequence depend on disk speed.
fn solo_auto_start(
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

/// Run the host until the window closes.
///
/// `App::run()` blocks on native, unlike in the browser where it hands control
/// back to `requestAnimationFrame` — and winit owns the main thread on Windows.
/// Everything else a native host does (the delivery HTTP listener, a transport)
/// therefore lives on other threads, and this call is the last thing the
/// binary's `main` does.
pub fn run(mut app: App) {
    app.run();
}
