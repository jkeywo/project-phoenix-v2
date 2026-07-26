//! Shared desired-motion planner (issue #741, PRD #735).
//!
//! The planner is the single ship-level planning pass that sits **in front of**
//! the per-axis helm AI. Once per shared AI-helm sim tick it turns a ship's
//! objective travel decision (`plan_helm_travel`, via `helm_ai_decision`) plus its
//! world hazards into a 3D **desired-motion contract** — a desired velocity and
//! a desired *facing*, kept separate so orientation can diverge from travel —
//! and a **hazard assessment** (repulsion force, urgency, primary hazard).
//!
//! It publishes both into [`HelmMotionPlan`], keyed by ship entity. The per-axis
//! `ai_helm_thrust` / `ai_helm_steering` systems then read that shared surface
//! (decoding their own actuator scalar from it) instead of each re-deriving the
//! decision — so both axes observe one plan, and the human and AI paths still
//! converge on the same admitted actuator input downstream (nothing here
//! branches on controller identity: AGENTS.md rule 6).
//!
//! The contract is deliberately 3D even though physics is planar today (issue
//! #741): baking planar assumptions in before bounded / full-3D craft arrive is
//! the thing the shared surface exists to avoid. The vertical axis stays 0 for
//! `Planar` hulls and is gated on the ship's authored
//! [`VerticalMovementMode`](crate::entity_config::VerticalMovementMode).

use bevy::prelude::*;

use crate::entity_config::VerticalMovementMode;
use crate::ship::helm_ai::{helm_ai_decision, HelmAiSurfacesFrame};
use crate::ship_state::ShipPhysics;

/// A ship's shared desired-motion contract: where it wants to go and where it
/// wants to point, both in the ship's local frame (`x` = starboard, `y` = up,
/// `z` = aft; forward travel is `-Z`). Facing is carried separately from
/// velocity so arc-bearing / docking can turn the ship without hijacking travel
/// (issue #741).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DesiredMotion {
    /// Desired velocity in the ship-local frame. The forward (`-Z`) component is
    /// the normalized throttle intent `[-1, 1]`.
    pub desired_velocity_local: Vec3,
    /// Desired facing as a ship-local unit direction.
    pub desired_facing_local: Vec3,
}

/// A ship's shared hazard assessment (issue #741): a boids-style repulsion
/// contribution, the peak avoidance urgency, and the strongest threat's
/// identity. A published fact, not a direct actuator order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HazardAssessment {
    /// Aggregate repulsion in the ship-local frame, pointing away from projected
    /// collisions.
    pub hazard_forces: Vec3,
    /// Peak threat fraction across all hazards, `[0, 1]`.
    pub urgency: f32,
    /// The strongest threat's UUID, if any.
    pub primary_hazard: Option<uuid::Uuid>,
    /// Peak threat fraction across only the **moving** (`movable`) hazards,
    /// `[0, 1]` (issue #744). The vertical-thrust actuator's initial policy
    /// dodges moving hazards only — static obstacles are left to the planar
    /// actuators — so the planner pre-filters the contribution list to movable
    /// hazards here rather than forwarding the whole (non-`Copy`) list.
    pub moving_hazard_threat: f32,
}

/// One ship's full plan for this tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShipMotionPlan {
    pub motion: DesiredMotion,
    pub hazard: HazardAssessment,
    /// True when a docking close manoeuvre (issue #742) authored this tick's
    /// `desired_velocity_local` — its lateral (`x`) and reverse (`+z`)
    /// components are a sanctioned docking translation, not objective travel.
    /// The lateral-thrust AI reads it to know the plan owns the lateral axis
    /// this tick (the controlled drift arc-bearing may never command).
    pub docking_active: bool,
}

/// Per-tick shared desired-motion + hazard surface, keyed by ship entity.
/// Rebuilt wholesale (cleared + refilled) by [`helm_motion_planner`] every
/// shared AI-helm sim tick and consumed read-only by the per-axis helm AI.
#[derive(Resource, Default)]
pub struct HelmMotionPlan {
    pub ships: std::collections::HashMap<Entity, ShipMotionPlan>,
}

