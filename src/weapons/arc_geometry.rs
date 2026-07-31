//! Pure hostile weapon-arc geometry (issue #874).
//!
//! Bevy-free (AGENTS.md #10). This module answers one question and answers it
//! for everybody: **where does a ship's authored weapon-bank arc point in the
//! world, and am I standing in it?**
//!
//! ## Why a new producer rather than the existing arc-request path
//!
//! [`crate::console::weapons::evaluate_family_arc_request`] answers a different
//! question — it is gated on a live combat lock, on emitters being online AND
//! usable, and on a firing *miss*, returning `None` otherwise. #874 wants arcs
//! that are **always known from config**, target-independent, with no scan gate:
//! a weapon bank's arc is a property of the hull, not of anyone's sensors.
//!
//! ## Frames and units
//!
//! Authored arcs are ship-relative degrees clockwise from ship-forward, exactly
//! as documented in [`crate::weapons::phaser`]: `0` forward, `+90` starboard,
//! `±180` aft, `−90` port. `fire_arc_deg` is the TOTAL width, so the half-angle
//! is half of it.
//!
//! A *sector* ([`WeaponArcSector`]) is the same arc expressed as a **world**
//! bearing. Taking the ship-local frame from `radar.rs`
//! (`radar_y = dx·sin(yaw) − dz·cos(yaw)` is the ahead component, so ship
//! forward is the world direction `(sin yaw, −cos yaw)`), a ship-relative
//! bearing `θ` points along `(sin(yaw + θ), −cos(yaw + θ))`. World bearing is
//! therefore simply `yaw + facing`, in the same convention as `yaw` itself.
//!
//! ## All-round banks, and why escape is a flag rather than a magnitude
//!
//! `fire_arc_deg = 360.0` is authored content — `ship_harrow_lancer.toml` gives
//! its phaser and blaster banks both — so a half-angle of 180 is a case this
//! module must answer honestly rather than a degenerate input it can dismiss.
//! Such a sector covers every bearing, so there is no bearing change that leaves
//! it. Feeding it through the same `half_angle − offset` arithmetic as a narrow
//! bank would report an "escape" of up to 360 degrees: a magnitude a dodging
//! movement policy would act on, and a lie.
//!
//! [`ArcExposure`] therefore reports it as a distinct THIRD reading. A policy
//! sees three states, not two:
//!
//! | `covering_count` | `inescapable` | meaning                              |
//! |------------------|---------------|--------------------------------------|
//! | `0`              | `false`       | nothing bears on me                  |
//! | `> 0`            | `false`       | turn `escape_offset_deg` and I am out |
//! | `> 0`            | `true`        | I cannot turn out of this            |
//!
//! Overloading `escape_offset_deg = 0.0` to mean the third state was the cheaper
//! option and was rejected: an observer sitting exactly on a narrow sector's
//! edge also reduces to zero, so the two would be indistinguishable at the one
//! bearing where the difference matters most.
//!
//! Emitting world bearings is deliberate: it is what lets the JS client draw
//! these sectors with **no arc math at all** beyond world-bearing → screen-angle
//! projection. The client *could* recompute the arcs itself (it receives every
//! hostile's `yaw` and `position`), but then human and AI would agree only by
//! coincidence. One server-side producer call feeds the AI fact reduction and
//! the wire payload, so they agree by construction (issue #874 AC4).

use crate::simmath;

/// One authored weapon bank's arc, ship-relative — the producer's input.
///
/// Deliberately narrower than the per-family bank configs: arc geometry cares
/// about facing, width and reach, and nothing about damage, cooldown or ammo.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct WeaponArcBank {
    /// Centre of the arc, degrees clockwise from ship-forward.
    pub facing_deg: f32,
    /// TOTAL arc width in degrees (the half-angle is half of this).
    pub fire_arc_deg: f32,
    /// Effective reach of this bank, world units.
    pub range: f32,
}

/// One weapon arc expressed as a world-bearing sector — the producer's output,
/// and the single representation both the AI fact and the helm-radar overlay
/// are derived from.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct WeaponArcSector {
    /// World bearing of the sector's centre-line, degrees, normalised to
    /// `(−180, 180]`. Same convention as ship yaw: `0` points along `−Z`.
    pub bearing_deg: f32,
    /// HALF the arc width, degrees. Half rather than total because every
    /// consumer (the in-sector test, the SVG wedge) wants the half-angle.
    pub half_angle_deg: f32,
    /// Effective reach of the bank this sector belongs to, world units.
    pub range: f32,
}

