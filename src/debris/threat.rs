//! The pure, Bevy-free **debris threat** vocabulary (issue #1347, parent #1337).
//!
//! A piece of debris is the first thing in this game that is dangerous without
//! being hostile. It has no faction, no doctrine and no opinion; it is a mass on
//! a course, and the only question worth asking about it is *what is it going to
//! hit, and when*. This module owns that question and nothing else.
//!
//! # THE ASSESSMENT IS A DERIVATION. THERE IS NO AUTHORED THREAT TEXT
//!
//! [`crate::science::scan`] states the principle this module inherits — "sensors
//! reveal state rather than scenario text" — and this module is built the same
//! way, for the same reason. A scenario cannot author "this rock is going to hit
//! the depot": it authors a **drift**, a **protected asset** and an **impact
//! radius**, and [`assess`] reads the geometry those three produce.
//!
//! * [`DebrisSubject`] is the whole input port, and every field on it is a raw
//!   quantity — two vectors, a radius, and the protected asset's own authored
//!   name id. There is no field for a verdict, a summary or a warning, so there
//!   is nothing for authored prose to arrive on.
//! * [`DebrisAssessment`] carries only numbers and one `strings.csv` id the
//!   author wrote against the *protected asset*, never against the threat.
//!   `on_collision_course` is a comparison of two of those numbers, not a flag
//!   somebody set.
//! * Re-author the drift to miss and the same code says it misses, with no line
//!   of copy edited and no handler touched.
//!
//! # Why the geometry is PLANAR
//!
//! Every other tactical judgement in the game — the selector's horizon filter,
//! the sensors radar's `in_range_pos`, the weapons radar's `within_range` — is
//! taken on the x/z plane, and a debris assessment that disagreed with the radar
//! the crew are reading it beside would be a second opinion rather than a
//! deeper one. So closest approach is planar, and [`DebrisAssessment::course`]
//! is carried in exactly the shape and units issue #1339's
//! `selected_target_relative_velocity` already publishes on that same radar.
//!
//! # Determinism
//!
//! `+ - * /`, `sqrt` and comparisons only — every one of them IEEE-754 exact on
//! every target, so two peers projecting the same rock against the same depot
//! report the same second. No trigonometry, no `f64` promotion, no accumulation
//! across ticks: [`assess`] is a total function of one tick's positions.

use serde::{Deserialize, Serialize};

// ── The authored table ───────────────────────────────────────────────────────

/// The `[debris]` table an entity template authors to become a moving hazard
/// (issue #1347).
///
/// Everything a debris contact *is* lives here, and everything it *does to the
/// scenario* is a flag this table names. That split is deliberate: the
/// simulation owns the geometry — where the rock is, what it is closing on, and
/// the tick it arrives — and the **scenario** owns every consequence, through
/// ordinary `on_flag_set` handlers on the four flags below. Fragment behaviour,
/// the damage a struck asset takes, which objective goes red and what the ship's
/// computer says are therefore authored data in the world file, exactly as issue
/// #1347 asks, and none of them is spelled in Rust.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebrisConfig {
    /// Drift velocity in world units per second, as `[x, y, z]`.
    ///
    /// A constant velocity, not an acceleration and not a pursuit: debris does
    /// not steer. That is the whole reason it can be projected at all, and the
    /// reason a crew who work out where it is going are ahead of it rather than
    /// merely watching it.
    #[serde(default)]
    pub drift: [f32; 3],
    /// The reference id (`[[entity]] name`) of the asset this debris is on
    /// course for — the handle a scenario types, never a minted UUID.
    ///
    /// Empty means "nothing in particular": the contact still drifts and is
    /// still scannable, but no closest approach is projected and it can never
    /// be confirmed. That is a legitimate authoring choice — a field of harmless
    /// wreckage the crew have to *rule out* is what makes ruling one in matter.
    #[serde(default)]
    pub protected_target: String,
    /// Distance from the protected asset's centre at which this debris strikes
    /// it, world units. Must be positive when a `protected_target` is named.
    #[serde(default)]
    pub impact_radius: f32,
    /// How many seconds before impact the contact is reckoned URGENT — the
    /// point at which a crew who have not fired yet are running out of room.
    ///
    /// Authored rather than derived because it is a doctrine number, not a
    /// physical one: how long *this scenario's* ship needs to solve a firing
    /// problem is the designer's call, and it is what
    /// [`DebrisConfig::urgent_flag`] fires on.
    #[serde(default)]
    pub urgent_secs: f32,
    /// World flag raised the first time an assessment of this contact comes
    /// back — "the crew have read this rock". Empty raises nothing.
    #[serde(default)]
    pub assessed_flag: String,
    /// World flag raised when an assessment says this contact is on course to
    /// strike its protected asset. Empty raises nothing.
    ///
    /// THIS IS THE AUTHORITATIVE THREAT STATE. Nothing else in the simulation
    /// promotes a rock to a threat: not the radar seeing it, not a human
    /// selecting it, and not the fact that it happens to be on course. Somebody
    /// has to *look*.
    #[serde(default)]
    pub confirmed_flag: String,
    /// World flag raised when a confirmed contact's projected impact falls
    /// inside [`urgent_secs`](Self::urgent_secs). Empty raises nothing.
    #[serde(default)]
    pub urgent_flag: String,
    /// World flag raised on the tick this debris reaches its protected asset.
    /// Empty raises nothing.
    #[serde(default)]
    pub impact_flag: String,
}