/// Assemble the shared desired-motion + hazard surface once per shared AI-helm
/// sim tick (issue #741). Runs `.after(build_helm_ai_surfaces_frame)` (whose
/// decision inputs it reads) and `.before` the per-axis helm AI (which consumes
/// its output), under the same `run_if(ai_helm_tick_ready)` gate.
///
/// Consumes the ship's authored [`HelmCapabilitySection`] when present, falling
/// back to `Planar` / default tuning otherwise — no shipped hull authors
/// `[helm_capability]` yet.
///
/// [`HelmCapabilitySection`]: crate::entities::spawner::HelmCapabilitySection
#[allow(clippy::too_many_arguments)]
pub(crate) fn helm_motion_planner(
    frame: Res<HelmAiSurfacesFrame>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
    mut plan: ResMut<HelmMotionPlan>,
    mut ships: Query<
        (
            Entity,
            &ShipPhysics,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&crate::ai_plugin::ObjectiveCursors>,
            Option<&crate::entities::spawner::HelmCapabilitySection>,
            // Docking intent (issue #742). Mutable because the planner — which
            // owns docking-motion-intent-state — clears it the moment its dock
            // target leaves the merged view (expiry), mirroring how
            // arc-bearing self-clears.
            Option<&mut crate::ship::components::DockingMotionIntent>,
            // The derived fly-through pass surface (issue #883), published by
            // `ai_policy_state_tick` from the ship's own Engines/Steering
            // policies. Read-only here: the planner selects a decision arm from
            // it, it decides nothing itself.
            Option<&crate::ship::helm_ai::HelmPassSurface>,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    plan.ships.clear();

    for (entity, physics, behaviour_section, cursors, capability, mut docking_intent, pass) in
        ships.iter_mut()
    {
        // Only ships some helm axis is actually flying carry a frame entry;
        // the frame is the shared decision surface built this same tick.
        let Some(sf) = frame.ships.get(&entity) else {
            continue;
        };

        let vertical_mode = capability
            .map(|c| c.0.vertical_movement_mode)
            .unwrap_or_default();

        // Ship-level hazard assessment over the radar-gated visible view.
        let avoidance_buffer = behaviour_section
            .map(|b| b.0.avoidance_buffer)
            .unwrap_or(crate::ai::AVOIDANCE_BUFFER);
        let avoidance_look_ahead = behaviour_section
            .map(|b| b.0.avoidance_look_ahead_secs)
            .unwrap_or(crate::ai::AVOIDANCE_LOOK_AHEAD_SECS);
        // Authored ignore-smaller rule (issue #743): a ship ignores hazards
        // below its own size rating, scaled by this ratio. `0.0` (the serde
        // default) leaves every dangerous hazard in the picture.
        let hazard_ignore_size_ratio = behaviour_section
            .map(|b| b.0.hazard_ignore_size_ratio)
            .unwrap_or(crate::ai::HAZARD_IGNORE_SIZE_RATIO);
        let hazard_raw = crate::ai::assess_hazards(
            &sf.merged_view,
            physics.forward_speed,
            avoidance_buffer,
            avoidance_look_ahead,
            hazard_ignore_size_ratio,
        );

        // ── Objective travel decision ────────────────────────────────────────
        //
        // Normally the same pure `plan_helm_travel` call the per-axis systems
        // used to each make, made once here. No objective -> hold (zero
        // throttle, face forward), exactly as the per-axis no-objective branch
        // did.
        //
        // A ship running an authored FLY-THROUGH pass (issue #883) takes a
        // different pure arm instead: `plan_fly_through_pass`. That is a
        // deliberate substitution rather than a mode flag on `helm_destroy` —
        // `helm_destroy` brakes through a decel zone and parks at `stop_dist`
        // re-facing the target, which is a station-keeping orbit and the exact
        // opposite of a pass. Bending it into both shapes would have made one
        // function answer two incompatible questions.
        //
        // Crucially the substitution happens HERE, in the planner, so the escape
        // leg's frozen heading arrives as a DESIRED FACING. Everything
        // downstream — the hazard force, the imminent-collision override below,
        // the per-axis actuators — composes onto it unchanged (AC3), and the
        // pass state is not an input to any of them.
        //
        // ## Only the INBOUND leg needs a target
        //
        // The inbound leg is a tracking solution: without a target it can
        // actually resolve in this tick's merged view there is nothing to track,
        // so it falls back to ordinary doctrine travel.
        //
        // The escape leg is target-FREE by construction — `plan_fly_through_pass`
        // never reads `target_pos` on it, because the heading was frozen at the
        // merge. Gating it on a live, visible target would be a silent
        // dependency on the very thing the pass exists to destroy: the common
        // combat case is that the run KILLS the target, and the ship would then
        // drop back to the doctrine arm with no objective geometry, brake to a
        // standstill, and stop holding the frozen heading — while the Boost
        // machine, which is independent of the planner, kept the drive lit for
        // the rest of the escape dwell. Neither escape state has a
        // `target_valid < 1` transition, precisely because the authored doctrine
        // says the target may "turn, run, or die — the escape does not care".
        let pass = pass.copied().unwrap_or_default();
        let pass_target = if pass.active {
            sf.destroy_target
                .or(sf.weapons_target)
                .and_then(|uuid| sf.merged_view.entities.iter().find(|e| e.uuid == uuid))
        } else {
            None
        };
        let fly_pass =
            |leg: crate::ai::FlyThroughLeg, target_pos: [f32; 3], target_uuid: uuid::Uuid| {
                crate::ai::plan_fly_through_pass(&crate::ai::FlyThroughPassInput {
                    leg,
                    self_pos: [physics.x, physics.y, physics.z],
                    self_yaw: physics.yaw,
                    self_speed: physics.forward_speed,
                    self_radius: sf.merged_view.self_radius,
                    target_pos,
                    target_uuid,
                    escape_heading_rad: pass.escape_heading_rad,
                    approach_speed: pass.approach_speed,
                    escape_speed: pass.escape_speed,
                    reengage_speed: pass.reengage_speed,
                    tracking_deadband_rad: pass.tracking_deadband_rad,
                    tracking_full_steer_rad: pass.tracking_full_steer_rad,
                    entities: &sf.merged_view.entities,
                    avoidance_buffer,
                    avoidance_look_ahead_secs: avoidance_look_ahead,
                })
            };
        // The ring legs — the shield-recovery standoff (issue #788) and the
        // combat broadside orbit (issue #790). Like the pass legs this is a
        // substitution *in the planner*, so the orbit arrives downstream as an
        // ordinary desired facing and the hazard force, the imminent-collision
        // override, and the per-axis actuators all compose onto it unchanged.
        //
        // ONE closure, three authored scalars passed in: the geometry is the
        // same tangent-of-a-ring solution in both cases and duplicating it would
        // let the two drift apart. What differs is only WHICH radius, throttle
        // and gain the surface published, and that difference is the doctrine's,
        // not the geometry's. The circulation direction is shared outright —
        // a ship circles one way at a time.
        let fly_orbit = |target_pos: [f32; 3],
                         target_uuid: uuid::Uuid,
                         ring_radius: f32,
                         ring_speed: f32,
                         spiral_gain: f32| {
            crate::ai::plan_recovery_orbit(&crate::ai::RecoveryOrbitInput {
                self_pos: [physics.x, physics.y, physics.z],
                self_yaw: physics.yaw,
                self_speed: physics.forward_speed,
                self_radius: sf.merged_view.self_radius,
                target_pos,
                target_uuid,
                safe_range: ring_radius,
                orbit_direction: pass.orbit_direction,
                spiral_gain,
                orbit_speed: ring_speed,
                tracking_deadband_rad: pass.tracking_deadband_rad,
                tracking_full_steer_rad: pass.tracking_full_steer_rad,
                entities: &sf.merged_view.entities,
                avoidance_buffer,
                avoidance_look_ahead_secs: avoidance_look_ahead,
            })
        };
        // Ordinary doctrine travel, byte-identical to the pre-#883 path.
        let doctrine_travel = || {
            helm_ai_decision(
                &sf.merged_view,
                &sf.scored,
                behaviour_section,
                &frame.anchors,
                cursors,
                sf.weapons_target,
                sf.destroy_target,
                sf.nav_waypoint,
                sf.forward_speed,
            )
        };
        let (thrust, steering) = match (sf.has_objective, pass.active) {
            // Escape: flown from the frozen heading alone, whether or not the
            // target still exists or is still visible.
            (true, true) if pass.escape => fly_pass(
                crate::ai::FlyThroughLeg::Escape,
                // Unread on this leg. Our own position, so the field can never
                // be a stale or invented world point even in a future misuse.
                [physics.x, physics.y, physics.z],
                // Used ONLY to exclude the target from the avoidance scan. A
                // dead or unseen target excludes nothing, which is correct — it
                // is no longer in `entities` either.
                pass_target.map(|t| t.uuid).unwrap_or_else(uuid::Uuid::nil),
            ),
            // Recovery orbit: a ring has a CENTRE, so this leg needs a
            // resolvable target the same way the inbound leg does. Without one
            // there is nothing to stand off from, and the doctrine's own
            // `safe_distance_held` reading has already gone true, so the machine
            // is about to leave the state anyway.
            (true, true) if pass.recover => pass_target.map_or_else(doctrine_travel, |target| {
                fly_orbit(
                    target.position,
                    target.uuid,
                    pass.safe_range,
                    pass.orbit_speed,
                    pass.orbit_spiral_gain,
                )
            }),
            // Combat broadside orbit (issue #790): the same ring geometry at the
            // hull's OWN authored fighting radius. Needs a resolvable target for
            // the same reason the recovery ring does — a ring has a centre — and
            // falls back to ordinary doctrine travel without one, which is where
            // the hull's own machine is heading anyway (its orbit state's only
            // exit is `target_valid < 1`).
            (true, true) if pass.combat_orbit => {
                pass_target.map_or_else(doctrine_travel, |target| {
                    fly_orbit(
                        target.position,
                        target.uuid,
                        pass.combat_orbit_range,
                        pass.combat_orbit_speed,
                        pass.combat_orbit_spiral_gain,
                    )
                })
            }
            // Re-entry pivot and inbound: both track the target, so both need a
            // resolvable one. They differ only in the authored throttle the leg
            // carries, which is what makes the pivot a cut-thrust turn rather
            // than the start of the run.
            //
            // This is the FALLBACK arm — every state that resolves no other leg
            // verb lands here — so it is gated on the hull actually authoring
            // the pass throttles (issue #790). A hull that flies only a combat
            // orbit has no `approach_speed`, and running its approach at an
            // unauthored zero would be a ship coasting into a fight; it takes
            // ordinary doctrine travel instead.
            (true, true) if pass.pass_legs => pass_target.map_or_else(doctrine_travel, |target| {
                fly_pass(
                    if pass.reengage {
                        crate::ai::FlyThroughLeg::Reengage
                    } else {
                        crate::ai::FlyThroughLeg::Inbound
                    },
                    target.position,
                    target.uuid,
                )
            }),
            // No leg authored / not active / no leg selected this tick.
            (true, _) => doctrine_travel(),
            (false, _) => (0.0, 0.0),
        };

        // Vertical intent is gated on the ship's authored movement mode: a
        // `Planar` hull can never be handed a vertical component. Bounded /
        // full-3D craft may take vertical avoidance from the hazard force —
        // which is 0 while hazards are planar, but the gate is real now.
        let vertical = match vertical_mode {
            VerticalMovementMode::Planar => 0.0,
            VerticalMovementMode::Bounded | VerticalMovementMode::Full3D => {
                hazard_raw.forces_local[1]
            }
        };

        let mut motion = DesiredMotion {
            desired_velocity_local: Vec3::from_array(crate::ai::encode_local_velocity(
                thrust, vertical,
            )),
            desired_facing_local: Vec3::from_array(crate::ai::encode_local_facing(steering)),
        };

        // ── Docking close manoeuvre (issue #742) ─────────────────────────────
        // A distinct intent from arc-bearing: it may translate the hull with
        // controlled reverse (`+z`) and lateral (`x`) — the motions arc-bearing
        // (facing-only) must never command. Gated on a live objective like
        // arc-bearing (the merged view is only built then), and cleared here
        // the moment its dock target is no longer visible (AC4 expiry).
        let mut docking_active = false;
        if sf.has_objective {
            if let Some(intent) = docking_intent.as_deref_mut() {
                if let Some(dock_uuid) = intent.0 {
                    match sf.merged_view.entities.iter().find(|e| e.uuid == dock_uuid) {
                        Some(dock) => {
                            let engage = behaviour_section
                                .map(|b| b.0.docking_engage_distance)
                                .unwrap_or(crate::ai::DOCKING_ENGAGE_DISTANCE);
                            let speed = behaviour_section
                                .map(|b| b.0.docking_approach_speed)
                                .unwrap_or(crate::ai::DOCKING_APPROACH_SPEED);
                            if let Some([lateral, aft]) = crate::ai::docking_close_manoeuvre(
                                physics.x,
                                physics.z,
                                physics.yaw,
                                dock.position[0],
                                dock.position[2],
                                engage,
                                speed,
                            ) {
                                // Overwrite the travel axes (facing untouched):
                                // docking translates the hull onto the berth.
                                motion.desired_velocity_local.x = lateral;
                                motion.desired_velocity_local.z = aft;
                                docking_active = true;
                            }
                        }
                        // Dock target gone (despawned / out of radar): expire the
                        // intent so the manoeuvre never outlives its target.
                        None => intent.0 = None,
                    }
                }
            }
        }

        // Moving-hazard threat for the vertical actuator (issue #744): the peak
        // threat fraction among only the movable contributions. Static
        // obstacles (asteroids etc.) never drive vertical avoidance.
        let moving_hazard_threat = hazard_raw
            .contributions
            .iter()
            .filter(|c| c.movable)
            .map(|c| c.threat_fraction)
            .fold(0.0_f32, f32::max);

        let hazard = HazardAssessment {
            hazard_forces: Vec3::from_array(hazard_raw.forces_local),
            urgency: hazard_raw.urgency,
            primary_hazard: hazard_raw.primary,
            moving_hazard_threat,
        };

        // ── Imminent-collision facing override (issue #780, AC4) ──────────────
        // Ordinary avoidance BENDS travel (the hazard force is a velocity
        // contribution the actuators read) without ever touching the active
        // doctrine or the desired facing. ONLY an imminent collision — hazard
        // urgency at or above the hull's AUTHORED threshold — may TEMPORARILY
        // override desired facing, turning the hull along the escape heading so
        // it can thrust clear. The override is stateless: it is recomputed from
        // this tick's urgency and evaporates the moment urgency drops back under
        // the threshold (like docking / arc-bearing self-clear), so no lifecycle
        // state is introduced. The threshold defaults to 1.0 (effectively off)
        // so no shipped hull changes behaviour until it opts in.
        let imminent_threshold = behaviour_section
            .map(|b| b.0.imminent_collision_facing_threshold)
            .unwrap_or(crate::ai::IMMINENT_COLLISION_FACING_THRESHOLD);
        if hazard.urgency >= imminent_threshold {
            // Escape heading = the horizontal repulsion direction (ship-local
            // x = starboard, z = aft). Facing is planar for every current hull,
            // so the vertical component is dropped here regardless of movement
            // mode — the planar-facing contract is unchanged.
            let escape = Vec3::new(hazard.hazard_forces.x, 0.0, hazard.hazard_forces.z);
            if escape.length_squared() > f32::EPSILON {
                motion.desired_facing_local = escape.normalize();
            }
        }

        if hazard.urgency > 0.0 {
            crate::pdebug!(
                log,
                crate::logging::LogCat::Helm,
                entity = entity,
                "helm planner: urgency={:.2} primary={:?}",
                hazard.urgency,
                hazard.primary_hazard
            );
        }

        plan.ships.insert(
            entity,
            ShipMotionPlan {
                motion,
                hazard,
                docking_active,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The planner encodes a forward Reach into a forward desired velocity and,
    /// for an anchor off the starboard bow, a starboard desired facing — with
    /// facing carried as a distinct field from velocity (issue #741).
    #[test]
    fn encodes_forward_travel_and_independent_facing() {
        // Reuse the pure codec directly: a positive throttle -> negative local-Z
        // velocity; a positive (starboard) steering -> +X facing.
        let motion = DesiredMotion {
            desired_velocity_local: Vec3::from_array(crate::ai::encode_local_velocity(0.6, 0.0)),
            desired_facing_local: Vec3::from_array(crate::ai::encode_local_facing(0.5)),
        };
        assert!(
            motion.desired_velocity_local.z < 0.0,
            "forward throttle must be a negative local-Z velocity"
        );
        assert!(
            motion.desired_facing_local.x > 0.0,
            "a starboard turn must point the desired facing to +X"
        );
        // Facing and travel are genuinely separate axes of the contract.
        assert_ne!(
            motion.desired_facing_local, motion.desired_velocity_local,
            "facing must be represented separately from travel"
        );
    }
}
