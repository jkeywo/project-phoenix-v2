//! Builds and drives the headless Bevy app.
//!
//! Since issue #1218 the core plugins, the render surrogate, and the whole
//! world-ingestion order all come from the shared [`crate::boot`] seam: this
//! module fills a [`BootPlan`](crate::boot::BootPlan) with
//! [`Headless`](crate::boot::BootProfile::Headless) and calls
//! [`boot::build`](crate::boot::build), so the headless inventory can no longer
//! drift from the two browser inventories (a drift boot's three-profile parity
//! test guards). What stays here is the genuinely headless-only work boot has no
//! reason to know about: the seed-precedence resolution, the player-hull
//! materiel, the simulation/lobby plugins, and the frame clock, auto-start and
//! telemetry the harness loop reads.
//!
//! The template preload and its model-marker gate moved out to
//! [`crate::entities::template_preload`] in issue #1121, so the native windowed
//! host runs the identical populate — it is behind no feature now, where it used
//! to be reachable only with `--features headless`. Two behaviours were
//! deliberately strengthened on the way (a canonicalised cache key; a
//! zero-template walk refused rather than returned as an empty success); that
//! module's docs say what each one is and why it is safe for this caller.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use crate::asteroids::lifecycle::AsteroidLifecyclePlugin;
use crate::boot::{BootError, BootPlan, BootProfile, NativeRenderSurface, WorldIngest};
use crate::core::messages::{GamePhase, ServerMessage};
use crate::entities::loader::TemplateLoader;
use crate::entities::template_preload::preload_entity_templates;
use crate::lobby::{LobbyOutbox, LobbyPlugin, SelectedShipResource, Target};
use crate::logging::LoggingPlugin;
use crate::modifiers::coordination::ModifierCoordinationPlugin;
use crate::perf::tick::TickSampler;
use crate::server_app::{
    add_simulation_plugins_with, RegistrationOrder, RegistrationProbes, SimPluginOptions,
};
use crate::ship_plugin::PendingShipConfig;
use crate::sim_rng::{SeedSource, SimRng};
use crate::world::load::LoadError;
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

/// Fold a [`BootError`] into the [`BuildError`] shape the harness has always
/// reported, preserving the substrings existing callers and tests assert.
///
/// The load-error arms keep the two special-cased messages the inline loader
/// carried: `could not read world` (the `missing_world_file_is_a_clean_error`
/// substring) and the `duel sides:` prefix a failing `--side-a`/`--side-b`
/// transform reports. A blocked activation keeps the `activation blocked`
/// wording the composition gate has always used; boot's message already names
/// the erroring findings' categories and text, which is what the
/// unresolvable-template test reads.
fn map_boot_error(e: BootError) -> BuildError {
    match e {
        BootError::WorldLoad(LoadError::ReadFailed { path }) => {
            BuildError(format!("could not read world {path:?}"))
        }
        BootError::WorldLoad(LoadError::TransformFailed { message }) => {
            BuildError(format!("duel sides: {message}"))
        }
        BootError::WorldLoad(other) => BuildError(format!("world load: {other}")),
        BootError::WorldInvalid(msg) => BuildError(format!("world activation blocked: {msg}")),
        // Unreachable from here: the native template-cache check is a
        // `BootProfile::NativeHost` property (issue #1121) and headless
        // populates that cache itself, before boot, for its own model-marker
        // gate. Mapped rather than `unreachable!`d so a future profile change
        // reports instead of panicking.
        BootError::NativeTemplatesMissing(missing) => BuildError(format!(
            "native entity templates not loaded: {}",
            missing.join(", ")
        )),
    }
}

