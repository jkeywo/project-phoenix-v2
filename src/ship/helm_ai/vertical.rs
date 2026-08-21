use super::*;

/// Per-ship inline stateless **Vertical Thrust** AI policy (issue #780). From
/// the authored `[helm_console.vertical_ai]` block. Read by
/// [`ai_helm_vertical_thrust`] to decide whether to actuate the climb/return this
/// tick; the authored `VerticalMovementMode` still gates the magnitude host-side.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmVerticalAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-axis helm AI: vertical thrust (issue #744). Decides the up/down axis for
/// ships whose `helm-vertical-thrust` system is AI-operated and emits it as an
/// admitted `VerticalThrustInput` into the ship's own `AdmittedCommands`,
/// through the same `emit_ai_command` arbiter as the other per-axis operators.
///
/// AI-only: the vertical axis has no player-facing control, so this operator is
/// the sole decider of it. Its behaviour is gated on the hull's authored
/// [`VerticalMovementMode`](crate::entity_config::VerticalMovementMode):
///
/// - **Planar** — never commands vertical motion (the axis stays at cruise).
/// - **Bounded** — climbs to dodge *moving* hazards up to the authored
///   `max_vertical_offset`, then eases back toward the cruise plane (`y = 0`) at
///   the authored `vertical_return_rate` once the moving-hazard threat falls
///   (the return is the hysteresis: it only engages when avoidance does not).
/// - **Full3D** — the same avoidance climb without the offset ceiling and with
///   no auto-return, exposing the full vertical degree of freedom.
///
/// The dodge responds to the shared hazard assessment's `moving_hazard_threat`
/// (the planner pre-filters the contribution list to movable hazards, issue
/// #744) weighted by the hull's authored `vertical_hazard_sensitivity` — so a
/// static obstacle, however close, never drives a vertical dodge.
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
            Option<&HelmVerticalAiPolicy>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    use crate::entity_config::VerticalMovementMode;
    for (
        entity,
        sources,
        physics,
        behaviour_section,
        capability,
        vertical_policy,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own axis alone (issue #800), like every per-axis operator.
        if !sources
            .0
            .policy_for(&crate::system_registry::vertical_thrust_system_id())
            .operate_ai
        {
            continue;
        }

        // Authored actuation-policy gate (issue #780, AC1/AC5): the DECISION to
        // actuate the vertical axis flows through HelmVerticalAiPolicy over a
        // fact snapshot seeded from the shared moving-hazard threat and the
        // ship's current vertical offset (for return-to-cruise guards). Its
        // default (unconditional actuate) preserves the pre-#780 behaviour; the
        // authored `VerticalMovementMode` still gates the magnitude below, so a
        // Planar hull takes no Y component regardless of the verb. A "hold"
        // resolution emits nothing.
        let mut facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            false,
            false,
            physics.y,
            frame_red_alert(frame.ships.get(&entity)),
        );
        // Issue #874: the vertical axis is a dodge axis too — climbing out of a
        // plane of fire is as valid a response as turning out of it, and a
        // doctrine cannot author that against a fact this host never seeds.
        seed_hostile_arc_facts(&mut facts, frame.ships.get(&entity));
        // No attached `[helm_console.vertical_ai]` ⇒ no AI action on this axis.
        // Since #885b stage 5d there is no synthesised stand-in: strict
        // AI-declaration mode rejects an AI-capable hull that omits the block at
        // load, so an absent component means the declaration is missing and a
        // missing declaration gets no automation (PRD #774 US7).
        let Some(vertical_policy) = vertical_policy else {
            continue;
        };
        let policy = &vertical_policy.0;
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(entity);
        if !helm_policy_actuates(
            policy,
            crate::entities::config::HELM_VERTICAL_CHANNEL,
            &facts,
            &crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
            &flag_chain,
        ) {
            continue;
        }

        let mode = capability
            .map(|c| c.0.vertical_movement_mode)
            .unwrap_or_default();

        let vertical = match mode {
            // A planar hull has no vertical axis — hold the cruise plane.
            VerticalMovementMode::Planar => 0.0,
            VerticalMovementMode::Bounded | VerticalMovementMode::Full3D => {
                let sensitivity = behaviour_section
                    .map(|b| b.0.vertical_hazard_sensitivity)
                    .unwrap_or(crate::ai::VERTICAL_HAZARD_SENSITIVITY);
                let moving_threat = plan
                    .ships
                    .get(&entity)
                    .map(|sp| sp.hazard.moving_hazard_threat)
                    .unwrap_or(0.0);
                // Climb to dodge; the initial policy only ever climbs (positive)
                // away from moving hazards sharing the cruise plane.
                let climb = (moving_threat * sensitivity).clamp(0.0, 1.0);

                if climb > f32::EPSILON {
                    match mode {
                        // Bounded: respect the authored ceiling — stop climbing
                        // once at/above the max offset from cruise.
                        VerticalMovementMode::Bounded => {
                            let max_offset = capability
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
                            let return_rate = capability
                                .map(|c| c.0.vertical_return_rate)
                                .unwrap_or(crate::ai::VERTICAL_RETURN_RATE);
                            (-physics.y * return_rate).clamp(-1.0, 1.0)
                        }
                        _ => 0.0,
                    }
                }
            }
        };

        emit_ai_command(
            entity_uuid,
            crate::system_registry::vertical_thrust_system_id(),
            crate::messages::SystemControlPayload::VerticalThrustInput { vertical },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}
