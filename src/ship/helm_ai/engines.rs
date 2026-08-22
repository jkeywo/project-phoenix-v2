use super::*;

use crate::ai::host::HostOutcome;
use crate::ai::policy::AiPolicyVerb;
use crate::messages::SystemControlPayload;

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

/// The **Engines** helm axis (issue #1208): resolve `[helm_console.engines_ai]`'s
/// `longitudinal` channel to the `actuate_desired_travel` mode verb, and on a
/// fire emit the planner-decoded forward throttle. A stateful axis — the
/// destroyer's fly-through machine runs here (issue #883).
///
/// The DECISION is a pure fact→mode map; the continuous magnitude comes from the
/// shared [`DesiredMotion`](crate::ship::helm_planner) planner fact, so no
/// geometry lives in the policy (AGENTS.md #11). A "hold" (no rule fires) emits
/// nothing and `ThrustInput` latches its last value — it does not coast, decay
/// or centre; `integrate_ship_physics` keeps integrating the latch every tick
/// (issue #968's capability gate is what covers the axis being destroyed).
pub(crate) struct EnginesAxis;

impl HelmAxisHost for EnginesAxis {
    fn system_id() -> crate::messages::SystemId {
        crate::system_registry::helm_thrust_system_id()
    }
    const CHANNEL: &'static str = crate::entities::config::HELM_LONGITUDINAL_CHANNEL;
    const STATEFUL: bool = true;

    fn accepts(verb: &AiPolicyVerb) -> bool {
        matches!(verb, AiPolicyVerb::ActuateDesiredTravel)
    }

    fn seed(cx: &HelmAxisCtx) -> crate::world::flags::AiFacts {
        // Issue #883 (AC5): a really-seeded snapshot — hazard/availability from
        // the shared surfaces, target-relative motion from the frame — so a
        // `fact(...)` guard on `longitudinal` evaluates against the world. The
        // availability pair is seeded honestly from the ship's own config
        // resources, never a hardcoded `false` (the #779 trap one fact narrower).
        let mut facts = seed_helm_actuator_facts(
            cx.plan.map(|sp| &sp.hazard),
            cx.impulse_cfg.is_some(),
            cx.boost_cfg.map(|c| c.enabled).unwrap_or(false),
            cx.physics.map(|p| p.y).unwrap_or(0.0),
            frame_red_alert(cx.frame),
        );
        if let Some(physics) = cx.physics {
            seed_helm_travel_facts(&mut facts, cx.frame, physics, cx.max_speed);
        }
        facts
    }

    fn act(
        outcome: HostOutcome,
        cx: &HelmAxisCtx,
        _io: &mut HelmAxisIo,
    ) -> Option<SystemControlPayload> {
        match outcome {
            HostOutcome::Act(verb) if Self::accepts(verb) => {
                // Decode our own axis (forward throttle) from the ship's 3D
                // desired velocity rather than re-deriving the decision here.
                let sp = cx.plan?;
                let thrust = crate::ai::decode_thrust_from_velocity(
                    sp.motion.desired_velocity_local.to_array(),
                );
                Some(SystemControlPayload::SetThrust { value: thrust })
            }
            _ => None,
        }
    }
}

/// Per-axis helm AI: throttle. Decides the throttle for ships whose
/// helm-thrust system is AI-operated and emits it as an admitted `SetThrust`
/// into the ship's own `AdmittedCommands` (issues #800, #704, #824) —
/// `process_helm_inputs` applies it to `ThrustInput` later this tick.
///
/// `AiHighFidelity`-scoped: the frame is only built for ships carrying that
/// marker, and the intent components the admitted command lands on only
/// exist there (`lod_ai_ships` inserts/removes them with the marker).
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<EnginesAxis>`](run_helm_axis) driver's; this body only
/// assembles the per-ship context and emits the payload it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_thrust(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
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
            // fact snapshot (see `EnginesAxis::seed`).
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&FineSystemAiPolicies>,
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
        fine_policies,
        engines_state,
        mut admitted,
    ) in ships.iter_mut()
    {
        // No plan entry (no AI helm axis / no frame) means nothing to actuate:
        // the throttle decode below reads the plan's decoded velocity.
        let Some(sp) = plan.ships.get(&entity) else {
            continue;
        };
        let cx = HelmAxisCtx {
            physics: Some(physics),
            max_speed: physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
            plan: Some(sp),
            frame: frame.ships.get(&entity),
            anchors: &frame.anchors,
            impulse: None,
            impulse_cfg,
            boost_cfg,
            boost: None,
            behaviour: None,
            capability: None,
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
        if let Some(payload) = run_helm_axis::<EnginesAxis>(
            sources,
            fine_policies.and_then(|p| p.0.get(&EnginesAxis::system_id())),
            engines_state.map(|s| &s.0),
            clock.0,
            &flag_chain,
            &cx,
            &mut io,
        ) {
            ai_env.emitter().emit(
                entity_uuid,
                EnginesAxis::system_id(),
                payload,
                sources,
                ship_config,
                &mut admitted,
            );
        }
    }
}
