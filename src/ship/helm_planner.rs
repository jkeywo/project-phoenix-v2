//! Shared desired-motion planner (issue #741, PRD #735).
//!
//! The planner is the single ship-level planning pass that sits **in front of**
//! the per-axis helm AI. Once per shared AI-helm sim tick it turns a ship's
//! objective travel decision (`operate_helm`, via `helm_ai_decision`) plus its
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
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    plan.ships.clear();

    for (entity, physics, behaviour_section, cursors, capability, mut docking_intent) in
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

        // Objective travel decision: the same pure `operate_helm` call the
        // per-axis systems used to each make, made once here. No objective ->
        // hold (zero throttle, face forward), exactly as the per-axis
        // no-objective branch did.
        let (thrust, steering) = if sf.has_objective {
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
        } else {
            (0.0, 0.0)
        };

        // Ship-level hazard assessment over the radar-gated visible view.
        let avoidance_buffer = behaviour_section
            .map(|b| b.0.avoidance_buffer)
            .unwrap_or(crate::ai::AVOIDANCE_BUFFER);
        let avoidance_look_ahead = behaviour_section
            .map(|b| b.0.avoidance_look_ahead_secs)
            .unwrap_or(crate::ai::AVOIDANCE_LOOK_AHEAD_SECS);
        let hazard_raw = crate::ai::assess_hazards(
            &sf.merged_view,
            physics.forward_speed,
            avoidance_buffer,
            avoidance_look_ahead,
        );

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

        let hazard = HazardAssessment {
            hazard_forces: Vec3::from_array(hazard_raw.forces_local),
            urgency: hazard_raw.urgency,
            primary_hazard: hazard_raw.primary,
        };

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
