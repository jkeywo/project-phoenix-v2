//! Activation.
use super::*;

/// Startup system: copy the unified `WorldConfig` from the WASM-side
/// thread-local cache into a Bevy `Resource` so downstream systems
/// (`spawn_world_entities`, `ai::server::tick_ai_controllers`) can read it
/// via `Res<WorldConfig>`.
///
/// On native (no WASM bridge) `get_world_config()` returns `None` and this
/// system is a no-op; downstream systems that iterate world entities
/// simply see an empty world (native unit tests only — production always
/// loads a world TOML through the WASM bridge).
pub fn insert_world_config_resource(mut commands: Commands) {
    if let Some(world_config) = crate::entities::config_cache::get_world_config() {
        commands.insert_resource(world_config);
    }
}

/// `Startup` system: populate [`RawWorldSource`] from the browser bridge's
/// stashed raw world TOML (issue #984, Rhai M6 phase 2a).
///
/// The script-loader's twin of [`insert_world_config_resource`]: `WorldConfig`
/// has dropped the raw `[script]` / `script` keys, so the Rhai loader needs the
/// untouched TOML text. Reads the [`BridgeWorldSource`]
/// Resource the wasm bridge inserts at `wasm_init` (issue #1181, replacing the
/// former `get_raw_world_source()` ambient free-function read). On native that
/// Resource is never inserted — headless compiles its scripts once in
/// `world::load::load` and hands them to `compile_world_scripts` as
/// [`PreCompiledScripts`] (issue #1214) — so `bridge` is `None` and this system
/// is a no-op there, exactly as its wasm-gated body used to be.
pub(crate) fn insert_raw_world_source_resource(
    mut commands: Commands,
    bridge: Option<Res<BridgeWorldSource>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    // `toml::Value`'s `FromStr` parses a single VALUE EXPRESSION (`1`, `"x"`,
    // `[1, 2]`), not a document — so it rejects every world file ("unexpected
    // content, expected nothing"). This arm shipped with that misuse in #984 P2a
    // and stayed invisible while every world was script-free: the error logged,
    // and there were no scripts to lose. `toml::from_str` is the document parser
    // the rest of the crate uses (and the same route `parse_world` takes, which
    // is why the world itself loaded while its scripts vanished).
    match toml::from_str::<toml::Value>(&bridge.toml) {
        Ok(toml) => commands.insert_resource(RawWorldSource {
            path: bridge.path.clone(),
            toml,
        }),
        Err(e) => bevy::log::error!(
            target: "world",
            "insert_raw_world_source_resource: world TOML at {} failed to re-parse: {e}",
            bridge.path
        ),
    }
}

