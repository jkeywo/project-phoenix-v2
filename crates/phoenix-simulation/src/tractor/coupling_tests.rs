use super::*;
use std::f32::consts::{FRAC_PI_2, PI};

/// Small helper: the pure module is exercised with plain `glam` values, no
/// Bevy anywhere.
fn approx(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a - b).length() <= eps
}

// ── The coupling-position module across representative offsets ────────────

#[test]
fn a_zero_offset_holds_the_target_exactly_on_the_operator() {
    // The degenerate rig: the load rides the operator itself, whatever the
    // operator's rotation.
    for yaw in [0.0, FRAC_PI_2, PI, -FRAC_PI_2] {
        let op = Vec3::new(10.0, -3.0, 25.0);
        let held = coupled_position(op, Quat::from_rotation_y(yaw), Vec3::ZERO);
        assert!(approx(held, op, 1e-4), "zero offset holds on the operator");
    }
}

#[test]
fn an_offset_on_an_unrotated_operator_is_a_plain_translation() {
    // Identity rotation: the held position is operator + offset, component
    // for component.
    let op = Vec3::new(100.0, 0.0, -50.0);
    let offset = Vec3::new(0.0, 0.0, -120.0);
    let held = coupled_position(op, Quat::IDENTITY, offset);
    assert!(approx(held, Vec3::new(100.0, 0.0, -170.0), 1e-4));
}

#[test]
fn a_quarter_turn_swings_an_astern_offset_onto_the_operators_new_heading() {
    // The rig is 120 units astern (−Z). Yaw the operator 90° about +Y and
    // that astern point rotates in the XZ plane: −Z maps to −X for a
    // positive (counter-clockwise looking down +Y) yaw in glam's frame.
    let op = Vec3::ZERO;
    let offset = Vec3::new(0.0, 0.0, -120.0);
    let held = coupled_position(op, Quat::from_rotation_y(FRAC_PI_2), offset);
    // Whatever the exact axis convention, the load is 120 units from the
    // operator (a rigid rotation preserves length) and no longer on the Z
    // axis it started on.
    assert!(
        (held.length() - 120.0).abs() < 1e-3,
        "a rigid rotation preserves the rig distance: got {}",
        held.length()
    );
    assert!(
        held.z.abs() < 1e-3 && held.x.abs() > 119.0,
        "the astern offset swung onto the operator's beam: {held:?}"
    );
}

#[test]
fn the_rig_translates_with_the_operator_after_it_turns() {
    // Rotation is applied about the operator's own position, then the whole
    // rig is carried to wherever the operator is — the tug-turns-with-its-
    // load property the tow relies on.
    let op = Vec3::new(900.0, 0.0, -600.0);
    let offset = Vec3::new(0.0, 0.0, -120.0);
    let held = coupled_position(op, Quat::from_rotation_y(PI), offset);
    // A half turn puts the astern offset ahead (+Z), still 120 out, still
    // centred on the operator.
    assert!(
        approx(held, Vec3::new(900.0, 0.0, -480.0), 1e-3),
        "{held:?}"
    );
}

#[test]
fn a_three_axis_offset_holds_its_length_under_rotation() {
    // A rig with a vertical component too: still a rigid transform, so the
    // separation is the offset's own length whatever the yaw.
    let offset = Vec3::new(30.0, 15.0, -90.0);
    let len = offset.length();
    for yaw in [0.3_f32, 1.1, 2.7, -2.0] {
        let held = coupled_position(Vec3::ZERO, Quat::from_rotation_y(yaw), offset);
        assert!(
            (held.length() - len).abs() < 1e-3,
            "yaw {yaw}: rig length {} drifted to {}",
            len,
            held.length()
        );
        // The vertical component is untouched by a yaw about +Y.
        assert!((held.y - 15.0).abs() < 1e-3, "yaw {yaw}: {held:?}");
    }
}

// ── The hold verdict ─────────────────────────────────────────────────────

