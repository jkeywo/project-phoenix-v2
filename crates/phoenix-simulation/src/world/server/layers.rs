//! Layers.
use super::*;

// ── Pending scenario load system ─────────────────────────────────────────────

/// Bevy system: drain `PendingScenarioLoad`, reading and recording each world
/// TOML — and merging NOTHING.
///
/// It once merged the additively-loaded world's trigger states and comms
/// templates into the live `WorldContentRuntime`. Issue #985 deleted both
/// front-ends, so what survives is: read the TOML (recording it into the content
/// ledger), parse it to prove it parses, and mark the path loaded so a duplicate
/// is not re-read. Nothing enqueues into the queue it drains, either — see
/// [`PendingScenarioLoad`], and `apply_world_layer_changes` for the path a
/// supporting world that wants to CONTRIBUTE something takes instead.
///
/// On WASM the TOML string is not available at runtime (JS pre-fetches only the
/// initial world), so we push paths into the WASM-side pending-world queue and
/// the implementation returns early until the JS bridge delivers the TOML via
/// `wasm_push_world_toml`. On native targets `std::fs::read_to_string` is used.
///
/// It does NOT re-anchor the mission clock, and would not even if it merged
/// again: this is additive to a LIVE runtime, so the base world's in-flight
/// `on_timer` triggers and `action_delays` are still scheduled against the
/// running clock, and rewinding it here would postpone every one of them by
/// however long the mission had been going. A genuinely fresh scenario is
/// reached through the lobby instead, and `arm_mission_clock` re-anchors there.
pub(super) fn apply_pending_scenario_loads(
    mut pending: ResMut<PendingScenarioLoad>,
    mut runtime: ResMut<WorldContentRuntime>,
) {
    if pending.0.is_empty() {
        return;
    }

    let paths: Vec<String> = pending.0.drain(..).collect();

    for path in paths {
        // Dedup before any TOML read so a duplicate never touches the WASM fetch
        // queue — and is never re-marked, since it is already recorded in
        // `loaded_scenario_paths`. (Absorbed from the former `world::scenario`
        // decision layer, issue #1215.)
        if runtime.loaded_scenario_paths.contains(&path) {
            continue;
        }
        // Resolve the TOML (I/O; also records the text into the content ledger).
        // `None` means the WASM fetch is still in flight — re-queue for the next
        // frame without marking the path loaded.
        let Some(toml_str) = load_scenario_toml(&path) else {
            pending.0.push(path);
            continue;
        };
        // Route the parse (and the merged world's script-carry, dropped here) through
        // the one world-load sequence under `Merge`. A `MemoryReader` seeded with the
        // text `load_scenario_toml` already read — and already recorded — keeps the
        // decision pure; the returned `LedgerPlan` is dropped (the ledger recording
        // is owned above).
        //
        // The compiled set is DISCARDED, and stays discarded after issue #1045 gave
        // supporting worlds their scripts back: that slice wired the LAYER path
        // (`apply_world_layer_changes`), which has a `WorldLayerMap` entry to hang a
        // layer's ASTs and origin-tagged trigger states off and so can retract them
        // at `UnloadWorld`. This path has neither — a `PendingScenarioLoad` merge is
        // one-way and untracked — so merging a script here would be a set nothing
        // could ever take back out. Nothing enqueues into that queue today; if
        // something ever does, it wants the layer path, not a second merge.
        // `NoSiblingScripts` for the same reason: there is nothing here to resolve
        // a sibling FOR.
        let reader = MemoryReader::new([(path.clone(), toml_str)]);
        let request = LoadRequest::new(
            path.as_str(),
            &reader,
            &crate::world::script::load::NoSiblingScripts,
            LoadPolicy::Merge,
        );
        match load(request) {
            Ok(loaded) => {
                // The compiled set is discarded (see above), but its ledger digest
                // is not: `load_world_scripts` used to write that itself and now
                // returns it (issue #1241), so applying it here keeps the ledger
                // byte-identical to before the lift. The TOML `records` are
                // dropped for the reason the comment above gives — this function
                // already recorded that text through `load_scenario_toml`.
                for digest in &loaded.ledger.digests {
                    digest.apply();
                }
            }
            Err(LoadError::ParseFailed { message, .. }) => {
                bevy::log::error!(
                    "apply_pending_scenario_loads: failed to parse {path}: {message}"
                );
            }
            // Unreachable with a MemoryReader under Merge (see `world::layers`), but
            // mapped defensively rather than panicking.
            Err(other) => {
                bevy::log::error!("apply_pending_scenario_loads: failed to load {path}: {other}");
            }
        }
        // Mark loaded on success AND on parse failure — a broken file must not be
        // retried frame after frame.
        runtime.loaded_scenario_paths.insert(path);
    }
}

