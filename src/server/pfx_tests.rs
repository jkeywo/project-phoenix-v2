use super::*;

/// `PfxPlugin::build` must not re-register an asset type the bootstrap has
/// already registered.
///
/// `init_asset` is not idempotent: it swaps in a fresh `Assets<A>` backed by
/// a new `AssetIndexAllocator` and overwrites the `AssetServer`'s handle
/// provider, orphaning every handle minted before it ran. Those handles
/// index past the end of the new storage, so the insert that lands when
/// their load finishes panics out of bounds — which is what crashed the
/// deployed server on load. Only the real bootstrap hit it: the automation
/// bootstrap skips `RendererPlugin`, and so never builds `PfxPlugin`.
#[test]
fn pfx_plugin_preserves_already_registered_image_assets() {
    let mut app = App::new();
    app.add_plugins(bevy::app::TaskPoolPlugin::default())
        .add_plugins(bevy::asset::AssetPlugin::default());

    // Mirror `ImagePlugin::build`: register `Image`, then seed the default
    // image that the rest of the engine expects to find.
    app.init_asset::<Image>();
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(&Handle::<Image>::default(), Image::default())
        .unwrap();

    // A handle minted from the original allocator, standing in for the ones
    // the render and UI plugins mint during `DefaultPlugins`.
    let minted: Handle<Image> = app.world().resource::<Assets<Image>>().reserve_handle();

    app.add_plugins(super::PfxPlugin);

    assert!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&Handle::<Image>::default())
            .is_some(),
        "PfxPlugin discarded the default image seeded by ImagePlugin"
    );

    // Completing that load must still land in this collection. Before the
    // guard, this insert panicked with an out-of-bounds index.
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(minted.id(), Image::default())
        .expect("a handle minted before PfxPlugin must still resolve after it");
}

#[test]
fn diff_torpedo_sets_spawns_new_uuids() {
    let in_flight: HashSet<String> = ["a".into(), "b".into()].into();
    let tracked: HashSet<String> = HashSet::new();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
    let mut to_spawn_sorted = to_spawn.clone();
    to_spawn_sorted.sort();
    assert_eq!(to_spawn_sorted, vec!["a".to_string(), "b".to_string()]);
    assert!(to_despawn.is_empty());
}

#[test]
fn diff_torpedo_sets_despawns_removed_uuids() {
    let in_flight: HashSet<String> = HashSet::new();
    let tracked: HashSet<String> = ["a".into()].into();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
    assert!(to_spawn.is_empty());
    assert_eq!(to_despawn, vec!["a".to_string()]);
}

#[test]
fn diff_torpedo_sets_no_change_when_same() {
    let in_flight: HashSet<String> = ["a".into()].into();
    let tracked: HashSet<String> = ["a".into()].into();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
    assert!(to_spawn.is_empty());
    assert!(to_despawn.is_empty());
}

#[test]
fn diff_torpedo_sets_mixed_spawn_and_despawn() {
    let in_flight: HashSet<String> = ["b".into(), "c".into()].into();
    let tracked: HashSet<String> = ["a".into(), "b".into()].into();
    let (to_spawn, to_despawn) = diff_torpedo_sets(&in_flight, &tracked);
    assert_eq!(to_spawn, vec!["c".to_string()]);
    assert_eq!(to_despawn, vec!["a".to_string()]);
}

#[test]
fn engine_pfx_settings_uses_renderer_defaults_for_sparse_config() {
    let cfg = EnginePfxConfig::default();
    let settings = EnginePfxSettings::from_config(Some(&cfg));
    assert_eq!(settings.color, ENGINE_DEFAULT_COLOR);
    assert_eq!(settings.lifetime_secs, ENGINE_TRAIL_CRUMB_LIFETIME_SECS);
    assert_eq!(settings.geometry_for(true).roll_radians, 0.0);
    assert_eq!(settings.geometry_for(true).scale, 1.0);
}