/// `Startup` system: compile the loaded world's Rhai scripts and insert the
/// [`WorldScriptRuntime`] (issue #984, Rhai M6 phase 2a).
///
/// Ordered after [`insert_world_config_resource`] /
/// [`insert_raw_world_source_resource`] and before `spawn_world_entities` /
/// [`init_world_runtime`]. A world with no `script` key short-circuits before
/// building any engine, so a script-free world is a true no-op — no
/// `WorldScriptRuntime`, no content-ledger records, nothing that could move a
/// digest.
///
/// # One compile per load (issue #1214)
///
/// Two sources feed this system, checked in order:
///
/// * [`PreCompiledScripts`] — the headless path. `build_headless_app` compiled
///   the world's scripts once in `world::load::load` (recording their content
///   into the not-yet-frozen content ledger there) and hands the result over, so
///   this system takes ownership and builds the runtime WITHOUT recompiling. This
///   is what collapses the old headless double-compile.
/// * [`RawWorldSource`] — the browser path. On wasm the raw world TOML is
///   inserted at `Startup` and this system compiles it here, as before.
///
/// Either way the construction below is identical; the only difference is where
/// the `CompiledScripts` came from.
///
/// Findings fold into the SAME atomic activation gate the composition findings
/// use: on a script error this records the [`ScriptActivationGate`] read by
/// both immediate spawn systems so a script-error world spawns zero entities,
/// and inserts no runtime. Headless additionally hard-fails the build for a
/// script error (see `build_headless_app`), so a broken script never reaches a
/// running authoritative host.
///
/// The browser's [`RawWorldSource`] arm also validates the exact compiled
/// script-spawn references for template composition/resolution and doctrine,
/// because browser boot does not pass through `LoadPolicy::Activate`. The
/// [`PreCompiledScripts`] arm deliberately does not repeat that gate: native
/// Activate already consumed the same compiled references before handing them
/// here.
pub fn compile_world_scripts(
    mut commands: Commands,
    pre: Option<ResMut<PreCompiledScripts>>,
    raw: Option<Res<RawWorldSource>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
) {
    // The materialization chain flushes this per-App result before spawning.
    // Reset even on a script-free reload of an App that previously failed.
    commands.insert_resource(ScriptActivationGate::default());
    if let Some(config) = world_config.as_deref() {
        match crate::console::helm::transit::StaticTransitScopes::build(config) {
            Ok(scopes) => {
                commands.insert_resource(scopes);
            }
            Err(error) => {
                bevy::log::error!(
                    "compile_world_scripts: refused accepted Transit World recipes: {error}"
                );
                commands.insert_resource(ScriptActivationGate { blocked: true });
                return;
            }
        }
    }

    // Prefer scripts already compiled at build time (issue #1214): the headless
    // path runs the world through `world::load::load` once and hands the result
    // over as `PreCompiledScripts`, so this builds the runtime from it rather than
    // re-reading `RawWorldSource` and compiling a second time. `.take()` leaves the
    // resource holding `None`; a script-free `world::load` already stored `None`
    // there, which short-circuits exactly as the absent-`script`-key arm below.
    // The browser (no `PreCompiledScripts`) falls through to the `RawWorldSource`
    // compile it always did.
    let mut resolved_spawn_findings = Vec::new();
    let compiled = if let Some(mut pre) = pre {
        let Some(compiled) = pre.0.take() else {
            return;
        };
        compiled
    } else {
        let Some(raw) = raw else {
            return;
        };
        // No `script` key → nothing to compile. Short-circuit before building any
        // Rhai engine so the entire shipped (script-free) set pays nothing and can
        // never record into the content ledger.
        if raw.toml.get("script").is_none() {
            return;
        }
        let resolver = crate::entities::config_cache::production_script_resolver();
        let compiled =
            crate::world::script::load::load_world_scripts(&raw.path, &raw.toml, &resolver);
        // The compiled set's digest used to be written from inside the loader;
        // since issue #1241 it comes back as data and its caller applies it. This
        // is the browser's caller. The following Startup system seals the
        // preloaded content only after this write, so native and browser saves
        // bind to the same compiled root-script record (#1316).
        if let Some(digest) = &compiled.ledger_digest {
            digest.apply();
        }
        // Browser boot does not pass through `LoadPolicy::Activate`, so its
        // already-compiled exact source set needs the root script-spawn half of
        // the composition gate here, before any runtime or spawn can land.
        // Deliberately confined to RawWorldSource: PreCompiledScripts came from
        // the native Activate gate and must not be validated twice.
        if let Some(config) = world_config.as_deref() {
            let cache = crate::entities::config_cache::get_config_cache();
            let templates = crate::entities::loader::SpawnTemplateLoader {
                cache: &cache,
                host: &crate::entities::loader::WasmTemplateLoader,
            };
            resolved_spawn_findings = crate::world::validate::validate_resolved_script_spawns(
                config,
                &compiled.spawned_templates,
                &templates,
                &crate::entities::include_resolve::HostFragmentSource,
            );
        }
        compiled
    };

    let script_blocked = crate::world::validate::has_error(&compiled.findings);
    let spawn_blocked = crate::world::validate::has_error(&resolved_spawn_findings);
    if script_blocked || spawn_blocked {
        for f in compiled.findings.iter().filter(|f| f.is_error()) {
            bevy::log::error!(
                target: "world",
                "compile_world_scripts: script [error] {}: {}",
                f.category, f.message
            );
        }
        for f in resolved_spawn_findings.iter().filter(|f| f.is_error()) {
            match f.source.line {
                Some(line) => bevy::log::error!(
                    target: "world",
                    "compile_world_scripts: composition [error] {}:{} [{}] {}",
                    f.source.file,
                    line,
                    f.category,
                    f.message
                ),
                None => bevy::log::error!(
                    target: "world",
                    "compile_world_scripts: composition [error] {} [{}] {}",
                    f.source.file,
                    f.category,
                    f.message
                ),
            }
        }
        // Block activation atomically with the composition gate — spawn nothing.
        commands.insert_resource(ScriptActivationGate { blocked: true });
        return;
    }

    // No scripts actually compiled (e.g. an empty `[script]` table) — `from_compiled`
    // returns `None`, so insert no runtime and leave behaviour identical to a
    // script-free world.
    if let Some(runtime) = WorldScriptRuntime::from_compiled(compiled) {
        commands.insert_resource(runtime);
    }
}

