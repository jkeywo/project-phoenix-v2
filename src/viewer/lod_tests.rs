use super::*;
use crate::entities::config::MeshShape;

/// Drive `refresh_ladder` then `apply_lod_mode` over the real shipped
/// sidecars, the way the viewer's schedule does.
///
/// Native, so `resolve_sidecar_rig` reads the files off disk in one call
/// rather than through the wasm fetch queue — which is what makes the
/// engine's own view of a ladder assertable at all.
fn run_schedule(model: &str, variant: Option<&str>, mode: LodMode) -> (usize, Showing) {
    run_schedule_at(model, variant, mode, 30.0)
}

/// As above, with the camera placed at a chosen distance from the subject.
fn run_schedule_at(
    model: &str,
    variant: Option<&str>,
    mode: LodMode,
    distance: f32,
) -> (usize, Showing) {
    let mut app = App::new();
    app.insert_resource(ViewerArgs {
        model: Some(model.to_string()),
        variant: variant.map(str::to_string),
        entity: None,
        gizmos: false,
    })
    .insert_resource(mode)
    .init_resource::<LadderState>()
    .init_resource::<SubjectState>()
    .add_systems(Update, (refresh_ladder, apply_lod_mode).chain());
    app.world_mut()
        .spawn((ViewerCamera, Transform::from_xyz(0.0, 0.0, distance)));
    app.update();

    let levels = app.world().resource::<LadderState>().levels.len();
    let showing = app.world().resource::<SubjectState>().showing.clone();
    (levels, showing)
}

/// The bug that made the panel's "view" button do nothing: the asteroids
/// have no `<stem>.model.toml`, so the base variant resolves to an identity
/// rig with no ladder — while the panel, which read whichever sidecar
/// happened to carry one, showed four levels to click on.
#[test]
fn the_engine_reads_the_ladder_of_the_variant_it_was_asked_for() {
    let (base, _) = run_schedule("assets/models/asteroid_common_1.glb", None, LodMode::Base);
    assert_eq!(base, 0, "the asteroids ship no base-variant sidecar");

    let (large, _) = run_schedule(
        "assets/models/asteroid_common_1.glb",
        Some("large"),
        LodMode::Base,
    );
    assert_eq!(large, 4, "the large variant carries the four-level ladder");
}

#[test]
fn fixed_mode_puts_that_level_on_screen() {
    let (_, showing) = run_schedule(
        "assets/models/asteroid_common_1.glb",
        Some("large"),
        LodMode::Fixed(2),
    );
    assert_eq!(
        showing,
        Showing::Glb {
            path: "assets/models/asteroid_common_1_lod2.glb".into(),
            variant: Some("large".into()),
            scale: Vec3::ONE,
        }
    );
}

/// The far level of every shipped asteroid ladder is a BILLBOARD, and the
/// viewer now previews it as the game draws it.
///
/// This test has been wrong twice. It first asserted that last level was a
/// bare `shape = "sphere"`, which is what those ladders ended in before the
/// imposter atlases landed; it went stale rather than red, because CI runs
/// `cargo test --features headless` and this module is behind `--features
/// viewer`, so nothing in the pipeline runs these tests. It was then
/// retargeted at the GAP — `showing_for` returning `Showing::Base` for a
/// level it could not build, so the panel's "fixed 3" showed the full-detail
/// model rather than the imposter the game draws at that range. This is the
/// gap closed (PRD #1023, module 5): the atlas, the ring, and the quad's
/// world size, all from the same functions `update_mesh_lod` uses.
#[test]
fn the_far_billboard_level_previews_as_the_games_imposter() {
    let model = "assets/models/asteroid_common_1.glb";
    let (levels, showing) = run_schedule(model, Some("large"), LodMode::Fixed(3));
    assert_eq!(levels, 4, "the large variant carries the four-level ladder");
    let Showing::Billboard(billboard) = showing else {
        panic!("the far level of a shipped rock ladder is a billboard, got {showing:?}");
    };
    assert!(
        billboard.atlas.ends_with(".png"),
        "a billboard level previews from its captured atlas, got {}",
        billboard.atlas
    );
    assert_eq!(
        billboard.views, 8,
        "every shipped atlas packs an eight-view yaw ring"
    );
    assert!(
        billboard.size[0] > 0.0 && billboard.size[1] > 0.0,
        "the imposter quad must have a size, got {:?}",
        billboard.size
    );
}

