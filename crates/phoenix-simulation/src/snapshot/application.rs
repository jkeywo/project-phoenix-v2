//! Application.
use super::*;

/// The inverse of [`world_event_record`]. An unrecognised tag is `None` — a save
/// from a build with an event kind this one does not have, which the version
/// gate is what refuses, not a `match` arm here.
pub(super) fn world_event_from_record(row: &WorldEventRecord) -> Option<WorldEvent> {
    Some(match row.kind {
        13 => WorldEvent::Activity(row.activity.clone()?),
        12 => {
            let valid = |value: &str| {
                !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
            };
            if !valid(&row.subject)
                || !valid(&row.other)
                || !valid(&row.system)
                || !crate::world::config::valid_enquiry_aim(&row.aim)
            {
                return None;
            }
            WorldEvent::Enquiry {
                target_uuid: row.subject.clone(),
                observer_uuid: row.other.clone(),
                system: phoenix_model::identity::SystemId(row.system.clone()),
                aim: row.aim.clone(),
                intrusive: row.intrusive,
                phase: row.enquiry_phase?,
            }
        }
        0 => WorldEvent::Destroyed {
            uuid: row.subject.clone(),
        },
        1 => WorldEvent::Attacked {
            uuid: row.subject.clone(),
            attacker_uuid: row.other.clone(),
        },
        2 => WorldEvent::HullDroppedBelow {
            uuid: row.subject.clone(),
            previous_fraction: row.numbers[0],
            current_fraction: row.numbers[1],
        },
        3 => WorldEvent::TimerElapsed {
            elapsed_secs: row.numbers[0],
        },
        4 => WorldEvent::Hailed {
            target_uuid: row.subject.clone(),
        },
        5 => WorldEvent::FlagSet {
            name: row.subject.clone(),
            origin_layer: row.origin_layer.clone(),
        },
        6 => WorldEvent::FlagCleared {
            name: row.subject.clone(),
            origin_layer: row.origin_layer.clone(),
        },
        7 => WorldEvent::WorldLoaded,
        8 => WorldEvent::EnteredRegion {
            uuid: row.subject.clone(),
        },
        9 => WorldEvent::ExitedRegion {
            uuid: row.subject.clone(),
        },
        10 => WorldEvent::WaypointReached {
            uuid: row.subject.clone(),
            waypoint: row.other.clone(),
        },
        11 => WorldEvent::Disabled {
            uuid: row.subject.clone(),
            disabled: row.disabled,
        },
        _ => return None,
    })
}

/// Refill [`crate::ship::helm_planner::HelmMotionPlan`] from the captured rows,
/// re-keying each by the restored world's own `Entity` (issue #1242).
///
/// The counterpart to [`rebuild_ai_world_snapshot`], and deliberately NOT the
/// same shape: that one RE-DERIVES its resource because it can, from inputs the
/// restore has already put back. This one cannot. The plan was computed on the
/// last AI tick, against that tick's positions; a restore lands on whatever tick
/// the capture was taken at — tick 399 of a 60 Hz run whose AI cadence is 30 Hz,
/// in the duel this issue was found in — and re-running the planner there would
/// plan against the positions physics has since integrated, not the ones the
/// plan was made from. That is a different plan, not a reconstructed one.
pub(super) fn restore_motion_plans(world: &mut World, snapshot: &PhoenixSnapshot) {
    let rows: std::collections::HashMap<&str, &MotionPlanState> = snapshot
        .entities
        .iter()
        .filter_map(|row| Some((row.uuid.as_str(), row.motion_plan.as_ref()?)))
        .collect();
    if rows.is_empty() {
        return;
    }
    let mut by_entity: std::collections::HashMap<
        Entity,
        crate::ship::helm_planner::ShipMotionPlan,
    > = std::collections::HashMap::new();
    {
        let Some(mut query) = world.try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>() else {
            return;
        };
        for (entity, uuid) in query.iter(world) {
            let Some(row) = rows.get(uuid.0.as_str()) else {
                continue;
            };
            by_entity.insert(
                entity,
                crate::ship::helm_planner::ShipMotionPlan {
                    motion: crate::ship::helm_planner::DesiredMotion {
                        desired_velocity_local: Vec3::from_array(row.desired_velocity_local),
                        desired_facing_local: Vec3::from_array(row.desired_facing_local),
                    },
                    hazard: crate::ship::helm_planner::HazardAssessment {
                        hazard_forces: Vec3::from_array(row.hazard_forces),
                        urgency: row.urgency,
                        primary_hazard: row
                            .primary_hazard
                            .as_deref()
                            .and_then(|u| uuid::Uuid::parse_str(u).ok()),
                        moving_hazard_threat: row.moving_hazard_threat,
                    },
                    docking_active: row.docking_active,
                },
            );
        }
    }
    if let Some(mut plan) = world.get_resource_mut::<crate::ship::helm_planner::HelmMotionPlan>() {
        plan.ships = by_entity;
    }
}

/// Replace the complete NPC frequency-match map after rebinding its ship keys.
///
/// Even `Some(default)` reaches `replace_continuation`: an empty capture is an
/// instruction to clear timers that the fresh bootstrap may have started.
pub(super) fn restore_npc_frequency_matches(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    let Some(stored) = &snapshot.npc_frequency_matches else {
        return;
    };
    let Some(mut query) = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>(
        )
    else {
        return;
    };
    let entities: std::collections::HashMap<_, _> = query
        .iter(world)
        .map(|(entity, uuid)| (uuid.0.clone(), entity))
        .collect();
    let mut restored = std::collections::HashMap::with_capacity(stored.states.len());
    for (uuid, state) in &stored.states {
        let Some(entity) = entities.get(uuid).copied() else {
            report.gaps.push(RestoreGap::MissingEntity(uuid.clone()));
            continue;
        };
        restored.insert(
            entity,
            crate::console_ai::FrequencyMatchState::from_continuation(
                state.current_target.clone(),
                state.elapsed_secs,
                state.match_sent,
            ),
        );
    }
    if let Some(mut runtime) =
        world.get_resource_mut::<crate::console::weapons::NpcFrequencyMatchStates>()
    {
        runtime.replace_continuation(restored);
    }
}

/// Replace the fresh bootstrap's unread Coordination suffix.
///
/// The `Messages` buffers alone are insufficient because consumed entries stay
/// retained for up to two updates. The resource cursor is the authority: clear
/// the destination buffers, align that cursor to the now-empty monotonic
/// message count, then append the projected suffix so the handler sees it once.
/// Source uuids resolve only after [`restore_entities`] has rebuilt/mapped the
/// captured roster.
pub(super) fn restore_coordination_staging(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    use crate::ship::components::{CoordinationEnqueue, CoordinationEnqueueCursor};
    use crate::ship::control_source::ControlSource;

    let Some(mut query) = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>(
        )
    else {
        if !snapshot.coordination_staging.is_empty() {
            report
                .gaps
                .push(RestoreGap::CoordinationStagingUnavailable {
                    pending: snapshot.coordination_staging.len(),
                });
        }
        return;
    };
    let entities: Vec<_> = query
        .iter(world)
        .map(|(entity, uuid)| (uuid.0.clone(), entity))
        .collect();
    let mut restored = Vec::with_capacity(snapshot.coordination_staging.len());
    for stored in &snapshot.coordination_staging {
        let Some((_, source_entity)) = entities
            .iter()
            .find(|(uuid, _)| uuid == &stored.source_uuid)
        else {
            report
                .gaps
                .push(RestoreGap::MissingEntity(stored.source_uuid.clone()));
            continue;
        };
        restored.push(CoordinationEnqueue {
            source_entity: *source_entity,
            sender_origin: match stored.sender_origin {
                0 => ControlSource::Human,
                1 => ControlSource::Ai,
                3 => ControlSource::Simplified,
                _ => ControlSource::Offline,
            },
            address: stored.address.clone(),
            payload: stored.payload.clone(),
            presentation: stored.presentation.clone(),
            sender_label: stored.sender_label.clone(),
            sender_system: SystemId(stored.sender_system.clone()),
        });
    }

    if !world.contains_resource::<Messages<CoordinationEnqueue>>()
        || !world.contains_resource::<CoordinationEnqueueCursor>()
    {
        if !snapshot.coordination_staging.is_empty() {
            report
                .gaps
                .push(RestoreGap::CoordinationStagingUnavailable {
                    pending: snapshot.coordination_staging.len(),
                });
        }
        return;
    }
    world.resource_scope(|world, mut cursor: Mut<CoordinationEnqueueCursor>| {
        let mut messages = world.resource_mut::<Messages<CoordinationEnqueue>>();
        messages.clear();
        cursor.0.clear(&messages);
        let _ = messages.write_batch(restored);
    });
}

/// Re-derive the AI cadence latches from the restored [`SimTick`].
///
/// The latches are intentionally absent from the payload because they are pure
/// functions of the tick and authored rates. The fresh bootstrap nevertheless
/// leaves them phased for ITS tick; without this immediate re-derivation the
/// first post-restore frame can consume a coincidentally armed or disarmed
/// latch before `FixedLast` corrects it.
pub(super) fn restore_ai_cadence(world: &mut World) {
    // A partial bare-World fixture may not have installed the cadence
    // resources. That is the same best-effort partial-restore contract used by
    // `restore_ai_world_snapshot` below.
    crate::ai::cadence::rederive_ai_cadence(world);
}

/// Rebuild each ship's `ShipModifiers` from the reactor allocation this restore
/// just wrote, for [`rebuild_ai_world_snapshot`]'s reason: the modifier cache is
/// a per-tick DERIVATION of power (issue #952 — WEAPONS buys `PhaserDamage`,
/// SHIELDS buys `ShieldRegen`, HELM buys `MaxSpeed`/`MaxYawRate`), and a fresh
/// app's bootstrap left it at the seeded rest values while [`PowerState`]
/// restored the allocation those modifiers derive from. Without this, the first
/// tick after the restore integrates beam damage and shield regen at the wrong
/// intensity for exactly one tick — invisible to `world_digest`, which leaves
/// shield charge deferred from the fold, and caught instead by the shield-charge
/// continuation in `tests/snapshot_resume.rs`, which reads `ShipShields` off the
/// live and resumed worlds directly and parts within a frame if this cache is
/// left at its seeded default. The system is a pure
/// function of restored power, so running it once here settles the cache before
/// the first tick reads it; the per-tick `translate_power_modifiers` recomputes
/// the identical values from then on. Errors are swallowed for the same
/// partial-world reason as the snapshot rebuild below.
pub(super) fn rebuild_power_modifiers(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    let _ = world.run_system_once(crate::modifiers::coordination::translate_power_modifiers);
}

/// Rebuild the AI's `WorldSnapshot` from the world this restore just wrote.
///
/// The one derivation a restore has to force, and the reason is the *cadence*
/// rather than the data. `build_world_snapshot` runs under
/// `run_if(ai_snapshot_ready)`, a latch that is a pure function of `SimTick`
/// (issue #895's anchor) — and [`restore_run_scope`] has just moved `SimTick`
/// to the capture's. So the resumed world's next arm lands on the same tick the
/// live world's does, which is right, but every tick *between* the restore and
/// that arm is spent steering from whatever snapshot the bootstrap happened to
/// leave behind — for a fresh app stopped the moment its roster appeared, an
/// empty one.
///
/// The measured consequence, and the last one this slice's continuation test
/// found: the resumed ships' radar-gated world view held no contacts at all, so
/// `seed_helm_travel_facts` resolved no target, `HelmRecoveryHistory` cleared
/// itself on the target switch, and both machines fell out of `torpedo_run` and
/// `inbound` into `acquire` on the first AI tick after a restore whose digest
/// had matched exactly.
///
/// Rebuilt rather than *stored*: the snapshot is a pure function of the world
/// at its tick, and after this restore the world already stands at that tick.
/// Putting a derivation in the save would be storing an answer the payload can
/// recompute — and one that a later build might compute differently.
/// Put the AI's shared world view back as it stood at the capture (issue #1242),
/// falling back to a rebuild for a save that predates the field.
///
/// The carried view WINS whenever there is one, and [`PhoenixSnapshot::ai_world`]
/// explains why at length: the view is built on the slowest AI cadence, so it is
/// deliberately behind the world, and rebuilding it produces a view that is right
/// for the restore tick rather than the one the run was steering from.
///
/// The rebuild survives as the pre-format-12 path. It is better than nothing —
/// which is what a resumed world otherwise has, the bootstrap's own view of its
/// own fight — and it is what those saves were verified against.
pub(super) fn restore_ai_world_snapshot(world: &mut World, snapshot: &PhoenixSnapshot) {
    if let Some(entities) = &snapshot.ai_world {
        if let Some(mut view) = world.get_resource_mut::<crate::ai::server::WorldSnapshot>() {
            view.entities = entities.clone();
            return;
        }
    }
    rebuild_ai_world_snapshot(world);
}

