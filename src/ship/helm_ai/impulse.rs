use super::*;

/// Per-ship inline stateless **Impulse** AI policy (issue #780). From
/// the authored `[helm_console.impulse_ai]` block. Read by
/// [`ai_helm_impulse`] to decide whether the impulse manoeuvre is permitted this
/// tick; the host still applies doctrine `use_impulse` and `decide_impulse`
/// geometry.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmImpulseAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-axis helm AI: impulse drive. Decides engage/cancel for ships whose
/// helm-impulse system is AI-operated and emits it as an admitted
/// `StartImpulseCharge`/`CancelImpulse` into the ship's own
/// `AdmittedCommands` (issues #703, #704, #824); `process_helm_inputs`
/// applies it to `ImpulseCommand` later this tick, before
/// `apply_helm_commands` consumes the transition.
///
/// **Reads the shared helm surfaces; mutates none of them.** It resolves where
/// the Helm is going via `resolve_helm_target_position`, over the frame's
/// radar-gated `visible_view` — deliberately NOT the merged view, preserving
/// the pre-#824 shape where the impulse decision never saw an out-of-radar
/// shared target — so the drive charges toward a point the Helm can actually
/// see.
///
/// Emits only on an `Engage`/`Cancel` decision, never on `NoChange`:
/// `apply_helm_commands` transitions on `ImpulseCommand` change detection, so
/// an unconditional emission would re-issue `start_charge`/`cancel_charge`
/// every tick.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_impulse(
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
            &ShipPhysics,
            Option<&ShipImpulse>,
            Option<&ImpulseConfigResource>,
            Option<&BoostConfigResource>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&HelmImpulseAiPolicy>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            Option<&crate::ai_plugin::ObjectiveCursors>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
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
        impulse_policy,
        entity_uuid,
        ship_config,
        cursors,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own system alone — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_impulse_system_id())
            .operate_ai
        {
            continue;
        }

        // No drive or no per-hull drive config → nothing to command. Matches
        // the monolith, which guards the same pair. Availability (AC6): the
        // presence of `ImpulseConfigResource` is the impulse capability — no
        // config, no emit.
        let (Some(impulse), Some(cfg)) = (impulse_comp, impulse_cfg) else {
            continue;
        };

        // Authored manoeuvre policy gate (issue #780, AC6): seed the hazard +
        // availability facts and resolve the `impulse` channel. Its default
        // (unconditional permit) preserves the pre-#780 baseline exactly — the
        // engage/cancel decision is still made below from doctrine + geometry —
        // while an authored guard may hold impulse. A "hold" resolution emits
        // nothing.
        let boost_available = boost_cfg.map(|c| c.enabled).unwrap_or(false);
        let mut facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            true,
            boost_available,
            physics.y,
            frame_red_alert(frame.ships.get(&entity)),
        );
        // Issue #874: the arc facts reach this axis too — see
        // `seed_hostile_arc_facts`. Seeded from the same frame entry the
        // decision below reads, so the guard and the manoeuvre cannot disagree
        // about the tick.
        seed_hostile_arc_facts(&mut facts, frame.ships.get(&entity));
        // No attached `[helm_console.impulse_ai]` ⇒ no AI action on this axis.
        // Since #885b stage 5d there is no synthesised stand-in: strict
        // AI-declaration mode rejects an AI-capable hull that omits the block at
        // load, so an absent component means the declaration is missing and a
        // missing declaration gets no automation (PRD #774 US7).
        let Some(impulse_policy) = impulse_policy else {
            continue;
        };
        let policy = &impulse_policy.0;
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(entity).ok(),
            world_runtime.as_deref(),
            layers.as_deref(),
        );
        if !helm_policy_actuates(
            policy,
            crate::entities::config::HELM_IMPULSE_CHANNEL,
            &facts,
            &crate::ai::policy::AiPolicyVerb::EngageImpulse,
            &flag_chain,
        ) {
            continue;
        }

        let Some(sf) = frame.ships.get(&entity) else {
            continue;
        };
        // No Helm objective → emit nothing. The monolith's no-objective
        // branch `continue`s before its impulse block for exactly the same
        // reason: an in-progress charge is not something a lull in objectives
        // should cancel. (Behaviourally a redundant early-out — the
        // top-objective filters below reach the same `continue` — kept
        // because it short-circuits the target resolution and keeps the shape
        // legible against the monolith it replaces.)
        if !sf.has_objective {
            continue;
        }

        // Resolve where the Helm is going, from the same surfaces `operate_helm`
        // reads, over the radar-gated visible view (see the doc comment).
        let Some(target_pos) = resolve_helm_target_position(
            &sf.scored,
            &sf.visible_view,
            &frame.anchors,
            cursors,
            sf.weapons_target,
        ) else {
            continue;
        };

        // Whether the AI may engage impulse at all while pursuing this
        // objective is TOML-authored per doctrine entry
        // (`[[behaviour.doctrine]] use_impulse`); an objective with no matching
        // doctrine entry never engages.
        let top_obj = sf.scored.iter().find(|o| {
            o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
        });
        let use_impulse = top_obj
            .and_then(|obj| {
                behaviour_section.and_then(|b| b.0.doctrine.iter().find(|d| d.id == obj.id))
            })
            .map(|d| d.effective_use_impulse())
            .unwrap_or(false);
        if !use_impulse {
            continue;
        }

        let decision = crate::ai::decide_impulse(&crate::ai::ImpulseDecisionInput {
            pos: [physics.x, physics.z],
            yaw: physics.yaw,
            target_pos,
            phase: impulse.0.phase,
            engage_distance: cfg.engage_distance,
            cancel_distance: cfg.cancel_distance,
            angle_tolerance: crate::ai::IMPULSE_ANGLE_TOLERANCE_RAD,
        });
        let payload = match decision {
            crate::ai::ImpulseDecision::Engage => {
                crate::messages::SystemControlPayload::StartImpulseCharge
            }
            crate::ai::ImpulseDecision::Cancel => {
                crate::messages::SystemControlPayload::CancelImpulse
            }
            crate::ai::ImpulseDecision::NoChange => continue,
        };
        emit_helm_ai_command(
            entity_uuid,
            crate::system_registry::helm_impulse_system_id(),
            payload,
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
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
