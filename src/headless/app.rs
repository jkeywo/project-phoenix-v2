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

use crate::asteroid_lifecycle::AsteroidLifecyclePlugin;
use crate::console_bridge::{AiChatterEvent, HudStateChanged, LobbyStateChanged};
use crate::entities::ai_declaration_manifest;
use crate::entity_config::EntityConfig;
use crate::lobby::{LobbyOutbox, LobbyPlugin, SelectedShipResource, Target};
use crate::logging::LoggingPlugin;
use crate::marker_validate::MarkerFinding;
use crate::messages::{GamePhase, ServerMessage};
use crate::modifier_coordination::ModifierCoordinationPlugin;
use crate::perf::tick::TickSampler;
use crate::server_app::{add_simulation_plugins_with, SimPluginOptions};
use crate::ship_plugin::PendingShipConfig;
use crate::sim_rng::{SeedSource, SimRng};
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
///
/// Every template that *does* parse is also checked against the model-marker
/// contract (issue #758): each authored `marker` / `markers` reference must
/// resolve in the rig sidecar the template's `[mesh]` selects. The findings are
/// returned rather than logged here so the caller can gate on them before
/// anything spawns — an unresolved marker would otherwise attach a beam,
/// exhaust plume, or camera to the ship's centre with no diagnostic at all.
/// Note the deliberate asymmetry with the parse-skip policy above: a marker
/// error in ANY discovered template aborts the run, because unlike a parse
/// failure it is silent and would corrupt the run's numbers rather than stop
/// it. See the gate in [`build_headless_app`].
///
/// The third return is the AI-declaration manifest (issue #885a): one rendered
/// line per (template, AI-capable fine system), saying whether the template
/// declared it or which synthesiser is filling it. Returned rather than logged
/// here for the same reason as the marker findings — this runs before
/// `LogPlugin` installs a subscriber, so anything emitted here goes nowhere.
/// Diagnostic only; nothing about it gates the load, because the thing that
/// gates is strict mode and that lives in `EntityConfig::from_toml`.
fn preload_entity_templates(
    dir: &str,
) -> Result<(usize, Vec<MarkerFinding>, AiDeclarationReport), BuildError> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| BuildError(format!("could not list {dir:?}: {e}")))?;
    let mut loaded = 0;
    let mut findings: Vec<MarkerFinding> = Vec::new();
    // Accumulated across the whole template set so the summary is a fleet total
    // rather than a per-file trickle.
    let mut report = AiDeclarationReport::default();
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
        if std::fs::read_to_string(&path).is_err() {
            warn!(target: "config", "template unreadable, skipping: {key}");
            continue;
        }
        // Resolve the template's `includes` closure BEFORE parsing (issue
        // #869): only the fully composed document is ever validated, and it is
        // the composed text that marker validation and the AI-declaration
        // manifest must read.
        //
        // Note the deliberate asymmetry with the parse-skip policy below. A
        // *composition* failure — cycle, missing fragment, malformed
        // `includes` — aborts the whole build, because a template that
        // declares includes has said it is incomplete on its own: skipping it
        // would silently drop content the author explicitly assembled. A plain
        // TOML parse error keeps the historical skip-with-warning, so one bad
        // cosmetic asteroid still cannot stop a combat test.
        let resolved = match crate::entity_includes::resolve_from_disk(&key) {
            Ok(resolved) => resolved,
            Err(e) => return Err(BuildError(format!("template composition failed: {e}"))),
        };
        let composed = resolved.is_composed();
        let toml = resolved.toml.clone();
        match resolved.parse() {
            Ok(cfg) => {
                findings.extend(validate_template_markers(&key, &toml, &cfg));
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let missing = ai_declaration_manifest::undeclared_keys(&cfg).len();
                if missing > 0 {
                    report.undeclared += missing;
                    report.templates_with_gaps += 1;
                    report
                        .lines
                        .extend(ai_declaration_manifest::manifest_lines(&stem, &cfg));
                }
                crate::config_cache::insert_native_config(key, cfg);
                loaded += 1;
            }
            Err(e) if composed => {
                // A composed template that does not validate is a load error:
                // the offending combination exists in no single authored file,
                // so skipping it would hide the one thing composition can get
                // wrong that authoring cannot.
                return Err(BuildError(format!("composed template is invalid: {e}")));
            }
            Err(e) => warn!(target: "config", "template failed to parse, skipping: {key}: {e}"),
        }
    }
    Ok((loaded, findings, report))
}