impl DebrisConfig {
    /// Reject an unusable table at content-load rather than at the tick that
    /// would have divided by it.
    ///
    /// A named protected asset is what makes the other two numbers mean
    /// anything, so they are required together and validated together: a rock
    /// aimed at a depot with a zero impact radius can never arrive, which is a
    /// silently inert hazard rather than an authored one.
    pub fn validate(&self) -> Result<(), String> {
        if !self.drift.iter().all(|c| c.is_finite()) {
            return Err(format!(
                "[debris] drift must be finite, got {:?}",
                self.drift
            ));
        }
        if !self.impact_radius.is_finite() || self.impact_radius < 0.0 {
            return Err(format!(
                "[debris] impact_radius must be finite and non-negative, got {}",
                self.impact_radius
            ));
        }
        if !self.urgent_secs.is_finite() || self.urgent_secs < 0.0 {
            return Err(format!(
                "[debris] urgent_secs must be finite and non-negative, got {}",
                self.urgent_secs
            ));
        }
        if !self.protected_target.is_empty() && !positive(self.impact_radius) {
            return Err(format!(
                "[debris] protects '{}' but authors impact_radius {} — a hazard \
                 that can never arrive",
                self.protected_target, self.impact_radius
            ));
        }
        Ok(())
    }
}

// ── The input port ───────────────────────────────────────────────────────────

/// Everything [`assess`] may see — **the whole input port**.
///
/// The shortness is the point: see the module docs. Both vectors are stated
/// RELATIVE to the protected asset, so the assessment is indifferent to where
/// the pair happens to be in the world and to who is doing the reading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebrisSubject {
    /// The debris's position relative to the protected asset, planar `[x, z]`.
    pub relative_position: [f32; 2],
    /// The debris's velocity relative to the protected asset, planar `[x, z]`
    /// in world units per second.
    pub relative_velocity: [f32; 2],
    /// `strings.csv` id for the protected asset's crew-facing name. The
    /// asset's own authored name — the same id the radar and the dossier show.
    pub protected_name: String,
    /// The authored radius inside which this debris strikes that asset.
    pub impact_radius: f32,
}

// ── The assessment ───────────────────────────────────────────────────────────

/// What an assessment of a debris contact came back with (issue #1347).
///
/// Stamped into a [`crate::science::ScanReading`] and **not** recomputed
/// afterwards, for that type's own reason: this is what the crew worked out when
/// they looked, and a plot that silently corrected itself would make looking
/// again pointless. A consumer that needs the number *now* dead-reckons it from
/// the reading's own tick — which is what a tactical plot has always done.
///
/// Every string is a `strings.csv` id; no English crosses the wire.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DebrisAssessment {
    /// `strings.csv` id for the protected asset's crew-facing name — WHAT IS
    /// UNDER THIS ROCK, which is the one thing a threat readout has to say.
    #[serde(default)]
    pub protected_name: String,
    /// The debris's course relative to the protected asset, planar `[x, z]` in
    /// world units per second — the same shape and units the sensors radar's
    /// `selected_target_relative_velocity` (issue #1339) already publishes, so
    /// a console draws this projection with the code it already has.
    pub course: [f32; 2],
    /// How close the debris gets to the protected asset on this course, world
    /// units. Equal to the present separation when the contact is opening.
    pub closest_approach: f32,
    /// Seconds until that closest approach. `0.0` when it is already past —
    /// the rock's nearest moment was behind it and it is drawing away.
    pub seconds_to_closest_approach: f32,
    /// Whether [`closest_approach`](Self::closest_approach) falls inside the
    /// authored impact radius. **The threat verdict**, and a comparison of two
    /// numbers above rather than a judgement of its own.
    pub on_collision_course: bool,
    /// Seconds until the debris crosses into the impact radius. `None` when it
    /// is not on a collision course at all — never `0.0` standing in for "no
    /// answer", because a crew reading zero seconds must be reading an impact
    /// that is happening, not one that will never happen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_to_impact: Option<f32>,
}