pub(super) fn rebuild_ai_world_snapshot(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    // Errors are swallowed on purpose: a bare-`App` fixture with the AI plugin
    // absent has no `WorldSnapshot` to rebuild, and that is the same
    // "partial world, partial restore" contract `capture` keeps.
    let _ = world.run_system_once(crate::ai::server::build_world_snapshot);
}

/// The run-scope resources, written last so the entity walks above cannot see a
/// half-updated tick.
pub(super) fn restore_run_scope(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    world.insert_resource(SimTick(snapshot.tick));

    world.insert_resource(crate::gm::actions::action::SimulationPaused(
        snapshot.paused,
    ));
    // The journal is itself folded authoritative state, so restore it exactly.
    // In particular a technical GM-join hold may make `SimulationPaused` true
    // without being a durable GM action; changing an empty journal's initial
    // baseline here would make the restored digest differ from the sender's.
    // The first later typed grant adopts the current pause on every peer at its
    // replicated admission boundary (`lockstep::apply_mesh_inbox`), which keeps
    // Resume effective without rewriting the captured history.
    let gm_actions = snapshot.gm_actions.clone();
    world.insert_resource(gm_actions.clone());
    // The journal's terminal result log is derived, never stored. Replace a
    // bootstrap's stale projection immediately so host-channel readers cannot
    // observe pre-restore results before the next PreUpdate recomputes them.
    world.insert_resource(gm_actions.applied_log());
    crate::gm::observation::activity::rebase_after_restore(world);
    // The quiet clock counts SIMULATION ticks, and the restore above just moved
    // `SimTick` to somebody else's. Rebase it onto the restored tick so the
    // continuation neither opens on a lull that belongs to the captured session
    // nor carries the pre-restore one forward under its old identity
    // (issue #1436).
    crate::gm::observation::quiet::rebase_after_restore(world);
    world.insert_resource(snapshot.gm_puppets.clone());
    // Rebase derived membership history to the restored authoritative set so
    // the first continuation pass cannot interpret every active target as a
    // fresh takeover or miss the next release.
    world.insert_resource(
        crate::gm::operations::puppet::PreviousStationPuppetTargets::from_puppets(
            &snapshot.gm_puppets,
        ),
    );
    // Unlike ordinary network receipt queues, these payloads already passed
    // live Station/System admission and are part of the captured boundary.
    world.insert_resource(snapshot.gm_station_commands.clone());
    world.insert_resource(snapshot.gm_direct_effects.clone());
    // Re-apply the save's GM faction decisions to the live registry, and revert
    // any pair this run moved that the save never did. Doing this through the
    // ordinary `add_enemy`/`remove_enemy` vocabulary keeps restore and the live
    // action on one path.
    crate::gm::operations::faction::restore_overrides(world, &snapshot.gm_faction_overrides);
    // The exposure stopwatch and its latch (issue #1443). Inserted absolutely,
    // like every other captured authoritative resource: a placement the save
    // never made must not keep a counter this run started, and a latch the save
    // DID carry must come back closed.
    world.insert_resource(snapshot.gm_spawn_exposure.clone());
    // The idle-NPC stopwatch (issue #1435), only into a peer that actually keeps
    // one. Inserting it on a peer with no attention queue would create a
    // resource no system maintains, which is worse than the honest absence:
    // there is nobody there to be advised.
    if world.contains_resource::<crate::gm::observation::attention::GmIdleNpcWatch>() {
        world.insert_resource(snapshot.gm_idle_npcs.clone());
    }
    // The sustained-overload stopwatch (issue #1438), on the same terms.
    crate::gm::observation::workload::restore_watch(world, &snapshot.gm_station_workload);
    if !snapshot.objective_records.is_empty()
        || world.contains_resource::<crate::world::server::ObjectiveManagerRes>()
    {
        let mut manager = crate::world::server::ObjectiveManagerRes::default();
        manager
            .0
            .restore_records(snapshot.objective_records.clone());
        world.insert_resource(manager);
    }
    world.insert_resource(crate::world::server::ObjectiveInstanceManagerRes(
        snapshot.objective_instances.clone(),
    ));
    if let Some(slots) = snapshot.frozen_ship_slots.clone() {
        world.insert_resource(slots);
    }
    // Consumer reply routes are transient and reconstructed from the accepted
    // pending commands above.  A pre-restore route must never settle a command
    // belonging to the new continuation.
    world.insert_resource(crate::gm::operations::puppet::PendingGmStationFeedbackRoutes::default());
    // Activity is presentation rebuilt from the restored timeline.
    world.insert_resource(crate::gm::operations::puppet::StationPuppetActivity::default());
    if snapshot.paused {
        // Pausing is always safe and must take effect before this frame can enter
        // FixedUpdate. Do not symmetrically unpause here: lockstep recovery,
        // model readiness, and peer stalls share Time<Virtual>, their gate ran
        // earlier in this PreUpdate frame, and releasing their hold here could
        // admit one forbidden tick. The next gate frame unpauses iff every hold
        // (including this restored product pause) is clear.
        if let Some(mut virtual_time) = world.get_resource_mut::<Time<bevy::time::Virtual>>() {
            virtual_time.pause();
            // `TimeSystem` already computed this rendered frame's delta in
            // First. Pausing prevents later frames, but RunFixedMainLoop would
            // still accumulate the current delta unless restore consumes it.
            virtual_time.advance_by(std::time::Duration::ZERO);
        }
    }

    if let (Some(stored_nanos), Some(mut fixed)) = (
        snapshot.fixed_overstep_nanos,
        world.get_resource_mut::<Time<bevy::time::Fixed>>(),
    ) {
        // Replace, do not add to, whatever fraction the bootstrap accumulated.
        // Both operations are exact `Duration` arithmetic; no seconds/f32
        // projection is allowed at this fixed-step boundary.
        let bootstrap_overstep = fixed.overstep();
        fixed.discard_overstep(bootstrap_overstep);
        fixed.accumulate_overstep(std::time::Duration::from_nanos(stored_nanos));
        if snapshot.paused {
            // A canonical capture carries only interpolation remainder here.
            // Preserve that exact fraction while refusing any malformed whole
            // step of pre-restore debt from running in the restore frame.
            crate::lockstep::discard_whole_fixed_overstep(&mut fixed);
        }
    }

    if let Some(mode) = snapshot.phaser_mode {
        world.insert_resource(crate::console::weapons::CurrentPhaserMode(mode));
    }

    if let Some(state) = snapshot.rng.clone() {
        match SimRng::from_state(state) {
            Some(rng) => crate::sim_rng::install(world, rng),
            None => report.gaps.push(RestoreGap::RngStreamsMoved),
        }
    }

    if let Some(state) = snapshot.mint.clone() {
        crate::world_id::install(world, WorldIdMint::from_state(state));
    }

    let mut restored_phase_entry: Option<GamePhase> = None;
    if let Some(phase) = snapshot.phase.clone() {
        // `State::new` rather than `NextState`: a queued transition applies on
        // the next `StateTransition`, which is a step this restore has not run
        // yet and must not depend on. The captured phase is where the world
        // already *is*.
        //
        // But `State::new` also skips `OnEnter`/`OnExit` entirely (issue #934),
        // and for a phase actually changing under this restore that silently
        // drops whatever that phase's entry effects do. Note the transition
        // here — before the direct write below — and let
        // `run_restored_phase_entry_effects` decide, once the rest of this
        // function has finished writing the resources those effects read.
        let previous = world
            .get_resource::<State<GamePhase>>()
            .map(|s| s.get().clone());
        if previous.as_ref() != Some(&phase) {
            restored_phase_entry = Some(phase.clone());
        }
        world.insert_resource(State::new(phase));
    }

    if let Some((reason, outcome)) = snapshot.game_over.clone() {
        let outcome = outcome
            .as_deref()
            .and_then(|label| crate::core::balance::Outcome::parse(label).ok());
        world.insert_resource(GameOverReason(reason, outcome));
    }

    // The post-mission report (issue #1344). Written unconditionally, unlike
    // the blocks either side of it: an EMPTY report is a real state — "this run
    // has recorded nothing yet" — and leaving a stale resource in place when
    // resuming into it would resurrect rows the captured tick did not have.
    // Order is restored verbatim, because order is content here.
    {
        let rows = snapshot
            .mission_report
            .iter()
            .filter_map(|row| {
                // An unrecognised state label drops the row rather than guessing
                // a fate — the same refusal `Outcome::parse` makes just above.
                let state = crate::core::report::ReportRowState::parse(&row.state).ok()?;
                Some(crate::core::report::ReportRow {
                    id: row.id.clone(),
                    heading_id: row.heading.clone(),
                    outcome_id: row.outcome.clone(),
                    state,
                    score: row.score,
                })
            })
            .collect();
        world.insert_resource(crate::core::report::MissionReport::from_rows(rows));
    }

    if !snapshot.captain_boosts.is_empty() {
        // `toggle` on an empty store inserts; there is no bulk setter and this
        // needs none — `CaptainPriorityBoost::default()` is empty by
        // construction, so one toggle per pair reproduces the map exactly.
        let mut boosts = CaptainPriorityBoost::default();
        for (scope, objective) in &snapshot.captain_boosts {
            boosts.toggle(scope, objective);
        }
        world.insert_resource(boosts);
    }

    if let Some(data) = snapshot.world.clone() {
        world.insert_resource(WorldResource(data));
    }

    if let Some(stored) = &snapshot.tracked_entities {
        world.insert_resource(crate::server_app::TrackedEntities {
            reported: stored.reported.iter().cloned().collect(),
            seeded: stored.seeded,
        });
    }

    if let Some(flags) = snapshot.flags.clone() {
        if let Some(mut runtime) = world.get_resource_mut::<WorldContentRuntime>() {
            runtime.flags = flags;
        }
    }

    if let Some(secs) = snapshot.ai_policy_clock {
        world.insert_resource(crate::ship::helm_ai::AiPolicyTickClock(secs));
    }

    if !snapshot.layer_flags.is_empty() {
        let mut missing = Vec::new();
        if let Some(mut layers) = world.get_resource_mut::<crate::world::server::WorldLayerMap>() {
            for layer in &snapshot.layer_flags {
                match layers.0.get_mut(&layer.path) {
                    Some(runtime) => {
                        runtime.flags = layer.flags.clone();
                        runtime.owned_objective_ids = layer.owned_objective_ids.clone();
                    }
                    None => missing.push(layer.path.clone()),
                }
            }
        } else {
            missing.extend(snapshot.layer_flags.iter().map(|layer| layer.path.clone()));
        }
        report
            .gaps
            .extend(missing.into_iter().map(RestoreGap::MissingWorldLayer));
    }

    restore_scenario(world, snapshot, report);
    restore_comms(world, snapshot, report);

    restore_collisions(world, snapshot);

    // Last, now that every resource an entry effect might read (`GameOverReason`
    // above included) carries the restored value.
    if let Some(phase) = restored_phase_entry {
        run_restored_phase_entry_effects(world, phase);
    }
}