/// The fleet-wide AI-declaration manifest gathered while preloading templates
/// (issue #885a), held until a `tracing` subscriber exists to receive it.
#[derive(Default)]
struct AiDeclarationReport {
    /// AI-capable fine systems that declared neither a policy nor an explicit
    /// idle state, across every template loaded.
    undeclared: usize,
    /// How many templates contributed at least one of those.
    templates_with_gaps: usize,
    /// One rendered line per (template, fine system) — the per-slot worklist.
    lines: Vec<String>,
}

impl AiDeclarationReport {
    /// Emit the manifest. `info` for the fleet total, `debug` for the per-slot
    /// worklist: both sit under the default `warn` filter, so a normal run is
    /// unchanged and `--log config=debug` is what asks for the breakdown.
    fn emit(&self) {
        if self.undeclared == 0 {
            return;
        }
        info!(
            target: "config",
            "AI-declaration manifest: {} AI-capable fine system(s) across {} \
             template(s) declare neither a policy nor an explicit idle state, so a \
             Rust-side synthesiser supplies their automation (PRD #774 US7; issue \
             #885b's worklist). Run with `--log config=debug` for the \
             per-(template, system) breakdown.",
            self.undeclared,
            self.templates_with_gaps
        );
        for line in &self.lines {
            debug!(target: "config", "{line}");
        }
    }
}

/// Model-marker contract check for one parsed template: resolve its rig
/// sidecar off disk (identity rig when genuinely absent, mirroring
/// `glb_visual::resolve_sidecar_rig` on native) and validate every authored
/// marker reference against it, plus the sidecar's own duplicate declarations.
fn validate_template_markers(key: &str, toml: &str, cfg: &EntityConfig) -> Vec<MarkerFinding> {
    let mut findings = Vec::new();
    let rig = cfg.mesh.as_ref().and_then(|mesh| {
        let model = mesh.model.as_deref()?;
        let path = crate::model_rig::sidecar_path(model, mesh.variant.as_deref());
        let sidecar = std::fs::read_to_string(&path).unwrap_or_default();
        findings.extend(crate::marker_validate::duplicate_marker_findings(
            &path, &sidecar,
        ));
        match crate::model_rig::ModelRig::from_toml(&sidecar) {
            Ok(rig) => Some(rig),
            Err(e) => {
                warn!(target: "config", "rig sidecar {path} failed to parse: {e}");
                None
            }
        }
    });
    findings.extend(crate::marker_validate::validate_entity_markers(
        key,
        toml,
        cfg,
        rig.as_ref(),
    ));
    findings
}

