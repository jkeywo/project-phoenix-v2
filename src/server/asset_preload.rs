//! Server-side asset pre-cache system.
//!
//! When the lobby opens, `begin_asset_preload` recursively walks the scenario
//! (world TOML + all referenced entity templates + triggers + comms responses)
//! to discover every renderable asset (GLB models, radar icons, model rig
//! sidecars). It calls `asset_server.load()` for each and tracks readiness.
//!
//! Discovery is two-phase. An entity template names one model; since #914 the
//! rest of that model's LOD ladder is declared in the model's own rig sidecar,
//! which is fetched asynchronously. So `poll_asset_preload` expands each
//! sidecar's `[[lod]]` chain the frame it is delivered
//! (`discover_sidecar_lod_assets`), adding the far GLBs and their sidecars to
//! the same gate — the way sub-worlds already extend the manifest incrementally.
//!
//! If the captain presses Engage before preload completes, the phase
//! transitions to `GamePhase::Loading` instead of `InProgress`. During
//! `Loading` the system broadcasts `LoadingProgress { fraction }` to clients
//! at ~10 Hz. Once all assets are ready, it auto-transitions to `InProgress`
//! and sends `GameStarted`.

use std::collections::{HashMap, HashSet};

use bevy::asset::LoadState;
use bevy::prelude::*;

use crate::core::messages::{GamePhase, ServerMessage};
use crate::entity_config::EntityConfig;
use crate::lobby::server::LobbyOutbox;
use crate::lobby::Target;
use crate::model_rig::sidecar_path;
use crate::world::config::{parse_world, WorldConfig};

// ── AssetManifest ──────────────────────────────────────────────────────────

/// All renderable assets discovered by walking the scenario recursively.
#[derive(Clone, Debug, Default)]
pub struct AssetManifest {
    /// GLB model paths relative to `assets/` (e.g. `"models/dynasty_destroyer.glb"`).
    pub glb_models: Vec<String>,
    /// Radar icon paths relative to `assets/` (e.g. `"radar_icons/Icon-Destroyer.png"`).
    pub radar_icons: Vec<String>,
    /// Dust/PFX texture paths relative to `assets/` (e.g. `"pfx/space_mote_soft_disc.png"`).
    pub pfx_textures: Vec<String>,
    /// Full sidecar paths (e.g. `"assets/models/dynasty_destroyer.model.toml"`).
    pub sidecars: Vec<String>,
    /// Planet texture paths (TOML-style, `assets/`-prefixed) with their sRGB
    /// flag. Loaded via `entity_planet::load_planet_image` so the loader
    /// settings match the renderer's (Bevy keeps the first load's settings).
    pub planet_textures: Vec<(String, bool)>,
    /// Sub-world TOML paths discovered (for tracking purposes; not loaded here).
    pub sub_worlds: Vec<String>,
}

/// Radar icon name injected onto the player's own ship at game-start spawn
/// (see `player_ship_identity` in `src/server_app.rs`). Because it is injected
/// at spawn rather than authored in any hull template, the template scan below
/// never discovers it — so it is preloaded unconditionally in
/// `discover_base_assets`. Keep this in sync with the injection site.
pub const PLAYER_SHIP_RADAR_ICON: &str = "playerShip";

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

