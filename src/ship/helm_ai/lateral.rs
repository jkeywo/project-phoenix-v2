use super::*;

use crate::ai::host::HostOutcome;
use crate::ai::policy::AiPolicyVerb;
use crate::messages::SystemControlPayload;

/// The **Lateral Thrust** helm axis (issue #1208): outside a docking manoeuvre,
/// gate the `lateral` channel on the authored `actuate_lateral_thrust` mode verb
/// and, on a fire, emit the shared-hazard dodge weighted by this hull's authored
/// `lateral_hazard_sensitivity`. A stateless axis.
///
/// The docking close manoeuvre (issue #742) is the sanctioned
/// [`pre_override`](HelmAxisHost::pre_override): its controlled translation owns
/// the lateral axis and PRECEDES the policy gate (but not the Control-Source
/// gate), so a docking hull always translates onto its berth. Since #743 the
/// dodge is no longer re-derived here — it reads the planner's `assess_hazards`
/// surface — so the dodge and the yaw agree because both read the one hazard
/// surface the planner built from the hull's authored avoidance tuning.
pub(crate) struct LateralAxis;

impl HelmAxisHost for LateralAxis {
    fn system_id() -> crate::messages::SystemId {
        crate::system_registry::lateral_thrust_system_id()
    }
    const CHANNEL: &'static str = crate::entities::config::HELM_LATERAL_CHANNEL;
    const STATEFUL: bool = false;

    fn accepts(verb: &AiPolicyVerb) -> bool {
        matches!(verb, AiPolicyVerb::ActuateLateralThrust)
    }

    fn seed(cx: &HelmAxisCtx) -> crate::world::flags::AiFacts {
        let mut facts = seed_helm_actuator_facts(
            cx.plan.map(|sp| &sp.hazard),
            false,
            false,
            0.0,
            frame_red_alert(cx.frame),
        );
        // Issue #874: lateral is the literal dodge axis, so this is the axis a
        // movement doctrine reaches for `fact(hostile_arc_exposure)` on first.
        seed_hostile_arc_facts(&mut facts, cx.frame);
        facts
    }

    fn pre_override(cx: &HelmAxisCtx) -> Option<SystemControlPayload> {
        // A docking close manoeuvre (issue #742), when the planner engaged one
        // this tick, owns the lateral axis: its controlled translation is read
        // straight off the shared desired-motion contract's `x`. An UNCONDITIONAL
        // sanctioned override — it precedes the policy gate.
        cx.plan
            .filter(|sp| sp.docking_active)
            .map(|sp| SystemControlPayload::LateralThrustInput {
                lateral: sp.motion.desired_velocity_local.x,
            })
    }

    fn act(
        outcome: HostOutcome,
        cx: &HelmAxisCtx,
        _io: &mut HelmAxisIo,
    ) -> Option<SystemControlPayload> {
        match outcome {
            HostOutcome::Act(verb) if Self::accepts(verb) => {}
            _ => return None,
        }
        let sf = cx.frame?;
        let lateral = if !sf.has_objective {
            // No objectives → zero the dodge rather than latch the last one,
            // matching what the monolith did for the axis.
            0.0
        } else {
            // Horizontal collision avoidance flows from the shared hazard
            // assessment (issue #743): the planner's `assess_hazards` publishes a
            // ship-local repulsion, and this actuator responds through its own
            // authored `lateral_hazard_sensitivity`.
            let sensitivity = cx
                .behaviour
                .map(|b| b.0.lateral_hazard_sensitivity)
                .unwrap_or(crate::ai::LATERAL_HAZARD_SENSITIVITY);
            cx.plan
                .map(|sp| (sp.hazard.hazard_forces.x * sensitivity).clamp(-1.0, 1.0))
                .unwrap_or(0.0)
        };
        Some(SystemControlPayload::LateralThrustInput { lateral })
    }
}

/// Per-axis helm AI: lateral thrust. Decides the dodge for ships whose
/// helm-lateral-thrust system is AI-operated and emits it as an admitted
/// `LateralThrustInput` into the ship's own `AdmittedCommands` (issues #703,
/// #704, #824). Docking translation still overrides it (issue #742), and the
/// emit → admit → apply arbiter path is unchanged.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<LateralAxis>`](run_helm_axis) driver's, with the docking
/// override expressed as [`LateralAxis::pre_override`](HelmAxisHost::pre_override);
/// this body assembles the per-ship context and emits the payload it returns.
pub(crate) fn ai_helm_lateral_thrust(
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
            // Optional, so it does not filter the iteration set: a ship without
            // a `[behaviour]` section still runs AI lateral thrust, on the
            // `crate::ai::*` fallbacks that match the serde defaults.
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&FineSystemAiPolicies>,
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
        fine_policies,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Lateral always needs its frame entry — the dodge zeroing and the
        // has-objective gate both read it — so no frame entry stands down (and
        // the docking override too, matching the pre-#1208 order).
        let Some(_sf) = frame.ships.get(&entity) else {
            continue;
        };
        let cx = HelmAxisCtx {
            // The Lateral query carries no `ShipPhysics` — its dodge reads the
            // plan's hazard surface, not the pose (see `HelmAxisCtx::physics`).
            physics: None,
            max_speed: 0.0,
            plan: plan.ships.get(&entity),
            frame: frame.ships.get(&entity),
            anchors: &frame.anchors,
            impulse: None,
            impulse_cfg: None,
            boost_cfg: None,
            boost: None,
            behaviour: behaviour_section,
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
        if let Some(payload) = run_helm_axis::<LateralAxis>(
            sources,
            fine_policies.and_then(|p| p.0.get(&LateralAxis::system_id())),
            None,
            0.0,
            &flag_chain,
            &cx,
            &mut io,
        ) {
            emit_helm_lateral_command(
                entity_uuid,
                LateralAxis::system_id(),
                payload,
                sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
        }
    }
}
