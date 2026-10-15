//! Triggers.
use super::*;

/// Observer: bridge `RegionEntered` (player ship boundary crossing into a
/// region) into a queued `WorldEvent::EnteredRegion` so `collect_world_events`
/// can buffer it for the trigger pipeline on the next tick.
///
/// Looks up the region entity's UUID via `RegionMembership.region_uuids`
/// (populated each tick by `update_region_membership`, and persisted after
/// the entity despawns). Drops the event silently if no UUID is cached
/// (e.g. a region entity spawned without an `EntityUuid` component — not
/// expected in production paths but possible in narrow unit tests).
///
/// Single-fire-per-transition is provided by the region containment
/// system itself: `update_region_membership` uses set differences between
/// the previous and current "inside" sets, so it only triggers the
/// observer event once per boundary crossing. Staying inside on the next
/// tick produces no further `RegionEntered` events.
///
/// After PRD #597 PR 9, `update_region_membership` tracks region membership
/// for every ship (player + NPCs). World-scenario triggers remain
/// player-driven, but every fleet player ship must be observed on every peer:
/// a remote escort crossing the convoy recovery site is the same world event
/// for all lockstep hosts. NPC crossings remain excluded.
pub(super) fn handle_region_entered_event(
    trigger: On<crate::regions::server::RegionEntered>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
    player_ship_q: Query<
        (),
        Or<(
            With<crate::server_app::LocalShip>,
            With<crate::lockstep::FleetSlotOf>,
        )>,
    >,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    // Every peer simulates the frozen fleet. LocalShip alone would make a
    // remote escort's authored region entry disappear from this peer's story.
    if player_ship_q.get(ev.subject).is_err() {
        return;
    }
    let Some(uuid) = membership.region_uuids.get(&ev.region_entity).cloned() else {
        return;
    };
    runtime
        .pending_world_events
        .push(WorldEvent::EnteredRegion { uuid });
}

/// Observer: mirror of `handle_region_entered_event` for region exits.
/// Fires both on boundary-crossing exits and on implicit exits when the
/// region entity is despawned while the ship is inside. Filters on the same
/// fleet-player set as entry so all peers observe the same authored events.
pub(super) fn handle_region_exited_event(
    trigger: On<crate::regions::server::RegionExited>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
    player_ship_q: Query<
        (),
        Or<(
            With<crate::server_app::LocalShip>,
            With<crate::lockstep::FleetSlotOf>,
        )>,
    >,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    if player_ship_q.get(ev.subject).is_err() {
        return;
    }
    let Some(uuid) = membership.region_uuids.get(&ev.region_entity).cloned() else {
        return;
    };
    runtime
        .pending_world_events
        .push(WorldEvent::ExitedRegion { uuid });
}

// -- AI-event trigger system -------------------------------------------------