/// Complete a preloaded host's one-time content boundary after compiling the
/// root scripts, including the script-free and blocked-script paths. Reader
/// boots already froze their full declared set and are deliberately untouched.
/// Exclusive execution keeps this thread-local ledger operation on the host
/// thread; the explicit Startup edges place it before either spawn pass.
pub fn freeze_host_preloaded_content(world: &mut World) {
    if world
        .remove_resource::<crate::world::PendingHostContentFreeze>()
        .is_some()
    {
        crate::content_ledger::freeze();
    }
}

/// `OnEnter(GamePhase::InProgress)` system: seeds the `ship_power` counter in
/// the world flag store from the fleet's authoritative hull `power_rating`.
///
/// This runs before `spawn_game_start_entities` so that `when` predicates on
/// `[[entity]]` entries with `spawn_on = "GameStart"` can gate spawns on
/// `counter(ship_power) >= N`.
///
/// The value must fold identically on every peer (issue #1116): the counter is
/// part of the scenario-flag digest, and a stationless GM owns no local ship
/// while two ship hosts each own a *different* one. So the rating is derived
/// from the frozen [`FleetRoster`]'s hull paths — resolved through the same
/// config cache on every peer — rather than from this peer's own
/// `ShipClientConfigResource`. A fleet ship whose `ship_path` is `None` is the
/// solo default ("the hull this peer's own lobby selected"), which is exactly
/// the local ship config; a solo run therefore keeps its pre-fleet value to the
/// byte. Multiple crewed hulls take the highest rating, so a mixed fleet is
/// gated by its heaviest ship rather than by whichever host happens to seed.
///
/// If no hull declares a `power_rating`, no counter is written and `ship_power`
/// defaults to `0`.
pub fn seed_ship_power_counter(
    fleet_roster: Option<Res<crate::lockstep::FleetRoster>>,
    ship_client_config: Res<crate::lobby::server::ShipClientConfigResource>,
    runtime: Option<ResMut<WorldContentRuntime>>,
) {
    let Some(mut runtime) = runtime else {
        return;
    };
    let cache = crate::entities::config_cache::get_config_cache();
    let rating = match fleet_roster.as_deref() {
        Some(roster) => roster
            .ships()
            .iter()
            .filter_map(|ship| match &ship.ship_path {
                Some(path) => cache.get(path).and_then(|config| config.power_rating),
                None => ship_client_config.0.power_rating,
            })
            .max(),
        None => ship_client_config.0.power_rating,
    };
    if let Some(rating) = rating {
        runtime.flags.set_flag_value("ship_power", rating as i64);
    }
}