#[test]
fn engine_pfx_settings_uses_configured_values() {
    let cfg = EnginePfxConfig {
        color: Some([0.1, 0.2, 0.3, 0.4]),
        markers: vec![],
        roll_degrees: Some(90.0),
        scale: Some(1.25),
        trail_lifetime_secs: Some(0.8),
        trail_spawn_interval_secs: Some(0.03),
    };
    let settings = EnginePfxSettings::from_config(Some(&cfg));
    assert_eq!(settings.color, [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(settings.lifetime_secs, 0.8);
    assert!((settings.roll_radians - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert_eq!(settings.scale, 1.25);
    assert_eq!(
        settings.geometry_for(true),
        EngineTrailGeometry {
            roll_radians: std::f32::consts::FRAC_PI_2,
            scale: 1.25,
        }
    );
    assert_eq!(
        settings.geometry_for(false),
        EngineTrailGeometry {
            roll_radians: 0.0,
            scale: 1.0,
        },
        "synthetic fallback emitters retain their unmodified geometry"
    );
}

#[test]
fn attached_trail_roll_rotates_ribbon_width_about_its_direction() {
    let crumbs = VecDeque::from([
        TrailCrumb {
            pos: Vec3::ZERO,
            width: 2.0,
            age: 0.0,
            lifetime: 1.0,
        },
        TrailCrumb {
            pos: Vec3::Z,
            width: 2.0,
            age: 0.0,
            lifetime: 1.0,
        },
    ]);
    let mut mesh = empty_ribbon_mesh();

    build_ribbon_into_mesh(&mut mesh, &crumbs, std::f32::consts::FRAC_PI_2);

    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) => positions,
        other => panic!("expected Float32x3 positions, got {other:?}"),
    };
    assert!(
        positions.iter().all(|position| position[1].abs() < 0.051),
        "a 90-degree roll moves this Z-aligned ribbon out of its default vertical plane"
    );
    assert!(
        positions.iter().any(|position| position[0].abs() > 0.9),
        "the rolled ribbon retains its authored width on the horizontal axis"
    );
}

#[test]
fn clamp_endpoint_returns_target_inside_center_range() {
    let start = Vec3::new(4.0, 0.0, 0.0);
    let target = Vec3::new(0.0, 0.0, -40.0);
    let range_origin = Vec3::ZERO;

    assert_eq!(clamp_endpoint(start, target, range_origin, 50.0), target);
}

#[test]
fn clamp_endpoint_limits_ray_to_centered_range_sphere() {
    let start = Vec3::new(4.0, 0.0, 0.0);
    let target = Vec3::new(4.0, 0.0, -80.0);
    let range_origin = Vec3::ZERO;
    let endpoint = clamp_endpoint(start, target, range_origin, 50.0);

    assert!((endpoint.x - 4.0).abs() < 1e-5);
    assert!((endpoint.z - -49.839745).abs() < 1e-4);
    assert!((endpoint.distance(range_origin) - 50.0).abs() < 1e-4);
}

#[test]
fn target_point_position_transforms_model_point() {
    let rig = crate::entities::model_rig::ModelRig::from_toml(
        r#"
[base]
offset = [1.0, 0.0, 0.0]
rotation = [0.0, 3.1415927, 0.0]
scale = [2.0, 2.0, 2.0]

[[target_points]]
position = [0.5, -0.1, 0.25]
"#,
    )
    .unwrap();
    let markers = ModelMarkers::from_rig(&rig);
    let transform = Transform::from_translation(Vec3::new(10.0, 2.0, -3.0));

    let point = target_point_position(&transform, Some(&markers), Some(0)).unwrap();

    assert!(
        (point - Vec3::new(10.0, 1.8, -3.5)).length() < 1e-5,
        "target point must include the sidecar base rig, got {point:?}"
    );
}

#[test]
fn target_point_choice_stays_stable_for_live_beam_key() {
    let mut state = BeamPfxState::default();
    let first = choose_target_point_index("beam:a", 3, &mut state).unwrap();
    let second = choose_target_point_index("beam:a", 3, &mut state).unwrap();

    assert_eq!(first, second);
    assert!(first < 3);

    assert_eq!(choose_target_point_index("beam:a", 0, &mut state), None);
    assert!(state.target_point_choices.is_empty());
}

#[test]
fn segment_transform_places_midpoint_and_scales_height() {
    let transform = segment_transform(Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0), 0.25);
    assert_eq!(transform.translation, Vec3::new(0.0, 2.0, 0.0));
    assert_eq!(transform.scale, Vec3::new(0.25, 4.0, 0.25));
}

/// Reads a quad mesh's positions zipped with its UVs, panicking with a
/// useful message if either attribute is missing or the wrong shape.
fn quad_position_uvs(mesh: &Mesh) -> Vec<([f32; 3], [f32; 2])> {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
        _ => panic!("quad must have Float32x3 positions"),
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(bevy::mesh::VertexAttributeValues::Float32x2(u)) => u.clone(),
        _ => panic!("quad must have Float32x2 UVs"),
    };
    assert_eq!(positions.len(), 4, "ribbon quad must have four corners");
    assert_eq!(uvs.len(), positions.len());
    positions.into_iter().zip(uvs).collect()
}