/// Put the scenario's progression back — see [`ScenarioState`].
///
/// Every write here is a wholesale replacement, [`apply_weapons`]' rule and for
/// its reason: the fresh app ran its *own* opening on the way to the restore
/// point, so merging would leave the resumed scenario holding a firing the
/// capture never made.
///
/// # Registry-owned continuation
///
/// The same content and layer activation order rebuild the paired trigger table.
/// Handlers and ASTs are not snapshot payload. The registry validates complete
/// row cardinality and index coverage before replacing any latch, retains each
/// row's handler, and advances its observer generation after successful restore.
/// A refusal reports a gap and leaves all trigger continuation untouched.
///
/// # The per-tick budget is reset, not restored
///
/// `WorldScriptRuntime::budget` is a per-tick circuit breaker and a capture is
/// taken at a tick boundary, with that tick's script work already complete — so
/// there is no partially-spent budget to preserve, and the live world's next
/// tick starts a fresh one. What IS written back is `budget_tick`, re-based to
/// the captured tick, and that is not cosmetic. Both script systems reset the
/// budget by comparing `budget_tick` against the *current* `SimTick`; the fresh
/// app's `budget_tick` is whatever tick its own bootstrap reached, and if that
/// number happened to equal the resumed world's next tick — a coincidence a
/// world whose fresh app runs hundreds of ticks can absolutely produce — the
/// resumed world would skip the reset and run its first continuation tick with
/// the bootstrap's operations already charged against it while the live world
/// ran with an empty budget. Re-basing makes the reset decision a function of
/// the captured state rather than of how far the fresh app happened to get.
pub(super) fn restore_scenario(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    let Some(stored) = snapshot.scenario.as_ref() else {
        return;
    };
    let now = world.get_resource::<Time<Fixed>>().map(Time::elapsed);
    let plan_definitions = world
        .get_resource::<crate::world::config::WorldConfig>()
        .map(|base| {
            let mut definitions = base.situation_definitions.clone();
            if let Some((compiled, choice)) = base
                .compiled_planning
                .as_ref()
                .zip(stored.plan_choice.as_ref())
            {
                let graph = compiled
                    .derive(choice)
                    .expect("whole-candidate plan preflight checked source graph");
                definitions.extend(graph.definitions.into_values());
            }
            if let Some(source) = &base.compiled_opportunity_search {
                for record in &stored.opportunities {
                    if let crate::world::opportunities::OpportunityState::Accepted {
                        input, ..
                    } = &record.state
                    {
                        let graph = source
                            .derive(&record.reference, input, &record.proposal)
                            .expect("whole-candidate optional source preflight");
                        definitions.extend(graph.definitions.into_values());
                    }
                }
            }
            definitions
        });

    if let Some(mut runtime) = world.get_resource_mut::<WorldContentRuntime>() {
        // Rebase exact age, with no subtraction or rounding through f32.
        runtime.mission_clock = stored.mission_elapsed.map(|elapsed| {
            crate::world::server::MissionClock::resume(
                now.expect("mission clock preflight requires fixed time"),
                elapsed,
            )
        });

        if let Err(error) = runtime.triggers.restore(&stored.triggers) {
            use crate::world::trigger_registry::TriggerRestoreError;
            report.gaps.push(match error {
                TriggerRestoreError::Cardinality { saved, found } => {
                    RestoreGap::ScenarioTriggersMoved { saved, found }
                }
                TriggerRestoreError::InvalidIndex { index } => {
                    RestoreGap::ScenarioTriggerIndexInvalid { index }
                }
            });
        }

        // The GM's armed Fires (issue #1301). Wholesale replacement, on this
        // walk's rule: the freshly-loaded world has none, and the capture's set
        // is the complete authoritative statement of what is still owed a run.
        runtime.pending_gm_event_fires = stored.pending_gm_event_fires.iter().cloned().collect();
        // And the armed Skips (issue #1304), on identical terms.
        runtime.pending_gm_event_skips = stored.pending_gm_event_skips.iter().cloned().collect();

        // The GM's armed placements (issue #1305), on the same rule and for the
        // same reason: the freshly-loaded world has none, and the capture's
        // queue is the complete statement of what is still owed — in the order
        // that decides which draws which uuid.
        runtime.pending_gm_spawns = stored.pending_gm_spawns.clone();
        runtime.pending_gm_despawns = stored.pending_gm_despawns.clone();
        // Wholesale replacement on this walk's rule (issue #1444): a
        // freshly-loaded world has removed nothing, so anything short of
        // replacement would leave a bootstrap's captures beside the saved
        // journal that is the only thing entitled to name them.
        runtime.gm_despawn_captures = stored.gm_despawn_captures.clone();
        runtime.contact_overrides = stored.contact_overrides.clone();
        runtime.contact_classifications = stored.contact_classifications.clone();
        runtime.presentation = stored.presentation.clone();
        runtime.contact_information = stored.contact_information.clone();
        // The GM's paused events (issue #1303). Wholesale replacement on the
        // same rule and for the sharper reason on `ScenarioState`: a
        // freshly-loaded world pauses nothing, so anything short of replacement
        // resumes conditions the GM stopped.
        runtime.paused_gm_events = stored.paused_gm_events.iter().cloned().collect();

        runtime.entity_groups = stored
            .entity_groups
            .iter()
            .map(|(group, members)| (group.clone(), members.iter().cloned().collect()))
            .collect();
        // Wholesale replacement (issue #863), on this walk's rule and with the
        // sharper reason `restore_entities` gives: the bootstrap's map names the
        // ships the bootstrap spawned, and the surplus sweep has just despawned
        // whichever of those the capture did not have. Merging would leave the
        // resumed scenario resolving a name to an entity that no longer exists,
        // which is how an `on_destroyed` handler waits forever for a ship that
        // has already been taken off the board.
        //
        // Empty in the payload means empty here, not "leave what you found":
        // this is the map a capture *had*, and a run with no named entities is a
        // real state a resumed world must be able to stand in.
        runtime.name_to_uuid = stored
            .name_to_uuid
            .iter()
            .map(|(name, uuid)| (name.clone(), uuid.clone()))
            .collect();
        runtime.observed_hull_fractions = stored
            .observed_hull_fractions
            .iter()
            .map(|(uuid, fraction)| (uuid.clone(), *fraction))
            .collect();
        runtime.situations = stored.situations.clone();
        runtime.plan_choice = stored.plan_choice.clone();
        runtime.opportunities = stored.opportunities.clone();
        // Source graph is derived once after candidate-only validation.
        if let Some(definitions) = plan_definitions {
            runtime.situation_definitions = definitions;
        }
        runtime.pending_world_events = stored
            .pending_events
            .iter()
            .filter_map(world_event_from_record)
            .collect();
        // Wholesale replacement, this module's rule throughout: the fresh app ran
        // its own `arm_mission_deadlines` on the way to the restore point, so
        // merging would leave the resumed world holding the bootstrap's due ticks
        // beside the capture's. Taking the table whole also carries its `armed`
        // latch, which is what stops the arming system re-arming over the top.
        runtime.deadlines = stored.deadlines.clone();
        // Wholesale replacement for the same rule (issue #1029), and here it is not
        // even a choice: a promise is only ever written by a script call, so the
        // fresh app's ledger is empty on the way to the restore point and merging
        // would be indistinguishable from replacing. Taking it whole is what stays
        // correct once #849's continuation log replays commands INTO a restored run.
        runtime.commitments = stored.commitments.clone();
        // Wholesale replacement again (issue #1031), on the ledger's terms and
        // with one extra reason of its own: the store deduplicates on
        // `(subject, provenance, text)` and keeps the FIRST tick, so a merge that
        // let the fresh app's own appends land first would re-stamp a finding at
        // the bootstrap's tick and quietly rewrite when the crew learned it.
        runtime.evidence = stored.evidence.clone();
        // And the workforce register (issue #1035), wholesale for the deadline
        // table's rule. The `armed` latch travels with it, which is what stops
        // `arm_mission_workforces` running on the resumed mission's first tick
        // and putting a settled strike back on.
        runtime.workforce = stored.workforce.clone();
    }

    match world.get_resource_mut::<WorldScriptRuntime>() {
        Some(mut script) => {
            script.pending_callbacks = PendingCallbacks(stored.script_callbacks.clone());
            script.reset_invocation_budget(snapshot.tick);
        }
        None if !stored.script_callbacks.is_empty() => {
            report.gaps.push(RestoreGap::ScriptRuntimeAbsent {
                pending_callbacks: stored.script_callbacks.len(),
            });
        }
        None => {}
    }
}

/// Put the conversation back — see [`CommsState`].
///
/// Wholesale replacement throughout, [`restore_scenario`]'s rule and for its
/// reason: the fresh app ran its own opening on the way to the restore point, so
/// merging would leave the resumed world holding a thread the capture never
/// opened. The inbox is rebuilt from empty rather than injected into, which is
/// the same statement made about a container `CommsInbox` has no bulk setter for
/// — `inject` skips duplicate ids, so injecting the captured rows in order into
/// a *cleared* inbox reproduces the record vec exactly.
///
/// # The scripted names resolve against the recompiled set
///
/// A [`DialogueState::script`] carries `(script_path, node_fn, on_pick[])` and
/// nothing else — no `AST`, no handle — so answering a restored dialogue means
/// looking `script_path` up in the *bootstrapped* world's
/// `WorldScriptRuntime::asts` and calling the named fn. What makes that the same
/// tree the capture was reading is issue #864's content binding, not anything
/// here: the load returns the compiled set's `content_hash` as a ledger record
/// for its caller to apply (issue #1241), `content_digest` folds it, and
/// `Versions::check` refuses a
/// save whose scripts moved before `restore` is ever reached. Editing a single
/// `on_pick` body therefore refuses the save rather than resolving the name
/// against a different fn. Underneath that, `enter_node` still refuses an
/// unresolvable name visibly (`EnterError::Unresolved` → the control flashes
/// red), and `validate_on_pick_fns` still lints the authored names at load.
pub(super) fn restore_comms(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    let Some(stored) = snapshot.comms.as_ref() else {
        return;
    };

    if let Some(mut inbox) = world.get_resource_mut::<CommsInboxRes>() {
        inbox.0 = crate::console::comms::inbox::CommsInbox::new();
        for message in &stored.inbox {
            inbox.0.inject(message.clone());
        }
        // A restore always owes its clients a push, whatever the inbox's own
        // dirty flag happened to be at the capture.
        inbox.0.mark_dirty();
    }

    if let Some(mut comms) = world.get_resource_mut::<CommsRuntime>() {
        comms.active_dialogues = stored
            .dialogues
            .iter()
            .map(|row| {
                (
                    row.message_id.clone(),
                    ActiveDialogue {
                        // The exact inverse of `reduce_dialogue_node` — see
                        // [`CommsState`] for why that is a copy rather than a
                        // reduction.
                        current_node: CommsDialogueNode {
                            body: row.body.clone(),
                            body_params: row.body_params.clone(),
                            responses: row
                                .responses
                                .iter()
                                .enumerate()
                                .map(|(i, (text, important))| CommsResponse {
                                    text: text.clone(),
                                    important: *important,
                                    // A short or absent parallel vector reads as
                                    // "no metadata" — see `DialogueState`.
                                    ai: row.response_ai.get(i).copied().unwrap_or_default(),
                                })
                                .collect(),
                        },
                        thread_id: row.thread_id.clone(),
                        script: row.script.clone(),
                    },
                )
            })
            .collect();

        comms.open_hails = stored.open_hails.clone();
        // The unmanned console's running waits resume where they were, rather
        // than restarting — see `CommsState::pending_ai_responses`.
        comms.pending_ai_responses = stored
            .pending_ai_responses
            .iter()
            .map(|(key, record)| (key.clone(), *record))
            .collect();
        // `range_flags` / `range_active` / `fleet_range_flags` /
        // `fleet_range_active` / `contacts` are all recomputed by
        // `update_comms_range_flags` on the next tick; what they need is for the
        // resumed world to push a fresh `CommsState` to its clients.
        comms.needs_broadcast = true;
    }

    let scripted_dialogues = stored.dialogues.len();
    match world.get_resource_mut::<WorldScriptRuntime>() {
        Some(mut script) => {
            script.pending_comms_opens = stored.pending_opens.clone();
        }
        // The dialogues are counted here too: a thread with no runtime behind it
        // is answerable by nothing, which is a loss the caller has to hear about
        // even when no open was queued.
        None if !stored.pending_opens.is_empty() || scripted_dialogues > 0 => {
            report.gaps.push(RestoreGap::CommsScriptRuntimeAbsent {
                pending_opens: stored.pending_opens.len(),
                scripted_dialogues,
            });
        }
        None => {}
    }
}

