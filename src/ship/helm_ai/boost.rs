use super::*;

/// Per-ship inline stateless **Boost** AI policy (issue #780). From
/// the authored `[helm_console.boost_ai]` block.
/// Read by [`ai_helm_boost`] to decide whether to engage boost this tick, emitted
/// through the same admitted `SetBoost` seam a human uses.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmBoostAiPolicy(pub crate::ai::policy::AiPolicy);

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
/// [`HelmBoostAiPolicy`] is immutable authored data; taking it `&mut` to tick
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

/// Per-axis helm AI: boost drive (issue #780). Decides engage/release for ships
/// whose `helm-boost` system is AI-operated and emits it as an admitted
/// `SetBoost { active }` into the ship's own `AdmittedCommands` — the SAME seam
/// a human `SetBoost`/`ToggleBoost` passes through (`process_helm_inputs`,
/// which since issue #881 applies boost for EVERY ship, not just the
/// `LocalShip`), preserving human/AI symmetry (AGENTS.md #6).
///
/// Modelled on [`ai_helm_impulse`]: discrete and on-change. Availability (AC6) is
/// the presence of an *enabled* [`BoostConfigResource`] — no config, or a
/// feature-disabled one, and the system stands down without emitting. The
/// DECISION flows through [`HelmBoostAiPolicy`] resolving the `boost` channel to
/// the `engage_boost` mode verb over a fact snapshot seeded from the shared
/// hazard surface: fires ⇒ boost on, holds ⇒ boost off. The canonical default is
/// idle, so a ship that authors no `[helm_console.boost_ai]` never AI-boosts —
/// the pre-#780 baseline. It emits only when the desired state differs from the
/// current `ShipBoost`, so it does not re-issue `SetBoost` every tick.
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
            Option<&HelmBoostAiPolicy>,
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
        boost_policy,
        boost_state,
        entity_uuid,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own system alone — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_boost_system_id())
            .operate_ai
        {
            continue;
        }

        // Availability (AC6): the feature must be present AND enabled. No
        // BoostConfigResource, or one with the feature disabled, means no boost
        // capability — emit nothing (mirrors the shared applier's
        // `enabled`-guard in `process_helm_inputs`).
        let (Some(boost), Some(cfg)) = (boost_comp, boost_cfg) else {
            continue;
        };
        if !cfg.enabled {
            continue;
        }

        // Authored manoeuvre policy (issue #780, AC6): resolve the `boost`
        // channel over a fact snapshot seeded from the shared hazard surface and
        // availability. Fires ⇒ engage; holds ⇒ release.
        // Issue #883 also seeds the target-relative travel facts here, so an
        // authored escape-leg boost rule can guard on the pass geometry (range,
        // closing rate, speed fraction) and not just on hazard/state time.
        let mut facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            impulse_cfg.is_some(),
            true,
            physics.y,
            frame_red_alert(frame.ships.get(&entity)),
        );
        seed_helm_travel_facts(
            &mut facts,
            frame.ships.get(&entity),
            physics,
            physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
        );
        // No attached `[helm_console.boost_ai]` ⇒ no AI action on this axis.
        // Since #885b stage 5d there is no synthesised stand-in: strict
        // AI-declaration mode rejects an AI-capable hull that omits the block at
        // load, so an absent component means the declaration is missing and a
        // missing declaration gets no automation (PRD #774 US7).
        let Some(boost_policy) = boost_policy else {
            continue;
        };
        let policy = &boost_policy.0;
        // Stateless (the shipped shape) resolves exactly as it always has.
        // A policy that opted into the #882 machine instead resolves the SAME
        // channel inside its current state — committed earlier this tick by
        // `ai_policy_state_tick`, so the outputs are the new state's outputs
        // immediately (AC2). The shared helper also carries the #883
        // silent-degradation guard for the "machine declared, state component
        // missing" case that used to fall through unnoticed.
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(entity);
        let desired_active = resolve_helm_channel(
            policy,
            boost_state.map(|s| &s.0),
            crate::entities::config::HELM_BOOST_CHANNEL,
            &facts,
            clock.0,
            &flag_chain,
        ) == Some(&crate::ai::policy::AiPolicyVerb::EngageBoost);

        // On-change only: `SetBoost` sets the desired active state, and the
        // shared integrator applies the transition; re-issuing an unchanged state
        // every tick is redundant. Mirrors `ai_helm_impulse`'s NoChange skip.
        if desired_active == boost.0.is_active() {
            continue;
        }

        emit_ai_command(
            entity_uuid,
            crate::system_registry::helm_boost_system_id(),
            crate::messages::SystemControlPayload::SetBoost {
                active: desired_active,
            },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}