/// Issue #938: the beam ribbon must sample one constant texture column
/// along its length. `beam_glow.png` / `beam_core.png` are lens-shaped —
/// they fade out at their left and right edges too — so any U that varies
/// with local Y stretches that horizontal falloff over the whole
/// muzzle-to-target span and reintroduces the mid-beam bulge and the
/// taper to nothing at both ends.
#[test]
fn beam_ribbon_quad_pins_u_so_the_beam_is_constant_width() {
    let mesh = beam_ribbon_quad_mesh();
    let corners = quad_position_uvs(&mesh);

    assert!(
        corners.iter().all(|(_, uv)| uv[0] == BEAM_PROFILE_U),
        "every corner must sample the same texture column, got {:?}",
        corners.iter().map(|(_, uv)| uv[0]).collect::<Vec<_>>()
    );

    // The across-width axis must still sweep the full 0..1 so V keeps
    // crossing the texture's soft edge — that is what gives the beam its
    // falloff instead of a hard-edged bar.
    let v_at_min_x = corners
        .iter()
        .filter(|(p, _)| p[0] < 0.0)
        .map(|(_, uv)| uv[1])
        .collect::<Vec<_>>();
    let v_at_max_x = corners
        .iter()
        .filter(|(p, _)| p[0] > 0.0)
        .map(|(_, uv)| uv[1])
        .collect::<Vec<_>>();
    assert_eq!(v_at_min_x, vec![0.0, 0.0], "-X edge must sample v=0");
    assert_eq!(v_at_max_x, vec![1.0, 1.0], "+X edge must sample v=1");
}

/// The projectile ribbon must keep its tail-to-head U sweep: blaster
/// bolts and torpedo flares use asymmetric textures (bright rounded tip,
/// tapered fade) where the length falloff is the effect, not a bug. This
/// guards the fix for #938 from being generalised onto them.
#[test]
fn projectile_ribbon_quad_keeps_its_tail_to_head_u_sweep() {
    let corners = quad_position_uvs(&unit_ribbon_quad_mesh());

    let u_at_tail = corners
        .iter()
        .filter(|(p, _)| p[1] < 0.0)
        .map(|(_, uv)| uv[0])
        .collect::<Vec<_>>();
    let u_at_head = corners
        .iter()
        .filter(|(p, _)| p[1] > 0.0)
        .map(|(_, uv)| uv[0])
        .collect::<Vec<_>>();

    assert_eq!(u_at_tail, vec![0.0, 0.0], "-Y (tail) edge must sample u=0");
    assert_eq!(u_at_head, vec![1.0, 1.0], "+Y (head) edge must sample u=1");
}