/// Test-only overrides for how the simulation plugins are registered.
///
/// These three knobs used to live on [`HeadlessArgs`], each documented there as
/// "not a command-line flag and never parsed from one" — a pure test seam that
/// had leaked into the CLI-shaped session type. They fold straight into
/// [`SimPluginOptions`] (see that type for what each one proves) and default to
/// the exact production configuration, so [`build_headless_app`] — the binary
/// path — composes byte-for-byte the app it always did.
///
/// Tests reach this through the `SimFixture` harness in `tests/common`, never
/// by hand; it is `pub` only because the determinism guards that drive it live
/// in a separate integration-test crate.
#[derive(Clone, Copy, Debug, Default)]
pub struct SimRegistrationOverrides {
    /// Register the physics plugin last instead of first
    /// (`SimPluginOptions::physics_last`, issue #896). The two orders must reach
    /// the same state; that they do is the evidence physics is pinned by the
    /// explicit `configure_sets` edges, not by `add_plugins` call order.
    pub physics_last: bool,
    /// Which order to register the `SimSet`-chain plugins in
    /// (`SimPluginOptions::registration_order`, issue #899). `Shuffled(seed)`
    /// permutes it deterministically; the digest must not move.
    pub registration_order: RegistrationOrder,
    /// Extra mutation-proof probes to fold into the shuffled group
    /// (`SimPluginOptions::extra_registration_probes`, issue #899). `None` in
    /// every real run.
    pub extra_registration_probes: Option<RegistrationProbes>,
}

/// Assemble the headless app. Does not run it — see [`run`].
///
/// The binary path: no test overrides. Delegates to [`build_headless_app_with`]
/// with the production [`SimRegistrationOverrides::default`], which is exactly
/// the configuration the three removed `HeadlessArgs` fields defaulted to.
pub fn build_headless_app(args: &HeadlessArgs) -> Result<App, BuildError> {
    build_headless_app_with(args, SimRegistrationOverrides::default())
}

