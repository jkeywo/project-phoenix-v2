//! Server-side asset pre-cache system.
//!
//! When the lobby opens, `begin_asset_preload` recursively walks the scenario
//! (world TOML + all referenced entity templates + triggers + comms responses)
//! to discover every renderable asset (GLB models, radar icons, model rig
//! sidecars). It calls `asset_server.load()` for each and tracks readiness.
//!
//! If the captain presses Engage before preload completes, the phase
//! transitions to `GamePhase::Loading` instead of `InProgress`. During
//! `Loading` the system broadcasts `LoadingProgress { fraction }` to clients
//! at ~2 Hz. Once all assets are ready, it auto-transitions to `InProgress`
//! and sends `GameStarted`.

use std::collections::{HashMap, HashSet};

use bevy::asset::LoadState;
use bevy::prelude::*;

use crate::core::messages::{GamePhase, LoadingProgress, ServerMessage};
use crate::entity_config::EntityConfig;
use crate::lobby::server::LobbyOutbox;
use crate::lobby::Target;
use crate::model_rig::sidecar_path;
use crate::world::config::{parse_world, CommsTemplate, TriggerAction, WorldConfig};

// ── AssetManifest ──────────────────────────────────────────────────────────

/// All renderable assets discovered by walking the scenario recursively.
#[derive(Clone, Debug, Default)]
pub struct AssetManifest {
    /// GLB model paths relative to `assets/` (e.g. `"models/dynasty_destroyer.glb"`).
    pub glb_models: Vec<String>,
    /// Radar icon paths relative to `assets/` (e.g. `"radar_icons/Icon-Destroyer.png"`).
    pub radar_icons: Vec<String>,
    /// Full sidecar paths (e.g. `"assets/models/dynasty_destroyer.model.toml"`).
    pub sidecars: Vec<String>,
    /// Sub-world TOML paths discovered (for tracking purposes; not loaded here).
    pub sub_worlds: Vec<String>,
}

// ── Icon naming convention (mirrors gui/radar.rs) ─────────────────────────

/// Convert an icon name (e.g. `"destroyer"`) to an asset path
/// (`"radar_icons/Icon-Destroyer.png"`). Same convention as the radar widget.
pub fn icon_asset_path(icon_name: &str) -> String {
    let mut chars = icon_name.chars();
    let capitalized = match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    format!("radar_icons/Icon-{capitalized}.png")
}

// ── Recursive discovery (pure, no Bevy deps) ──────────────────────────────

/// Walk every `TriggerAction` that references file paths and collect them.
fn collect_action_paths(
    actions: &[TriggerAction],
    out_entities: &mut Vec<String>,
    out_worlds: &mut Vec<String>,
) {
    for action in actions {
        match action {
            TriggerAction::LoadWorld { path } => out_worlds.push(path.clone()),
            TriggerAction::SpawnEntity { template_path, .. } => {
                out_entities.push(template_path.clone());
            }
            _ => {}
        }
    }
}

/// Walk comms dialogue tree for `SpawnEntity` / `LoadWorld` actions.
fn collect_comms_action_paths(
    templates: &[CommsTemplate],
    out_entities: &mut Vec<String>,
    out_worlds: &mut Vec<String>,
) {
    fn walk_node(
        node: &crate::world::config::CommsDialogueNode,
        out_entities: &mut Vec<String>,
        out_worlds: &mut Vec<String>,
    ) {
        for response in &node.responses {
            collect_action_paths(&response.actions, out_entities, out_worlds);
            if let Some(ref follow_up) = response.follow_up {
                walk_node(follow_up, out_entities, out_worlds);
            }
        }
    }
    for tmpl in templates {
        walk_node(&tmpl.node, out_entities, out_worlds);
    }
}