/// Extract GLB model, radar icon, and sidecar paths from one entity config.
fn discover_entity_config_assets(config: &EntityConfig, manifest: &mut AssetManifest) {
    // GLB model + sidecar
    if let Some(ref mesh) = config.mesh {
        if let Some(ref model_path) = mesh.model {
            let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
            if !manifest.glb_models.contains(&rel.to_string()) {
                manifest.glb_models.push(rel.to_string());
            }
            let sc = sidecar_path(model_path, mesh.variant.as_deref());
            if !manifest.sidecars.contains(&sc) {
                manifest.sidecars.push(sc);
            }
            // The far LOD levels are NOT discoverable here any more: the ladder
            // lives in that sidecar (issue #914), which has not been fetched
            // yet. `poll_asset_preload` expands it via
            // `discover_sidecar_lod_assets` the frame the sidecar lands, which
            // is what keeps the whole ladder inside the loading gate.
        }
    }
    // Planet textures
    if let Some(ref planet) = config.planet {
        for entry in crate::entity_planet::planet_texture_paths(planet) {
            if !manifest.planet_textures.contains(&entry) {
                manifest.planet_textures.push(entry);
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
            if !manifest.sub_worlds.iter().any(|w| w == a_path.path()) {
                // Tracked via entity_paths concept; stored separately
                // since they'll be looked up in the config cache.
            }
        }
    }
}

/// Extract the GLB models — and their own rig sidecars — named by a model rig
/// sidecar's `[[lod]]` chain (issue #914).
///
/// This is the second half of discovery. An entity template names one model;
/// that model's sidecar names the rest of its ladder. The chain is therefore
/// only visible once the sidecar itself has been delivered, so this runs from
/// the poll loop rather than the initial walk — the same incremental shape
/// sub-worlds already use.
///
/// `distance`, when known, is this sidecar's CLOSEST placed `[[entity]]`
/// instance's distance from the player's starting position (issue
/// lod-preload-by-distance): only the ladder level [`select_lod`] would pick
/// for that distance is preloaded, rather than every GLB the ladder declares.
/// A scenario with many far-off, high-ladder-count models used to gate game
/// start on every one of their unseen levels; now it gates on only the level
/// actually shown at spawn. The rest of the ladder is warmed in the
/// background once the game is running — see `prefetch_next_lod_level` in
/// `server_app.rs`, which always tries to have the next-more-detailed level
/// ready before an approaching ship actually needs it.
///
/// `distance` is `None` for anything without a statically known placement —
/// procedurally-spawned asteroid-field members chief among them, since their
/// position isn't decided until the field's runtime grid streams them in.
/// Those preload their WHOLE ladder, same as before this feature: with no
/// distance to reason from, guessing which single level is "close enough" is
/// worse than just loading all of them up front.
///
/// `path` is the sidecar's own path: a level that omits `variant` inherits the
/// variant of the sidecar it was declared in, which is by construction the
/// variant the entity used to reach it, and therefore agrees with the
/// renderer's `MeshConfig::variant` fallback in `update_mesh_lod`.
///
/// A sidecar that fails to parse contributes nothing to the ladder. It is not
/// fatal — `resolve_sidecar_rig` degrades the same file to an identity rig,
/// so the entity still renders its flat `[mesh]` — but it is the same
/// present-but-malformed case `resolve_sidecar_rig` logs at ERROR (a typo
/// silently losing a whole LOD chain is not a warning), so this logs at the
/// same level rather than a quieter one for the same file.
pub fn discover_sidecar_lod_assets(
    sidecar_toml: &str,
    path: &str,
    distance: Option<f32>,
    manifest: &mut AssetManifest,
) {
    if sidecar_toml.trim().is_empty() {
        return;
    }
    let rig = match crate::model_rig::ModelRig::from_toml(sidecar_toml) {
        Ok(rig) => rig,
        Err(e) => {
            bevy::log::error!(
                "asset_preload: rig sidecar {path} failed to parse: {e}; any [[lod]] chain it \
                 declares will not preload"
            );
            return;
        }
    };
    if rig.lod.is_empty() {
        return;
    }
    let own_variant = crate::model_rig::sidecar_variant(path);

    // Known distance -> just the level `select_lod` would pick for it: the
    // rest of the ladder warms in the background once the game is running
    // (see the doc comment above). Unknown distance -> the whole ladder, same
    // as before this feature.
    let wanted: Vec<usize> = match distance {
        Some(d) => vec![crate::entity_config::select_lod(&rig.lod, d, None)],
        None => (0..rig.lod.len()).collect(),
    };

    for i in wanted {
        let Some(level) = rig.lod.get(i) else {
            continue;
        };
        let Some(ref lod_model) = level.model else {
            continue;
        };
        let rel = lod_model
            .strip_prefix("assets/")
            .unwrap_or(lod_model)
            .to_string();
        if !manifest.glb_models.contains(&rel) {
            manifest.glb_models.push(rel);
        }
        let sc = sidecar_path(lod_model, level.variant.as_deref().or(own_variant));
        if !manifest.sidecars.contains(&sc) {
            manifest.sidecars.push(sc);
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
            walk_entity(p.path(), config_cache, seen_entities, manifest);
        }
        for p in &field.cosmetic_type_paths {
            walk_entity(p.path(), config_cache, seen_entities, manifest);
        }
    }
}

/// Walk a single parsed `WorldConfig` and discover all assets it references.
/// Returns sub-world TOML paths that were NOT already in `seen_worlds`
/// (the caller should fetch + recurse into those).
/// `world_key` uniquely identifies this world (e.g. `"(base)"` for the
/// base world, or the TOML path for sub-worlds) and is used to detect
/// duplicate processing.
///
/// `player_start` and `sidecar_distance` are the distance-based LOD preload
/// feature's inputs/output (issue lod-preload-by-distance): every `[[entity]]`
/// instance in `world` has a statically resolvable position (sub-worlds share
/// the base world's coordinate space, so one `player_start` covers all of
/// them), so its distance from `player_start` is computed here and merged
/// into `sidecar_distance` — keyed by the sidecar path the instance's model
/// resolves to, keeping the SMALLEST distance across every instance that
/// shares one template. `discover_sidecar_lod_assets` reads it back once that
/// sidecar's own TOML is delivered.
#[allow(clippy::too_many_arguments)]
fn discover_world_assets(
    world: &WorldConfig,
    config_cache: &HashMap<String, EntityConfig>,
    seen_entities: &mut HashSet<String>,
    manifest: &mut AssetManifest,
    extra_worlds_out: &mut Vec<String>,
    _world_key: &str,
    player_start: [f32; 3],
    sidecar_distance: &mut HashMap<String, f32>,
) {
    // `relative_to` needs every named, non-relative_to instance's position
    // resolved first — the same map `spawn_game_start_entities` builds before
    // spawning, computed here ahead of anything actually spawning.
    let named_positions = crate::world::config::build_named_entity_positions(world);

    // Walk every [[entity]] in the world
    for entity_inst in &world.entities {
        // Distance from the player's start, when this instance's position
        // resolves (an unresolvable anchor/relative_to is not fatal here —
        // `walk_entity` below still discovers its assets, just without a
        // distance to narrow the LOD ladder by).
        if let Ok(pos) = crate::world::config::resolve_entity_position_with(
            entity_inst,
            &world.anchors,
            &named_positions,
        ) {
            if let Some(config) = config_cache.get(&entity_inst.template_path) {
                if let Some(ref mesh) = config.mesh {
                    if let Some(ref model_path) = mesh.model {
                        let sc = sidecar_path(model_path, mesh.variant.as_deref());
                        let dx = pos[0] - player_start[0];
                        let dy = pos[1] - player_start[1];
                        let dz = pos[2] - player_start[2];
                        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
                        sidecar_distance
                            .entry(sc)
                            .and_modify(|d| {
                                if distance < *d {
                                    *d = distance;
                                }
                            })
                            .or_insert(distance);
                    }
                }
            }
        }

        walk_entity(
            &entity_inst.template_path,
            config_cache,
            seen_entities,
            manifest,
        );
        // If the entity has overrides with a model, we can't discover those
        // statically — they'd only be resolved at runtime. Acceptable gap.
    }

    // The `[[trigger.action]]` / `[[comms.response.action]]` walks that used to
    // discover `spawn_entity` templates and `load_world` sub-worlds went with
    // those front-ends (issue #985). A SCRIPTED spawn's `template_path` is
    // discovered instead by `world::config::entity_template_paths`' static scan
    // of the `[script]` bodies; a scripted `load_world` is not discoverable at
    // all before the handler runs, and is fetched on demand by the layer applier
    // (`LayerLoadOutcome::TomlUnavailable` re-queues until the fetch lands).
    for path in &world.extra_worlds {
        if !extra_worlds_out.contains(path) {
            extra_worlds_out.push(path.clone());
        }
    }

    // Dust textures. Resolved through the renderer rather than read straight
    // off `world.dust`, because a world that declares no `[[dust.layer]]`
    // still gets the built-in layers and their textures.
    for path in crate::server::pfx::dust_texture_paths(Some(world)) {
        if !manifest.pfx_textures.contains(&path) {
            manifest.pfx_textures.push(path);
        }
    }
}

/// Resolve the player's starting position for distance-based LOD preload
/// selection (issue lod-preload-by-distance): `[player_spawn]`'s own
/// position/anchor when authored, else wherever the player ship's own
/// `[[entity]]` instance resolves to — the same precedence
/// `spawn_game_start_entities` applies when it actually places the ship,
/// computed here ahead of anything spawning. Falls back to the origin when
/// neither is determinable (no ship in the world at all, or an unresolvable
/// anchor) — the origin is also a safe "unknown" default: every distance
/// computed from it is still A distance, just not necessarily a tight one, so
/// the LOD it picks errs toward more detail rather than none.
fn resolve_player_start(
    world: &WorldConfig,
    config_cache: &HashMap<String, EntityConfig>,
) -> [f32; 3] {
    if let Some(ref spawn) = world.player_spawn {
        if let Some(pos) = spawn.position {
            return pos;
        }
        if let Some(ref anchor) = spawn.anchor {
            if let Some(pos) = world.anchors.get(anchor) {
                return *pos;
            }
        }
    }

    let named_positions = crate::world::config::build_named_entity_positions(world);
    for entity_inst in &world.entities {
        if entity_inst.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart {
            continue;
        }
        let is_ship = config_cache
            .get(&entity_inst.template_path)
            .is_some_and(|c| c.tags.iter().any(|t| t == "ship"));
        if !is_ship {
            continue;
        }
        if let Ok(pos) = crate::world::config::resolve_entity_position_with(
            entity_inst,
            &world.anchors,
            &named_positions,
        ) {
            return pos;
        }
    }

    [0.0, 0.0, 0.0]
}

/// Build the initial `AssetManifest` from the base world + config cache.
/// Returns a list of sub-world TOML paths that need to be fetched and
/// recursively processed.
/// Returns `(manifest, pending_world_paths, seen_entity_paths, sidecar_distance)`.
/// The caller should store `seen_entity_paths` in `AssetPreloadResource` so
/// that incremental sub-world processing shares the same dedup set and does
/// not push duplicate sidecar paths into `pending_sidecars`; `sidecar_distance`
/// likewise belongs in `AssetPreloadResource` so a later-delivered sidecar
/// (including from a sub-world processed after this call) can look its
/// distance back up.
pub fn discover_base_assets(
    world: &WorldConfig,
    config_cache: &HashMap<String, EntityConfig>,
) -> (
    AssetManifest,
    Vec<String>,
    HashSet<String>,
    HashMap<String, f32>,
    [f32; 3],
) {
    let mut seen_entities = HashSet::new();
    let mut manifest = AssetManifest::default();
    let mut pending_worlds = Vec::new();
    let mut sidecar_distance = HashMap::new();
    let player_start = resolve_player_start(world, config_cache);

    discover_world_assets(
        world,
        config_cache,
        &mut seen_entities,
        &mut manifest,
        &mut pending_worlds,
        "(base)",
        player_start,
        &mut sidecar_distance,
    );

    // The player-ship radar icon is injected onto the selected hull at player
    // spawn, not authored in any template, so the template scan above never
    // sees it. Preload it unconditionally so every client has the PNG ready for
    // the player blip regardless of which hull is flown.
    let player_icon = icon_asset_path(PLAYER_SHIP_RADAR_ICON);
    if !manifest.radar_icons.contains(&player_icon) {
        manifest.radar_icons.push(player_icon);
    }

    (
        manifest,
        pending_worlds,
        seen_entities,
        sidecar_distance,
        player_start,
    )
}

/// Process a sub-world TOML string that was fetched from disk/network.
/// Returns the paths of any further sub-worlds discovered.
///
/// `player_start` is the SAME point `discover_base_assets` resolved for the
/// base world — sub-worlds share the base world's coordinate space, so a
/// sub-world's own entities are distanced from the one player start the whole
/// scenario has, not re-resolved per sub-world. `sidecar_distance` is the
/// same running map `discover_base_assets` began; a sub-world entity sharing
/// a template with a closer base-world instance leaves that entry unchanged
/// (the merge keeps the minimum), never widens it.
pub fn process_sub_world_toml(
    toml_str: &str,
    config_cache: &HashMap<String, EntityConfig>,
    seen_entities: &mut HashSet<String>,
    manifest: &mut AssetManifest,
    path: &str,
    player_start: [f32; 3],
    sidecar_distance: &mut HashMap<String, f32>,
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
        player_start,
        sidecar_distance,
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

    // Sub-world tracking (for incremental discovery)
    pending_sub_worlds: Vec<String>,
    /// Tracks all worlds ever processed (including "(base)"). Length minus one
    /// gives the total sub-world count used in progress fraction computation.
    seen_worlds: HashSet<String>,
    #[cfg(target_arch = "wasm32")]
    seen_entities: HashSet<String>,

    // Distance-based LOD preload (issue lod-preload-by-distance). `player_start`
    // is resolved once in `begin_asset_preload` and reused for every sub-world
    // discovered afterward (they share the base world's coordinate space).
    // Only read back on WASM (`poll_asset_preload`'s incremental sub-world
    // loop) — native discovers sub-worlds synchronously in `begin_asset_preload`
    // and passes the same value straight through as a local instead.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    player_start: [f32; 3],
    /// Maps a model rig sidecar path to the closest placed `[[entity]]`
    /// instance's distance from `player_start`; consulted the frame that
    /// sidecar's TOML is delivered so only the ladder level it actually needs
    /// preloads. A sidecar with no entry (no statically-placed instance — e.g.
    /// a procedurally-spawned asteroid) preloads its whole ladder.
    sidecar_distance: HashMap<String, f32>,

    // Timer for throttling progress broadcasts
    progress_timer: Timer,
    /// True once `broadcast_loading_progress` has sent at least one update.
    /// Used to send the current fraction immediately on first entry, rather
    /// than waiting for the 0.1s timer to fire (which may never fire if
    /// `poll_asset_preload` sets `complete=true` before the timer elapses).
    progress_sent: bool,
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
            pending_sub_worlds: Vec::new(),
            seen_worlds: HashSet::new(),
            #[cfg(target_arch = "wasm32")]
            seen_entities: HashSet::new(),
            player_start: [0.0, 0.0, 0.0],
            sidecar_distance: HashMap::new(),
            progress_timer: Timer::from_seconds(0.1, TimerMode::Repeating),
            progress_sent: false,
        }
    }
}