/// The preview is the size the GAME draws, on both ladder conventions —
/// which is the whole reason the size rule lives in the billboard module
/// rather than being restated here. Both conventions record a billboard's
/// extents in WORLD units (the capture tool renders a hull under its own
/// `[base]` rig, and the capture script scales a rock's quad by its
/// variant's), so on both an imposter is exactly its authored `scale` and
/// neither takes the model's `[base].scale` a second time.
#[test]
fn a_billboard_preview_is_the_size_the_game_draws() {
    let rock = "assets/models/asteroid_common_1.glb";
    let (_, showing) = run_schedule(rock, Some("large"), LodMode::Fixed(3));
    let Showing::Billboard(imposter) = showing else {
        panic!("the rock's far level is a billboard");
    };
    let levels = {
        let rig = resolve_sidecar_rig(rock, Some("large")).expect("native reads are sync");
        rig.lod
    };
    let authored = levels[3]
        .scale
        .expect("a shipped billboard authors its size");
    assert!(
        (imposter.size[0] - authored[0]).abs() < 1e-4
            && (imposter.size[1] - authored[1]).abs() < 1e-4,
        "a pipeline ladder's imposter is its authored size, got {:?} for {authored:?}",
        imposter.size
    );

    let hull = "assets/models/alliance_destroyer.glb";
    let (hull_levels, hull_showing) = {
        let rig = resolve_sidecar_rig(hull, None).expect("native reads are sync");
        let last = rig.lod.len() - 1;
        (
            rig.lod.clone(),
            run_schedule(hull, None, LodMode::Fixed(last)).1,
        )
    };
    let Showing::Billboard(hull_imposter) = hull_showing else {
        panic!("the destroyer's far level is a billboard");
    };
    let hull_authored = hull_levels
        .last()
        .and_then(|l| l.scale)
        .expect("the destroyer's billboard authors its size");
    // The destroyer's `[base].scale` is 0.75, and it must NOT appear here:
    // its atlas was measured with that rig already applied, so the authored
    // number is the world size and multiplying by it again would draw the
    // hull three-quarters size at range — the same fault the starbase showed
    // at 6x.
    assert!(
        (hull_imposter.size[0] - hull_authored[0]).abs() < 1e-3,
        "a hull imposter is its authored world size, got {:?} for {hull_authored:?}",
        hull_imposter.size
    );
}

/// Auto mode picks the band the camera is standing in.
///
/// The distance is derived from the ladder's own first bound rather than
/// written here: these are switch distances a designer retunes, and a test
/// that pins one goes red the day someone does their job. What must hold is
/// the relationship — inside band 0, you get level 0.
#[test]
fn auto_mode_shows_the_level_whose_band_the_camera_is_in() {
    let model = "assets/models/asteroid_common_1.glb";
    let mut app = App::new();
    app.insert_resource(ViewerArgs {
        model: Some(model.to_string()),
        variant: Some("large".to_string()),
        entity: None,
        gizmos: false,
    })
    .insert_resource(LodMode::Base)
    .init_resource::<LadderState>()
    .init_resource::<SubjectState>()
    .add_systems(Update, refresh_ladder);
    app.update();
    let levels = app.world().resource::<LadderState>().levels.clone();
    let first_bound = levels[0]
        .max_distance
        .expect("the near level is a bounded band");

    let (_, near) = run_schedule_at(model, Some("large"), LodMode::Auto, first_bound * 0.5);
    assert_eq!(
        near,
        Showing::Glb {
            path: model.into(),
            variant: Some("large".into()),
            scale: Vec3::ONE,
        },
        "half of {first_bound} is inside the first band",
    );
}

