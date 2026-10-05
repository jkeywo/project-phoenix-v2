use super::*;
use crate::entities::config::{MeshConfig, MeshShape};

fn primary_mesh() -> MeshSection {
    MeshSection(MeshConfig {
        model: Some("assets/models/dynasty_destroyer.glb".to_string()),
        variant: None,
        shape: MeshShape::Cuboid,
        colour: vec![0.5, 0.5, 0.5],
        radius: 1.0,
        size: Some([2.0, 1.0, 4.0]),
        minor_radius: 0.0,
        emissive: None,
        scale: 1.0,
        rotation: [0.0, 0.0, 0.0],
    })
}

#[derive(Resource, Default)]
struct PresentationGeometry(Option<(Vec3, Vec3, Vec3)>);

fn observe_presentation_geometry(
    subjects: Query<(&Transform, &ModelMarkers)>,
    mut observed: ResMut<PresentationGeometry>,
) {
    let (transform, markers) = subjects.single().expect("one visual subject");
    observed.0 = Some((
        markers
            .resolve_world_position(transform, "fore_emitter")
            .unwrap(),
        markers
            .resolve_world_direction(transform, "fore_emitter")
            .unwrap(),
        markers
            .resolve_target_point_world_position(transform, 2)
            .unwrap(),
    ));
}

fn profile(with_presentation_consumer: bool, mesh: MeshSection) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<ModelRigReadiness>()
        .add_systems(PreUpdate, sync_authoritative_model_markers);
    if with_presentation_consumer {
        app.init_resource::<PresentationGeometry>()
            .add_systems(Update, observe_presentation_geometry);
    }
    let entity = app
        .world_mut()
        .spawn((
            mesh,
            Transform::from_translation(Vec3::new(12.0, 3.0, -8.0)),
        ))
        .id();
    app.update();
    (app, entity)
}

/// The authoritative seam has no GLB, Scene, AssetServer, camera, or render
/// plugin dependency. A rendererless GM profile therefore gets the same
/// weapon origin and target geometry a rendered host consumes.
#[test]
fn primary_rig_markers_load_without_a_renderer_or_glb_scene() {
    let (app, entity) = profile(false, primary_mesh());
    assert!(
        app.world().get_resource::<AssetServer>().is_none(),
        "precondition: this profile owns no renderer or asset server"
    );

    let transform = app.world().get::<Transform>(entity).unwrap();
    let markers = app
        .world()
        .get::<ModelMarkers>(entity)
        .expect("the simulation-side loader attaches the primary rig");
    let weapon_origin = markers
        .resolve_world_position(transform, "fore_emitter")
        .expect("the authored weapon marker resolves");
    let target = markers
        .resolve_target_point_world_position(transform, 1)
        .expect("the authored target point resolves");

    assert!(
        (weapon_origin - Vec3::new(11.999_293, 2.9, -7.455_968)).length() < 1e-4,
        "primary base rig must be composed into the weapon origin, got {weapon_origin:?}"
    );
    assert!(
        (target - Vec3::new(12.250_398, 2.8, -8.149_602)).length() < 1e-4,
        "primary base rig must be composed into the target point, got {target:?}"
    );
}

#[cfg(not(target_arch = "wasm32"))]
struct RigFixture {
    directory: std::path::PathBuf,
    model: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl RigFixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("phoenix-marker-batch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let model = directory.join("fixture.glb").to_string_lossy().into_owned();
        Self { directory, model }
    }

    fn write(&self, variant: Option<&str>, body: &str) {
        std::fs::write(
            crate::entities::model_rig::sidecar_path(&self.model, variant),
            body,
        )
        .unwrap();
    }

