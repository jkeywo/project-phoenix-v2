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
    /// participants to ready up, which is what a crewed session wants — but
    /// **no participant can arrive yet**: the transport is issue #1112's, and
    /// there is no native force-start. [`build_native_host_app`] therefore warns
    /// loudly when this is `false`, rather than refusing a mode that becomes
    /// correct the moment #1112 lands.
    pub solo: bool,
    /// The hulls a restricting `--manifest` publishes for this world (issue
    /// #917's curated allowlist), or empty when the catalogue is unrestricted.
    ///
    /// Only the DEFAULT hull consults it: an explicit `--ship` still wins, the
    /// way an explicit `?ship=` does in the browser. Its job is to stop one
    /// process publishing a curated catalogue over HTTP and simultaneously
    /// flying something that catalogue excludes.
    pub curated_ships: Vec<String>,
    /// Pin Bevy's task pool to one thread, so this host's system execution order
    /// is fixed run to run — [`BootPlan::single_threaded`].
    ///
    /// `false` for a shipped host: a rendered viewscreen is not reproduced
    /// tick-for-tick and the multithreaded pool is worth having. `true` is for a
    /// digest comparison, where an unpinned executor makes the claim a race
    /// rather than a measurement (see `tests/native_headless_digest.rs`).
    pub deterministic: bool,
    /// Local Station panes, already opened and published (issue #1122).
    ///
    /// Opened by the caller rather than here, because a pane's document is
    /// published at the delivery listener's own address and a `:0` bind does not
    /// know its port until it has bound. What this builder does with them is
    /// install the transport — a pane is a participant, so its traffic enters
    /// through the same `NativeTransportPlugin` seam issue #1121 built and is
    /// subject to the same reserved-token refusal.
    pub panes: Option<crate::native_host::panes::LocalPanes>,
    /// A validated bridge-display profile (issue #1123), or `None` for the
    /// single-window #1121 behaviour.
    ///
    /// Already **validated** by the caller — a bad profile (an unknown role, a
    /// Station with three panes) fails at the prompt, so by the time it reaches
    /// here its roles and density are sound and the density rule need not be
    /// re-checked. When present, [`build_native_host_app`] installs
    /// [`BridgeDisplayPlugin`](crate::native_host::bridge_display::BridgeDisplayPlugin),
    /// which opens one borderless-fullscreen surface per configured monitor:
    /// the viewscreen on the primary window, each Station on its own window.
    pub bridge_profile: Option<crate::native_host::bridge_profile::ValidatedProfile>,
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
            curated_ships: Vec::new(),
            deterministic: false,
            panes: None,
            bridge_profile: None,
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