/// Re-run the observable entry effects of a restored phase transition that the
/// direct `State::new` write above skips (issue #934).
///
/// Not a blanket "run every restored phase's `OnEnter`" — that is wrong for
/// `InProgress` specifically. `ready_to_restore` (below) gates every restore on
/// the fresh app's own roster already existing, which only happens after that
/// app ran its *own* `OnEnter(InProgress)` for its own game start — the mint,
/// the spawns, the command-log reset. Re-running that schedule here would
/// re-spawn and re-reset exactly what the entity/asteroid restore above just
/// wrote. So `InProgress` gets nothing, on purpose. `Lobby` has no `OnEnter`
/// registered at all. `Loading` does (`broadcast_loading_start`), but it is a
/// transient phase a resumed run has no business landing in — capture refuses
/// to record it (see the guard where `capture` is called) — so there is
/// nothing to re-enter here either.
///
/// `GameOver` is the case the issue was filed for: a fresh app still
/// `InProgress` restoring a captured `GameOver` needs `on_game_over_enter`
/// (`server_app.rs`) and `push_game_over_hud_state`
/// (`server/viewscreen_border.rs`) to run, or the host never emits the
/// `GameOver` message and the HUD never leaves its live state. Both are
/// audited safe to re-run: they only *read* `GameOverReason` and
/// `MissionReport` — both restored above, before this call — and *write* an
/// outbox message, a HUD resource, and one `ReportFinalized` narrative beat.
/// None of the three spawns, despawns, or resets anything the rest of this
/// restore depends on.
///
/// That third effect is issue #1344's, and it is REPRODUCED on purpose rather
/// than suppressed. `on_game_over_enter` writes
/// `NarrativeKind::ReportFinalized` when the report holds rows, and a restored
/// report-bearing `GameOver` comes back holding exactly those rows — so
/// `core::balance::classify` still calls the resumed run `reported`. A resumed
/// timeline with no `report_finalized` in it would then contradict the outcome
/// printed beside it in the very same exit report. The beat is the ending's
/// observable effect on the after-action surface in precisely the way the
/// `GameOver` message is its effect on the wire, and putting those back is this
/// function's whole job.
///
/// It is also inert with respect to the continuation claim this module exists
/// to make: nothing authoritative reads `RunTelemetry::narrative_events` — no
/// digest stage folds it (see the note on that field) — so re-emitting the beat
/// cannot move a resumed run's simulation off the live one's. What it does NOT
/// do is reconstruct the *captured* run's timeline: the snapshot carries no
/// narrative telemetry at all, by the same design that keeps it out of the
/// digest. A resumed run's timeline is therefore what the resumed process
/// itself observed, ending beat included, and `tests/snapshot_resume.rs` pins
/// that count at exactly one.
///
/// Run via `OnEnter(GameOver)` itself rather than by naming the systems, so a
/// future addition to that schedule is covered by construction — but that also
/// means a system landing in `OnEnter(GameOver)` later needs this same audit
/// before it can be trusted here. #1344's `ReportFinalized` write is what that
/// sentence looks like when it comes due, and the paragraphs above are the
/// audit it asked for.
pub(super) fn run_restored_phase_entry_effects(world: &mut World, phase: GamePhase) {
    if phase == GamePhase::GameOver {
        let _ = world.try_run_schedule(OnEnter(GamePhase::GameOver));
    }
}

pub(super) fn restore_collisions(world: &mut World, snapshot: &PhoenixSnapshot) {
    crate::core::collision_history::restore(world, &snapshot.collisions);
    let cursor = world
        .get_resource::<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>()
        .map(bevy::ecs::message::Messages::get_cursor_current)
        .unwrap_or_default();
    let Some(mut telemetry) = world.get_resource_mut::<RunTelemetry>() else {
        return;
    };
    // Every non-collision event the bootstrap produced goes too. The resumed
    // run's telemetry is the capture's, not the capture's plus whatever the
    // bootstrap happened to generate on its way to the restore point.
    telemetry.balance_events.clear();
    telemetry.balance_events.extend(
        snapshot
            .collisions
            .iter()
            .map(CollisionRecord::stamped_event),
    );
    // Its report reader must also forget unread bootstrap events, without
    // clearing the shared message channel or swallowing restored entry events.
    telemetry.balance_cursor = cursor;
}

pub(super) fn validate_entity_repair_jobs(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    validate_entity_logistics(world, entity, row)?;
    match (
        world.get::<crate::resources::recovery_server::CompiledShipRecovery>(entity),
        world.get::<crate::resources::recovery_server::ShipRecovery>(entity),
        row.recovery.as_ref(),
    ) {
        (Some(profile), Some(_), Some(state)) => {
            if state.target.as_deref() == Some(row.uuid.as_str()) {
                return Err("recovery cannot target its own entity".into());
            }
            state.validate(&profile.0, state.last_tick)?;
            if let Some(destination) = &state.destination_store {
                let logistics = world
                    .get::<crate::resources::logistics_server::CompiledShipLogistics>(entity)
                    .ok_or("saved recovery has no fitted logistics profile")?;
                let store = logistics.0.store(destination)?;
                if store.role != phoenix_model::logistics::StoreRole::Main
                    || state.resource.as_ref() != Some(&store.resource)
                    || logistics.0.graph().compartment_for_system(&state.owner)
                        != Some(&store.compartment)
                {
                    return Err("saved recovery destination is not a compatible Main store in the tool compartment".into());
                }
            }
        }
        (None, None, None) => {}
        _ => return Err("saved recovery owner disagrees with authored equipment".into()),
    }
    match (
        world.get::<crate::ship::compartment_damage::ShipDamage>(entity),
        row.physical_damage.as_ref(),
    ) {
        (Some(damage), Some(state)) => state.validate(&damage.definition)?,
        (None, None) => {}
        _ => return Err("saved physical damage owner disagrees with authored equipment".into()),
    }
    match (
        world.get::<crate::ship::disablement::DisablementObservation>(entity),
        row.disablement.as_ref(),
    ) {
        (Some(_), Some(_)) | (None, None) => {}
        _ => return Err("saved disablement frontier disagrees with physical owner".into()),
    }
    match (
        world.get::<crate::ship::access::InteriorAccess>(entity),
        row.access.as_ref(),
    ) {
        (Some(access), Some(states)) => access.clone().restore(states)?,
        (None, None) => {}
        _ => return Err("saved access boundaries disagree with authored equipment".into()),
    }
    let Some(stored) = row.repair.as_ref() else {
        return if world.get::<ShipRepairTeams>(entity).is_some() {
            Err("saved repair state is missing for the authored team owner".into())
        } else {
            Ok(())
        };
    };
    match (world.get::<ShipRepairTeams>(entity), stored.jobs.as_ref()) {
        (Some(teams), Some(jobs)) => {
            teams.0.clone().restore_state(jobs)?;
            validate_physical_repair_claims(world, entity, row, jobs)
        }
        (None, None) => Ok(()),
        _ => Err("saved team owner is missing or disagrees with the authored crew".into()),
    }
}

/// Check cargo conservation against the saved endpoint owners, before any live
/// owner is written. Bootstrap quantities are not substitutes for saved stocks.
fn validate_entity_logistics(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    use crate::resources::logistics::{EndpointStock, ResourceEndpoint};
    use crate::resources::logistics_server::{CompiledShipLogistics, ShipLogistics};
    match (
        world.get::<CompiledShipLogistics>(entity),
        world.get::<ShipLogistics>(entity),
        &row.logistics,
    ) {
        (Some(profile), Some(_), Some(state)) => {
            if row.infrastructure.as_ref().is_some_and(|owner| {
                owner.capacities().iter().any(|capacity| {
                    !capacity.stock.incoming.is_empty()
                        && !profile.0.config().stores.iter().any(|store| {
                            store.endpoint
                                == (ResourceEndpoint::Capacity {
                                    id: capacity.id.clone(),
                                })
                        })
                })
            }) || row
                .weapons
                .as_ref()
                .and_then(|owner| owner.magazine.as_ref())
                .is_some_and(|stock| {
                    !stock.incoming.is_empty()
                        && !profile
                            .0
                            .config()
                            .stores
                            .iter()
                            .any(|store| store.endpoint == (ResourceEndpoint::TorpedoMagazine {}))
                })
            {
                return Err("saved incoming claim names an unmanaged endpoint".into());
            }
            let mut stocks = std::collections::BTreeMap::new();
            for binding in &profile.0.config().stores {
                let value = match &binding.endpoint {
                    ResourceEndpoint::Capacity { id } => EndpointStock {
                        stock: row
                            .infrastructure
                            .as_ref()
                            .and_then(|owner| owner.capacity_stock(id))
                            .ok_or("saved logistics is missing its capacity stock")?
                            .clone(),
                        held: 0,
                    },
                    ResourceEndpoint::TorpedoMagazine {} => {
                        let weapons = row
                            .weapons
                            .as_ref()
                            .ok_or("saved logistics has no weapon owner")?;
                        let held = weapons
                            .tubes
                            .iter()
                            .try_fold(0i64, |sum, tube| {
                                sum.checked_add(i64::from(tube.loaded_count))
                            })
                            .and_then(|sum| {
                                weapons.bursts.iter().try_fold(sum, |sum, burst| {
                                    sum.checked_add(i64::from(burst.pending))
                                })
                            })
                            .ok_or("saved local magazine occupancy overflows")?;
                        EndpointStock {
                            stock: weapons
                                .magazine
                                .clone()
                                .ok_or("saved logistics has no magazine stock")?,
                            held,
                        }
                    }
                };
                stocks.insert(binding.endpoint.clone(), value);
            }
            match (
                world.get::<crate::resources::production_server::CompiledShipProduction>(entity),
                world.get::<crate::resources::production_server::ShipProduction>(entity),
                &row.production,
            ) {
                (Some(production), Some(_), Some(work)) => {
                    work.validate(&production.0, &stocks, state)
                }
                (None, None, None) => state.validate(&profile.0, &stocks),
                _ => Err("saved production owner disagrees with authored equipment".into()),
            }
        }
        (None, None, None) => {
            if row.production.is_some()
                || world
                    .get::<crate::resources::production_server::ShipProduction>(entity)
                    .is_some()
            {
                return Err("saved production has no fitted logistics owner".into());
            }
            let incoming = row.infrastructure.as_ref().is_some_and(|owner| {
                owner
                    .capacities()
                    .iter()
                    .any(|capacity| !capacity.stock.incoming.is_empty())
            }) || row
                .weapons
                .as_ref()
                .and_then(|owner| owner.magazine.as_ref())
                .is_some_and(|stock| !stock.incoming.is_empty());
            if incoming {
                Err("saved incoming claims have no fitted logistics owner".into())
            } else {
                Ok(())
            }
        }
        _ => Err("saved logistics owner disagrees with authored equipment".into()),
    }
}

