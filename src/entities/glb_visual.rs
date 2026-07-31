//! Spawning a GLB model as a child visual, composed with its `.model.toml` rig.
//!
//! This is the single implementation of "turn a model path into something on
//! screen". The game's flat renderer (`render_spawned_entities`) and LOD swapper
//! (`update_mesh_lod`) both go through [`spawn_glb_visual`], as does the
//! standalone model viewer, so all three share identical async loading and
//! rig-composition behaviour.

use bevy::prelude::*;

/// Holds a pending GLB scene handle so the asset server keeps the asset alive
/// across frames until it finishes loading.
#[derive(Component)]
pub struct PendingSceneHandle(pub Handle<bevy::scene::Scene>);

/// Read a model-rig sidecar TOML for `path`.
///
/// - **Native**: `std::fs::read_to_string` (returns `None` when absent).
/// - **WASM**: checks the pending-sidecar queue populated by JS via
///   `wasm_push_sidecar_toml`; fires a deferred JS fetch on first miss and
///   returns `None` until the fetch resolves. An empty pushed string (404)
///   resolves to `Some(String::new())`, which parses to an identity rig.
///
/// **Non-destructive**: the entry stays in the queue, so every entity sharing a
/// model reads the same body and the preload poller can read it too (that is
/// what lets `asset_preload` expand a sidecar's `[[lod]]` chain without stealing
/// it from the renderer). Callers that only need readiness should still prefer
/// [`crate::config_cache::is_pending_sidecar_delivered`].
fn load_sidecar_toml(path: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::take_pending_sidecar_toml(path).or_else(|| {
            crate::config_cache::request_sidecar_fetch(path.to_string());
            None
        })
    }
}

/// Resolve a model's rig sidecar to a `ModelRig`.
///
/// Returns:
/// - `Some(rig)` once the sidecar is resolved — either parsed, or an identity
///   rig when the sidecar is genuinely absent (native: file missing; wasm: JS
///   pushed an empty string for a 404) or fails to parse.
/// - `None` while a wasm fetch is still in flight (caller retries next frame).
///   On native this never returns `None` (the filesystem read is synchronous).
///
/// # Failure modes now that the sidecar owns the LOD chain (issue #914)
///
/// The identity fallback is deliberately *degrade, never black-hole*: a model
/// with no readable sidecar still appears on screen. But an identity rig also
/// carries an EMPTY `lod`, so the two absence cases mean different things and
/// are reported differently:
///
/// * **Genuinely absent sidecar** — no ladder was ever authored. That is the
///   normal case for every ship hull, so it is silent, and the entity renders
///   its flat `[mesh]` exactly as a model with no ladder always has.
/// * **Present but malformed sidecar** — the author *did* write something and
///   we cannot tell how much of it was a ladder. Falling back silently would
///   drop the whole chain and quietly render one level forever, so this logs at
///   ERROR (not warn) and says so explicitly.
pub fn resolve_sidecar_rig(
    model_path: &str,
    variant: Option<&str>,
) -> Option<crate::model_rig::ModelRig> {
    let path = crate::model_rig::sidecar_path(model_path, variant);
    match load_sidecar_toml(&path) {
        Some(toml_str) => {
            if toml_str.trim().is_empty() {
                // Absent (404 / empty) → identity rig so the model still renders.
                Some(crate::model_rig::ModelRig::default())
            } else {
                match crate::model_rig::ModelRig::from_toml(&toml_str) {
                    Ok(rig) => Some(rig),
                    Err(e) => {
                        // A present-but-malformed sidecar degrades to an identity
                        // rig so the model still renders — but that identity rig
                        // has no markers AND no LOD chain, so say both out loud
                        // rather than let a typo pass as "this model has no ladder".
                        bevy::log::error!(
                            target: crate::logging::LogCat::Assets.target(),
                            "rig sidecar {path} failed to parse: {e}; falling back to an \
                             identity rig — this model loses its markers AND any [[lod]] \
                             chain, and will render only its flat [mesh] level"
                        );
                        Some(crate::model_rig::ModelRig::default())
                    }
                }
            }
        }
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                // Native: a missing file is "genuinely absent" → identity rig.
                Some(crate::model_rig::ModelRig::default())
            }
            #[cfg(target_arch = "wasm32")]
            {
                // WASM: fetch still in flight → retry next frame.
                None
            }
        }
    }
}

