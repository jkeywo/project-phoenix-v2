use super::*;

/// Shape and billboard tiers are presentation replacements only. Crossing
/// either boundary must preserve the primary rig's authoritative weapon
/// geometry on the parent (issue #1291).
#[test]
fn non_glb_lod_tiers_cannot_remove_authoritative_markers() {
    fn level(toml: &str) -> crate::entities::config::LodLevel {
        toml::from_str(toml).expect("fixture LOD parses")
    }

    fn lods(level: crate::entities::config::LodLevel) -> MeshLods {
        MeshLods {
            levels: vec![level],
            base: crate::entities::config::MeshConfig {
                model: Some("assets/models/primary.glb".into()),
                variant: None,
                shape: crate::entities::config::MeshShape::Sphere,
                colour: vec![0.5, 0.5, 0.5],
                radius: 1.0,
                size: None,
                minor_radius: 0.0,
                emissive: None,
                scale: 1.0,
                rotation: [0.0, 0.0, 0.0],
            },
            base_rig: crate::entities::model_rig::ModelRig::default(),
            base_scale: [1.0, 1.0, 1.0],
            tier_scale: Some(Vec3::ONE),
            current: None,
            scene_child: None,
            is_local_ship: false,
        }
    }

    let rig = crate::entities::model_rig::ModelRig::from_toml(
        r#"
            [markers.weapon]
            position = [1.0, 2.0, 3.0]
            direction = [0.0, 0.0, -1.0]
            "#,
    )
    .unwrap();
    let markers = crate::entities::model_rig::ModelMarkers::from_rig(&rig);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<bevy::scene::Scene>()
        .init_resource::<ProceduralMeshCache>()
        .add_systems(Update, update_mesh_lod);
    app.world_mut().spawn((
        crate::render_setup::GameCamera,
        GlobalTransform::from_translation(Vec3::ZERO),
    ));
    let shape = app
        .world_mut()
        .spawn((
            Transform::from_translation(Vec3::new(0.0, 0.0, 10.0)),
            lods(level("shape = \"sphere\"")),
            markers.clone(),
        ))
        .id();
    let billboard = app
        .world_mut()
        .spawn((
            Transform::from_translation(Vec3::new(0.0, 0.0, 20.0)),
            lods(level(
                "billboard = \"assets/models/fixture.png\"\nscale = [2.0, 1.0, 1.0]",
            )),
            markers,
        ))
        .id();

    app.update();

    for entity in [shape, billboard] {
        assert_eq!(
            app.world()
                .get::<crate::entities::model_rig::ModelMarkers>(entity)
                .and_then(|markers| markers.get("weapon"))
                .map(|marker| marker.position),
            Some([1.0, 2.0, 3.0]),
            "changing visual tiers must leave canonical primary-rig markers intact"
        );
    }
}