pub(super) fn validate_physical_repair_claims(
    world: &World,
    entity: Entity,
    row: &EntityState,
    jobs: &crate::jobs::RepairJobState,
) -> Result<(), String> {
    use crate::resources::service::{resolve_endpoint, ResourceEndpoint, ResourceLink};
    let mut expected = std::collections::BTreeMap::new();
    for (id, work) in &jobs.physical_work {
        if !work.reserved {
            continue;
        }
        let definition = &world
            .get::<crate::ship::compartment_damage::ShipDamage>(entity)
            .ok_or("repair claims without physical owner")?
            .definition;
        let recipe = definition
            .config
            .remedies
            .get(
                work.recipe
                    .as_ref()
                    .ok_or("reserved stage missing recipe")?,
            )
            .ok_or("unknown reserved repair recipe")?;
        let services = world.get::<crate::resources::server::ResourceServices>(entity);
        for (index, part) in recipe.requirements.iter().enumerate() {
            let services = services.ok_or("repair parts have no authored resource services")?;
            let ResourceEndpoint::Capacity { id: capacity } = resolve_endpoint(
                &part.resource,
                &services.0,
                &ResourceLink {
                    service: Some(part.service.clone()),
                    provider: None,
                },
            )?
            else {
                return Err("repair parts do not name physical capacity stock".into());
            };
            let claim = format!("{}/{index}", work.claim(*id));
            if expected.insert((capacity, claim), part.amount).is_some() {
                return Err("duplicate repair stage claim".into());
            }
        }
    }
    let actual: std::collections::BTreeMap<_, _> = row
        .infrastructure
        .as_ref()
        .into_iter()
        .flat_map(|inventory| inventory.capacities())
        .flat_map(|capacity| {
            capacity
                .stock
                .reserved
                .iter()
                .filter(|(claim, _)| claim.starts_with("repair/"))
                .map(|(claim, amount)| ((capacity.id.clone(), claim.clone()), *amount))
        })
        .collect();
    if actual != expected {
        return Err("physical repair progress and inventory claims disagree".into());
    }
    Ok(())
}

/// Put ONE captured row back onto one live entity, at the caller's tick.
///
/// Lifted out of [`restore_entities`] unchanged so the GM despawn inverse
/// (issue #1444) restores a single entity through the EXACT writes a resumed
/// save performs. A second per-entity applier would be a second answer to
/// "what is the state of this ship", and the two would drift the first time a
/// continuation was added to only one of them.
///
/// `tick` is the capture's own simulation tick for a resume, and the CURRENT
/// tick for an inverse: nothing here simulates the interval, so the value is
/// simply the reading every age-relative continuation is measured against.
pub(crate) fn apply_entity_state(world: &mut World, entity: Entity, row: &EntityState, tick: u64) {
    if row.probe_custody.is_some() || row.bay_jobs.is_some() {
        bevy::log::error!("refused single-row craft restore: full custody roster is required");
        return;
    }
    if let Err(reason) = super::transit::validate_live(world, entity, row, tick) {
        bevy::log::error!("refused Transit restore for {}: {}", row.uuid, reason);
        return;
    }
    if let Err(reason) = super::helm_movement::validate_live(world, entity, row, tick) {
        bevy::log::error!("refused movement restore for {}: {}", row.uuid, reason);
        return;
    }
    if let Err(reason) = super::activities::validate_single_row(world, row) {
        bevy::log::error!("refused activity job rewind for {}: {}", row.uuid, reason);
        return;
    }
    if let Err(reason) = super::security::validate_live(world, entity, row, tick) {
        bevy::log::error!("refused Security job restore for {}: {}", row.uuid, reason);
        return;
    }
    if let Err(reason) = super::personnel::validate_live(world, entity, row, tick) {
        bevy::log::error!("refused personnel restore for {}: {}", row.uuid, reason);
        return;
    }
    if row.weapon_flight_carrier {
        bevy::log::error!(
            "refused single-row restore: physical flight carriers require full-roster restore"
        );
        return;
    }
    if !super::flight_carriers::unique_single_row(world, entity, row) {
        bevy::log::error!("refused single-row restore: guided flight is already owned elsewhere");
        return;
    }
    if let Err(reason) = validate_entity_repair_jobs(world, entity, row) {
        bevy::log::error!("refused repair job restore for {}: {}", row.uuid, reason);
        return;
    }
    if let Err(reason) = super::guided_source::validate_entity(world, Some(entity), row, tick) {
        bevy::log::error!("refused guided source restore for {}: {}", row.uuid, reason);
        return;
    }
    if let Err(reason) = super::navigation_formation::validate_live(world, entity, row) {
        bevy::log::error!(
            "refused Formation course restore for {}: {}",
            row.uuid,
            reason
        );
        return;
    }
    if let Err(reason) = super::preflight::validate_entity_resources(world, entity, row) {
        bevy::log::error!("refused resource restore for {}: {}", row.uuid, reason);
        return;
    }
    let restored_at = fixed_elapsed_secs(world).unwrap_or_default();
    let restored_alert_tick = bevy::ecs::change_detection::Tick::new(
        world
            .read_change_tick()
            .get()
            .wrapping_sub(bevy::ecs::change_detection::MAX_CHANGE_AGE),
    );
    apply_entity_state_with(world, entity, row, tick, restored_at, restored_alert_tick);
}