/// Outcome of attempting to spawn a GLB visual (flat render or LOD swap).
pub enum GlbSpawnOutcome {
    /// The scene + rig resolved; the `SceneRoot` child entity was spawned.
    Spawned(Entity),
    /// The scene asset or rig sidecar is still loading — retry next frame.
    Pending,
    /// The GLB failed to load permanently.
    Failed,
}

/// Spawn a GLB scene as a child of `entity`, mirroring PATH A of the flat
/// renderer. Resolves the scene handle (storing a [`PendingSceneHandle`] on the
/// parent to keep it alive across frames), waits for both the scene asset and
/// the rig sidecar, then spawns the `SceneRoot` child and attaches
/// [`crate::model_rig::ModelMarkers`] to the parent. Returns the spawned child
/// so callers can tear it down on an LOD switch, or decorate it — the local
/// ship, for instance, adds `Visibility::Hidden` and `NoFrustumCulling` to the
/// returned entity.
///
/// `resolved_rig` lets a caller that has ALREADY resolved this exact sidecar
/// this frame (to answer some prior question, e.g. `render_spawned_entities`
/// checking whether the model has a `[[lod]]` chain at all) hand the rig
/// straight through instead of making this function read/parse the same
/// sidecar a second time. Pass `None` to resolve it here as before.
pub fn spawn_glb_visual(
    commands: &mut Commands,
    asset_server: &AssetServer,
    scenes: &Assets<bevy::scene::Scene>,
    entity: Entity,
    model_path: &str,
    variant: Option<&str>,
    pending: Option<&PendingSceneHandle>,
    resolved_rig: Option<&crate::model_rig::ModelRig>,
) -> GlbSpawnOutcome {
    let scene: Handle<bevy::scene::Scene> = match pending {
        Some(p) => p.0.clone(),
        None => {
            // `asset_server` resolves paths relative to the `assets/` root, but
            // the TOML `model` field carries an `assets/` prefix. Strip it so
            // the GLB resolves instead of looking for `assets/assets/...`.
            let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
            let path = format!("{}#Scene0", rel);
            let h: Handle<bevy::scene::Scene> = asset_server.load(&path);
            bevy::log::info!(
                "spawn_glb_visual: requesting scene {path} (load_state={:?})",
                asset_server.load_state(h.id())
            );
            commands
                .entity(entity)
                .insert(PendingSceneHandle(h.clone()));
            h
        }
    };
    // A `LoadState::Failed` GLB never appears in `Assets<Scene>`, so stop
    // retrying and let the caller settle without a mesh.
    if matches!(
        asset_server.load_state(scene.id()),
        bevy::asset::LoadState::Failed(_)
    ) {
        bevy::log::warn!(
            "spawn_glb_visual: GLB failed to load for entity {entity:?}, path={model_path} — entity will exist without a mesh"
        );
        commands.entity(entity).remove::<PendingSceneHandle>();
        return GlbSpawnOutcome::Failed;
    }
    // Wait for BOTH the GLB scene AND the rig sidecar before finalising.
    if scenes.get(&scene).is_none() {
        return GlbSpawnOutcome::Pending;
    }
    // Only re-read the sidecar when the caller hasn't already resolved it.
    let rig_owned;
    let rig: &crate::model_rig::ModelRig = match resolved_rig {
        Some(rig) => rig,
        None => {
            rig_owned = match resolve_sidecar_rig(model_path, variant) {
                Some(rig) => rig,
                // Sidecar fetch still in flight (wasm) — retry next frame.
                None => return GlbSpawnOutcome::Pending,
            };
            &rig_owned
        }
    };
    commands.entity(entity).remove::<PendingSceneHandle>();

    // Composition: entityTransform ∘ baseRig ∘ model. The base rig is applied
    // INNER to the per-entity transform by spawning the GLB SceneRoot as a
    // CHILD carrying `base_bevy_transform()`.
    let base_tf = rig.base_bevy_transform();
    let child = commands
        .spawn((bevy::scene::SceneRoot(scene), base_tf))
        .id();
    commands.entity(entity).add_child(child);
    // Attach the resolved marker map so downstream systems (weapons, exhaust, …)
    // can resolve mount points by name.
    commands
        .entity(entity)
        .insert(crate::model_rig::ModelMarkers::from_rig(rig));
    GlbSpawnOutcome::Spawned(child)
}