/// The renderer may choose a non-unit tier scale, but that presentation
/// choice must not change any simulation-owned transform or marker result.
/// A rendererless GM therefore resolves exactly the same weapon geometry as
/// a rendered peer before and after a real LOD transition (issue #1291).
#[test]
fn non_unit_lod_transition_matches_rendererless_authority() {
    fn level(toml: &str) -> crate::entities::config::LodLevel {
        toml::from_str(toml).expect("fixture LOD parses")
    }

    fn resolved_geometry(world: &World, entity: Entity) -> (Vec3, Vec3, Vec3) {
        let transform = world
            .get::<Transform>(entity)
            .expect("authoritative transform remains present");
        let markers = world
            .get::<crate::entities::model_rig::ModelMarkers>(entity)
            .expect("authoritative markers remain present");
        (
            markers
                .resolve_world_position(transform, "weapon")
                .expect("weapon position resolves"),
            markers
                .resolve_world_direction(transform, "weapon")
                .expect("weapon direction resolves"),
            markers
                .resolve_target_point_world_position(transform, 0)
                .expect("target point resolves"),
        )
    }

    let rig = crate::entities::model_rig::ModelRig::from_toml(
        r#"
            [base]
            offset = [0.25, -0.5, 1.0]
            rotation = [0.1, 0.2, -0.3]
            scale = [2.0, 3.0, 4.0]

            [markers.weapon]
            position = [1.0, 2.0, 3.0]
            direction = [0.25, 0.5, -1.0]

            [[target_points]]
            position = [-0.5, 0.25, 1.5]
            "#,
    )
    .expect("fixture rig parses");
    let markers = crate::entities::model_rig::ModelMarkers::from_rig(&rig);
    let canonical = Transform {
        translation: Vec3::new(11.0, -4.0, 20.0),
        rotation: Quat::from_euler(EulerRot::XYZ, -0.2, 0.4, 0.15),
        scale: Vec3::splat(1.75),
    };

    let mut rendererless = World::new();
    let rendererless_entity = rendererless.spawn((canonical, markers.clone())).id();
    let rendererless_geometry = resolved_geometry(&rendererless, rendererless_entity);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<bevy::scene::Scene>()
        .init_resource::<ProceduralMeshCache>()
        .add_systems(Update, update_mesh_lod);
    let camera = app
        .world_mut()
        .spawn((
            crate::render_setup::GameCamera,
            GlobalTransform::from_translation(canonical.translation),
        ))
        .id();
    let rendered_entity = app
        .world_mut()
        .spawn((
            canonical,
            MeshLods {
                levels: vec![
                    level("max_distance = 10.0\nshape = \"sphere\"\nscale = [1.0, 1.0, 1.0]"),
                    level("shape = \"cuboid\"\nscale = [0.5, 2.0, 1.0]"),
                ],
                base: crate::entities::config::MeshConfig {
                    model: Some("assets/models/primary.glb".into()),
                    variant: None,
                    shape: crate::entities::config::MeshShape::Sphere,
                    colour: vec![0.5, 0.5, 0.5],
                    radius: 1.0,
                    size: None,
                    minor_radius: 0.0,
                    emissive: None,
                    scale: 1.75,
                    rotation: [-0.2, 0.4, 0.15],
                },
                base_rig: rig,
                base_scale: [2.0, 3.0, 4.0],
                tier_scale: Some(Vec3::new(2.0, 3.0, 4.0)),
                current: None,
                scene_child: None,
                is_local_ship: false,
            },
            markers,
        ))
        .id();

    app.update();
    assert_eq!(
        *app.world().get::<Transform>(rendered_entity).unwrap(),
        canonical,
        "selecting the near tier cannot mutate the simulation transform"
    );
    assert_eq!(
        resolved_geometry(app.world(), rendered_entity),
        rendererless_geometry
    );
    let near_root = app
        .world()
        .get::<MeshLods>(rendered_entity)
        .and_then(|lods| lods.scene_child)
        .expect("near visual root is recorded");
    assert_eq!(
        app.world().get::<Transform>(near_root).unwrap().scale,
        Vec3::ONE,
        "the primary GLB convention folds no extra compensation into tier zero"
    );

    app.world_mut()
        .entity_mut(camera)
        .insert(GlobalTransform::from_translation(
            canonical.translation + Vec3::X * 100.0,
        ));
    app.update();

    assert_eq!(
        *app.world().get::<Transform>(rendered_entity).unwrap(),
        canonical,
        "selecting a non-unit far tier cannot mutate simulation state"
    );
    assert_eq!(
        resolved_geometry(app.world(), rendered_entity),
        rendererless_geometry,
        "renderer and rendererless peers must resolve identical geometry"
    );
    let far_root = app
        .world()
        .get::<MeshLods>(rendered_entity)
        .and_then(|lods| lods.scene_child)
        .expect("far visual root is recorded");
    assert_eq!(
        app.world().get::<Transform>(far_root).unwrap().scale,
        Vec3::new(1.0, 6.0, 4.0),
        "the non-unit far scale still applies to presentation"
    );
}

// ── LOD tier retirement (PRD #1023, module 5) ────────────────────────