fn ladder(levels: Vec<LodLevel>) -> LadderState {
    LadderState {
        levels,
        ..Default::default()
    }
}

fn glb(max_distance: Option<f32>, model: &str) -> LodLevel {
    LodLevel {
        max_distance,
        model: Some(model.to_string()),
        ..Default::default()
    }
}

#[test]
fn base_mode_shows_no_level_even_with_a_ladder() {
    let state = ladder(vec![glb(Some(50.0), "a.glb"), glb(None, "b.glb")]);
    assert_eq!(state.desired(LodMode::Base, 10.0), None);
}

#[test]
fn auto_mode_picks_the_band_the_distance_falls_in() {
    let state = ladder(vec![glb(Some(50.0), "a.glb"), glb(None, "b.glb")]);
    assert_eq!(state.desired(LodMode::Auto, 10.0), Some(0));
    assert_eq!(state.desired(LodMode::Auto, 500.0), Some(1));
}

#[test]
fn fixed_mode_holds_its_level_at_any_distance() {
    let state = ladder(vec![glb(Some(50.0), "a.glb"), glb(None, "b.glb")]);
    assert_eq!(state.desired(LodMode::Fixed(1), 1.0), Some(1));
    assert_eq!(state.desired(LodMode::Fixed(1), 9000.0), Some(1));
}

/// A mode held across a model switch can name a level the new model has
/// not got.
#[test]
fn a_fixed_level_past_the_end_falls_back_to_the_base_model() {
    let state = ladder(vec![glb(None, "a.glb")]);
    assert_eq!(state.desired(LodMode::Fixed(3), 10.0), None);
}

#[test]
fn a_model_with_no_ladder_has_nothing_to_show_in_any_mode() {
    let state = ladder(vec![]);
    assert_eq!(state.desired(LodMode::Auto, 10.0), None);
    assert_eq!(state.desired(LodMode::Fixed(0), 10.0), None);
}

#[test]
fn mode_names_round_trip() {
    for (name, mode) in [
        ("base", LodMode::Base),
        ("auto", LodMode::Auto),
        ("fixed", LodMode::Fixed(2)),
    ] {
        assert_eq!(LodMode::parse(name, 2), mode);
        assert_eq!(mode.name(), name);
    }
}

#[test]
fn an_unknown_mode_name_is_the_base_model() {
    assert_eq!(LodMode::parse("nonsense", 0), LodMode::Base);
}

/// The shipped asteroid ladders end in a bare `shape = "sphere"` with no
/// radius: it stands in for the whole rock, so it is sized from the model.
#[test]
fn a_procedural_level_with_no_radius_is_sized_from_the_model() {
    let mut state = ladder(vec![LodLevel {
        shape: Some(MeshShape::Sphere),
        ..Default::default()
    }]);
    state.extent = Some(8.0);
    let Showing::Shape(level) = showing_for(0, &state.levels[0], &ViewerArgs::default(), &state)
    else {
        panic!("a shape level renders as a shape");
    };
    assert_eq!(level.radius, 4.0);
}

#[test]
fn a_glb_level_inherits_the_panel_variant_when_it_names_none() {
    let state = ladder(vec![glb(None, "assets/models/x_lod1.glb")]);
    let args = ViewerArgs {
        variant: Some("large".into()),
        ..Default::default()
    };
    assert_eq!(
        showing_for(0, &state.levels[0], &args, &state),
        Showing::Glb {
            path: "assets/models/x_lod1.glb".into(),
            variant: Some("large".into()),
            scale: Vec3::ONE,
        }
    );
}