#[test]
fn upsert_engine_head_crumb_pins_existing_head_to_marker() {
    let mut crumbs = VecDeque::from([
        TrailCrumb {
            pos: Vec3::new(0.02, 0.0, 0.0),
            width: 0.2,
            age: 0.2,
            lifetime: 1.0,
        },
        TrailCrumb {
            pos: Vec3::new(0.0, 0.0, 0.5),
            width: 0.2,
            age: 0.4,
            lifetime: 1.0,
        },
    ]);

    upsert_engine_head_crumb(&mut crumbs, Vec3::ZERO, 0.5, 1.5);

    assert_eq!(crumbs.len(), 2);
    assert_eq!(crumbs[0].pos, Vec3::ZERO);
    assert_eq!(crumbs[0].width, 0.5);
    assert_eq!(crumbs[0].age, 0.0);
    assert_eq!(crumbs[1].pos, Vec3::new(0.0, 0.0, 0.5));
}

#[test]
fn render_crumbs_from_marker_adds_backward_tail_for_new_trail() {
    let crumbs = VecDeque::from([TrailCrumb {
        pos: Vec3::ZERO,
        width: 0.5,
        age: 0.0,
        lifetime: 1.5,
    }]);

    let render = render_crumbs_from_marker(&crumbs, Vec3::ZERO, Vec3::Z, 0.5, 1.5);

    assert_eq!(render.len(), 2);
    assert_eq!(render[0].pos, Vec3::ZERO);
    assert_eq!(render[1].pos, Vec3::Z * ENGINE_TRAIL_MIN_CRUMB_DIST);
}

#[test]
fn reverse_engine_trail_is_not_created_or_retained() {
    assert_eq!(
        classify_engine_trail_motion(-0.01, 12.5),
        EngineTrailMotion::Reverse
    );
    assert_eq!(
        classify_engine_trail_motion(6.25, 12.5),
        EngineTrailMotion::Forward(0.5)
    );
    assert_eq!(
        classify_engine_trail_motion(0.0, 12.5),
        EngineTrailMotion::Idle,
        "a stopped ship keeps the existing idle trail-fade behaviour"
    );

    let mut crumbs = VecDeque::from([TrailCrumb {
        pos: Vec3::ZERO,
        width: 0.5,
        age: 0.0,
        lifetime: 1.5,
    }]);

    // This is the state transition `update_engine_trail` applies to a
    // pre-existing ribbon when `forward_speed` becomes negative.
    clear_engine_trail_crumbs(&mut crumbs);

    assert!(
        crumbs.is_empty(),
        "reverse motion must leave no engine trail crumbs"
    );
}

#[test]
fn engine_emitters_use_marker_direction() {
    let rig = crate::entities::model_rig::ModelRig::from_toml(
        r#"
[base]
offset = [1.0, 0.0, 0.0]
rotation = [0.0, 3.1415927, 0.0]
scale = [2.0, 1.0, 2.0]

[markers.aft_exhaust]
position = [1.0, 0.5, -2.0]
direction = [0.0, 0.0, -1.0]
"#,
    )
    .unwrap();
    let markers = ModelMarkers::from_rig(&rig);
    let cfg = EnginePfxConfig {
        color: None,
        markers: vec!["aft_exhaust".to_string()],
        roll_degrees: None,
        scale: None,
        trail_lifetime_secs: None,
        trail_spawn_interval_secs: None,
    };

    let emitters = engine_emitters(
        &Transform::from_translation(Vec3::new(10.0, 0.0, 20.0)),
        Some(&markers),
        Some(&cfg),
    );

    assert_eq!(emitters.len(), 1);
    assert!((emitters[0].origin - Vec3::new(9.0, 0.5, 24.0)).length() < 1e-5);
    assert!((emitters[0].direction - Vec3::Z).length() < 1e-5);
    assert!(emitters[0].is_marker_attached);
}