/// Extract GLB model, radar icon, and sidecar paths from one entity config.
fn discover_entity_config_assets(
    config: &EntityConfig,
    manifest: &mut AssetManifest,
) {
    // GLB model + sidecar
    if let Some(ref mesh) = config.mesh {
        if let Some(ref model_path) = mesh.model {
            let rel = model_path
                .strip_prefix("assets/")
                .unwrap_or(model_path);
            if !manifest.glb_models.contains(&rel.to_string()) {
                manifest.glb_models.push(rel.to_string());
            }
            let sc = sidecar_path(model_path, mesh.variant.as_deref());
            if !manifest.sidecars.contains(&sc) {
                manifest.sidecars.push(sc);
            }
        }
    }
    // Radar icon
    if let Some(ref radar) = config.radar_appearance {
        if let Some(ref icon) = radar.icon {
            let icon_path = icon_asset_path(icon);
            if !manifest.radar_icons.contains(&icon_path) {
                manifest.radar_icons.push(icon_path);
            }
        }
    }
    // Nested entity templates (asteroid subtypes)
    if let Some(ref field) = config.asteroid_field {
        for a_path in &field.asteroid_type_paths {
            if !manifest.sub_worlds.contains(a_path) {
                // Tracked via entity_paths concept; stored separately
                // since they'll be looked up in the config cache.
            }
        }
    }
}

/// Recursively discover assets from a single entity template path.
fn walk_entity(
    template_path: &str,
    config_cache: &HashMap<String, EntityConfig>,
    seen_entities: &mut HashSet<String>,
    manifest: &mut AssetManifest,
) {
    if !seen_entities.insert(template_path.to_string()) {
        return;
    }
    let Some(config) = config_cache.get(template_path) else {
        return;
    };
    discover_entity_config_assets(config, manifest);

    // Recurse into nested asteroid entity templates
    if let Some(ref field) = config.asteroid_field {
        for p in &field.asteroid_type_paths {
            walk_entity(p, config_cache, seen_entities, manifest);
        }
        for p in &field.cosmetic_type_paths {
            walk_entity(p, config_cache, seen_entities, manifest);
        }
    }
}

/// Walk a single parsed `WorldConfig` and discover all assets it references.
/// Returns sub-world TOML paths that were NOT already in `seen_worlds`
/// (the caller should fetch + recurse into those).
/// `world_key` uniquely identifies this world (e.g. `"(base)"` for the
/// base world, or the TOML path for sub-worlds) and is used to detect
/// duplicate processing.
fn discover_world_assets(
    world: &WorldConfig,
    config_cache: &HashMap<String, EntityConfig>,
    seen_entities: &mut HashSet<String>,
    manifest: &mut AssetManifest,
    extra_worlds_out: &mut Vec<String>,
    _world_key: &str,
) {
    // Walk every [[entity]] in the world
    for entity_inst in &world.entities {
        walk_entity(&entity_inst.template_path, config_cache, seen_entities, manifest);
        // If the entity has overrides with a model, we can't discover those
        // statically — they'd only be resolved at runtime. Acceptable gap.
    }

    // Walk triggers for LoadWorld / SpawnEntity actions
    let mut entity_paths_from_triggers = Vec::new();
    let mut world_paths_from_triggers = Vec::new();
    for trigger in &world.triggers {
        collect_action_paths(&trigger.actions, &mut entity_paths_from_triggers, &mut world_paths_from_triggers);
    }

    // Walk comms responses for the same
    collect_comms_action_paths(&world.comms, &mut entity_paths_from_triggers, &mut world_paths_from_triggers);

    // Process discovered entity paths
    for path in entity_paths_from_triggers {
        walk_entity(&path, config_cache, seen_entities, manifest);
    }

    // Track sub-world paths for the caller to fetch & recurse
    // (caller handles deduplication against seen_worlds)
    extra_worlds_out.extend(world_paths_from_triggers);
    extra_worlds_out.extend(world.extra_worlds.clone());
}

/// Build the initial `AssetManifest` from the base world + config cache.
/// Returns a list of sub-world TOML paths that need to be fetched and
/// recursively processed.
/// Returns `(manifest, pending_world_paths, seen_entity_paths)`.
/// The caller should store `seen_entity_paths` in `AssetPreloadResource` so
/// that incremental sub-world processing shares the same dedup set and does
/// not push duplicate sidecar paths into `pending_sidecars`.
pub fn discover_base_assets(
    world: &WorldConfig,
    config_cache: &HashMap<String, EntityConfig>,
) -> (AssetManifest, Vec<String>, HashSet<String>) {
    let mut seen_entities = HashSet::new();
    let mut manifest = AssetManifest::default();
    let mut pending_worlds = Vec::new();

    discover_world_assets(
        world,
        config_cache,
        &mut seen_entities,
        &mut manifest,
        &mut pending_worlds,
        "(base)",
    );

    (manifest, pending_worlds, seen_entities)
}

