use super::*;
use std::f32::consts::{FRAC_PI_2, PI};

fn approx(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a - b).length() <= eps
}

fn pose(t: Vec3, yaw: f32) -> Pose {
    Pose {
        translation: t,
        rotation: Quat::from_rotation_y(yaw),
    }
}

/// A marker on the own ship's nose (local `-Z`) and one on a target's nose,
/// with the two hulls facing each other head-on, mates at the shared marker
/// point with the own ship turned to oppose the target's plate.
#[test]
fn a_single_pair_mates_marker_to_marker_facing_the_target() {
    // Own ship at origin facing -Z, dock marker 5 units ahead facing out.
    let own = pose(Vec3::ZERO, 0.0);
    let own_markers = [DockMarker {
        position: Vec3::new(0.0, 0.0, -5.0),
        direction: Vec3::new(0.0, 0.0, -1.0),
    }];
    // Target 100 ahead on -Z, facing back toward the own ship (+Z means its
    // own -Z forward points at us when yawed 180°). Author its marker on its
    // own nose facing out.
    let target = pose(Vec3::new(0.0, 0.0, -100.0), PI);
    let target_markers = [DockMarker {
        position: Vec3::new(0.0, 0.0, -5.0),
        direction: Vec3::new(0.0, 0.0, -1.0),
    }];

    let sol = nearest_viable_pair(own, &own_markers, target, &target_markers)
        .expect("one viable pair mates");
    assert_eq!((sol.own_marker, sol.target_marker), (0, 0));

    // The target marker resolves to world: target at (0,0,-100) yawed 180°,
    // marker local (0,0,-5) → world (0,0,-95).
    let target_marker_world = Vec3::new(0.0, 0.0, -95.0);
    // At the mate pose the own marker must land exactly on the target
    // marker's world point.
    let own_marker_world = sol.mate.translation + sol.mate.rotation * own_markers[0].position;
    assert!(
        approx(own_marker_world, target_marker_world, 1e-3),
        "own marker mates onto the target marker point, got {own_marker_world:?}"
    );
    // And the own marker's world direction opposes the target marker's.
    let own_dir_world = sol.mate.rotation * own_markers[0].direction;
    let target_dir_world = target.rotation * target_markers[0].direction;
    assert!(
        approx(own_dir_world, -target_dir_world, 1e-3),
        "the mated plates face each other: own {own_dir_world:?} vs target {target_dir_world:?}"
    );
}

/// With MULTIPLE candidate markers on each hull, the nearest pair by current
/// world separation is chosen, and the mate transform mates THAT pair.
#[test]
fn nearest_pair_is_chosen_among_many_candidates() {
    // Own ship at origin, unrotated, two dock markers: one to port (-X) and
    // one to starboard (+X), each facing outward along its axis.
    let own = pose(Vec3::ZERO, 0.0);
    let own_markers = [
        DockMarker {
            position: Vec3::new(-10.0, 0.0, 0.0),
            direction: Vec3::new(-1.0, 0.0, 0.0),
        },
        DockMarker {
            position: Vec3::new(10.0, 0.0, 0.0),
            direction: Vec3::new(1.0, 0.0, 0.0),
        },
    ];
    // Target sits far out on +X, so its markers are nearest the own ship's
    // STARBOARD marker (index 1). The target has two markers too.
    let target = pose(Vec3::new(200.0, 0.0, 0.0), 0.0);
    let target_markers = [
        DockMarker {
            position: Vec3::new(-10.0, 0.0, 0.0), // world x = 190, nearest
            direction: Vec3::new(-1.0, 0.0, 0.0),
        },
        DockMarker {
            position: Vec3::new(10.0, 0.0, 0.0), // world x = 210, farther
            direction: Vec3::new(1.0, 0.0, 0.0),
        },
    ];

    let sol = nearest_viable_pair(own, &own_markers, target, &target_markers)
        .expect("a viable pair exists");
    // Own starboard marker (1) with the target's near marker (0): own marker
    // world x=10, target marker world x=190 → separation 180, the minimum of
    // the four candidate pairs.
    assert_eq!(
        (sol.own_marker, sol.target_marker),
        (1, 0),
        "the nearest candidate pair is chosen"
    );
    assert!(
        (sol.separation - 180.0).abs() < 1e-3,
        "the reported separation is the current marker distance, got {}",
        sol.separation
    );
    // The mate transform mates that exact pair.
    let own_marker_world = sol.mate.translation + sol.mate.rotation * own_markers[1].position;
    let target_marker_world = target.translation + target.rotation * target_markers[0].position;
    assert!(
            approx(own_marker_world, target_marker_world, 1e-3),
            "the chosen pair's markers coincide at the mate: {own_marker_world:?} vs {target_marker_world:?}"
        );
}

