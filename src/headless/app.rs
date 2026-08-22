//! Builds and drives the headless Bevy app.
//!
//! Since issue #1218 the core plugins, the render surrogate, and the whole
//! world-ingestion order all come from the shared [`crate::boot`] seam: this
//! module fills a [`BootPlan`](crate::boot::BootPlan) with
//! [`Headless`](crate::boot::BootProfile::Headless) and calls
//! [`boot::build`](crate::boot::build), so the headless inventory can no longer
//! drift from the two browser inventories (a drift boot's three-profile parity
//! test guards). What stays here is the genuinely headless-only work boot has no
//! reason to know about: the diagnostic template preload and its model-marker
//! gate, the seed-precedence resolution, the player-hull materiel, the
//! simulation/lobby plugins, and the frame clock, auto-start and telemetry the
//! harness loop reads.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

use crate::asteroids::lifecycle::AsteroidLifecyclePlugin;
use crate::boot::{BootError, BootPlan, BootProfile, WorldIngest};
use crate::core::messages::{GamePhase, ServerMessage};
use crate::entities::ai_declaration_manifest;
use crate::entities::config::EntityConfig;
use crate::entities::loader::TemplateLoader;
use crate::entities::marker_validate::MarkerFinding;
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
    }
}

/// Every spawnable template under `dir`, recursively, EXCEPT the fragment tree.
///
/// Recursive since issue #954, which moved the three-weapon RNG-coverage escort
/// to `assets/entities/test/rng_coverage_lancer.toml` so that no *shipped fleet*
/// hull carries all three weapon kinds. That relocation is invisible to the
/// fleet walks, which read the top level only — but it must NOT be invisible
/// here.
///
/// **The reason has changed since #973, and the old one is no longer true.** It
/// used to be that the spawn path was cache-only, so a world naming a template
/// this walk skipped logged "entity template not found in cache" and silently
/// spawned nothing. `entity_loader::resolve_entity_via` now falls back to
/// `WasmTemplateLoader`, which on native reads the filesystem, so that
/// particular hole is closed at the spawn rather than here. What the recursive
/// walk still buys is the model-marker contract gate below, which is scoped to
/// exactly the templates this walk discovers and validates them *before*
/// `App::new()` — a template it skips is a template whose markers nobody
/// checks. Keeping the cache complete is a second, smaller benefit: it is what
/// stops the spawn depending on that filesystem fallback in the first place.
///
/// `fragments/` is the one subdirectory excluded, and it is excluded for a
/// reason that is a property of its contents rather than of its name: nothing in
/// it is spawnable. They are partial documents that hulls compose FROM (see
/// `include_resolve::tests::the_fragments_live_outside_the_shipped_template_directory`),
/// so caching them as templates would offer the world loader entities that are
/// not entities.
fn spawnable_templates_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
    // Sorted so the cache is populated in the same order on every filesystem —
    // the load order is observable through `content_ledger::record`.
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "fragments") {
                continue;
            }
            spawnable_templates_under(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            out.push(path);
        }
    }
}

