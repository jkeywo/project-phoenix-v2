//! The **Impulse** helm axis (issue #1208): the `impulse` channel, gating on
//! `EngageImpulse` then running the `use_impulse`/`decide_impulse` geometry
//! over the radar-gated `visible_view`. Stateless.
//!
//! [`ai_helm_impulse`] is the Bevy system; [`ImpulseAxis`] is its
//! [`super::HelmAxisHost`] impl.
//!
//! Invariant: emits only on an `Engage`/`Cancel` transition, never on
//! `NoChange` — `apply_helm_commands` transitions on `ImpulseCommand` change
//! detection, so an unconditional emit would re-issue `start_charge`/
//! `cancel_charge` every tick.

use super::*;

use crate::ai::host::HostOutcome;
use crate::ai::policy::AiPolicyVerb;
use crate::core::messages::SystemControlPayload;

/// The **Impulse** helm axis (issue #1208): gate the `impulse` channel on the
/// authored `engage_impulse` mode verb, then — on a permit — run the doctrine
/// `use_impulse` + `decide_impulse` geometry and emit engage/cancel on change.
///
/// Reads the radar-gated `visible_view` (deliberately NOT the merged view), so
/// the drive charges toward a point the Helm can actually see. Emits only on an
/// `Engage`/`Cancel` decision, never on `NoChange`: `apply_helm_commands`
/// transitions on `ImpulseCommand` change detection, so an unconditional
/// emission would re-issue `start_charge`/`cancel_charge` every tick. A
/// stateless axis (the frozen `resolve_channel` path).
pub(crate) struct ImpulseAxis;

impl HelmAxisHost for ImpulseAxis {
    fn system_id() -> crate::core::messages::SystemId {
        crate::ship::system_registry::helm_impulse_system_id()
    }
    const CHANNEL: &'static str = crate::entities::config::HELM_IMPULSE_CHANNEL;
    const STATEFUL: bool = false;

    fn accepts(verb: &AiPolicyVerb) -> bool {
        matches!(verb, AiPolicyVerb::EngageImpulse)
    }

    fn seed(cx: &HelmAxisCtx) -> crate::world::flags::AiFacts {
        // impulse_available is literally `true` here — the impulse capability
        // (an `ImpulseConfigResource`) is proven present by the body's
        // availability guard before this seed runs.
        let mut facts = seed_helm_actuator_facts(
            cx.plan.map(|sp| &sp.hazard),
            true,
            cx.boost_cfg.map(|c| c.enabled).unwrap_or(false),
            cx.physics.map(|p| p.y).unwrap_or(0.0),
            frame_red_alert(cx.frame),
        );
        // Issue #874: the arc facts reach this axis too, seeded from the same
        // frame entry the decision below reads.
        seed_hostile_arc_facts(&mut facts, cx.frame);
        facts
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

        // No Helm objective → emit nothing (a lull in objectives should not
        // cancel an in-progress charge).
        let sf = cx.frame?;
        if !sf.has_objective {
            return None;
        }

        // Resolve where the Helm is going, over the radar-gated visible view.
        let target_pos = resolve_helm_target_position(
            &sf.scored,
            &sf.visible_view,
            cx.anchors,
            cx.cursors,
            sf.weapons_target,
        )?;

        // Whether the AI may engage impulse while pursuing this objective is
        // TOML-authored per doctrine entry; an objective with no matching
        // doctrine entry never engages.
        let top_obj = sf.scored.iter().find(|o| {
            o.score > 0.0
                && o.relevance
                    .contains(&crate::core::messages::SystemAffinity::Helm)
        });
        let use_impulse = top_obj
            .and_then(|obj| {
                cx.behaviour
                    .and_then(|b| b.0.doctrine.iter().find(|d| d.id == obj.id))
            })
            .map(|d| d.effective_use_impulse())
            .unwrap_or(false);
        if !use_impulse {
            return None;
        }

        let impulse = cx.impulse?;
        let cfg = cx.impulse_cfg?;
        let physics = cx.physics?;
        let decision = crate::ai::decide_impulse(&crate::ai::ImpulseDecisionInput {
            pos: [physics.x, physics.z],
            yaw: physics.yaw,
            target_pos,
            phase: impulse.0.phase,
            engage_distance: cfg.engage_distance,
            cancel_distance: cfg.cancel_distance,
            angle_tolerance: crate::ai::IMPULSE_ANGLE_TOLERANCE_RAD,
        });
        match decision {
            crate::ai::ImpulseDecision::Engage => Some(SystemControlPayload::StartImpulseCharge),
            crate::ai::ImpulseDecision::Cancel => Some(SystemControlPayload::CancelImpulse),
            crate::ai::ImpulseDecision::NoChange => None,
        }
    }
}

