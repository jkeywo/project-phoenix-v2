//! The pure, Bevy-free heart of helm docking (issue #1159).
//!
//! Two things live here and nothing else: the **marker-mating module**
//! ([`nearest_viable_pair`]) — given the two hulls' poses and their dock markers,
//! which pair is nearest and viable and what pose mates them — and the authored
//! `[dock]` terms ([`DockConfig`]) with the refusal vocabulary
//! ([`DockRefusal`]). The sibling [`crate::dock::server`] adapter gathers the
//! live world, calls in, and applies what comes back, deciding nothing itself
//! (AGENTS.md rule 10).
//!
//! # Why this is a module of its own, Bevy-free
//!
//! The geometry — which two markers mate, and the pose the OWN ship must adopt so
//! its dock marker meets the target's and faces it — is decided here in isolation
//! and unit-tested here, with plain `glam` types and no app, world or schedule.
//! Bevy re-exports `glam`, so a `Transform`'s `translation`/`rotation` ARE these
//! types and the adapter passes them straight in; nothing here imports `bevy`.
//! `glam` is pinned with the `libm` feature (see `Cargo.toml`), the same backing
//! `simmath` uses, so this maths agrees bit-for-bit on native and wasm.
//!
//! # The mating transform
//!
//! Two dock markers mate when they sit at the same world point and their
//! outward directions oppose — the two hulls meet nose-plate to nose-plate. The
//! own ship's marker is authored in the ship's own frame; the target's marker,
//! resolved to world, is a fixed point the own ship must bring its marker onto.
//! So the mate pose is the ship pose whose marker-direction is the negation of
//! the target marker's world direction, translated so the two marker points
//! coincide. Facing is planar (yaw about +Y) like every current hull, so the
//! mate is a yaw and a translation.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// The authored dock terms for a hull's `[dock]` table (issue #1159).
///
/// Every field is a designer's number, read from TOML: AGENTS.md rule 11, no
/// hardcoded gameplay values. A hull that authors no `[dock]` table carries no
/// [`crate::dock::server::DockMarkers`] and no [`crate::dock::server::DockControl`]
/// component and is unchanged in every way — it can neither dock nor be docked
/// with. The **power group** the dock draws from is NOT here — it is the
/// `power_group` field of the dock `[[system]]` block, the one authoritative
/// place a system names its group — and the adapter resolves it at spawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockConfig {
    /// The furthest a viable dock-marker pair may sit and still offer the dock
    /// control, and the separation past which a forming or held dock parts, in
    /// world units. The contextual control appears only while the nearest viable
    /// pair is within this reach.
    pub range: f32,
    /// The distance within which the close-in manoeuvre engages
    /// ([`crate::ai::docking_close_manoeuvre`]). Authored `>= range` so the
    /// manoeuvre is live for the whole of the dock approach.
    pub engage_distance: f32,
    /// How fast the own ship closes on the mate pose, in world units per second.
    /// The manoeuvre is a low-speed mate, never a ram.
    pub approach_speed: f32,
    /// The marker-pair separation under which the two hulls count as mated and
    /// the docked relationship forms, in world units.
    pub mate_tolerance: f32,
    /// How far the own ship backs straight out along its dock marker's outward
    /// direction on undock before it returns to ordinary flight, in world units.
    pub undock_clear_distance: f32,
    /// The lowest power-group level at which the dock manoeuvre runs. Below it a
    /// forming or held dock parts ([`DockRefusal::Unpowered`]). Authored, not
    /// derived from the group's nominal rung.
    pub min_power_level: u8,
}

impl DockConfig {
    /// Reject an authored `[dock]` table that describes a dock that could never
    /// mate (issue #1159). Non-positive distances, an engage distance shorter
    /// than the range (which would leave the manoeuvre idle inside dock range),
    /// or a zero minimum power level are author mistakes whose only other
    /// symptom would be a control the crew can press that never completes.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("range", self.range),
            ("engage_distance", self.engage_distance),
            ("approach_speed", self.approach_speed),
            ("mate_tolerance", self.mate_tolerance),
            ("undock_clear_distance", self.undock_clear_distance),
        ] {
            if value.is_nan() || value <= 0.0 {
                return Err(format!(
                    "[dock] {name} must be a positive distance, got {value}"
                ));
            }
        }
        if self.engage_distance < self.range {
            return Err(format!(
                "[dock] engage_distance ({}) must be >= range ({}) so the close-in manoeuvre is \
                 live for the whole dock approach",
                self.engage_distance, self.range
            ));
        }
        if self.mate_tolerance > self.range {
            return Err(format!(
                "[dock] mate_tolerance ({}) must be <= range ({}) — a pair cannot mate further \
                 out than the control ever offers",
                self.mate_tolerance, self.range
            ));
        }
        if self.min_power_level == 0 {
            return Err(
                "[dock] min_power_level must be at least 1 — a dock that runs at level 0 would \
                 never lose its allocation"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// The one reason a dock did not form (or ended) this tick (issue #1159), as the
/// console shows it — a `strings.csv` id, never English. Copies the tractor's
/// refusal-plus-`string_id` shape ([`crate::tractor::TractorRefusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockRefusal {
    /// No viable dock-marker pair exists — the own hull, the target, or both
    /// declare no dock markers, so nothing can ever mate. This is the "a hull
    /// that declares none can never be docked with, and says so" reason.
    NoDockMarkers,
    /// No hull carrying dock markers sits within the authored range.
    NoTarget,
    /// The chosen pair drifted past the authored `range` while forming or held.
    OutOfRange,
    /// The dock's power group is below the authored `min_power_level`.
    Unpowered,
    /// The dock system is damaged to `Disabled` (or `Destroyed`).
    Disabled,
    /// The docked target no longer exists — destroyed, or otherwise despawned.
    TargetLost,
}

impl DockRefusal {
    /// The `strings.csv` id the console resolves through `t()`. A `match`, not a
    /// composed id, so `check-strings.mjs` can see every id a new variant needs a
    /// row for.
    pub fn string_id(self) -> &'static str {
        match self {
            DockRefusal::NoDockMarkers => "dock.refused.no_markers",
            DockRefusal::NoTarget => "dock.refused.no_target",
            DockRefusal::OutOfRange => "dock.refused.out_of_range",
            DockRefusal::Unpowered => "dock.refused.unpowered",
            DockRefusal::Disabled => "dock.refused.disabled",
            DockRefusal::TargetLost => "dock.refused.target_lost",
        }
    }
}