/// Project one debris contact against the asset it is drifting toward.
///
/// Deterministic and total: the same separation and the same closing velocity
/// always return the same assessment, and there is no path that returns nothing
/// — a rock going nowhere is assessed as going nowhere, which is a finding.
///
/// The projection is the standard closest-point-of-approach: with the protected
/// asset at the origin, the debris sits at `p` and moves at `v`, so its
/// separation squared is the quadratic `|p + v t|²` and the nearest moment is
/// its vertex, `t = -(p·v) / (v·v)`. A stationary contact (`v·v == 0`) has no
/// vertex and is reported where it stands, forever.
pub fn assess(subject: &DebrisSubject) -> DebrisAssessment {
    let p = subject.relative_position;
    let v = subject.relative_velocity;
    let vv = dot(v, v);
    let pp = dot(p, p);

    // The vertex of |p + v t|². Clamped at zero: a contact whose nearest moment
    // is in the PAST is at its closest right now, as far as anyone can still do
    // anything about it.
    let t_ca = if positive(vv) {
        let t = -dot(p, v) / vv;
        if t > 0.0 {
            t
        } else {
            0.0
        }
    } else {
        0.0
    };
    let closest_sq = sq_at(p, v, t_ca);
    let closest_approach = closest_sq.sqrt();

    let radius = subject.impact_radius;
    let on_collision_course = positive(radius) && closest_approach <= radius;

    // Seconds to the impact SPHERE, not to the closest approach: the rock stops
    // being a projection and becomes a collision the moment it crosses the line,
    // which is earlier and is the number a firing solution is racing.
    //
    // |p + v t|² = r² is `vv t² + 2(p·v) t + (pp - r²) = 0`; the smaller
    // non-negative root is the crossing. A contact ALREADY inside the radius
    // (pp <= r²) is arriving now.
    let seconds_to_impact = if !on_collision_course {
        None
    } else if pp <= radius * radius {
        Some(0.0)
    } else if positive(vv) {
        let b = dot(p, v);
        let c = pp - radius * radius;
        let disc = b * b - vv * c;
        if disc < 0.0 {
            // Unreachable while `on_collision_course` holds — the closest
            // approach is inside the radius, so the quadratic has real roots.
            // Kept as an arm rather than an `expect` because a fail-closed
            // "no answer" is the honest reading of an incomparable value.
            None
        } else {
            let t = (-b - disc.sqrt()) / vv;
            Some(if t > 0.0 { t } else { 0.0 })
        }
    } else {
        // Inside the radius is handled above, so a stationary contact outside it
        // never arrives, whatever its closest approach says.
        None
    };

    DebrisAssessment {
        protected_name: subject.protected_name.clone(),
        course: v,
        closest_approach,
        seconds_to_closest_approach: t_ca,
        on_collision_course,
        seconds_to_impact,
    }
}