/// Collect this tick's externally-sourced `WorldEvent`s into `WorldEventBuffer`.
///
/// Three sources, in order:
/// 1. AI-plugin messages (`AiEntityAttacked` / `AiEntityDestroyed` /
///    `AiWaypointReached`) bridged into `WorldEvent`s via `AiEventReaders`.
/// 2. `runtime.pending_world_events` (`WorldLoaded`, `EnteredRegion`, ...
///    queued by `init_world_runtime`, `apply_world_layer_changes`, the region
///    observers, and `tick_delayed_actions` on the previous tick).
/// 3. (#475) A synthesised `TimerElapsed` event once the mission clock is
///    anchored. `on_timer` triggers fire when `elapsed_secs >= after_secs`,
///    measured from `mission_clock` - which [`anchor_mission_clock`]
///    stamps on the first simulation tick of `GamePhase::InProgress`, one place
///    earlier in this same chain. So `after_secs = 0` fires on that first
///    mission tick, and `after_secs = 300` fires 300s into the MISSION however
///    long the lobby was up beforehand (#960 - until then the anchor was taken
///    at `Startup` and a 90-second lobby retired the 0/45/90 triggers in one
///    batch). Single-shot semantics on `TriggerState.fired` prevent re-firing.
///    `Time` is optional so test apps without `TimePlugin` continue to work
///    (they just never see `TimerElapsed`).
///
/// Ordering: chained before `tick_trigger_pipeline`, which consumes the buffer
/// for trigger evaluation. It also ran after `tick_pending_follow_ups` and
/// before `inject_comms_templates` — the comms halves of the #718/#719 chain,
/// both deleted with the `[[comms]]` front-end in issue #985.
///
/// Change detection: `runtime` is only mutably dereferenced when
/// `pending_world_events` has entries to drain, and the buffer is only
/// mutably dereferenced when its contents change, so an event-free tick
/// (minimal test apps without `TimePlugin`) marks neither resource changed.
pub(crate) fn collect_world_events(
    mut ai_events: AiEventReaders,
    mut runtime: ResMut<WorldContentRuntime>,
    mut buffer: ResMut<WorldEventBuffer>,
    time: Option<Res<bevy::time::Time>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    base_world: Option<Res<crate::world::config::WorldConfig>>,
    fixed: Option<Res<Time<Fixed>>>,
    planning_actors: Query<(
        &crate::entities::spawner::EntityUuid,
        Option<&crate::ship_slots::AuthoredShipSlotId>,
        &crate::ship_plugin::ShipConfigComponent,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        Option<&crate::console::repair::server::ShipRepairTeams>,
        Option<&crate::resources::logistics_server::CompiledShipLogistics>,
        Option<&crate::ship_plugin::ShipSystemControlSources>,
        Option<&crate::ship_plugin::ActiveStationRatings>,
        Option<&crate::lobby::crew_replication::ShipCrewPresence>,
        Option<&crate::ship_plugin::HumanSeekingHosts>,
    )>,
    hulls: Query<(
        &crate::entities::spawner::EntityUuid,
        Option<&crate::entities::spawner::EntitySystemHull>,
        Option<&crate::ship::compartment_damage::ShipDamage>,
    )>,
) {
    if let Some(tick) = tick {
        crate::world::activities::acknowledge_pending(&mut runtime, tick.0);
        if let Some(base) = base_world.as_deref() {
            let mut actors = planning_actors.iter().collect::<Vec<_>>();
            actors.sort_by(|left, right| left.0 .0.cmp(&right.0 .0));
            for (uuid, slot, config, damage, teams, logistics, sources, ratings, presence, hosts) in
                actors
            {
                let forecast = sources
                    .zip(ratings)
                    .zip(presence)
                    .zip(fixed.as_deref())
                    .and_then(|(((sources, ratings), presence), fixed)| {
                        Some(crate::world::planning::ForecastSources {
                            config,
                            sources,
                            ratings,
                            presence,
                            hosts,
                            tick_nanos: u64::try_from(fixed.timestep().as_nanos()).ok()?,
                        })
                    });
                let result = crate::world::planning::advance_with_forecast(
                    &mut runtime,
                    base,
                    &uuid.0,
                    slot.map(|id| id.0.as_str()),
                    tick.0,
                    forecast.as_ref(),
                    |request| match request {
                        phoenix_sim_world::world::activities::ActivityRequest::Repair {
                            ..
                        } => damage.zip(teams).is_some_and(|(damage, teams)| {
                            crate::world::activities::repair_binding_supported(
                                request,
                                &config.0,
                                damage,
                                &crate::ship::system_registry::repair_system_id(),
                                teams.0.slots().len(),
                            )
                        }),
                        phoenix_sim_world::world::activities::ActivityRequest::Delivery {
                            ..
                        } => logistics.is_some_and(|profile| {
                            crate::world::activities::delivery_binding_supported(
                                request, &profile.0,
                            )
                        }),
                    },
                );
                if let Err(error) = result {
                    bevy::log::warn!(actor=%uuid.0,%error,"Maintenance plan release withheld");
                }
            }
        }
    }
    let mut world_events: Vec<WorldEvent> = Vec::new();
    for ev in ai_events.attacked.read() {
        world_events.push(WorldEvent::Attacked {
            uuid: ev.entity_uuid.clone(),
            attacker_uuid: ev.attacker_uuid.to_string(),
        });
    }
    for ev in ai_events.destroyed.read() {
        world_events.push(WorldEvent::Destroyed {
            uuid: ev.entity_uuid.clone(),
        });
    }
    for ev in ai_events.disabled.read() {
        world_events.push(WorldEvent::Disabled {
            uuid: ev.entity_uuid.clone(),
            disabled: ev.disabled,
        });
    }
    // `advance_objective_cursors` writes these in `SimSet::Modifiers`, i.e.
    // after this system has already run for the tick, so an arrival is
    // observed here on the following tick.
    for ev in ai_events.waypoint_reached.read() {
        world_events.push(WorldEvent::WaypointReached {
            uuid: ev.entity_uuid.clone(),
            waypoint: ev.waypoint.clone(),
        });
    }
    let mut live_hulls = HashSet::new();
    for (uuid, hull, damage) in &hulls {
        let Some(current_fraction) =
            crate::ship::compartment_damage::structural_fraction(damage, hull)
        else {
            continue;
        };
        live_hulls.insert(uuid.0.clone());
        if let Some(previous_fraction) = runtime
            .observed_hull_fractions
            .insert(uuid.0.clone(), current_fraction)
        {
            if current_fraction < previous_fraction {
                world_events.push(WorldEvent::HullDroppedBelow {
                    uuid: uuid.0.clone(),
                    previous_fraction,
                    current_fraction,
                });
            }
        }
    }
    // Avoid a no-op mutable dereference on an empty cache: in a minimal app
    // with no hulls, this keeps an otherwise event-free tick unchanged.
    if !live_hulls.is_empty() || !runtime.observed_hull_fractions.is_empty() {
        runtime
            .observed_hull_fractions
            .retain(|uuid, _| live_hulls.contains(uuid));
    }
    // Drain any externally-queued world events (e.g. WorldLoaded pushed by
    // init_world_runtime or apply_world_layer_changes). The emptiness check
    // is a read: it keeps event-free ticks from marking WorldContentRuntime
    // changed via a no-op DerefMut.
    if !runtime.pending_world_events.is_empty() {
        for event in std::mem::take(&mut runtime.pending_world_events) {
            if crate::world::activities::is_activity(&event) {
                runtime.pending_world_events.push(event);
            } else {
                world_events.push(event);
            }
        }
    }
    let elapsed_secs = time.as_ref().and_then(|t| {
        runtime
            .mission_clock
            .map(|clock| clock.elapsed_secs(t.elapsed()))
    });
    if let Some(es) = elapsed_secs {
        world_events.push(WorldEvent::TimerElapsed { elapsed_secs: es });
    }
    // Leave the buffer holding exactly THIS tick's events. Skip the mutable
    // deref when both the old and new contents are empty — replacing an
    // empty Vec with an empty Vec is a no-op that would otherwise mark the
    // buffer changed every tick.
    if world_events.is_empty() && buffer.0.is_empty() {
        return;
    }
    buffer.0 = world_events;
}