/// Assemble the headless app. Does not run it — see [`run`].
pub fn build_headless_app(args: &HeadlessArgs) -> Result<App, BuildError> {
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
    // `Startup`, so it has to be populated before the app is built.
    let template_dir = std::path::Path::new(&ship_path)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "assets/entities".to_string());
    let (loaded, marker_findings, ai_declarations) = preload_entity_templates(&template_dir)?;

    // Model-marker contract gate (issue #758). This validates EVERY template
    // discovered in `template_dir` — not just the ones this run will actually
    // spawn — and a single error aborts the whole build before `App::new()`.
    //
    // That is deliberately stricter than the parse-skip policy above (a
    // template that fails to *parse* is skipped so one bad cosmetic asteroid
    // cannot stop a combat test). The asymmetry is the point: a parse failure
    // is loud and self-limiting — the template simply isn't in the cache, so
    // anything that needs it fails visibly — whereas an unresolved marker is
    // silent by construction. It attaches the beam, exhaust, or camera to the
    // ship's centre and produces a plausible-looking run whose numbers are
    // wrong. Since the run's spawn set is not known until the world and the
    // AI have had their say, "every discovered template" is the only scope
    // that can be checked before anything spawns.
    //
    // Errors are folded into the returned `BuildError`; warnings are reported
    // below, after `LogPlugin` installs a subscriber (before it, every
    // `tracing` line goes nowhere).
    if crate::marker_validate::has_error(&marker_findings) {
        let errors: Vec<String> = marker_findings
            .iter()
            .filter(|f| f.is_error())
            .map(MarkerFinding::describe)
            .collect();
        return Err(BuildError(format!(
            "model-marker contract violated; spawning blocked ({} error(s)): {}",
            errors.len(),
            errors.join("; ")
        )));
    }

    let mut app = App::new();

    app.add_plugins((
        PanicHandlerPlugin,
        LogPlugin {
            // Our own categories gate the `plog!` call sites; this filter only
            // governs bevy-internal events. Keep it quiet by default so the
            // report is the loudest thing on stdout.
            //
            // Root cause of issue #840 ("--log emits nothing yet slows the
            // run"): the plumbing here was always correct — the parser, the
            // `LogFilterConfig` gate, this subscriber, and the `EnvFilter`
            // targets all line up. What was missing were `plog!` *call sites*:
            // `ai`/`power` had none, `weapons` had a single `ptrace!`, so
            // `--log ai=debug` had nothing to print. #840 added the load-bearing
            // sites (target changes, opened/ceased fire, power energize/brownout,
            // damage). The slowdown is inherent, not lost output: any `debug`/
            // `trace` directive raises `tracing`'s global max-level hint, so
            // bevy/rapier's own dense debug/trace callsites flip from statically
            // compiled-out to dynamically `EnvFilter`-checked every tick. That
            // cost is the filter *running*, and it is unavoidable while the
            // process shares one global subscriber — logs go to stderr, so it
            // never corrupts the stdout report.
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
        // Necessary but not sufficient on its own — the other half is the
        // seeded `SimRng` inserted below, which is why `--seed` turns this on.
        // The contract is same binary, same machine.
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

    // Marker-contract warnings (issue #758). Reported HERE, not next to the
    // error gate above: `LogPlugin::build` has only just installed the global
    // `tracing` subscriber, and anything emitted before it is silently
    // dropped. `warn!` rather than `info!` so the default "warn" filter above
    // still lets these through — an unresolved default camera marker is the
    // kind of thing a run should say out loud.
    for f in marker_findings.iter().filter(|f| !f.is_error()) {
        warn!(target: "assets", "marker validation [warn] {}", f.describe());
    }

    // AI-declaration manifest (issue #885a), reported here for the same reason
    // as the marker warnings above — it was gathered before the subscriber
    // existed. Below the default `warn` filter, so it costs a normal run
    // nothing and `--log config=debug` is what asks for it.
    ai_declarations.emit();

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
    let mut world_config = crate::world::config::parse_world(&world_toml)
        .map_err(|e| BuildError(format!("world {:?} failed to parse: {e}", args.world_path)))?;

    // Duel side transform (issue #844). Only when `--side-a`/`--side-b` is
    // given, so a plain `--world` run is untouched. Pure over `WorldConfig`;
    // the filesystem-backed resolver is injected here in production.
    if !args.side_a.is_empty() || !args.side_b.is_empty() {
        world_config = super::duel::apply_duel_sides(
            world_config,
            &args.side_a,
            &args.side_b,
            &super::duel::resolve_template,
        )
        .map_err(|e| BuildError(format!("duel sides: {e}")))?;
    }

    // Seed precedence: `--seed`, then the world TOML's `[global] seed`, then a
    // seed drawn from the OS. Resolved here because this is the first point at
    // which both the CLI args and the parsed world are in scope. Inserted
    // *after* `add_simulation_plugins_with`'s `init_resource` further down
    // would also work — `insert_resource` wins either way — but keeping it
    // beside the world config keeps the precedence chain readable.
    let sim_rng = match (args.seed, world_config.global.seed) {
        (Some(seed), _) => SimRng::new(seed, SeedSource::Cli),
        (None, Some(seed)) => SimRng::new(seed, SeedSource::World),
        (None, None) => SimRng::random(),
    };
    info!(
        target: "config",
        "headless: seed={} ({})", sim_rng.seed(), sim_rng.source().as_str()
    );

    // Atomic composition validation (issue #750). Resolve every authored world
    // reference across the effective composition (root + additive
    // `extra_worlds`) BEFORE the world config is inserted and anything spawns.
    // Any error finding aborts the whole build, so a broken composition leaves
    // zero partial root-world content active.
    {
        use crate::world::validate::{has_error, validate_composition, WorldSource};
        // Own the child TOML sources + parsed configs so the borrowed
        // `WorldSource`s outlive the validation call.
        let mut child_owned: Vec<(String, String, crate::world::config::WorldConfig)> = Vec::new();
        for path in &world_config.extra_worlds {
            match std::fs::read_to_string(path) {
                Ok(toml) => match crate::world::config::parse_world(&toml) {
                    Ok(cfg) => child_owned.push((path.clone(), toml, cfg)),
                    Err(e) => {
                        return Err(BuildError(format!(
                            "world composition: extra_world {path:?} failed to parse: {e}"
                        )));
                    }
                },
                Err(e) => {
                    return Err(BuildError(format!(
                        "world composition: could not read extra_world {path:?}: {e}"
                    )));
                }
            }
        }
        let root_src = WorldSource::new(args.world_path.clone(), &world_toml, &world_config);
        let children: Vec<WorldSource> = child_owned
            .iter()
            .map(|(p, t, c)| WorldSource::new(p.clone(), t, c))
            .collect();
        let findings = validate_composition(&root_src, &children);
        for f in &findings {
            let loc = f
                .source
                .line
                .map(|l| format!("{}:{}", f.source.file, l))
                .unwrap_or_else(|| f.source.file.clone());
            info!(
                target: "world",
                "world validation [{}] {}: {} ({loc})",
                match f.severity {
                    crate::world::validate::Severity::Error => "error",
                    crate::world::validate::Severity::Warning => "warn",
                },
                f.category,
                f.message
            );
        }
        if has_error(&findings) {
            let errors: Vec<String> = findings
                .iter()
                .filter(|f| f.is_error())
                .map(|f| {
                    let loc = f
                        .source
                        .line
                        .map(|l| format!("{}:{}", f.source.file, l))
                        .unwrap_or_else(|| f.source.file.clone());
                    format!("[{}] {} ({loc})", f.category, f.message)
                })
                .collect();
            return Err(BuildError(format!(
                "world composition invalid; activation blocked ({} error(s)): {}",
                errors.len(),
                errors.join("; ")
            )));
        }
    }

    app.insert_resource(world_config);

    // Ship config, before `LobbyPlugin`: the native twin of
    // `wasm_validate_stations`. Without it `update_session_with_config` falls
    // back to `load_ship_config_from_disk`, which returns the *battleship*
    // roster regardless of `--ship` — so every station, and therefore every
    // backfilled AI system, would belong to the wrong hull.
    // `read_toml` first so an unreadable ship still reports the io error it
    // always did; composition then resolves any `includes` the hull declares
    // (issue #869) so the native `PendingShipConfig` matches the composed hull
    // the cache holds.
    let _ = read_toml(&ship_path, "ship")?;
    let ship_entity_config = crate::entity_includes::load_entity_config(&ship_path)
        .map_err(|e| BuildError(format!("ship {ship_path:?} failed to parse: {e}")))?;
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
            physics_last: args.physics_last,
            registration_order: args.registration_order,
            extra_registration_probes: args.extra_registration_probes,
        },
    );
    // After the plugins, so it overrides their OS-seeded `init_resource`.
    app.insert_resource(sim_rng);
    app.add_plugins(WorldPlugin);

    // Frame clock. `ManualDuration` makes every `Time` clock advance by
    // exactly `dt` per `update()` regardless of wall clock. Since issue #895
    // the SIMULATION rate is no longer this frame rate: the sim runs in
    // `FixedUpdate` at the world's `[global] sim_tick_hz` (the `WorldConfig`
    // inserted above, applied by `reconcile_fixed_timestep`), and each
    // `update()` here steps it zero or more whole logical ticks so that sim
    // time tracks the `dt`-per-frame virtual clock. At the default
    // `--hz 60` against the default `sim_tick_hz = 60` that is exactly one
    // tick per frame.
    //
    // Rapier no longer needs telling anything here (issue #896). It used to be
    // handed `TimestepMode::Fixed { dt: args.dt }` from this very line — the
    // FRAME period, a different clock from the one the simulation runs on, and
    // wrong the moment `--hz` and `sim_tick_hz` disagreed. Since #896 physics
    // runs inside `FixedUpdate` at the authored `sim_tick_hz`, set once in
    // `server_app::register_physics` and kept in step by
    // `sim_tick::reconcile_fixed_timestep` like every other tick-rate consumer.
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(args.dt),
    ));

    app.add_systems(
        FixedUpdate,
        headless_auto_start.before(crate::sim_sets::SimSet::Input),
    );

    // Telemetry. `collect_outbound` and `collect_balance_events` run in
    // `Last` and stamp each record with `Res<SimTick>` (issue #895
    // re-review — a per-`update()` frame counter used to do this and folded
    // multiple logical ticks into one stamp whenever `--hz` ran slower than
    // `sim_tick_hz`); `register_sim_tick` above (`add_simulation_plugins_with`)
    // guarantees that resource exists.
    app.insert_resource(super::report::RunTelemetry {
        capture_stream: args.report_format == super::args::ReportFormat::Ndjson,
        ..Default::default()
    })
    // Chained so an ndjson tick reads message-traffic-then-balance rather
    // than in whatever order the executor happened to pick.
    .add_systems(
        Last,
        (
            super::report::collect_outbound,
            super::report::collect_balance_events,
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