/// The composition half of the atomic-activation gate, shared by **both** immediate-spawn
/// systems: `spawn_world_entities` (asteroid fields + named entries) and
/// `setup_world` in `server_app.rs` (the anonymous non-asteroid remainder —
/// stars, planets, nebulae).
///
/// Returns `true` when this world must spawn nothing, having logged every
/// blocking finding. `system` names the caller so the log says which half was
/// stopped; the answer itself is identical for both, because it reads only the
/// parsed [`crate::world::config::WorldConfig`]. The scheduling adapters also
/// consult their App's [`ScriptActivationGate`] before invoking either helper.
///
/// Both callers matter. The two systems are registered independently with no
/// ordering relationship between them, and each answers a failed entity
/// resolution by logging and moving on. Gating only one converts "the world is
/// missing one entity" into "the world is missing one half", which is a worse
/// failure than the one being fixed and breaks the atomicity
/// `world-content-lifecycle-state` promises. See
/// [`crate::world::validate::activation_findings`] for what is checked.
///
/// `config_cache` is the cache the gated spawn will resolve templates from —
/// the global one at `Startup`, a layer's own when a layer is spawning. It is
/// threaded in (issue #973) so the template-resolution check asks the *same*
/// question the spawn is about to ask, rather than a filesystem-backed
/// approximation of it that can pass where the spawn then fails.
///
/// # What is gated on has widened past `[[entity]]` (issue #1046)
///
/// The findings this reads now cover the hulls a world's SCRIPTS spawn as well
/// as the ones it declares — a scripted `spawn_entity` naming a literal
/// `template_path` is template-resolved and doctrine-anchor checked like any
/// other. On native that has teeth here: `SpawnTemplateLoader` is authoritative
/// about absence, so a script wave pointed at a deleted hull now blanks BOTH
/// immediate-spawn halves at `Startup` instead of logging once, mid-mission,
/// when its timer fires.
///
/// That is the intended reading of atomicity rather than a side effect — a
/// world that cannot build one of its waves is as broken as one that cannot
/// build a planet, and finding out at `Startup` is the whole point of the gate.
/// It does mean a finding here can now name a spawn site that is not an
/// `[[entity]]` block, which is why script findings carry the script's own line
/// (`world::validate`'s `script_spawn_line`): a bare template path with no
/// authored `name` is otherwise very hard to place.
///
/// # A note for whoever writes the next bare-`App` fixture
///
/// [`crate::entities::loader::SpawnTemplateLoader`] takes its authority from the
/// host behind the cache, so on native this gate is authoritative about paths
/// like `"fixture/station.toml"` that no filesystem holds. That is right — the
/// host really can decide them, and the answer really is "absent" — but it
/// means **a fixture that adds an `[[entity]]` and forgets the matching
/// `ConfigCache` entry gets zero spawns, not one.** The diagnostic naming the
/// entity and its template goes through `bevy::log::error!`, and a bare `App`
/// installs no `tracing` subscriber, so without help that failure reads as
/// "spawning is broken" rather than "your fixture is incomplete". The
/// `cfg(test)` mirror below is that help: unit tests, the only population at
/// risk, see the findings on stderr. Production keeps the single `bevy::log`
/// channel.
pub fn world_activation_blocked(
    world_config: &crate::world::config::WorldConfig,
    config_cache: &crate::entities::config_cache::ConfigCache,
    system: &str,
) -> bool {
    let templates = crate::entities::loader::SpawnTemplateLoader {
        cache: config_cache,
        host: &crate::entities::loader::WasmTemplateLoader,
    };
    let findings = crate::world::validate::activation_findings(
        world_config,
        &crate::entities::include_resolve::HostFragmentSource,
        &templates,
    );
    let errors = findings.iter().filter(|f| f.is_error()).count();
    if errors == 0 {
        return false;
    }
    for f in findings.iter().filter(|f| f.is_error()) {
        bevy::log::error!(target: "world", "world validation [error] {}: {}", f.category, f.message);
    }
    if errors > 0 {
        bevy::log::error!(
            target: "world",
            "{system}: spawn blocked: world composition invalid ({errors} error(s)); spawning zero entities"
        );
    }
    // See the doc above: a bare `App` has no `tracing` subscriber, so every
    // line emitted above is dropped and a fixture with an incomplete
    // `ConfigCache` looks like a broken spawner. Test builds only — integration
    // tests under `tests/` link the lib without `cfg(test)` and run a real app
    // with `LogPlugin`, so they already see the lines above.
    #[cfg(test)]
    {
        for f in findings.iter().filter(|f| f.is_error()) {
            eprintln!(
                "{system}: world validation [error] {}: {}",
                f.category, f.message
            );
        }
    }
    true
}

// -- Startup systems ---------------------------------------------------------