/// Load every spawnable template under `assets/entities/` into the native
/// template cache.
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
    // Trailing slash trimmed for the same reason the old `format!`-built key did
    // it: the cache key is this path with separators normalised, and
    // `"assets/entities/"` would key everything under `assets/entities//…`,
    // which matches nothing a world file authors.
    let root = std::path::Path::new(dir.trim_end_matches('/'));
    // A missing directory stays an error, as it was when this read the directory
    // itself: a preload that silently caches nothing is the worst possible way
    // to report a wrong `--ship` path.
    std::fs::read_dir(root).map_err(|e| BuildError(format!("could not list {dir:?}: {e}")))?;
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    spawnable_templates_under(root, &mut entries);

    let mut loaded = 0;
    let mut findings: Vec<MarkerFinding> = Vec::new();
    // Accumulated across the whole template set so the summary is a fleet total
    // rather than a per-file trickle.
    let mut report = AiDeclarationReport::default();
    for path in entries {
        // Key on the repo-relative path the world TOML uses, with forward
        // slashes — on Windows `Path::display` would emit backslashes and every
        // lookup would miss. Built from the WHOLE path rather than
        // `dir` + file name, so a template in a subdirectory is keyed by the
        // path a world actually names it with
        // (`assets/entities/test/rng_coverage_lancer.toml`, not
        // `assets/entities/rng_coverage_lancer.toml`).
        let key = path.to_string_lossy().replace('\\', "/");
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
        let resolved = match crate::entities::include_resolve::resolve_from_disk(&key) {
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
                crate::entities::config_cache::insert_native_config(key, cfg);
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
        let path = crate::entities::model_rig::sidecar_path(model, mesh.variant.as_deref());
        let sidecar = std::fs::read_to_string(&path).unwrap_or_default();
        findings.extend(crate::entities::marker_validate::duplicate_marker_findings(
            &path, &sidecar,
        ));
        match crate::entities::model_rig::ModelRig::from_toml(&sidecar) {
            Ok(rig) => Some(rig),
            Err(e) => {
                warn!(target: "config", "rig sidecar {path} failed to parse: {e}");
                None
            }
        }
    });
    findings.extend(crate::entities::marker_validate::validate_entity_markers(
        key,
        toml,
        cfg,
        rig.as_ref(),
    ));
    findings
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
    let (loaded, marker_findings, ai_declarations) = preload_entity_templates(&template_dir)?;

    // Model-marker contract gate (issue #758). This validates EVERY template
    // discovered in `template_dir` — not just the ones this run will actually
    // spawn — and a single error aborts the whole build before boot composes an
    // `App`.
    //
    // That is deliberately stricter than the parse-skip policy in the preload (a
    // template that fails to *parse* is skipped so one bad cosmetic asteroid
    // cannot stop a combat test). The asymmetry is the point: a parse failure is
    // loud and self-limiting — the template simply isn't in the cache, so
    // anything that needs it fails visibly — whereas an unresolved marker is
    // silent by construction. It attaches the beam, exhaust, or camera to the
    // ship's centre and produces a plausible-looking run whose numbers are
    // wrong. Since the run's spawn set is not known until the world and the AI
    // have had their say, "every discovered template" is the only scope that can
    // be checked before anything spawns.
    //
    // Errors abort now; warnings are reported below, once boot's `LogPlugin` has
    // installed a subscriber (before it, every `tracing` line goes nowhere).
    if crate::entities::marker_validate::has_error(&marker_findings) {
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
    };
    let mut app = crate::boot::build(plan).map_err(map_boot_error)?;

    // Marker-contract warnings (issue #758) and the AI-declaration manifest
    // (issue #885a), both gathered by the preload BEFORE any subscriber existed.
    // Reported HERE, after boot's `LogPlugin::build` installed the global
    // `tracing` subscriber: anything emitted before it is silently dropped.
    // `warn!` for the marker findings so the default `warn` filter still lets
    // them through; the manifest sits below it (`--log config=debug` asks for
    // the breakdown), so a normal run pays nothing.
    for f in marker_findings.iter().filter(|f| !f.is_error()) {
        warn!(target: "assets", "marker validation [warn] {}", f.describe());
    }
    ai_declarations.emit();

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

    // Telemetry. `collect_outbound` and `collect_balance_events` run in `Last`
    // and stamp each record with `Res<SimTick>` (issue #895); `register_sim_tick`
    // inside `add_simulation_plugins_with` guarantees that resource exists.
    app.insert_resource(super::report::RunTelemetry {
        capture_stream: args.report_format == super::args::ReportFormat::Ndjson,
        ..Default::default()
    })
    // Chained so an ndjson tick reads message-traffic-then-balance rather than in
    // whatever order the executor happened to pick.
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