/// Read this tick's externally-sourced `WorldEvent`s from `WorldEventBuffer`
/// (filled by `collect_world_events` earlier in the Physics chain), evaluate
/// the scenario trigger table, and execute the resulting actions (including
/// `SetAiState`, `ApplyModifier`, `RemoveModifier`, `ApplyFlag`, and
/// `RemoveFlag`).
pub(crate) fn tick_trigger_pipeline(
    mut runtime: ResMut<WorldContentRuntime>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    buffer: Res<WorldEventBuffer>,
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::server_app::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>,
    mut faction_dispatch: FactionDispatchParams,
    time: Option<Res<bevy::time::Time>>,
    id_mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Entity as usize }>,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    // The scripting seam (issue #984, Rhai M6 phase 2a). Both `Option`, so a
    // script-free world (no `WorldScriptRuntime`) and every bare-`App` fixture
    // take the `None` arm and the scripted-handler branch below is skipped
    // entirely — behaviour there is byte-identical to before scripting existed.
    mut script: ScriptRuntimeParams,
    // The per-owner effect queues a fired trigger's scripted handler pushes onto
    // (issue #1223).
    mut effect_queues: EffectQueues,
) {
    let empty_anchors: HashMap<String, [f32; 3]> = HashMap::new();
    // Seeded UUID source for `SpawnEntity` dispatch. Bound once per system run
    // because `DispatchContext::uuid_source` is a `&dyn Fn`.
    let uuid_source = || {
        crate::world_id::mint_live_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity)
    };
    // Template source for `SpawnEntity` dispatch (issue #715), built once per
    // system run. `WasmTemplateLoader` unconditionally: it serves the
    // preloaded config cache first and, on native, falls back to the
    // filesystem — reproducing the old cfg-split inline block on both targets.
    let template_loader = crate::entities::loader::WasmTemplateLoader;
    if buffer.0.is_empty()
        && runtime.situations.ready(&runtime.situation_definitions).is_empty()
        && runtime.pending_delayed_actions.is_empty()
        && runtime.pending_gm_event_fires.is_empty()
        && runtime.pending_gm_spawns.is_empty()
        && runtime.pending_gm_despawns.is_empty()
        // An armed despawn inverse (issue #1444) is work this pipeline owes
        // even on a tick with no world events at all: a GM undoing a removal
        // during a lull is exactly the case, and returning here would leave the
        // arm sitting in the runtime while the journal already reports Applied.
        // The ARMS only — a run that merely RETAINS captures has no work, and
        // testing the whole store would wake this pipeline every tick after the
        // first GM removal of the session.
        && !runtime.gm_despawn_captures.has_armed_restores()
    {
        return;
    }

    // (#475/#718) `elapsed_secs` anchors delayed-action scheduling below
    // (`fire_at_elapsed = elapsed_secs + delay`). Recomputed from `Time`
    // rather than derived from the buffer's `TimerElapsed` event because the
    // delay check distinguishes `None` (no `Time` resource or no world-load
    // anchor — delayed actions are silently dropped) from `Some`, and tests
    // push `TimerElapsed` events into `pending_world_events` without a
    // world-load anchor; deriving from the buffer would flip `None` to
    // `Some` there. `Time` is updated once per frame, so this reads the same
    // value `collect_world_events` used earlier in the chain.
    let elapsed_secs = time.as_ref().and_then(|t| {
        runtime
            .mission_clock
            .map(|clock| clock.elapsed_secs(t.elapsed()))
    });

    // Scripting seam (issue #984, Rhai M6 phase 2a): the clock a scripted
    // handler's deferred work is stamped against, and the shared per-tick budget
    // reset. `now_tick`/`tick_hz` matter only for `after(..)` callbacks (deferred
    // to 2b); `elapsed_secs` is what a scripted `in_seconds(..)` delayed effect
    // stamps against, mirroring the declarative `action_delays` path below.
    let now_tick = script.sim_tick.as_ref().map(|t| t.0).unwrap_or(0);
    let script_clock = SchedClock {
        tick: now_tick,
        elapsed_secs: elapsed_secs.unwrap_or(0.0),
        tick_hz: world_layers
            .base_world_config
            .as_ref()
            .map(|wc| wc.global.sim_tick_hz)
            .unwrap_or(SchedClock::ZERO.tick_hz),
    };
    // Reset the shared budget once per tick (`SimTick`-keyed), so it spans every
    // chaining pass this tick exactly as the M0 spike's aggregate caps require.
    if let Some(sr) = script.runtime.as_deref_mut() {
        sr.prepare_invocation_tick(now_tick);
    }

    // Reborrow the `ResMut` as a plain `&mut` so the evaluation loop below can
    // split disjoint field borrows (`&runtime.flags` for condition chains while
    // `&mut runtime.triggers` evaluates the ordered table) — a smart
    // pointer cannot split, a plain reference can. Placed after the early
    // return so change detection still only marks the resource on ticks that
    // actually process events (the pre-existing behaviour: every path past
    // this point mutated through the `ResMut` anyway).
    let runtime = &mut *runtime;

    let name_to_uuid = runtime.name_to_uuid.clone();

    // Build UUID → ECS Entity map once per tick so the six per-entity
    // modifier/flag arms below can resolve their `entity` target in O(1)
    // instead of scanning `entity_uuid_query` each time. Used by
    // `ApplyModifier` / `RemoveModifier` / `ApplyFlag` / `RemoveFlag` /
    // `ApplyIntModifier` / `RemoveIntModifier` to write to the target
    // entity's per-entity `ShipModifiers` Component.
    let uuid_to_entity: std::collections::HashMap<String, Entity> = entity_uuid_query
        .iter()
        .map(|(ent, uuid_comp)| (uuid_comp.0.clone(), ent))
        .collect();

    // Loop to support within-tick chaining: a trigger that fires a
    // `set_flag` action emits a `FlagSet` event which a downstream
    // `on_flag_set` trigger can react to in the same Bevy frame. Bounded
    // for safety against pathological feedback loops.
    //
    // PRD #397 fix 1: each trigger is evaluated with its OWN flag chain
    // and layer chain, computed from its `origin_layer` by walking
    // `loader_path` pointers up via `WorldLayerMap` until reaching the
    // base world (whose store is `runtime.flags`). Trigger ordering within
    // a pass is deterministic because each pass is two-phase: ALL
    // conditions are evaluated (reading the live stores, which nothing
    // mutates during evaluation) before ANY fired action is dispatched, so
    // later triggers in the same pass see the same flag values as earlier
    // ones; their mutations land in `next_events` and are observed on the
    // next pass.
    // Seed the chain from the buffer's contents. The buffer stays borrowed
    // (`collect_world_events` owns refilling it next tick), so clone rather
    // than move — one Vec clone per non-empty tick, the same cost the
    // pre-#716 local `world_events.clone()` paid.
    let mut current_events = buffer.0.clone();

    // Accepted despawn inverses (issue #1444). Executed BEFORE this tick's
    // removals so a restore and a removal armed in the same `PreUpdate` cannot
    // race for one identity: the reducer already refused a restore whose uuid
    // was live, and running the rebuild first means a removal armed against the
    // rebuilt entity finds it, rather than silently doing nothing.
    if runtime.gm_despawn_captures.has_armed_restores() {
        commands.queue(|world: &mut World| {
            crate::gm::operations::despawn_undo::apply_armed_restores(world);
        });
    }

    // Safe removals use the same command and Destroyed cascade as authored removal.
    // Keep name/group history: on_destroyed/on_all_destroyed resolve against it.
    for uuid in std::mem::take(&mut runtime.pending_gm_despawns) {
        if !uuid_to_entity.contains_key(&uuid) {
            continue;
        }
        let result = crate::world::dispatch::DispatchResult {
            commands: vec![ActionCmd::DestroyEntity { uuid: uuid.clone() }],
            new_events: vec![WorldEvent::Destroyed { uuid }],
            ..Default::default()
        };
        apply_dispatch_result(
            result,
            "tick_trigger_pipeline (gm removal)",
            &mut current_events,
            &uuid_to_entity,
            &mut *runtime,
            &mut objectives,
            &mut commands,
            &mut ship_modifiers,
            world_layers.pending_layers.as_deref_mut(),
            world_layers.layer_map.as_deref_mut(),
            next_state.as_deref_mut(),
            game_over_reason.as_deref_mut(),
            &mut faction_dispatch,
            &mut ai_query,
            balance_events.as_deref_mut(),
            now_tick,
            None,
            None,
            &mut effect_queues.out(),
        );
    }

    // The GM's armed placements (issue #1305), performed BEFORE the chaining
    // loop rather than inside it.
    //
    // Before, because a placement is not a trigger firing: it emits no
    // `WorldEvent` of its own, and running it inside a pass would make the
    // uuid a spawn draws depend on how many chaining passes that tick happened
    // to take. Draining it here — in canonical grant order, through the SAME
    // `dispatch_action` + `apply_dispatch_result` path a scripted
    // `spawn_entity` uses — gives every peer the same mint order at the same
    // tick, and gives the spawned entity the same name→uuid and group
    // registrations a scripted one gets, so a later objective target or
    // `on_all_destroyed` sees no difference between them.
    //
    // Drained unconditionally: an entry whose palette entry has gone (its layer
    // unloaded between the apply tick and here) is DROPPED rather than retained,
    // because unlike an armed Fire — which waits for a `when` predicate that may
    // yet read true — nothing about a placement can become possible later.
    if !runtime.pending_gm_spawns.is_empty() {
        let armed = std::mem::take(&mut runtime.pending_gm_spawns);
        let mut spawn_events: Vec<WorldEvent> = Vec::new();
        for pending in armed {
            // The action is built (and the palette borrow released) before the
            // apply below takes `&mut runtime`.
            let action = match crate::gm::operations::spawn::palette_entry(
                &runtime.gm_palette,
                &pending.palette,
            ) {
                Some(entry) => crate::gm::operations::spawn::spawn_action(&pending, entry),
                None => {
                    bevy::log::warn!(
                        "tick_trigger_pipeline: armed GM placement names palette \
                         entry '{}', which this world no longer authors - dropping",
                        pending.palette
                    );
                    continue;
                }
            };
            let name_to_uuid = runtime.name_to_uuid.clone();
            let layers = project_layer_views(world_layers.layer_map.as_deref());
            let result = {
                let ctx = DispatchContext {
                    // A GM placement belongs to the run, not to a layer: it
                    // carries resolved world coordinates, so it needs no anchor
                    // table, and a base-world origin is what keeps the entity
                    // alive across a layer unload.
                    origin_layer: None,
                    entity_name: None,
                    name_to_uuid: &name_to_uuid,
                    base_flags: &runtime.flags,
                    layers: &layers,
                    base_anchors: world_layers
                        .base_world_config
                        .as_ref()
                        .map(|wc| &wc.anchors)
                        .unwrap_or(&empty_anchors),
                    factions: faction_dispatch.registry.as_deref().map(|r| &r.0),
                    uuid_source: &uuid_source,
                    template_loader: &template_loader,
                };
                dispatch_action(&action, &ctx)
            };
            apply_dispatch_result(
                result,
                "tick_trigger_pipeline (gm placement)",
                &mut spawn_events,
                &uuid_to_entity,
                &mut *runtime,
                &mut objectives,
                &mut commands,
                &mut ship_modifiers,
                world_layers.pending_layers.as_deref_mut(),
                world_layers.layer_map.as_deref_mut(),
                next_state.as_deref_mut(),
                game_over_reason.as_deref_mut(),
                &mut faction_dispatch,
                &mut ai_query,
                balance_events.as_deref_mut(),
                now_tick,
                None,
                None,
                &mut effect_queues.out(),
            );
        }
        // A spawn emits no `WorldEvent` today, but the shared applier owns that
        // decision; anything it does emit joins the NEXT tick's buffer rather
        // than this tick's chain, matching `tick_delayed_actions`.
        runtime.pending_world_events.append(&mut spawn_events);
    }

    let mut pass = 0;
    loop {
        pass += 1;
        // Compute current_elapsed from TimerElapsed events in current_events.
        let current_elapsed = current_events
            .iter()
            .filter_map(|e| {
                if let crate::world::content::WorldEvent::TimerElapsed { elapsed_secs } = e {
                    Some(*elapsed_secs)
                } else {
                    None
                }
            })
            .fold(0.0_f32, |max_e, e| e.max(max_e));
        // All conditions read the live stores before any fired effect lands.
        let mut fired = runtime.triggers.evaluate_filtered(
            &current_events,
            &name_to_uuid,
            &runtime.entity_groups,
            current_elapsed,
            |origin| {
                layered_flag_chain_with_paths(
                    origin,
                    &runtime.flags,
                    world_layers.layer_map.as_deref(),
                )
            },
            (
                |state| {
                    !crate::gm::operations::event::state_event_id(state)
                        .is_some_and(|id| runtime.paused_gm_events.contains(&id))
                },
                |state| {
                    !crate::gm::operations::event::state_event_id(state)
                        .is_some_and(|id| runtime.pending_gm_event_skips.remove(&id))
                },
            ),
        );

        // An armed Skip whose event no longer exists cannot ever be honoured,
        // and authoritative state must not accumulate it — the Fire pass's rule
        // below, through the one `live_event_ids` both levers share. Nothing
        // else drops a Skip: a spent once-only event can be re-armed by
        // `reset_trigger`, and a `when` that reads false is a moment, not an
        // answer.
        if pass == 1 && !runtime.pending_gm_event_skips.is_empty() {
            let live = crate::gm::operations::event::live_event_ids(&runtime.triggers);
            runtime
                .pending_gm_event_skips
                .retain(|id| live.contains(id));
        }

        // The GM's armed Fires (issue #1301). Only in the first pass: a Fire is
        // one authored occurrence, and letting it re-enter a chaining pass
        // would let one press run a repeatable handler several times in a tick.
        //
        // Iterated over `triggers` in table order rather than over the
        // pending set, so the intra-tick order of a manual fire is the same
        // authored order an automatic one has, on every peer. Entries leave the
        // set when they are consumed by an actual firing, or when the event
        // they name can no longer take one — a spent once-only trigger, or an
        // id whose layer has been unloaded. Everything else (a `when` that
        // currently reads false, a cooldown that has not elapsed) deliberately
        // KEEPS the arm, so a Fire during a suppressed moment lands when the
        // moment arrives instead of being silently dropped.
        if pass == 1 && !runtime.pending_gm_event_fires.is_empty() {
            fired.extend(runtime.triggers.fire_manual(
                &mut runtime.pending_gm_event_fires,
                current_elapsed,
                crate::gm::operations::event::state_event_id,
                |origin| {
                    layered_flag_chain(origin, &runtime.flags, world_layers.layer_map.as_deref())
                },
            ));
        }

        // Scenario lowering joins the same dispatch/application body. Its
        // private evidence never appears as an event for legacy callbacks.
        let mut dispatches: Vec<_> = fired.into_iter().map(|fired| (fired, None)).collect();
        for activation in runtime.situations.ready(&runtime.situation_definitions) {
            if let Some(call) = &activation.call {
                dispatches.push((
                    phoenix_sim_world::world::trigger_registry::FiredWorldTrigger {
                        trigger_id: None,
                        handler: Some(
                            phoenix_sim_world::world::trigger_registry::ScriptHandlerRef {
                                script_path: call.script.clone(),
                                fn_name: call.function.clone(),
                            },
                        ),
                        context: crate::world::content::FiredTrigger {
                            origin_layer: None,
                            entity_name: None,
                        },
                    },
                    Some(activation),
                ));
            } else {
                runtime
                    .situations
                    .finish_activation(
                        &activation,
                        phoenix_sim_world::world::activities::ScenarioCallResult::Applied,
                        &runtime.situation_definitions,
                    )
                    .expect("ready closed Scenario transition");
            }
        }
        if dispatches.is_empty() {
            break;
        }

        let mut next_events: Vec<WorldEvent> = Vec::new();
        for (fired, activation) in dispatches {
            let handler = fired.handler;
            let ft = fired.context;
            let origin = handler
                .as_ref()
                .map(|handler| handler.script_path.clone())
                .or_else(|| ft.origin_layer.clone())
                .unwrap_or_else(|| "base-world".to_owned());
            let trigger_id = fired.trigger_id.or_else(|| {
                handler
                    .as_ref()
                    .map(|handler| format!("{}::{}", handler.script_path, handler.fn_name))
            });
            if let (Some(trigger_id), Some(msgs)) = (trigger_id, balance_events.as_deref_mut()) {
                let entity = ft
                    .entity_name
                    .as_deref()
                    .and_then(|name| name_to_uuid.get(name))
                    .cloned();
                msgs.write(crate::core::balance::BalanceEvent::TriggerFired {
                    trigger_id,
                    origin,
                    entity,
                });
            }

            // The handler for this trigger (IP-2, issue #984, Rhai M6 phase 2a).
            // A per-action dispatch loop for the fired trigger's own
            // `[[trigger.action]]` array used to run first; issue #985 deleted
            // the parser that filled it, so this is the whole of a fire. The
            // handler runs on the runtime host and its result goes through the
            // SAME apply path, so a scripted flag write chains into the next
            // pass exactly as a declarative `set_flag` used to
            // (`apply_script_commands`).
            if let Some(sr) = script.runtime.as_deref_mut() {
                if let Some(h) = handler {
                    let context = ScriptCallContext {
                        log_ctx: "tick_trigger_pipeline (script)",
                        clock: script_clock,
                        mission_clock_anchored: elapsed_secs.is_some(),
                        origin_layer: ft.origin_layer.clone(),
                        entity_name: ft.entity_name.clone(),
                        script_path: &h.script_path,
                        function: &h.fn_name,
                    };
                    let effects = if let Some(activation) = &activation {
                        match sr.invoke_situation_effects(
                            &context,
                            runtime,
                            world_layers.layer_map.as_deref(),
                            activation,
                        ) {
                            Ok(effects) => effects,
                            Err(error) => {
                                bevy::log::warn!(instance=%activation.instance,%error,"Scenario call refused; buffered effects discarded");
                                runtime.situations.finish_activation(activation, phoenix_sim_world::world::activities::ScenarioCallResult::Refused, &runtime.situation_definitions).expect("ready Scenario refusal");
                                None
                            }
                        }
                    } else {
                        match sr.invoke_effects(
                            &context,
                            runtime,
                            world_layers.layer_map.as_deref(),
                        ) {
                            Ok(effects) => Some(effects),
                            Err(_) => {
                                bevy::log::warn!("tick_trigger_pipeline: scripted handler '{}' names a missing unit '{}'",h.fn_name,h.script_path);
                                None
                            }
                        }
                    };
                    if let Some(effects) = effects {
                        let completed = effects.completed;
                        apply_script_call(
                            effects,
                            context,
                            ScriptEventTarget::TriggerChain(&mut next_events),
                            sr,
                            &uuid_to_entity,
                            runtime,
                            &mut objectives,
                            &mut commands,
                            &mut ship_modifiers,
                            world_layers.pending_layers.as_deref_mut(),
                            world_layers.layer_map.as_deref_mut(),
                            next_state.as_deref_mut(),
                            game_over_reason.as_deref_mut(),
                            &mut faction_dispatch,
                            &mut ai_query,
                            balance_events.as_deref_mut(),
                            // Reuse the SAME `uuid_source`/`template_loader`/anchors
                            // the declarative dispatch above used, and thread this
                            // trigger's origin/entity — so a scripted name-resolving
                            // effect resolves identically to its declarative twin
                            // (issue #984, Rhai M6).
                            &uuid_source,
                            &template_loader,
                            world_layers
                                .base_world_config
                                .as_ref()
                                .map(|wc| &wc.anchors)
                                .unwrap_or(&empty_anchors),
                            &mut effect_queues.out(),
                        );
                        if completed {
                            if let Some(activation) = &activation {
                                runtime.situations.finish_activation(activation, phoenix_sim_world::world::activities::ScenarioCallResult::Applied, &runtime.situation_definitions).expect("applied Scenario activation");
                            }
                        }
                    }
                }
            } else if let Some(activation) = &activation {
                bevy::log::warn!(instance=%activation.instance,"Scenario call refused: no accepted script runtime");
                runtime
                    .situations
                    .finish_activation(
                        activation,
                        phoenix_sim_world::world::activities::ScenarioCallResult::Refused,
                        &runtime.situation_definitions,
                    )
                    .expect("ready Scenario missing-runtime refusal");
            }
        }

        if next_events.is_empty() {
            break;
        }
        if pass >= MAX_CHAIN_PASSES {
            bevy::log::warn!(
                "tick_trigger_pipeline: trigger chain exceeded {MAX_CHAIN_PASSES} passes; \
                 stopping to prevent infinite loop"
            );
            break;
        }
        current_events = next_events;
    }
}