impl AssetPreloadResource {
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
    let (manifest, pending_worlds, base_seen_entities, sidecar_distance, player_start) =
        discover_base_assets(&world_config, &config_cache);
    #[cfg(not(target_arch = "wasm32"))]
    let mut manifest = manifest;
    #[cfg(not(target_arch = "wasm32"))]
    let mut sidecar_distance = sidecar_distance;

    // Start loading GLB models
    let mut glb_handles = Vec::new();
    for glb_path in &manifest.glb_models {
        let path = format!("{}#Scene0", glb_path);
        let handle: Handle<bevy::scene::Scene> = asset_server.load(&path);
        glb_handles.push((glb_path.clone(), handle));
    }

    // Start loading radar icons, and the dust textures alongside them — both
    // are plain `Handle<Image>`, so they share the same load tracking.
    let mut icon_handles = Vec::new();
    for icon_path in manifest.radar_icons.iter().chain(&manifest.pfx_textures) {
        let handle: Handle<Image> = asset_server.load(icon_path);
        icon_handles.push((icon_path.clone(), handle));
    }
    // Planet textures ride the icon path too (plain `Handle<Image>`), but must
    // go through the shared loader so the sRGB/sampler settings match the
    // renderer's (Bevy keeps the settings of the first load of a path).
    for (path, srgb) in &manifest.planet_textures {
        let handle = crate::entity_planet::load_planet_image(&asset_server, path, *srgb);
        icon_handles.push((path.clone(), handle));
    }

