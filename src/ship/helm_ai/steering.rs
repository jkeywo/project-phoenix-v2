use super::*;

/// Per-ship inline stateless **Steering** AI policy (issue #779). Mirror of
/// [`HelmEnginesAiPolicy`] for the `yaw` channel: from the authored
/// `[helm_console.steering_ai]` block. Read by [`ai_helm_steering`] to decide
/// whether to actuate the planner's desired facing.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmSteeringAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship runtime state for a STATEFUL Steering policy (issue #883). The
/// Steering twin of [`HelmEnginesAiPolicyState`]; this is the one whose current
/// state decides which leg [`HelmPassSurface`] publishes, because the yaw
/// channel is the axis that carries the two different facing verbs.
#[derive(Component, Clone, Debug, Default)]
pub struct HelmSteeringAiPolicyState(pub crate::ai::policy::AiPolicyRuntimeState);

/// Per-axis helm AI: steering. Decides the yaw for ships whose helm-steering
/// system is AI-operated and emits it as an admitted `SetSteering` into the
/// ship's own `AdmittedCommands` (issues #800, #704, #824); it owns the
/// arc-bearing step outright.
///
/// Steers toward the selected waypoint/target chosen by the pure
/// `crate::ai::operate_helm`, which resolves the top-scored Helm-relevant
/// directive. That includes the **Retreat consumer** (issue #688): when
/// `AiDirective::Retreat` is the top-scored directive, `operate_helm`'s Retreat
/// arm resolves its named anchor and steers toward it.
/// `ai_helm_steering_retreats_toward_anchor` pins that behaviour through this
/// system, and `ai_helm_steering_retreat_with_unknown_anchor_falls_through`
/// pins the other side of it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_steering(
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
            // Availability of the two optional drives — see `ai_helm_thrust`.
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&HelmSteeringAiPolicy>,
            Option<&HelmSteeringAiPolicyState>,
            Option<&mut PendingArcBearingRequest>,
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
        steering_policy,
        steering_state,
        mut pending_bearing,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Gate on our own axis alone (issue #800) — see the module note above.
        if !sources
            .0
            .policy_for(&crate::system_registry::helm_steering_system_id())
            .operate_ai
        {
            continue;
        }
        // Base steering comes from the shared desired-motion contract published
        // by the motion planner this tick (issue #741): decode the yaw intent
        // from the ship's 3D desired facing. Arc-bearing (issue #677) is a
        // facing override this axis still owns, applied on top and resolved
        // against the frame's merged view.
        let (Some(sp), Some(sf)) = (plan.ships.get(&entity), frame.ships.get(&entity)) else {
            continue;
        };

        // Resolve the data-authored #779 Steering policy's `yaw` mode verb to
        // decide WHETHER to actuate this tick (see `ai_helm_thrust` for the
        // mode-verb rationale). "Hold" emits nothing, so `SteeringInput` keeps
        // the last yaw fraction it was given — including any pending arc-bearing
        // this axis owns. It does NOT coast: the integrator applies that latched
        // fraction every tick, so a held steering axis is a ship turning at a
        // constant rate for ever, not one settling straight. Issue #968: a hull
        // whose steering system was destroyed circled out of its scenario on
        // exactly that mechanism, which the integrator's capability gate and the
        // offline clear in `process_helm_inputs` now close. Whether a policy
        // "hold" should ALSO decay toward neutral is a separate, open question.
        //
        // Issue #883 gives this channel a SECOND mode verb and (AC5) a really
        // seeded fact snapshot. `hold_committed_heading` actuates exactly like
        // `actuate_desired_facing` here — both emit the planner's decoded yaw —
        // because the difference between them was already resolved upstream:
        // the planner solved the facing against the FROZEN heading rather than
        // against the moving target. That is deliberate. Overriding
        // `SteeringInput` here instead would bypass the planner, and #780's
        // hazard contribution would stop composing onto the escape (AC3).
        // The availability pair is seeded honestly from the ship's own config
        // resources — see the note in `ai_helm_thrust`.
        // No attached `[helm_console.steering_ai]` ⇒ no AI action on this axis.
        // Since #885b stage 5d there is no synthesised stand-in: strict
        // AI-declaration mode rejects an AI-capable hull that omits the block at
        // load, so an absent component means the declaration is missing and a
        // missing declaration gets no automation (PRD #774 US7).
        let Some(steering_policy) = steering_policy else {
            continue;
        };
        let policy = &steering_policy.0;
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
        let actuates = matches!(
            resolve_helm_channel(
                policy,
                steering_state.map(|s| &s.0),
                crate::entities::config::HELM_YAW_CHANNEL,
                &facts,
                clock.0,
                &flag_chain,
            ),
            Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing)
                | Some(&crate::ai::policy::AiPolicyVerb::HoldCommittedHeading)
                // Issue #788's two recovery mode verbs actuate here identically
                // too, and for the identical reason: the difference between them
                // was already resolved upstream, by the planner solving the
                // facing against a ring tangent or against the target rather
                // than against a frozen heading. Overriding `SteeringInput` here
                // would bypass the planner and stop hazard avoidance composing
                // onto the orbit.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldRecoveryOrbit)
                | Some(&crate::ai::policy::AiPolicyVerb::PivotToReengage)
                // ...and issue #790's combat broadside orbit, for the third
                // time and the same reason: the planner already solved the
                // facing against the fighting ring's tangent, so this axis's
                // only job is to emit it.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldCombatOrbit)
                // ...and issue #791's torpedo-opportunity bow hold, for the
                // fourth time and the same reason: the planner already solved
                // the facing against the target's live position, so this axis's
                // only job is to emit it — and emitting it through the planner
                // is what keeps hazard avoidance composing onto the hold.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldTorpedoBearing)
                // ...and issue #792's artillery firing position, for the fifth
                // time and the same reason: the planner already solved the
                // facing against the PREDICTED intercept, so this axis's only
                // job is to emit it — and emitting it through the planner is
                // what keeps hazard avoidance composing onto the hold.
                | Some(&crate::ai::policy::AiPolicyVerb::HoldArtilleryPosition)
        );
        if !actuates {
            continue;
        }

        let mut steering =
            crate::ai::decode_steering_from_facing(sp.motion.desired_facing_local.to_array());

        // ── Weapons->Helm arc-bearing request (issue #677) ───────────────
        // Gated on a live objective, matching the pre-#741 shape: with nothing
        // to pursue the ship holds its facing rather than turning to bear.
        //
        // ...and on the consent of the leg this helm is flying (issue #918).
        // The request replaces `steering` outright with a bow-on tracking
        // solution, which is right for a helm that is merely travelling and
        // wrong for one whose doctrine has committed to a heading: on the
        // cruiser's broadside ring the tubes can never bear from the tangent, so
        // an obeyed request hauled the hull bow-on every tick and sawtoothed it
        // through the enemy's envelope. The question asked is what THIS helm is
        // doing — the authored leg, resolved from the ship's own steering
        // machine — never who raised the request; admission has already stripped
        // that and there is nothing here to branch on (AGENTS.md #6). A helm
        // with no doctrine leg to defend answers `true` and behaves exactly as
        // it did before.
        let leg_yields =
            policy.leg_yields_to_arc_requests(steering_state.map(|s| s.0.current.as_str()));
        if sf.has_objective {
            apply_arc_bearing_request(
                &mut steering,
                pending_bearing.as_deref_mut(),
                &sf.merged_view,
                physics,
                leg_yields,
            );
        }

        emit_helm_ai_command(
            entity_uuid,
            crate::system_registry::helm_steering_system_id(),
            crate::messages::SystemControlPayload::SetSteering { value: steering },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}