/// Drain actions from `pending_delayed_actions` whose `fire_at_elapsed` has
/// elapsed and dispatch them through the same `world::dispatch` table
/// `tick_trigger_pipeline` uses.
///
/// Registered after `tick_trigger_pipeline` in `SimSet::Physics` so that it sees
/// the same tick's `mission_clock` anchor.
pub(crate) fn tick_delayed_actions(
    mut runtime: ResMut<WorldContentRuntime>,
    time: Option<Res<bevy::time::Time>>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::server_app::GameOverReason>>,
    mut pending_layers: Option<ResMut<PendingWorldLayerChanges>>,
    mut layer_map: Option<ResMut<WorldLayerMap>>,
    base_world_config: Option<Res<crate::world::config::WorldConfig>>,
    entity_uuid_query: Query<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>,
    mut faction_dispatch: FactionDispatchParams,
    // Issue #710: `RemoveFactionEnemy` re-validates AI targets after a
    // successful removal. The immediate path always did; the delayed path did
    // not, because the old duplicated dispatch table it called had no
    // `ai_query` — a latent bug that let a delayed `remove_faction_enemy` leave
    // an in-progress engagement stuck on a now-friendly target. Both paths now
    // share one table, so both re-validate.
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    id_mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Entity as usize }>,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    // The per-owner effect queues (issue #1223): a delayed action can resolve a
    // `hold_fire` / `order_civilian` / infrastructure adjustment exactly as an
    // immediate one does, so it needs the same sinks.
    mut effect_queues: EffectQueues,
) {
    let Some(elapsed) = time.as_ref().and_then(|t| {
        runtime
            .mission_clock
            .map(|clock| clock.elapsed_secs(t.elapsed()))
    }) else {
        return;
    };

    if runtime.pending_delayed_actions.is_empty() {
        return;
    }

    let uuid_to_entity: std::collections::HashMap<String, Entity> = entity_uuid_query
        .iter()
        .map(|(ent, uuid_comp)| (uuid_comp.0.clone(), ent))
        .collect();

    let empty_anchors: HashMap<String, [f32; 3]> = HashMap::new();
    // Same template source as `tick_trigger_pipeline` (issue #715): one
    // `WasmTemplateLoader` per system run, both targets.
    let template_loader = crate::entities::loader::WasmTemplateLoader;
    let uuid_source = || {
        crate::world_id::mint_live_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity)
    };

    // Ready/still-pending is a pure decision (`world::delayed`); only the
    // elapsed-clock read above and the dispatch below touch Bevy.
    let queued = std::mem::take(&mut runtime.pending_delayed_actions);
    let schedule = partition_delayed_actions(queued, elapsed);
    runtime.pending_delayed_actions = schedule.still_pending;

    let trace_tick = effect_queues.sim_tick.as_deref().map_or(0, |tick| tick.0);
    let mut effects_out = effect_queues.out();
    for pda in schedule.ready {
        // Same live-store rule as `tick_trigger_pipeline`: re-project per action so
        // each dispatch sees the previous one's writes.
        let name_to_uuid = runtime.name_to_uuid.clone();
        let layers = project_layer_views(layer_map.as_deref());
        let result = {
            let ctx = DispatchContext {
                origin_layer: pda.origin_layer.clone(),
                entity_name: pda.entity_name.clone(),
                name_to_uuid: &name_to_uuid,
                base_flags: &runtime.flags,
                layers: &layers,
                base_anchors: base_world_config
                    .as_ref()
                    .map(|wc| &wc.anchors)
                    .unwrap_or(&empty_anchors),
                factions: faction_dispatch.registry.as_deref().map(|r| &r.0),
                uuid_source: &uuid_source,
                template_loader: &template_loader,
            };
            dispatch_action(&pda.action, &ctx)
        };

        // Unlike `tick_trigger_pipeline`, a delayed action's `new_events` are queued
        // for the NEXT tick: this system runs after `tick_trigger_pipeline` has
        // already drained `pending_world_events` for this one.
        let mut out_events: Vec<WorldEvent> = Vec::new();
        apply_dispatch_result(
            result,
            "tick_delayed_actions",
            &mut out_events,
            &uuid_to_entity,
            &mut runtime,
            &mut objectives,
            &mut commands,
            &mut ship_modifiers,
            pending_layers.as_deref_mut(),
            layer_map.as_deref_mut(),
            next_state.as_deref_mut(),
            game_over_reason.as_deref_mut(),
            &mut faction_dispatch,
            &mut ai_query,
            balance_events.as_deref_mut(),
            trace_tick,
            None,
            None,
            &mut effects_out,
        );
        runtime.pending_world_events.extend(out_events);
    }
}