/// The hulls a scenario manifest publishes for `world_path` — issue #917's
/// curated allowlist, read for the process's OWN default hull.
///
/// `phoenix-host --manifest assets/scenarios.demo.toml` restricts what this
/// process publishes over HTTP; without this it did not restrict what the same
/// process flies, so one host could serve a curated catalogue and simultaneously
/// run a hull that catalogue excludes. The browser has no such gap: its picker
/// is built from the curated list.
///
/// Empty means **unrestricted**, and it is the answer for all three of "the
/// manifest curates nothing", "the manifest does not publish this world at all"
/// and "the manifest does not parse". That is deliberate: this is a narrowing
/// of the default hull, not a second gate on the world — refusing a `--world`
/// the catalogue happens not to list would break the ordinary dev invocation,
/// where the world is named directly and the manifest is beside the point.
pub fn curated_hulls_for_world(manifest_toml: &str, world_path: &str) -> Vec<String> {
    let wanted = crate::entities::include_resolve::canonical_template_path(world_path);
    let Ok(manifest) = crate::world::manifest::parse_manifest(manifest_toml) else {
        return Vec::new();
    };
    manifest
        .scenarios
        .iter()
        .find(|s| crate::entities::include_resolve::canonical_template_path(&s.world) == wanted)
        .map(|s| s.ships.clone())
        .unwrap_or_default()
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
        // A shipped rendered host is not reproduced tick-for-tick, so it keeps
        // the multithreaded pool — the same answer both browser profiles give.
        // A digest comparison asks for the pinned executor instead; boot
        // honours it on every native composition path, wgpu included.
        single_threaded: cfg.deterministic,
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
    // entry the published catalogue still offers, which is what the browser's
    // ship picker pre-selects — `entity_template_paths` filters that same list
    // by the same allowlist, in the world's own authored order.
    //
    // Consulting `curated_ships` here is what stops one process publishing a
    // curated catalogue over HTTP (`--manifest assets/scenarios.demo.toml`,
    // issue #917's native half) and simultaneously flying a hull that catalogue
    // excludes.
    let ship_path = match &cfg.ship_path {
        Some(path) => path.clone(),
        None => world_config
            .available_ships
            .iter()
            .find(|s| {
                cfg.curated_ships.is_empty()
                    || cfg.curated_ships.iter().any(|c| c == &s.template_path)
            })
            .map(|s| s.template_path.clone())
            .ok_or_else(|| {
                NativeHostError::NoShip(if cfg.curated_ships.is_empty() {
                    format!(
                        "{} authors no [[available_ships]] and no --ship was given",
                        cfg.world_path
                    )
                } else {
                    format!(
                        "{} authors no [[available_ships]] the manifest's curated hull list \
                         admits ({}), and no --ship was given",
                        cfg.world_path,
                        cfg.curated_ships.join(", ")
                    )
                })
            })?,
    };

    // The hull's own half of boot's template-cache gate (issue #1121).
    //
    // `check_native_templates` sees the world's DECLARED set, and an explicit
    // `--ship` need not be in it: issue #935 made the player's hull authored
    // content that may sit outside `available_ships` entirely. So a `--ship`
    // pointing at a template the preload never cached would walk straight past
    // that gate, and `lobby::server::update_session_with_config` — which looks
    // this exact path up in the cache with NO filesystem fallback — would keep
    // a DEFAULT `ShipClientConfig`: default helm radar range, default
    // impulse-charge duration, default hostile-arc colour, a plausible mission,
    // a clean log. The refusal below is the same refusal boot makes, sited
    // where the resolved hull is finally known.
    let ship_key = crate::entities::include_resolve::canonical_template_path(&ship_path);
    if crate::entities::config_cache::get_cached_entity_config(&ship_key).is_none() {
        return Err(NativeHostError::Ship(format!(
            "{ship_key} is not in the native entity-template cache, so the host would \
             read a Default hull configuration — helm radar range, impulse-charge \
             duration and hostile-arc colour — instead of this hull's authored one. \
             Check --content-dir and that the hull lives under <content-dir>/assets/entities"
        )));
    }

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
    let ship_config = ship_entity_config
        .ship_config
        .ok_or_else(|| NativeHostError::Ship(format!("{ship_path:?} has no [[station]] blocks")))?;
    app.insert_resource(PendingShipConfig(ship_config));
    // Store the CANONICAL key, not the raw `--ship` string: every downstream
    // reader (`lobby::server::update_session_with_config`, `server::radar`,
    // `server::reference_grid`, `server_app::world_setup`) looks this path up
    // in the native template cache with NO filesystem fallback, and that cache
    // is keyed canonically. The gate two lines above already canonicalises
    // before checking, so a raw string here would let a `--ship` spelled with
    // `./` or Windows backslashes pass the gate and then miss every one of
    // those lookups, silently keeping a Default `ShipClientConfig`.
    app.insert_resource(SelectedShipResource(ship_key.clone()));

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

    // Local Station panes (issue #1122). They are participants, so the transport
    // they get is the ORDINARY seam above — not a side door. Everything a pane
    // says enters through `Messages<InboundMessage>` and past the reserved-token
    // refusal; everything it hears is a `Target` the broadcaster resolved
    // through `SessionManager::holder_for_station`.
    //
    // A pane host with panes is therefore also a host that HAS a transport,
    // which is why the `--solo` warning below asks whether one is installed
    // rather than assuming nobody can ever connect.
    if let Some(panes) = &cfg.panes {
        app.insert_resource(crate::native_host::transport::NativeTransportLink::new(
            panes.bus.transport(),
        ));
        app.insert_resource(crate::native_host::panes::PaneBusResource(
            panes.bus.clone(),
        ));
        #[cfg(feature = "ultralight")]
        {
            app.insert_resource(crate::native_host::panes::ultralight::PaneDisplayConfig {
                panes: panes.views(),
            });
            app.add_plugins(crate::native_host::panes::ultralight::PaneDisplayPlugin);
        }
    }

    // Bridge-display profile (issue #1123). Installed unconditionally — the
    // plugin is a no-op with no `BridgeDisplayConfig` — and the config is
    // inserted only when a validated profile was given, so the single-window
    // #1121 host is byte-for-byte unchanged when no `--profile` is passed. When
    // present it opens one borderless-fullscreen surface per configured monitor:
    // the viewscreen on the primary window, each Station on its own window.
    app.add_plugins(crate::native_host::bridge_display::BridgeDisplayPlugin);
    if let Some(profile) = &cfg.bridge_profile {
        app.insert_resource(crate::native_host::bridge_display::BridgeDisplayConfig {
            profile: profile.clone(),
        });
    }

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
    } else if cfg.panes.as_ref().is_none_or(|p| p.opened.is_empty()) {
        // The mode is correct and the flag is not refused — but with nothing
        // able to connect it opens a window onto a lobby nothing can start.
        // Every route to `InProgress` needs a session: collective `SetReady`
        // auto-start, or the host page's force-start
        // (`drain_force_start_input`, wasm-only). Browser participants are
        // still deferred to issue #1112, and there is no native force-start, so
        // say so at the top of the log rather than leaving an operator watching
        // a lobby that will never move.
        //
        // A host with local Station panes (issue #1122) does NOT take this arm:
        // its panes are participants, they ready up like any other, and the
        // mission starts when they do.
        crate::pwarn!(
            cfg.log,
            crate::logging::LogCat::Lobby,
            "no --solo and no --pane: this host is waiting in the lobby for \
             participants, but browser clients cannot join a native host yet \
             (issue #1112 — PeerJS is browser JavaScript). Nothing can ready up \
             and there is no native force-start, so the mission will not start. \
             Re-run with --solo to fly it on Backfill, or --pane <NAME> to crew \
             it locally."
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
