//! Spawning a GLB model as a child visual, composed with its `.model.toml` rig.
//!
//! This is the single implementation of "turn a model path into something on
//! screen". The game's flat renderer (`render_spawned_entities`) and LOD swapper
//! (`update_mesh_lod`) both go through [`spawn_glb_visual`], as does the
//! Workshop model preview, so all three share identical async loading and
//! rig-composition behaviour.

use bevy::prelude::*;

// Compatibility re-exports for viewer/render call sites that historically
// reached the sidecar resolver through this presentation module. Ownership is
// now simulation-side in `model_markers` (issue #1291).
pub use crate::entities::model_markers::{
    resolve_sidecar_rig, resolve_sidecar_rig_optional, Absence,
};

/// Holds a pending GLB scene handle so the asset server keeps the asset alive
/// across frames until it finishes loading.
#[derive(Component)]
pub struct PendingSceneHandle(pub Handle<bevy::scene::Scene>);

/// The extra scale a NON-near LOD tier's composition root must supply, given
/// the primary sidecar's `[base].scale` and the scale the ladder's own
/// GENERATED tiers already carry.
///
/// Every tier of one model has to reach the same world size — the primary
/// sidecar's `[base].scale`. Two ladder shapes deliver it differently, and
/// nothing that composes a tier may assume either:
///
/// * A **hull ladder** (every ship, the starbase, the research outpost) ships
///   no sidecar beside its generated tier GLBs. Each generated tier therefore
///   resolves an identity rig, so the tier root must supply the whole base
///   scale.
///   This is the case bf4c4b02 fixed: before it, a starbase at
///   `[base].scale = [15, 18, 18]` snapped back to raw model size the moment it
///   left its near band.
/// * A **pipeline ladder** (every asteroid class, since e20a5035) writes a
///   sidecar beside EVERY tier GLB carrying the primary's `[base]` rig
///   verbatim. The GLB child applies the base scale itself, so the tier root
///   must supply NONE of it.
///
/// This is a question about GLB TIERS ONLY. A billboard level's `scale` is the
/// quad's world size on both conventions — `capture-billboards.mjs` records it
/// that way on both — so nothing here is folded onto it. bf4c4b02 did fold it,
/// and drew every hull ladder's imposter at its own `[base].scale` too large;
/// see [`crate::entities::billboard::billboard_quad_size`], which is the one
/// place that rule lives.
///
/// Dividing the base scale by whatever a generated tier already carries covers
/// both without either convention having to know about the other: an identity
/// child yields the whole base scale, a base-scaled child yields 1. Folding the
/// base scale in unconditionally instead SQUARES it on a pipeline ladder, which
/// is how a `huge` rock (`[base].scale` 12.6756) came to render at 160.67x raw
/// instead of 12.6756x — 12.68x oversize, "almost planet sized" — from the
/// moment it crossed out of its 45-unit near band.
///
/// Lives here, beside [`resolve_sidecar_rig`], because it is a fact about how a
/// model's rig composes across its ladder — not about either of the two things
/// that need the answer. `update_mesh_lod` (the game) and `super::super::viewer`
/// (the Workshop model preview) both build a tier's transform, and a second
/// copy of this reasoning in the viewer is exactly how the viewer came to be
/// showing a size the game did not.
pub fn tier_parent_scale(base_scale: [f32; 3], generated_child_scale: [f32; 3]) -> Vec3 {
    // A zero/degenerate child scale carries no usable information — read it as
    // the hull-ladder case rather than dividing by ~0 into a non-finite scale.
    let axis = |base: f32, child: f32| {
        if child.abs() > 1e-6 {
            base / child
        } else {
            base
        }
    };
    Vec3::new(
        axis(base_scale[0], generated_child_scale[0]),
        axis(base_scale[1], generated_child_scale[1]),
        axis(base_scale[2], generated_child_scale[2]),
    )
}

