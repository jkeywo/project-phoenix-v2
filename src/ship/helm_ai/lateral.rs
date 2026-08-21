use super::*;

/// Per-ship inline stateless **Lateral Thrust** AI policy (issue #780). From
/// the authored `[helm_console.lateral_ai]` block. Read by
/// [`ai_helm_lateral_thrust`] to decide whether to actuate the dodge this tick;
/// the continuous magnitude still comes from the shared hazard surface.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmLateralAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-axis helm AI: lateral thrust. Decides the dodge for ships whose
/// helm-lateral-thrust system is AI-operated and emits it as an admitted
/// `LateralThrustInput` into the ship's own `AdmittedCommands` (issues #703,
/// #704, #824).
///
/// Since #743 the dodge is no longer re-derived here: it reads the shared
/// hazard assessment the planner published in `HelmMotionPlan` (the ship-level
/// `assess_hazards` surface built from the hull's authored avoidance tuning)
/// and weights its starboard repulsion by this hull's authored
/// `lateral_hazard_sensitivity`. Docking translation still overrides it (issue
/// #742), and the emit → admit → apply arbiter path is unchanged.
pub(crate) fn ai_helm_lateral_thrust(
    // Read-only scenario flag/counter chain (issue #891 stage 2). `Option` so
    // bare-`App` fixtures still pass parameter validation.
    world_runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            // Optional, so it does not filter the iteration set: a ship without
            // a `[behaviour]` section still runs AI lateral thrust, on the
            // `crate::ai::*` fallbacks that match the serde defaults.
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&HelmLateralAiPolicy>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (
        entity,
        sources,
        behaviour_section,
        lateral_policy,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own system alone (issue #703) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::lateral_thrust_system_id())
            .operate_ai
        {
            continue;
        }
        let Some(sf) = frame.ships.get(&entity) else {
            continue;
        };

        // The ship's plan for this tick: both the docking translation (issue
        // #742) and the shared hazard surface (issue #743) are read off it, so
        // the human and AI paths stay symmetric downstream (the planner is the
        // single writer of both).
        let ship_plan = plan.ships.get(&entity);

        // A docking close manoeuvre (issue #742), when the planner engaged one
        // this tick, owns the lateral axis: its controlled translation is the
        // sanctioned use of lateral thrust, distinct from the avoidance dodge
        // below and from the facing-only arc-bearing request. Read straight off
        // the shared desired-motion contract's `x`. This is an UNCONDITIONAL
        // sanctioned override (issue #780): it precedes the policy gate so a
        // docking hull always translates onto its berth.
        let docking_lateral = ship_plan
            .filter(|sp| sp.docking_active)
            .map(|sp| sp.motion.desired_velocity_local.x);

        // Authored actuation-policy gate (issue #780, AC1/AC3): outside a docking
        // manoeuvre, the DECISION to actuate the dodge flows through
        // HelmLateralAiPolicy over a fact snapshot seeded from the shared hazard
        // surface — never a doctrine swap (AC3), only a gate on the dodge. Its
        // default (unconditional actuate) reproduces the pre-#780 always-on
        // avoidance; a "hold" resolution emits nothing, so `LateralThrustInput`
        // keeps the last fraction it was given. Not a coast — the latch is
        // re-integrated every tick, so a held lateral axis strafes at a fixed
        // rate indefinitely (issue #968 measured a wrecked destroyer doing
        // exactly that at 8.77 u/s for 300 s). The capability gate in
        // `integrate_ship_physics` and the offline clear in
        // `process_helm_inputs` cover the DESTROYED case; whether a policy
        // "hold" should also decay toward neutral is still open.
        if docking_lateral.is_none() {
            let mut facts = seed_helm_actuator_facts(
                ship_plan.map(|sp| &sp.hazard),
                false,
                false,
                0.0,
                sf.red_alert,
            );
            // Issue #874: lateral is the literal dodge axis, so this is the axis
            // a movement doctrine reaches for `fact(hostile_arc_exposure)` on
            // first. `sf` above is this tick's frame entry — see
            // `seed_hostile_arc_facts`.
            seed_hostile_arc_facts(&mut facts, Some(sf));
            // No attached `[helm_console.lateral_ai]` ⇒ no AI action on this axis.
            // Since #885b stage 5d there is no synthesised stand-in: strict
            // AI-declaration mode rejects an AI-capable hull that omits the block at
            // load, so an absent component means the declaration is missing and a
            // missing declaration gets no automation (PRD #774 US7).
            let Some(lateral_policy) = lateral_policy else {
                continue;
            };
            let policy = &lateral_policy.0;
            // The scenario flag chain, anchored at the layer that spawned
            // this ship (issue #891 stage 2).
            let flag_chain = crate::world::server::entity_flag_chain(
                origin_q.get(entity).ok(),
                world_runtime.as_deref(),
                layers.as_deref(),
            );
            if !helm_policy_actuates(
                policy,
                crate::entities::config::HELM_LATERAL_CHANNEL,
                &facts,
                &crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
                &flag_chain,
            ) {
                continue;
            }
        }

        let lateral = if let Some(docking_lateral) = docking_lateral {
            docking_lateral
        } else if !sf.has_objective {
            // No objectives → zero the dodge rather than latch the last one,
            // matching what the monolith did for the axis.
            0.0
        } else {
            // Horizontal collision avoidance now flows from the shared hazard
            // assessment (issue #743): the planner's `assess_hazards` publishes a
            // ship-local repulsion, and this actuator responds through its own
            // authored `lateral_hazard_sensitivity` rather than re-deriving the
            // projected-collision geometry in a separate helper. The dodge and
            // the yaw agree because both read the one hazard surface the planner
            // built from the hull's authored avoidance tuning.
            // `lateral_thrust_ai_honours_toml_authored_avoidance_buffer` /
            // `..._look_ahead` pin the buffer/look-ahead reaching that surface;
            // `lateral_thrust_ai_responds_to_shared_hazard_surface` pins the
            // sensitivity weighting.
            let sensitivity = behaviour_section
                .map(|b| b.0.lateral_hazard_sensitivity)
                .unwrap_or(crate::ai::LATERAL_HAZARD_SENSITIVITY);
            ship_plan
                .map(|sp| (sp.hazard.hazard_forces.x * sensitivity).clamp(-1.0, 1.0))
                .unwrap_or(0.0)
        };

        emit_helm_lateral_command(
            entity_uuid,
            crate::system_registry::lateral_thrust_system_id(),
            crate::messages::SystemControlPayload::LateralThrustInput { lateral },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}