/// Process a sub-world TOML string that was fetched from disk/network.
/// Returns the paths of any further sub-worlds discovered.
pub fn process_sub_world_toml(
    toml_str: &str,
    config_cache: &HashMap<String, EntityConfig>,
    seen_entities: &mut HashSet<String>,
    manifest: &mut AssetManifest,
    path: &str,
) -> Result<Vec<String>, String> {
    let world = parse_world(toml_str)?;
    let mut pending_worlds = Vec::new();

    discover_world_assets(
        &world,
        config_cache,
        seen_entities,
        manifest,
        &mut pending_worlds,
        path,
    );

    Ok(pending_worlds)
}

// ── Bevy Resource ─────────────────────────────────────────────────────────

/// Tracks the progress of server-side asset pre-caching.
#[derive(Resource)]
pub struct AssetPreloadResource {
    /// True once `begin_asset_preload` has run.
    pub started: bool,
    /// True when all discovered assets are ready to render.
    pub complete: bool,
    /// Total number of items to load (GLB scenes + radar icons + sidecars).
    pub total_count: usize,
    /// Number of items that have finished loading.
    pub ready_count: usize,

    // Internal handles — kept alive so the asset server does not drop them.
    glb_handles: Vec<(String, Handle<bevy::scene::Scene>)>,
    icon_handles: Vec<(String, Handle<Image>)>,

    // Set of GLB paths for which we've already logged a Failed warning, so the
    // warning fires exactly once per asset (poll runs every frame).
    failed_glbs: HashSet<String>,

    // Sidecar tracking
    pending_sidecars: Vec<String>,
    /// All sidecar paths ever pushed to `pending_sidecars` — prevents duplicates
    /// when sub-world processing re-discovers the same model.
    registered_sidecars: HashSet<String>,
    initial_sidecar_count: usize,

    // Sub-world tracking (for incremental discovery)
    pending_sub_worlds: Vec<String>,
    initial_sub_world_count: usize,
    seen_worlds: HashSet<String>,
    seen_entities: HashSet<String>,

    // Timer for throttling progress broadcasts
    progress_timer: Timer,
}

/// Safe default: preload not started (`started=false`), not complete
/// (`complete=false`).  `init_resource` uses this so `poll_asset_preload`
/// can always access the resource without panicking.
impl Default for AssetPreloadResource {
    fn default() -> Self {
        Self {
            started: false,
            complete: false,
            total_count: 0,
            ready_count: 0,
            glb_handles: Vec::new(),
            icon_handles: Vec::new(),
            failed_glbs: HashSet::new(),
            pending_sidecars: Vec::new(),
            registered_sidecars: HashSet::new(),
            initial_sidecar_count: 0,
            pending_sub_worlds: Vec::new(),
            initial_sub_world_count: 0,
            seen_worlds: HashSet::new(),
            seen_entities: HashSet::new(),
            progress_timer: Timer::from_seconds(0.5, TimerMode::Repeating),
        }
    }
}

impl AssetPreloadResource {
    fn new() -> Self {
        Self {
            started: false,
            complete: false,
            total_count: 0,
            ready_count: 0,
            glb_handles: Vec::new(),
            icon_handles: Vec::new(),
            failed_glbs: HashSet::new(),
            pending_sidecars: Vec::new(),
            registered_sidecars: HashSet::new(),
            initial_sidecar_count: 0,
            pending_sub_worlds: Vec::new(),
            initial_sub_world_count: 0,
            seen_worlds: HashSet::new(),
            seen_entities: HashSet::new(),
            progress_timer: Timer::from_seconds(0.5, TimerMode::Repeating),
        }
    }

    /// Progress fraction 0.0–1.0.
    pub fn fraction(&self) -> f32 {
        if self.total_count == 0 {
            return 1.0;
        }
        (self.ready_count as f32 / self.total_count as f32).clamp(0.0, 1.0)
    }
}