#[test]
fn engine_emitters_rotate_marker_direction_with_ship() {
    let mut map = HashMap::new();
    map.insert(
        "aft_exhaust".to_string(),
        crate::entities::model_rig::Marker {
            position: [0.0, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
        },
    );
    let markers = ModelMarkers::from_markers(map);
    let cfg = EnginePfxConfig {
        color: None,
        markers: vec!["aft_exhaust".to_string()],
        roll_degrees: None,
        scale: None,
        trail_lifetime_secs: None,
        trail_spawn_interval_secs: None,
    };
    let transform = Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));

    let emitters = engine_emitters(&transform, Some(&markers), Some(&cfg));

    let expected = transform.rotation * Vec3::NEG_Z;
    assert!((emitters[0].direction - expected).length() < 1e-6);
}

#[test]
fn engine_emitters_fallback_points_aft_from_ship_forward() {
    let emitters = engine_emitters(
        &Transform::from_translation(Vec3::new(1.0, 0.0, 2.0)),
        None,
        None,
    );

    assert_eq!(emitters.len(), 1);
    assert_eq!(emitters[0].origin, Vec3::new(1.0, 0.0, 5.0));
    assert_eq!(emitters[0].direction, Vec3::Z);
    assert!(!emitters[0].is_marker_attached);
}

// ── Integration: does the beam PFX actually track ship movement? ──────
//
// The pure-function tests above all pass in isolation, but the reported
// bug ("beam frozen when attacker/target move") is a whole-schedule
// ordering hazard: `sync_phaser_beams` reads ship `Transform`, which is
// written by `sync_ship_position` (in `ShipPlugin`), which in turn must
// run *after* whatever system computes this tick's `ShipPhysics`.
//
// These tests drive movement through the REAL production pipeline
// (`process_helm_inputs`, via a constant `HelmInput` admitted command on
// a `LocalShip`) rather than a synthetic mover system. A synthetic mover
// can only be pinned to a shared *label* (e.g. `AiTickLabel`) from
// outside `ship_plugin.rs`, since the real writer/reader systems are
// private to that module — and two systems that both merely reference
// the same label, without an edge *between* them, have no defined
// relative order (this was tried and silently passed for the wrong
// reason). Driving `process_helm_inputs` for real gets the exact,
// explicit `.after(process_helm_inputs)` edge on `sync_ship_position`
// for free, with no privacy workarounds needed.
fn thrust_command() -> crate::core::messages::AdmittedCommands {
    crate::core::messages::AdmittedCommands(vec![crate::core::messages::AdmittedCommand {
        target: crate::ship::system_registry::helm_thrust_system_id(),
        payload: crate::core::messages::SystemControlPayload::SetThrust { value: 1.0 },
        response_token: None,
        feedback_correlation: None,
    }])
}

fn beam_test_app() -> App {
    let mut app = App::new();
    app.configure_sets(
        Update,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    )
    .add_plugins(crate::lobby::LobbyPlugin)
    .add_plugins(bevy::app::TaskPoolPlugin::default())
    .add_plugins(bevy::time::TimePlugin)
    .add_plugins(bevy::asset::AssetPlugin::default())
    .init_asset::<Mesh>()
    .init_asset::<StandardMaterial>()
    .init_asset::<Image>()
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(34),
    ))
    .init_resource::<crate::core::messages::InterSystemQueue>()
    .insert_resource(PhaserRenderConfig {
        beam_range: 1000.0,
        ..Default::default()
    })
    .insert_resource(crate::console::weapons::PhaserCombatConfigResource(
        crate::entities::config::PhaserCombatConfig { banks: vec![] },
    ))
    .add_plugins(crate::ship_plugin::ShipPlugin)
    .add_plugins(super::PfxPlugin);

    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));
    app
}

