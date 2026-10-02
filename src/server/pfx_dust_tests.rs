use super::*;

#[test]
fn pack_texture_refresh_retires_dust_without_removing_unrelated_pfx() {
    let mut world = World::new();
    world.insert_resource(DustFieldState::default());
    let mote = world
        .spawn(DustMote {
            kind: DustMoteKind::Layer(0),
            width: 1.0,
            length_scale: 1.0,
            turbulence: Vec3::ZERO,
        })
        .id();
    let unrelated = world.spawn(PfxEntity).id();
    world.resource_mut::<DustFieldState>().spawn_s = 1.0;
    reset_pack_textures(&mut world);
    assert!(world.get_entity(mote).is_err());
    assert!(world.get_entity(unrelated).is_ok());
    assert_eq!(world.resource::<DustFieldState>().spawn_s, 0.0);
}

fn physics(yaw: f32, forward: f32, lateral: f32) -> ShipPhysics {
    ShipPhysics {
        x: 0.0,
        z: 0.0,
        yaw,
        forward_speed: forward,
        roll: 0.0,
        lateral_speed: lateral,
        ..Default::default()
    }
}

fn settings() -> DustPfxSettings {
    DustPfxSettings::from_world(None)
}

// --- ship_velocity -----------------------------------------------------

#[test]
fn ship_velocity_at_zero_yaw_points_down_negative_z() {
    let v = ship_velocity(&physics(0.0, 10.0, 0.0));
    assert!(
        (v - Vec3::new(0.0, 0.0, -10.0)).length() < 1e-4,
        "got {v:?}"
    );
}