// ── World layer system (LoadWorld / UnloadWorld) ──────────────────────────────

/// Build a `ConfigCache` suitable for spawning entities from a world layer.
///
/// On WASM the global config cache (pre-loaded by the JS bridge) is returned
/// unchanged.  On native the global cache is always empty (no WASM pre-load
/// step), so we fall back to reading each template file from disk so that
/// `spawn_immediate_entities_internal` can resolve them.
///
/// # What this walk does NOT cover, and the I/O that leaves (issue #973)
///
/// It walks `_world_config.entities` — the layer's static `[[entity]]` blocks —
/// and nothing else. A `spawn_entity` **trigger action**'s `template_path` is
/// therefore not in the cache this returns, while
/// [`crate::world::validate::activation_findings`] checks every spawned
/// instance, triggers included. So on native, the activation gate resolves each
/// distinct trigger template through the filesystem at layer load.
///
/// That is real I/O on a runtime transition that previously did none. It is
/// bounded — once per distinct template per layer load, never per frame, and
/// `#[cfg(target_arch = "wasm32")]` builds never touch a filesystem at all —
/// and it does not move the content digest, because
/// [`crate::content_ledger`] is keyed by canonical path and records the same
/// bytes the eager walk already recorded. Widening this walk to trigger
/// templates would trade the I/O here for the same I/O one step earlier, so it
/// is recorded rather than pre-emptively "fixed".
pub(super) fn build_layer_config_cache(
    _world_config: &crate::world::config::WorldConfig,
) -> crate::entities::config_cache::ConfigCache {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut cache = crate::entities::config_cache::get_config_cache();
        for entity in &_world_config.entities {
            if cache.contains_key(&entity.template_path) {
                continue;
            }
            if std::fs::metadata(&entity.template_path).is_err()
                && crate::entities::config_cache::mod_pack_overlay_get(&entity.template_path)
                    .is_none()
            {
                // Template not on disk (e.g. test fixture); skip silently.
                // spawn_immediate_entities_internal logs and continues for
                // missing templates.
                continue;
            }
            // The `includes` closure resolves here too (issue #869), so a
            // composed hull referenced by a world reaches the layer cache fully
            // merged — the same single document the browser preload assembles.
            match crate::entities::include_resolve::load_entity_config(&entity.template_path) {
                Ok(cfg) => {
                    cache.insert(entity.template_path.clone(), cfg);
                }
                Err(e) => {
                    // Warn only: this builds a cache, it does not decide
                    // whether the world activates. Since #906 the decision
                    // belongs to `validate_template_composition`, which
                    // `spawn_immediate_entities_internal` consults before it
                    // spawns anything — so a composition failure now blocks the
                    // whole world instead of quietly costing it one entity.
                    bevy::log::warn!(
                        "build_layer_config_cache: failed to resolve '{}': {e}",
                        entity.template_path
                    );
                }
            }
        }
        cache
    }

    #[cfg(target_arch = "wasm32")]
    {
        crate::entities::config_cache::get_config_cache()
    }
}