/// Per-axis helm AI: impulse drive. Decides engage/cancel for ships whose
/// helm-impulse system is AI-operated and emits it as an admitted
/// `StartImpulseCharge`/`CancelImpulse` into the ship's own
/// `AdmittedCommands` (issues #703, #704, #824); `process_helm_inputs`
/// applies it to `ImpulseCommand` later this tick, before
/// `apply_helm_commands` consumes the transition.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<ImpulseAxis>`](run_helm_axis) driver's; this body checks
/// the impulse capability, assembles the per-ship context, and emits the payload
/// it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_impulse(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    mut ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&ShipImpulse>,
            Option<&ImpulseConfigResource>,
            Option<&BoostConfigResource>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&FineSystemAiPolicies>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            Option<&crate::ai::server::ObjectiveCursors>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    for (
        entity,
        sources,
        physics,
        impulse_comp,
        impulse_cfg,
        boost_cfg,
        behaviour_section,
        fine_policies,
        entity_uuid,
        ship_config,
        cursors,
        mut admitted,
    ) in ships.iter_mut()
    {
        // No drive or no per-hull drive config → nothing to command. Availability
        // (AC6): the presence of `ImpulseConfigResource` is the impulse
        // capability — no config, no emit.
        let (Some(impulse), Some(cfg)) = (impulse_comp, impulse_cfg) else {
            continue;
        };
        let cx = HelmAxisCtx {
            physics: Some(physics),
            max_speed: 0.0,
            plan: plan.ships.get(&entity),
            frame: frame.ships.get(&entity),
            anchors: &frame.anchors,
            impulse: Some(impulse),
            impulse_cfg: Some(cfg),
            boost_cfg,
            boost: None,
            behaviour: behaviour_section,
            capability: None,
            cursors,
        };
        let mut io = HelmAxisIo {
            policy: None,
            state: None,
            pending: None,
        };
        // The scenario flag chain, anchored at the layer that spawned this ship
        // (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(entity);
        if let Some(payload) = run_helm_axis::<ImpulseAxis>(
            sources,
            fine_policies.and_then(|p| p.0.get(&ImpulseAxis::system_id())),
            None,
            0.0,
            &flag_chain,
            &cx,
            &mut io,
        ) {
            ai_env.emitter().emit(
                entity_uuid,
                ImpulseAxis::system_id(),
                payload,
                sources,
                ship_config,
                &mut admitted,
            );
        }
    }
}

// ── Per-axis helm AI: lateral thrust (issues #697, #703, #824) ────────────────
//
// Born in #697 as `operate_lateral_thrust_ai`, a partial-automation system
// gated `L && !C`; #703 collapsed the gate to `L` alone and closed three
// behaviour divergences against the monolith (radar gating, snapshot
// fallback, no-objective zeroing); #704 deleted the monolith, leaving `L`
// the whole story. #824 moved the transport: the dodge is now emitted as an
// admitted `LateralThrustInput` command rather than a direct
// `LateralThrustInput` component write — see the per-axis module note above.
//
// The ~30 Hz cadence predates the split (it was the private
// `AiLateralThrustTimer` until #803) and is load-bearing: production `Update`
// is rAF-driven, so without the shared `run_if(ai_tick_ready)` gate the
// dodge cadence would follow the host's display refresh rate — precisely the
// nondeterminism PRD #620 (P2P deterministic lockstep) exists to remove.
// A skipped frame runs none of the four axis systems, so an axis simply
// holds its last applied intent through the gap and `integrate_ship_physics`
// keeps integrating it.
// `*_runs_on_the_shared_sim_tick_not_per_frame` pins the cadence for each of
// the four systems.
