use super::*;

fn toml_config(body: &str) -> Result<ReferenceGridConfig, toml::de::Error> {
    toml::from_str(body)
}

// ── Parsing ───────────────────────────────────────────────────────────

#[test]
fn an_empty_table_is_the_calibrated_default() {
    let cfg = toml_config("").expect("a bare [reference_grid] is legal");
    assert_eq!(cfg, ReferenceGridConfig::default());
    assert_eq!(cfg.minor_spacing, 10.0);
    assert_eq!(cfg.major_spacing, 50.0);
}

#[test]
fn every_field_is_authorable() {
    let cfg = toml_config(
        r#"
            minor_spacing = 4.0
            major_spacing = 20.0
            minor_colour = [0.1, 0.2, 0.3, 0.4]
            major_colour = [0.5, 0.6, 0.7, 0.8]
            opacity = 0.5
            patch_radius = 200.0
            fade_band = 50.0
            fade_exponent = 3.0
            plane_y = -1.25
            minor_line_width_px = 2.0
            major_line_width_px = 3.0
            "#,
    )
    .expect("parses");
    assert_eq!(cfg.minor_spacing, 4.0);
    assert_eq!(cfg.major_spacing, 20.0);
    assert_eq!(cfg.minor_colour, [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(cfg.major_colour, [0.5, 0.6, 0.7, 0.8]);
    assert_eq!(cfg.opacity, 0.5);
    assert_eq!(cfg.patch_radius, 200.0);
    assert_eq!(cfg.fade_band, 50.0);
    assert_eq!(cfg.fade_exponent, 3.0);
    assert_eq!(cfg.plane_y, -1.25);
    assert_eq!(cfg.minor_line_width_px, 2.0);
    assert_eq!(cfg.major_line_width_px, 3.0);
}

#[test]
fn an_unknown_field_is_refused() {
    let err = toml_config("minor_spacing = 10.0\nminor_spaceing = 5.0\n")
        .expect_err("deny_unknown_fields catches the typo");
    assert!(
        err.to_string().contains("minor_spaceing"),
        "the error should name the offending key, got: {err}"
    );
}

#[test]
fn the_default_table_round_trips_through_toml() {
    let encoded = toml::to_string(&ReferenceGridConfig::default()).expect("encodes");
    let decoded: ReferenceGridConfig = toml::from_str(&encoded).expect("re-parses");
    assert_eq!(decoded, ReferenceGridConfig::default());
}

// ── Validation ────────────────────────────────────────────────────────

#[test]
fn the_shipped_defaults_validate() {
    ReferenceGridConfig::default()
        .validate()
        .expect("the defaults are what a bare table gets, so they must be legal");
}

#[test]
fn a_zero_or_negative_minor_spacing_is_refused() {
    for bad in [0.0, -10.0] {
        let cfg = ReferenceGridConfig {
            minor_spacing: bad,
            ..Default::default()
        };
        let err = cfg
            .validate()
            .expect_err("a zero or negative spacing has no lines to draw");
        assert!(err.contains("minor_spacing"), "spacing {bad}: got {err}");
    }
}

#[test]
fn a_zero_major_spacing_is_refused() {
    let cfg = ReferenceGridConfig {
        major_spacing: 0.0,
        ..Default::default()
    };
    let err = cfg.validate().expect_err("refused");
    assert!(err.contains("major_spacing"), "got: {err}");
}

#[test]
fn a_non_finite_spacing_is_refused() {
    for bad in [f32::NAN, f32::INFINITY] {
        let cfg = ReferenceGridConfig {
            minor_spacing: bad,
            ..Default::default()
        };
        assert!(cfg.validate().is_err(), "{bad} must not pass validation");
    }
}

#[test]
fn a_major_spacing_that_is_not_a_whole_multiple_of_minor_is_refused() {
    let cfg = ReferenceGridConfig {
        minor_spacing: 10.0,
        major_spacing: 35.0,
        ..Default::default()
    };
    let err = cfg.validate().expect_err("3.5 cells is not a lattice");
    assert!(err.contains("whole multiple"), "got: {err}");
}

#[test]
fn a_major_spacing_finer_than_minor_is_refused() {
    let cfg = ReferenceGridConfig {
        minor_spacing: 50.0,
        major_spacing: 10.0,
        ..Default::default()
    };
    let err = cfg
        .validate()
        .expect_err("the major lattice is the coarse one");
    assert!(err.contains("finer"), "got: {err}");
}

#[test]
fn a_major_spacing_equal_to_minor_is_accepted() {
    // One cell per major line is a degenerate but coherent grid: every line
    // is a major line. Nothing about it is unreadable, so it is not the
    // validator's business to refuse it.
    let cfg = ReferenceGridConfig {
        minor_spacing: 10.0,
        major_spacing: 10.0,
        ..Default::default()
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn a_fade_band_wider_than_the_patch_is_refused() {
    let cfg = ReferenceGridConfig {
        patch_radius: 100.0,
        fade_band: 150.0,
        ..Default::default()
    };
    let err = cfg.validate().expect_err("refused");
    assert!(err.contains("fade_band"), "got: {err}");
}

#[test]
fn a_zero_patch_radius_is_refused() {
    let cfg = ReferenceGridConfig {
        patch_radius: 0.0,
        fade_band: 0.0,
        ..Default::default()
    };
    let err = cfg.validate().expect_err("refused");
    assert!(err.contains("patch_radius"), "got: {err}");
}

#[test]
fn an_out_of_range_opacity_is_refused() {
    for bad in [-0.1, 1.5] {
        let cfg = ReferenceGridConfig {
            opacity: bad,
            ..Default::default()
        };
        assert!(cfg.validate().is_err(), "opacity {bad} must be refused");
    }
}

#[test]
fn an_over_range_colour_component_is_refused() {
    let cfg = ReferenceGridConfig {
        minor_colour: [1.0, 0.5, 0.5, 4.0],
        ..Default::default()
    };
    let err = cfg
        .validate()
        .expect_err("an HDR-hot grid line would bloom");
    assert!(err.contains("minor_colour"), "got: {err}");
}

#[test]
fn a_zero_line_width_is_refused() {
    let cfg = ReferenceGridConfig {
        minor_line_width_px: 0.0,
        ..Default::default()
    };
    let err = cfg.validate().expect_err("refused");
    assert!(err.contains("minor_line_width_px"), "got: {err}");
}

// ── Derived uniform values ────────────────────────────────────────────

#[test]
fn fade_start_is_the_radius_less_the_band() {
    let cfg = ReferenceGridConfig {
        patch_radius: 400.0,
        fade_band: 150.0,
        ..Default::default()
    };
    assert_eq!(cfg.fade_start(), 250.0);
    assert_eq!(cfg.fade_span(), 150.0);
    assert_eq!(cfg.patch_half_size(), 400.0);
}

#[test]
fn a_zero_fade_band_leaves_a_hard_edge_the_shader_can_still_divide_by() {
    let cfg = ReferenceGridConfig {
        patch_radius: 400.0,
        fade_band: 0.0,
        ..Default::default()
    };
    assert_eq!(cfg.fade_start(), 400.0);
    assert!(
        cfg.fade_span() > 0.0,
        "the span is what the shader divides by; it must never be zero"
    );
    assert_eq!(
        radial_fade(399.0, cfg.fade_start(), cfg.fade_span(), 1.0),
        1.0
    );
    assert_eq!(
        radial_fade(400.0, cfg.fade_start(), cfg.fade_span(), 1.0),
        0.0
    );
}

// ── Lattice maths ─────────────────────────────────────────────────────

#[test]
fn a_coordinate_on_a_line_is_zero_distance_from_it() {
    for coord in [0.0, 10.0, -10.0, 250.0, -1230.0] {
        assert_eq!(
            distance_to_nearest_line(coord, 10.0),
            0.0,
            "{coord} is a multiple of 10 and so sits on a line"
        );
    }
}

#[test]
fn the_lattice_is_world_locked_not_ship_locked() {
    // The property the whole feature rests on. The function takes NO patch
    // centre and no ship position — a world coordinate alone decides how
    // far it is from a line, so wherever the patch is dragged the lines
    // stay put underneath it and the ship is what appears to move.
    //
    // Checked against hand-computed answers rather than against itself: at
    // 10-unit spacing, 1234.5 is 4.5 past the line at 1230, and 1237.0 is
    // 3.0 short of the line at 1240.
    for (coord, expected) in [
        (1234.5_f32, 4.5_f32),
        (1237.0, 3.0),
        (-1234.5, 4.5),
        (-1237.0, 3.0),
        (0.0, 0.0),
    ] {
        let actual = distance_to_nearest_line(coord, 10.0);
        assert!(
            (actual - expected).abs() < 1.0e-3,
            "at world {coord} expected {expected} from the nearest line, got {actual}"
        );
    }
}

#[test]
fn distance_never_exceeds_half_a_cell() {
    let spacing = 10.0_f32;
    let mut coord = -37.0_f32;
    while coord < 37.0 {
        let d = distance_to_nearest_line(coord, spacing);
        assert!(
            (0.0..=spacing / 2.0 + 1.0e-4).contains(&d),
            "distance {d} out of range at {coord}"
        );
        coord += 0.37;
    }
}

#[test]
fn the_major_lattice_lands_on_minor_lines() {
    // Why validation insists on a whole multiple: every major line has to
    // be a minor line too, or the grid shows paired lines a fraction apart.
    let cfg = ReferenceGridConfig::default();
    let mut major = 0.0_f32;
    while major <= 500.0 {
        assert_eq!(
            distance_to_nearest_line(major, cfg.minor_spacing),
            0.0,
            "major line at {major} does not sit on a minor line"
        );
        major += cfg.major_spacing;
    }
}

#[test]
fn coverage_is_full_on_the_line_and_gone_a_width_away() {
    let world_per_px = 0.5;
    let half_width_px = 1.0;
    assert_eq!(line_coverage(0.0, half_width_px, world_per_px), 1.0);
    // One pixel away in world units is 0.5, and the half-width is one pixel.
    assert_eq!(line_coverage(0.5, half_width_px, world_per_px), 0.0);
    // Half a pixel away is half covered — the antialiasing ramp.
    assert!((line_coverage(0.25, half_width_px, world_per_px) - 0.5).abs() < 1.0e-6);
}

#[test]
fn coverage_stays_bounded_for_a_degenerate_fragment() {
    // A fragment at a grazing angle can report a huge or zero derivative.
    // Neither may produce a NaN that propagates into the blend.
    assert_eq!(line_coverage(1.0, 1.0, 0.0), 0.0);
    assert_eq!(line_coverage(1.0, 0.0, 1.0), 0.0);
    assert_eq!(line_coverage(0.0, 1.0, 1.0e9), 1.0);
}

#[test]
fn a_line_seen_nearly_edge_on_dims_rather_than_aliasing() {
    // As one pixel comes to span more world units, a line 2 units away
    // falls inside fewer pixel widths and so covers MORE of the fragment,
    // rising smoothly to a uniform tint instead of breaking into a moiré
    // pattern. That monotonic climb is the whole antialiasing claim.
    let mut previous = 0.0_f32;
    for world_per_px in [0.1_f32, 0.5, 1.0, 4.0, 20.0] {
        let coverage = line_coverage(2.0, 1.0, world_per_px);
        assert!(
            coverage >= previous - 1.0e-6,
            "coverage fell from {previous} to {coverage} at {world_per_px} world/px"
        );
        assert!(
            (0.0..=1.0).contains(&coverage),
            "coverage {coverage} out of range"
        );
        previous = coverage;
    }
    // The extremes. Up close one pixel spans 0.1 units, so the line is 20
    // pixels away and nowhere near this fragment: nothing drawn. Far off
    // one pixel spans 20 units, so the line is 0.1 of a pixel from centre
    // and covers all but a tenth of the half width.
    assert_eq!(line_coverage(2.0, 1.0, 0.1), 0.0);
    assert!((line_coverage(2.0, 1.0, 20.0) - 0.9).abs() < 1.0e-6);
}

#[test]
fn the_radial_fade_is_full_inside_and_gone_outside() {
    let fade_start = 250.0;
    let fade_span = 150.0;
    assert_eq!(radial_fade(0.0, fade_start, fade_span, 1.0), 1.0);
    assert_eq!(radial_fade(250.0, fade_start, fade_span, 1.0), 1.0);
    assert_eq!(radial_fade(400.0, fade_start, fade_span, 1.0), 0.0);
    assert_eq!(radial_fade(10_000.0, fade_start, fade_span, 1.0), 0.0);
}

#[test]
fn the_radial_fade_is_monotonic_across_the_band() {
    let (fade_start, fade_span) = (250.0_f32, 150.0_f32);
    let mut previous = 1.0_f32;
    let mut distance = 250.0_f32;
    while distance <= 400.0 {
        let fade = radial_fade(distance, fade_start, fade_span, 1.0);
        assert!(
            fade <= previous + 1.0e-6,
            "fade rose from {previous} to {fade} at {distance}"
        );
        assert!((0.0..=1.0).contains(&fade), "fade {fade} out of range");
        previous = fade;
        distance += 5.0;
    }
}

#[test]
fn the_fade_is_half_way_through_at_the_middle_of_the_band() {
    // smoothstep's midpoint. Asserted because it is the one point on the
    // curve a re-implementation is most likely to get subtly wrong. At the
    // neutral exponent of 1.0 the fade is the bare smoothstep.
    assert!((radial_fade(325.0, 250.0, 150.0, 1.0) - 0.5).abs() < 1.0e-6);
}

#[test]
// The expected value mirrors radial_fade's own std powf — this is
// presentation math, never a digest input (see radial_fade's note).
#[allow(clippy::disallowed_methods)]
fn a_higher_fade_exponent_dissolves_the_band_faster() {
    // The steepening knob John asked for: at any interior point the fade is
    // strictly dimmer for a larger exponent, while the endpoints stay
    // pinned at full and nothing — so the grid dissolves harder toward the
    // rim without ever lighting a bright far edge.
    let (start, span) = (250.0_f32, 150.0_f32);
    let mid = 325.0_f32; // smoothstep = 0.5 here
    let plain = radial_fade(mid, start, span, 1.0);
    let steep = radial_fade(mid, start, span, 2.5);
    assert!(
        steep < plain,
        "exponent 2.5 must dim the mid-band below plain"
    );
    assert!((steep - 0.5_f32.powf(2.5)).abs() < 1.0e-6);
    // Endpoints are exponent-invariant.
    assert_eq!(radial_fade(start, start, span, 2.5), 1.0);
    assert_eq!(radial_fade(start + span, start, span, 2.5), 0.0);
}