/// A single dock marker in a hull's OWN frame (issue #1159): where the mate
/// point sits on the hull and which way it faces outward. Lifted from the rig
/// [`crate::entities::model_rig::Marker`] vocabulary — the same `[markers.<name>]` blocks
/// that carry engines and hardpoints — with the base rig already folded in by
/// the adapter, so these are ship-local points a `Transform` maps to world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockMarker {
    /// The mount point in the hull's own frame.
    pub position: Vec3,
    /// The outward-facing unit direction in the hull's own frame (forward basis
    /// `(0,0,-1)`, as the rig authors it).
    pub direction: Vec3,
}

/// A rigid pose — a translation and a rotation — the adapter reads off a Bevy
/// `Transform` and passes straight in, and reads back onto one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub translation: Vec3,
    pub rotation: Quat,
}

/// The chosen dock-marker pair and the pose that mates them (issue #1159).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatingSolution {
    /// Index of the own hull's marker in the pair.
    pub own_marker: usize,
    /// Index of the target hull's marker in the pair.
    pub target_marker: usize,
    /// The pose the OWN ship must adopt so its chosen marker meets the target's
    /// and faces it. Yaw about +Y and a translation — facing is planar.
    pub mate: Pose,
    /// The current world distance between the two chosen markers, before the
    /// mate. This is what the range test and the nearest-pair selection read.
    pub separation: f32,
}

/// The yaw of a horizontal direction, in the ship convention `0 = -Z forward`
/// (the same convention `ShipPhysics::yaw` uses). Returns `None` for a direction
/// with no usable horizontal component — a purely vertical marker a planar mate
/// cannot orient against.
fn horizontal_yaw(dir: Vec3) -> Option<f32> {
    let (x, z) = (dir.x, dir.z);
    if x * x + z * z <= 1e-8 {
        return None;
    }
    Some(simmath_atan2(x, -z))
}

/// `atan2` routed through the determinism-pinned `simmath`, so the mate yaw a
/// host computes agrees bit-for-bit across native and wasm — the same backing
/// the rest of the flight maths uses.
fn simmath_atan2(y: f32, x: f32) -> f32 {
    crate::simmath::atan2(y, x)
}

/// **The marker-mating module.** Pick the nearest viable dock-marker pair across
/// the two hulls and answer the pose that mates it (issue #1159).
///
/// Pure: the adapter resolves each hull's dock markers into its own frame (base
/// rig folded in), reads both hulls' live poses off their `Transform`s, and
/// passes them here. For every own-marker × target-marker pair it resolves both
/// markers to world, measures their current separation, and keeps the nearest
/// **viable** one — viable meaning both markers have a usable horizontal
/// direction, so a planar mate yaw exists. The mate pose for that pair is the
/// ship pose whose marker direction opposes the target marker's world direction,
/// translated so the two marker points coincide.
///
/// Returns `None` when either hull declares no dock markers, or no pair is
/// viable — the adapter turns that into [`DockRefusal::NoDockMarkers`]. A hull
/// that declares none can never be docked with.
pub fn nearest_viable_pair(
    own: Pose,
    own_markers: &[DockMarker],
    target: Pose,
    target_markers: &[DockMarker],
) -> Option<MatingSolution> {
    let mut best: Option<MatingSolution> = None;
    for (oi, om) in own_markers.iter().enumerate() {
        let own_dir_local_yaw = match horizontal_yaw(om.direction) {
            Some(y) => y,
            None => continue,
        };
        let own_marker_world = own.translation + own.rotation * om.position;
        for (ti, tm) in target_markers.iter().enumerate() {
            let target_dir_world = target.rotation * tm.direction;
            // The direction the own marker must face is the negation of the
            // target marker's world direction — the two plates meet head on.
            let desired_yaw = match horizontal_yaw(-target_dir_world) {
                Some(y) => y,
                None => continue,
            };
            let target_marker_world = target.translation + target.rotation * tm.position;
            let separation = own_marker_world.distance(target_marker_world);
            if best.map(|b| separation < b.separation).unwrap_or(true) {
                // Ship yaw so that the own marker's local direction (at local yaw
                // `own_dir_local_yaw`) rotates to the desired world yaw.
                let ship_yaw = desired_yaw - own_dir_local_yaw;
                let mate_rotation = Quat::from_rotation_y(ship_yaw);
                let mate_translation = target_marker_world - mate_rotation * om.position;
                best = Some(MatingSolution {
                    own_marker: oi,
                    target_marker: ti,
                    mate: Pose {
                        translation: mate_translation,
                        rotation: mate_rotation,
                    },
                    separation,
                });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
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
            let own_marker_world =
                sol.mate.translation + sol.mate.rotation * own_markers[0].position;
            let target_marker_world =
                target.translation + target.rotation * target_markers[0].position;
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
}
