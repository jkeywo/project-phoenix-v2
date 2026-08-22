//! The **Vertical Thrust** helm axis (issue #1208, #744): the `vertical`
//! channel, gating on `ActuateVerticalThrust` and emitting a climb/return
//! dodge scaled by `vertical_hazard_sensitivity`. Stateless, and AI-only —
//! there is no player-facing vertical control.
//!
//! [`ai_helm_vertical_thrust`] is the Bevy system; [`VerticalAxis`] is its
//! [`super::HelmAxisHost`] impl.
//!
//! Invariant: behaviour forks on the hull's authored `VerticalMovementMode`
//! — Planar never commands vertical motion, Bounded climbs up to a ceiling
//! and auto-returns, Full3D climbs unbounded with no auto-return.

use super::*;

use crate::ai::host::HostOutcome;
use crate::ai::policy::AiPolicyVerb;
use crate::core::messages::SystemControlPayload;

/// The **Vertical Thrust** helm axis (issue #1208, issue #744): gate the
/// `vertical` channel on the `actuate_vertical_thrust` mode verb and, on a fire,
/// emit the climb/return the hull's authored [`VerticalMovementMode`] permits. A
/// stateless axis, and AI-only — the vertical axis has no player-facing control.
///
/// - **Planar** — never commands vertical motion (the axis stays at cruise).
/// - **Bounded** — climbs to dodge *moving* hazards up to `max_vertical_offset`,
///   then eases back toward the cruise plane at `vertical_return_rate`.
/// - **Full3D** — the same avoidance climb without the ceiling and with no
///   auto-return.
///
/// The dodge responds to the shared hazard assessment's `moving_hazard_threat`
/// weighted by the hull's authored `vertical_hazard_sensitivity`, so a static
/// obstacle never drives a vertical dodge.
///
/// [`VerticalMovementMode`]: crate::entities::config::VerticalMovementMode
pub(crate) struct VerticalAxis;

impl HelmAxisHost for VerticalAxis {
    fn system_id() -> crate::core::messages::SystemId {
        crate::ship::system_registry::vertical_thrust_system_id()
    }
    const CHANNEL: &'static str = crate::entities::config::HELM_VERTICAL_CHANNEL;
    const STATEFUL: bool = false;

    fn accepts(verb: &AiPolicyVerb) -> bool {
        matches!(verb, AiPolicyVerb::ActuateVerticalThrust)
    }

    fn seed(cx: &HelmAxisCtx) -> crate::world::flags::AiFacts {
        let mut facts = seed_helm_actuator_facts(
            cx.plan.map(|sp| &sp.hazard),
            false,
            false,
            cx.physics.map(|p| p.y).unwrap_or(0.0),
            frame_red_alert(cx.frame),
        );
        // Issue #874: the vertical axis is a dodge axis too — climbing out of a
        // plane of fire is as valid a response as turning out of it.
        seed_hostile_arc_facts(&mut facts, cx.frame);
        facts
    }

    fn act(
        outcome: HostOutcome,
        cx: &HelmAxisCtx,
        _io: &mut HelmAxisIo,
    ) -> Option<SystemControlPayload> {
        use crate::entities::config::VerticalMovementMode;
        match outcome {
            HostOutcome::Act(verb) if Self::accepts(verb) => {}
            _ => return None,
        }
        let physics = cx.physics?;
        let mode = cx
            .capability
            .map(|c| c.0.vertical_movement_mode)
            .unwrap_or_default();

        let vertical = match mode {
            // A planar hull has no vertical axis — hold the cruise plane.
            VerticalMovementMode::Planar => 0.0,
            VerticalMovementMode::Bounded | VerticalMovementMode::Full3D => {
                let sensitivity = cx
                    .behaviour
                    .map(|b| b.0.vertical_hazard_sensitivity)
                    .unwrap_or(crate::ai::VERTICAL_HAZARD_SENSITIVITY);
                let moving_threat = cx
                    .plan
                    .map(|sp| sp.hazard.moving_hazard_threat)
                    .unwrap_or(0.0);
                // Climb to dodge; the initial policy only ever climbs (positive)
                // away from moving hazards sharing the cruise plane.
                let climb = (moving_threat * sensitivity).clamp(0.0, 1.0);

                if climb > f32::EPSILON {
                    match mode {
                        // Bounded: respect the authored ceiling.
                        VerticalMovementMode::Bounded => {
                            let max_offset = cx
                                .capability
                                .map(|c| c.0.max_vertical_offset)
                                .unwrap_or(crate::ai::MAX_VERTICAL_OFFSET);
                            if physics.y >= max_offset {
                                0.0
                            } else {
                                climb
                            }
                        }
                        // Full3D: unbounded vertical DOF, no ceiling.
                        _ => climb,
                    }
                } else {
                    // No moving hazard: Bounded eases back to the cruise plane;
                    // Full3D holds its altitude (no auto-return).
                    match mode {
                        VerticalMovementMode::Bounded => {
                            let return_rate = cx
                                .capability
                                .map(|c| c.0.vertical_return_rate)
                                .unwrap_or(crate::ai::VERTICAL_RETURN_RATE);
                            (-physics.y * return_rate).clamp(-1.0, 1.0)
                        }
                        _ => 0.0,
                    }
                }
            }
        };

        Some(SystemControlPayload::VerticalThrustInput { vertical })
    }
}

/// Per-axis helm AI: vertical thrust (issue #744). Decides the up/down axis for
/// ships whose `helm-vertical-thrust` system is AI-operated and emits it as an
/// admitted `VerticalThrustInput` into the ship's own `AdmittedCommands`,
/// through the same `emit_ai_command` arbiter as the other per-axis operators.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<VerticalAxis>`](run_helm_axis) driver's; this body
/// assembles the per-ship context and emits the payload it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_vertical_thrust(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&crate::entities::spawner::HelmCapabilitySection>,
            Option<&FineSystemAiPolicies>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    for (
        entity,
        sources,
        physics,
        behaviour_section,
        capability,
        fine_policies,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        let cx = HelmAxisCtx {
            physics: Some(physics),
            max_speed: 0.0,
            plan: plan.ships.get(&entity),
            frame: frame.ships.get(&entity),
            anchors: &frame.anchors,
            impulse: None,
            impulse_cfg: None,
            boost_cfg: None,
            boost: None,
            behaviour: behaviour_section,
            capability,
            cursors: None,
        };
        let mut io = HelmAxisIo {
            policy: None,
            state: None,
            pending: None,
        };
        // The scenario flag chain, anchored at the layer that spawned this ship
        // (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(entity);
        if let Some(payload) = run_helm_axis::<VerticalAxis>(
            sources,
            fine_policies.and_then(|p| p.0.get(&VerticalAxis::system_id())),
            None,
            0.0,
            &flag_chain,
            &cx,
            &mut io,
        ) {
            emit_ai_command(
                entity_uuid,
                VerticalAxis::system_id(),
                payload,
                sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
        }
    }
}
