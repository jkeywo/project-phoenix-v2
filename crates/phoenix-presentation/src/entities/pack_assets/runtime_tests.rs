use super::*;
use crate::entities::config_cache::{push_mod_pack, remove_mod_pack, ActivePack};
use bevy::gltf::{Gltf, GltfMesh, GltfPlugin};
use std::sync::Arc;

const MODEL: &str = "assets/models/__workshop_pack_refresh/triangle.glb";
const BUFFER: &str = "assets/models/__workshop_pack_refresh/vertices.bin";

fn model() -> Arc<[u8]> {
    let mut json = br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"vertices.bin","byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[4,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":4}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"#.to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bytes = b"glTF".to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(((20 + json.len()) as u32).to_le_bytes());
    bytes.extend((json.len() as u32).to_le_bytes());
    bytes.extend(b"JSON");
    bytes.extend(json);
    Arc::from(bytes)
}

fn vertices(width: f32) -> Arc<[u8]> {
    let values: [f32; 9] = [0.0, 0.0, 0.0, width, 0.0, 0.0, 0.0, 1.0, 0.0];
    Arc::from(
        values
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
    )
}

fn app() -> App {
    let mut app = App::new();
    register(&mut app);
    app.add_plugins((
        bevy::app::TaskPoolPlugin::default(),
        bevy::asset::AssetPlugin {
            meta_check: bevy::asset::AssetMetaCheck::Never,
            ..default()
        },
        bevy::scene::ScenePlugin,
    ));
    app.init_resource::<crate::server_app_render::ProceduralMeshCache>()
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .register_type::<MeshMaterial3d<StandardMaterial>>()
        .add_plugins(GltfPlugin::default());
    app.finish();
    app.cleanup();
    app
}

fn loaded_model(app: &mut App) -> Handle<Gltf> {
    let server = app.world().resource::<AssetServer>();
    let handle: Handle<Gltf> = server.load(asset_path(server, MODEL));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.update();
        if app
            .world()
            .resource::<AssetServer>()
            .is_loaded_with_dependencies(handle.id())
        {
            return handle;
        }
        let state = app
            .world()
            .resource::<AssetServer>()
            .load_state(handle.id());
        assert!(
            !matches!(state, bevy::asset::LoadState::Failed(_)),
            "{state:?}"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "model load timed out: {state:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn width(app: &App, model: &Handle<Gltf>) -> f32 {
    let gltf = app.world().resource::<Assets<Gltf>>().get(model).unwrap();
    let primitive = &app
        .world()
        .resource::<Assets<GltfMesh>>()
        .get(&gltf.meshes[0])
        .unwrap()
        .primitives[0];
    let mesh = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(&primitive.mesh)
        .unwrap();
    let bevy::mesh::VertexAttributeValues::Float32x3(positions) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        panic!("positions")
    };
    positions[1][0]
}

struct Cleanup(&'static str);
impl Drop for Cleanup {
    fn drop(&mut self) {
        remove_mod_pack(self.0);
    }
}

#[test]
fn replacing_only_an_external_buffer_loads_new_mesh_bytes_under_an_unchanged_glb() {
    let _lock = crate::entities::config_cache::overlay_test_guard();
    let _cleanup = Cleanup("pack-refresh-buffer");
    let pack = |width| ActivePack {
        id: "pack-refresh-buffer".into(),
        assets: [(MODEL.into(), model()), (BUFFER.into(), vertices(width))].into(),
        ..default()
    };
    push_mod_pack(pack(1.0));
    let mut app = app();
    let old = loaded_model(&mut app);
    assert_eq!(width(&app, &old), 1.0);
    remove_mod_pack("pack-refresh-buffer");
    push_mod_pack(pack(4.0));
    let new = loaded_model(&mut app);
    assert_ne!(old.id(), new.id());
    assert_eq!(width(&app, &new), 4.0);
    assert_eq!(
        width(&app, &old),
        1.0,
        "held old handles are immutable, never rewritten as current"
    );
}

#[test]
fn removing_a_pack_only_model_retires_its_visual_and_refuses_new_and_old_requests() {
    let _lock = crate::entities::config_cache::overlay_test_guard();
    let _cleanup = Cleanup("pack-refresh-remove");
    push_mod_pack(ActivePack {
        id: "pack-refresh-remove".into(),
        assets: [(MODEL.into(), model()), (BUFFER.into(), vertices(1.0))].into(),
        ..default()
    });
    let mut app = app();
    let model = loaded_model(&mut app);
    let scene = app
        .world()
        .resource::<Assets<Gltf>>()
        .get(&model)
        .unwrap()
        .scenes[0]
        .clone();
    let owner = app
        .world_mut()
        .spawn((
            crate::entities::spawner::MeshSection(
                toml::from_str(&format!(
                    "shape = 'sphere'\ncolour = [1.0, 1.0, 1.0]\nmodel = {MODEL:?}"
                ))
                .unwrap(),
            ),
            Transform::from_xyz(7.0, 8.0, 9.0),
        ))
        .id();
    let visual = app
        .world_mut()
        .spawn((
            PackVisualRoot,
            bevy::scene::SceneRoot(scene),
            Transform::default(),
            ChildOf(owner),
        ))
        .id();
    let pending = app
        .world()
        .resource::<AssetServer>()
        .get_path(model.id())
        .unwrap()
        .clone_owned();
    // The native lobby changes the accepted stack during Update. Invalidation
    // must complete in this same frame, after any queued visual attachments.
    app.add_systems(Update, |mut removed: Local<bool>| {
        if !*removed {
            remove_mod_pack("pack-refresh-remove");
            *removed = true;
        }
    });
    app.update();
    assert!(
        app.world().get_entity(visual).is_err(),
        "the old visual cannot survive a missing replacement"
    );
    assert_eq!(
        app.world().get::<Transform>(owner).unwrap().translation,
        Vec3::new(7.0, 8.0, 9.0)
    );
    {
        let reader = app
            .world()
            .resource::<AssetServer>()
            .get_source(versioned::SOURCE)
            .unwrap()
            .reader();
        assert!(matches!(
            bevy::tasks::block_on(reader.read(pending.path())),
            Err(bevy::asset::io::AssetReaderError::NotFound(_))
        ));
    }
    let fresh: Handle<Gltf> = app
        .world()
        .resource::<AssetServer>()
        .load(asset_path(app.world().resource::<AssetServer>(), MODEL));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.update();
        if matches!(
            app.world().resource::<AssetServer>().load_state(fresh.id()),
            bevy::asset::LoadState::Failed(_)
        ) {
            break;
        }
        assert!(app.world().resource::<Assets<Gltf>>().get(&fresh).is_none());
        assert!(
            std::time::Instant::now() < deadline,
            "missing replacement did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
