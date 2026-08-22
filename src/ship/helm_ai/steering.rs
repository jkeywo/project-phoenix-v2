//! The **Steering** helm axis (issue #1208): the `yaw` channel, resolving
//! `[helm_console.steering_ai]` against seven mode verbs
//! (`ActuateDesiredFacing` plus six `Hold*`/`PivotToReengage` doctrine legs)
//! and emitting the planner-decoded yaw. Stateful — the fly-through machine
//! decides which leg [`super::HelmPassSurface`] publishes.
//!
//! [`ai_helm_steering`] is the Bevy system; [`SteeringAxis`] is its
//! [`super::HelmAxisHost`] impl. Unique among the six axes: it also owns the
//! Weapons→Helm arc-bearing override (issue #677), applied on top of the
//! planner's decoded facing when the current leg consents to yield.

use super::*;

use crate::ai::host::HostOutcome;
use crate::ai::policy::AiPolicyVerb;
use crate::core::messages::SystemControlPayload;

/// Per-ship runtime state for a STATEFUL Steering policy (issue #883). The
/// Steering twin of [`HelmEnginesAiPolicyState`]; this is the one whose current
/// state decides which leg [`HelmPassSurface`] publishes, because the yaw
/// channel is the axis that carries the two different facing verbs.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmSteeringAiPolicyState(pub crate::ai::policy::AiPolicyRuntimeState);

/// The **Steering** helm axis (issue #1208): resolve `[helm_console.steering_ai]`'s
/// `yaw` channel and, on a fire, emit the planner-decoded yaw — with the
/// Weapons→Helm arc-bearing request (issue #677) applied on top as a facing
/// override this axis owns. A stateful axis (issue #883).
///
/// Seven mode verbs actuate here identically — [`ActuateDesiredFacing`], the
/// frozen-heading [`HoldCommittedHeading`], the recovery [`HoldRecoveryOrbit`] /
/// [`PivotToReengage`], the combat [`HoldCombatOrbit`], the torpedo
/// [`HoldTorpedoBearing`] and the artillery [`HoldArtilleryPosition`] — because
/// the difference between them was already resolved upstream by the planner
/// solving the facing against the right reference; this axis's only job is to
/// emit the decoded yaw so #780's hazard contribution keeps composing onto it.
///
/// [`ActuateDesiredFacing`]: AiPolicyVerb::ActuateDesiredFacing
/// [`HoldCommittedHeading`]: AiPolicyVerb::HoldCommittedHeading
/// [`HoldRecoveryOrbit`]: AiPolicyVerb::HoldRecoveryOrbit
/// [`PivotToReengage`]: AiPolicyVerb::PivotToReengage
/// [`HoldCombatOrbit`]: AiPolicyVerb::HoldCombatOrbit
/// [`HoldTorpedoBearing`]: AiPolicyVerb::HoldTorpedoBearing
/// [`HoldArtilleryPosition`]: AiPolicyVerb::HoldArtilleryPosition
pub(crate) struct SteeringAxis;

impl HelmAxisHost for SteeringAxis {
    fn system_id() -> crate::core::messages::SystemId {
        crate::ship::system_registry::helm_steering_system_id()
    }
    const CHANNEL: &'static str = crate::entities::config::HELM_YAW_CHANNEL;
    const STATEFUL: bool = true;

    fn accepts(verb: &AiPolicyVerb) -> bool {
        matches!(
            verb,
            AiPolicyVerb::ActuateDesiredFacing
                | AiPolicyVerb::HoldCommittedHeading
                | AiPolicyVerb::HoldRecoveryOrbit
                | AiPolicyVerb::PivotToReengage
                | AiPolicyVerb::HoldCombatOrbit
                | AiPolicyVerb::HoldTorpedoBearing
                | AiPolicyVerb::HoldArtilleryPosition
        )
    }