/// The per-entity write list itself, with the two restore-wide readings the
/// bulk path takes once and hands to every row.
pub(super) fn apply_entity_state_with(
    world: &mut World,
    entity: Entity,
    row: &EntityState,
    tick: u64,
    _restored_at: f32,
    restored_alert_tick: bevy::ecs::change_detection::Tick,
) {
    if row.weapon_flight_carrier {
        // Remaining banks are captured physical state, not an immutable hull
        // recipe. Replace their shape under the bulk-restore rehome guard.
        crate::console::weapons::flight_carrier::seed_banks(world, entity, &row.carrier_bolt_banks);
    }
    super::fitting::apply(world, entity, row);
    let emp_clock = crate::ship::emp::clock(world, tick).ok();
    let mut entity_mut = world.entity_mut(entity);
    if let Some(saved) = &row.equipment_recovery {
        entity_mut.insert(saved.clone());
    }
    if let (Some(mut provider), Some(state)) = (
        entity_mut.get_mut::<crate::resources::ports_server::PortProviderState>(),
        row.port_provider.as_ref(),
    ) {
        provider.0 = state.clone();
    }
    if let (Some(mut recovery), Some(state)) = (
        entity_mut.get_mut::<crate::resources::recovery_server::ShipRecovery>(),
        row.recovery.as_ref(),
    ) {
        recovery.0 = state.clone();
    }
    if let (Some(mut production), Some(state)) = (
        entity_mut.get_mut::<crate::resources::production_server::ShipProduction>(),
        row.production.as_ref(),
    ) {
        production.0 = state.clone();
    }
    if let (Some(mut logistics), Some(state)) = (
        entity_mut.get_mut::<crate::resources::logistics_server::ShipLogistics>(),
        row.logistics.as_ref(),
    ) {
        logistics.0 = state.clone();
    }
    if let (Some(mut damage), Some(stored)) = (
        entity_mut.get_mut::<crate::ship::compartment_damage::ShipDamage>(),
        row.physical_damage.as_ref(),
    ) {
        damage.state = stored.clone();
        damage.emp_clock = emp_clock;
    }
    if let (Some(mut frontier), Some(stored)) = (
        entity_mut.get_mut::<crate::ship::disablement::DisablementObservation>(),
        row.disablement.as_ref(),
    ) {
        *frontier = stored.clone();
    }
    if let (Some(mut access), Some(states)) = (
        entity_mut.get_mut::<crate::ship::access::InteriorAccess>(),
        row.access.as_ref(),
    ) {
        access.restore(states).expect("preflighted access state");
    }
    // Reconcile the canonical LOD bundle before applying any of the
    // continuations it owns. A High capture restored onto a Low bootstrap
    // needs those components inserted so the writes below have somewhere to
    // land; a Low capture restored onto High must remove them before any
    // high-fidelity system can observe the stale bootstrap marker.
    if let Some(ai_fidelity) = &row.ai_fidelity {
        apply_ai_fidelity(&mut entity_mut, ai_fidelity);
    }
    if let Some(mut sources) = entity_mut.get_mut::<crate::ship_plugin::ShipSystemControlSources>()
    {
        sources.0.replace_offline_systems(
            row.damage_offline_systems
                .iter()
                .cloned()
                .map(crate::core::messages::SystemId),
        );
        sources.0.replace_gm_disabled_systems(
            row.gm_disabled_systems
                .iter()
                .cloned()
                .map(crate::core::messages::SystemId),
        );
    }
    let restored_doctrine = row
        .npc_doctrine
        .as_ref()
        .map(|state| state.doctrine.clone())
        .or_else(|| {
            entity_mut
                .get::<crate::gm::operations::npc::NpcDoctrineState>()
                .and_then(|state| state.0.as_ref().map(|state| state.baseline.clone()))
        });
    if let Some(doctrine) = restored_doctrine {
        if let Some(mut behaviour) =
            entity_mut.get_mut::<crate::entities::spawner::BehaviourSection>()
        {
            behaviour.0.doctrine = doctrine;
        }
    }
    if entity_mut.contains::<crate::gm::operations::npc::NpcDoctrineState>()
        || row.npc_doctrine.is_some()
    {
        entity_mut.insert(crate::gm::operations::npc::NpcDoctrineState(
            row.npc_doctrine.clone(),
        ));
    }
    if let Some(p) = row.physics {
        if let Some(mut physics) = entity_mut.get_mut::<ShipPhysics>() {
            physics.x = p[0];
            physics.y = p[1];
            physics.z = p[2];
            physics.yaw = p[3];
            physics.forward_speed = p[4];
            physics.roll = p[5];
            physics.lateral_speed = p[6];
            physics.vertical_speed = p[7];
        }
        // The renderer and the physics solver both read `Transform`, so a
        // restored ship that only moved its `ShipPhysics` would sit in one
        // place and be drawn in another until the next helm integration.
        //
        // The ROTATION is written for a sharper reason than drawing:
        // `build_helm_ai_surfaces_frame` reads a target's facing straight
        // off `Transform::rotation`, not off its `ShipPhysics`. A resumed
        // world that restored only the translation therefore had every ship
        // steering against a target whose heading was the *bootstrap's* —
        // and a Harrow inbound on a bearing of pi was read as facing 0.
        //
        // Derived here rather than stored, and by the same expression
        // `physics_systems::apply_ship_physics` uses, so the two can only
        // ever agree: the transform is a projection of `ShipPhysics`, and a
        // save that stored it separately could contradict the thing it is a
        // projection of.
        if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
            transform.translation = Vec3::new(p[0], p[1], p[2]);
            transform.rotation = Quat::from_euler(bevy::math::EulerRot::YXZ, -p[3], 0.0, p[5]);
        }
    }
    if let Some(rows) = &row.hull {
        if let Some(mut hull) = entity_mut.get_mut::<EntitySystemHull>() {
            apply_hull(&mut hull.0, rows);
        }
    }
    if let Some(active) = row.red_alert {
        if let Some(mut alert) = entity_mut.get_mut::<ShipRedAlert>() {
            alert.bypass_change_detection().0 = active;
            alert.set_last_changed(restored_alert_tick);
        }
    }
    // Issues #1107–#1109. The per-ship Command stance map IS folded into the
    // sim digest, so a resume that dropped it stood at a different digest
    // than the capture the instant any stance was in force. Overwrite the
    // authoritative map from the row — an empty row clears it, byte-identical
    // to a never-commanded hull. Re-wrapped into `StationId` from the sorted
    // scalar pairs the capture stored.
    if let Some(mut stances) =
        entity_mut.get_mut::<crate::console::command::server::ShipStationStances>()
    {
        stances.0 = row
            .station_stances
            .iter()
            .map(|(station, stance)| {
                (
                    crate::core::messages::StationId(station.clone()),
                    stance.clone(),
                )
            })
            .collect();
    }
    if !row.console_views.is_empty()
        || entity_mut.contains::<crate::ship::components::ConsoleWorkingViews>()
    {
        entity_mut.insert(crate::ship::components::ConsoleWorkingViews(
            row.console_views.clone(),
        ));
    }
    // Reseed the #1108 edge scratch (`LastDirectedControl`) from the restored
    // control sources. It is NOT folded into the digest (it is a pure
    // function of already-authoritative control state, classified `derived`),
    // so this does not move the digest-at-restore assertion — but the scratch
    // is what makes the Human→AI stance-resume trigger fire on an EDGE rather
    // than every tick, and a restored map is empty. An empty scratch treats
    // the first post-restore tick as a first OBSERVATION and fires nothing;
    // a continuous host would have carried `Some(prev)` and could fire the
    // edge that tick. Recording the directed Station's CURRENT
    // `station_is_ai_controlled` result makes the first tick a continuation
    // of the state the resumed world actually restored into, not a spurious
    // first-observation no-op. The captured session-level human/AI split on
    // the target is deliberately NOT recoverable — control sources are
    // derived from who is at a console, which the snapshot excludes — so the
    // honest reseed is the restored world's own current reading. See the
    // WARNING on `command_plugin::LastDirectedControl`.
    let reseed = {
        let config = entity_mut.get::<crate::ship_plugin::ShipConfigComponent>();
        let sources = entity_mut.get::<crate::ship_plugin::ShipSystemControlSources>();
        match (config, sources) {
            (Some(config), Some(sources)) => {
                crate::console::command::server::command_console(&config.0)
                    .and_then(|command| command.command_target.clone())
                    .map(|target| {
                        let now_ai = crate::console::command::server::station_is_ai_controlled(
                            &config.0, &sources.0, &target,
                        );
                        (target, now_ai)
                    })
            }
            _ => None,
        }
    };
    if let Some((target, now_ai)) = reseed {
        if let Some(mut last) =
            entity_mut.get_mut::<crate::console::command::server::LastDirectedControl>()
        {
            last.0.clear();
            last.0.insert(target, now_ai);
        }
    }
    if let Some(control) = &row.control {
        control.restore_into(&mut entity_mut);
    }
    if let Some(drive) = &row.drive {
        if let Some(mut history) = entity_mut.get_mut::<crate::server_app::ImpulseHullHistory>() {
            history.0 = drive.impulse_previous_hull_hp;
        }
        if let Some(mut impulse) = entity_mut.get_mut::<ShipImpulse>() {
            impulse.0.phase = match drive.impulse_phase {
                1 => ImpulsePhase::Charging,
                2 => ImpulsePhase::Active,
                _ => ImpulsePhase::Idle,
            };
            impulse.0.charge_progress = drive.impulse_charge_progress;
        }
        if let Some(mut boost) = entity_mut.get_mut::<ShipBoost>() {
            boost.0.active = drive.boost_active;
            boost.0.battery = drive.boost_battery;
        }
        if let Some(mut actuation) =
            entity_mut
                .get_mut::<phoenix_sim_gameplay::ship::impulse_boost_systems::BoostActuation>()
        {
            actuation.0 = 0.0;
        }
    }
    if let Some(waypoint) = &row.navigation_waypoint {
        apply_navigation_waypoint(&mut entity_mut, waypoint);
    }
    if let Some(clearance_issue) = &row.navigation_clearance_issue {
        apply_navigation_clearance_issue(&mut entity_mut, clearance_issue);
    }
    if let Some(helm_clearance) = &row.helm_waypoint_clearance {
        apply_helm_waypoint_clearance(&mut entity_mut, helm_clearance);
    }
    if let Some(weapons) = &row.weapons {
        apply_weapons(&mut entity_mut, weapons);
    }
    if let Some(shield_ai) = &row.shield_ai {
        apply_shield_ai(&mut entity_mut, shield_ai);
    }
    if let Some(sensors_threat) = &row.sensors_threat {
        apply_sensors_threat(&mut entity_mut, sensors_threat);
    }
    if let Some(intent_narration) = &row.intent_narration {
        apply_intent_narration(&mut entity_mut, intent_narration);
    }
    if let Some(recent_combat) = &row.recent_combat {
        apply_recent_combat(&mut entity_mut, recent_combat);
    }
    if let Some(frequency_hint) = &row.frequency_hint {
        apply_frequency_hint(&mut entity_mut, frequency_hint);
    }
    if let Some(phaser_frequency) = &row.phaser_frequency {
        apply_ship_phaser_frequency(&mut entity_mut, phaser_frequency);
    }
    if let Some(tactical_frequency_hint) = &row.tactical_frequency_hint {
        apply_tactical_frequency_hint(&mut entity_mut, tactical_frequency_hint);
    }
    if let Some(last_system_tiers) = &row.last_system_tiers {
        apply_last_system_tiers(&mut entity_mut, last_system_tiers);
    }
    if let Some(power_brownout) = &row.power_brownout {
        apply_power_brownout(&mut entity_mut, power_brownout);
    }
    if let Some(shields_coordination) = &row.shields_coordination {
        apply_shields_coordination(&mut entity_mut, shields_coordination);
    }
    if let Some(coordination_queue) = &row.coordination_queue {
        apply_coordination_queue(&mut entity_mut, coordination_queue, tick);
    }
    if let Some(repair) = &row.repair {
        apply_repair(&mut entity_mut, repair);
    }
    if let Some(arc) = &row.arc_request {
        apply_arc_request(&mut entity_mut, arc);
    }
    if let Some(power) = &row.power {
        if let Some(mut reactor) = entity_mut.get_mut::<crate::ship::power::ShipPowerSystem>() {
            reactor
                .0
                .restore_continuation(power)
                .expect("energy continuation preflight passed");
        }
    }
    if let Some(surface) = row.pass_surface {
        if let Some(mut pass) = entity_mut.get_mut::<crate::ship::helm_ai::HelmPassSurface>() {
            *pass = surface;
        }
    }
    if let Some(infrastructure) = &row.infrastructure {
        if let Some(mut condition) =
            entity_mut.get_mut::<crate::infrastructure::InfrastructureCondition>()
        {
            condition.0 = infrastructure.clone();
        }
    }
    if let Some(tractor) = &row.tractor {
        if let Some(mut beam) = entity_mut.get_mut::<crate::tractor::TractorBeam>() {
            beam.restore(tractor);
        }
    }
    if let Some(trees) = &row.ai_trees {
        entity_mut.insert(trees.clone());
    }
    if let Some(dock) = &row.dock {
        if let Some(mut control) = entity_mut.get_mut::<crate::dock::DockControl>() {
            control.restore(dock);
        }
    }
    if let Some(umbilical) = &row.umbilical {
        if let Some(mut control) = entity_mut.get_mut::<crate::umbilical::TransferUmbilical>() {
            control.restore(umbilical);
        }
    }
    if let Some(mut teams) = entity_mut.get_mut::<crate::security::ShipSecurityTeams>() {
        if let Some(security) = &row.security {
            teams
                .restore_at(security, tick)
                .expect("preflighted Security commitments");
        } else {
            teams.reset_idle();
        }
    }
    if let Some(scan) = &row.scan {
        if let Some(mut record) = entity_mut.get_mut::<crate::science::ShipScanRecord>() {
            record.restore(scan);
        }
    }
    if let Some(saved) = &row.shield_frequency {
        super::shield_enquiry::apply(&mut entity_mut, saved);
    }
    if let Some(field) = &row.region_field {
        entity_mut.insert(field.clone());
    }
    if let Some(saved) = &row.mounts {
        entity_mut.insert(saved.clone());
    }
    if let Some(saved) = &row.comms_media {
        entity_mut.insert(saved.clone());
    } else {
        entity_mut.remove::<crate::comms::media_control::ShipCommsMediaIntent>();
    }
    if let Some(saved) = &row.identity_emitters {
        entity_mut.insert((
            saved.clone(),
            crate::science::identity::IdentityEmissions::default(),
        ));
    } else {
        entity_mut.remove::<(
            crate::science::identity::ShipIdentityEmitters,
            crate::science::identity::IdentityEmissions,
        )>();
    }
    if let Some(saved) = &row.concealment_emitters {
        entity_mut.insert((
            saved.clone(),
            crate::science::concealment::ConcealmentFields::default(),
        ));
    } else {
        entity_mut.remove::<(
            crate::science::concealment::ShipConcealmentEmitters,
            crate::science::concealment::ConcealmentFields,
        )>();
    }
    if let Some(saved) = &row.signal_protection {
        entity_mut.insert((
            saved.clone(),
            crate::science::protection::ProtectionGrants::default(),
        ));
    } else {
        entity_mut.remove::<(
            crate::science::protection::ShipSignalProtection,
            crate::science::protection::ProtectionGrants,
        )>();
    }
    if let Some(saved) = &row.jamming {
        entity_mut.insert((
            saved.clone(),
            crate::comms::jamming::JammingEmissions::default(),
        ));
    } else {
        entity_mut.remove::<(
            crate::comms::jamming::ShipJamming,
            crate::comms::jamming::JammingEmissions,
        )>();
    }
    if let Some(saved) = &row.illumination {
        entity_mut.insert((
            saved.clone(),
            crate::console::weapons::illumination::IlluminationEmissions::default(),
        ));
    } else {
        entity_mut.remove::<(
            crate::console::weapons::illumination::ShipIllumination,
            crate::console::weapons::illumination::IlluminationEmissions,
        )>();
    }
    if let Some(saved) = &row.guided_source {
        entity_mut
            .insert(crate::console::weapons::guidance_link::ShipGuidanceKnowledge(saved.clone()));
    } else {
        entity_mut.remove::<crate::console::weapons::guidance_link::ShipGuidanceKnowledge>();
    }
    if let Some(saved) = &row.helm_movement {
        entity_mut.insert(saved.clone());
    } else {
        entity_mut.remove::<phoenix_sim_gameplay::ship::helm_control::HelmControlState>();
    }
    if let Some(saved) = &row.bay_jobs {
        entity_mut.insert(saved.clone());
    } else {
        entity_mut.remove::<crate::entities::craft::ShipBayJobs>();
    }
    if let Some(saved) = &row.probe_custody {
        // Derived presence is projected after all canonical row writes.
        entity_mut.insert(saved.clone());
    }
    if let Some(saved) = &row.transit {
        entity_mut.insert((
            saved.clone(),
            crate::console::helm::transit::TransitReadouts::default(),
        ));
    } else {
        entity_mut.remove::<(
            crate::console::helm::transit::ShipTransitDrives,
            crate::console::helm::transit::TransitReadouts,
        )>();
    }
    if let Some(saved) = &row.weapon_targets {
        entity_mut.insert(crate::console::weapons::assignments::ShipWeaponTargets(
            saved.clone(),
        ));
    }
    if let Some(saved) = &row.broad {
        entity_mut.insert(crate::science::broad::BroadTracks(saved.clone()));
    }
    if let Some(saved) = &row.navigation_sharing {
        entity_mut.insert(crate::console::navigation::sharing::NavigationSharing(
            saved.clone(),
        ));
    }
    if let Some(passive) = &row.passive {
        entity_mut.insert(passive.clone());
    }
    if let Some(civilian) = &row.civilian {
        if let Some(mut traffic) = entity_mut.get_mut::<crate::civilian::CivilianTraffic>() {
            traffic.0 = civilian.clone();
        }
    }
    if let Some(debris) = &row.debris {
        if let Some(mut threat) = entity_mut.get_mut::<crate::debris::DebrisThreat>() {
            threat.restore(debris);
        }
        // The drifted position, written beside the latches for the same
        // reason a resumed ship's `Transform` is written beside its
        // `ShipPhysics` above: `DebrisThreat::restore` cannot reach its own
        // entity's other components, and a rock carries no physics record
        // for the transform to be a projection of. The rotation and scale
        // are left alone — `tick_debris_drift` never touches either, so what
        // the respawn put there is still right.
        if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
            transform.translation = Vec3::new(
                debris.translation[0],
                debris.translation[1],
                debris.translation[2],
            );
        }
    }
    if !row.patrol_cursors.is_empty() {
        if let Some(mut cursors) = entity_mut.get_mut::<crate::ai::server::ObjectiveCursors>() {
            cursors.0 = row
                .patrol_cursors
                .iter()
                .map(|(id, index, settled)| {
                    crate::ai::patrol_cursor::PatrolCursor::restored(
                        id.clone(),
                        *index as usize,
                        *settled,
                    )
                })
                .collect();
        }
    }
    if !row.blackboards.is_empty() {
        if let Some(mut boards) = entity_mut.get_mut::<crate::server_app::ShipSystemBlackboards>() {
            boards.0 = row
                .blackboards
                .iter()
                .map(|(id, board)| (SystemId(id.clone()), board.clone()))
                .collect();
        }
    }
}