/// Drain the scripted `after(n, |ctx| …)` callbacks that have become due and run
/// them through the live pipeline (issue #984, Rhai M6 phase 2b).
///
/// The completing half of the deferred-work seam M3 stood up: a scripted handler
/// (or an earlier callback) that calls `ctx.schedule.after(n, |ctx| { … })`
/// records a serialisable [`ScheduledCall`](crate::world::script::schedule::ScheduledCall)
/// on [`WorldScriptRuntime::pending_callbacks`]; this system drains the entries
/// whose `fire_tick` has arrived, resolves each against its unit's retained AST,
/// and feeds the call's effects through the SAME apply path the trigger handlers
/// use ([`apply_script_commands`]). Its effect kinds route identically to the
/// trigger-handler branch:
/// * `commands` — applied this tick; their chaining `new_events` queue onto
///   `pending_world_events` for the NEXT tick, exactly as `tick_delayed_actions`
///   routes its own (there is no within-tick chaining loop here).
/// * `delayed` — `in_seconds(..)` effects join `pending_delayed_actions`, dropped
///   when the mission clock is unanchored (same rule as the trigger path).
/// * `callbacks` — a callback that scheduled another callback re-queues it on
///   `pending_callbacks` for a future tick.
/// * `comms_opens` — an `open_comms` request queues on `pending_comms_opens` for
///   the comms module to materialise (issue #984).
///
/// # Shared per-tick budget
/// The [`TickBudget`] on `WorldScriptRuntime` is reset once per tick, keyed on
/// `SimTick`, and spans trigger-handler calls AND callback calls per the M3
/// contract. `tick_trigger_pipeline` runs first and normally does the reset; but
/// on a tick where it early-returned (no buffered events, no delayed actions) it
/// did not, so this system resets when it observes a new tick. Whichever script
/// system reaches the guard first this tick resets; the other sees the same tick
/// and shares the budget.
///
/// # Determinism
/// A no-op for every script-free world: with no `WorldScriptRuntime` the system
/// returns before any `DerefMut`, so it writes nothing and flips no change-detection
/// tick; and it is pinned `.after(tick_trigger_pipeline).before(tick_delayed_actions)`
/// with a conflict set that is a subset of those neighbours', so it introduces no new
/// ordering ambiguity among the RNG-drawing `Physics` systems. Together those force a
/// script-free digest to stay byte-identical. `drain_due` returns due calls in
/// authored order and every peer drains the same calls on the same tick (`fire_tick`
/// is a deterministic function of the tick a callback was scheduled on).
pub(crate) fn tick_script_callbacks(
    mut script: ScriptRuntimeParams,
    mut runtime: ResMut<WorldContentRuntime>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::server_app::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>,
    mut faction_dispatch: FactionDispatchParams,
    time: Option<Res<bevy::time::Time>>,
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    // The mint a callback-scheduled `spawn_entity` draws its `EntityUuid` from
    // (issue #984, Rhai M6). Added so the callback path builds the IDENTICAL
    // `uuid_source` the trigger path holds — else a spawn from an `after(..)`
    // callback would fall back to the process-global mint and diverge (R2). `None`
    // for a bare-`App` fixture, exactly like `tick_delayed_actions`.
    id_mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Entity as usize }>,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    // The per-owner effect queues a due callback's script pushes onto (issue
    // #1223), the same sinks the trigger and delayed paths use.
    mut effect_queues: EffectQueues,
) {
    // `now_tick` before the `WorldScriptRuntime` borrow (disjoint `script` field).
    let now_tick = script.sim_tick.as_ref().map(|t| t.0).unwrap_or(0);
    // Script-free world (no `WorldScriptRuntime`) or a bare-`App` fixture: nothing
    // to do. The `ResMut` params are fetched but never `DerefMut`'d on this arm, so
    // no change-detection tick flips — byte-identical to before this system existed.
    let Some(sr) = script.runtime.as_deref_mut() else {
        return;
    };

    // Reset the shared budget once per tick (`SimTick`-keyed): whichever script
    // system runs first this tick resets it, so trigger-handler ops and callback
    // ops share ONE budget per the M3 contract.
    sr.prepare_invocation_tick(now_tick);

    // Split off the due callbacks in authored order; the rest stay queued. Taking
    // the snapshot first means a callback re-queued this tick (even at delay 0,
    // `fire_tick == now_tick`) fires on a LATER tick, never re-entrantly here.
    let due = sr.pending_callbacks.drain_due(now_tick);
    if due.is_empty() {
        return;
    }

    // Named deadlines (issue #1024): whichever of the calls just split off IS a
    // deadline's arming becomes that deadline's firing. A lookup inside a drain
    // that already happens — not a second drain, and nothing here reads a clock
    // or scans for due work. Done BEFORE dispatch so a deadline's own handler
    // reads its state as `"fired"`, which is the honest answer while it runs.
    runtime.deadlines.note_fired(&due);

    // The clock a callback's OWN deferred work is stamped against — same shape as
    // `tick_trigger_pipeline`'s `script_clock`. `elapsed_secs` anchors a
    // callback-scheduled `in_seconds` effect; `tick`/`tick_hz` anchor a
    // callback-scheduled `after` callback.
    let elapsed_secs = time.as_ref().and_then(|t| {
        runtime
            .mission_clock
            .map(|clock| clock.elapsed_secs(t.elapsed()))
    });
    let script_clock = SchedClock {
        tick: now_tick,
        elapsed_secs: elapsed_secs.unwrap_or(0.0),
        tick_hz: world_layers
            .base_world_config
            .as_ref()
            .map(|wc| wc.global.sim_tick_hz)
            .unwrap_or(SchedClock::ZERO.tick_hz),
    };

    let uuid_to_entity: std::collections::HashMap<String, Entity> = entity_uuid_query
        .iter()
        .map(|(ent, uuid_comp)| (uuid_comp.0.clone(), ent))
        .collect();

    // The name-resolving-effect dispatch context (issue #984, Rhai M6), built
    // once like `tick_trigger_pipeline`'s: the SAME `mint_id_with(id_mint, Entity)`
    // closure so a callback-scheduled `spawn_entity` mints inside
    // `dispatch_spawn_entity` from the real `WorldIdMint`, the same
    // `WasmTemplateLoader`, and an empty base-anchors fallback.
    let empty_anchors: HashMap<String, [f32; 3]> = HashMap::new();
    let template_loader = crate::entities::loader::WasmTemplateLoader;
    let uuid_source = || {
        crate::world_id::mint_live_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity)
    };

    // Reborrow the `WorldContentRuntime` `ResMut` as a plain `&mut` so `runtime.flags`
    // (the flag overlay base) and `&mut runtime` (the apply path) can be borrowed
    // in sequence — the same disjoint-field split `tick_trigger_pipeline` uses.
    let runtime = &mut *runtime;

    for call in due {
        let callback_origin = call.origin_layer.clone();
        if let Some(trace) = effect_queues.test_trace.as_deref_mut() {
            let line = sr
                .function_lines
                .get(&call.script_path)
                .and_then(|lines| lines.get(&call.fn_name))
                .copied();
            trace.push(
                now_tick,
                Some(&call.script_path),
                line,
                crate::workshop::test_protocol::TestTraceKind::CallbackFired {
                    function: call.fn_name.clone(),
                    scheduled_tick: call.fire_tick,
                },
            );
        }
        let context = ScriptCallContext {
            log_ctx: "tick_script_callbacks",
            clock: script_clock,
            mission_clock_anchored: elapsed_secs.is_some(),
            origin_layer: callback_origin.clone(),
            entity_name: None,
            script_path: &call.script_path,
            function: &call.fn_name,
        };
        let effects = match sr.invoke_effects(&context, runtime, world_layers.layer_map.as_deref())
        {
            Ok(effects) => Some(effects),
            Err(_) => {
                bevy::log::warn!(
                    "tick_script_callbacks: callback '{}' names a missing unit '{}'",
                    call.fn_name,
                    call.script_path
                );
                None
            }
        };
        let Some(effects) = effects else {
            continue;
        };

        // A callback's chaining events queue for the NEXT tick, exactly as
        // `tick_delayed_actions` does — this system runs after
        // `tick_trigger_pipeline` has already drained `pending_world_events`.
        apply_script_call(
            effects,
            context,
            ScriptEventTarget::Pending,
            sr,
            &uuid_to_entity,
            runtime,
            &mut objectives,
            &mut commands,
            &mut ship_modifiers,
            world_layers.pending_layers.as_deref_mut(),
            world_layers.layer_map.as_deref_mut(),
            next_state.as_deref_mut(),
            game_over_reason.as_deref_mut(),
            &mut faction_dispatch,
            &mut ai_query,
            balance_events.as_deref_mut(),
            // Preserve the callback's captured owner. It has no trigger entity,
            // hence the separate `None` for `entity_name`.
            &uuid_source,
            &template_loader,
            world_layers
                .base_world_config
                .as_ref()
                .map(|wc| &wc.anchors)
                .unwrap_or(&empty_anchors),
            &mut effect_queues.out(),
        );
    }
}