    fn seed(cx: &HelmAxisCtx) -> crate::world::flags::AiFacts {
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
        io: &mut HelmAxisIo,
    ) -> Option<SystemControlPayload> {
        let HostOutcome::Act(verb) = outcome else {
            return None;
        };
        if !Self::accepts(verb) {
            return None;
        }
        let sp = cx.plan?;
        let sf = cx.frame?;
        let physics = cx.physics?;
        let mut steering =
            crate::ai::decode_steering_from_facing(sp.motion.desired_facing_local.to_array());

        // ── Weapons->Helm arc-bearing request (issue #677) ───────────────
        // Gated on a live objective, and on the consent of the leg this helm is
        // flying (issue #918): the request replaces `steering` with a bow-on
        // tracking solution, right for a helm merely travelling, wrong for one
        // whose doctrine committed to a heading. The question is what THIS helm
        // is doing — never who raised the request (admission stripped that;
        // AGENTS.md #6). A helm with no doctrine leg to defend answers `true` and
        // behaves exactly as it did before.
        let leg_yields = io
            .policy
            .map(|p| p.leg_yields_to_arc_requests(io.state.map(|s| s.current.as_str())))
            .unwrap_or(true);
        if sf.has_objective {
            apply_arc_bearing_request(
                &mut steering,
                io.pending.as_deref_mut(),
                &sf.merged_view,
                physics,
                leg_yields,
            );
        }

        Some(SystemControlPayload::SetSteering { value: steering })
    }
}

/// Per-axis helm AI: steering. Decides the yaw for ships whose helm-steering
/// system is AI-operated and emits it as an admitted `SetSteering` into the
/// ship's own `AdmittedCommands` (issues #800, #704, #824); it owns the
/// arc-bearing step outright.
///
/// Steers toward the selected waypoint/target chosen by the pure
/// `crate::ai::operate_helm`, including the **Retreat consumer** (issue #688).
/// `ai_helm_steering_retreats_toward_anchor` pins that behaviour through this
/// system, and `ai_helm_steering_retreat_with_unknown_anchor_falls_through` the
/// other side of it.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<SteeringAxis>`](run_helm_axis) driver's; this body
/// assembles the per-ship context (including the mutable arc-bearing request)
/// and emits the payload it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_steering(
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
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            // Availability of the two optional drives — see `ai_helm_thrust`.
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&FineSystemAiPolicies>,
            Option<&HelmSteeringAiPolicyState>,
            Option<&mut PendingArcBearingRequest>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
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
        steering_state,
        mut pending_bearing,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Base steering comes from the plan's decoded facing; arc-bearing is a
        // facing override this axis owns, resolved against the frame's merged
        // view. Both are required to actuate, so no plan/frame entry stands down.
        let (Some(sp), Some(sf)) = (plan.ships.get(&entity), frame.ships.get(&entity)) else {
            continue;
        };
        let cx = HelmAxisCtx {
            physics: Some(physics),
            max_speed: physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
            plan: Some(sp),
            frame: Some(sf),
            anchors: &frame.anchors,
            impulse: None,
            impulse_cfg,
            boost_cfg,
            boost: None,
            behaviour: None,
            capability: None,
            cursors: None,
        };
        // Resolve this axis's authored policy out of the ship's one keyed
        // `FineSystemAiPolicies` map (issue #1209); `Option<&AiPolicy>` is `Copy`,
        // so the same reference feeds both the io scratch and the driver below.
        let steering_policy = fine_policies.and_then(|p| p.0.get(&SteeringAxis::system_id()));
        let mut io = HelmAxisIo {
            policy: steering_policy,
            state: steering_state.map(|s| &s.0),
            pending: pending_bearing.as_deref_mut(),
        };
        // The scenario flag chain, anchored at the layer that spawned this ship
        // (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(entity);
        if let Some(payload) = run_helm_axis::<SteeringAxis>(
            sources,
            steering_policy,
            steering_state.map(|s| &s.0),
            clock.0,
            &flag_chain,
            &cx,
            &mut io,
        ) {
            ai_env.emitter().emit(
                entity_uuid,
                SteeringAxis::system_id(),
                payload,
                sources,
                ship_config,
                &mut admitted,
            );
        }
    }
}
