use super::*;
use bevy::ecs::{system::RunSystemOnce, world::CommandQueue};
use std::time::Duration;

#[test]
fn torpedo_impact_layers_keep_their_appearance_then_fade_and_expire() {
    let mut world = World::new();
    let mut images = Assets::<Image>::default();
    let assets = PhaserPfxAssets {
        beam_glow: default(),
        beam_core: default(),
        radial_glow: images.add(Image::default()),
        impact_ring: images.add(Image::default()),
        spark_streak: images.add(Image::default()),
    };
    let explosion = ShipExplosionPfxAssets {
        puff: images.add(Image::default()),
    };
    let mut meshes = Assets::<Mesh>::default();
    let mut materials = Assets::<StandardMaterial>::default();
    let mut queue = CommandQueue::default();
    let position = Vec3::new(1.0, 2.0, 3.0);
    spawn_torpedo_impact_burst(
        position,
        &assets,
        &explosion,
        &mut Commands::new(&mut queue, &world),
        &mut meshes,
        &mut materials,
    );
    queue.apply(&mut world);
    let mut sprites = world.query::<(Entity, &Transform, &MeshMaterial3d<StandardMaterial>)>();
    let rows: Vec<_> = sprites
        .iter(&world)
        .map(|(entity, transform, material)| (entity, *transform, material.0.clone()))
        .collect();
    assert_eq!(rows.len(), 3 + TORPEDO_IMPACT_SPARK_COUNT);
    assert_eq!(
        rows.iter()
            .map(|(_, _, handle)| handle.id())
            .collect::<HashSet<_>>()
            .len(),
        rows.len()
    );
    let mut flash = None;
    let mut spark = None;
    for (entity, transform, handle) in &rows {
        let material = materials.get(handle).unwrap();
        assert_eq!(material.alpha_mode, AlphaMode::Add);
        assert!(material.unlit && material.double_sided);
        assert!(material.cull_mode.is_none());
        let texture = material.base_color_texture.as_ref().unwrap();
        let start_scale = if texture == &assets.radial_glow {
            flash = Some((*entity, handle.clone()));
            TORPEDO_IMPACT_FLASH_START_SIZE
        } else if texture == &explosion.puff {
            TORPEDO_IMPACT_PLASMA_START_SCALE
        } else if texture == &assets.impact_ring {
            TORPEDO_IMPACT_RING_START_SCALE
        } else {
            assert_eq!(texture, &assets.spark_streak);
            spark = Some(*entity);
            assert!(transform.translation.distance(position) <= TORPEDO_IMPACT_SPARK_SPREAD + 1e-5);
            TORPEDO_IMPACT_SPARK_SCALE
        };
        assert_eq!(transform.scale, Vec3::splat(start_scale));
        if texture != &assets.spark_streak {
            assert_eq!(transform.translation, position);
        }
    }
    let (flash, flash_material) = flash.unwrap();
    world.insert_resource(materials);
    world.insert_resource(Time::<()>::default());
    world
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(
            TORPEDO_IMPACT_FLASH_LIFETIME_SECS / 2.0,
        ));
    world.run_system_once(tick_lifetime_pfx).unwrap();
    world.run_system_once(tick_bursts).unwrap();
    let scale = world.get::<Transform>(flash).unwrap().scale.x;
    assert!(
        (scale - (TORPEDO_IMPACT_FLASH_START_SIZE + TORPEDO_IMPACT_FLASH_END_SIZE) / 2.0).abs()
            < 1e-5
    );
    assert_eq!(
        world.get::<Transform>(spark.unwrap()).unwrap().scale,
        Vec3::splat(TORPEDO_IMPACT_SPARK_SCALE)
    );
    let materials = world.resource::<Assets<StandardMaterial>>();
    let material = materials.get(&flash_material).unwrap();
    assert!((material.base_color.alpha() - TORPEDO_CORE_COLOR[3] / 2.0).abs() < 1e-5);
    assert!(
        (material.emissive.red - TORPEDO_CORE_COLOR[0] * TORPEDO_CORE_EMISSIVE / 2.0).abs() < 1e-5
    );
    world
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs(10));
    world.run_system_once(tick_lifetime_pfx).unwrap();
    assert_eq!(world.query::<&Mesh3d>().iter(&world).count(), 0);
}

#[test]
fn blaster_muzzle_flash_retains_its_untextured_single_sided_material() {
    let mut world = World::new();
    let mut queue = CommandQueue::default();
    let mut meshes = Assets::<Mesh>::default();
    let mut materials = Assets::<StandardMaterial>::default();
    spawn_blaster_muzzle_flash(
        Vec3::ZERO,
        BLASTER_BOLT_COLOR,
        &mut Commands::new(&mut queue, &world),
        &mut meshes,
        &mut materials,
    );
    queue.apply(&mut world);
    let handle = world
        .query::<&MeshMaterial3d<StandardMaterial>>()
        .single(&world)
        .unwrap();
    let material = materials.get(&handle.0).unwrap();
    assert!(material.base_color_texture.is_none());
    assert!(!material.double_sided);
    assert_eq!(material.cull_mode, StandardMaterial::default().cull_mode);
    assert_eq!(material.alpha_mode, AlphaMode::Add);
}
