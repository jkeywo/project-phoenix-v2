use super::*;
#[test]
fn streak_tracks_perspective_motion_including_reverse_and_strafe() {
    for position in [Vec3::new(3.0, 2.0, -10.0), Vec3::new(-2.0, -1.0, -15.0)] {
        for velocity in [
            Vec3::Z,
            Vec3::NEG_Z,
            Vec3::X,
            Vec3::Y,
            Vec3::new(2.0, 1.0, 3.0),
        ] {
            let project = |p: Vec3| p.truncate() / -p.z;
            let observed = (project(position + velocity * 0.001) - project(position)).normalize();
            assert!(
                projected_motion(position, velocity)
                    .normalize()
                    .dot(observed)
                    > 0.999
            );
        }
    }
}

fn mote_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<MoteMaterial>()
        .init_resource::<MotePool>()
        .insert_resource(State::new(GamePhase::InProgress))
        .insert_resource(WorldConfig::default())
        .add_systems(Update, update_motes);
    app.world_mut().spawn((GameCamera, Transform::default()));
    app.world_mut()
        .spawn((LocalShip, ShipPhysics::default(), ShipViewMode::default()));
    app
}
fn mote_ids(app: &mut App) -> Vec<Entity> {
    app.world_mut()
        .query_filtered::<Entity, With<NativeMote>>()
        .iter(app.world())
        .collect()
}
#[test]
fn pack_texture_refresh_retires_active_motes_and_rebuilds_unchanged_configuration() {
    let mut app = mote_app();
    app.update();
    let old = mote_ids(&mut app);
    assert!(!old.is_empty());
    let unrelated = app.world_mut().spawn(Transform::default()).id();
    crate::server::pfx::reset_pack_textures(app.world_mut());
    assert!(mote_ids(&mut app).is_empty());
    assert!(app.world().get_entity(unrelated).is_ok());
    assert!(app.world().resource::<MotePool>().config.is_none());
    app.update();
    let new = mote_ids(&mut app);
    assert_eq!(new.len(), old.len());
    assert!(new.iter().all(|entity| !old.contains(entity)));
}
#[test]
fn mote_pool_rebuilds_count_and_stays_hidden_outside_3d_then_retires_on_disable_and_mission_exit() {
    let mut app = mote_app();
    let config: crate::world::config::RenderConfig =
        toml::from_str("[native]\nmote_count = 7\n[web]\nmote_count = 7").unwrap();
    app.world_mut().resource_mut::<WorldConfig>().render = Some(config);
    app.update();
    assert_eq!(mote_ids(&mut app).len(), 7);
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .get_mut::<ShipViewMode>(ship)
        .unwrap()
        .view_mode = ViewMode::Radar;
    app.update();
    assert!(app
        .world_mut()
        .query_filtered::<&Visibility, With<NativeMote>>()
        .iter(app.world())
        .all(|v| *v == Visibility::Hidden));
    let before = mote_ids(&mut app);
    let mut config = app.world_mut().resource_mut::<WorldConfig>();
    let render = config.render.as_mut().unwrap();
    render.native.mote_count = 3;
    render.web.mote_count = 3;
    drop(config);
    app.update();
    let rebuilt = mote_ids(&mut app);
    assert_eq!(rebuilt.len(), 3);
    assert!(rebuilt.iter().all(|id| !before.contains(id)));
    app.world_mut().resource_mut::<WorldConfig>().dust =
        Some(crate::world::config::DustPfxConfig {
            enabled: Some(false),
            ..Default::default()
        });
    app.update();
    assert!(mote_ids(&mut app).is_empty());
    app.world_mut().resource_mut::<WorldConfig>().dust = None;
    app.update();
    assert_eq!(mote_ids(&mut app).len(), 3);
    app.insert_resource(State::new(GamePhase::Lobby));
    app.update();
    assert!(mote_ids(&mut app).is_empty());
}
#[test]
fn preload_discovery_uses_selected_platform_textures_and_master_switch() {
    let mut world = WorldConfig::default();
    world.render = Some(toml::from_str("[native]\nmote_textures = ['native/a.png', 'native/a.png', 'native/b.png']\n[web]\nmote_textures = ['web/a.png', 'web/a.png', 'web/b.png']").unwrap());
    let prefix = if cfg!(target_arch = "wasm32") {
        "web"
    } else {
        "native"
    };
    assert_eq!(
        crate::server::pfx::dust_texture_paths(Some(&world)),
        vec![format!("{prefix}/a.png"), format!("{prefix}/b.png")]
    );
    world.dust = Some(crate::world::config::DustPfxConfig {
        enabled: Some(false),
        ..Default::default()
    });
    assert!(crate::server::pfx::dust_texture_paths(Some(&world)).is_empty());
}