/// A sphere standing in for a hull wants to be hull-shaped, which is the
/// whole point of a per-level `scale`.
#[test]
fn a_shape_level_carries_its_own_xyz_scale() {
    let state = ladder(vec![LodLevel {
        shape: Some(MeshShape::Sphere),
        radius: Some(2.0),
        scale: Some([3.0, 1.0, 0.5]),
        ..Default::default()
    }]);
    let Showing::Shape(level) = showing_for(0, &state.levels[0], &ViewerArgs::default(), &state)
    else {
        panic!("a shape level renders as a shape");
    };
    assert_eq!(level.scale, Vec3::new(3.0, 1.0, 0.5));
}

/// A shape level's rotation reaches the visual — the game puts it on the
/// mesh child rather than the entity, whose rotation the sim owns.
#[test]
fn a_shape_level_carries_its_own_rotation() {
    let state = ladder(vec![LodLevel {
        shape: Some(MeshShape::Sphere),
        rotation: Some([0.0, std::f32::consts::FRAC_PI_2, 0.0]),
        ..Default::default()
    }]);
    let Showing::Shape(level) = showing_for(0, &state.levels[0], &ViewerArgs::default(), &state)
    else {
        panic!("a shape level renders as a shape");
    };
    let expected = Quat::from_euler(EulerRot::XYZ, 0.0, std::f32::consts::FRAC_PI_2, 0.0);
    assert!(level.rotation.abs_diff_eq(expected, 1e-6));
}

#[test]
fn a_shape_level_without_a_rotation_is_unrotated() {
    let state = ladder(vec![LodLevel {
        shape: Some(MeshShape::Sphere),
        ..Default::default()
    }]);
    let Showing::Shape(level) = showing_for(0, &state.levels[0], &ViewerArgs::default(), &state)
    else {
        panic!("a shape level renders as a shape");
    };
    assert_eq!(level.rotation, Quat::IDENTITY);
}

/// The colour a level declares is what it renders; omitting it inherits the
/// entity's, which in the viewer is the neutral stand-in.
#[test]
fn a_shape_level_uses_its_declared_colour() {
    let state = ladder(vec![LodLevel {
        shape: Some(MeshShape::Sphere),
        colour: Some(vec![0.5, 0.25, 0.125]),
        ..Default::default()
    }]);
    let Showing::Shape(level) = showing_for(0, &state.levels[0], &ViewerArgs::default(), &state)
    else {
        panic!("a shape level renders as a shape");
    };
    assert_eq!(level.colour, vec![0.5, 0.25, 0.125]);
}

/// A level that declares no scale renders at the entity's own, which in the
/// viewer is unity — the same "recompute, never accumulate" rule
/// `update_mesh_lod` follows.
#[test]
fn a_level_without_a_scale_renders_unscaled() {
    let state = ladder(vec![glb(None, "assets/models/x.glb")]);
    assert_eq!(
        showing_for(0, &state.levels[0], &ViewerArgs::default(), &state),
        Showing::Glb {
            path: "assets/models/x.glb".into(),
            variant: None,
            scale: Vec3::ONE,
        }
    );
}

/// A level that names its own variant keeps it — that is how a ladder
/// points at a differently-rigged far level.
#[test]
fn a_level_variant_wins_over_the_panel_variant() {
    let mut level = glb(None, "assets/models/x_lod1.glb");
    level.variant = Some("cosmetic".into());
    let state = ladder(vec![level]);
    let args = ViewerArgs {
        variant: Some("large".into()),
        ..Default::default()
    };
    assert_eq!(
        showing_for(0, &state.levels[0], &args, &state),
        Showing::Glb {
            path: "assets/models/x_lod1.glb".into(),
            variant: Some("cosmetic".into()),
            scale: Vec3::ONE,
        }
    );
}

// ── One world size across the ladder, both conventions ───────────────