/// Startup system: initialise `WorldContentRuntime` and `WorldResource`
/// from the loaded `WorldConfig` (if any).
///
/// This is the post-PRD-#341 sole runtime-init entry point: the legacy
/// scenario / map split is gone. The comms half (`CommsRuntime`,
/// `CommsInboxRes`) is initialised by `comms::server::init_comms_runtime`,
/// which runs after this system in the Startup schedule. When no
/// `WorldConfig` resource is present (native unit tests) this is a
/// no-op and downstream trigger systems remain quiet.
pub(crate) fn init_world_runtime(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut world_resource: ResMut<WorldResource>,
    mut script_runtime: Option<ResMut<WorldScriptRuntime>>,
) {
    let Some(world_config) = world_config else {
        return;
    };
    runtime.situations = Default::default();
    runtime.plan_choice = None;
    runtime.situation_definitions = world_config.situation_definitions.clone();
    runtime.situation_configs = world_config.situations.clone();

    // The mission clock is NOT anchored here. Loading the world and starting
    // the mission are different moments - the lobby sits between them - and
    // every `after_secs` in a world TOML is authored against the second one.
    // `anchor_mission_clock` stamps `mission_clock` on the first
    // simulation tick of `GamePhase::InProgress`; see that system for what a
    // boot anchor cost.

    // Populate scenario metadata so the lobby title/description render correctly.
    world_resource.0.scenario_title = world_config.global.title.clone().unwrap_or_default();
    world_resource.0.scenario_description =
        world_config.global.description.clone().unwrap_or_default();

    // `spawn_world_entities` ran earlier in the Startup chain and already
    // populated `runtime.name_to_uuid` for named [[entity]] instances. Fold
    // in any additional names from `WorldConfig.name_to_uuid` WITHOUT
    // overwriting: existing entries win (single source of truth from the
    // spawn pass).
    for (name, uuid) in &world_config.name_to_uuid {
        runtime
            .name_to_uuid
            .entry(name.clone())
            .or_insert_with(|| uuid.clone());
    }

    // Replace the authored GM palette on each world load (#1305).
    runtime.gm_palette = world_config.gm_palette.clone();
    runtime.gm_objective_palette = world_config.gm_objective_palette.clone();
    runtime.gm_npc_doctrine_palette = world_config.gm_npc_doctrine_palette.clone();

    // Reset the complete paired table; scripts are the production source.
    runtime.triggers.clear();

    // Merge script-authored triggers (issue #984, Rhai M6 phase 2a). `None`
    // origin: these are the BASE world's own, so they anchor at the base flag
    // store and no `UnloadWorld` can retract them.
    if let Some(script_runtime) = script_runtime.as_deref_mut() {
        merge_script_triggers(&mut runtime, script_runtime, None);
    }

    // Issue #415: emit a WorldLoaded event so `on_world_loaded` triggers
    // declared in the base world fire on the first Update tick. Pushed onto
    // the pending queue (rather than evaluated here) so the dispatch logic
    // inside `tick_trigger_pipeline` is the single owner of trigger action
    // execution.
    runtime.pending_world_events.push(WorldEvent::WorldLoaded);
}

/// Transfer staged registrations into the registry as one ordered layer.
pub(crate) fn merge_script_triggers(
    runtime: &mut WorldContentRuntime,
    script_runtime: &mut WorldScriptRuntime,
    origin_layer: Option<&str>,
) {
    runtime
        .triggers
        .append_scripted(std::mem::take(&mut script_runtime.triggers), origin_layer);
}