/// Bring one already-evaluated layer into the world: merge its scripts, register
/// its names, spawn its entities, and record it in the `WorldLayerMap`.
///
/// Shared by both routes into `apply_world_layer_changes`' Loaded case — the
/// same-tick one and the [`WorldLayerChange::DeferredApply`] one a tick later —
/// so a scripted layer and a scriptless one are brought in by identical code and
/// the deferral cannot drift into a second implementation.
///
/// `script_runtime` is `None` only for a layer that authored no script; the
/// callers refuse a scripted layer with no runtime before reaching here.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_loaded_layer(
    path: &str,
    loader_path: Option<String>,
    layer: crate::world::layers::LoadedLayer,
    commands: &mut Commands,
    layer_map: &mut WorldLayerMap,
    runtime: &mut WorldContentRuntime,
    script_runtime: Option<&mut WorldScriptRuntime>,
    id_mint: Option<&crate::world_id::EntityMint>,
    now_tick: u64,
    root_tick_hz: f32,
    test_trace: Option<&mut crate::workshop::test_trace::TestTrace>,
) {
    let crate::world::layers::LoadedLayer {
        mut name_to_uuid_inserts,
        mut scenario_config,
        emit_world_loaded,
        scripts,
    } = layer;

    if scenario_config.planning.is_some() {
        bevy::log::error!(
            "layer {path}: maintenance planning is supported only on the accepted base world"
        );
        return;
    }

    if scenario_config.gm_npc_doctrine_palette.iter().any(|entry| {
        runtime
            .gm_npc_doctrine_palette
            .iter()
            .any(|live| live.id == entry.id)
    }) {
        bevy::log::error!("layer {path} has a duplicate GM NPC doctrine palette id");
        return;
    }
    if scenario_config.gm_objective_palette.iter().any(|entry| {
        runtime
            .gm_objective_palette
            .iter()
            .any(|live| live.id == entry.id && live.instance_id() == entry.instance_id())
    }) {
        bevy::log::error!("layer {path} has a duplicate GM Objective palette id");
        return;
    }

    // A named layer entity is authored identity, not one incarnation of the
    // layer. Unload deliberately leaves the live name registry intact, so a
    // later load can restore the same UUID instead of exposing a removal and
    // reappearance as two unrelated contacts (issue #1296). Keep the parsed
    // config and the registrations in lockstep: the former is what spawning
    // reads, while the latter is what the live runtime records below. Anonymous
    // layer entities have no entry here and retain their mint-on-load policy.
    for (name, uuid) in &mut name_to_uuid_inserts {
        let Some(existing_uuid) = runtime.name_to_uuid.get(name).cloned() else {
            continue;
        };
        *uuid = existing_uuid.clone();
        scenario_config
            .name_to_uuid
            .insert(name.clone(), existing_uuid);
    }

    // Merge the layer's compiled `[script]` set into the live script runtime
    // (issue #1045). Script-free layers take the `None` branch without changing
    // the live runtime.
    let script_units = match (scripts, script_runtime) {
        (Some(compiled), Some(sr)) => {
            let calls = runtime.deadlines.arm_scoped(
                &scenario_config.deadlines,
                &compiled.deadline_handlers,
                now_tick,
                root_tick_hz,
                Some(path),
            );
            if let Some(trace) = test_trace {
                for callback in &calls {
                    let line = compiled
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
            sr.pending_callbacks.extend(calls);
            merge_layer_scripts(path, compiled, runtime, sr)
        }
        (Some(_), None) => {
            // Refused by both callers before this point; belt to that brace.
            bevy::log::error!(
                target: "world",
                "apply_loaded_layer: layer {path} carries scripts with no runtime to \
                 merge into; dropping them"
            );
            Vec::new()
        }
        (None, _) => Vec::new(),
    };

    // Register the layer's named entities in the live name_to_uuid map.
    for (name, uuid) in name_to_uuid_inserts {
        runtime.name_to_uuid.insert(name, uuid);
    }

    // Spawn the layer's [[entity]] blocks into the ECS. On native the global
    // config cache is always empty (no WASM pre-load step), so we build a local
    // cache by reading each referenced template from disk. WASM uses the
    // pre-loaded global cache as normal.
    let config_cache = build_layer_config_cache(&scenario_config);
    let spawned_entities = spawn_immediate_entities_internal(
        commands,
        &scenario_config,
        &config_cache,
        Some(&runtime.flags),
        id_mint,
    );
    // Stamp each entity's origin layer (issue #891 review finding 1) so
    // `entity_flag_chain` can read it in O(1) instead of scanning `WorldLayerMap`
    // for it later — the second of the two spawn sites.
    for &spawned in &spawned_entities {
        commands
            .entity(spawned)
            .insert(EntityOriginLayer(path.to_string()));
    }

    let delayed_unload_resolve = matches!(
        scenario_config.delayed_unload_policy,
        crate::world::config::DelayedUnloadPolicy::Resolve
    );
    let activation_order = layer_map
        .0
        .values()
        .filter(|layer| layer.is_active)
        .map(|layer| layer.activation_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    layer_map.0.insert(
        path.to_string(),
        WorldRuntime {
            inspection_config: Some(Box::new(scenario_config.clone())),
            is_active: true,
            activation_order,
            spawned_entities,
            anchors: scenario_config.anchors.clone(),
            flags: crate::world::flags::FlagStore::new(),
            loader_path,
            owned_objective_ids: Vec::new(),
            delayed_unload_resolve,
            script_units,
        },
    );

    for mut entry in scenario_config.gm_objective_palette.clone() {
        entry.origin_layer = Some(path.to_string());
        runtime.gm_objective_palette.push(entry);
    }
    for mut entry in scenario_config.gm_npc_doctrine_palette.clone() {
        entry.origin_layer = Some(path.to_string());
        runtime.gm_npc_doctrine_palette.push(entry);
    }

    // Expose WorldLoaded only after every part of activation is live: ASTs,
    // handlers, deadlines, entities, flags and the layer map.
    if emit_world_loaded {
        runtime.pending_world_events.push(WorldEvent::WorldLoaded);
    }
}

/// Snapshot the doctrine-anchor namespace visible to one layer evaluation.
///
/// Rebuilt immediately before every load decision, rather than once per drain:
/// a scriptless layer applied earlier in the same authored batch is already in
/// `layer_map` and may legitimately provide an anchor to the next layer. Failed
/// sentinels and inactive entries contribute nothing.
pub(super) fn layer_validation_context<'a>(
    base_world: Option<&crate::world::config::WorldConfig>,
    layer_map: &WorldLayerMap,
    template_loader: &'a dyn crate::entities::loader::TemplateLoader,
    fragment_source: &'a dyn crate::entities::include_resolve::FragmentSource,
) -> LayerValidationContext<'a> {
    let root_anchors = base_world
        .into_iter()
        .flat_map(|world| world.anchors.keys().cloned());
    let layer_anchors = layer_map
        .0
        .values()
        .filter(|layer| layer.is_active)
        .flat_map(|layer| layer.anchors.keys().cloned());
    LayerValidationContext::new(
        template_loader,
        fragment_source,
        root_anchors.chain(layer_anchors),
    )
}

/// Bevy system: drain `PendingWorldLayerChanges` and apply each `LoadWorld` or
/// `UnloadWorld` command to `WorldLayerMap` and `WorldContentRuntime`.
///
/// `LoadWorld` parses the TOML, spawns the layer's entities, merges its compiled
/// `[script]` set into the live runtime, and stores a `WorldRuntime` snapshot
/// keyed by path so `UnloadWorld` can reverse it.
///
/// `UnloadWorld` removes the stored snapshot, despawns the layer's entities, and
/// retracts exactly the scenario logic it brought: the trigger states origin-tagged
/// with its path (each with its parallel `handlers` entry — see
/// [`crate::world::trigger_registry::WorldTriggerRegistry::remove_layer`]), the AST units it added, and any deadline
/// declaration or queued `after(..)` callback keyed to one of those units.
///
/// # The script runtime is created on demand, one tick early (issue #1045)
///
/// `WorldScriptRuntime` is absent for a script-free base world, so a layer that
/// brings the session's first `[script]` block has nowhere to merge into. This
/// system inserts an empty runtime and RE-QUEUES that load — the same answer the
/// `TomlUnavailable` arm gives a world that is not ready yet — rather than merging
/// into one it created on the spot.
///
/// The reason is ordering, not tidiness. A `Commands` insert lands at the
/// schedule's sync point, which may fall AFTER `tick_trigger_pipeline` has already
/// run this tick. Merging in the body would then publish the layer's trigger
/// states (on `WorldContentRuntime`, written directly) and its `WorldLoaded` event
/// a whole tick before the `handlers` that describe them — so the pipeline would
/// evaluate each `on_world_loaded` against an absent runtime, latch it `fired`,
/// and the layer's opening handler would never run at all. Waiting a tick means
/// the paired registrations become visible only with their execution runtime.
///
/// The re-queued item is a [`WorldLayerChange::DeferredApply`] carrying the
/// EVALUATED layer, not a fresh `Load`. Re-evaluating would reparse the layer and
/// mint different `[[entity]]` UUIDs. Carrying the outcome means those UUIDs are
/// minted exactly once, so the deferral costs a tick and nothing else.
pub(super) fn apply_world_layer_changes(
    mut commands: Commands,
    mut pending: ResMut<PendingWorldLayerChanges>,
    mut layer_map: ResMut<WorldLayerMap>,
    mut runtime: ResMut<WorldContentRuntime>,
    // The live script runtime a layer's compiled set merges into, and retracts
    // from at unload (issue #1045). `Option` because a script-free world has
    // none — every shipped world, until one authors a scripted layer.
    mut script_runtime: Option<ResMut<WorldScriptRuntime>>,
    id_mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Entity as usize }>,
    // Layer-owned objective cleanup on unload (issue #751). `Option` so bare
    // `App` fixtures without an `ObjectiveManagerRes` still run the loader.
    mut objectives: Option<ResMut<ObjectiveManagerRes>>,
    // Related runtime state to prune when a layer's objectives are removed
    // (issue #752): a captain priority boost pointing at a removed objective,
    // and any per-ship route cursor keyed to it.
    mut captain_boost: Option<ResMut<crate::server_app::CaptainPriorityBoost>>,
    mut objective_cursors_q: Query<&mut crate::ai::server::ObjectiveCursors>,
    // Consecutive `TomlUnavailable` answers per path, so a source that never
    // arrives is refused rather than re-queued forever (issue #1045). A `Local`
    // because it is scheduling bookkeeping, not world state: nothing reads it,
    // no digest folds it, and a fresh app starts it empty.
    mut toml_retries: Local<HashMap<String, u32>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    base_world: Option<Res<crate::world::config::WorldConfig>>,
    // A layer-owned dialogue is executable scenario state just like its queued
    // callback. Keep the comms surfaces optional for bare loader fixtures, but
    // retire all of them atomically when the owner unloads.
    mut comms_runtime: Option<ResMut<crate::comms::server::CommsRuntime>>,
    mut comms_inbox: Option<ResMut<crate::comms::server::CommsInboxRes>>,
    mut on_screen_message: Option<ResMut<crate::comms::server::OnScreenMessage>>,
    mut test_trace: Option<ResMut<crate::workshop::test_trace::TestTrace>>,
) {
    if pending.0.is_empty() {
        return;
    }

    let changes: Vec<WorldLayerChange> = pending.0.drain(..).collect();
    let now_tick = sim_tick.as_deref().map_or(0, |tick| tick.0);
    let root_tick_hz = base_world
        .as_deref()
        .map_or(SchedClock::ZERO.tick_hz, |world| world.global.sim_tick_hz);

    // Resolves a layer's top-level `script = "wave.rhai"` sibling. Built once per
    // drain: `world::layers` keeps the read injected so the decision stays pure,
    // and this applier is the impure half that owns filesystem / bridge access.
    let script_resolver = crate::entities::config_cache::production_script_resolver();
    let template_loader = crate::entities::loader::WasmTemplateLoader;
    let fragment_source = crate::entities::include_resolve::HostFragmentSource;

    for change in changes {
        match change {
            WorldLayerChange::Load { path, loader_path } => {
                // Decisions (dedup / requeue / parse handling / origin
                // tagging / name→UUID assignment) are pure (`world::layers`);
                // this applier resolves the TOML (I/O), spawns entities,
                // merges comms, and mutates the layer map. The dedup check
                // happens before the TOML read so a duplicate never touches
                // the WASM fetch queue.
                // A layer whose apply is DEFERRED counts as loaded for dedup
                // purposes even though it has no map entry yet (issue #1045).
                // Without this, two `Load`s for the same path in one drain — a
                // duplicated `extra_worlds` entry, or a script calling
                // `load_world` twice in a tick — would each evaluate and each
                // defer, and the second apply would overwrite the first's map
                // entry, leaking its spawned entities. It also keeps the second
                // one off the world source, which on wasm can only be read once.
                let deferred_already = pending.0.iter().any(|queued| {
                    matches!(
                        queued,
                        WorldLayerChange::DeferredApply { path: p, .. }
                            | WorldLayerChange::AwaitingScript { path: p, .. }
                            if p == &path
                    )
                });
                let already_loaded = layer_map.0.contains_key(&path) || deferred_already;
                #[cfg(target_arch = "wasm32")]
                if let crate::entities::config_cache::WorldFetchState::Failed(message) =
                    crate::entities::config_cache::world_fetch_state(&path)
                {
                    bevy::log::error!(
                        target: "world",
                        "apply_world_layer_changes: refusing {path} atomically after fetch \
                         failure: {message}"
                    );
                    layer_map.0.insert(path, WorldRuntime::default());
                    continue;
                }
                let toml_str = if already_loaded {
                    None
                } else {
                    load_scenario_toml(&path)
                };
                #[cfg(target_arch = "wasm32")]
                if let Some(toml) = toml_str.as_deref() {
                    if let Some(script_path) =
                        crate::world::script::load::declared_sibling_script_path(&path, toml)
                    {
                        use crate::entities::config_cache::WorldFetchState;
                        match crate::entities::config_cache::world_fetch_state(&script_path) {
                            WorldFetchState::Ready(_) => {}
                            WorldFetchState::Failed(message) => {
                                bevy::log::error!(
                                    target: "world",
                                    "apply_world_layer_changes: refusing {path} atomically: \
                                     sibling script {script_path} failed to fetch: {message}"
                                );
                                layer_map.0.insert(path, WorldRuntime::default());
                                continue;
                            }
                            WorldFetchState::NotRequested | WorldFetchState::Pending => {
                                crate::entities::config_cache::request_world_fetch(
                                    script_path.clone(),
                                );
                                pending.0.push(WorldLayerChange::AwaitingScript {
                                    path,
                                    loader_path,
                                    toml: toml.to_string(),
                                    script_path,
                                });
                                continue;
                            }
                        }
                    }
                }
                let validation = layer_validation_context(
                    base_world.as_deref(),
                    &layer_map,
                    &template_loader,
                    &fragment_source,
                );
                let result = evaluate_layer_load(
                    &path,
                    already_loaded,
                    toml_str.as_deref(),
                    &script_resolver,
                    &validation,
                    || {
                        crate::world_id::mint_live_id_with(
                            id_mint.as_deref(),
                            crate::world_id::IdNamespace::Entity,
                        )
                    },
                );
                for warning in &result.warnings {
                    bevy::log::error!("apply_world_layer_changes: {warning}");
                }
                // The ledger writes the evaluation gathered (issue #1241): the
                // layer's compiled-script digest, which the loader used to write
                // itself. Applied HERE — right after the evaluation, before the
                // outcome is acted on and before any deferral — so it lands at the
                // same moment the eager write did, on every outcome the eager write
                // covered. Empty for a script-free layer and for every outcome
                // that never reached a compile.
                result.ledger.apply();
                match result.outcome {
                    LayerLoadOutcome::AlreadyLoaded => {
                        // De-duplicate, no-op.
                        toml_retries.remove(&path);
                        continue;
                    }
                    LayerLoadOutcome::TomlUnavailable => {
                        // WASM: re-queue until the fetch completes — but not
                        // forever. A world source can become permanently
                        // unreadable mid-session (the browser's pending map is
                        // CONSUMED by a read and its fetch guard refuses to
                        // re-ask, which a contrived `[Load A, Unload A, Load A]`
                        // in one drain reaches), and an unbounded re-queue turns
                        // that into a silent spin with nothing in the log. Refuse
                        // the layer loudly once the retries are plainly not going
                        // anywhere, the same way a broken file is refused.
                        let attempts = toml_retries.entry(path.clone()).or_insert(0);
                        *attempts += 1;
                        if *attempts > MAX_TOML_RETRIES {
                            bevy::log::error!(
                                target: "world",
                                "apply_world_layer_changes: giving up on {path} after \
                                 {MAX_TOML_RETRIES} drains with no world source; the layer \
                                 is refused and will not be retried"
                            );
                            toml_retries.remove(&path);
                            layer_map.0.insert(path, WorldRuntime::default());
                        } else {
                            pending.0.push(WorldLayerChange::Load { path, loader_path });
                        }
                    }
                    LayerLoadOutcome::ParseFailed => {
                        // Insert an empty entry so we don't retry a broken file.
                        toml_retries.remove(&path);
                        layer_map.0.insert(path, WorldRuntime::default());
                    }
                    LayerLoadOutcome::Loaded(layer) => {
                        toml_retries.remove(&path);
                        // A scripted layer needs a live `WorldScriptRuntime` to merge
                        // into, and a script-free base world has none. Insert an empty
                        // one and re-queue the EVALUATED layer rather than merging into
                        // one created on the spot — see the system docs for why the
                        // extra tick is the whole point, and why what is re-queued is
                        // the outcome rather than the load. Script-free layers skip
                        // this branch entirely.
                        if layer.scripts.is_some() && script_runtime.is_none() {
                            commands.insert_resource(WorldScriptRuntime::empty());
                            pending.0.push(WorldLayerChange::DeferredApply {
                                path,
                                loader_path,
                                layer,
                            });
                            continue;
                        }
                        apply_loaded_layer(
                            &path,
                            loader_path,
                            *layer,
                            &mut commands,
                            &mut layer_map,
                            &mut runtime,
                            script_runtime.as_deref_mut(),
                            id_mint.as_deref(),
                            now_tick,
                            root_tick_hz,
                            test_trace.as_deref_mut(),
                        );
                    }
                }
            }
            WorldLayerChange::AwaitingScript {
                path,
                loader_path,
                toml,
                script_path,
            } => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let _ = (path, loader_path, toml, script_path);
                    unreachable!("AwaitingScript is produced only by the wasm fetch edge");
                }
                #[cfg(target_arch = "wasm32")]
                {
                    use crate::entities::config_cache::WorldFetchState;
                    match crate::entities::config_cache::world_fetch_state(&script_path) {
                        WorldFetchState::NotRequested | WorldFetchState::Pending => {
                            crate::entities::config_cache::request_world_fetch(script_path.clone());
                            pending.0.push(WorldLayerChange::AwaitingScript {
                                path,
                                loader_path,
                                toml,
                                script_path,
                            });
                        }
                        WorldFetchState::Failed(message) => {
                            bevy::log::error!(
                                target: "world",
                                "apply_world_layer_changes: refusing {path} atomically: sibling \
                                 script {script_path} failed to fetch: {message}"
                            );
                            layer_map.0.insert(path, WorldRuntime::default());
                        }
                        WorldFetchState::Ready(_) => {
                            let validation = layer_validation_context(
                                base_world.as_deref(),
                                &layer_map,
                                &template_loader,
                                &fragment_source,
                            );
                            let result = evaluate_layer_load(
                                &path,
                                layer_map.0.contains_key(&path),
                                Some(&toml),
                                &script_resolver,
                                &validation,
                                || {
                                    crate::world_id::mint_live_id_with(
                                        id_mint.as_deref(),
                                        crate::world_id::IdNamespace::Entity,
                                    )
                                },
                            );
                            for warning in &result.warnings {
                                bevy::log::error!("apply_world_layer_changes: {warning}");
                            }
                            result.ledger.apply();
                            match result.outcome {
                                LayerLoadOutcome::AlreadyLoaded => {}
                                LayerLoadOutcome::TomlUnavailable => {
                                    unreachable!("retained layer TOML cannot become unavailable")
                                }
                                LayerLoadOutcome::ParseFailed => {
                                    layer_map.0.insert(path, WorldRuntime::default());
                                }
                                LayerLoadOutcome::Loaded(layer) => {
                                    if layer.scripts.is_some() && script_runtime.is_none() {
                                        commands.insert_resource(WorldScriptRuntime::empty());
                                        pending.0.push(WorldLayerChange::DeferredApply {
                                            path,
                                            loader_path,
                                            layer,
                                        });
                                    } else {
                                        apply_loaded_layer(
                                            &path,
                                            loader_path,
                                            *layer,
                                            &mut commands,
                                            &mut layer_map,
                                            &mut runtime,
                                            script_runtime.as_deref_mut(),
                                            id_mint.as_deref(),
                                            now_tick,
                                            root_tick_hz,
                                            test_trace.as_deref_mut(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            WorldLayerChange::DeferredApply {
                path,
                loader_path,
                layer,
            } => {
                // The empty `WorldScriptRuntime` queued alongside this on the previous
                // tick is visible now, so the merge and the `WorldLoaded` it fires from
                // land in this one body. Nothing is re-read or re-evaluated.
                if layer.scripts.is_some() && script_runtime.is_none() {
                    // Cannot happen: the insert and this queue entry were written in
                    // the same body, and commands are applied before the next run.
                    // Refuse the layer rather than drop its logic silently or defer
                    // again (which is how a queue starts spinning).
                    bevy::log::error!(
                        target: "world",
                        "apply_world_layer_changes: deferred layer {path} still has no \
                         WorldScriptRuntime to merge into; refusing it"
                    );
                    layer_map.0.insert(path, WorldRuntime::default());
                    continue;
                }
                apply_loaded_layer(
                    &path,
                    loader_path,
                    *layer,
                    &mut commands,
                    &mut layer_map,
                    &mut runtime,
                    script_runtime.as_deref_mut(),
                    id_mint.as_deref(),
                    now_tick,
                    root_tick_hz,
                    test_trace.as_deref_mut(),
                );
            }
            WorldLayerChange::Unload(path) => {
                // Cancel any load for this path still sitting in the queue, so
                // `[Load A, Unload A]` in one drain nets to NOT LOADED (issue #1045).
                // Two kinds can be waiting: a `DeferredApply` this drain just queued
                // for a scripted layer, and a `Load` re-queued because its TOML has
                // not arrived. Without this the unload finds no map entry, no-ops,
                // and the load lands a tick later — an ordering inversion that turns
                // "load it then take it away" into "it is loaded".
                //
                // Cancelling a `DeferredApply` leaves the empty
                // `WorldScriptRuntime` its `Load` already queued for insertion
                // resident, deliberately: the insert is idempotent, an empty
                // runtime changes no behaviour (every read of it finds nothing),
                // and the next scripted layer merges straight into it instead of
                // waiting a tick. Chasing the `Commands` insert back would cost
                // more than it saves.
                pending.0.retain(|queued| match queued {
                    WorldLayerChange::Load { path: p, .. }
                    | WorldLayerChange::DeferredApply { path: p, .. }
                    | WorldLayerChange::AwaitingScript { path: p, .. } => p != &path,
                    WorldLayerChange::Unload(_) => true,
                });

                let Some(layer) = layer_map.0.remove(&path) else {
                    continue; // Not loaded - no-op.
                };

                // Retire every live comms surface owned by this layer before
                // the response handler can observe it. `script_path` cannot be
                // used as ownership: two loaded layers may share one sibling
                // AST, and the survivor must retain its own dialogues while the
                // departing owner's offered choices become unaddressable.
                if let Some(comms) = comms_runtime.as_deref_mut() {
                    crate::comms::server::retire_layer_owned_dialogues(
                        &path,
                        comms,
                        comms_inbox.as_deref_mut(),
                        on_screen_message.as_deref_mut(),
                    );
                }

                // Despawn ECS entities that were spawned when this layer loaded.
                // Use try_despawn: entities may have already died (e.g. hull = 0) before the layer unloads.
                for entity in &layer.spawned_entities {
                    commands.entity(*entity).try_despawn();
                }

                // Retraction pairs rows even in fixtures with no script runtime.
                let removed = remove_layer_triggers(&mut runtime, &path);
                if removed > 0 {
                    bevy::log::debug!(target: "world",
                        "apply_world_layer_changes: unloaded {path} retracted {removed} trigger(s)");
                }
                if let Some(sr) = script_runtime.as_deref_mut() {
                    for call in runtime.deadlines.remove_origin(&path) {
                        sr.pending_callbacks.retract(&call);
                    }
                    sr.pending_callbacks
                        .0
                        .retain(|call| call.origin_layer.as_deref() != Some(path.as_str()));
                    sr.pending_comms_opens
                        .retain(|open| open.origin_layer.as_deref() != Some(path.as_str()));
                    for unit in &layer.script_units {
                        let remove_unit = sr.ast_owners.get_mut(unit).is_some_and(|owners| {
                            owners.remove(&Some(path.clone()));
                            owners.is_empty()
                        });
                        if remove_unit {
                            sr.ast_owners.remove(unit);
                            sr.asts.remove(unit);
                            sr.function_lines.remove(unit);
                        }
                    }
                    // Every callback still waiting on its fire tick has already
                    // been retracted by exact `origin_layer`; that includes named
                    // deadlines, ordinary `after(..)` calls, and callbacks nested
                    // from either one. Shared ASTs remain until their last owner.
                }

                runtime
                    .gm_objective_palette
                    .retain(|entry| entry.origin_layer.as_deref() != Some(path.as_str()));
                runtime
                    .gm_npc_doctrine_palette
                    .retain(|entry| entry.origin_layer.as_deref() != Some(path.as_str()));
                // Remove objectives this layer's triggers added (issue #751)
                // and prune the runtime state that referenced them (issue #752):
                // a captain priority boost pointing at a removed objective, and
                // any per-ship route cursor keyed to it. Otherwise a stale boost
                // would keep re-scoring a gone objective and a re-added same-id
                // objective would inherit the old cursor's waypoint index.
                for id in &layer.owned_objective_ids {
                    if let Some(obj) = objectives.as_deref_mut() {
                        obj.0.remove(id);
                    }
                    if let Some(boost) = captain_boost.as_deref_mut() {
                        boost.prune_objective(id);
                    }
                    for mut cursors in objective_cursors_q.iter_mut() {
                        cursors.0.retain(|c| &c.objective_id != id);
                    }
                }

                // Cancel or resolve this layer's pending delayed actions by
                // the authored policy (issue #751). Pure partition; the
                // resolved actions are pulled to fire immediately on the next
                // `tick_delayed_actions`.
                let queued = std::mem::take(&mut runtime.pending_delayed_actions);
                runtime.pending_delayed_actions =
                    crate::world::delayed::partition_delayed_actions_on_unload(
                        queued,
                        &path,
                        layer.delayed_unload_resolve,
                    );
            }
        }
    }
}

/// Load a world TOML string for the given path.
///
/// - **Native**: uses `std::fs::read_to_string` (for tests and dev builds).
/// - **WASM**: checks the pending world TOML queue populated by JS via
///   `wasm_push_world_toml`; returns `None` if the fetch is not yet complete.
pub(super) fn load_scenario_toml(path: &str) -> Option<String> {
    // Issue #935: layer worlds loaded through this function (extra worlds,
    // additive layers) are authored content too — a designer editing one
    // moves nothing in the content digest unless it is recorded. This does
    // NOT reset the ledger: a layer load is additive to the same run, not a
    // new scenario/world load (see `content_ledger`'s reset-semantics docs).
    let text = load_scenario_toml_text(path);
    if let Some(text) = &text {
        crate::content_ledger::record(path, text);
    }
    text
}

pub(super) fn load_scenario_toml_text(path: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::content_fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::entities::config_cache::resolved_world_source(path).or_else(|| {
            // Fire a JS fetch request if we haven't already.
            crate::entities::config_cache::request_world_fetch(path.to_string());
            None
        })
    }
}

/// Remove paired trigger entries and only their exact GM Pause identities.
pub(super) fn remove_layer_triggers(runtime: &mut WorldContentRuntime, path: &str) -> usize {
    let removed_ids: Vec<_> = runtime
        .triggers
        .iter()
        .filter(|state| state.origin_layer.as_deref() == Some(path))
        .filter_map(crate::gm::operations::event::state_event_id)
        .collect();
    let removed = runtime.triggers.remove_layer(path);
    for id in removed_ids {
        runtime.paused_gm_events.remove(&id);
    }
    removed
}
