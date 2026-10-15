use super::*;

fn profile() -> FieldConfig {
    FieldConfig {
        falloff: FieldFalloff::Step,
        strength: 1.0,
        sensor_absorption_per_unit: 0.2,
        comms_absorption_per_unit: 0.0,
        evolution: None,
    }
}
fn volume<'a>(id: &'a str, shape: &'a RegionShape, profile: &'a FieldConfig) -> SignalVolume<'a> {
    SignalVolume {
        id,
        shape,
        pose: FieldPose::stationary(Vec3::ZERO),
        field: Some(profile),
        material: None,
    }
}
fn close(left: f32, right: f32) {
    assert!((left - right).abs() < 1e-6, "{left} != {right}");
}

#[test]
fn outside_endpoints_still_cross_cloud_and_classes_remain_distinct() {
    let shape = RegionShape::Sphere { radius: 10.0 };
    let profile = profile();
    let (a, b) = (-Vec3::X * 20.0, Vec3::X * 20.0);
    assert!(!shape.contains(a, Vec3::ZERO) && !shape.contains(b, Vec3::ZERO));
    close(
        signal_transmission(
            vec![volume("cloud", &shape, &profile)],
            a,
            b,
            SignalClass::Sensors,
        ),
        0.2,
    );
    assert_eq!(
        signal_transmission(
            vec![volume("cloud", &shape, &profile)],
            a,
            b,
            SignalClass::Comms
        ),
        1.0
    );
}

#[test]
fn overlap_is_stable_and_thin_clouds_are_not_skipped() {
    let shape = RegionShape::Sphere { radius: 0.1 };
    let profile = profile();
    let (a, b) = (-Vec3::X * 10_000.0, Vec3::X * 10_000.0);
    let first = signal_transmission(
        vec![volume("b", &shape, &profile), volume("a", &shape, &profile)],
        a,
        b,
        SignalClass::Sensors,
    );
    let reverse = signal_transmission(
        vec![volume("a", &shape, &profile), volume("b", &shape, &profile)],
        a,
        b,
        SignalClass::Sensors,
    );
    assert_eq!(first, reverse);
    close(first, 1.0 / 1.08);
}

#[test]
fn sphere_and_box_grazing_are_clear_but_substantial_terrain_blocks() {
    let sphere = RegionShape::Sphere { radius: 10.0 };
    let cube = RegionShape::Box {
        half_extents: [10.0; 3],
        yaw: 0.0,
    };
    let material = SignalMaterialConfig {
        sensor_absorption_per_unit: 0.0,
        comms_absorption_per_unit: 0.0,
        sensor_opaque: true,
        comms_opaque: false,
    };
    let terrain = |shape| SignalVolume {
        id: "planet",
        shape,
        pose: FieldPose::stationary(Vec3::ZERO),
        field: None,
        material: Some(&material),
    };
    for shape in [&sphere, &cube] {
        assert_eq!(
            signal_transmission(
                vec![terrain(shape)],
                Vec3::new(-20.0, 0.0, 10.0),
                Vec3::new(20.0, 0.0, 10.0),
                SignalClass::Sensors
            ),
            1.0
        );
        assert_eq!(
            signal_transmission(
                vec![terrain(shape)],
                -Vec3::X * 20.0,
                Vec3::X * 20.0,
                SignalClass::Sensors
            ),
            0.0
        );
        assert_eq!(
            signal_transmission(
                vec![terrain(shape)],
                -Vec3::X * 20.0,
                Vec3::X * 20.0,
                SignalClass::Comms
            ),
            1.0
        );
    }
}

#[test]
fn planar_annulus_has_two_intervals_and_preserves_vertical_membership() {
    let annulus = RegionShape::Torus {
        inner_radius: 1.0,
        outer_radius: 3.0,
    };
    let profile = profile();
    close(
        signal_transmission(
            vec![volume("ring", &annulus, &profile)],
            -Vec3::X * 5.0,
            Vec3::X * 5.0,
            SignalClass::Sensors,
        ),
        1.0 / 1.8,
    );
    close(
        signal_transmission(
            vec![volume("ring", &annulus, &profile)],
            Vec3::new(2.0, -5.0, 0.0),
            Vec3::new(2.0, 5.0, 0.0),
            SignalClass::Sensors,
        ),
        1.0 / 3.0,
    );
    let disc = RegionShape::Torus {
        inner_radius: 0.0,
        outer_radius: 3.0,
    };
    close(
        signal_transmission(
            vec![volume("disc", &disc, &profile)],
            -Vec3::Y * 5.0,
            Vec3::Y * 5.0,
            SignalClass::Sensors,
        ),
        1.0 / 3.0,
    );
}

#[test]
fn falloff_is_integrated_once_and_zero_length_has_no_obstruction() {
    let shape = RegionShape::Sphere { radius: 10.0 };
    let mut profile = profile();
    profile.falloff = FieldFalloff::Linear;
    close(
        signal_transmission(
            vec![volume("cloud", &shape, &profile)],
            -Vec3::X * 20.0,
            Vec3::X * 20.0,
            SignalClass::Sensors,
        ),
        1.0 / 3.0,
    );
    assert_eq!(
        signal_transmission(
            vec![volume("cloud", &shape, &profile)],
            Vec3::ZERO,
            Vec3::ZERO,
            SignalClass::Sensors
        ),
        1.0
    );
    assert_eq!(
        signal_transmission(vec![], -Vec3::X, Vec3::X, SignalClass::Sensors),
        1.0
    );
}

#[test]
fn very_long_links_keep_thin_cloud_and_opaque_intersections() {
    let shape = RegionShape::Sphere { radius: 0.1 };
    let mut profile = profile();
    let (a, b) = (-Vec3::X * 100_000_000.0, Vec3::X * 100_000_000.0);
    close(
        signal_transmission(
            vec![volume("cloud", &shape, &profile)],
            a,
            b,
            SignalClass::Sensors,
        ),
        1.0 / 1.04,
    );
    profile.falloff = FieldFalloff::Linear;
    close(
        signal_transmission(
            vec![volume("cloud", &shape, &profile)],
            a,
            b,
            SignalClass::Sensors,
        ),
        1.0 / 1.02,
    );
    let material = SignalMaterialConfig {
        sensor_absorption_per_unit: 0.0,
        comms_absorption_per_unit: 0.0,
        sensor_opaque: true,
        comms_opaque: true,
    };
    let mut terrain = volume("opaque", &shape, &profile);
    terrain.material = Some(&material);
    assert_eq!(
        signal_transmission(vec![terrain], a, b, SignalClass::Sensors),
        0.0
    );
}