fn beam_body_translation(app: &mut App) -> Vec3 {
    // Multiple BeamBody entities exist now (crossed glow + core ribbon
    // layers), but they all share the same start/end and therefore the
    // same segment midpoint — any one of them is representative.
    let mut q = app
        .world_mut()
        .query_filtered::<&Transform, With<BeamBody>>();
    q.iter(app.world())
        .next()
        .expect("BeamBody entity must exist")
        .translation
}

#[test]
fn beam_transform_tracks_target_ship_physics_movement_across_ticks() {
    let mut app = beam_test_app();

    let target_uuid = "target-uuid-1".to_string();

    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid("shooter-uuid-1".to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        ShipPhysics::default(),
        {
            let mut beam = ActiveBeam::default();
            beam.start("", target_uuid.clone(), 10.0, 0.0);
            beam
        },
    ));

    let target = app
        .world_mut()
        .spawn((
            LocalShip,
            EntityUuid(target_uuid),
            Transform::from_xyz(0.0, 0.0, -10.0),
            ShipPhysics {
                z: -10.0,
                ..Default::default()
            },
            thrust_command(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::LastHelmInput::default(),
            // `integrate_ship_physics` (issue #695) is scoped to
            // `AiHighFidelity` — add the marker + helm intent
            // components so `process_helm_inputs` -> physics still
            // moves this ship, matching pre-#695 behavior.
            crate::ai::server::AiHighFidelity,
            (
                crate::ship::helm::ThrustInput::default(),
                crate::ship::helm::SteeringInput::default(),
                crate::ship::helm::LateralThrustInput::default(),
                crate::ship::helm::VerticalThrustInput::default(),
                crate::ship::helm::ImpulseCommand::default(),
                crate::ship::helm::BoostCommand::default(),
            ),
        ))
        .id();

    // Precise check (catches a same-tick-stale ordering bug, not just a
    // hard freeze): after every tick, the beam midpoint must reflect
    // THIS tick's `ShipPhysics.z` (ground truth, read directly), not the
    // previous tick's. A `sync_ship_position` ordered before the system
    // that writes `ShipPhysics` this tick would make the beam trail by
    // exactly one tick's movement -- a loose "did it move at all?" check
    // would not catch that, since a laggy beam still moves every tick.
    for _ in 0..5 {
        app.update();
        let ground_truth_z = app.world().get::<ShipPhysics>(target).unwrap().z;
        let expected_mid_z = ground_truth_z / 2.0; // shooter stays at z=0
        let actual_mid_z = beam_body_translation(&mut app).z;
        assert!(
            (actual_mid_z - expected_mid_z).abs() < 0.01,
            "beam midpoint.z={actual_mid_z} should match this tick's target \
                 position (expected {expected_mid_z}, ground-truth target.z={ground_truth_z}) \
                 -- beam is reading a stale (last-tick) Transform"
        );
    }
}