/// Merge one layer's compiled `[script]` set into the live script runtime
/// (issue #1045), returning the AST keys this load ADDED so the unload can
/// retract exactly them.
///
/// The one place a supporting world's scripts join a running session.
///
/// **ASTs** are inserted under their authored key. A key already present is left
/// alone and NOT recorded as this layer's: two worlds may name the same sibling
/// `.rhai`, and an unload must not pull a unit someone else still calls into.
///
/// **Triggers** are staged onto `triggers` and drained by
/// [`merge_script_triggers`] with this layer's path as their origin, so they
/// evaluate against the layer's flag chain and retract with it.
///
/// # Only what this layer ACTUALLY added is registered
///
/// A trigger is registered only when the unit that built it is one this load
/// inserted. The two go together: a registration is *derived from running a
/// unit's top level*, so a layer that names a `.rhai` the base world (or an
/// earlier layer) already loaded arrives carrying that unit's registrations a
/// second time. Appending them unconditionally would give the shared script two
/// `on_world_loaded` states — and the applier's own `WorldLoaded` push would fire
/// the intro twice on the tick the layer landed. Instead the AST is retained once
/// and the registrations are instantiated once per owning layer, with that layer
/// stamped on every trigger/callback so unload can retract the exact slice.
///
/// `registrations` (the generic `on(..)` set) has no runtime consumer — it exists
/// for the load-time cross-reference pass, which already ran. Deadline handlers
/// are consumed by the applier immediately before this merge, when it arms the
/// layer's deadline rows at the activation tick. `content_hash` stays whatever
/// the base world set because the layer's set binds a save through its own
/// `<layer path>#scripts` ledger record.
pub(crate) fn merge_layer_scripts(
    layer_path: &str,
    compiled: crate::world::script::load::CompiledScripts,
    runtime: &mut WorldContentRuntime,
    script_runtime: &mut WorldScriptRuntime,
) -> Vec<String> {
    script_runtime
        .recipient_declarations
        .extend(compiled.recipient_references.declarations);
    for (path, lines) in &compiled.function_lines {
        script_runtime
            .function_lines
            .entry(path.clone())
            .or_insert_with(|| lines.clone());
    }
    let mut referenced_units: Vec<String> = Vec::new();
    for (unit_path, ast) in compiled.asts {
        referenced_units.push(unit_path.clone());
        script_runtime
            .ast_owners
            .entry(unit_path.clone())
            .or_default()
            .insert(Some(layer_path.to_string()));
        if let std::collections::btree_map::Entry::Vacant(slot) =
            script_runtime.asts.entry(unit_path)
        {
            slot.insert(ast);
        }
    }

    // Registrations belong to the world that declared the unit, not to the AST
    // path. Two layers sharing one sibling script receive distinct scoped
    // triggers while retaining one compiled AST.
    script_runtime.triggers.extend(compiled.script_triggers);
    merge_script_triggers(runtime, script_runtime, Some(layer_path));

    referenced_units
}

/// Startup system: queue all `extra_worlds` paths from the loaded `WorldConfig`
/// as `LoadWorld` commands so they are merged into the runtime on the first frame.
///
/// Runs after `init_world_runtime` in the Startup chain. Each path is pushed
/// into `PendingWorldLayerChanges` rather than applied directly so the same
/// `apply_world_layer_changes` path handles both startup and trigger-fired loads.
pub(crate) fn load_extra_worlds(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut pending: ResMut<PendingWorldLayerChanges>,
) {
    let Some(world_config) = world_config else {
        return;
    };
    for path in &world_config.extra_worlds {
        pending.0.push(WorldLayerChange::Load {
            path: path.clone(),
            loader_path: None,
        });
    }
}

// -- Mission clock -----------------------------------------------------------

/// `OnEnter(GamePhase::InProgress)` system: disarm the mission clock so the
/// next simulation tick re-stamps it.
///
/// One line, and it is the whole multi-round half of the fix. A session can
/// reach `InProgress` more than once (`ReturnToLobby` from the game-over screen
/// puts the crew back in the lobby and a second round starts from there, which
/// is why `reset_command_log` and `reset_broadcast_caches_on_start` sit in the
/// same `OnEnter` chain). Without this, round two would measure `after_secs`
/// from round one's start and arrive with its whole schedule already expired.
///
/// It writes `None` rather than a reading of its own because it cannot know
/// which clock it is standing on. Bevy applies a `NextState<GamePhase>` write
/// at whichever `StateTransition` site comes first; `Time` resolves to
/// `Time<Fixed>` at the fixed-schedule site `register_fixed_state_transition`
/// installs and to `Time<Virtual>` at the frame-level one, and those two clocks
/// disagree by up to one timestep — so a reading taken here would make the
/// schedule a function of which path started the mission and of frame pacing.
/// Deferring the reading to [`anchor_mission_clock`], which only ever runs
/// inside a fixed step, keeps it on one clock.
///
/// Since issue #1121's fix round every *production* start path writes from
/// `FixedUpdate`: the lobby countdown, headless and native-host auto-start, and
/// `auto_transition_from_loading`, which moved there for the #907 reason this
/// paragraph describes. That makes the hazard harder to reach, not gone — a
/// bare-`App` fixture or a test driver writing the phase from a frame schedule
/// still lands on the frame-level site, and the deferral is what keeps the
/// mission clock right for those too.
pub(crate) fn arm_mission_clock(mut runtime: ResMut<WorldContentRuntime>) {
    runtime.mission_clock = None;
}

