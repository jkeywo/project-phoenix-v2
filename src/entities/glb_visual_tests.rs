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

use crate::entities::config::{LodLevel, TierRig};

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
        Some(crate::entities::model_rig::ModelRig::default()),
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
        let Ok(rig) = crate::entities::model_rig::ModelRig::from_toml(&text) else {
            continue;
        };
        if rig.lod.is_empty() {
            continue;
        }
        let own_variant = crate::entities::model_rig::sidecar_variant(&name);

        for level in rig.lod.iter().skip(1) {
            let Some(model) = level.model.as_deref() else {
                continue;
            };
            let sidecar = crate::entities::model_rig::sidecar_path(
                model,
                level.variant.as_deref().or(own_variant),
            );
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
            identity, 44,
            "expected the 11 original ladders plus ten recreated hull/station ladders and \
             the recreated docking variant's two generated tiers each to ship no sidecar of their own"
        );
    assert_eq!(
        baked, 64,
        "expected the 32 shipped asteroid variant ladders' two generated tiers \
             each to ship one"
    );
}

/// Materialising a scene is presentation work. Even when the active tier's
/// rig disagrees with the primary authored rig, `spawn_glb_visual` must not
/// replace the parent's authoritative marker geometry (issue #1291).
#[test]
fn spawning_a_glb_visual_cannot_replace_authoritative_markers() {
    fn materialise_visual(
        mut commands: Commands,
        asset_server: Res<AssetServer>,
        scenes: Res<Assets<bevy::scene::Scene>>,
        parent: Query<(Entity, &PendingSceneHandle)>,
    ) {
        let (entity, pending) = parent.single().expect("one parent");
        let presentation_rig = crate::entities::model_rig::ModelRig::from_toml(
            r#"
                [markers.weapon]
                position = [99.0, 0.0, 0.0]
                direction = [1.0, 0.0, 0.0]
                "#,
        )
        .unwrap();
        assert!(matches!(
            spawn_glb_visual(
                &mut commands,
                &asset_server,
                &scenes,
                entity,
                "assets/models/presentation-only.glb",
                None,
                Some(pending),
                Some(&presentation_rig),
            ),
            GlbSpawnOutcome::Spawned(_)
        ));
    }

    let canonical_rig = crate::entities::model_rig::ModelRig::from_toml(
        r#"
            [markers.weapon]
            position = [1.0, 2.0, 3.0]
            direction = [0.0, 0.0, -1.0]
            "#,
    )
    .unwrap();

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<bevy::scene::Scene>()
        .add_systems(Update, materialise_visual);
    let scene = app
        .world_mut()
        .resource_mut::<Assets<bevy::scene::Scene>>()
        .add(bevy::scene::Scene::new(World::new()));
    let parent = app
        .world_mut()
        .spawn((
            crate::entities::model_rig::ModelMarkers::from_rig(&canonical_rig),
            PendingSceneHandle(scene),
        ))
        .id();

    app.update();

    assert_eq!(
        app.world()
            .get::<crate::entities::model_rig::ModelMarkers>(parent)
            .and_then(|markers| markers.get("weapon"))
            .map(|marker| marker.position),
        Some([1.0, 2.0, 3.0]),
        "the visual tier's marker map must not overwrite the primary rig"
    );
}
