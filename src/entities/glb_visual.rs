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
fn load_sidecar_toml(path: &str, absence: Absence) -> Option<String> {
    let text = load_sidecar_toml_text(path, absence);
    // Issue #935: a rig sidecar is authored content too — record it into the
    // ledger the same way the world/entity loaders do.
    if let Some(text) = &text {
        crate::content_ledger::record(path, text);
    }
    text
}

/// Whether a sidecar that turns out not to exist is news.
///
/// On native this changes nothing — a missing file is a `None` from the
/// filesystem either way. On wasm the read is an HTTP fetch, and the page logs a
/// failed one as an error, because a sidecar that should be there and is not is
/// a model rendering without its markers and its ladder. A read we are taking
/// *in order to find out whether the file exists* must not be reported that way:
/// absence is one of its two valid answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Absence {
    /// The file is expected to exist; a 404 is a defect worth logging.
    Unexpected,
    /// The read is itself the existence test; a 404 is an answer, not a fault.
    Expected,
}

fn load_sidecar_toml_text(path: &str, absence: Absence) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = absence;
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::take_pending_sidecar_toml(path).or_else(|| {
            crate::config_cache::request_sidecar_fetch(
                path.to_string(),
                absence == Absence::Expected,
            );
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
    resolve_sidecar_rig_where(model_path, variant, Absence::Unexpected)
}

/// [`resolve_sidecar_rig`] for a read that is ITSELF an existence test — the
/// legacy convention probe in [`resolve_tier_parent_scale`], and nothing else.
///
/// Same answers, but a wasm 404 resolves quietly to the identity rig instead of
/// being logged as a failed fetch. A ladder that declares `tier_rig` never comes
/// here; only a sidecar predating the field does.
pub fn resolve_sidecar_rig_optional(
    model_path: &str,
    variant: Option<&str>,
) -> Option<crate::model_rig::ModelRig> {
    resolve_sidecar_rig_where(model_path, variant, Absence::Expected)
}

fn resolve_sidecar_rig_where(
    model_path: &str,
    variant: Option<&str>,
    absence: Absence,
) -> Option<crate::model_rig::ModelRig> {
    let path = crate::model_rig::sidecar_path(model_path, variant);
    match load_sidecar_toml(&path, absence) {
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

/// The scale a NON-near LOD tier folds onto the PARENT transform, given the
/// primary sidecar's `[base].scale` and the scale the ladder's own GENERATED
/// tiers already carry.
///
/// Every tier of one model has to reach the same world size — the primary
/// sidecar's `[base].scale`. Two ladder shapes deliver it differently, and
/// nothing that composes a tier may assume either:
///
/// * A **hull ladder** (every ship, the starbase, the research outpost) ships
///   no sidecar beside its generated tier GLBs. Each generated tier therefore
///   resolves an identity rig, so the parent must supply the whole base scale.
///   This is the case bf4c4b02 fixed: before it, a starbase at
///   `[base].scale = [15, 18, 18]` snapped back to raw model size the moment it
///   left its near band.
/// * A **pipeline ladder** (every asteroid class, since e20a5035) writes a
///   sidecar beside EVERY tier GLB carrying the primary's `[base]` rig
///   verbatim. The child applies the base scale itself, so the parent must
///   supply NONE of it.
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
/// (the standalone model viewer) both build a tier's transform, and a second
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
/// [`crate::entity_config::TierRig`], written by the script that authored the
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
    levels: &[crate::entity_config::LodLevel],
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
        Some(crate::entity_config::TierRig::Identity) => [1.0, 1.0, 1.0],
        // Declared: a sidecar IS there. Read it — the number that matters is
        // what that file actually says, not what the convention implies it
        // ought to say, and the fetch resolves rather than 404ing.
        Some(crate::entity_config::TierRig::Baked) => {
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
    level: &crate::entity_config::LodLevel,
) -> Option<crate::model_rig::ModelRig> {
    match level.tier_rig {
        Some(crate::entity_config::TierRig::Identity) => {
            Some(crate::model_rig::ModelRig::default())
        }
        _ => None,
    }
}

/// The scale the tier at `index` folds onto its PARENT transform.
///
/// The near tier (index 0) IS the primary GLB, so its child already carries the
/// whole `[base].scale` from the primary sidecar and the parent must fold in
/// nothing; every other tier takes [`resolve_tier_parent_scale`]'s answer. Both
/// the game's LOD swap and the viewer's ask exactly this question, so they ask
/// it in one place.
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

// ── Tests ────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// A HULL ladder ships no sidecar beside its generated tier GLBs, so each
    /// resolves an identity rig and the parent owes that tier the whole
    /// `[base].scale`. These are `alliance_starbase.model.toml`'s real numbers —
    /// the model bf4c4b02 was written for. That repair must survive.
    #[test]
    fn a_hull_ladder_far_tier_takes_the_whole_base_scale() {
        assert_eq!(
            tier_parent_scale([15.0, 18.0, 18.0], [1.0, 1.0, 1.0]),
            Vec3::new(15.0, 18.0, 18.0),
            "a generated tier with no sidecar of its own must be scaled by the \
             parent, or the starbase snaps back to raw model size past its near band"
        );
    }

    /// A PIPELINE ladder writes the primary's `[base]` rig beside EVERY tier
    /// GLB, so the child already applies the base scale and the parent owes it
    /// nothing. These are `asteroid_common_1.huge.toml`'s real numbers, and the
    /// square of them is precisely the bug John reported: folding the base scale
    /// in regardless rendered a `huge` rock at 12.6756² = 160.67x raw instead of
    /// 12.6756x — "almost planet sized".
    #[test]
    fn a_pipeline_ladder_far_tier_takes_none_of_the_base_scale() {
        let huge_rock = [12.675_623, 12.675_623, 12.675_623];
        // `asteroid_common_1_lod1.huge.toml` carries the primary rig verbatim.
        let got = tier_parent_scale(huge_rock, huge_rock);
        assert!(
            (got - Vec3::ONE).length() < 1e-5,
            "a generated tier that carries its own base rig must not be scaled \
             again by the parent, got {got:?}"
        );
    }

    /// Every size class lands on 1 — the error factor WAS the base scale, which
    /// is why the fault scaled with the rock and the X3 class showed it worst.
    #[test]
    fn every_rock_size_class_takes_none_of_the_base_scale() {
        // asteroid_common_1's four shipped size classes, to f32 precision.
        for scale in [1.056_302_f32, 2.112_604, 4.225_208, 12.675_623] {
            let base = [scale, scale, scale];
            let got = tier_parent_scale(base, base);
            assert!(
                (got - Vec3::ONE).length() < 1e-5,
                "size class {scale} must fold nothing onto the parent, got {got:?}"
            );
        }
    }

    /// A zero child scale carries no information about the ladder's convention,
    /// so it reads as the hull case rather than dividing into a non-finite scale.
    #[test]
    fn a_degenerate_child_scale_reads_as_the_hull_convention() {
        let got = tier_parent_scale([15.0, 18.0, 18.0], [0.0, 0.0, 0.0]);
        assert_eq!(got, Vec3::new(15.0, 18.0, 18.0));
        assert!(
            got.is_finite(),
            "must never produce a non-finite parent scale"
        );
    }

    // ── `tier_rig`: the ladder states its convention instead of being probed ──

    use crate::entity_config::{LodLevel, TierRig};

    /// A real asteroid tier: its sidecar exists and carries a base scale far
    /// from 1, so "did the resolver read this file?" has a visible answer.
    const BAKED_TIER: &str = "assets/models/asteroid_common_1_lod1.glb";
    const BAKED_TIER_VARIANT: Option<&str> = Some("small");

    fn level(model: &str, variant: Option<&str>, tier_rig: Option<TierRig>) -> LodLevel {
        LodLevel {
            max_distance: Some(100.0),
            model: Some(model.to_string()),
            variant: variant.map(str::to_string),
            tier_rig,
            ..Default::default()
        }
    }

    fn near_level() -> LodLevel {
        level("assets/models/asteroid_common_1.glb", None, None)
    }

    /// The scale the shipped `asteroid_common_1_lod1.small.toml` actually
    /// applies — what a probe of that file would come back with.
    fn baked_tier_scale() -> [f32; 3] {
        resolve_sidecar_rig(BAKED_TIER, BAKED_TIER_VARIANT)
            .expect("native sidecar reads are synchronous")
            .base
            .scale
    }

    /// A level that DECLARES `identity` is answered from the declaration, not
    /// from the file. Proven by pointing it at a tier whose sidecar exists and
    /// carries a large base scale: a resolver that still read it would divide by
    /// that scale and come back with something near 1, where honouring the
    /// declaration yields the whole base scale.
    ///
    /// That difference is the defect. On a hull ladder the file genuinely is not
    /// there, so both readings agreed and only the 404 told them apart.
    #[test]
    fn a_declared_identity_tier_is_answered_without_reading_its_sidecar() {
        let base = [15.0, 18.0, 18.0];
        let levels = vec![
            near_level(),
            level(BAKED_TIER, BAKED_TIER_VARIANT, Some(TierRig::Identity)),
        ];
        let got = resolve_tier_parent_scale(&levels, base, None).expect("native reads are sync");
        assert_eq!(
            got,
            Vec3::from_array(base),
            "a declared identity tier owes the parent the whole base scale, and \
             the sidecar sitting beside that .glb must not have been consulted"
        );
    }

    /// A level that declares `baked` still reads the file. The convention says a
    /// sidecar is there; the NUMBER has to come from the file itself, because
    /// what the parent owes depends on what that tier actually carries — not on
    /// what the convention implies it ought to.
    #[test]
    fn a_declared_baked_tier_reads_the_sidecar_that_is_there() {
        let base = baked_tier_scale();
        let levels = vec![
            near_level(),
            level(BAKED_TIER, BAKED_TIER_VARIANT, Some(TierRig::Baked)),
        ];
        let got = resolve_tier_parent_scale(&levels, base, None).expect("native reads are sync");
        assert!(
            (got - Vec3::ONE).length() < 1e-5,
            "a baked tier applies the base scale itself, so the parent owes it \
             nothing — got {got:?}"
        );
    }

    /// A sidecar predating the field — a mod pack's, in practice — still
    /// resolves, by the probe this always used. The fallback is the whole reason
    /// `tier_rig` is optional rather than required.
    #[test]
    fn an_undeclared_tier_still_resolves_by_probing_as_it_always_did() {
        let base = baked_tier_scale();
        let levels = vec![near_level(), level(BAKED_TIER, BAKED_TIER_VARIANT, None)];
        let got = resolve_tier_parent_scale(&levels, base, None).expect("native reads are sync");
        assert!(
            (got - Vec3::ONE).length() < 1e-5,
            "an undeclared tier must reach the same answer the probe always \
             gave — got {got:?}"
        );

        // And the probe's OTHER answer: a tier with no sidecar at all reads as
        // the hull convention, exactly as an absent file always did.
        let missing = vec![
            near_level(),
            level("assets/models/dynasty_cruiser_lod1.glb", None, None),
        ];
        let hull = [1.5, 1.5, 1.5];
        let got = resolve_tier_parent_scale(&missing, hull, None).expect("native reads are sync");
        assert_eq!(got, Vec3::from_array(hull));
    }

    /// `declared_tier_rig` speaks only for the declaration it is given: an
    /// identity tier gets the rig it would have resolved, and everything else
    /// gets `None`, meaning "resolve this the ordinary way".
    #[test]
    fn only_a_declared_identity_tier_short_circuits_the_rig_read() {
        let identity = level(BAKED_TIER, None, Some(TierRig::Identity));
        assert_eq!(
            declared_tier_rig(&identity),
            Some(crate::model_rig::ModelRig::default()),
            "an identity tier resolves the default rig without a read"
        );
        assert_eq!(
            declared_tier_rig(&level(BAKED_TIER, None, Some(TierRig::Baked))),
            None,
            "a baked tier has a sidecar and must go and read it"
        );
        assert_eq!(
            declared_tier_rig(&level(BAKED_TIER, None, None)),
            None,
            "an undeclared tier says nothing, so nothing is short-circuited"
        );
    }

    /// `tier_rig` survives a TOML round trip in both directions, and a level
    /// that omits it parses — which is what every sidecar written before the
    /// field existed does.
    #[test]
    fn tier_rig_round_trips_through_toml_and_is_optional() {
        let declared: LodLevel = toml::from_str(
            r#"
            max_distance = 100.0
            model = "assets/models/x_lod1.glb"
            tier_rig = "identity"
            "#,
        )
        .expect("a declared tier parses");
        assert_eq!(declared.tier_rig, Some(TierRig::Identity));

        let baked: LodLevel = toml::from_str(r#"tier_rig = "baked""#).expect("baked parses");
        assert_eq!(baked.tier_rig, Some(TierRig::Baked));

        let legacy: LodLevel = toml::from_str(
            r#"
            max_distance = 100.0
            model = "assets/models/x_lod1.glb"
            "#,
        )
        .expect("a sidecar predating the field still parses");
        assert_eq!(legacy.tier_rig, None);

        // Round trip: what we write is what we read back.
        let text = toml::to_string(&declared).expect("serialises");
        assert!(
            text.contains(r#"tier_rig = "identity""#),
            "the field serialises in the spelling the pipeline emits, got:\n{text}"
        );
        assert_eq!(
            toml::from_str::<LodLevel>(&text)
                .expect("re-parses")
                .tier_rig,
            Some(TierRig::Identity)
        );

        // `deny_unknown_fields` is on, so a misspelling is loud rather than
        // silently resolving to "undeclared" and reinstating the probe.
        assert!(
            toml::from_str::<LodLevel>(r#"tier_rigs = "identity""#).is_err(),
            "a mistyped key must fail rather than fall back to probing"
        );
        assert!(
            toml::from_str::<LodLevel>(r#"tier_rig = "hull""#).is_err(),
            "an unknown convention must fail rather than be guessed at"
        );
    }

    /// The anti-staleness gate. `tier_rig` is a claim about a file on disk, and
    /// a claim that can drift from what it describes is worse than no claim: the
    /// renderer would skip a sidecar that IS there (a tier drawn at the wrong
    /// size) or fetch one that is not (the 404 this field removes).
    ///
    /// So: every generated tier of every shipped ladder declares the field, and
    /// what it declares is what `assets/models` actually holds. The pipeline
    /// emits it (`scripts/viewer-lods.mjs` `LEVEL_KEYS`, so every writer that
    /// rewrites a ladder rewrites this too) and this holds the pipeline to it.
    #[test]
    fn every_shipped_ladder_declares_the_tier_rig_its_files_actually_have() {
        let mut identity = 0usize;
        let mut baked = 0usize;

        let dir = std::fs::read_dir("assets/models").expect("assets/models must be readable");
        let mut sidecars: Vec<std::path::PathBuf> = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        sidecars.sort();

        for path in sidecars {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).expect("read sidecar");
            let Ok(rig) = crate::model_rig::ModelRig::from_toml(&text) else {
                continue;
            };
            if rig.lod.is_empty() {
                continue;
            }
            let own_variant = crate::model_rig::sidecar_variant(&name);

            for level in rig.lod.iter().skip(1) {
                let Some(model) = level.model.as_deref() else {
                    continue;
                };
                let sidecar =
                    crate::model_rig::sidecar_path(model, level.variant.as_deref().or(own_variant));
                let on_disk = std::path::Path::new(&sidecar).exists();
                let want = if on_disk {
                    TierRig::Baked
                } else {
                    TierRig::Identity
                };
                assert_eq!(
                    level.tier_rig,
                    Some(want),
                    "{name}: tier {model} declares {:?}, but {sidecar} {} — a shipped \
                     ladder must say what its files actually are. Re-run the pipeline \
                     that wrote this ladder (scripts/author-ladders.mjs for a hull, \
                     scripts/import-asteroids.mjs for a rock).",
                    level.tier_rig,
                    if on_disk { "exists" } else { "does not exist" }
                );
                if on_disk {
                    baked += 1;
                } else {
                    identity += 1;
                }
            }
        }

        assert_eq!(
            identity, 20,
            "expected the 10 shipped hull ladders' two generated tiers each to ship \
             no sidecar of their own"
        );
        assert_eq!(
            baked, 64,
            "expected the 32 shipped asteroid variant ladders' two generated tiers \
             each to ship one"
        );
    }
}
