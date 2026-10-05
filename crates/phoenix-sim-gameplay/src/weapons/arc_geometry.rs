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
//! `fire_arc_deg = 360.0` is authored content — `alliance_destroyer.toml` gives
//! its `omni` suppression phaser one (issue #639) — so a half-angle of 180 is a
//! case this module must answer honestly rather than a degenerate input it can
//! dismiss.
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
#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
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
#[path = "arc_geometry_tests.rs"]
mod tests;

/// Signed radian difference, retaining both endpoints of [-PI, PI].
pub(super) fn signed_difference(a: f32, b: f32) -> f32 {
    use std::f32::consts::PI;
    let mut d = a - b;
    while d > PI {
        d -= 2.0 * PI;
    }
    while d < -PI {
        d += 2.0 * PI;
    }
    d
}