/// Drive the real schedule over a SHIPPED ladder and compose what the viewer
/// would actually put on screen at `index`: the subject transform's scale
/// (`Showing::Glb.scale`) times whatever that tier's own sidecar applies to
/// the GLB child (`spawn_glb_visual`'s `base_bevy_transform`).
///
/// That product is the model's on-screen size, and it must be the primary
/// sidecar's `[base].scale` at EVERY tier — otherwise the model changes size
/// as it crosses its own LOD bands, which is the artefact the viewer exists
/// to catch and was itself producing.
fn composed_world_scale(model: &str, variant: Option<&str>, index: usize) -> Vec3 {
    let (_, showing) = run_schedule(model, variant, LodMode::Fixed(index));
    let Showing::Glb {
        path,
        variant: tier_variant,
        scale,
    } = showing
    else {
        panic!("level {index} of {model} is a GLB level");
    };
    let tier_rig = resolve_sidecar_rig(&path, tier_variant.as_deref())
        .expect("native sidecar reads are synchronous");
    scale * Vec3::from_array(tier_rig.base.scale)
}

fn primary_base_scale(model: &str, variant: Option<&str>) -> Vec3 {
    Vec3::from_array(
        resolve_sidecar_rig(model, variant)
            .expect("native sidecar reads are synchronous")
            .base
            .scale,
    )
}

/// A HULL ladder: no sidecar beside the generated tier GLBs, so the parent
/// owes each of them the whole `[base].scale` (0.75 for the destroyer).
/// Before the fix the viewer folded in none of it, and the destroyer visibly
/// SHRANK to raw model size the moment it crossed out of its near band.
#[test]
fn a_hull_ladder_is_one_size_across_the_viewers_tiers() {
    let model = "assets/models/alliance_destroyer.glb";
    let want = primary_base_scale(model, None);
    assert_eq!(
        want,
        Vec3::splat(0.75),
        "the destroyer's authored base scale"
    );
    for index in 0..=2 {
        let got = composed_world_scale(model, None, index);
        assert!(
            (got - want).length() < 1e-5,
            "tier {index} composes to {got:?}, but every tier must reach {want:?}"
        );
    }
}

/// A PIPELINE ladder: every tier GLB ships a sidecar carrying the primary
/// `[base]` rig, so the child already applies the base scale and the parent
/// must fold in nothing. Folding it in anyway is what made a rock render at
/// its base scale SQUARED in the game (bf4c4b02, fixed in `update_mesh_lod`);
/// the viewer has to agree, or the tool disagrees with what it is inspecting.
#[test]
fn a_pipeline_ladder_is_one_size_across_the_viewers_tiers() {
    let model = "assets/models/asteroid_common_1.glb";
    let want = primary_base_scale(model, Some("large"));
    for index in 0..=2 {
        let got = composed_world_scale(model, Some("large"), index);
        assert!(
            (got - want).length() < 1e-4,
            "tier {index} composes to {got:?}, but every tier must reach {want:?}"
        );
    }
}

/// The two conventions pull the parent scale in opposite directions, so the
/// test above would pass on a viewer that simply never scaled anything. This
/// pins the halves apart: a hull ladder's far tier carries the base scale on
/// the PARENT, a rock's carries it on the CHILD and the parent stays at 1.
#[test]
fn the_two_conventions_put_the_base_scale_on_opposite_transforms() {
    let (_, hull) = run_schedule(
        "assets/models/alliance_destroyer.glb",
        None,
        LodMode::Fixed(1),
    );
    let Showing::Glb {
        scale: hull_parent, ..
    } = hull
    else {
        panic!("the destroyer's level 1 is a GLB level");
    };
    assert!(
        (hull_parent - Vec3::splat(0.75)).length() < 1e-5,
        "a hull ladder's far tier needs the whole base scale on the parent, got {hull_parent:?}"
    );

    let (_, rock) = run_schedule(
        "assets/models/asteroid_common_1.glb",
        Some("large"),
        LodMode::Fixed(1),
    );
    let Showing::Glb {
        scale: rock_parent, ..
    } = rock
    else {
        panic!("the rock's level 1 is a GLB level");
    };
    assert!(
        (rock_parent - Vec3::ONE).length() < 1e-5,
        "a pipeline ladder's far tier already carries it on the child, so the \
             parent must stay at 1, got {rock_parent:?}"
    );
}