    // Request sidecar fetches (WASM) — on native these are read synchronously later
    #[cfg(target_arch = "wasm32")]
    for sc_path in &manifest.sidecars {
        crate::config_cache::request_sidecar_fetch(sc_path.clone());
    }

    let mut seen_worlds = HashSet::new();
    seen_worlds.insert("(base)".to_string());

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
                    player_start,
                    &mut sidecar_distance,
                );
                // Load any newly discovered GLBs/icons/sidecars
                for glb_path in &manifest_mut.glb_models {
                    let path = format!("{}#Scene0", glb_path);
                    let handle: Handle<bevy::scene::Scene> = asset_server.load(&path);
                    glb_handles.push((glb_path.clone(), handle));
                }
                for icon_path in manifest_mut
                    .radar_icons
                    .iter()
                    .chain(&manifest_mut.pfx_textures)
                {
                    let handle: Handle<Image> = asset_server.load(icon_path);
                    icon_handles.push((icon_path.clone(), handle));
                }
                for (path, srgb) in &manifest_mut.planet_textures {
                    let handle =
                        crate::entity_planet::load_planet_image(&asset_server, path, *srgb);
                    icon_handles.push((path.clone(), handle));
                }
                manifest.glb_models.extend(manifest_mut.glb_models);
                manifest.radar_icons.extend(manifest_mut.radar_icons);
                manifest.pfx_textures.extend(manifest_mut.pfx_textures);
                manifest.sidecars.extend(manifest_mut.sidecars);
                manifest
                    .planet_textures
                    .extend(manifest_mut.planet_textures);
                seen_worlds.insert(world_path.clone());
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

    let resource = AssetPreloadResource {
        started: true,
        // Don't mark complete if sub-worlds are pending — they may add more
        // GLBs/icons/sidecars once their TOMLs arrive.
        complete: total_count == 0 && pending_worlds.is_empty(),
        total_count,
        ready_count: 0,
        glb_handles,
        icon_handles,
        failed_glbs: HashSet::new(),
        pending_sidecars,
        registered_sidecars,
        pending_sub_worlds: pending_worlds,
        seen_worlds,
        #[cfg(target_arch = "wasm32")]
        seen_entities: base_seen_entities,
        player_start,
        sidecar_distance,
        progress_timer: Timer::from_seconds(0.1, TimerMode::Repeating),
        progress_sent: false,
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
    if !preload.started {
        return;
    }
    // Keep polling even after complete=true during the Lobby phase: sub-world
    // GLBs need continued tracking and any newly-discovered assets need
    // asset_server.load calls so they're cached before the game starts.
    // Only skip when there is genuinely nothing left to discover or wait for.
    if preload.complete
        && preload.pending_sub_worlds.is_empty()
        && preload.pending_sidecars.is_empty()
    {
        return;
    }

    // Poll sidecar delivery. The inbox is a PERSISTENT cache — both
    // `is_pending_sidecar_delivered` and `take_pending_sidecar_toml` leave the
    // entry in place — which is what lets the poller and the renderer read the
    // same body, and what lets many rocks of one type share one sidecar. Do not
    // "optimise" either into a real take: the prefetch would win the race,
    // discard the TOML, and `render_spawned_entities` would then wait forever
    // for a body that already arrived.
    //
    // On native this is a no-op observation (the inbox is always empty —
    // native `load_sidecar_toml` reads from `std::fs` directly), but the
    // call is cheap and keeps the code path identical across targets.
    let mut still_pending = Vec::new();
    let mut newly_delivered = Vec::new();
    for path in &preload.pending_sidecars {
        if crate::config_cache::is_pending_sidecar_delivered(path) {
            newly_delivered.push(path.clone());
        } else {
            still_pending.push(path.clone());
        }
    }
    preload.pending_sidecars = still_pending;

    // A delivered sidecar can name the rest of its own LOD ladder (issue #914),
    // so expand it now: those GLBs and their sidecars join the gate exactly as
    // the entity-declared ones used to. A path leaves `pending_sidecars` once,
    // so each sidecar is expanded exactly once. `take_pending_sidecar_toml` is
    // non-destructive, so this never steals the TOML from the renderer.
    for path in newly_delivered {
        let Some(toml_str) = crate::config_cache::take_pending_sidecar_toml(&path) else {
            continue;
        };
        let mut ladder = AssetManifest::default();
        let distance = preload.sidecar_distance.get(&path).copied();
        discover_sidecar_lod_assets(&toml_str, &path, distance, &mut ladder);
        for glb_path in &ladder.glb_models {
            if preload.glb_handles.iter().any(|(p, _)| p == glb_path) {
                continue;
            }
            let handle: Handle<bevy::scene::Scene> =
                asset_server.load(format!("{glb_path}#Scene0"));
            preload.glb_handles.push((glb_path.clone(), handle));
        }
        for sc_path in &ladder.sidecars {
            if preload.registered_sidecars.insert(sc_path.clone()) {
                crate::config_cache::request_sidecar_fetch(sc_path.clone());
                preload.pending_sidecars.push(sc_path.clone());
            }
        }
    }

    // Poll sub-world TOML delivery and process incrementally. On native,
    // sub-worlds are read synchronously in `begin_asset_preload` so there
    // is nothing to poll — gate the whole loop on WASM to avoid `mut`
    // bindings that would never be mutated on native.
    #[cfg(target_arch = "wasm32")]
    {
        let mut new_glbs: Vec<String> = Vec::new();
        let mut new_icons: Vec<String> = Vec::new();
        let mut new_planet_textures: Vec<(String, bool)> = Vec::new();
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
                let player_start = preload.player_start;
                // A single reborrow so the two field-level `&mut`s below split
                // off ONE `&mut AssetPreloadResource` rather than each going
                // through `preload`'s own `DerefMut` separately — the borrow
                // checker only proves disjoint field access for the former.
                let preload = &mut *preload;
                match process_sub_world_toml(
                    &toml_str,
                    cache_ref,
                    &mut preload.seen_entities,
                    &mut manifest,
                    world_path,
                    player_start,
                    &mut preload.sidecar_distance,
                ) {
                    Ok(more_worlds) => {
                        new_glbs.extend(manifest.glb_models);
                        // Dust textures ride the icon path — both are Images.
                        new_icons.extend(manifest.radar_icons);
                        new_icons.extend(manifest.pfx_textures);
                        new_planet_textures.extend(manifest.planet_textures);
                        new_sidecars.extend(manifest.sidecars);
                        new_sub_worlds.extend(more_worlds);
                        preload.seen_worlds.insert(world_path.clone());
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
        for (path, srgb) in &new_planet_textures {
            let handle = crate::entity_planet::load_planet_image(&asset_server, path, *srgb);
            preload.icon_handles.push((path.clone(), handle));
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
    // GLBs load from the Lobby so the viewscreen has models ready at game start.
    // Treat Failed as terminal so a bad asset doesn't deadlock the gate.
    let glbs_terminal = glbs_loaded + glbs_failed;
    let glbs_done = glbs_terminal == preload.glb_handles.len();

    // Use dynamic totals so sidecars/sub-worlds discovered from sub-worlds
    // are included: `registered_sidecars` only grows, and `seen_worlds`
    // tracks every world ever processed (minus the synthetic "(base)" entry).
    let total_sidecars = preload.registered_sidecars.len();
    let total_sub_worlds = preload.seen_worlds.len().saturating_sub(1); // subtract "(base)"
    preload.total_count =
        preload.icon_handles.len() + total_sidecars + total_sub_worlds + preload.glb_handles.len();

    let sidecars_done = preload.pending_sidecars.is_empty();
    let sub_worlds_done = preload.pending_sub_worlds.is_empty();
    let icons_ready = preload
        .icon_handles
        .iter()
        .all(|(_, h)| images.get(h).is_some());

    let icon_ready = preload
        .icon_handles
        .iter()
        .filter(|(_, h)| images.get(h).is_some())
        .count();
    let sidecar_ready = total_sidecars.saturating_sub(preload.pending_sidecars.len());
    let sub_world_ready = total_sub_worlds.saturating_sub(preload.pending_sub_worlds.len());
    preload.ready_count = icon_ready + sidecar_ready + sub_world_ready + glbs_terminal;

    if icons_ready && sidecars_done && sub_worlds_done && glbs_done {
        preload.complete = true;
        preload.ready_count = preload.total_count;
        bevy::log::info!(
            "asset_preload: all assets ready — icons={}, sidecars={}, sub_worlds={}, GLBs={}+{} failed",
            icons_ready, sidecars_done, sub_worlds_done, glbs_loaded, glbs_failed,
        );
    } else {
        bevy::log::debug!(
            "asset_preload: waiting — icons={}, sidecars={} (pending {}), sub_worlds={} (pending {}), GLBs {}/{} ({} failed)",
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

/// Broadcast `LoadingProgress` at ~10 Hz during the `Loading` phase.
///
/// Sends the current fraction immediately on first entry (before the timer
/// fires) so the user sees progress even when assets complete before the
/// 0.1 s throttle interval elapses. Subsequent sends are throttled by the
/// repeating timer.
pub fn broadcast_loading_progress(
    mut preload: ResMut<AssetPreloadResource>,
    mut outbox: ResMut<LobbyOutbox>,
    time: Res<Time>,
) {
    if !preload.started || preload.complete {
        return;
    }
    preload.progress_timer.tick(time.delta());

    // Send on first entry regardless of timer state, then let the timer
    // throttle subsequent updates to ~10 Hz.  Without this gate the very
    // first `LoadingProgress` (after the initial 0 %) would be delayed by
    // 0.1 s, and if `poll_asset_preload` sets complete=true before the
    // timer fires, the user would never see anything but 0 %.
    if !preload.progress_timer.just_finished() && preload.progress_sent {
        return;
    }
    preload.progress_sent = true;

    let fraction = preload.fraction();
    outbox
        .0
        .push((Target::All, ServerMessage::LoadingProgress { fraction }));
}

/// Sends an immediate LoadingProgress(0%) on the first frame of Loading so
/// clients always show the loading overlay even when assets complete quickly.
pub fn broadcast_loading_start(mut outbox: ResMut<LobbyOutbox>) {
    outbox.0.push((
        Target::All,
        ServerMessage::LoadingProgress { fraction: 0.0 },
    ));
}

/// Auto-transition from `Loading` → `InProgress` when preload completes.
pub fn auto_transition_from_loading(
    preload: Res<AssetPreloadResource>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
) {
    if !preload.complete {
        bevy::log::debug!(
            "auto_transition_from_loading: preload.complete=false, staying in Loading"
        );
        return;
    }
    bevy::log::info!(
        "auto_transition_from_loading: preload.complete=true, transitioning to InProgress"
    );
    next_state.set(GamePhase::InProgress);
    // Send 100% before GameStarted so clients always see the final value,
    // even when assets completed before the 0.1s timer fired.
    outbox.0.push((
        Target::All,
        ServerMessage::LoadingProgress { fraction: 1.0 },
    ));
    outbox.0.push((Target::All, ServerMessage::GameStarted));
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::*;

    #[test]
    fn icon_asset_path_capitalizes_first_letter() {
        assert_eq!(
            icon_asset_path("destroyer"),
            "radar_icons/Icon-Destroyer.png"
        );
        assert_eq!(icon_asset_path("Star"), "radar_icons/Icon-Star.png");
        assert_eq!(
            icon_asset_path("playerShip"),
            "radar_icons/Icon-PlayerShip.png"
        );
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
        let config = EntityConfig {
            mesh: Some(MeshConfig {
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
            }),
            radar_appearance: Some(RadarAppearanceConfig {
                icon: Some("testShip".into()),
                colour: Some(vec![1.0, 0.0, 0.0]),
                size: None,
                region_colour: None,
            }),
            ..Default::default()
        };

        let mut manifest = AssetManifest::default();
        discover_entity_config_assets(&config, &mut manifest);

        assert_eq!(manifest.glb_models, vec!["models/test_ship.glb"]);
        assert!(manifest
            .sidecars
            .iter()
            .any(|s| s.contains("test_ship.model.toml")));
        assert_eq!(manifest.radar_icons, vec!["radar_icons/Icon-TestShip.png"]);
    }

    /// A world that declares no `[[dust.layer]]` still renders the built-in
    /// layers, so their textures must be discovered even though the world file
    /// never names them.
    #[test]
    fn discover_base_assets_preloads_builtin_dust_textures() {
        let world = WorldConfig {
            dust: Some(crate::world::config::DustPfxConfig {
                enabled: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
        assert!(
            !manifest.pfx_textures.is_empty(),
            "built-in dust layers must contribute textures"
        );
        assert!(
            manifest.pfx_textures.iter().all(|p| p.starts_with("pfx/")),
            "got {:?}",
            manifest.pfx_textures
        );
    }

    /// The player-ship radar icon is injected onto the selected hull at spawn,
    /// not authored in any template, so the template scan never discovers it.
    /// `discover_base_assets` must add it unconditionally so clients always
    /// have the player blip PNG — even for a world that references no ships.
    #[test]
    fn discover_base_assets_always_preloads_player_ship_icon() {
        let world = WorldConfig::default();
        let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
        let expected = icon_asset_path(PLAYER_SHIP_RADAR_ICON);
        assert!(
            manifest.radar_icons.contains(&expected),
            "player-ship radar icon must always preload; got {:?}",
            manifest.radar_icons
        );
    }

    #[test]
    fn discover_base_assets_skips_dust_textures_when_disabled() {
        let world = WorldConfig {
            dust: Some(crate::world::config::DustPfxConfig {
                enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
        assert!(manifest.pfx_textures.is_empty());
    }

    #[test]
    fn discover_base_assets_preloads_declared_dust_textures() {
        let world = WorldConfig {
            dust: Some(crate::world::config::DustPfxConfig {
                layers: vec![crate::world::config::DustLayerConfig {
                    texture: Some("pfx/custom_mote.png".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
        assert!(
            manifest
                .pfx_textures
                .contains(&"pfx/custom_mote.png".to_string()),
            "got {:?}",
            manifest.pfx_textures
        );
    }

    #[test]
    fn discover_entity_config_with_model_adds_glb_and_sidecar() {
        let config = EntityConfig {
            mesh: Some(MeshConfig {
                model: Some("assets/models/alliance_cruiser.glb".into()),
                variant: None,
                shape: MeshShape::Sphere,
                colour: vec![],
                radius: 1.0,
                size: None,
                minor_radius: 0.0,
                emissive: None,
                scale: 1.0,
                rotation: [0.0, 0.0, 0.0],
            }),
            ..Default::default()
        };
        let mut manifest = AssetManifest::default();
        discover_entity_config_assets(&config, &mut manifest);
        // GLB is always added; local ship rendering is skipped at render time.
        assert!(manifest
            .glb_models
            .iter()
            .any(|s| s.contains("alliance_cruiser.glb")));
        // Sidecar must also be added for ModelMarkers.
        assert!(manifest
            .sidecars
            .iter()
            .any(|s| s.contains("alliance_cruiser.model.toml")));
    }

    /// A `[planet]` section contributes every declared texture path with the
    /// correct sRGB flag, so the preload gate covers planet textures and they
    /// load with the same settings the renderer uses.
    #[test]
    fn discover_entity_config_with_planet_adds_textures() {
        let config = EntityConfig {
            planet: Some(PlanetConfig {
                radius: 20.0,
                longitude_segments: 64,
                latitude_segments: 32,
                surface: PlanetSurfaceConfig {
                    albedo: "assets/planets/earth/albedo.webp".into(),
                    normal: Some("assets/planets/earth/normal.webp".into()),
                    roughness: None,
                    emissive_colour: Some("assets/planets/earth/emissive_colour.webp".into()),
                    emissive_mask: None,
                    emissive_night_only: true,
                    emissive_strength: 1.0,
                },
                clouds: Some(PlanetCloudsConfig {
                    albedo: "assets/planets/earth/cloud_albedo.webp".into(),
                    opacity: Some("assets/planets/earth/cloud_opacity.webp".into()),
                    normal: None,
                    scale: 1.03,
                    drift_speed: 0.0,
                }),
                atmosphere: None,
            }),
            ..Default::default()
        };
        let mut manifest = AssetManifest::default();
        discover_entity_config_assets(&config, &mut manifest);
        assert_eq!(
            manifest.planet_textures,
            vec![
                ("assets/planets/earth/albedo.webp".to_string(), true),
                ("assets/planets/earth/normal.webp".to_string(), false),
                (
                    "assets/planets/earth/emissive_colour.webp".to_string(),
                    true
                ),
                ("assets/planets/earth/cloud_albedo.webp".to_string(), true),
                ("assets/planets/earth/cloud_opacity.webp".to_string(), false),
            ]
        );
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

    // ── Sidecar-owned LOD ladders (issue #914) ────────────────────────────

    /// The entity walk sees ONE model now. The far levels are behind the
    /// sidecar, so claiming to have found them here would be a lie that
    /// silently shrinks the loading gate.
    #[test]
    fn the_entity_walk_discovers_only_the_model_it_names() {
        let config = EntityConfig {
            mesh: Some(MeshConfig {
                model: Some("assets/models/rock.glb".into()),
                variant: Some("large".into()),
                shape: MeshShape::Sphere,
                colour: vec![0.5, 0.5, 0.5],
                radius: 4.0,
                size: None,
                minor_radius: 0.0,
                emissive: None,
                scale: 1.0,
                rotation: [0.0, 0.0, 0.0],
            }),
            ..Default::default()
        };
        let mut manifest = AssetManifest::default();
        discover_entity_config_assets(&config, &mut manifest);
        assert_eq!(manifest.glb_models, vec!["models/rock.glb"]);
        assert_eq!(manifest.sidecars, vec!["assets/models/rock.large.toml"]);
    }

    /// The second phase: the delivered sidecar contributes the rest of the
    /// ladder — every far GLB and the sidecar each of those needs in turn.
    #[test]
    fn a_delivered_sidecar_contributes_its_whole_ladder() {
        let sidecar = r#"
[base]
scale = [1.0, 1.0, 1.0]

[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"

[[lod]]
shape = "sphere"
"#;
        let mut manifest = AssetManifest::default();
        discover_sidecar_lod_assets(
            sidecar,
            "assets/models/rock.large.toml",
            None,
            &mut manifest,
        );

        assert_eq!(
            manifest.glb_models,
            vec!["models/rock.glb", "models/rock_lod2.glb"],
            "the procedural fallback level names no GLB"
        );
        // A level that omits `variant` inherits the sidecar's own — which is
        // the same fallback `update_mesh_lod` applies from `[mesh] variant`.
        assert_eq!(
            manifest.sidecars,
            vec![
                "assets/models/rock.large.toml",
                "assets/models/rock_lod2.large.toml"
            ]
        );
    }

    /// Issue lod-preload-by-distance: a known distance preloads ONLY the
    /// level `select_lod` would pick for it, not the whole ladder.
    #[test]
    fn a_known_distance_preloads_only_the_needed_level() {
        let sidecar = r#"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod1.glb"

[[lod]]
max_distance = 300.0
model = "assets/models/rock_lod2.glb"

[[lod]]
shape = "sphere"
"#;
        // 200 world units falls in the third band (150..300) -> index 2.
        let mut manifest = AssetManifest::default();
        discover_sidecar_lod_assets(
            sidecar,
            "assets/models/rock.large.toml",
            Some(200.0),
            &mut manifest,
        );
        assert_eq!(
            manifest.glb_models,
            vec!["models/rock_lod2.glb"],
            "only the level covering 200 units must preload"
        );
        assert_eq!(
            manifest.sidecars,
            vec!["assets/models/rock_lod2.large.toml"]
        );
    }

    /// The nearest band (distance 0) selects level 0 — the entity's own named
    /// model, not a decimated step.
    #[test]
    fn a_known_distance_at_the_near_band_preloads_the_base_level() {
        let sidecar = r#"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod1.glb"

[[lod]]
shape = "sphere"
"#;
        let mut manifest = AssetManifest::default();
        discover_sidecar_lod_assets(
            sidecar,
            "assets/models/rock.large.toml",
            Some(5.0),
            &mut manifest,
        );
        assert_eq!(manifest.glb_models, vec!["models/rock.glb"]);
    }

    /// The final, unbounded level (usually the procedural-sphere fallback)
    /// names no GLB — a distance that lands there must not error, just yield
    /// an empty manifest.
    #[test]
    fn a_known_distance_past_every_glb_level_yields_no_glb() {
        let sidecar = r#"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
shape = "sphere"
"#;
        let mut manifest = AssetManifest::default();
        discover_sidecar_lod_assets(
            sidecar,
            "assets/models/rock.large.toml",
            Some(10_000.0),
            &mut manifest,
        );
        assert!(manifest.glb_models.is_empty());
        assert!(manifest.sidecars.is_empty());
    }

    #[test]
    fn a_level_may_override_the_variant_it_inherits() {
        let sidecar = "[[lod]]\nmodel = \"assets/models/rock_lod1.glb\"\nvariant = \"weathered\"\n";
        let mut manifest = AssetManifest::default();
        discover_sidecar_lod_assets(
            sidecar,
            "assets/models/rock.large.toml",
            None,
            &mut manifest,
        );
        assert_eq!(
            manifest.sidecars,
            vec!["assets/models/rock_lod1.weathered.toml"]
        );
    }

    /// Absent (404 → empty push) and malformed sidecars contribute nothing and
    /// must not panic: the entity still renders its flat `[mesh]`.
    #[test]
    fn an_absent_or_malformed_sidecar_contributes_no_ladder() {
        for body in ["", "   \n", "[[lod]\nbroken", "lods = 3\n"] {
            let mut manifest = AssetManifest::default();
            discover_sidecar_lod_assets(body, "assets/models/rock.large.toml", None, &mut manifest);
            assert!(manifest.glb_models.is_empty(), "body: {body:?}");
            assert!(manifest.sidecars.is_empty(), "body: {body:?}");
        }
    }

    /// A ship hull's sidecar declares no ladder at all — the common case, and
    /// it must stay a no-op rather than registering phantom assets.
    #[test]
    fn a_sidecar_with_markers_but_no_ladder_contributes_nothing() {
        let sidecar = "[markers.fore]\nposition = [0.0, 0.0, -1.0]\ndirection = [0.0, 0.0, -1.0]\n";
        let mut manifest = AssetManifest::default();
        discover_sidecar_lod_assets(
            sidecar,
            "assets/models/ship.model.toml",
            None,
            &mut manifest,
        );
        assert!(manifest.glb_models.is_empty());
        assert!(manifest.sidecars.is_empty());
    }

    // ── resolve_player_start (issue lod-preload-by-distance) ─────────────────

    #[test]
    fn resolve_player_start_prefers_explicit_position() {
        let world = WorldConfig {
            player_spawn: Some(crate::world::config::PlayerSpawnEntry {
                anchor: None,
                position: Some([10.0, 0.0, 20.0]),
                rotation: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            resolve_player_start(&world, &HashMap::new()),
            [10.0, 0.0, 20.0]
        );
    }

    #[test]
    fn resolve_player_start_falls_back_to_player_spawn_anchor() {
        let mut anchors = HashMap::new();
        anchors.insert("dock".to_string(), [5.0, 0.0, 5.0]);
        let world = WorldConfig {
            anchors,
            player_spawn: Some(crate::world::config::PlayerSpawnEntry {
                anchor: Some("dock".to_string()),
                position: None,
                rotation: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            resolve_player_start(&world, &HashMap::new()),
            [5.0, 0.0, 5.0]
        );
    }

    /// No `[player_spawn]` at all: falls back to wherever the player ship's
    /// own `[[entity]]` instance resolves to — same precedence
    /// `spawn_game_start_entities` applies when it actually places the ship.
    #[test]
    fn resolve_player_start_falls_back_to_the_player_ship_entity() {
        let world = WorldConfig {
            entities: vec![crate::world::config::WorldEntity {
                template_path: "assets/entities/alliance_cruiser.toml".to_string(),
                spawn_on: crate::world::config::WorldEntitySpawnOn::GameStart,
                transform: Some(crate::world::config::TransformConfig {
                    position: Some([30.0, 0.0, 40.0]),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut config_cache = HashMap::new();
        config_cache.insert(
            "assets/entities/alliance_cruiser.toml".to_string(),
            EntityConfig {
                tags: vec!["ship".to_string()],
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_player_start(&world, &config_cache),
            [30.0, 0.0, 40.0]
        );
    }

    #[test]
    fn resolve_player_start_defaults_to_origin_when_nothing_resolves() {
        let world = WorldConfig::default();
        assert_eq!(
            resolve_player_start(&world, &HashMap::new()),
            [0.0, 0.0, 0.0]
        );
    }

    // ── distance-based LOD preload: discover_world_assets/discover_base_assets

    /// A placed `[[entity]]` instance's distance from the player start is
    /// tracked per the sidecar its model resolves to (issue
    /// lod-preload-by-distance) — `discover_sidecar_lod_assets` reads it back
    /// once that sidecar's TOML is delivered.
    #[test]
    fn discover_base_assets_tracks_the_closest_instance_distance_per_sidecar() {
        let world = WorldConfig {
            player_spawn: Some(crate::world::config::PlayerSpawnEntry {
                anchor: None,
                position: Some([0.0, 0.0, 0.0]),
                rotation: None,
            }),
            entities: vec![crate::world::config::WorldEntity {
                template_path: "assets/entities/outpost.toml".to_string(),
                transform: Some(crate::world::config::TransformConfig {
                    position: Some([30.0, 0.0, 40.0]),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut config_cache = HashMap::new();
        config_cache.insert(
            "assets/entities/outpost.toml".to_string(),
            EntityConfig {
                mesh: Some(crate::entity_config::MeshConfig {
                    model: Some("assets/models/outpost.glb".into()),
                    variant: None,
                    shape: crate::entity_config::MeshShape::Sphere,
                    colour: vec![],
                    radius: 1.0,
                    size: None,
                    minor_radius: 0.0,
                    emissive: None,
                    scale: 1.0,
                    rotation: [0.0, 0.0, 0.0],
                }),
                ..Default::default()
            },
        );

        let (_manifest, _pending, _seen, sidecar_distance, player_start) =
            discover_base_assets(&world, &config_cache);

        assert_eq!(player_start, [0.0, 0.0, 0.0]);
        // 30-40-0 from the origin is a 3-4-5 triangle scaled by 10 -> 50.
        let sc = sidecar_path("assets/models/outpost.glb", None);
        assert_eq!(sidecar_distance.get(&sc).copied(), Some(50.0));
    }

    /// Two instances of the same template at different distances: the
    /// tracked distance is the CLOSEST one — that is the instance whose LOD
    /// actually needs the detail preloaded.
    #[test]
    fn discover_base_assets_keeps_the_minimum_distance_across_instances() {
        fn entity_at(pos: [f32; 3]) -> crate::world::config::WorldEntity {
            crate::world::config::WorldEntity {
                template_path: "assets/entities/outpost.toml".to_string(),
                transform: Some(crate::world::config::TransformConfig {
                    position: Some(pos),
                    ..Default::default()
                }),
                ..Default::default()
            }
        }
        let world = WorldConfig {
            player_spawn: Some(crate::world::config::PlayerSpawnEntry {
                anchor: None,
                position: Some([0.0, 0.0, 0.0]),
                rotation: None,
            }),
            entities: vec![entity_at([100.0, 0.0, 0.0]), entity_at([10.0, 0.0, 0.0])],
            ..Default::default()
        };
        let mut config_cache = HashMap::new();
        config_cache.insert(
            "assets/entities/outpost.toml".to_string(),
            EntityConfig {
                mesh: Some(crate::entity_config::MeshConfig {
                    model: Some("assets/models/outpost.glb".into()),
                    variant: None,
                    shape: crate::entity_config::MeshShape::Sphere,
                    colour: vec![],
                    radius: 1.0,
                    size: None,
                    minor_radius: 0.0,
                    emissive: None,
                    scale: 1.0,
                    rotation: [0.0, 0.0, 0.0],
                }),
                ..Default::default()
            },
        );

        let (_manifest, _pending, _seen, sidecar_distance, _player_start) =
            discover_base_assets(&world, &config_cache);

        let sc = sidecar_path("assets/models/outpost.glb", None);
        assert_eq!(sidecar_distance.get(&sc).copied(), Some(10.0));
    }

    /// `extra_worlds` entries are de-duplicated before they reach the fetch
    /// queue.
    ///
    /// The duplicate used to come from two `[[trigger]]` blocks naming the same
    /// `load_world` path; issue #985 deleted that walk with the front-end that
    /// fed it, and a scripted `load_world` is not discoverable before its handler
    /// runs (the layer applier fetches on demand instead). `extra_worlds` is the
    /// surviving static source, and a duplicate there is the same hang: the first
    /// `pop_pending_world_toml` consumes the TOML and the second copy waits
    /// forever, because `request_world_fetch` will not re-fire.
    #[test]
    fn discover_base_assets_deduplicates_extra_world_paths() {
        use crate::world::config::parse_world;

        let toml = r#"
extra_worlds = [
  "assets/worlds/branch_a.toml",
  "assets/worlds/branch_a.toml",
  "assets/worlds/branch_b.toml",
]

[global]
seed = 1
title = "Test"
"#;

        let world = parse_world(toml).expect("parse must succeed");
        let config_cache = HashMap::new();
        let (_manifest, pending_worlds, _, _, _) = discover_base_assets(&world, &config_cache);

        let branch_a_count = pending_worlds
            .iter()
            .filter(|p| p.as_str() == "assets/worlds/branch_a.toml")
            .count();
        assert_eq!(
            branch_a_count, 1,
            "duplicate sub-world path causes permanent preload hang; got {branch_a_count} copies"
        );
        assert_eq!(pending_worlds.len(), 2, "expected branch_a + branch_b only");
    }
}