pub(super) fn apply_navigation_waypoint(
    entity: &mut EntityWorldMut<'_>,
    stored: &NavigationWaypointState,
) {
    let mode = stored.position.map(|[x, z]| {
        stored.source_uuid.as_ref().map_or(
            crate::console::navigation::WaypointMode::Free { x, z },
            |source_uuid| crate::console::navigation::WaypointMode::Anchored {
                source_uuid: source_uuid.clone(),
                last_x: x,
                last_z: z,
            },
        )
    });
    let mode = stored
        .formation
        .as_ref()
        .map(
            |course| crate::console::navigation::WaypointMode::Formation {
                course: course.clone(),
            },
        )
        .or(mode);
    if let Some(mut waypoint) = entity.get_mut::<crate::console::navigation::NavigationWaypoint>() {
        waypoint.replace_continuation(mode, stored.generation);
        waypoint.accepted_requests = stored.accepted_requests.clone();
    } else {
        let mut waypoint = crate::console::navigation::NavigationWaypoint::default();
        waypoint.replace_continuation(mode, stored.generation);
        waypoint.accepted_requests = stored.accepted_requests.clone();
        entity.insert(waypoint);
    }
}

pub(super) fn apply_navigation_clearance_issue(
    entity: &mut EntityWorldMut<'_>,
    stored: &NavigationClearanceIssueState,
) {
    if let Some(mut state) = entity.get_mut::<crate::console::navigation::NavClearanceIssueState>()
    {
        state.replace_continuation(stored.issued_generation, stored.helm_axes_were_ai);
    } else {
        let mut state = crate::console::navigation::NavClearanceIssueState::default();
        state.replace_continuation(stored.issued_generation, stored.helm_axes_were_ai);
        entity.insert(state);
    }
}

pub(super) fn apply_helm_waypoint_clearance(
    entity: &mut EntityWorldMut<'_>,
    stored: &HelmWaypointClearanceState,
) {
    if let Some(mut clearance) = entity.get_mut::<crate::ship_plugin::HelmWaypointClearance>() {
        clearance.0 = stored.generation;
    } else {
        entity.insert(crate::ship_plugin::HelmWaypointClearance(stored.generation));
    }
}

/// Restore the two runtime components that form Shields AI's continuation.
///
/// The payload carries the authoritative record ticks verbatim. Snapshot
/// restore later installs the capture's [`SimTick`], so the next Shields AI
/// decision observes the same recorded ticks against the restored logical tick.
pub(super) fn apply_shield_ai(entity: &mut EntityWorldMut<'_>, stored: &ShieldsAiState) {
    if let Some(mut history) = entity.get_mut::<crate::ship::shields::ShieldsDamageHistory>() {
        history.arcs = stored
            .damage_by_arc
            .iter()
            .map(|records| {
                records
                    .iter()
                    .map(
                        |(recorded_tick, amount)| crate::ship::shields::DamageRecord {
                            recorded_tick: *recorded_tick,
                            amount: *amount,
                        },
                    )
                    .collect()
            })
            .collect();
        history.last_hp = stored.last_hp.clone();
    }
    if let Some(mut pending) = entity.get_mut::<crate::ship::shields::PendingShieldsThreatBearing>()
    {
        pending.0 = stored.pending_threat_bearing_rad;
    }
}

/// Restore exactly the state read by `tick_sensors_threat_warning`'s debounce.
pub(super) fn apply_sensors_threat(
    entity: &mut EntityWorldMut<'_>,
    stored: &SensorsThreatMemoryState,
) {
    if let Some(mut state) = entity.get_mut::<crate::ship::sensors::SensorsThreatState>() {
        state.last_threat_uuid = stored.last_threat_uuid.clone();
        state.last_bearing_rad = stored.last_bearing_rad;
        state.last_label = stored.last_label.clone();
        state.last_distance = stored.last_distance;
    }
}

pub(super) fn apply_intent_narration(
    entity: &mut EntityWorldMut<'_>,
    stored: &IntentNarrationState,
) {
    if let Some(mut narration) =
        entity.get_mut::<crate::ship::intent_narration_systems::ShipIntentNarration>()
    {
        narration.replace_continuation(
            stored
                .last
                .iter()
                .map(|(station, snapshot)| {
                    (
                        crate::core::messages::StationId(station.clone()),
                        crate::ship::intent_narration::IntentSnapshot {
                            target_label: snapshot.target_label.clone(),
                            combat_posture: snapshot.combat_posture,
                            hull_fraction: snapshot.hull_fraction,
                            shield_focus: snapshot.shield_focus.clone(),
                            brownout_groups: snapshot.brownout_groups.clone(),
                            manoeuvre: snapshot.manoeuvre.clone(),
                        },
                    )
                })
                .collect(),
            stored.generation,
        );
    }
}

pub(super) fn apply_recent_combat(
    entity: &mut EntityWorldMut<'_>,
    stored: &RecentCombatActivityState,
) {
    if let Some(mut activity) =
        entity.get_mut::<crate::ship::combat_activity::RecentCombatActivity>()
    {
        activity.replace_continuation(
            stored.last_damage_taken_secs,
            stored.last_hostile_fire_taken_secs,
            stored.last_weapon_fired_secs,
            stored.prev_hull,
        );
    }
}

pub(super) fn apply_frequency_hint(
    entity: &mut EntityWorldMut<'_>,
    stored: &FrequencyHintContinuationState,
) {
    if let Some(mut state) = entity.get_mut::<crate::console_ai::server::ShipFrequencyHintState>() {
        state.replace_continuation(
            stored.current_target.clone(),
            stored.elapsed_secs,
            stored.hint_sent,
        );
    }
}

/// Replace, rather than merge, one entity's AI LOD lifecycle.
///
/// The insert/remove unit is the same canonical bundle `lod_ai_ships` uses.
/// `LodTransitionTimer` is intentionally outside that bundle, so its presence
/// is replaced separately and its canonical logical timestamp copied exactly.
/// A complete High bundle and a present timer are
/// updated in place: removing and reinserting an already-correct bundle would
/// make an unnecessary archetype round-trip, and archetype iteration order is
/// itself determinism-sensitive (see `AiHighFidelity`'s write-guard history).
pub(super) fn apply_ai_fidelity(entity: &mut EntityWorldMut<'_>, stored: &AiFidelityState) {
    use crate::ai::server::{
        ai_high_fidelity_components, AiHighFidelityComponents, LodTransitionTimer,
    };

    if stored.high_fidelity {
        if !crate::ai::server::has_ai_high_fidelity_components(entity) {
            // Inserting the complete tuple replaces every present partial
            // member with its canonical default before the continuation writes
            // below restore the captured values.
            entity.insert(ai_high_fidelity_components());
        }
    } else {
        entity.remove::<AiHighFidelityComponents>();
    }

    match stored.last_transition_secs {
        Some(last_state_change_secs) => {
            if let Some(mut timer) = entity.get_mut::<LodTransitionTimer>() {
                timer.last_state_change_secs = last_state_change_secs;
            } else {
                entity.insert(LodTransitionTimer {
                    last_state_change_secs,
                });
            }
        }
        None => {
            entity.remove::<LodTransitionTimer>();
        }
    }
}

pub(super) fn apply_ship_phaser_frequency(
    entity: &mut EntityWorldMut<'_>,
    stored: &ShipPhaserFrequencyState,
) {
    if let Some(mut frequency) = entity.get_mut::<crate::ship::state::ShipPhaserFrequency>() {
        frequency.replace_continuation(stored.frequency);
    }
}

pub(super) fn apply_tactical_frequency_hint(
    entity: &mut EntityWorldMut<'_>,
    stored: &TacticalFrequencyHintInboxState,
) {
    if let Some(mut pending) =
        entity.get_mut::<crate::ship::components::PendingTacticalFrequencyHint>()
    {
        pending.replace_continuation(stored.frequency);
    }
}

pub(super) fn damage_tier_from_tag(tag: u8) -> crate::ship::damage::DamageTier {
    match tag {
        0 => crate::ship::damage::DamageTier::Operational,
        1 => crate::ship::damage::DamageTier::Damaged,
        2 => crate::ship::damage::DamageTier::Disabled,
        _ => crate::ship::damage::DamageTier::Destroyed,
    }
}

pub(super) fn apply_last_system_tiers(
    entity: &mut EntityWorldMut<'_>,
    stored: &LastSystemTiersState,
) {
    if let Some(mut state) = entity.get_mut::<crate::ship::components::LastSystemTiers>() {
        state.replace_continuation(
            stored
                .tiers
                .iter()
                .map(|(system, tier)| (SystemId(system.clone()), damage_tier_from_tag(*tier)))
                .collect(),
            stored
                .hp
                .iter()
                .map(|(system, hp)| (SystemId(system.clone()), *hp))
                .collect(),
        );
    }
}

pub(super) fn apply_power_brownout(
    entity: &mut EntityWorldMut<'_>,
    stored: &PowerBrownoutContinuationState,
) {
    if let Some(mut state) = entity.get_mut::<crate::ship::power::PowerBrownoutState>() {
        state.replace_continuation(
            stored.notified_groups.iter().cloned().collect(),
            stored.locked_changed,
        );
    }
}

pub(super) fn apply_shields_coordination(
    entity: &mut EntityWorldMut<'_>,
    stored: &ShieldsCoordinationContinuationState,
) {
    if let Some(mut state) = entity.get_mut::<crate::ship::shields::ShieldsCoordinationState>() {
        state.replace_continuation(
            stored.down_notified.clone(),
            stored.restore_notified.clone(),
        );
    }
}

/// Replace the fresh bootstrap's in-flight messages with the capture's queue.
///
/// Queue order is the delivery tie-break, so rebuilding iterates the stored
/// vector verbatim. `restore_entities` runs before `restore_run_scope` installs
/// the captured [`SimTick`], so due ticks are rebuilt from the snapshot's tick
/// passed explicitly rather than from the fresh bootstrap resource.
pub(super) fn apply_coordination_queue(
    entity: &mut EntityWorldMut<'_>,
    stored: &CoordinationQueueState,
    snapshot_tick: u64,
) {
    use crate::ship::control_source::ControlSource;
    use crate::ship::coordination::QueuedCoordination;

    if let Some(mut queue) = entity.get_mut::<crate::ship::components::CoordinationQueue>() {
        queue.0.replace(
            stored
                .pending
                .iter()
                .map(|message| QueuedCoordination {
                    sender_origin: match message.sender_origin {
                        0 => ControlSource::Human,
                        1 => ControlSource::Ai,
                        3 => ControlSource::Simplified,
                        _ => ControlSource::Offline,
                    },
                    address: message.address.clone(),
                    payload: message.payload.clone(),
                    presentation: message.presentation.clone(),
                    sender_label: message.sender_label.clone(),
                    due_tick: snapshot_tick
                        .saturating_add(message.due_after_fixed_ticks.saturating_sub(1)),
                })
                .collect(),
        );
    }
}