#[test]
fn ship_velocity_follows_yaw() {
    // Yawed 90° to starboard, forward should be +X.
    let v = ship_velocity(&physics(std::f32::consts::FRAC_PI_2, 10.0, 0.0));
    assert!((v - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-3, "got {v:?}");
}

/// Regression: dust ignored `lateral_speed` entirely, so the field froze
/// while the ship strafed. Pure lateral motion must produce pure lateral
/// velocity — at yaw 0, starboard is +X.
#[test]
fn ship_velocity_reacts_to_pure_strafe() {
    let v = ship_velocity(&physics(0.0, 0.0, 7.0));
    assert!(
        v.length() > 0.0,
        "strafing with no forward speed must still yield velocity"
    );
    assert!((v - Vec3::new(7.0, 0.0, 0.0)).length() < 1e-4, "got {v:?}");
}

#[test]
fn ship_velocity_combines_forward_and_lateral() {
    let v = ship_velocity(&physics(0.0, 3.0, 4.0));
    // Forward -Z and starboard +X are perpendicular, so the magnitude is
    // the hypotenuse rather than either component.
    assert!((v.length() - 5.0).abs() < 1e-4, "got {}", v.length());
    assert!((v - Vec3::new(4.0, 0.0, -3.0)).length() < 1e-4, "got {v:?}");
}

#[test]
fn ship_velocity_reverse_flips_direction() {
    let v = ship_velocity(&physics(0.0, -6.0, 0.0));
    assert!((v - Vec3::new(0.0, 0.0, 6.0)).length() < 1e-4, "got {v:?}");
}

#[test]
fn ship_velocity_has_no_vertical_component() {
    // ShipPhysics is an XZ model; a Y drift would be motion the ship is
    // not making.
    let v = ship_velocity(&physics(0.9, 8.0, -3.0));
    assert_eq!(v.y, 0.0);
}

// --- billboard orientation --------------------------------------------

#[test]
fn billboard_aligns_local_x_with_direction_of_travel() {
    let travel = Vec3::new(0.0, 0.0, -1.0);
    let to_cam = Vec3::new(0.0, 1.0, 0.0);
    let rot = dust_billboard_rotation(travel, to_cam);
    let local_x = rot * Vec3::X;
    assert!(
        (local_x - travel).length() < 1e-4,
        "quad's long axis must follow travel, got {local_x:?}"
    );
}

#[test]
fn billboard_faces_camera_as_closely_as_possible() {
    let travel = Vec3::new(0.0, 0.0, -1.0);
    let to_cam = Vec3::new(0.0, 1.0, 0.0);
    let rot = dust_billboard_rotation(travel, to_cam);
    let normal = rot * Vec3::Z;
    // to_cam is already perpendicular to travel, so the quad can face it
    // exactly.
    assert!(
        (normal - to_cam).length() < 1e-4,
        "quad normal should point at the camera, got {normal:?}"
    );
}

#[test]
fn billboard_basis_stays_orthonormal() {
    let rot = dust_billboard_rotation(Vec3::new(1.0, 0.0, -2.0), Vec3::new(0.3, 0.9, 0.1));
    let (x, y, z) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
    assert!(x.dot(y).abs() < 1e-4);
    assert!(x.dot(z).abs() < 1e-4);
    assert!(y.dot(z).abs() < 1e-4);
    assert!((x.length() - 1.0).abs() < 1e-4);
}

/// A mote heading straight at the camera has no projected direction. The
/// Gram-Schmidt projection collapses to zero there, so this must fall back
/// rather than produce a NaN rotation — and it is the common case when
/// flying forward, not an edge case.
#[test]
fn billboard_degenerate_head_on_case_is_finite() {
    let travel = Vec3::new(0.0, 0.0, -1.0);
    let rot = dust_billboard_rotation(travel, travel);
    assert!(
        rot.is_finite(),
        "head-on mote produced a non-finite rotation"
    );
    let local_x = rot * Vec3::X;
    assert!(
        (local_x - travel).length() < 1e-4,
        "fallback must still align with travel, got {local_x:?}"
    );
}

#[test]
fn billboard_zero_velocity_is_identity_not_nan() {
    let rot = dust_billboard_rotation(Vec3::ZERO, Vec3::Y);
    assert!(rot.is_finite());
}

// --- speed curves and smoothing ---------------------------------------

#[test]
fn dust_ramp_interpolates_between_rest_and_full_speed() {
    assert_eq!(dust_ramp([2.0, 10.0], 0.0), 2.0);
    assert_eq!(dust_ramp([2.0, 10.0], 1.0), 10.0);
    assert_eq!(dust_ramp([2.0, 10.0], 0.5), 6.0);
}

#[test]
fn speed_curve_keeps_the_effect_restrained_at_low_speed() {
    // Half speed through an S² curve should land well under half strength,
    // which is the whole point of the exponent (spec §2).
    let half = dust_speed_fraction(&physics(0.0, 6.25, 0.0), None, 2.0);
    assert!((half - 0.25).abs() < 1e-3, "got {half}");
}

#[test]
fn speed_curve_uses_true_velocity_not_just_forward_speed() {
    let forward_only = dust_speed_fraction(&physics(0.0, 3.0, 0.0), None, 1.0);
    let with_strafe = dust_speed_fraction(&physics(0.0, 3.0, 4.0), None, 1.0);
    assert!(
        with_strafe > forward_only,
        "strafing must raise the speed fraction ({with_strafe} vs {forward_only})"
    );
}

#[test]
fn speed_fraction_clamps_at_full_speed() {
    let s = dust_speed_fraction(&physics(0.0, 999.0, 0.0), None, 2.0);
    assert!((s - 1.0).abs() < 1e-6, "got {s}");
}

#[test]
fn dust_smooth_converges_toward_target() {
    let mut v = 0.0;
    for _ in 0..200 {
        v = dust_smooth(v, 1.0, 0.1, 0.016);
    }
    assert!((v - 1.0).abs() < 1e-3, "got {v}");
}

#[test]
fn dust_smooth_zero_response_snaps() {
    assert_eq!(dust_smooth(0.0, 1.0, 0.0, 0.016), 1.0);
}

/// Spec §10: streak length should lead, brightness follow, density lag.
/// That ordering is what makes acceleration feel immediate without motes
/// visibly popping into existence.
#[test]
fn response_rates_stagger_streak_then_brightness_then_density() {
    let cfg = settings();
    let dt = 0.1;
    let streak = dust_smooth(0.0, 1.0, cfg.streak_response_secs, dt);
    let brightness = dust_smooth(0.0, 1.0, cfg.brightness_response_secs, dt);
    let spawn = dust_smooth(0.0, 1.0, cfg.spawn_response_secs, dt);
    assert!(
        streak > brightness && brightness > spawn,
        "expected streak > brightness > spawn, got {streak} / {brightness} / {spawn}"
    );
}

// --- tint --------------------------------------------------------------

#[test]
fn tint_runs_cool_grey_blue_to_near_white() {
    let cfg = settings();
    let at_rest = dust_tint(&cfg, 0.0);
    let at_speed = dust_tint(&cfg, 1.0);
    assert_eq!(at_rest, cfg.low_speed_tint);
    assert_eq!(at_speed, cfg.high_speed_tint);
    // "Whiter when fast" means the channels converge, not just brighten.
    let spread = |c: [f32; 3]| {
        c.iter().cloned().fold(f32::MIN, f32::max) - c.iter().cloned().fold(f32::MAX, f32::min)
    };
    assert!(
        spread(at_speed) < spread(at_rest),
        "high-speed tint should be less saturated than the low-speed tint"
    );
}

// --- spawn budget ------------------------------------------------------

#[test]
fn spawn_budget_accumulates_fractional_motes() {
    let mut acc = 0.0;
    // 10/sec for 0.05s = 0.5 motes — nothing yet.
    assert_eq!(dust_take_spawn_budget(&mut acc, 10.0, 100, 0.05), 0);
    // Another 0.05s tips it over 1.0.
    assert_eq!(dust_take_spawn_budget(&mut acc, 10.0, 100, 0.05), 1);
}

/// Without the at-cap clamp the accumulator grows while every slot is
/// occupied, then discharges as a burst the moment slots free up.
#[test]
fn spawn_budget_does_not_bank_motes_while_at_cap() {
    let mut acc = 0.0;
    for _ in 0..100 {
        assert_eq!(dust_take_spawn_budget(&mut acc, 200.0, 0, 0.016), 0);
    }
    assert!(
        acc <= 200.0 * 0.016 + 1e-6,
        "accumulator banked up to {acc}"
    );
}

// --- edge bias ---------------------------------------------------------

#[test]
fn edge_bias_pushes_samples_toward_the_screen_edges() {
    // Near layer weights spawns to the edges so big close streaks stay
    // peripheral (spec §13).
    let biased = dust_edge_shape(0.5, 0.7);
    assert!(biased > 0.5, "expected outward push, got {biased}");
    assert!(biased <= 1.0);
}

#[test]
fn edge_bias_zero_is_uniform_and_preserves_sign() {
    assert!((dust_edge_shape(0.5, 0.0) - 0.5).abs() < 1e-5);
    assert!((dust_edge_shape(-0.5, 0.0) + 0.5).abs() < 1e-5);
}

#[test]
fn edge_bias_negative_pulls_samples_toward_the_centre() {
    assert!(dust_edge_shape(0.5, -1.0) < 0.5);
}

// --- screen-relative sizing --------------------------------------------

/// Layer widths are fractions of the screen, not world units. Treating
/// them as world units makes the far layer (40–150 units out, width 0.006)
/// sub-pixel and invisible, which is exactly what happened first time.
#[test]
fn view_extent_grows_with_depth() {
    let fov = std::f32::consts::FRAC_PI_4;
    let near = dust_view_min_extent_at(10.0, fov, 16.0 / 9.0);
    let far = dust_view_min_extent_at(100.0, fov, 16.0 / 9.0);
    assert!(
        (far / near - 10.0).abs() < 1e-3,
        "should scale linearly with depth"
    );
}

/// The bug: `fov` is Bevy's *vertical* FOV, so sizing off it alone shrank
/// motes on a wide, short viewport even though the screen had grown. Motes
/// must track whichever screen dimension is the tighter constraint.
#[test]
fn view_extent_tracks_the_smaller_screen_dimension() {
    let fov = std::f32::consts::FRAC_PI_4;
    let depth = 30.0;
    let square = dust_view_min_extent_at(depth, fov, 1.0);

    // Landscape: height is the smaller dimension, so widening the viewport
    // must not change mote size at all.
    assert!(
        (dust_view_min_extent_at(depth, fov, 16.0 / 9.0) - square).abs() < 1e-4,
        "a wider-than-tall viewport is still height-constrained"
    );
    assert!(
        (dust_view_min_extent_at(depth, fov, 4.0) - square).abs() < 1e-4,
        "an extremely wide viewport is still height-constrained"
    );

    // Portrait: width is now the smaller dimension, so motes shrink with it
    // rather than bloating to the taller height.
    let portrait = dust_view_min_extent_at(depth, fov, 0.5);
    assert!(
        portrait < square,
        "a taller-than-wide viewport must size off its width, got {portrait} vs {square}"
    );
    assert!((portrait / square - 0.5).abs() < 1e-4, "and do so linearly");
}

#[test]
fn screen_relative_width_holds_apparent_size_across_depth_bands() {
    let fov = std::f32::consts::FRAC_PI_4;
    let aspect = 16.0 / 9.0;
    let frac = 0.02;
    // Two motes of the same authored width at very different depths must
    // subtend the same fraction of the view.
    let near_world = frac * dust_view_min_extent_at(10.0, fov, aspect);
    let far_world = frac * dust_view_min_extent_at(120.0, fov, aspect);
    assert!(
        far_world > near_world,
        "a deeper mote needs more world width"
    );
    let apparent = |w: f32, d: f32| w / dust_view_min_extent_at(d, fov, aspect);
    assert!(
        (apparent(near_world, 10.0) - apparent(far_world, 120.0)).abs() < 1e-6,
        "apparent size must not depend on depth"
    );
}

#[test]
fn builtin_widths_are_screen_fractions_not_world_units() {
    let cfg = settings();
    for layer in &cfg.layers {
        assert!(
            layer.width > 0.0 && layer.width < 0.5,
            "width {} does not read as a screen fraction",
            layer.width
        );
    }
}

// --- lifetime ----------------------------------------------------------

/// A mote must live long enough to actually transit the volume and pass the
/// camera. A fixed lifetime kills fast motes while they are still distant
/// specks, so the field never reads as streaming past you.
#[test]
fn lifetime_covers_transit_to_behind_the_camera() {
    // 100 units out, closing at 50/s → ~2.1s to reach 5 units behind.
    let life = dust_lifetime(100.0, 50.0, 10.0, 1.0);
    assert!((life - 2.1).abs() < 1e-3, "got {life}");
}

#[test]
fn lifetime_shortens_as_speed_rises() {
    let slow = dust_lifetime(100.0, 10.0, 100.0, 1.0);
    let fast = dust_lifetime(100.0, 100.0, 100.0, 1.0);
    assert!(fast < slow, "faster motes should transit sooner");
}

#[test]
fn lifetime_is_capped_so_slow_motes_do_not_hang() {
    // Crawling: transit would be ~1000s. The cap is what stops motes
    // hanging in space looking like snow.
    let life = dust_lifetime(100.0, 0.1, 2.0, 1.0);
    assert_eq!(life, 2.0);
}

#[test]
fn lifetime_at_zero_speed_falls_back_to_the_cap() {
    assert_eq!(dust_lifetime(50.0, 0.0, 3.0, 1.0), 3.0);
}

#[test]
fn lifetime_never_returns_zero() {
    assert!(dust_lifetime(0.0, 1e6, 5.0, 1.0) >= 0.05);
}

// --- config resolution -------------------------------------------------

#[test]
fn settings_without_world_config_use_builtin_layers() {
    let cfg = settings();
    assert!(cfg.enabled);
    assert_eq!(cfg.layers.len(), 3);
    assert_eq!(cfg.speed_curve_exponent, DUST_SPEED_CURVE_EXPONENT);
    // Warp is opt-in: absent [dust.warp] means no warp field.
    assert!(!cfg.warp.enabled);
}

#[test]
fn builtin_layers_run_near_to_far() {
    let cfg = settings();
    let depths: Vec<f32> = cfg.layers.iter().map(|l| l.depth_band[0]).collect();
    assert!(
        depths.windows(2).all(|w| w[0] < w[1]),
        "layers should be ordered near→far, got {depths:?}"
    );
    // Far motes must stay below the bloom threshold or the scene fogs.
    let far = cfg.layers.last().expect("three layers");
    let near = cfg.layers.first().expect("three layers");
    assert!(far.brightness[1] < near.brightness[1]);
    assert!(!far.additive, "far layer should alpha-blend, not add");
    assert!(near.additive, "near layer should be additive");
}

#[test]
fn world_config_overrides_layers_positionally() {
    use crate::world::config::{DustLayerConfig, DustPfxConfig};
    let world = crate::world::config::WorldConfig {
        dust: Some(DustPfxConfig {
            layers: vec![DustLayerConfig {
                max_motes: Some(7),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg = DustPfxSettings::from_world(Some(&world));
    assert_eq!(cfg.layers.len(), 1);
    assert_eq!(cfg.layers[0].max_motes, 7);
    // Unset fields fall back to the matching built-in layer.
    assert_eq!(cfg.layers[0].texture, DUST_DEFAULT_LAYERS[0].texture);
    assert_eq!(cfg.layers[0].width, DUST_DEFAULT_LAYERS[0].width);
}

#[test]
fn world_config_overrides_scalars() {
    use crate::world::config::DustPfxConfig;
    let world = crate::world::config::WorldConfig {
        dust: Some(DustPfxConfig {
            enabled: Some(false),
            turbulence: Some(0.5),
            low_speed_tint: Some([0.1, 0.2, 0.3]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg = DustPfxSettings::from_world(Some(&world));
    assert!(!cfg.enabled);
    assert_eq!(cfg.turbulence, 0.5);
    assert_eq!(cfg.low_speed_tint, [0.1, 0.2, 0.3]);
    // Untouched fields keep their defaults.
    assert_eq!(cfg.high_speed_tint, DUST_HIGH_SPEED_TINT);
}

#[test]
fn empty_dust_block_keeps_builtin_layers() {
    let world = crate::world::config::WorldConfig {
        dust: Some(Default::default()),
        ..Default::default()
    };
    let cfg = DustPfxSettings::from_world(Some(&world));
    assert_eq!(cfg.layers.len(), 3);
}

// --- quad geometry -----------------------------------------------------

/// `space_mote_streak_head.png` carries its bright head at the low-U end,
/// and the billboard aligns local +X with travel. The quad's UVs are
/// therefore mirrored so the head leads; if this flips, every near streak
/// trails head-first and the field reads as moving backwards.
#[test]
fn quad_uvs_put_low_u_at_positive_x_so_streak_heads_lead() {
    let mesh = dust_quad_mesh();
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
        _ => panic!("quad must have Float32x3 positions"),
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(bevy::mesh::VertexAttributeValues::Float32x2(u)) => u.clone(),
        _ => panic!("quad must have Float32x2 UVs"),
    };
    assert_eq!(positions.len(), 4);

    let u_at_max_x = positions
        .iter()
        .zip(&uvs)
        .filter(|(p, _)| p[0] > 0.0)
        .map(|(_, uv)| uv[0])
        .collect::<Vec<_>>();
    let u_at_min_x = positions
        .iter()
        .zip(&uvs)
        .filter(|(p, _)| p[0] < 0.0)
        .map(|(_, uv)| uv[0])
        .collect::<Vec<_>>();

    assert!(
        u_at_max_x.iter().all(|&u| u == 0.0),
        "leading (+X) edge must sample u=0 where the streak head lives, got {u_at_max_x:?}"
    );
    assert!(
        u_at_min_x.iter().all(|&u| u == 1.0),
        "trailing (-X) edge must sample u=1 (the tail), got {u_at_min_x:?}"
    );
}