/// [`build_headless_app`], with the test-only registration overrides threaded
/// into the simulation plugins.
///
/// The core plugins, the render surrogate, and the whole world-ingestion order
/// (reset → load → validate → ledger apply → eager-record → freeze → insert the
/// world config and its compiled scripts) come from
/// [`boot::build`](crate::boot::build); see the module docs for what stays here.
pub fn build_headless_app_with(
    args: &HeadlessArgs,
    sim_overrides: SimRegistrationOverrides,
) -> Result<App, BuildError> {
    // When `--side-a` is given, its first entry chooses the player ship
    // (issue #844): resolve it to a template path and use it in place of
    // `--ship` so the preload, `PendingShipConfig`, and `SelectedShipResource`
    // all agree on the hull. `--ship` and `--side-a` are rejected together at
    // parse time, so this never silently overrides an explicit `--ship`.
    let ship_path = match args.side_a.first() {
        Some(name) => super::duel::resolve_template(name)
            .map_err(|e| BuildError(format!("--side-a player ship: {e}")))?,
        None => args.ship_path.clone(),
    };

    // Templates first: `update_session_with_config` reads the cache during
    // `Startup`, so it has to be populated before the app is built. This bulk
    // preload is deliberately NOT recorded into the content ledger (see
    // `content_ledger`'s module docs); its job is the cache plus the marker gate
    // below, both of which must precede the boot build.
    let template_dir = std::path::Path::new(&ship_path)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "assets/entities".to_string());
    let preload = preload_entity_templates(&template_dir).map_err(BuildError)?;
    let loaded = preload.loaded();

    // Model-marker contract gate (issue #758). This validates EVERY template
    // discovered in `template_dir` — not just the ones this run will actually
    // spawn — and a single error aborts the whole build before boot composes an
    // `App`. See [`TemplatePreload::marker_gate`] for why it is stricter than
    // the preload's own parse-skip policy.
    //
    // Errors abort now; warnings are reported below, once boot's `LogPlugin` has
    // installed a subscriber (before it, every `tracing` line goes nowhere).
    preload.marker_gate().map_err(BuildError)?;

    // The duel side transform (issue #844), now the boot load's `raw_transform`
    // hook. It rewrites only the raw `toml::Value` the script loader reads —
    // regenerating the slot drivers inside `duel.toml`'s `[script]` source — and
    // never the parsed `WorldConfig`, which the load derives from the untouched
    // text. Attached only when `--side-a`/`--side-b` is given, so a plain
    // `--world` run's raw value is untouched.
    let raw_transform: Option<Box<dyn Fn(toml::Value) -> Result<toml::Value, String>>> =
        if args.side_a.is_empty() && args.side_b.is_empty() {
            None
        } else {
            let side_a = args.side_a.clone();
            let side_b = args.side_b.clone();
            Some(Box::new(move |raw: toml::Value| {
                super::duel::apply_duel_sides(
                    raw,
                    &side_a,
                    &side_b,
                    &super::duel::resolve_template,
                    &super::duel::DuelTemplateLoader,
                )
                .map_err(|e| e.to_string())
            }))
        };

    // The boot seam (issue #1218). `boot::build` composes the shared core, the
    // render surrogate (the four render asset types, the three host-page bridge
    // messages, and the lobby-state push), and runs the one world load —
    // resetting the content ledger, reading the root and its `extra_worlds`
    // children, validating the composition and compiling the scripts exactly
    // once, aborting on a broken world (Headless is authoritative), applying the
    // ledger records, eager-recording the world's declared entity templates and
    // freezing the ledger, then inserting the `WorldConfig` and the
    // `PreCompiledScripts` for `WorldPlugin`'s `Startup` to consume. The
    // once-compiled set feeds both that Startup insertion and the build-time
    // fail-fast gate boot ran, so headless no longer compiles a world's scripts
    // twice.
    let plan = BootPlan {
        profile: BootProfile::Headless,
        // Headless reads the world off the filesystem through boot's reader — the
        // full reset→load→validate→compile→apply→freeze→insert order.
        world_ingest: WorldIngest::FromReader,
        // Keep bevy-internal events quiet by default so the report is the
        // loudest thing on stdout; a `--log` spec is folded in after the `warn`
        // floor. (Issue #840: `--log` needs `plog!` call sites, not just this
        // filter, to print anything.)
        log_filter: if args.log_spec.is_empty() {
            "warn".to_string()
        } else {
            format!("warn,{}", args.log_spec)
        },
        world_path: args.world_path.clone(),
        reader: Box::new(crate::world::load::FsReader),
        script_resolver: Box::new(crate::entities::config_cache::production_script_resolver()),
        // `--deterministic`/`--seed` pins the scheduler to one thread; the seeded
        // `SimRng` inserted below is the other half. The contract is same binary,
        // same machine.
        single_threaded: args.deterministic,
        raw_transform,
        // Headless has no renderer at all, so the native presentation axis is
        // inert for it — `Contract` is the only value a non-render-stack
        // profile can carry, and boot never consults it here.
        native_surface: NativeRenderSurface::Contract,
    };
    let mut app = crate::boot::build(plan).map_err(map_boot_error)?;

    // Marker-contract warnings (issue #758) and the AI-declaration manifest
    // (issue #885a), both gathered by the preload BEFORE any subscriber existed.
    // Reported HERE, after boot's `LogPlugin::build` installed the global
    // `tracing` subscriber: anything emitted before it is silently dropped.
    // `warn!` for the marker findings so the default `warn` filter still lets
    // them through; the manifest sits below it (`--log config=debug` asks for
    // the breakdown), so a normal run pays nothing.
    preload.report();

    // Crate-side log filtering (`plog!`), separate from boot's bevy `LogPlugin`.
    app.insert_resource(args.log.clone())
        .add_plugins(LoggingPlugin);

    // Seed precedence: `--seed`, then the world TOML's `[global] seed`, then a
    // seed drawn from the OS. The world config boot parsed and inserted is read
    // back here — the first point at which both the CLI args and the parsed
    // world are in scope. Inserted into the app *after*
    // `add_simulation_plugins_with`'s `init_resource` below, so it overrides the
    // OS-seeded default.
    let world_seed = app
        .world()
        .resource::<crate::world::config::WorldConfig>()
        .global
        .seed;
    let sim_rng = match (args.seed, world_seed) {
        (Some(seed), _) => SimRng::new(seed, SeedSource::Cli),
        (None, Some(seed)) => SimRng::new(seed, SeedSource::World),
        (None, None) => SimRng::random(),
    };
    info!(
        target: "config",
        "headless: seed={} ({})", sim_rng.seed(), sim_rng.source().as_str()
    );

    // Ship config, before `LobbyPlugin`: the native twin of
    // `wasm_validate_stations`. Without it `update_session_with_config` falls
    // back to `load_ship_config_from_disk`, which returns the *battleship*
    // roster regardless of `--ship` — so every station, and therefore every
    // backfilled AI system, would belong to the wrong hull. `read_toml` first so
    // an unreadable ship still reports the io error it always did; composition
    // then resolves any `includes` the hull declares (issue #869) so the native
    // `PendingShipConfig` matches the composed hull the cache holds.
    let _ = read_toml(&ship_path, "ship")?;
    let ship_entity_config = crate::entities::include_resolve::load_entity_config(&ship_path)
        .map_err(|e| BuildError(format!("ship {ship_path:?} failed to parse: {e}")))?;
    // Issue #935: the player's own hull is authored content too, and it is not
    // necessarily among `world_config.entities` (a duel side is chosen by
    // `--ship`/`--side-a`, not authored into the world), so boot's freeze of the
    // world's declared set need not have named it. `FsTemplateLoader` records the
    // composed hull into the content ledger as a side effect of resolving it (see
    // its doc comment); re-freezing then folds it into the frozen digest a save
    // is checked against, exactly as the single inline freeze did before boot
    // owned the first one. The ledger fold is path-sorted and order-independent,
    // so the frozen digest is byte-identical whether the hull rode in on boot's
    // eager walk or here.
    let _ = crate::entities::loader::FsTemplateLoader.load_template(&ship_path);
    crate::content_ledger::freeze();
    let ship_config = ship_entity_config
        .ship_config
        .ok_or_else(|| BuildError(format!("ship {ship_path:?} has no [[station]] blocks")))?;
    app.insert_resource(PendingShipConfig(ship_config));
    app.insert_resource(SelectedShipResource(ship_path.clone()));

    // `ConfigCachePlugin` is wasm-only; its two jobs are the template cache
    // (done above) and the faction registry, which `add_simulation_plugins`
    // already inserts from the `include_str!`ed native registry.
    app.add_plugins(AsteroidLifecyclePlugin)
        .add_plugins(ModifierCoordinationPlugin)
        .add_plugins(LobbyPlugin)
        .add_plugins(crate::lobby::lobby_outbox_broadcaster());

    add_simulation_plugins_with(
        &mut app,
        SimPluginOptions {
            render: false,
            physics_last: sim_overrides.physics_last,
            registration_order: sim_overrides.registration_order,
            extra_registration_probes: sim_overrides.extra_registration_probes,
        },
    );
    // After the plugins, so it overrides their OS-seeded `init_resource`.
    app.insert_resource(sim_rng);
    app.add_plugins(WorldPlugin);

    // Frame clock. `ManualDuration` makes every `Time` clock advance by exactly
    // `dt` per `update()` regardless of wall clock. Since issue #895 the
    // SIMULATION rate is no longer this frame rate: the sim runs in `FixedUpdate`
    // at the world's `[global] sim_tick_hz`, and each `update()` here steps it
    // zero or more whole logical ticks so that sim time tracks the
    // `dt`-per-frame virtual clock. At the default `--hz 60` against the default
    // `sim_tick_hz = 60` that is exactly one tick per frame. Rapier no longer
    // needs telling anything here (issue #896): physics runs inside `FixedUpdate`
    // at the authored `sim_tick_hz`, set once in `server_app::register_physics`.
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(args.dt),
    ));

    app.add_systems(
        FixedUpdate,
        headless_auto_start.before(crate::sim_sets::SimSet::Input),
    );

    // Console input-to-feedback latency (issue #1169). `DebugPlugin` installs
    // this flag `false` on every target; a headless run turns it on only when
    // asked, so a plain run takes no wall-clock reading and its digest is
    // byte-identical to one built before this existed
    // (`tests/console_latency.rs`).
    if args.console_latency {
        app.insert_resource(crate::debug::DebugConsoleLatencyEnabled(true));
    }

    // Telemetry. `collect_outbound` and `collect_balance_events` run in `Last`
    // and stamp each record with `Res<SimTick>` (issue #895); `register_sim_tick`
    // inside `add_simulation_plugins_with` guarantees that resource exists.
    app.insert_resource(super::report::RunTelemetry {
        capture_stream: args.report_format == super::args::ReportFormat::Ndjson,
        ..Default::default()
    })
    // Chained so an ndjson tick reads message-traffic-then-balance-then-story
    // rather than in whatever order the executor happened to pick. Narrative
    // last (issue #1338) so the beat a tick's damage caused prints after the
    // damage that caused it, and so the monotonic `seq` this collector assigns
    // is a fixed function of the tick rather than of executor order.
    .add_systems(
        Last,
        (
            super::report::collect_outbound,
            super::report::collect_balance_events,
            super::report::collect_narrative_events,
        )
            .chain(),
    );

    info!(
        target: "config",
        "headless: world={} ship={} templates={} dt={:.5}s ({:.1} Hz) ticks={}",
        args.world_path, ship_path, loaded, args.dt, args.hz(), args.max_ticks
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
///
/// **Registered in `FixedUpdate`, not `PreUpdate` (issue #907 review).**
/// `NextState<GamePhase>` writers that ran from `PreUpdate` applied at the
/// FRAME-level `StateTransition` (right after `PreUpdate`, before that
/// frame's fixed steps run), so `OnEnter(GamePhase::InProgress)` — and the
/// player-ship mint inside it, `spawn_game_start_entities` — fired at a point
/// in the schedule whose relationship to `SimTick` depended on frame pacing,
/// not on the tick that (would) apply the transition. `FixedUpdate` puts this
/// write on the same tick-scoped `StateTransition` site every other phase
/// writer uses (`register_fixed_state_transition` in `sim_tick.rs`,
/// `tick_countdown` in `lobby/server.rs`), so the mint inside `OnEnter` now
/// stamps a deterministic tick regardless of `--hz`/`dt`.
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

/// Pump the app for `args.max_ticks` FRAMES (`update()` calls), each of which
/// advances `args.dt` of virtual time and therefore runs however many fixed
/// simulation steps that covers — one apiece at the default `--hz 60` against
/// the default `sim_tick_hz = 60` (issue #895). `--ticks` keeps its pre-#895
/// name; what it counts is frames.
///
/// Deliberately not `App::run()`: with no `WinitPlugin` and no
/// `ScheduleRunnerPlugin` the default runner calls `update()` exactly once.
/// Driving the loop by hand also gives the frame budget and the exit condition
/// for free.
pub fn run(app: &mut App, max_ticks: u64) -> u64 {
    run_sampled(app, max_ticks, None)
}

/// `run`, with the harness-loop performance collector attached (issue #868).
///
/// Sampling brackets `app.update()` from outside, so the simulation cannot
/// observe it and a measured run steps identically to an unmeasured one. The
/// sampler is passed in rather than created here because the caller owns the
/// capture the run produces.
pub fn run_sampled(app: &mut App, max_ticks: u64, mut sampler: Option<&mut TickSampler>) -> u64 {
    app.finish();
    app.cleanup();
    let mut ticks = 0;
    while ticks < max_ticks {
        if let Some(sampler) = sampler.as_deref_mut() {
            sampler.tick_begin();
        }
        app.update();
        if let Some(sampler) = sampler.as_deref_mut() {
            sampler.tick_end();
        }
        ticks += 1;
        if app.world().resource::<State<GamePhase>>().get() == &GamePhase::GameOver {
            break;
        }
    }
    ticks
}