mod lod_cross_fade {
    use super::*;
    use crate::entities::visual_fade::{FadeDirection, VisualFade};

    /// The outgoing tier's own local scale before retirement. Any non-unit
    /// value does; what the test reads is what happened to it.
    const OUTGOING_CHILD_SCALE: f32 = 2.0;

    /// The flat `[mesh]` a `MeshLods` falls back to. Retirement reads none
    /// of it, so the values only have to be legal.
    fn bare_mesh_config() -> crate::entities::config::MeshConfig {
        crate::entities::config::MeshConfig {
            model: None,
            variant: None,
            shape: crate::entities::config::MeshShape::Sphere,
            colour: vec![0.5, 0.5, 0.5],
            radius: 1.0,
            size: None,
            minor_radius: 0.0,
            emissive: None,
            scale: 1.0,
            rotation: [0.0, 0.0, 0.0],
        }
    }

    /// Retire one LOD visual through a real `Commands`, and hand back the
    /// world plus the child that was retired.
    fn retire(fade_secs: f32) -> (World, Entity) {
        let mut world = World::new();
        let entity = world.spawn(Transform::default()).id();
        let child = world
            .spawn(Transform::from_scale(Vec3::splat(OUTGOING_CHILD_SCALE)))
            .id();
        world.entity_mut(entity).add_child(child);

        let mut lods = MeshLods {
            levels: Vec::new(),
            base: bare_mesh_config(),
            base_rig: crate::entities::model_rig::ModelRig::default(),
            base_scale: [1.0, 1.0, 1.0],
            tier_scale: None,
            current: Some(0),
            scene_child: Some(child),
            is_local_ship: false,
        };
        {
            let mut commands = world.commands();
            retire_lod_visual(&mut commands, &mut lods, fade_secs);
        }
        world.flush();
        assert_eq!(
            lods.scene_child, None,
            "a retired visual is no longer the LOD's own child either way"
        );
        (world, child)
    }

    /// No authored window is the same-frame cut this always was — the
    /// behaviour every LOD test written before the cross-fade assumes.
    #[test]
    fn a_zero_window_despawns_the_outgoing_tier_immediately() {
        let (world, child) = retire(0.0);
        assert!(
            world.get_entity(child).is_err(),
            "with no window the outgoing tier goes this frame"
        );
    }

    /// With a window, the outgoing tier stays on screen and is handed to
    /// the fade driver, which owns its despawn.
    #[test]
    fn a_window_keeps_the_outgoing_tier_and_fades_it() {
        let (world, child) = retire(0.25);
        let fade = world
            .get::<VisualFade>(child)
            .expect("the outgoing tier is handed to the fade driver");
        assert_eq!(fade.direction, FadeDirection::Out);
        assert_eq!(fade.duration, 0.25);
        assert_eq!(fade.alpha(), 1.0, "the fade starts from fully visible");
    }

    /// Each tier now owns its presentation scale, so retirement must leave
    /// the outgoing root unchanged while the fade driver owns its lifetime.
    #[test]
    fn retirement_leaves_the_outgoing_visual_scale_alone() {
        let (world, child) = retire(0.25);
        assert_eq!(
            world.get::<Transform>(child).unwrap().scale,
            Vec3::splat(OUTGOING_CHILD_SCALE)
        );
    }

    /// Nothing to retire is not an error: the first tier an entity ever
    /// shows has no predecessor, and the swap path must be able to say so
    /// without spawning a fade over an entity that does not exist.
    #[test]
    fn retiring_nothing_does_nothing() {
        let mut world = World::new();
        let mut lods = MeshLods {
            levels: Vec::new(),
            base: bare_mesh_config(),
            base_rig: crate::entities::model_rig::ModelRig::default(),
            base_scale: [1.0, 1.0, 1.0],
            tier_scale: None,
            current: None,
            scene_child: None,
            is_local_ship: false,
        };
        {
            let mut commands = world.commands();
            retire_lod_visual(&mut commands, &mut lods, 0.25);
        }
        world.flush();
        assert_eq!(lods.scene_child, None);
    }
}