/// Resolve [`tier_parent_scale`] for a ladder from its first GENERATED tier —
/// the first level past the near one that carries its own GLB. That one tier
/// settles the convention for the whole ladder, because a ladder's tiers are
/// generated together, by one pipeline, from one source model.
/// (`every_shipped_ladder_holds_one_world_size_across_its_tiers` holds that
/// claim to the shipped assets: no ladder mixes the two conventions.)
///
/// The tier states its convention itself, in
/// [`crate::entities::config::TierRig`], written by the script that authored the
/// ladder. A declared `Identity` tier is answered without reading anything: the
/// claim is precisely "there is no sidecar here", and the old way of checking
/// that was to fetch the absent file and watch the 404 come back — one alarming
/// console error per hull model, per browser session, for shipped content
/// behaving exactly as designed.
///
/// `entity_variant` is the `[mesh] variant` fallback a level uses when it
/// declares none of its own — the same fallback the GLB spawn path applies.
///
/// Returns `None` only on wasm, while a sidecar's fetch is still in flight; the
/// caller retries next frame, the same wait the GLB spawn path already takes. On
/// native the read is synchronous and this always resolves.
pub fn resolve_tier_parent_scale(
    levels: &[crate::entities::config::LodLevel],
    base_scale: [f32; 3],
    entity_variant: Option<&str>,
) -> Option<Vec3> {
    // Level 0 is the primary GLB itself and so always resolves the PRIMARY
    // sidecar — it says nothing about how the GENERATED tiers were written, and
    // asking it would report every ladder as already pre-scaled.
    let generated = levels.iter().skip(1).find_map(|level| {
        level
            .model
            .as_deref()
            .map(|m| (m, level.variant.as_deref(), level.tier_rig))
    });
    let Some((model_path, level_variant, tier_rig)) = generated else {
        // A ladder with no generated GLB tier (a near GLB straight to a
        // billboard) has nothing to measure against, so keep the hull reading.
        return Some(Vec3::from_array(base_scale));
    };
    let variant = level_variant.or(entity_variant);
    let child_scale = match tier_rig {
        // Declared: no sidecar there, so the tier resolves an identity rig and
        // the parent owes it everything. Read nothing.
        Some(crate::entities::config::TierRig::Identity) => [1.0, 1.0, 1.0],
        // Declared: a sidecar IS there. Read it — the number that matters is
        // what that file actually says, not what the convention implies it
        // ought to say, and the fetch resolves rather than 404ing.
        Some(crate::entities::config::TierRig::Baked) => {
            resolve_sidecar_rig(model_path, variant)?.base.scale
        }
        // Undeclared — a sidecar predating the field, which in practice means
        // mod-pack content. Probe as this always did, but as an existence test:
        // an absent file is one of the two answers, not a failed fetch.
        None => {
            resolve_sidecar_rig_optional(model_path, variant)?
                .base
                .scale
        }
    };
    Some(tier_parent_scale(base_scale, child_scale))
}

/// The rig a generated tier resolves to WITHOUT reading anything, when its
/// ladder has already declared that no sidecar sits beside it.
///
/// `Some(identity)` means "hand this straight to [`spawn_glb_visual`] and let it
/// skip the read"; `None` means the level says nothing and the sidecar must be
/// resolved the ordinary way. Every path that builds a generated tier's visual
/// asks this first, so a hull ladder's absent per-tier sidecars are never
/// requested by anyone — the probe was only one of the three askers.
pub fn declared_tier_rig(
    level: &crate::entities::config::LodLevel,
) -> Option<crate::entities::model_rig::ModelRig> {
    match level.tier_rig {
        Some(crate::entities::config::TierRig::Identity) => {
            Some(crate::entities::model_rig::ModelRig::default())
        }
        _ => None,
    }
}

/// The extra composition scale for the tier at `index`.
///
/// The near tier (index 0) IS the primary GLB, so its child already carries the
/// whole `[base].scale` from the primary sidecar and needs no extra scale;
/// every other tier takes [`resolve_tier_parent_scale`]'s answer. The game puts
/// this on a presentation-only visual root, while Workshop preview may put
/// it on its preview subject. Both ask the same composition question here.
pub fn tier_parent_scale_at(index: usize, ladder_tier_scale: Vec3) -> Vec3 {
    if index == 0 {
        Vec3::ONE
    } else {
        ladder_tier_scale
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
/// the rig sidecar, then spawns the `SceneRoot` child. Returns the spawned child
/// so callers can tear it down on an LOD switch, or decorate it — the local
/// ship, for instance, adds `Visibility::Hidden` and `NoFrustumCulling` to the
/// returned entity. Authoritative marker geometry is deliberately outside this
/// presentation helper; [`crate::entities::model_markers`] resolves it from the
/// primary authored rig independently of whichever visual is active.
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
    resolved_rig: Option<&crate::entities::model_rig::ModelRig>,
) -> GlbSpawnOutcome {
    let scene: Handle<bevy::scene::Scene> = match pending {
        Some(p) => p.0.clone(),
        None => {
            // `asset_server` resolves paths relative to the `assets/` root, but
            // the TOML `model` field carries an `assets/` prefix. Strip it so
            // the GLB resolves instead of looking for `assets/assets/...`.
            let rel = model_path.strip_prefix("assets/").unwrap_or(model_path);
            let path = super::pack_assets::asset_path(asset_server, &format!("{rel}#Scene0"));
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
    let rig: &crate::entities::model_rig::ModelRig = match resolved_rig {
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
        .spawn((
            bevy::scene::SceneRoot(scene),
            base_tf,
            super::pack_assets::PackVisualRoot,
        ))
        .id();
    commands.entity(entity).add_child(child);
    GlbSpawnOutcome::Spawned(child)
}

// ── Tests ────────────────────────────────────────────────────────────
#[cfg(test)]
#[path = "glb_visual_tests.rs"]
mod tests;