#[test]
fn a_locked_powered_undamaged_in_range_beam_holds() {
    assert_eq!(
        hold_status(Some("derelict"), Some(300.0), 500.0, 3, 2, false),
        Ok(())
    );
    // Exactly at the range boundary still holds.
    assert_eq!(
        hold_status(Some("derelict"), Some(500.0), 500.0, 2, 2, false),
        Ok(())
    );
}

#[test]
fn no_lock_refuses_with_no_lock_even_powered_and_undamaged() {
    assert_eq!(
        hold_status(None, None, 500.0, 3, 2, false),
        Err(TractorRefusal::NoLock)
    );
}

#[test]
fn a_target_past_the_authored_range_refuses_out_of_range() {
    assert_eq!(
        hold_status(Some("derelict"), Some(500.1), 500.0, 3, 2, false),
        Err(TractorRefusal::OutOfRange)
    );
    // A present lock whose entity cannot be found (no separation) is also
    // "nothing in range".
    assert_eq!(
        hold_status(Some("derelict"), None, 500.0, 3, 2, false),
        Err(TractorRefusal::OutOfRange)
    );
}

#[test]
fn power_below_the_minimum_refuses_unpowered_before_target_checks() {
    // Even with no lock, the more-actionable power refusal wins.
    assert_eq!(
        hold_status(None, None, 500.0, 1, 2, false),
        Err(TractorRefusal::Unpowered)
    );
    assert_eq!(
        hold_status(Some("derelict"), Some(10.0), 500.0, 1, 2, false),
        Err(TractorRefusal::Unpowered)
    );
}

#[test]
fn a_disabled_tractor_refuses_first_of_all() {
    // Disabled beats every other failing condition — hardware before power
    // before acquisition.
    assert_eq!(
        hold_status(None, None, 500.0, 0, 2, true),
        Err(TractorRefusal::Disabled)
    );
    assert_eq!(
        hold_status(Some("derelict"), Some(10.0), 500.0, 4, 2, true),
        Err(TractorRefusal::Disabled)
    );
}

// ── Config validation ────────────────────────────────────────────────────

/// A valid tow-load curve for the config literals below.
fn curve() -> TowLoadCurve {
    TowLoadCurve {
        half_penalty_mass: 10_000.0,
        max_penalty: 0.8,
    }
}

