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
    let text = load_sidecar_toml_text(path);
    // Issue #935: a rig sidecar is authored content too — record it into the
    // ledger the same way the world/entity loaders do.
    if let Some(text) = &text {
        crate::content_ledger::record(path, text);
    }
    text
}

fn load_sidecar_toml_text(path: &str) -> Option<String> {
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

/// Resolve [`tier_parent_scale`] for a ladder by reading the sidecar of its
/// first GENERATED tier — the first level past the near one that carries its own
/// GLB. That one tier settles the convention for the whole ladder, because a
/// ladder's tiers are generated together, by one pipeline, from one source
/// model. (`every_shipped_ladder_holds_one_world_size_across_its_tiers` holds
/// that claim to the shipped assets: no ladder mixes the two conventions.)
///
/// `entity_variant` is the `[mesh] variant` fallback a level uses when it
/// declares none of its own — the same fallback the GLB spawn path applies.
///
/// Returns `None` only on wasm, while that sidecar's fetch is still in flight;
/// the caller retries next frame, the same wait the GLB spawn path already
/// takes. On native the read is synchronous and this always resolves.
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
            .map(|m| (m, level.variant.as_deref()))
    });
    let Some((model_path, level_variant)) = generated else {
        // A ladder with no generated GLB tier (a near GLB straight to a
        // billboard) has nothing to measure against, so keep the hull reading.
        return Some(Vec3::from_array(base_scale));
    };
    let rig = resolve_sidecar_rig(model_path, level_variant.or(entity_variant))?;
    Some(tier_parent_scale(base_scale, rig.base.scale))
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

    /// The regression itself, measured against the SHIPPED sidecars rather than
    /// numbers copied into a test: for every ladder in `assets/models`, every
    /// generated tier must compose — parent scale times whatever that tier's own
    /// sidecar applies to the child — to exactly the primary `[base].scale`.
    /// That is the invariant "the model is the same size at every view
    /// distance", which is what John was actually looking at.
    ///
    /// Before the fix, all 32 asteroid ladders composed to their base scale
    /// SQUARED. It also holds the claim `resolve_tier_parent_scale` rests on:
    /// no ladder mixes the two conventions across its own tiers, so probing the
    /// first generated tier settles the rest.
    #[test]
    fn every_shipped_ladder_holds_one_world_size_across_its_tiers() {
        let mut hull = 0usize;
        let mut pipeline = 0usize;
        let mut ambiguous = 0usize;

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
            // Only the ladder-bearing sidecars: a generated tier's own sidecar
            // carries a `[base]` rig but no `[[lod]]` chain of its own.
            if rig.lod.is_empty() {
                continue;
            }
            let variant = crate::model_rig::sidecar_variant(&name);
            let parent = resolve_tier_parent_scale(&rig.lod, rig.base.scale, variant)
                .expect("native sidecar reads are synchronous");

            let mut generated_tiers = 0usize;
            for level in rig.lod.iter().skip(1) {
                let Some(model) = level.model.as_deref() else {
                    continue;
                };
                generated_tiers += 1;
                let tier = resolve_sidecar_rig(model, level.variant.as_deref().or(variant))
                    .expect("native sidecar reads are synchronous");
                let composed = parent * Vec3::from_array(tier.base.scale);
                let want = Vec3::from_array(rig.base.scale);
                assert!(
                    (composed - want).length() < 1e-3,
                    "{name}: tier {model} composes to {composed:?}, but every tier of \
                     this model must reach the primary [base].scale {want:?} — that \
                     mismatch IS the on-screen size change across an LOD crossing"
                );
            }
            assert!(
                generated_tiers > 0,
                "{name} declares a ladder with no generated GLB tier"
            );

            // Book-keeping, so the assertions above cannot pass vacuously and a
            // future ladder in a THIRD convention is noticed rather than folded
            // silently into one of these two.
            let base = Vec3::from_array(rig.base.scale);
            if (base - Vec3::ONE).length() < 1e-4 {
                // A model authored at world size needs no base scale at all, so
                // the two conventions coincide and neither reading is wrong.
                ambiguous += 1;
            } else if (parent - Vec3::ONE).length() < 1e-4 {
                pipeline += 1;
            } else if (parent - base).length() < 1e-4 {
                hull += 1;
            } else {
                panic!(
                    "{name}: parent tier scale {parent:?} is neither 1 (a pipeline \
                     ladder) nor the base scale {base:?} (a hull ladder) — a ladder \
                     in a convention this renderer has not been taught"
                );
            }
        }

        assert_eq!(
            hull, 8,
            "expected the 8 shipped hull ladders whose base scale is not 1 to need \
             the full base scale on their far tiers"
        );
        assert_eq!(
            pipeline, 32,
            "expected the 32 shipped asteroid ladders to need none of it"
        );
        assert_eq!(
            ambiguous, 2,
            "expected alliance_cruiser and dynasty_destroyer — authored at world \
             size, so [base].scale is 1 and the conventions coincide"
        );
    }

    /// The blind spot in the test above, closed.
    ///
    /// `every_shipped_ladder_holds_one_world_size_across_its_tiers` walks
    /// `levels.iter().skip(1)` and `continue`s on any level with no `model` — so
    /// it inspects a ladder's GLB tiers and NOTHING else. Every shipped ladder
    /// ends in a level with no `model`: a billboard imposter. That tier was
    /// therefore free to be any size at all while the test stayed green, and it
    /// was: `alliance_starbase` shipped a 204-unit-wide imposter over a 34-unit
    /// station, and John watched it blink down to size on approach with the whole
    /// suite passing.
    ///
    /// What holds it now is that a billboard's world size does not depend on the
    /// tier scale AT ALL — the authored number is already world units on both
    /// conventions. So the size a ladder's imposter draws at must come out the
    /// same whichever convention that ladder turns out to be in, and it must
    /// match the size the ladder's own resolved tier scale produces. A
    /// reintroduced fold fails this on every hull ladder at once, rather than
    /// waiting for someone to look at a station from 400 units.
    ///
    /// The magnitudes themselves — is a starbase imposter the same size as the
    /// starbase MESH — are held where the meshes can actually be measured, in
    /// `tests/client/billboard-world-size.test.js`; a `.glb` bounding box is not
    /// something this crate can read.
    #[test]
    fn every_shipped_billboard_tier_is_convention_independent() {
        let mut billboards = 0usize;

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
            let variant = crate::model_rig::sidecar_variant(&name);
            let parent = resolve_tier_parent_scale(&rig.lod, rig.base.scale, variant)
                .expect("native sidecar reads are synchronous");

            for level in rig.lod.iter() {
                if level.billboard.is_none() {
                    continue;
                }
                billboards += 1;
                let authored = level
                    .scale
                    .unwrap_or_else(|| panic!("{name}: a shipped billboard authors its size"));
                let actual = crate::entities::billboard::billboard_quad_size(level.scale, parent);

                // The size this ladder actually draws IS the authored world
                // size — not it scaled by anything.
                assert!(
                    (actual[0] - authored[0]).abs() < 1e-3
                        && (actual[1] - authored[1]).abs() < 1e-3,
                    "{name}: imposter draws {actual:?} from an authored world size of \
                     [{}, {}] — a billboard's recorded extents are world units on both \
                     ladder conventions, so nothing may be folded onto them",
                    authored[0],
                    authored[1]
                );

                // And it is the same answer under the OTHER convention's parent
                // scale, which is what makes the tier safe against a ladder
                // whose convention is misread.
                let other = if (parent - Vec3::ONE).length() < 1e-4 {
                    Vec3::from_array(rig.base.scale)
                } else {
                    Vec3::ONE
                };
                let under_other =
                    crate::entities::billboard::billboard_quad_size(level.scale, other);
                assert_eq!(
                    actual, under_other,
                    "{name}: imposter size moves with the tier scale ({parent:?} vs \
                     {other:?}) — that is exactly the size change an LOD crossing \
                     must not have"
                );
            }
        }

        assert_eq!(
            billboards, 42,
            "expected every one of the 42 shipped ladders to end in a billboard \
             imposter — a ladder that stopped shipping one is a ladder this test \
             silently stopped covering"
        );
    }
}