#[test]
fn beam_transform_tracks_shooter_ship_physics_movement_across_ticks() {
    let mut app = beam_test_app();

    let target_uuid = "target-uuid-2".to_string();

    let shooter = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            LocalShip,
            EntityUuid("shooter-uuid-2".to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            ShipPhysics::default(),
            {
                let mut beam = ActiveBeam::default();
                beam.start("", target_uuid.clone(), 10.0, 0.0);
                beam
            },
            thrust_command(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::LastHelmInput::default(),
            // `integrate_ship_physics` (issue #695) is scoped to
            // `AiHighFidelity` — add the marker + helm intent
            // components so `process_helm_inputs` -> physics still
            // moves this ship, matching pre-#695 behavior.
            crate::ai::server::AiHighFidelity,
            (
                crate::ship::helm::ThrustInput::default(),
                crate::ship::helm::SteeringInput::default(),
                crate::ship::helm::LateralThrustInput::default(),
                crate::ship::helm::VerticalThrustInput::default(),
                crate::ship::helm::ImpulseCommand::default(),
                crate::ship::helm::BoostCommand::default(),
            ),
        ))
        .id();

    app.world_mut().spawn((
        EntityUuid(target_uuid),
        Transform::from_xyz(0.0, 0.0, -10.0),
        ShipPhysics {
            z: -10.0,
            ..Default::default()
        },
    ));

    // Same precise per-tick check as the target-movement test above, but
    // with the roles reversed: the shooter is the one being moved.
    for _ in 0..5 {
        app.update();
        let ground_truth_z = app.world().get::<ShipPhysics>(shooter).unwrap().z;
        let expected_mid_z = (ground_truth_z + (-10.0)) / 2.0; // target stays at z=-10
        let actual_mid_z = beam_body_translation(&mut app).z;
        assert!(
            (actual_mid_z - expected_mid_z).abs() < 0.01,
            "beam midpoint.z={actual_mid_z} should match this tick's shooter \
                 position (expected {expected_mid_z}, ground-truth shooter.z={ground_truth_z}) \
                 -- beam is reading a stale (last-tick) Transform"
        );
    }
}

/// Issue #938, the delivery half. The two mesh-factory tests above prove
/// `beam_ribbon_quad_mesh` pins U, but neither proves the phaser is *built*
/// from it: swapping the single `meshes.add(..)` call in `upsert_beam` back
/// to `unit_ribbon_quad_mesh()` reintroduces the reported artefact in full
/// with both of them still green. So boot the real plugin, let
/// `sync_phaser_beams` spawn actual beam entities, and read the UVs off the
/// mesh those entities genuinely reference.
#[test]
fn spawned_beam_bodies_use_the_pinned_u_ribbon_mesh() {
    let mut app = beam_test_app();

    let target_uuid = "target-uuid-3".to_string();

    app.world_mut().spawn((
        crate::server_app::Ship,
        LocalShip,
        EntityUuid("shooter-uuid-3".to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        ShipPhysics::default(),
        {
            let mut beam = ActiveBeam::default();
            beam.start("", target_uuid.clone(), 10.0, 0.0);
            beam
        },
    ));

    app.world_mut().spawn((
        EntityUuid(target_uuid),
        Transform::from_xyz(0.0, 0.0, -10.0),
        ShipPhysics {
            z: -10.0,
            ..Default::default()
        },
    ));

    app.update();

    // Both layers (glow, core) and both crossed quads (`_a`, `_b`) are
    // separate entities, so check every one rather than the first: a fix
    // applied to only one layer must not pass. `>=` rather than `== 4` so
    // adding a further layer later doesn't fail this for the wrong reason,
    // while still catching a query that silently matched nothing (which
    // would make the per-quad assertion below vacuous).
    let handles: Vec<Handle<Mesh>> = {
        let mut q = app.world_mut().query_filtered::<&Mesh3d, With<BeamBody>>();
        q.iter(app.world()).map(|m| m.0.clone()).collect()
    };
    assert!(
        handles.len() >= 4,
        "expected the crossed glow + core BeamBody quads to be spawned, got {}",
        handles.len()
    );

    let meshes = app.world().resource::<Assets<Mesh>>();
    for (i, handle) in handles.iter().enumerate() {
        let mesh = meshes
            .get(handle)
            .expect("a spawned BeamBody's mesh handle must resolve");
        let corners = quad_position_uvs(mesh);
        assert!(
            corners.iter().all(|(_, uv)| uv[0] == BEAM_PROFILE_U),
            "BeamBody quad {i} must sample the pinned texture column \
                 {BEAM_PROFILE_U} at every corner, got {:?} -- `upsert_beam` is \
                 building the beam from the projectile ribbon, whose U sweeps \
                 along the beam and stretches the texture's horizontal falloff \
                 over the whole muzzle-to-target span (issue #938)",
            corners.iter().map(|(_, uv)| uv[0]).collect::<Vec<_>>()
        );
    }
}