/// A hull that declares no dock markers can never be docked with — the module
/// answers `None`, which the adapter turns into the "no markers" refusal.
#[test]
fn no_markers_on_either_hull_yields_no_solution() {
    let own = pose(Vec3::ZERO, 0.0);
    let target = pose(Vec3::new(50.0, 0.0, 0.0), 0.0);
    let markers = [DockMarker {
        position: Vec3::new(0.0, 0.0, -5.0),
        direction: Vec3::new(0.0, 0.0, -1.0),
    }];
    assert!(nearest_viable_pair(own, &[], target, &markers).is_none());
    assert!(nearest_viable_pair(own, &markers, target, &[]).is_none());
    assert!(nearest_viable_pair(own, &[], target, &[]).is_none());
}

/// A marker with no usable horizontal direction (purely vertical) is not
/// viable — a planar mate cannot orient the hull against it — so it is
/// skipped in favour of a viable pair.
#[test]
fn a_purely_vertical_marker_is_not_viable() {
    let own = pose(Vec3::ZERO, 0.0);
    let target = pose(Vec3::new(60.0, 0.0, 0.0), 0.0);
    // Own hull: one vertical (unviable) marker and one horizontal one.
    let own_markers = [
        DockMarker {
            position: Vec3::new(0.0, 5.0, 0.0),
            direction: Vec3::new(0.0, 1.0, 0.0), // vertical → skipped
        },
        DockMarker {
            position: Vec3::new(5.0, 0.0, 0.0),
            direction: Vec3::new(1.0, 0.0, 0.0),
        },
    ];
    let target_markers = [DockMarker {
        position: Vec3::new(-5.0, 0.0, 0.0),
        direction: Vec3::new(-1.0, 0.0, 0.0),
    }];
    let sol = nearest_viable_pair(own, &own_markers, target, &target_markers)
        .expect("the horizontal marker is viable");
    assert_eq!(sol.own_marker, 1, "the vertical marker is skipped");
}

/// The mate pose turns the own ship to whatever yaw makes its plate oppose
/// the target's, regardless of the ship's current heading — docking is a mate,
/// not a fly-past. Here the target's plate faces +X, so the own ship must end
/// facing so its own -X marker opposes it.
#[test]
fn mate_yaw_opposes_the_target_plate_from_any_start_heading() {
    let own_markers = [DockMarker {
        position: Vec3::new(-3.0, 0.0, 0.0),
        direction: Vec3::new(-1.0, 0.0, 0.0),
    }];
    let target = pose(Vec3::new(0.0, 0.0, 0.0), 0.0);
    let target_markers = [DockMarker {
        position: Vec3::new(20.0, 0.0, 0.0),
        direction: Vec3::new(1.0, 0.0, 0.0), // target plate faces +X (world)
    }];
    for start_yaw in [0.0, FRAC_PI_2, PI, -FRAC_PI_2, 0.7] {
        let own = pose(Vec3::new(-50.0, 0.0, 0.0), start_yaw);
        let sol = nearest_viable_pair(own, &own_markers, target, &target_markers)
            .expect("a viable pair exists");
        let own_dir_world = sol.mate.rotation * own_markers[0].direction;
        let target_dir_world = target.rotation * target_markers[0].direction;
        assert!(
            approx(own_dir_world, -target_dir_world, 1e-3),
            "start yaw {start_yaw}: mated plates oppose, own {own_dir_world:?}"
        );
        let own_marker_world = sol.mate.translation + sol.mate.rotation * own_markers[0].position;
        let target_marker_world = target.translation + target.rotation * target_markers[0].position;
        assert!(
            approx(own_marker_world, target_marker_world, 1e-3),
            "start yaw {start_yaw}: markers coincide"
        );
    }
}

// ── Config validation ────────────────────────────────────────────────────

fn good_config() -> DockConfig {
    DockConfig {
        range: 200.0,
        engage_distance: 400.0,
        approach_speed: 60.0,
        mate_tolerance: 4.0,
        undock_clear_distance: 120.0,
        min_power_level: 2,
    }
}

#[test]
fn a_well_formed_dock_config_validates() {
    assert!(good_config().validate().is_ok());
}

#[test]
fn bad_dock_configs_are_rejected() {
    assert!(DockConfig {
        range: 0.0,
        ..good_config()
    }
    .validate()
    .is_err());
    assert!(DockConfig {
        engage_distance: 100.0, // < range
        ..good_config()
    }
    .validate()
    .is_err());
    assert!(DockConfig {
        mate_tolerance: 500.0, // > range
        ..good_config()
    }
    .validate()
    .is_err());
    assert!(DockConfig {
        min_power_level: 0,
        ..good_config()
    }
    .validate()
    .is_err());
    assert!(DockConfig {
        approach_speed: -1.0,
        ..good_config()
    }
    .validate()
    .is_err());
}
