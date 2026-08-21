use super::*;

/// Per-ship inline stateless **Engines** AI policy (issue #779).
///
/// Attached at spawn from the ship's authored `[helm_console.engines_ai]`
/// block. Read by [`ai_helm_thrust`], which resolves its `longitudinal` channel
/// over a per-tick fact snapshot to decide *whether* to actuate the planner's
/// desired travel — the DECISION now flows through a data-authored policy verb
/// instead of an unconditional hardcoded branch. The continuous thrust magnitude
/// still comes from the shared `DesiredMotion` planner fact (issue #741).
///
/// Since #885b stage 5d there is no Rust-side synthesised default behind it: a
/// ship without the component takes no AI action on this axis.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmEnginesAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship runtime state for a STATEFUL Engines policy (issue #883).
///
/// The Engines twin of [`HelmBoostAiPolicyState`], and separate from it for the
/// same structural reason: private memory belongs to ONE fine system. The
/// destroyer's Engines, Steering and Boost each run their own copy of the
/// fly-through machine over the same host-seeded facts, so they reach the same
/// leg on the same tick *independently* — there is no ship-wide pass state that
/// one of them owns and the others read.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmEnginesAiPolicyState(pub crate::ai::policy::AiPolicyRuntimeState);

/// Per-axis helm AI: throttle. Decides the throttle for ships whose
/// helm-thrust system is AI-operated and emits it as an admitted `SetThrust`
/// into the ship's own `AdmittedCommands` (issues #800, #704, #824) —
/// `process_helm_inputs` applies it to `ThrustInput` later this tick.
///
/// `AiHighFidelity`-scoped: the frame is only built for ships carrying that
/// marker, and the intent components the admitted command lands on only
/// exist there (`lod_ai_ships` inserts/removes them with the marker).
///
/// Decodes only its own axis from the shared motion plan (built this tick by
/// `helm_motion_planner` from the pure `plan_helm_travel` decision, see the
/// module note).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_thrust(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    sessions: Res<crate::lobby::Sessions>,
    clock: Res<AiPolicyTickClock>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            // Availability of the two optional drives, seeded honestly into the
            // fact snapshot below (see the `seed_helm_actuator_facts` call).
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&HelmEnginesAiPolicy>,
            Option<&HelmEnginesAiPolicyState>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (
        entity,
        sources,
        physics,
        physics_cfg,
        entity_uuid,
        ship_config,
        boost_cfg,
        impulse_cfg,
        engines_policy,
        engines_state,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own axis alone (issue #800) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_thrust_system_id())
            .operate_ai
        {
            continue;
        }
        // Consume the shared desired-motion contract published by the motion
        // planner this tick (issue #741): decode our own axis (forward
        // throttle) from the ship's 3D desired velocity rather than
        // re-deriving the decision here. No plan entry (no AI helm axis / no
        // frame) means nothing to actuate.
        let Some(sp) = plan.ships.get(&entity) else {
            continue;
        };
        // Resolve the data-authored #779 Engines policy's `longitudinal` mode
        // verb to decide WHETHER to actuate this tick. The stateless policy is a
        // pure fact→mode map; the continuous magnitude below still comes from
        // the planner fact, so no geometry lives in the policy (AGENTS.md #11).
        // A "hold" resolution (no rule fires / explicit idle) emits nothing, and
        // `ThrustInput` therefore keeps the last value it was given. Note what
        // that is NOT: the throttle does not coast, decay or centre. The intent
        // component is a latch and `integrate_ship_physics` goes on integrating
        // it every tick, so "hold" means the ship carries on doing exactly what
        // it was last told to do, indefinitely. Issue #968 is what that costs
        // when the axis can no longer be commanded at all: the fix there is a
        // capability gate in the integrator plus an offline clear in
        // `process_helm_inputs`, and whether a POLICY "hold" should also decay
        // toward neutral is still open.
        //
        // Issue #883 (AC5) closes the #779 empty-facts gap on this axis: the
        // snapshot below is really seeded — hazard/availability from the shared
        // surfaces, target-relative motion from the frame's merged view — so a
        // `fact(...)` guard on `longitudinal` evaluates against the world
        // instead of validating and never firing.
        //
        // The availability pair is seeded from the ship's OWN config resources,
        // exactly as `ai_policy_state_tick` and `ai_helm_boost` seed it. Passing
        // a hardcoded `false` here would have been the #779 trap one fact
        // narrower: a guard on `fact(boost_available)` would validate at load
        // and then read 0 for ever, which is silently wrong in the same way an
        // absent fact is.
        // No attached `[helm_console.engines_ai]` ⇒ no AI action on this axis.
        // Since #885b stage 5d there is no synthesised stand-in: strict
        // AI-declaration mode rejects an AI-capable hull that omits the block at
        // load, so an absent component means the declaration is missing and a
        // missing declaration gets no automation (PRD #774 US7).
        let Some(engines_policy) = engines_policy else {
            continue;
        };
        let policy = &engines_policy.0;
        let mut facts = seed_helm_actuator_facts(
            Some(&sp.hazard),
            impulse_cfg.is_some(),
            boost_cfg.map(|c| c.enabled).unwrap_or(false),
            physics.y,
            frame_red_alert(frame.ships.get(&entity)),
        );
        seed_helm_travel_facts(
            &mut facts,
            frame.ships.get(&entity),
            physics,
            physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
        );
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(entity);
        if resolve_helm_channel(
            policy,
            engines_state.map(|s| &s.0),
            crate::entities::config::HELM_LONGITUDINAL_CHANNEL,
            &facts,
            clock.0,
            &flag_chain,
        ) != Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel)
        {
            continue;
        }
        let thrust =
            crate::ai::decode_thrust_from_velocity(sp.motion.desired_velocity_local.to_array());

        emit_helm_ai_command(
            entity_uuid,
            crate::system_registry::helm_thrust_system_id(),
            crate::messages::SystemControlPayload::SetThrust { value: thrust },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}