#[test]
fn a_well_formed_tractor_config_validates() {
    let cfg = TractorConfig {
        range: 600.0,
        coupling_offset: [0.0, 0.0, -120.0],
        min_power_level: 2,
        tow_load: curve(),
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn a_zero_range_or_zero_min_power_or_nonfinite_offset_is_rejected() {
    let base = TractorConfig {
        range: 600.0,
        coupling_offset: [0.0, 0.0, -120.0],
        min_power_level: 2,
        tow_load: curve(),
    };
    assert!(TractorConfig {
        range: 0.0,
        ..base.clone()
    }
    .validate()
    .is_err());
    assert!(TractorConfig {
        range: -1.0,
        ..base.clone()
    }
    .validate()
    .is_err());
    assert!(TractorConfig {
        min_power_level: 0,
        ..base.clone()
    }
    .validate()
    .is_err());
    assert!(TractorConfig {
        coupling_offset: [0.0, f32::NAN, 0.0],
        ..base
    }
    .validate()
    .is_err());
}

#[test]
fn a_tractor_config_with_a_bad_tow_load_curve_is_rejected() {
    // The curve is validated as part of the config, so a bad `[tractor.
    // tow_load]` is a load error like any other authoring mistake.
    let base = TractorConfig {
        range: 600.0,
        coupling_offset: [0.0, 0.0, -120.0],
        min_power_level: 2,
        tow_load: curve(),
    };
    assert!(TractorConfig {
        tow_load: TowLoadCurve {
            half_penalty_mass: 0.0,
            max_penalty: 0.8,
        },
        ..base.clone()
    }
    .validate()
    .is_err());
    assert!(TractorConfig {
        tow_load: TowLoadCurve {
            half_penalty_mass: 10_000.0,
            max_penalty: 1.0,
        },
        ..base
    }
    .validate()
    .is_err());
}

// ── The tow-load penalty (issue #1157) ───────────────────────────────────

#[test]
fn a_well_formed_tow_load_curve_validates_and_a_bad_one_does_not() {
    assert!(curve().validate().is_ok());
    // Zero-and-below cap is a legal (if pointless) "no penalty" curve.
    assert!(TowLoadCurve {
        half_penalty_mass: 10_000.0,
        max_penalty: 0.0,
    }
    .validate()
    .is_ok());
    // Knee mass must be positive; cap must stay under 1.0; both must be finite.
    for bad in [
        TowLoadCurve {
            half_penalty_mass: -1.0,
            max_penalty: 0.5,
        },
        TowLoadCurve {
            half_penalty_mass: f32::NAN,
            max_penalty: 0.5,
        },
        TowLoadCurve {
            half_penalty_mass: 10_000.0,
            max_penalty: 1.0,
        },
        TowLoadCurve {
            half_penalty_mass: 10_000.0,
            max_penalty: -0.1,
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn the_penalty_is_zero_at_zero_mass_and_halves_the_cap_at_the_knee() {
    let c = curve();
    assert!(
        tow_load_penalty(0.0, &c).abs() < 1e-6,
        "a massless load is free"
    );
    // At exactly the knee mass the penalty is half the cap, by construction.
    let at_knee = tow_load_penalty(c.half_penalty_mass, &c);
    assert!(
        (at_knee - c.max_penalty / 2.0).abs() < 1e-6,
        "the knee mass yields half the cap: got {at_knee}"
    );
}

#[test]
fn a_light_target_is_barely_felt_and_a_heavy_one_is_severe_from_the_same_curve() {
    let c = curve();
    // A buoy an order of magnitude under the knee: a whisper of a penalty.
    let light = tow_load_penalty(1_000.0, &c);
    assert!(
        light < 0.1,
        "a light target is barely felt: penalty {light} should be well under 0.1"
    );
    // A laden freighter many times the knee: most of the way to the cap.
    let heavy = tow_load_penalty(200_000.0, &c);
    assert!(
        heavy > 0.7,
        "a heavy target is severe: penalty {heavy} should be most of the 0.8 cap"
    );
    // Same curve, so heavier always costs more, and neither ever reaches the
    // cap (which is only approached asymptotically).
    assert!(heavy > light);
    assert!(heavy < c.max_penalty);
}

#[test]
fn the_penalty_is_monotonic_in_mass_and_bounded_by_the_cap() {
    let c = curve();
    let mut prev = -1.0;
    for mass in [0.0, 500.0, 5_000.0, 10_000.0, 50_000.0, 1_000_000.0, 1e9] {
        let p = tow_load_penalty(mass, &c);
        assert!(
            p > prev,
            "penalty must strictly increase with mass at {mass}"
        );
        assert!(
            p < c.max_penalty,
            "penalty must stay under the cap at {mass}"
        );
        prev = p;
    }
}

#[test]
fn the_unauthored_mass_default_folds_a_moderate_penalty() {
    // A target that authored no mass carries #1154's DEFAULT_ENTITY_MASS, so
    // the penalty it exacts is exactly that mass's point on the curve — no
    // special case for "unauthored".
    let c = curve();
    let default_mass = crate::entities::config::DEFAULT_ENTITY_MASS;
    let expected = c.max_penalty * default_mass / (default_mass + c.half_penalty_mass);
    assert!((tow_load_penalty(default_mass, &c) - expected).abs() < 1e-6);
    // DEFAULT_ENTITY_MASS (10_000) sits right at this curve's knee, so it is
    // half the cap — a sanity check that the default is a felt-but-fair tow.
    assert!(
        (tow_load_penalty(default_mass, &c) - c.max_penalty / 2.0).abs() < 1e-3,
        "the default mass sits at this curve's knee"
    );
}