// ── Bevy Systems ──────────────────────────────────────────────────────────

/// Build the asset manifest and begin loading. Runs every Update frame
/// (gated by internal guards) so it works with `init_state()` which does
/// not fire `OnEnter` for the initial state.
pub fn begin_asset_preload(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    world_config: Option<Res<WorldConfig>>,
    preload: Option<Res<AssetPreloadResource>>,
) {
    // Guard: only fire when not already started
    if let Some(ref p) = preload {
        if p.started || p.complete {
            return;
        }
    }
    // Guard: need WorldConfig to proceed (may not be ready on frame 1)
    let Some(world_config) = world_config else {
        bevy::log::info!("asset_preload: WorldConfig not ready yet, will retry next frame");
        return;
    };

    let config_cache = crate::config_cache::get_config_cache();
    bevy::log::info!(
        "asset_preload: config_cache has {} entries; starting asset discovery",
        config_cache.len()
    );

    // Guard: if config cache is empty, entity configs haven't landed yet.
    // On WASM the JS preload populates this cache before Bevy starts, so
    // this guard is belt-and-suspenders. On native the cache is always
    // empty (entity configs are read from disk at resolution time), so
    // we proceed with whatever we have (= empty manifest = no-op preload).
    #[cfg(not(target_arch = "wasm32"))]
    let cache_is_empty = false;
    #[cfg(target_arch = "wasm32")]
    let cache_is_empty = config_cache.is_empty();
    if cache_is_empty {
        bevy::log::info!("asset_preload: config cache empty, retrying next frame");
        return;
    }

    // Initial discovery from base world
    let (mut manifest, pending_worlds, base_seen_entities) = discover_base_assets(&world_config, &config_cache);

    // Start loading GLB models
    let mut glb_handles = Vec::new();
    for glb_path in &manifest.glb_models {
        let path = format!("{}#Scene0", glb_path);
        let handle: Handle<bevy::scene::Scene> = asset_server.load(&path);
        glb_handles.push((glb_path.clone(), handle));
    }

    // Start loading radar icons
    let mut icon_handles = Vec::new();
    for icon_path in &manifest.radar_icons {
        let handle: Handle<Image> = asset_server.load(icon_path);
        icon_handles.push((icon_path.clone(), handle));
    }

    // Request sidecar fetches (WASM) — on native these are read synchronously later
    #[cfg(target_arch = "wasm32")]
    for sc_path in &manifest.sidecars {
        crate::config_cache::request_sidecar_fetch(sc_path.clone());
    }

    // Request sub-world TOML fetches
    for world_path in &pending_worlds {
        #[cfg(target_arch = "wasm32")]
        crate::config_cache::request_world_fetch(world_path.clone());
        #[cfg(not(target_arch = "wasm32"))]
        {
            // On native we can read the file synchronously right now
            if let Ok(toml_str) = std::fs::read_to_string(world_path) {
                let mut seen_entities = base_seen_entities.clone();
                let mut manifest_mut = AssetManifest::default();
                let _ = process_sub_world_toml(
                    &toml_str,
                    &config_cache,
                    &mut seen_entities,
                    &mut manifest_mut,
                    world_path,
                );
                // Load any newly discovered GLBs/icons/sidecars
                for glb_path in &manifest_mut.glb_models {
                    let path = format!("{}#Scene0", glb_path);
                    let handle: Handle<bevy::scene::Scene> = asset_server.load(&path);
                    glb_handles.push((glb_path.clone(), handle));
                }
                for icon_path in &manifest_mut.radar_icons {
                    let handle: Handle<Image> = asset_server.load(icon_path);
                    icon_handles.push((icon_path.clone(), handle));
                }
                manifest.glb_models.extend(manifest_mut.glb_models);
                manifest.radar_icons.extend(manifest_mut.radar_icons);
                manifest.sidecars.extend(manifest_mut.sidecars);
            }
        }
    }

    // GLBs are now included in `total_count` (PRD: prefetch sidecar race fix).
    // The poll loop tracks each GLB's `LoadState` and treats `Loaded` and
    // `Failed` as terminal. A failed parse no longer deadlocks the gate; it
    // just logs a warning and counts as "ready" so the rest of the world can
    // proceed.
    let total_count = glb_handles.len() + icon_handles.len() + manifest.sidecars.len();

    bevy::log::info!(
        "asset_preload: discovered {} GLBs, {} icons, {} sidecars, {} sub-worlds (gating total {})",
        glb_handles.len(),
        icon_handles.len(),
        manifest.sidecars.len(),
        pending_worlds.len(),
        total_count,
    );

    // Track pending sidecars and sub-worlds for the poll loop
    let pending_sidecars = manifest.sidecars.clone();
    let registered_sidecars: HashSet<String> = manifest.sidecars.iter().cloned().collect();
    let initial_sidecar_count = pending_sidecars.len();
    let initial_sub_world_count = pending_worlds.len();
    let mut seen_worlds = HashSet::new();
    seen_worlds.insert("(base)".to_string());

    let resource = AssetPreloadResource {
        started: true,
        complete: total_count == 0,
        total_count,
        ready_count: 0,
        glb_handles,
        icon_handles,
        failed_glbs: HashSet::new(),
        pending_sidecars,
        registered_sidecars,
        initial_sidecar_count,
        pending_sub_worlds: pending_worlds,
        initial_sub_world_count,
        seen_worlds,
        seen_entities: base_seen_entities,
        progress_timer: Timer::from_seconds(0.5, TimerMode::Repeating),
    };

    commands.insert_resource(resource);
}