    fn mesh(&self, variant: Option<&str>, scale: f32) -> MeshSection {
        let mut mesh = primary_mesh();
        mesh.0.model = Some(self.model.clone());
        mesh.0.variant = variant.map(str::to_owned);
        mesh.0.scale = scale;
        mesh
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for RigFixture {
    fn drop(&mut self) {
        for entry in crate::repo_fixtures::fs::read_dir(&self.directory).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(&self.directory).unwrap();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn rig_body(x: f32) -> String {
    format!(
        "[base]\noffset = [1.0, 0.0, 0.0]\nscale = [2.0, 2.0, 2.0]\n\
             [markers.fore_emitter]\nposition = [{x}, 0.0, 0.0]\n\
             direction = [0.0, 0.0, -1.0]\n\
             [[target_points]]\nposition = [{x}, 1.0, 0.0]\n"
    )
}

/// A shared rig is reusable geometry, not a shared entity transform or a
/// model-only key: authored variants and parent scales stay independent.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn marker_batch_preserves_variants_and_each_entities_transform() {
    let fixture = RigFixture::new();
    fixture.write(None, &rig_body(3.0));
    fixture.write(Some("alternate"), &rig_body(5.0));
    let mut app = App::new();
    app.init_resource::<ModelRigReadiness>()
        .add_systems(PreUpdate, sync_authoritative_model_markers);
    let subjects: Vec<_> = [
        (None, 1.0, 7.0),
        (None, 3.0, 21.0),
        (Some("alternate"), 1.0, 11.0),
    ]
    .into_iter()
    .map(|(variant, scale, expected_x)| {
        let entity = app
            .world_mut()
            .spawn((
                fixture.mesh(variant, scale),
                Transform::from_xyz(10.0, 0.0, 0.0),
            ))
            .id();
        (entity, scale, expected_x + 10.0)
    })
    .collect();
    app.update();
    for (entity, scale, expected_x) in subjects {
        let transform = app.world().get::<Transform>(entity).unwrap();
        let markers = app.world().get::<ModelMarkers>(entity).unwrap();
        assert_eq!(transform.scale, Vec3::splat(scale));
        assert_eq!(
            markers.resolve_world_position(transform, "fore_emitter"),
            Some(Vec3::new(expected_x, 0.0, 0.0))
        );
        assert_eq!(
            markers.resolve_target_point_world_position(transform, 0),
            Some(Vec3::new(expected_x, 2.0 * scale, 0.0))
        );
    }
    assert!(!app
        .world()
        .resource::<ModelRigReadiness>()
        .blocks_simulation());
}

/// No lookup result survives the sync invocation, including the identity
/// fallback for absence/malformed content. Future spawns resolve the latest
/// body while already-attached entities retain their canonical geometry.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn marker_batch_observes_sidecar_delivery_and_replacement_on_later_passes() {
    let fixture = RigFixture::new();
    let mut app = App::new();
    app.init_resource::<ModelRigReadiness>()
        .add_systems(PreUpdate, sync_authoritative_model_markers);
    let mut prior = Vec::new();
    for (body, expected) in [
        (None, None),
        (Some("[malformed".to_string()), None),
        (Some(rig_body(3.0)), Some(7.0)),
        (Some(rig_body(5.0)), Some(11.0)),
    ] {
        if let Some(body) = body {
            fixture.write(None, &body);
        }
        // More than one entity exercises sharing inside each pass.
        for _ in 0..2 {
            let entity = app
                .world_mut()
                .spawn((fixture.mesh(None, 1.0), Transform::default()))
                .id();
            prior.push((entity, expected));
        }
        app.update();
        for &(entity, expected) in &prior {
            let markers = app.world().get::<ModelMarkers>(entity).unwrap();
            assert_eq!(
                markers.resolve_world_position(&Transform::IDENTITY, "fore_emitter"),
                expected.map(|x| Vec3::new(x, 0.0, 0.0))
            );
        }
    }
}

/// Registering a presentation-side read must not change canonical geometry
/// or the authoritative digest. This is the profile-axis parity guard: both
/// worlds load their markers through the simulation system; render merely
/// observes the result.
#[test]
fn rendered_and_rendererless_profiles_share_geometry_and_digest() {
    let mut authored_mesh = primary_mesh();
    authored_mesh.0.scale = 1.75;
    authored_mesh.0.rotation = [0.15, -0.35, 0.2];
    let (rendererless, rendererless_entity) = profile(false, authored_mesh.clone());
    let (rendered, rendered_entity) = profile(true, authored_mesh);

    let geometry = |app: &App, entity: Entity| {
        let transform = app.world().get::<Transform>(entity).unwrap();
        let markers = app.world().get::<ModelMarkers>(entity).unwrap();
        (
            markers
                .resolve_world_position(transform, "fore_emitter")
                .unwrap(),
            markers
                .resolve_world_direction(transform, "fore_emitter")
                .unwrap(),
            markers
                .resolve_target_point_world_position(transform, 2)
                .unwrap(),
        )
    };

    let rendered_geometry = geometry(&rendered, rendered_entity);
    assert_eq!(
        rendered.world().get::<Transform>(rendered_entity),
        rendererless.world().get::<Transform>(rendererless_entity),
        "authored mesh scale/rotation must be applied before the render axis splits"
    );
    assert_eq!(
        rendered_geometry,
        geometry(&rendererless, rendererless_entity),
        "the render axis may not select different weapon/target geometry"
    );
    assert_eq!(
        rendered
            .world()
            .resource::<PresentationGeometry>()
            .0
            .expect("the rendered profile consumed canonical markers"),
        rendered_geometry,
        "presentation must read the simulation-owned component, not resolve a second rig"
    );
    assert_eq!(
        crate::sim_digest::state_digest(&rendered),
        crate::sim_digest::state_digest(&rendererless),
        "the same authored parent transform must leave both profiles on one digest; the \
             geometry assertion above separately covers ModelMarkers, which are deferred state"
    );
}

/// Native filesystem ingestion and the browser's JS-delivery shape must
/// freeze the identical sidecar key/body pair. The raw body is the identity
/// input even when it is malformed (runtime falls back) or empty (404), so
/// neither failure mode may alias the valid authored rig.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_and_wasm_shaped_sidecars_share_and_move_frozen_content_identity() {
    let mesh = primary_mesh();
    let config = crate::entities::config::EntityConfig {
        mesh: Some(mesh.0.clone()),
        ..Default::default()
    };
    let path = primary_sidecar_path(&mesh.0).expect("fixture has a model");
    let body = crate::repo_fixtures::fs::read_to_string(&path).expect("shipped primary rig exists");

    crate::content_ledger::reset();
    record_primary_sidecar_from_fs(&config);
    crate::content_ledger::freeze();
    assert!(crate::content_ledger::frozen_covers(&path));
    let native_digest = crate::content_ledger::frozen_or_live().fold();

    crate::content_ledger::reset();
    let _ = crate::entities::config_cache::wasm_push_sidecar_toml(path.clone(), body);
    crate::content_ledger::freeze();
    let wasm_shaped_digest = crate::content_ledger::frozen_or_live().fold();
    assert_eq!(
        wasm_shaped_digest, native_digest,
        "the two boot profiles must bind the same canonical path and exact bytes"
    );

    crate::content_ledger::reset();
    let _ = crate::entities::config_cache::wasm_push_sidecar_toml(
        path.clone(),
        "[malformed-primary-rig".to_string(),
    );
    crate::content_ledger::freeze();
    let malformed_digest = crate::content_ledger::frozen_or_live().fold();
    assert_ne!(malformed_digest, native_digest);

    crate::content_ledger::reset();
    let _ = crate::entities::config_cache::wasm_push_sidecar_toml(path, String::new());
    crate::content_ledger::freeze();
    let absent_digest = crate::content_ledger::frozen_or_live().fold();
    assert_ne!(absent_digest, native_digest);
    assert_ne!(absent_digest, malformed_digest);
    crate::content_ledger::reset();
}

#[derive(Resource, Default)]
struct FixedConsumerProbe {
    steps: u32,
    saw_missing_markers: bool,
    saw_canonical_markers: bool,
}

fn spawn_runtime_model(mut commands: Commands, mut spawned: Local<bool>) {
    if !*spawned {
        commands.spawn((primary_mesh(), Transform::default()));
        *spawned = true;
    }
}

fn consume_runtime_model(
    mut probe: ResMut<FixedConsumerProbe>,
    models: Query<Has<ModelMarkers>, With<MeshSection>>,
) {
    for has_markers in &models {
        probe.saw_missing_markers |= !has_markers;
        probe.saw_canonical_markers |= has_markers;
    }
    probe.steps += 1;
}

/// A model spawned by one fixed step is synchronised in `FixedLast`, before
/// the following step can present it to an authoritative consumer. This is
/// the runtime/script-spawn half of the seam; it does not rely on a render
/// frame occurring between fixed steps.
#[test]
fn runtime_spawn_is_canonical_before_the_next_fixed_consumer() {
    let period = std::time::Duration::from_millis(10);
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .init_resource::<ModelRigReadiness>()
        .init_resource::<FixedConsumerProbe>()
        .add_systems(FixedUpdate, (spawn_runtime_model, consume_runtime_model))
        .add_systems(FixedLast, sync_authoritative_model_markers);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);

    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    app.update();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 2));
    app.update();

    let probe = app.world().resource::<FixedConsumerProbe>();
    assert_eq!(probe.steps, 2);
    assert!(!probe.saw_missing_markers);
    assert!(probe.saw_canonical_markers);
}

