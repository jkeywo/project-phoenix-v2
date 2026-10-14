use super::*;

#[test]
fn the_payload_reports_the_level_it_is_showing() {
    let stats = SubjectStats {
        triangles: 138_790,
        meshes: 12,
        textures: 3,
        measured_textures: 3,
        texture_pixels: 786_432,
        largest_texture: 512,
    };
    let ladder = LadderState {
        distance: 87.5,
        current: Some(1),
        levels: vec![Default::default(), Default::default()],
        ..Default::default()
    };
    let json = render_stats(&stats, &ladder, LodMode::Auto, true, 8.0, None);
    assert!(json.contains(r#""triangles":138790"#), "{json}");
    assert!(json.contains(r#""distance":87.50"#), "{json}");
    assert!(json.contains(r#""mode":"auto""#), "{json}");
    assert!(json.contains(r#""level":1"#), "{json}");
    assert!(json.contains(r#""levels":2"#), "{json}");
    assert!(json.contains(r#""settled":true"#), "{json}");
    assert!(json.contains(r#""extent":8.00"#), "{json}");
}

/// The base model is "no level", and JSON has a word for that.
#[test]
fn no_level_is_null_rather_than_a_number() {
    let json = render_stats(
        &SubjectStats::default(),
        &LadderState::default(),
        LodMode::Base,
        false,
        0.0,
        None,
    );
    assert!(json.contains(r#""level":null"#), "{json}");
}

#[test]
fn scene_measurement_waits_for_descendants_and_resident_mesh_data() {
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<SubjectStats>()
        .init_resource::<LadderState>()
        .init_resource::<LodMode>()
        .insert_resource(SubjectState {
            settled: true,
            ..Default::default()
        })
        .add_systems(Update, (measure_subject, publish_stats).chain())
        // The WASM exporter reads a thread-local; run its publication on this
        // test thread rather than the native multithreaded scheduler's workers.
        .edit_schedule(Update, |schedule| {
            schedule.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
        });
    app.update();
    assert!(!app.world().resource::<SubjectState>().measured);
    assert!(STATS_JSON.with(|value| value.borrow().contains("\"settled\":false")));

    let handle = app.world().resource::<Assets<Mesh>>().reserve_handle();
    app.world_mut().spawn(Mesh3d(handle.clone()));
    app.update();
    assert!(!app.world().resource::<SubjectState>().measured);
    assert_eq!(app.world().resource::<SubjectStats>().meshes, 0);
    assert!(STATS_JSON.with(|value| value.borrow().contains("\"settled\":false")));

    app.world_mut()
        .resource_mut::<Assets<Mesh>>()
        .insert(handle.id(), Mesh::from(Cuboid::new(2.0, 2.0, 2.0)))
        .unwrap();
    app.update();
    assert!(app.world().resource::<SubjectState>().measured);
    assert_eq!(app.world().resource::<SubjectStats>().triangles, 12);
    assert_eq!(app.world().resource::<SubjectStats>().meshes, 1);
    assert!(STATS_JSON.with(|value| value.borrow().contains("\"settled\":true")));
}
