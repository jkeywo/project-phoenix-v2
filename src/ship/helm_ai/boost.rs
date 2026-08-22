use super::*;

use crate::ai::host::HostOutcome;
use crate::ai::policy::AiPolicyVerb;
use crate::messages::SystemControlPayload;

/// Per-ship runtime state for a STATEFUL Boost policy (issue #882) — the
/// minimal host that proves the optional stateful path end to end.
///
/// Boost was chosen as the demonstrator because it is the smallest credible
/// stateful axis in the game: its shipped default policy is *idle*, so nothing
/// that ships today changes behaviour, and its host already resolves exactly
/// one channel from an already-seeded fact snapshot. (The destroyer doctrine
/// this spine exists for is issue #883, deliberately not built here.)
///
/// ## Why this is a separate component
///
/// The authored Boost policy (this axis's entry in
/// [`FineSystemAiPolicies`]) is immutable authored data; taking it `&mut` to tick
/// a state machine would dirty Bevy change-detection on the policy every tick.
/// So the runtime state is its own sibling component.
///
/// ## Why it is per-fine-system, not per-ship
///
/// This component belongs to the Boost fine system ALONE, and there is
/// deliberately no `ShipAiState`. That is the structural answer to AC3: the
/// `memory(...)` / `state_time` bag handed to an evaluation is seeded from
/// THIS component, so no sibling fine system's policy can observe it and no
/// ship-wide state machine can form by accretion.
///
/// Inserted/removed alongside `AiHighFidelity` by `lod_ai_ships`, so a demoted
/// ship drops its policy state and a re-promoted one starts from `initial`
/// (AC5).
#[derive(Component, Clone, Debug, Default)]
pub struct HelmBoostAiPolicyState(pub crate::ai::policy::AiPolicyRuntimeState);

/// The **Boost** helm axis (issue #1208, issue #780): resolve the `boost`
/// channel to the `engage_boost` mode verb over the shared hazard/travel facts
/// and emit `SetBoost { active }` on change. A stateful axis — the #882 machine
/// demonstrator.
///
/// Unlike the other axes, Boost actuates on `Held` too: a hold (or any non-boost
/// verb) means *release*, so the axis emits `SetBoost { false }` when the drive
/// is currently active. It emits only when the desired state differs from the
/// current [`ShipBoost`], so it does not re-issue `SetBoost` every tick. The
/// canonical default policy is idle, so a ship that authors no
/// `[helm_console.boost_ai]` never AI-boosts.
pub(crate) struct BoostAxis;

impl HelmAxisHost for BoostAxis {
    fn system_id() -> crate::messages::SystemId {
        crate::system_registry::helm_boost_system_id()
    }
    const CHANNEL: &'static str = crate::entities::config::HELM_BOOST_CHANNEL;
    const STATEFUL: bool = true;

    fn accepts(verb: &AiPolicyVerb) -> bool {
        matches!(verb, AiPolicyVerb::EngageBoost)
    }

    fn seed(cx: &HelmAxisCtx) -> crate::world::flags::AiFacts {
        // boost_available is literally `true` here — this IS the boost axis, and
        // the body's availability guard has proven an enabled `BoostConfigResource`.
        let mut facts = seed_helm_actuator_facts(
            cx.plan.map(|sp| &sp.hazard),
            cx.impulse_cfg.is_some(),
            true,
            cx.physics.map(|p| p.y).unwrap_or(0.0),
            frame_red_alert(cx.frame),
        );
        // Issue #883 seeds the target-relative travel facts too, so an authored
        // escape-leg boost rule can guard on the pass geometry, not just hazard.
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
        // Fires (`engage_boost`) ⇒ boost on; holds or any other verb ⇒ boost
        // off. Undeclared/not-AI never touches the drive.
        let desired = match outcome {
            HostOutcome::Act(AiPolicyVerb::EngageBoost) => true,
            HostOutcome::Act(_) | HostOutcome::Held => false,
            HostOutcome::Undeclared | HostOutcome::NotAiOperated => return None,
        };
        // On-change only: re-issuing an unchanged state every tick is redundant.
        let current = cx.boost.map(|b| b.0.is_active()).unwrap_or(false);
        if desired == current {
            None
        } else {
            Some(SystemControlPayload::SetBoost { active: desired })
        }
    }
}

/// Per-axis helm AI: boost drive (issue #780). Decides engage/release for ships
/// whose `helm-boost` system is AI-operated and emits it as an admitted
/// `SetBoost { active }` into the ship's own `AdmittedCommands` — the SAME seam
/// a human `SetBoost`/`ToggleBoost` passes through (`process_helm_inputs`),
/// preserving human/AI symmetry (AGENTS.md #6).
///
/// Availability (AC6) is the presence of an *enabled* [`BoostConfigResource`].
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<BoostAxis>`](run_helm_axis) driver's; this body checks the
/// boost capability, assembles the per-ship context, and emits the payload it
/// returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_boost(
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
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&ShipBoost>,
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&FineSystemAiPolicies>,
            Option<&HelmBoostAiPolicyState>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
    clock: Res<AiPolicyTickClock>,
) {
    for (
        entity,
        sources,
        physics,
        physics_cfg,
        boost_comp,
        boost_cfg,
        impulse_cfg,
        fine_policies,
        boost_state,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Availability (AC6): the feature must be present AND enabled. No
        // BoostConfigResource, or one with the feature disabled, means no boost
        // capability — emit nothing.
        let (Some(boost), Some(cfg)) = (boost_comp, boost_cfg) else {
            continue;
        };
        if !cfg.enabled {
            continue;
        }
        let cx = HelmAxisCtx {
            physics: Some(physics),
            max_speed: physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
            plan: plan.ships.get(&entity),
            frame: frame.ships.get(&entity),
            anchors: &frame.anchors,
            impulse: None,
            impulse_cfg,
            boost_cfg: Some(cfg),
            boost: Some(boost),
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
        if let Some(payload) = run_helm_axis::<BoostAxis>(
            sources,
            fine_policies.and_then(|p| p.0.get(&BoostAxis::system_id())),
            boost_state.map(|s| &s.0),
            clock.0,
            &flag_chain,
            &cx,
            &mut io,
        ) {
            emit_ai_command(
                entity_uuid,
                BoostAxis::system_id(),
                payload,
                sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
        }
    }
}