/// Stamp time zero for the mission clock on the first simulation tick of the
/// mission.
///
/// # The bug this closes (latent since #475, lethal since #960)
///
/// `after_secs` used to be measured from a `Time::elapsed_secs()` reading taken
/// by `init_world_runtime`, a `Startup` system. `GamePhase` defaults to
/// `Lobby`, the world is loaded at `Startup`, and `Time<Virtual>` (and through
/// it `Time<Fixed>`) runs the whole time the crew is picking stations. The
/// `SimSet` chain is gated on `in_state(GamePhase::InProgress)`, so no trigger
/// was *evaluated* during the lobby - but the clock they would be evaluated
/// against kept running. After a 90-second lobby the first `InProgress` tick
/// therefore emitted `TimerElapsed { elapsed_secs: 90 }`, and every trigger
/// authored at 0, 45 and 90 fired in one dispatch batch. In `combat_test` that
/// is three waves and four comms bursts landing together on tick one; at a
/// five-minute lobby the entire eight-wave raid arrives at once and the victory
/// trigger is armed before the player has moved.
///
/// Nothing caught it because the only automated driver is headless, which
/// auto-starts on the first fixed step with nobody connected - elapsed is
/// approximately zero at `InProgress`, so the boot anchor and the mission
/// anchor agree to within a tick. It takes a lobby to tell them apart, and
/// `combat_test_wave_clock_measures_from_mission_start_not_app_boot`
/// (`tests/headless_runner.rs`) supplies one.
///
/// # Why here
///
/// `SimSet::Physics` is gated on `InProgress`, so "the first tick this system
/// runs" IS "the first simulation tick of the mission" - the gate does the
/// work, and there is no second predicate to keep in step with it. Running
/// inside the fixed schedule also means `Time` is `Time<Fixed>`, the same clock
/// `collect_world_events` and `tick_delayed_actions` read the anchor back
/// against, and the same clock two hosts would agree on: the reading is a whole
/// number of sim ticks, not a frame-pacing artifact.
///
/// Ordered before `collect_world_events` so a mission whose first wave is
/// authored at `after_secs = 0` still gets its `TimerElapsed { 0.0 }` on that
/// very tick rather than one tick late.
///
/// # What it does not do
///
/// Nothing re-anchors while a mission is running. In particular
/// `apply_pending_scenario_loads` does not: that applier MERGES a world TOML
/// into a live runtime (it appends trigger states, it does not replace them),
/// so re-anchoring there would rewind the clock the base world's own in-flight
/// `on_timer` triggers and `action_delays` are already scheduled against. A
/// genuinely new scenario arrives the other way - back to the lobby and in
/// again - and that path re-arms through [`arm_mission_clock`].
///
/// `world_config` gates the stamp for the same reason `init_world_runtime`
/// gates on it: an app with no world (native unit-test fixtures) must go on
/// seeing no `TimerElapsed` events at all, or `collect_world_events` would
/// start writing `WorldEventBuffer` every tick in apps that previously left it
/// untouched.
pub(crate) fn anchor_mission_clock(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
    time: Option<Res<bevy::time::Time>>,
) {
    // Both reads go through an immutable deref, so an already-anchored tick
    // does not mark `WorldContentRuntime` changed.
    if world_config.is_none() || runtime.mission_clock.is_some() {
        return;
    }
    // `Time` is optional so test apps without `TimePlugin` keep working: they
    // never anchor, and therefore never see `TimerElapsed` - same as before.
    if let Some(t) = time {
        runtime.mission_clock = Some(MissionClock::new(t.elapsed()));
    }
}