/// Run every frame: poll asset readiness and update progress.
pub fn poll_asset_preload(
    mut preload: ResMut<AssetPreloadResource>,
    _scenes: Res<Assets<bevy::scene::Scene>>,
    images: Res<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    if !preload.started || preload.complete {
        return;
    }

    // Poll sidecar delivery. Use a non-destructive check
    // (`is_pending_sidecar_delivered`) so we don't drain the TOML out from
    // under the renderer — `render_spawned_entities` is the sole consumer
    // and reads the contents via `take_pending_sidecar_toml`. Calling the
    // destructive `take_*` here would race the renderer and silently leave
    // models unattached to their entities (the prefetch wins, then discards
    // the TOML, so the renderer's later `take` returns `None` forever).
    //
    // On native this is a no-op observation (the inbox is always empty —
    // native `load_sidecar_toml` reads from `std::fs` directly), but the
    // call is cheap and keeps the code path identical across targets.
    let mut still_pending = Vec::new();
    for path in &preload.pending_sidecars {
        if !crate::config_cache::is_pending_sidecar_delivered(path) {
            still_pending.push(path.clone());
        }
    preload.pending_sidecars = still_pending;

    // Poll sub-world TOML delivery and process incrementally. On native,
    // sub-worlds are read synchronously in `begin_asset_preload` so there
    // is nothing to poll — gate the whole loop on WASM to avoid `mut`
    // bindings that would never be mutated on native.
    #[cfg(target_arch = "wasm32")]
    {
        let mut new_glbs: Vec<String> = Vec::new();
        let mut new_icons: Vec<String> = Vec::new();
        let mut new_sidecars: Vec<String> = Vec::new();
        let mut new_sub_worlds: Vec<String> = Vec::new();
        let mut still_pending_worlds: Vec<String> = Vec::new();

        let pending_sub_worlds = preload.pending_sub_worlds.clone();
        for world_path in &pending_sub_worlds {
            if let Some(toml_str) = crate::config_cache::pop_pending_world_toml(world_path) {
                let mut manifest = AssetManifest::default();
                let cache = crate::config_cache::get_config_cache();
                let cache_ref: &std::collections::HashMap<
                    String,
                    crate::entity_config::EntityConfig,
                > = &*cache;
                match process_sub_world_toml(
                    &toml_str,
                    cache_ref,
                    &mut preload.seen_entities,
                    &mut manifest,
                    world_path,
                ) {
                    Ok(more_worlds) => {
                        new_glbs.extend(manifest.glb_models);
                        new_icons.extend(manifest.radar_icons);
                        new_sidecars.extend(manifest.sidecars);
                        new_sub_worlds.extend(more_worlds);
                    }
                    Err(e) => {
                        bevy::log::warn!(
                            "asset_preload: failed to parse sub-world {world_path}: {e}"
                        );
                    }
                }
            } else {
                still_pending_worlds.push(world_path.clone());
            }
        }
        preload.pending_sub_worlds = still_pending_worlds;

        // Load newly discovered assets from sub-worlds
        for glb_path in &new_glbs {
            let path = format!("{}#Scene0", glb_path);
            let handle: Handle<bevy::scene::Scene> = asset_server.load(&path);
            preload.glb_handles.push((glb_path.clone(), handle));
        }
        for icon_path in &new_icons {
            let handle: Handle<Image> = asset_server.load(icon_path);
            preload.icon_handles.push((icon_path.clone(), handle));
        }
        for sc_path in &new_sidecars {
            // Guard: only push sidecars that haven't been registered yet. The same
            // GLB sidecar may be discovered again when a sub-world re-references a
            // model already seen in the base world. Pushing a duplicate would leave
            // a pending entry that can never be popped (JS delivers the TOML once).
            if preload.registered_sidecars.insert(sc_path.clone()) {
                crate::config_cache::request_sidecar_fetch(sc_path.clone());
                preload.pending_sidecars.push(sc_path.clone());
            }
        }
        for w_path in &new_sub_worlds {
            if !preload.pending_sub_worlds.contains(w_path) {
                crate::config_cache::request_world_fetch(w_path.clone());
                preload.pending_sub_worlds.push(w_path.clone());
            }
        }
    }

    // ── GLB readiness ────────────────────────────────────────────────────
    // For each GLB handle, query its LoadState. Treat `Loaded` and `Failed`
    // as terminal. A `Failed` GLB logs a warn! once (so authoring errors
    // surface) and counts as "ready" so it does not deadlock the gate; the
    // entity will simply render without its model.
    //
    // Collect newly-failed paths into a local Vec first so the iterator's
    // immutable borrow of `preload.glb_handles` doesn't conflict with the
    // mutable `preload.failed_glbs.insert` we need for "warn once" tracking.
    let mut glbs_loaded = 0usize;
    let mut glbs_failed = 0usize;
    let mut newly_failed: Vec<String> = Vec::new();
    for (path, handle) in &preload.glb_handles {
        match asset_server.load_state(handle.id()) {
            LoadState::Loaded => glbs_loaded += 1,
            LoadState::Failed(_) => {
                if !preload.failed_glbs.contains(path) {
                    newly_failed.push(path.clone());
                }
                glbs_failed += 1;
            }
            // NotLoaded / Loading — still in flight, don't count.
            _ => {}
        }
    }
    for path in newly_failed {
        bevy::log::warn!(
            "asset_preload: GLB failed to load: {path} — entities referencing this model will render without a mesh"
        );
        preload.failed_glbs.insert(path);
    }
    let glbs_terminal = glbs_loaded + glbs_failed;
    let glbs_done = glbs_terminal == preload.glb_handles.len();

    // Recompute totals: icons + sidecars + sub-worlds + GLBs.
    preload.total_count = preload.icon_handles.len()
        + preload.initial_sidecar_count
        + preload.initial_sub_world_count
        + preload.glb_handles.len();

    // Check completion: icons + sidecars + sub-worlds + GLBs.
    let sidecars_done = preload.pending_sidecars.is_empty();
    let sub_worlds_done = preload.pending_sub_worlds.is_empty();
    let icons_ready = preload
        .icon_handles
        .iter()
        .all(|(_, h)| images.get(h).is_some());

    // Recompute ready count for progress display.
    let icon_ready = preload
        .icon_handles
        .iter()
        .filter(|(_, h)| images.get(h).is_some())
        .count();
    let sidecar_ready = preload
        .initial_sidecar_count
        .saturating_sub(preload.pending_sidecars.len());
    let sub_world_ready = preload
        .initial_sub_world_count
        .saturating_sub(preload.pending_sub_worlds.len());
    preload.ready_count = icon_ready + sidecar_ready + sub_world_ready + glbs_terminal;

    if icons_ready && sidecars_done && sub_worlds_done && glbs_done {
        preload.complete = true;
        preload.ready_count = preload.total_count;
        bevy::log::info!(
            "asset_preload: all assets ready (icons={}, sidecars_done={}, sub_worlds_done={}, GLBs={}+{} failed)",
            icons_ready, sidecars_done, sub_worlds_done, glbs_loaded, glbs_failed
        );
    } else {
        bevy::log::debug!(
            "asset_preload: waiting: icons_ready={}, sidecars_done={} (pending={}), sub_worlds_done={} (pending={}), GLBs={}/{} ({} failed)",
            icons_ready,
            sidecars_done,
            preload.pending_sidecars.len(),
            sub_worlds_done,
            preload.pending_sub_worlds.len(),
            glbs_terminal,
            preload.glb_handles.len(),
            glbs_failed,
        );
    }
}

/// Broadcast `LoadingProgress` at ~2 Hz during the `Loading` phase.
pub fn broadcast_loading_progress(
    mut preload: ResMut<AssetPreloadResource>,
    mut outbox: ResMut<LobbyOutbox>,
    time: Res<Time>,
) {
    if !preload.started || preload.complete {
        return;
    }
    preload.progress_timer.tick(time.delta());
    if !preload.progress_timer.just_finished() {
        return;
    }

    let fraction = preload.fraction();
    outbox.0.push((
        Target::All,
        ServerMessage::LoadingProgress {
            data: LoadingProgress { fraction },
        },
    ));
}

/// Auto-transition from `Loading` → `InProgress` when preload completes.
pub fn auto_transition_from_loading(
    preload: Res<AssetPreloadResource>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
) {
    if !preload.complete {
        bevy::log::debug!("auto_transition_from_loading: preload.complete=false, staying in Loading");
        return;
    }
    bevy::log::info!("auto_transition_from_loading: preload.complete=true, transitioning to InProgress");
    next_state.set(GamePhase::InProgress);
    outbox.0.push((Target::All, ServerMessage::GameStarted));
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::*;

    #[test]
    fn icon_asset_path_capitalizes_first_letter() {
        assert_eq!(icon_asset_path("destroyer"), "radar_icons/Icon-Destroyer.png");
        assert_eq!(icon_asset_path("Star"), "radar_icons/Icon-Star.png");
        assert_eq!(icon_asset_path("playerShip"), "radar_icons/Icon-PlayerShip.png");
    }

    #[test]
    fn icon_asset_path_empty_returns_empty_icon_name() {
        // The naming convention doesn't make sense for empty input,
        // but it should not panic.
        let path = icon_asset_path("");
        assert!(path.starts_with("radar_icons/Icon-"));
    }

    #[test]
    fn discover_entity_config_with_model_and_icon() {
        let mut config = EntityConfig::default();
        config.mesh = Some(MeshConfig {
            model: Some("assets/models/test_ship.glb".into()),
            variant: None,
            shape: MeshShape::Sphere,
            colour: vec![1.0, 0.0, 0.0],
            radius: 1.0,
            size: None,
            minor_radius: 0.0,
            emissive: None,
            scale: 1.0,
            rotation: [0.0, 0.0, 0.0],
        });
        config.radar_appearance = Some(RadarAppearanceConfig {
            icon: Some("testShip".into()),
            colour: Some(vec![1.0, 0.0, 0.0]),
            size: None,
            region_colour: None,
        });

        let mut manifest = AssetManifest::default();
        discover_entity_config_assets(&config, &mut manifest);

        assert_eq!(manifest.glb_models, vec!["models/test_ship.glb"]);
        assert!(manifest.sidecars.iter().any(|s| s.contains("test_ship.model.toml")));
        assert_eq!(manifest.radar_icons, vec!["radar_icons/Icon-TestShip.png"]);
    }

    #[test]
    fn discover_entity_config_no_model_no_icon() {
        let config = EntityConfig::default();
        let mut manifest = AssetManifest::default();
        discover_entity_config_assets(&config, &mut manifest);
        assert!(manifest.glb_models.is_empty());
        assert!(manifest.sidecars.is_empty());
        assert!(manifest.radar_icons.is_empty());
    }
}