/// Put a ship's weapon state machines back mid-cycle.
///
/// Every write here is a wholesale replacement rather than a merge, and that is
/// the point: a fresh app's bootstrap ran its own tubes and its own beams on
/// the way to the restore point, and merging would leave the resumed ship
/// carrying a shot the capture never fired.
pub(super) fn apply_weapons(entity: &mut EntityWorldMut<'_>, stored: &WeaponState) {
    if let Some(mut beam) = entity.get_mut::<ActiveBeam>() {
        beam.restore_live_banks(stored.beams.iter().map(
            |(bank, target, remaining, accumulator, pending_cooldown, operation)| {
                (
                    bank.clone(),
                    ActiveBeamSlot {
                        operation: *operation,
                        impact_sample_target: stored
                            .beam_impact_samples
                            .get(bank)
                            .map(|(target, _)| target.clone()),
                        impact_sample: stored
                            .beam_impact_samples
                            .get(bank)
                            .map(|(_, sample)| sample.clone()),
                        strike_damage_bonus: stored
                            .beam_strike_bonuses
                            .get(bank)
                            .copied()
                            .unwrap_or(0.0),
                        target_uuid: target.clone(),
                        remaining_secs: *remaining,
                        damage_accumulator: *accumulator,
                        pending_cooldown_secs: *pending_cooldown,
                    },
                )
            },
        ));
    }
    if let Some(mut cooldown) = entity.get_mut::<PhaserCooldown>() {
        cooldown.restore_banks(stored.phaser_cooldowns.iter().cloned());
    }
    if let Some(mut arcs) = entity.get_mut::<EntityShipArcHull>() {
        for (id, current, _max) in &stored.arc_hull {
            arcs.0.set_hp(id, *current);
        }
    }
    if let Some(mut shields) = entity.get_mut::<crate::ship::shields::ShipShields>() {
        // Charge, not structural hull (that is `arc_hull` above): overwrite the
        // per-facing hp / fractional accumulator / offline timer / focus, then
        // let the system re-derive its focus-dependent fields so the restored
        // system is field-identical to the captured one (issue #997 follow-up).
        // Verified directly off `ShipShields` in `snapshot_resume`, since the
        // digest deliberately defers shield charge from the fold. An arc the
        // save omits is left as the bootstrap built it.
        if !stored.shield_charge.is_empty() {
            shields.0.restore_facings(&stored.shield_charge);
        }
        shields
            .0
            .restore_operations(&stored.shield_operations)
            .expect("shield operation continuation passed preflight");
    }
    if let Some(mut torpedoes) = entity.get_mut::<TorpedoSystemResource>() {
        let system = &mut torpedoes.0;
        apply_torpedo_inventory(system, stored);
        system.in_flight = stored
            .torpedoes_in_flight
            .iter()
            .map(|t| Torpedo {
                previous_position: None,
                collision_interval_secs: None,
                last_guidance: t.last_guidance.clone(),
                profile: t.profile,
                durability_remaining: t.durability_remaining,
                strike_damage_bonus: t.strike_damage_bonus,
                uuid: t.uuid.clone(),
                x: t.position[0],
                y: t.position[1],
                z: t.position[2],
                heading: t.heading,
                pitch: t.pitch,
                lifespan_remaining: t.lifespan_remaining,
                target_uuid: t.target_uuid.clone(),
                source_uuid: t.source_uuid.clone(),
                tube_id: t.tube_id.clone(),
            })
            .collect();
    }
    if let Some(mut blasters) =
        entity.get_mut::<crate::console::weapons::blaster::BlasterSystemResource>()
    {
        // Joined by authored bank order, the order the component keeps and the
        // capture walked. A bank the save does not reach (fewer stored than the
        // hull carries) is left alone, `apply_hull`'s rule: an unmentioned bank
        // is a save written against a different hull, which the content digest
        // refuses.
        for (bank, row) in blasters.0.iter_mut().zip(stored.blasters.iter()) {
            let v = &mut bank.volley;
            v.pending_volley = row.pending_volley;
            v.schedule = row.schedule.clone();
            v.next_step = row.next_step as usize;
            v.volley_elapsed = row.volley_elapsed;
            v.active_barrels = row.active_barrels.clone();
            v.current_step = row.current_step;
            v.on_cooldown = row.on_cooldown;
            v.cooldown_remaining = row.cooldown_remaining;
            v.charging = row.charging;
            v.charge_elapsed = row.charge_elapsed;
            bank.in_flight = row
                .in_flight
                .iter()
                .map(|p| crate::weapons::blaster::BlasterProjectile {
                    impact: p.impact,
                    id: p.id.clone(),
                    x: p.x,
                    y: p.y,
                    pitch: p.pitch,
                    visual_scale: p.visual_scale,
                    // Snapshot boundary is settled after Damage; no pending interval.
                    previous_position: None,
                    collision_interval_secs: None,
                    z: p.z,
                    heading: p.heading,
                    speed: p.speed,
                    lifespan_remaining: p.lifespan_remaining,
                    collision_radius: p.collision_radius,
                    damage: p.damage,
                    shield_pierce: p.shield_pierce,
                    source_uuid: p.source_uuid.clone(),
                })
                .collect();
        }
    }
}

/// Put a ship's repair crew back where it was standing.
pub(super) fn apply_repair(entity: &mut EntityWorldMut<'_>, stored: &RepairState) {
    if let Some(mut teams) = entity.get_mut::<ShipRepairTeams>() {
        if let Some(jobs) = &stored.jobs {
            teams
                .0
                .restore_state(jobs)
                .expect("repair job state prevalidated against its authored owner");
        }
    }
    if let Some(mut queue) = entity.get_mut::<RepairRequestQueue>() {
        queue.recoveries = stored.recoveries.clone();
        queue.personnel_destinations = stored.personnel_destinations.clone();
        queue.entries = stored
            .queue
            .iter()
            .map(
                |(group, station_label, tier, deficit, observations)| RepairQueueEntry {
                    readings: stored
                        .readings
                        .iter()
                        .find(|(key, _)| key == group)
                        .map(|(_, rows)| rows.clone())
                        .unwrap_or_default(),
                    observations: observations.clone(),
                    group: group.clone(),
                    station_label: station_label.clone(),
                    tier: *tier,
                    deficit: *deficit,
                },
            )
            .collect();
    }
    if let Some(mut alerted) = entity.get_mut::<RepairHumanAlerted>() {
        alerted.0 = stored.alerted.iter().cloned().collect();
    }
}

/// Put the Weapons→Helm arc-bearing seam back — see [`ArcRequestState`].
///
/// Both halves are wholesale replacements, [`apply_weapons`]' rule: a settled
/// debounce is restored so the first cadence tick after the restore does not
/// re-fire a request the capture had already spent, and the pending bearing is
/// restored so Helm resumes folding the same bias it was folding at the capture
/// rather than steering as though no request were outstanding.
pub(super) fn apply_arc_request(entity: &mut EntityWorldMut<'_>, stored: &ArcRequestState) {
    if let Some(mut weapons_state) =
        entity.get_mut::<crate::console::weapons::WeaponsArcRequestState>()
    {
        weapons_state.last = stored
            .last
            .as_ref()
            .map(|(family, target, arcs)| (*family, target.clone(), arcs.clone()));
    }
    if let Some(mut pending) = entity.get_mut::<crate::ship_plugin::PendingArcBearingRequest>() {
        pending.target = stored
            .pending_target
            .as_deref()
            .and_then(|t| uuid::Uuid::parse_str(t).ok());
        pending.arcs = stored.pending_arcs.clone();
    }
}

/// Write captured per-system HP onto a hull the fresh world built from config.
///
/// `set_hp` rather than replacing the whole `SystemHull`: the tier
/// thresholds, display names and insertion order are authored config, and the
/// bootstrapped hull already has them right. A system the capture does not
/// mention is left alone rather than zeroed — an unmentioned system is a save
/// written against a different hull, which the content digest is what refuses.
pub(super) fn apply_hull(hull: &mut crate::ship::damage::SystemHull, rows: &[(String, f32, f32)]) {
    for (id, current, _max) in rows {
        hull.set_hp(&SystemId(id.clone()), *current);
    }
}

/// Explicit mesh continuation only. Do not call from ordinary save restore:
/// a standalone resumed game intentionally takes its newly connected crew.
/// The candidate need not have FleetRoster/FleetLockstep until join Commit.
pub(crate) fn restore_mesh_crew(world: &mut World, snapshot: &PhoenixSnapshot) {
    use crate::core::messages::{StationId, SystemId};
    use crate::ship::control_source::ControlSource;
    use crate::ship_plugin::{ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources};
    let mut query = world.query::<(
        Entity,
        &EntityUuid,
        &ShipConfigComponent,
        &mut ActiveStationRatings,
        &mut ShipSystemControlSources,
        Option<&mut crate::console::command::server::LastDirectedControl>,
        Option<&mut crate::lobby::crew_replication::ShipCrewPresence>,
    )>();
    let mut missing_presence = Vec::new();
    for (entity, uuid, config, mut ratings, mut sources, last, presence) in query.iter_mut(world) {
        let Some(crew) = snapshot
            .entities
            .iter()
            .find(|row| row.uuid == uuid.0)
            .and_then(|row| row.mesh_crew.as_ref())
        else {
            continue;
        };
        let captured_ratings: std::collections::HashMap<_, _> = crew
            .ratings
            .iter()
            .map(|(station, rating)| (StationId(station.clone()), rating.clone()))
            .collect();
        // seed_boot_ratings invokes the same apply_rating used by admitted
        // AssignStationRating, and also restores ownerless ai_only defaults.
        let (mut restored, _) = crate::ship::rating::seed_boot_ratings(&config.0, |station| {
            captured_ratings
                .get(&station.id)
                .cloned()
                .unwrap_or_else(|| crate::ship::rating::BACKFILL_RATING.to_string())
        });
        for (system, source) in &crew.sources {
            restored.set(
                SystemId(system.clone()),
                match source {
                    MeshControlSource::Human => ControlSource::Human,
                    MeshControlSource::Ai => ControlSource::Ai,
                    MeshControlSource::Offline => ControlSource::Offline,
                    MeshControlSource::Simplified => ControlSource::Simplified,
                },
            );
        }
        restored.replace_offline_systems(sources.0.offline_entries().cloned());
        restored.replace_gm_disabled_systems(sources.0.gm_disabled_entries().cloned());
        ratings.0 = captured_ratings;
        sources.0 = restored;
        let restored_presence = crate::lobby::crew_replication::ShipCrewPresence(
            crew.occupied.iter().cloned().map(StationId).collect(),
        );
        if let Some(mut presence) = presence {
            *presence = restored_presence;
        } else {
            missing_presence.push((entity, restored_presence));
        }
        // Ordinary restore seeded this edge detector against bootstrap crew.
        // Reseed after the mesh authority frontier replaces that policy.
        if let Some(mut last) = last {
            last.0.clear();
            if let Some(target) = crate::console::command::server::command_console(&config.0)
                .and_then(|command| command.command_target.clone())
            {
                let ai = crate::console::command::server::station_is_ai_controlled(
                    &config.0, &sources.0, &target,
                );
                last.0.insert(target, ai);
            }
        }
    }
    for (entity, presence) in missing_presence {
        world.entity_mut(entity).insert(presence);
    }
}

/// Shared by actual restore and its non-mutating preflight. Committed burst
/// rounds belong to ammunition inventory even though they have left the tube.
pub(super) fn apply_torpedo_inventory(
    system: &mut crate::weapons::torpedo::TorpedoSystem,
    stored: &WeaponState,
) {
    system.burst_states = stored
        .bursts
        .iter()
        .map(|b| TubeBurstState {
            launch_uuid: b.launch_uuid.clone(),
            profile: b.profile,
            burst_interval_secs: b.burst_interval_secs,
            tube_id: b.tube_id.clone(),
            pending: b.pending,
            timer: b.timer,
            launch_x: b.launch[0],
            launch_y: b.launch[1],
            launch_z: b.launch[2],
            launch_heading: b.launch_heading,
            target_uuid: b.target_uuid.clone(),
            source_uuid: b.source_uuid.clone(),
            barrel_origins: b
                .barrel_origins
                .iter()
                .map(|o| (o[0], o[1], o[2]))
                .collect(),
            barrel_sequence: b.barrel_sequence.clone(),
            next_shot_index: b.next_shot_index,
        })
        .collect();
    if let Some(magazine) = &stored.magazine {
        system.magazine = magazine.clone();
    }
    for tube in system.tubes.iter_mut() {
        let Some(row) = stored.tubes.iter().find(|t| t.id == tube.id) else {
            // A tube the save does not mention is left alone rather than
            // emptied, for `apply_hull`'s reason: an unmentioned tube is a
            // save written against a different hull, and the content digest
            // is what refuses that.
            continue;
        };
        tube.load_state = match row.load_phase {
            1 => TubeLoadState::Loading {
                remaining: row.load_timer[0],
                total: row.load_timer[1],
            },
            2 => TubeLoadState::Loaded,
            3 => TubeLoadState::Unloading {
                remaining: row.load_timer[0],
                total: row.load_timer[1],
            },
            // Including anything this build does not recognise — see
            // `TubeState::load_phase`.
            _ => TubeLoadState::Unloaded,
        };
        tube.loaded_count = row.loaded_count;
        tube.target_count = row.target_count;
        tube.active_barrels = row.active_barrels.clone();
        tube.pattern_step = row.pattern_step;
    }
}