/// Arm every `[[deadline]]` this world authored, on the first simulation tick of
/// the mission (issue #1024).
///
/// Chained immediately after [`anchor_mission_clock`] and gated the same way, so
/// "the tick this runs for the first time" IS "the first simulation tick of the
/// mission": `SimSet::Physics` is gated on `InProgress`, and `due_secs` therefore
/// measures from mission start rather than from app boot. That is the #960 fix
/// applied to this vocabulary from the outset — a ninety-second lobby must not
/// retire a mission's deadlines before the crew has the con.
///
/// # It arms; it does not tick
///
/// The only thing arming *does* is push one ordinary
/// [`ScheduledCall`](crate::world::script::schedule::ScheduledCall) per deadline
/// onto `WorldScriptRuntime::pending_callbacks` — the queue
/// [`tick_script_callbacks`] already drains. No system introduced by issue #1024
/// walks the deadline table looking for due work, and this one runs its body
/// exactly once per mission. See [`crate::world::deadlines`] for why that is the
/// whole point.
///
/// # Determinism
///
/// A no-op for every world that authors no deadline: the early returns happen
/// before any `DerefMut`, so no change-detection tick flips and a deadline-free
/// run is byte-identical to one from before this system existed. Where deadlines
/// ARE authored, every fire tick is `now_tick + seconds_to_ticks(due_secs, hz)`
/// over values two peers both read from the world file, so both peers arm the
/// same ticks.
pub(crate) fn arm_mission_deadlines(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut script: Option<ResMut<WorldScriptRuntime>>,
    mut test_trace: Option<ResMut<crate::workshop::test_trace::TestTrace>>,
) {
    // Immutable reads only on the already-armed path, so an armed mission does
    // not mark `WorldContentRuntime` changed every tick.
    let Some(world_config) = world_config else {
        return;
    };
    if runtime.deadlines.armed || world_config.deadlines.is_empty() {
        return;
    }
    // A deadline fires a script fn, so a world with deadlines and no script
    // runtime has nothing to arm against. `validate_deadline_handlers` already
    // blocks such a world at load; this is the belt to that brace.
    let Some(script) = script.as_deref_mut() else {
        return;
    };
    let now_tick = sim_tick.map(|t| t.0).unwrap_or(0);
    let queued = runtime.deadlines.arm(
        &world_config.deadlines,
        &script.deadline_handlers,
        now_tick,
        world_config.global.sim_tick_hz,
    );
    if let Some(trace) = test_trace.as_deref_mut() {
        for callback in &queued {
            let line = script
                .function_lines
                .get(&callback.script_path)
                .and_then(|lines| lines.get(&callback.fn_name))
                .copied();
            trace.push(
                now_tick,
                Some(&callback.script_path),
                line,
                crate::workshop::test_protocol::TestTraceKind::CallbackScheduled {
                    function: callback.fn_name.clone(),
                    fire_tick: callback.fire_tick,
                },
            );
        }
    }
    // THE reuse, in one line: a deadline's firing is an entry on the EXISTING
    // deferred-callback queue.
    script.pending_callbacks.extend(queued);
}

/// Build the live workforce register from the world's `[[workforce]]` blocks,
/// mirroring each side's opening state into the flag store (issue #1035).
///
/// The twin of [`arm_mission_deadlines`] and deliberately smaller: a workforce
/// queues nothing, so there is no `ScheduledCall` to push and nothing to
/// retract later. What it does have to be is **early** — the register is what
/// decides whether a depot refuses a transfer, so it must hold the authored
/// answer before the first script handler runs rather than one tick after.
///
/// # Why the mirror flags are written straight into the store
///
/// Every *later* move of a workforce writes its flag as an ordinary
/// [`ActionCmd::MutateFlag`] through the trigger pipeline, so an
/// `on_flag_cleared` chains off it. The opening state deliberately does not: a
/// transition event on tick one would announce "the strike just started" for a
/// strike that was already happening when the crew arrived, and would fire
/// every trigger authored to watch for the settlement's opposite. This is the
/// same reading `InfrastructureState::from_config` takes when it level-evaluates
/// a degraded structure's thresholds instead of flipping them on tick one.
///
/// # Determinism
///
/// A no-op for every world that authors no workforce: the early returns happen
/// before any `DerefMut`, so no change-detection tick flips and a
/// workforce-free run is byte-identical to one from before this system existed.
pub(crate) fn arm_mission_workforces(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
) {
    // Immutable reads only on the already-armed path, so an armed mission does
    // not mark `WorldContentRuntime` changed every tick.
    let Some(world_config) = world_config else {
        return;
    };
    if runtime.workforce.armed || world_config.workforces.is_empty() {
        return;
    }
    let mirror = runtime.workforce.arm(&world_config.workforces);
    for write in mirror {
        runtime.flags.set_flag_value(&write.name, write.value);
    }
}