/// `|p + v t|²`, the separation the projection above is the vertex of.
fn sq_at(p: [f32; 2], v: [f32; 2], t: f32) -> f32 {
    let x = p[0] + v[0] * t;
    let z = p[1] + v[1] * t;
    x * x + z * z
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

// Fail-closed on an incomparable value: a NaN radius or speed refuses rather
// than passes, which is why this is spelled via `partial_cmp` instead of
// `x > 0.0` under a negation (the lint), matching `science::scan::positive`.
fn positive(x: f32) -> bool {
    matches!(x.partial_cmp(&0.0), Some(std::cmp::Ordering::Greater))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rock 100 units up-track from a depot, closing at 10 units/sec straight
    /// down the line: it arrives, and the arithmetic is checkable by eye.
    fn head_on() -> DebrisSubject {
        DebrisSubject {
            relative_position: [100.0, 0.0],
            relative_velocity: [-10.0, 0.0],
            protected_name: "world.probe.entity.depot.name".into(),
            impact_radius: 20.0,
        }
    }

    #[test]
    fn a_head_on_contact_is_confirmed_and_reports_its_arrival() {
        let a = assess(&head_on());
        assert!(a.on_collision_course, "it is aimed at the middle of it");
        assert_eq!(a.closest_approach, 0.0);
        assert_eq!(a.seconds_to_closest_approach, 10.0);
        // The IMPACT is the radius crossing at 80 units, not the centre at 100.
        assert_eq!(a.seconds_to_impact, Some(8.0));
        assert_eq!(a.course, [-10.0, 0.0]);
        assert_eq!(a.protected_name, "world.probe.entity.depot.name");
    }

    #[test]
    fn a_contact_that_passes_wide_is_not_a_threat_however_close_it_comes() {
        // Same closing speed, offset 30 units across the depot's beam — outside
        // the authored 20-unit radius, so it misses. The projection still says
        // exactly how near and exactly when, which is the finding.
        let mut subject = head_on();
        subject.relative_position = [100.0, 30.0];
        let a = assess(&subject);
        assert!(!a.on_collision_course);
        assert_eq!(a.closest_approach, 30.0);
        assert_eq!(a.seconds_to_closest_approach, 10.0);
        assert_eq!(
            a.seconds_to_impact, None,
            "no arrival exists, and 0.0 must never stand in for that"
        );
    }

    #[test]
    fn a_contact_already_drawing_away_reports_its_present_separation() {
        let mut subject = head_on();
        subject.relative_velocity = [10.0, 0.0];
        let a = assess(&subject);
        assert_eq!(
            a.seconds_to_closest_approach, 0.0,
            "the vertex is behind it"
        );
        assert_eq!(a.closest_approach, 100.0);
        assert!(!a.on_collision_course);
    }

    #[test]
    fn a_stationary_contact_outside_the_radius_never_arrives() {
        let mut subject = head_on();
        subject.relative_velocity = [0.0, 0.0];
        let a = assess(&subject);
        assert_eq!(a.closest_approach, 100.0);
        assert_eq!(a.seconds_to_closest_approach, 0.0);
        assert!(!a.on_collision_course);
        assert_eq!(a.seconds_to_impact, None);
    }

    #[test]
    fn a_contact_already_inside_the_radius_is_arriving_now() {
        let mut subject = head_on();
        subject.relative_position = [10.0, 0.0];
        let a = assess(&subject);
        assert!(a.on_collision_course);
        assert_eq!(a.seconds_to_impact, Some(0.0));
    }

    #[test]
    fn an_unprotected_contact_is_never_on_a_collision_course() {
        // A field of harmless wreckage: no protected asset, so no radius, so
        // nothing to confirm — but the projection still reads.
        let subject = DebrisSubject {
            relative_position: [100.0, 0.0],
            relative_velocity: [-10.0, 0.0],
            protected_name: String::new(),
            impact_radius: 0.0,
        };
        let a = assess(&subject);
        assert!(!a.on_collision_course);
        assert_eq!(a.seconds_to_impact, None);
        assert_eq!(a.closest_approach, 0.0);
    }

    #[test]
    fn the_assessment_is_reproducible_from_the_same_inputs() {
        // The lockstep claim, at the pure boundary: the same subject twice is
        // the same bytes twice.
        let subject = head_on();
        assert_eq!(assess(&subject), assess(&subject));
    }

    #[test]
    fn a_protected_contact_with_no_impact_radius_is_refused_at_load() {
        let cfg = DebrisConfig {
            protected_target: "world.probe.entity.depot.name".into(),
            impact_radius: 0.0,
            ..Default::default()
        };
        let err = cfg.validate().expect_err("a hazard that cannot arrive");
        assert!(err.contains("can never arrive"), "got: {err}");
    }

    #[test]
    fn an_unprotected_table_validates_without_a_radius() {
        let cfg = DebrisConfig {
            drift: [1.0, 0.0, 2.0],
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn a_non_finite_drift_is_refused_at_load() {
        let cfg = DebrisConfig {
            drift: [f32::NAN, 0.0, 0.0],
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }
}