fn count_fixed_steps(mut probe: ResMut<FixedConsumerProbe>) {
    probe.steps += 1;
}

/// The runtime-spawn safeguard is dormant in an ordinary frame: all whole
/// fixed steps and the fractional interpolation remainder survive when no
/// live entity is waiting on a primary sidecar.
#[test]
fn normal_frame_keeps_fixed_overstep_when_no_rig_is_blocked() {
    let period = std::time::Duration::from_millis(10);
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .init_resource::<ModelRigReadiness>()
        .init_resource::<FixedConsumerProbe>()
        .add_systems(FixedUpdate, count_fixed_steps)
        .add_systems(FixedLast, discard_blocked_model_rig_overstep);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);

    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    app.update();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        period * 3 + period / 2,
    ));
    app.update();

    assert_eq!(app.world().resource::<FixedConsumerProbe>().steps, 3);
    assert_eq!(app.world().resource::<Time<Fixed>>().overstep(), period / 2);
}

#[test]
fn blocked_runtime_rig_discards_only_unbegun_whole_steps() {
    let period = std::time::Duration::from_millis(10);
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .init_resource::<ModelRigReadiness>()
        .init_resource::<FixedConsumerProbe>()
        .add_systems(FixedUpdate, count_fixed_steps)
        .add_systems(FixedLast, discard_blocked_model_rig_overstep);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    app.world_mut()
        .resource_mut::<ModelRigReadiness>()
        .live_blocked = true;

    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    app.update();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        period * 3 + period / 2,
    ));
    app.update();

    assert_eq!(app.world().resource::<FixedConsumerProbe>().steps, 1);
    assert_eq!(app.world().resource::<Time<Fixed>>().overstep(), period / 2);
}
