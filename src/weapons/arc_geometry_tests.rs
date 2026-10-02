use super::*;

fn bank(facing_deg: f32, fire_arc_deg: f32, range: f32) -> WeaponArcBank {
    WeaponArcBank {
        facing_deg,
        fire_arc_deg,
        range,
    }
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

// ── AC1: table test over representative poses / configs ──────────────────

#[test]
fn sector_bearing_is_yaw_plus_facing_across_representative_poses() {
    // (yaw_deg, facing_deg, expected world bearing_deg)
    let cases = [
        (0.0_f32, 0.0_f32, 0.0_f32),
        (0.0, 90.0, 90.0),
        (0.0, -90.0, -90.0),
        (0.0, 180.0, 180.0),
        (90.0, 0.0, 90.0),
        (90.0, 90.0, 180.0),
        (90.0, 180.0, -90.0),
        (-90.0, -90.0, 180.0),
        (180.0, 180.0, 0.0),
        (45.0, 45.0, 90.0),
        (350.0, 20.0, 10.0),
    ];
    for (yaw_deg, facing_deg, expected) in cases {
        let out = weapon_arc_sectors(yaw_deg.to_radians(), &[bank(facing_deg, 60.0, 100.0)]);
        assert_eq!(out.len(), 1);
        assert!(
            approx(out[0].bearing_deg, expected),
            "yaw {yaw_deg} + facing {facing_deg} => {} (want {expected})",
            out[0].bearing_deg
        );
        assert!(approx(out[0].half_angle_deg, 30.0));
        assert!(approx(out[0].range, 100.0));
    }
}

#[test]
fn half_angle_is_half_the_authored_total_width() {
    for total in [1.0_f32, 45.0, 60.0, 90.0, 180.0, 270.0, 360.0] {
        let out = weapon_arc_sectors(0.0, &[bank(0.0, total, 10.0)]);
        assert!(approx(out[0].half_angle_deg, total * 0.5), "total {total}");
    }
}

#[test]
fn degenerate_banks_produce_no_sector() {
    let out = weapon_arc_sectors(
        0.0,
        &[
            bank(0.0, 0.0, 100.0),   // no width
            bank(0.0, -30.0, 100.0), // negative width
            bank(0.0, 60.0, 0.0),    // no reach
            bank(0.0, 60.0, -5.0),   // negative reach
        ],
    );
    assert!(out.is_empty(), "got {out:?}");
}

#[test]
fn every_authored_bank_yields_its_own_sector_in_order() {
    let out = weapon_arc_sectors(
        0.0,
        &[
            bank(-90.0, 180.0, 50.0),
            bank(90.0, 180.0, 60.0),
            bank(0.0, 30.0, 70.0),
        ],
    );
    assert_eq!(out.len(), 3);
    assert!(approx(out[0].bearing_deg, -90.0) && approx(out[0].range, 50.0));
    assert!(approx(out[1].bearing_deg, 90.0) && approx(out[1].range, 60.0));
    assert!(approx(out[2].bearing_deg, 0.0) && approx(out[2].range, 70.0));
}

// ── world_bearing_deg ───────────────────────────────────────────────────

#[test]
fn world_bearing_matches_the_yaw_convention() {
    // yaw 0 faces -Z, so a contact at -Z bears 0.
    assert!(approx(world_bearing_deg(0.0, 0.0, 0.0, -10.0), 0.0));
    // +X is starboard of a yaw-0 ship => +90.
    assert!(approx(world_bearing_deg(0.0, 0.0, 10.0, 0.0), 90.0));
    assert!(approx(world_bearing_deg(0.0, 0.0, 0.0, 10.0), 180.0));
    assert!(approx(world_bearing_deg(0.0, 0.0, -10.0, 0.0), -90.0));
}

// ── AC2 reduction: table test ───────────────────────────────────────────

#[test]
fn exposure_table_over_representative_relative_poses() {
    // A yaw-0 hostile at the origin with a 60-degree forward bank, reach 100.
    let sectors = weapon_arc_sectors(0.0, &[bank(0.0, 60.0, 100.0)]);
    // (observer_x, observer_z, covered)
    let cases = [
        (0.0_f32, -50.0_f32, true), // dead ahead
        (20.0, -50.0, true),        // +21.8 deg, inside the 30 deg half
        (-20.0, -50.0, true),       // -21.8 deg, inside
        (50.0, -50.0, false),       // +45 deg, outside
        (0.0, 50.0, false),         // astern
        (0.0, -150.0, false),       // in arc but beyond reach
        (100.0, 0.0, false),        // abeam
    ];
    for (x, z, covered) in cases {
        let e = arc_exposure(&sectors, 0.0, 0.0, x, z);
        assert_eq!(
            e.covering_count > 0,
            covered,
            "observer ({x}, {z}) => {e:?}"
        );
    }
}

#[test]
fn escape_offset_is_the_shorter_way_out_of_the_covering_sector() {
    let sectors = weapon_arc_sectors(0.0, &[bank(0.0, 60.0, 100.0)]);
    // Dead ahead: symmetric, 30 degrees either way; ties resolve positive.
    let centre = arc_exposure(&sectors, 0.0, 0.0, 0.0, -50.0);
    assert_eq!(centre.covering_count, 1);
    assert!(approx(centre.escape_offset_deg, 30.0), "{centre:?}");

    // Sitting at +20 degrees: 10 degrees further to starboard gets out,
    // 50 degrees to port would. Shorter way is positive.
    let stbd_x = 50.0_f32 * simmath::tan((20.0_f32).to_radians());
    let stbd = arc_exposure(&sectors, 0.0, 0.0, stbd_x, -50.0);
    assert_eq!(stbd.covering_count, 1);
    assert!(approx(stbd.escape_offset_deg, 10.0), "{stbd:?}");

    // Mirror image: shorter way is negative.
    let port = arc_exposure(&sectors, 0.0, 0.0, -stbd_x, -50.0);
    assert!(approx(port.escape_offset_deg, -10.0), "{port:?}");
}

#[test]
fn escape_offset_clears_every_covering_sector_not_just_the_narrowest() {
    // Two overlapping forward banks: a narrow one and a wide one.
    let sectors = weapon_arc_sectors(0.0, &[bank(0.0, 30.0, 100.0), bank(0.0, 160.0, 100.0)]);
    let e = arc_exposure(&sectors, 0.0, 0.0, 0.0, -50.0);
    assert_eq!(e.covering_count, 2);
    // Leaving the 15-degree half-arc is not enough; the 80-degree one rules.
    assert!(approx(e.escape_offset_deg, 80.0), "{e:?}");
}

#[test]
fn a_broadside_pair_counts_only_the_side_that_bears() {
    let sectors = weapon_arc_sectors(0.0, &[bank(-90.0, 120.0, 100.0), bank(90.0, 120.0, 100.0)]);
    let to_starboard = arc_exposure(&sectors, 0.0, 0.0, 50.0, 0.0);
    assert_eq!(to_starboard.covering_count, 1);
    let to_port = arc_exposure(&sectors, 0.0, 0.0, -50.0, 0.0);
    assert_eq!(to_port.covering_count, 1);
    // Dead ahead sits on the edge of neither.
    let ahead = arc_exposure(&sectors, 0.0, 0.0, 0.0, -50.0);
    assert_eq!(ahead.covering_count, 0);
    assert!(approx(ahead.escape_offset_deg, 0.0));
}

#[test]
fn rotating_the_hostile_rotates_its_exposure() {
    // Observer due +X of the hostile. A forward bank bears on it only once
    // the hostile has turned to starboard.
    let observer = (100.0_f32, 0.0_f32);
    let facing_away = weapon_arc_sectors(0.0, &[bank(0.0, 60.0, 200.0)]);
    assert_eq!(
        arc_exposure(&facing_away, 0.0, 0.0, observer.0, observer.1).covering_count,
        0
    );
    let facing_observer = weapon_arc_sectors((90.0_f32).to_radians(), &[bank(0.0, 60.0, 200.0)]);
    assert_eq!(
        arc_exposure(&facing_observer, 0.0, 0.0, observer.0, observer.1).covering_count,
        1
    );
}

#[test]
fn exposure_wraps_correctly_across_the_aft_seam() {
    // An aft bank centred on 180 degrees must cover a contact at -179.
    let sectors = weapon_arc_sectors(0.0, &[bank(180.0, 20.0, 200.0)]);
    let z = 100.0_f32;
    let x = -z * simmath::tan((1.0_f32).to_radians());
    let e = arc_exposure(&sectors, 0.0, 0.0, x, z);
    assert_eq!(e.covering_count, 1, "{e:?}");
}

/// An all-round bank covers every bearing, so it must read as covering from
/// every relative pose in reach — and must never claim an escape magnitude.
/// `alliance_destroyer.toml` authors one of these (`omni`).
#[test]
fn an_all_round_bank_is_inescapable_from_every_bearing() {
    let sectors = weapon_arc_sectors(0.0, &[bank(0.0, 360.0, 100.0)]);
    assert!(approx(sectors[0].half_angle_deg, 180.0));
    // (observer_x, observer_z, in reach)
    let cases = [
        (0.0_f32, -50.0_f32, true), // dead ahead
        (50.0, 0.0, true),          // abeam to starboard
        (-50.0, 0.0, true),         // abeam to port
        (0.0, 50.0, true),          // dead astern
        (35.0, 35.0, true),         // quarter
        (0.0, -150.0, false),       // beyond reach: covered by nothing
    ];
    for (x, z, in_reach) in cases {
        let e = arc_exposure(&sectors, 0.0, 0.0, x, z);
        assert_eq!(
            e.covering_count > 0,
            in_reach,
            "observer ({x}, {z}) => {e:?}"
        );
        assert_eq!(
            e.inescapable, in_reach,
            "an all-round bank in reach must flag inescapable ({x}, {z}) => {e:?}"
        );
        assert!(
            approx(e.escape_offset_deg, 0.0),
            "an all-round bank must not report an escape magnitude; ({x}, {z}) => {e:?}"
        );
    }
}

/// The distinction #877's dodging policies turn on: "nothing bears on me"
/// and "I cannot turn out of this" are different readings, not both zero.
#[test]
fn clear_and_inescapable_are_distinguishable() {
    let all_round = weapon_arc_sectors(0.0, &[bank(0.0, 360.0, 100.0)]);
    let clear = arc_exposure(&all_round, 0.0, 0.0, 0.0, -500.0);
    let trapped = arc_exposure(&all_round, 0.0, 0.0, 0.0, -50.0);
    assert_eq!(clear.covering_count, 0);
    assert!(!clear.inescapable);
    assert!(trapped.covering_count > 0);
    assert!(trapped.inescapable);
    assert_ne!(clear, trapped);

    // And an escapable covering sector is a third, distinct reading: it
    // carries a real magnitude and does not raise the flag.
    let narrow = weapon_arc_sectors(0.0, &[bank(0.0, 60.0, 100.0)]);
    let escapable = arc_exposure(&narrow, 0.0, 0.0, 0.0, -50.0);
    assert!(escapable.covering_count > 0);
    assert!(!escapable.inescapable);
    assert!(escapable.escape_offset_deg.abs() > 0.0);
}

/// A wide-but-finite bank alongside an all-round one: the pair is still
/// inescapable, and the finite bank's real exit must not be reported as if
/// it were a way out.
#[test]
fn an_all_round_bank_suppresses_a_sibling_banks_escape() {
    let sectors = weapon_arc_sectors(0.0, &[bank(0.0, 60.0, 100.0), bank(0.0, 360.0, 100.0)]);
    let e = arc_exposure(&sectors, 0.0, 0.0, 0.0, -50.0);
    assert_eq!(e.covering_count, 2, "{e:?}");
    assert!(e.inescapable, "{e:?}");
    assert!(
        approx(e.escape_offset_deg, 0.0),
        "leaving the 30-degree half-arc does not leave the all-round one; {e:?}"
    );
}

#[test]
fn no_sectors_means_no_exposure() {
    let e = arc_exposure(&[], 0.0, 0.0, 10.0, 10.0);
    assert_eq!(e, ArcExposure::default());
}