/// The scalar reduction of a sector list against one observer's position.
///
/// `AiFacts` values are `f64` scalars, so a `Vec` of sectors can never be a
/// `fact()` atom — the policy-readable form of this geometry *must* be a
/// reduction. These two readings are what a dodging movement policy actually
/// needs: a gate ("am I being borne on, and by how many guns") and a direction
/// ("which way is out, and how far").
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ArcExposure {
    /// How many of the hostile's sectors currently bear on the observer — in
    /// arc AND within that bank's reach. `0` means clear.
    pub covering_count: u32,
    /// Signed bearing change, degrees, that would take the observer out of
    /// EVERY covering sector by the shorter way round. Positive means "further
    /// round toward the hostile's starboard side". `0.0` when not covered, and
    /// `0.0` when [`Self::inescapable`] — see the module note on all-round
    /// banks.
    pub escape_offset_deg: f32,
    /// `true` when at least one covering sector spans a full turn or more, so
    /// NO bearing change leaves it.
    ///
    /// This is the flag a movement policy gates on before it acts on
    /// [`Self::escape_offset_deg`]: `covering_count > 0 && !inescapable` is
    /// "turn this far and you are clear", `covering_count > 0 && inescapable`
    /// is "you cannot turn out of this — open the range or accept the fire",
    /// and `covering_count == 0` is clear. Without it the three collapse into
    /// two, because an all-round bank has no honest escape magnitude to report.
    pub inescapable: bool,
}

/// Convert a ship's authored banks + its world yaw into world-bearing sectors.
///
/// **This is the producer.** Called once per entity per world-snapshot rebuild;
/// its output feeds both [`arc_exposure`] (the AI fact) and the helm-radar wire
/// payload. Banks with a non-positive arc width or reach are dropped: a
/// zero-width sector is not a threat and drawing it would be a lie.
pub fn weapon_arc_sectors(ship_yaw_rad: f32, banks: &[WeaponArcBank]) -> Vec<WeaponArcSector> {
    banks
        .iter()
        .filter(|b| b.fire_arc_deg > 0.0 && b.range > 0.0)
        .map(|b| WeaponArcSector {
            bearing_deg: normalise_deg(ship_yaw_rad.to_degrees() + b.facing_deg),
            half_angle_deg: b.fire_arc_deg * 0.5,
            range: b.range,
        })
        .collect()
}

/// World bearing from `(from_x, from_z)` to `(to_x, to_z)`, degrees, in the same
/// convention as [`WeaponArcSector::bearing_deg`].
///
/// Projection only — the XZ plane, matching every other range/bearing check in
/// the sim.
pub fn world_bearing_deg(from_x: f32, from_z: f32, to_x: f32, to_z: f32) -> f32 {
    let dx = to_x - from_x;
    let dz = to_z - from_z;
    normalise_deg(simmath::atan2(dx, -dz).to_degrees())
}

/// Reduce one hostile's sectors against an observer's position.
///
/// The escape offset is resolved across ALL covering sectors at once: leaving
/// only the narrowest one still leaves the observer under fire, so the positive
/// escape is the largest positive exit any covering sector demands, and likewise
/// negative. The smaller magnitude of the two wins, which is the shorter way
/// out.
///
/// An all-round bank is reported as [`ArcExposure::inescapable`] rather than as
/// a magnitude — see the module note.
pub fn arc_exposure(
    sectors: &[WeaponArcSector],
    hostile_x: f32,
    hostile_z: f32,
    observer_x: f32,
    observer_z: f32,
) -> ArcExposure {
    let dx = observer_x - hostile_x;
    let dz = observer_z - hostile_z;
    let dist = (dx * dx + dz * dz).sqrt();
    let bearing = world_bearing_deg(hostile_x, hostile_z, observer_x, observer_z);

    let mut covering_count = 0u32;
    let mut inescapable = false;
    let mut exit_positive: f32 = 0.0;
    let mut exit_negative: f32 = 0.0;
    for s in sectors {
        if dist > s.range {
            continue;
        }
        let offset = signed_deg_diff(bearing, s.bearing_deg);
        if offset.abs() > s.half_angle_deg {
            continue;
        }
        covering_count += 1;
        // An all-round bank (`fire_arc_deg >= 360`, half-angle >= 180) has no
        // exit bearing at all. Folding its `half_angle - offset` into the maxima
        // below would emit a number up to 360 — "turn a full circle and you're
        // clear" — which is false, so it is flagged instead of measured.
        if s.half_angle_deg >= 180.0 {
            inescapable = true;
            continue;
        }
        exit_positive = exit_positive.max(s.half_angle_deg - offset);
        exit_negative = exit_negative.max(s.half_angle_deg + offset);
    }

    let escape_offset_deg = if covering_count == 0 || inescapable {
        0.0
    } else if exit_positive <= exit_negative {
        exit_positive
    } else {
        -exit_negative
    };
    ArcExposure {
        covering_count,
        escape_offset_deg,
        inescapable,
    }
}

/// Normalise degrees to `(−180, 180]`.
fn normalise_deg(deg: f32) -> f32 {
    let mut d = deg % 360.0;
    if d > 180.0 {
        d -= 360.0;
    }
    while d <= -180.0 {
        d += 360.0;
    }
    d
}

/// Signed angular difference `a − b`, degrees, wrapped to `(−180, 180]`.
fn signed_deg_diff(a: f32, b: f32) -> f32 {
    normalise_deg(a - b)
}

#[cfg(test)]
mod tests {
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
        let sectors =
            weapon_arc_sectors(0.0, &[bank(-90.0, 120.0, 100.0), bank(90.0, 120.0, 100.0)]);
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
        let facing_observer =
            weapon_arc_sectors((90.0_f32).to_radians(), &[bank(0.0, 60.0, 200.0)]);
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
    /// `ship_harrow_lancer.toml` authors two of these.
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
}
