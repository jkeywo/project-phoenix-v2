//! Effects.
use super::*;

/// Replay a script call's buffered `ctx.deadlines.slip(…)` / `.cancel(…)` against
/// the live table, taking each resulting edit to the **existing** callback queue
/// (issue #1024).
///
/// This is where "a slipped deadline does not also fire at its old time" is
/// actually enforced: the pure table returns the exact `ScheduledCall` to
/// retract, and it is removed from `pending_callbacks` here before the
/// replacement is pushed. Nothing reconciles a table against a queue later;
/// there is one edit, applied at the point the script authored it.
///
/// Private to complete script-call application: the action dispatcher does not
/// own the callback queue, and callers must not replay this part separately.
pub(super) fn apply_deadline_changes(
    changes: &[crate::world::deadlines::DeadlineChange],
    deadlines: &mut crate::world::deadlines::DeadlineTable,
    pending_callbacks: &mut PendingCallbacks,
    now_tick: u64,
    tick_hz: f32,
) -> Vec<crate::world::script::schedule::ScheduledCall> {
    let mut scheduled = Vec::new();
    for change in changes {
        let Some(edit) = deadlines.apply(change, now_tick, tick_hz) else {
            continue;
        };
        if let Some(stale) = edit.retract {
            pending_callbacks.retract(&stale);
        }
        if let Some(fresh) = edit.push {
            scheduled.push(fresh.clone());
            pending_callbacks.push(fresh);
        }
    }
    scheduled
}

/// Replay a script call's buffered `ctx.commitments.record(…)` / `.keep(…)` /
/// `.break_promise(…)` against the live ledger (issue #1029).
///
/// The twin of [`apply_deadline_changes`], and deliberately smaller: a
/// commitment mutation edits no queue, so there is nothing to retract and
/// nothing to push. The campaign flag a resolution writes is **not** applied
/// here — it was emitted into the call's ordered effect buffer at the point the
/// script authored it, and `apply_script_commands` has already written it by the
/// time this runs. That ordering is deliberate rather than incidental: the
/// `FlagSet` it produces is evaluated only after the complete call returns,
/// whether in the next trigger chaining pass or after pending-event collection,
/// so an `on_flag_set` handler reading `ctx.commitments.state(…)` sees the settled
/// promise.
///
/// A duplicate id is logged rather than propagated. The script surface already
/// raised on it — dropping that call's whole buffer under settled decision 10,
/// which is where the author is told — so reaching here means the live ledger
/// disagreed with the per-call snapshot the raise was decided against, which
/// only two calls resolving the same new id in one tick can produce. Refusing
/// the second is the same answer the snapshot would have given.
///
/// Private to [`apply_script_call`], which commits the whole call before any
/// emitted event is evaluated by another handler.
pub(super) fn apply_commitment_changes(
    changes: &[crate::world::commitments::CommitmentChange],
    commitments: &mut crate::world::commitments::CommitmentLedger,
    now_tick: u64,
) {
    for change in changes {
        if let Err(duplicate) = commitments.apply(change, now_tick) {
            bevy::log::warn!(
                target: crate::logging::LogCat::World.target(),
                "{duplicate}"
            );
        }
    }
}

/// Resolve a dispatched UUID to its target entity's `ShipModifiers` component.
///
/// The pure layer resolves entity *name* -> *UUID* and stops there (writing a
/// component is irreducibly impure), so the last two hops — UUID -> `Entity` ->
/// component — land here. `what` names the calling action for the warn text.
pub(super) fn world_modifiers<'a>(
    ship_modifiers: &'a mut ShipModifiersParams,
    uuid_to_entity: &HashMap<String, Entity>,
    uuid: &str,
    log_ctx: &str,
    what: &str,
) -> Option<Mut<'a, crate::modifiers::ShipModifiers>> {
    let Some(target) = uuid_to_entity.get(uuid).copied() else {
        bevy::log::warn!("{log_ctx}: {what}: no ECS entity with UUID '{uuid}'");
        return None;
    };
    match ship_modifiers.components.get_mut(target) {
        Ok(mods) => Some(mods),
        Err(_) => {
            bevy::log::warn!(
                "{log_ctx}: {what}: entity with UUID '{uuid}' has no ShipModifiers component"
            );
            None
        }
    }
}

/// Rebuild the `ModifierSource::World` that identifies a world-applied modifier.
///
/// Not a tunable value: this identity is what lets a later `RemoveModifier`
/// find what an earlier `ApplyModifier` added.
pub(super) fn world_modifier_source(tag: String) -> crate::core::messages::ModifierSource {
    crate::core::messages::ModifierSource::World {
        id: WORLD_MODIFIER_SOURCE_ID.to_string(),
        tag,
    }
}

/// Give a scripted flag write the scope its own handler reads from (issue #1045).
/// Returns `false` when the command must be dropped instead of applied.
///
/// `ctx.flags.armed = 1` emits `MutateFlag { target_layer: None }` — the script
/// host has no idea which world its handler came from — while the matching
/// `on_flag_set("armed", …)` in that same script resolves its target layer from
/// the trigger's ORIGIN (`world::content::resolve_layer_prefix`). For a base-world
/// script both are `None` and they meet. For a LAYER they never did: the write
/// landed in the base store while the condition watched the layer's own store, so
/// a script that worked perfectly standalone went quietly dead the moment the same
/// world was loaded as a supporting layer. That asymmetry is what this closes.
///
/// The rule is the one `parent:` already implies: an unprefixed name means "my
/// scope", and each `parent:` steps one layer outward — resolved against the
/// handler's own loader chain, so `parent:` from a directly-loaded layer reaches
/// the base world. A base-origin handler has the one-entry chain `[None]`, so
/// every unprefixed write still resolves to `None` and every base world's
/// behaviour is byte-identical to before.
///
/// A name with more `parent:` steps than the chain has entries is DROPPED with a
/// warning rather than written somewhere arbitrary — the same "past root" answer
/// `resolve_layer_prefix` gives a trigger condition, which simply does not fire.
///
/// Every script call site threads the same owner: trigger handlers, deferred and
/// deadline callbacks, comms roots and comms response handlers. `None` therefore
/// means the root world everywhere; a supporting-world call retains its layer
/// even when its AST path is shared by another owner.
pub(super) fn scope_scripted_flag_write(
    cmd: &mut ActionCmd,
    log_ctx: &str,
    origin_layer: Option<&str>,
    layer_map: Option<&WorldLayerMap>,
) -> bool {
    let ActionCmd::MutateFlag {
        target_layer, name, ..
    } = cmd
    else {
        return true;
    };
    // Only an unscoped emission is rewritten. Nothing sets `Some` on this path
    // today; if something ever does, it has said what it means and is left alone.
    if target_layer.is_some() {
        return true;
    }
    // The common case by far — a base-world handler writing an unprefixed name —
    // resolves to exactly what it already is, so no chain is built for it.
    if origin_layer.is_none() && !name.starts_with("parent:") {
        return true;
    }
    let chain = layer_path_chain(origin_layer, layer_map);
    match crate::world::content::resolve_layer_prefix(name, &chain) {
        Some((stripped, resolved)) => {
            *name = stripped;
            *target_layer = resolved;
            true
        }
        None => {
            bevy::log::warn!(
                target: "world",
                "{log_ctx}: scripted flag write {name:?} walks past the root of layer \
                 {origin_layer:?}; dropping it"
            );
            false
        }
    }
}

/// The invocation facts that differ between trigger, callback and Comms calls.
/// Ownership is captured by the adapter; effect routing is owned by
/// [`apply_script_call`].
pub(crate) struct ScriptCallContext<'a> {
    pub log_ctx: &'a str,
    pub clock: SchedClock,
    /// A zero scheduling clock also exists before the mission has an anchor.
    /// Only an anchored mission accepts `in_seconds` work into its queue.
    pub mission_clock_anchored: bool,
    pub origin_layer: Option<String>,
    pub entity_name: Option<String>,
    pub script_path: &'a str,
    pub function: &'a str,
}

/// Where a completed call's World events next become eligible. Only a trigger
/// handler is inside a chaining pass. The other adapters use the ordinary
/// pending queue, observed when the World event collector next runs.
pub(crate) enum ScriptEventTarget<'a> {
    TriggerChain(&'a mut Vec<WorldEvent>),
    Pending,
}

/// Commit every effect of one successful script call (issue #1408).
///
/// Immediate actions retain their authored order and the existing pure
/// dispatcher. Future work then joins its owner queues, followed by deadline
/// edits (which can retract/re-key those callbacks) and commitment edits. The
/// exhaustive destructuring makes a new effect collection an implementation
/// obligation here, rather than a silent omission in one of four adapters.
///
/// A raising/refused script produces no effects at the runtime-host seam. A
/// successful Comms call with a malformed return still commits its effects;
/// deciding whether to display that return remains the Comms adapter's job.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_script_call(
    call: CallEffects,
    context: ScriptCallContext<'_>,
    event_target: ScriptEventTarget<'_>,
    script_runtime: &mut WorldScriptRuntime,
    uuid_to_entity: &HashMap<String, Entity>,
    runtime: &mut WorldContentRuntime,
    objectives: &mut ObjectiveManagerRes,
    commands: &mut Commands,
    ship_modifiers: &mut ShipModifiersParams,
    pending_layers: Option<&mut PendingWorldLayerChanges>,
    layer_map: Option<&mut WorldLayerMap>,
    next_state: Option<&mut NextState<GamePhase>>,
    game_over_reason: Option<&mut crate::server_app::GameOverReason>,
    faction_dispatch: &mut FactionDispatchParams,
    ai_query: &mut Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    balance_events: Option<&mut bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    uuid_source: &dyn Fn() -> String,
    template_loader: &dyn crate::entities::loader::TemplateLoader,
    base_anchors: &HashMap<String, [f32; 3]>,
    effect_queues: &mut EffectQueuesOut,
) {
    let CallEffects {
        completed,
        commands: immediate,
        delayed,
        callbacks,
        comms_opens,
        deadline_changes,
        commitment_changes,
    } = call;
    let source_line = script_runtime
        .function_lines
        .get(context.script_path)
        .and_then(|lines| lines.get(context.function))
        .copied();
    if completed {
        if let Some(trace) = effect_queues.test_trace.as_deref_mut() {
            trace.push(
                context.clock.tick,
                Some(context.script_path),
                source_line,
                crate::workshop::test_protocol::TestTraceKind::HostCall {
                    function: context.function.to_string(),
                },
            );
        }
    }
    let queue_events = matches!(&event_target, ScriptEventTarget::Pending);
    let mut pending_events = Vec::new();
    let events_out = match event_target {
        ScriptEventTarget::TriggerChain(events) => events,
        ScriptEventTarget::Pending => &mut pending_events,
    };
    apply_script_commands(
        immediate,
        context.log_ctx,
        events_out,
        uuid_to_entity,
        runtime,
        objectives,
        commands,
        ship_modifiers,
        pending_layers,
        layer_map,
        next_state,
        game_over_reason,
        faction_dispatch,
        ai_query,
        balance_events,
        uuid_source,
        template_loader,
        base_anchors,
        context.origin_layer,
        context.entity_name,
        effect_queues,
        context.clock.tick,
        context.script_path,
        source_line,
    );
    if queue_events {
        runtime.pending_world_events.extend(pending_events);
    }
    if context.mission_clock_anchored {
        runtime.pending_delayed_actions.extend(delayed);
    }
    if let Some(trace) = effect_queues.test_trace.as_deref_mut() {
        for callback in &callbacks {
            let line = script_runtime
                .function_lines
                .get(&callback.script_path)
                .and_then(|lines| lines.get(&callback.fn_name))
                .copied();
            trace.push(
                context.clock.tick,
                Some(&callback.script_path),
                line,
                crate::workshop::test_protocol::TestTraceKind::CallbackScheduled {
                    function: callback.fn_name.clone(),
                    fire_tick: callback.fire_tick,
                },
            );
        }
    }
    script_runtime.pending_callbacks.extend(callbacks);
    script_runtime.pending_comms_opens.extend(comms_opens);
    let deadline_callbacks = apply_deadline_changes(
        &deadline_changes,
        &mut runtime.deadlines,
        &mut script_runtime.pending_callbacks,
        context.clock.tick,
        context.clock.tick_hz,
    );
    if let Some(trace) = effect_queues.test_trace.as_deref_mut() {
        for callback in deadline_callbacks {
            let line = script_runtime
                .function_lines
                .get(&callback.script_path)
                .and_then(|lines| lines.get(&callback.fn_name))
                .copied();
            trace.push(
                context.clock.tick,
                Some(&callback.script_path),
                line,
                crate::workshop::test_protocol::TestTraceKind::CallbackScheduled {
                    function: callback.fn_name,
                    fire_tick: callback.fire_tick,
                },
            );
        }
    }
    apply_commitment_changes(
        &commitment_changes,
        &mut runtime.commitments,
        context.clock.tick,
    );
}

/// Apply a scripted handler's raw [`ActionCmd`]s through the same path as a
/// declarative action, computing each flag mutation's transition event first
/// (issue #984, Rhai M6 phase 2a).
///
/// **DETERMINISM-CRITICAL.** A script's `ctx.flags.*` write emits
/// [`ActionCmd::MutateFlag`] DIRECTLY onto the effect buffer, bypassing
/// `dispatch_action`'s `push_flag_transition` — the step that turns a declarative
/// `set_flag` into the `FlagSet` / `FlagCleared` event that chains downstream
/// `on_flag_set` / `on_flag_cleared` triggers. [`apply_dispatch_result`]'s
/// `MutateFlag` arm assumes that transition event is already in `events_out`. So
/// for each `MutateFlag` this previews the transition against the LIVE store and
/// pushes the resulting event into `events_out` BEFORE applying — so a scripted
/// flag write fires downstream triggers identically to a declarative one.
///
/// Processed command-by-command (one single-command `DispatchResult` each), so
/// each mutation is applied to the live store before the next command's preview,
/// mirroring the per-action decide-then-apply cycle the declarative loop uses.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_script_commands(
    commands_in: Vec<BufferedEffect>,
    log_ctx: &str,
    events_out: &mut Vec<WorldEvent>,
    uuid_to_entity: &HashMap<String, Entity>,
    runtime: &mut WorldContentRuntime,
    objectives: &mut ObjectiveManagerRes,
    commands: &mut Commands,
    ship_modifiers: &mut ShipModifiersParams,
    mut pending_layers: Option<&mut PendingWorldLayerChanges>,
    mut layer_map: Option<&mut WorldLayerMap>,
    mut next_state: Option<&mut NextState<GamePhase>>,
    mut game_over_reason: Option<&mut crate::server_app::GameOverReason>,
    faction_dispatch: &mut FactionDispatchParams,
    ai_query: &mut Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    mut balance_events: Option<
        &mut bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>,
    >,
    // The dispatch context a name-resolving `BufferedEffect::Action` needs (issue
    // #984, Rhai M6). `uuid_source` is the SAME closure `tick_trigger_pipeline`
    // binds — so a scripted `spawn_entity` mints its `EntityUuid` inside
    // `dispatch_spawn_entity` from the real `WorldIdMint`, in the same order as
    // the declarative twin (never at the effects.rs boundary, never a fallback
    // mint). `origin_layer`/`entity_name` come from the call owner: triggers,
    // deferred callbacks, deadline callbacks and comms nodes all preserve the
    // layer that authored them.
    uuid_source: &dyn Fn() -> String,
    template_loader: &dyn crate::entities::loader::TemplateLoader,
    base_anchors: &HashMap<String, [f32; 3]>,
    origin_layer: Option<String>,
    entity_name: Option<String>,
    // The per-owner effect queues (issue #1223), threaded through to the shared
    // `apply_dispatch_result` below unchanged.
    effects: &mut EffectQueuesOut,
    trace_tick: u64,
    script_path: &str,
    source_line: Option<usize>,
) {
    for eff in commands_in {
        match eff {
            // A resolved command (the M1 set + flag writes): applied directly, as
            // before. A `MutateFlag` gets its transition event previewed here (it
            // was pushed onto the sink DIRECTLY by `ctx.flags.*`, bypassing
            // `dispatch_action`'s transition step), so a scripted flag write chains
            // a downstream `on_flag_set` exactly as a declarative `set_flag` does.
            BufferedEffect::Cmd(cmd) => {
                let mut cmd: ActionCmd = cmd.into_resolved();
                let mut new_events: Vec<WorldEvent> = Vec::new();
                // Immediate `ctx.effects.load_world` is emitted before the host
                // knows which retained layer unit is running, so its command
                // starts with no loader. Stamp the same origin the delayed
                // `ctx.schedule.in_seconds(..).load_world` path carries on its
                // `DelayedAction`; explicit loader ownership, if a future
                // caller supplies it, always wins.
                if let ActionCmd::LoadWorld { loader_path, .. } = &mut cmd {
                    if loader_path.is_none() {
                        *loader_path = origin_layer.clone();
                    }
                }
                if !scope_scripted_flag_write(
                    &mut cmd,
                    log_ctx,
                    origin_layer.as_deref(),
                    layer_map.as_deref(),
                ) {
                    continue;
                }
                if let ActionCmd::MutateFlag {
                    target_layer,
                    name,
                    mutation,
                } = &cmd
                {
                    // Resolve the store this write lands in — `scope_scripted_flag_write`
                    // above has already turned the script's unscoped emission into the
                    // handler's own layer — preview the mutation against it, and push the
                    // FlagSet/FlagCleared into `events_out` (via `new_events`) BEFORE
                    // the MutateFlag is applied. This is the transition event a
                    // declarative `set_flag` gets at dispatch time.
                    let store: &crate::world::flags::FlagStore = match target_layer {
                        None => &runtime.flags,
                        Some(path) => layer_map
                            .as_deref()
                            .and_then(|lm| lm.0.get(path))
                            .map(|wr| &wr.flags)
                            .unwrap_or(&runtime.flags),
                    };
                    let (before, after) =
                        crate::world::dispatch::preview_mutation(store, name, mutation);
                    crate::world::dispatch::push_flag_transition(
                        &mut new_events,
                        name,
                        target_layer,
                        before,
                        after,
                    );
                }
                let result = DispatchResult {
                    commands: vec![cmd],
                    new_events,
                    ..Default::default()
                };
                apply_dispatch_result(
                    result,
                    log_ctx,
                    events_out,
                    uuid_to_entity,
                    runtime,
                    objectives,
                    commands,
                    ship_modifiers,
                    pending_layers.as_deref_mut(),
                    layer_map.as_deref_mut(),
                    next_state.as_deref_mut(),
                    game_over_reason.as_deref_mut(),
                    faction_dispatch,
                    ai_query,
                    balance_events.as_deref_mut(),
                    trace_tick,
                    Some(script_path),
                    source_line,
                    effects,
                );
            }

            // A name-resolving effect: resolve the buffered declarative action
            // through the SAME `dispatch_action` the TOML evaluator uses, then feed
            // the WHOLE `DispatchResult` (commands + new_events + name/group inserts
            // + warnings) to `apply_dispatch_result`. Feeding the whole result — not
            // just `.commands` — is load-bearing: a `spawn_entity`'s name→uuid and
            // group memberships ride in the insert vecs, and dropping them would let
            // a later `on_all_destroyed{group}` or objective-target lookup silently
            // diverge. Re-project `name_to_uuid`/`layers` per action so each sees the
            // previous one's writes (the live-store rule `DispatchContext::base_flags`
            // documents), matching `tick_delayed_actions`.
            BufferedEffect::Action(action) => {
                let name_to_uuid = runtime.name_to_uuid.clone();
                let layers = project_layer_views(layer_map.as_deref());
                let result = {
                    let ctx = DispatchContext {
                        origin_layer: origin_layer.clone(),
                        entity_name: entity_name.clone(),
                        name_to_uuid: &name_to_uuid,
                        base_flags: &runtime.flags,
                        layers: &layers,
                        base_anchors,
                        factions: faction_dispatch.registry.as_deref().map(|r| &r.0),
                        uuid_source,
                        template_loader,
                    };
                    dispatch_action(&action, &ctx)
                };
                apply_dispatch_result(
                    result,
                    log_ctx,
                    events_out,
                    uuid_to_entity,
                    runtime,
                    objectives,
                    commands,
                    ship_modifiers,
                    pending_layers.as_deref_mut(),
                    layer_map.as_deref_mut(),
                    next_state.as_deref_mut(),
                    game_over_reason.as_deref_mut(),
                    faction_dispatch,
                    ai_query,
                    balance_events.as_deref_mut(),
                    trace_tick,
                    Some(script_path),
                    source_line,
                    effects,
                );
            }
        }
    }
}

/// The transient per-tick effect queues an applied dispatch can enqueue
/// (issue #1223), borrowed as plain `&mut Vec<T>` so [`apply_dispatch_result`]
/// and [`apply_script_commands`] stay Bevy-agnostic — the same shape their
/// `runtime: &mut WorldContentRuntime` parameter already has. An effect-applying
/// SYSTEM holds the `EffectQueue<T>` resources (via [`EffectQueues`]) and
/// lends them here with [`EffectQueues::out`]; a bare-`App` test lends local
/// `Vec`s instead.
pub(crate) struct EffectQueuesOut<'a> {
    /// Drained by `crate::infrastructure::tick_infrastructure_condition`.
    pub condition_adjustments: &'a mut Vec<crate::infrastructure::ConditionAdjustment>,
    /// Drained by the same infrastructure tick.
    pub capacity_adjustments: &'a mut Vec<crate::infrastructure::CapacityAdjustment>,
    /// Drained by `crate::civilian::tick_civilian_traffic`.
    pub civilian_orders: &'a mut Vec<crate::civilian::PendingCivilianOrder>,
    /// Drained by `crate::ship::power::drain_scripted_power_orders` (issue
    /// #1398) — the scripted reactor orders `hold_fire` / `release_fire` push.
    pub power_orders: &'a mut Vec<crate::modifiers::power_system::PendingGroupPower>,
    /// Drained by `crate::narrative::emit_authored_and_marked_entity_narrative`
    /// (issue #1338): the authored beats and marked-entity outcomes a script
    /// declared this tick, plus the scripted-removal signal every
    /// `DestroyEntity` leaves behind, on their way to
    /// `Messages<NarrativeEvent>`.
    pub narrative: &'a mut Vec<crate::core::narrative::NarrativeRequest>,
    /// Drained by `crate::narrative::tick_computer_message` (issue #1342): a
    /// scenario's `ctx.effects.show_message(..)`, already validated at the
    /// script boundary, on its way to becoming the authoritative
    /// `ActiveComputerMessage` and a `shown`/`superseded` narrative pair.
    pub computer_message: &'a mut Vec<crate::core::computer_message::ComputerMessageRequest>,
    /// Drained by `crate::mission_report::apply_report_rows` (issue #1344): the
    /// post-mission report rows a script wrote this tick, on their way to the
    /// `MissionReport` accumulator.
    pub report_rows: &'a mut Vec<crate::core::report::ReportRow>,
    /// Test-only observation sink; absent from ordinary browser/native play.
    pub test_trace: Option<&'a mut crate::workshop::test_trace::TestTrace>,
}

/// The per-owner [`EffectQueue`] resources an effect-applying SYSTEM needs,
/// bundled as one `SystemParam` (issue #1223) so a dispatch system gains one
/// parameter rather than one each. Each resource is registered and declared
/// `ClearedAtFold` by its OWNING plugin (Infrastructure / Civilian / captain);
/// this bundle only borrows them at the push site.
///
/// Each queue is `Option<ResMut>` with a `Local` fallback so a bare-`App` test
/// that runs a dispatch system WITHOUT the owning plugins does not panic on a
/// missing resource: an effect with nowhere real to land goes to the fallback and
/// is dropped. In a full sim app every owning plugin registers its queue, so the
/// fallback is never reached — a property the digest A/B leans on, because an
/// effect silently dropped there would move a shipped world's digest.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct EffectQueues<'w, 's> {
    pub(super) condition:
        Option<ResMut<'w, EffectQueue<crate::infrastructure::ConditionAdjustment>>>,
    pub(super) capacity: Option<ResMut<'w, EffectQueue<crate::infrastructure::CapacityAdjustment>>>,
    pub(super) civilian_orders:
        Option<ResMut<'w, EffectQueue<crate::civilian::PendingCivilianOrder>>>,
    pub(super) power_orders:
        Option<ResMut<'w, EffectQueue<crate::modifiers::power_system::PendingGroupPower>>>,
    pub(super) narrative: Option<ResMut<'w, EffectQueue<crate::core::narrative::NarrativeRequest>>>,
    pub(super) computer_message:
        Option<ResMut<'w, EffectQueue<crate::core::computer_message::ComputerMessageRequest>>>,
    pub(super) report_rows: Option<ResMut<'w, EffectQueue<crate::core::report::ReportRow>>>,
    pub(super) test_trace: Option<ResMut<'w, crate::workshop::test_trace::TestTrace>>,
    pub(super) sim_tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    pub(super) condition_fallback: Local<'s, Vec<crate::infrastructure::ConditionAdjustment>>,
    pub(super) capacity_fallback: Local<'s, Vec<crate::infrastructure::CapacityAdjustment>>,
    pub(super) civilian_orders_fallback: Local<'s, Vec<crate::civilian::PendingCivilianOrder>>,
    pub(super) power_orders_fallback:
        Local<'s, Vec<crate::modifiers::power_system::PendingGroupPower>>,
    pub(super) narrative_fallback: Local<'s, Vec<crate::core::narrative::NarrativeRequest>>,
    pub(super) computer_message_fallback:
        Local<'s, Vec<crate::core::computer_message::ComputerMessageRequest>>,
    pub(super) report_rows_fallback: Local<'s, Vec<crate::core::report::ReportRow>>,
}

impl EffectQueues<'_, '_> {
    /// Borrow every queue as an [`EffectQueuesOut`] to lend to the applier,
    /// falling back to the per-queue `Local` sink when the resource is absent.
    pub(crate) fn out(&mut self) -> EffectQueuesOut<'_> {
        EffectQueuesOut {
            condition_adjustments: match &mut self.condition {
                Some(q) => &mut q.0,
                None => &mut self.condition_fallback,
            },
            capacity_adjustments: match &mut self.capacity {
                Some(q) => &mut q.0,
                None => &mut self.capacity_fallback,
            },
            civilian_orders: match &mut self.civilian_orders {
                Some(q) => &mut q.0,
                None => &mut self.civilian_orders_fallback,
            },
            power_orders: match &mut self.power_orders {
                Some(q) => &mut q.0,
                None => &mut self.power_orders_fallback,
            },
            narrative: match &mut self.narrative {
                Some(q) => &mut q.0,
                None => &mut self.narrative_fallback,
            },
            computer_message: match &mut self.computer_message {
                Some(q) => &mut q.0,
                None => &mut self.computer_message_fallback,
            },
            report_rows: match &mut self.report_rows {
                Some(q) => &mut q.0,
                None => &mut self.report_rows_fallback,
            },
            test_trace: self.test_trace.as_deref_mut(),
        }
    }
}

/// Perform everything one `DispatchResult` decided.
///
/// The impure half of the dispatch table (issue #710): `world::dispatch` decides
/// *what* should happen from read-only data, and this turns that into ECS
/// mutations. It is the shared apply path for `tick_trigger_pipeline` (immediate
/// actions), `tick_delayed_actions` (delayed ones), and
/// `console::comms::server::handle_respond_to_message` (comms-response actions,
/// issue #722) — `pub(crate)` so the comms module can reach it.
///
/// The one thing callers must decide for themselves is where `new_events` go, so
/// they are written to the caller's `events_out`: `tick_trigger_pipeline` points that
/// at the current pass's `next_events` (same tick, next chaining pass), whereas
/// `tick_delayed_actions` and `handle_respond_to_message` both drain it into
/// `runtime.pending_world_events` (next tick — there is no chaining loop in
/// either of those two callers, only `tick_trigger_pipeline`'s). Same events,
/// different destination.
///
/// `log_ctx` prefixes the pure layer's `warnings` so each message still names
/// the system it came from.
pub(crate) fn apply_dispatch_result(
    result: DispatchResult,
    log_ctx: &str,
    events_out: &mut Vec<WorldEvent>,
    uuid_to_entity: &HashMap<String, Entity>,
    runtime: &mut WorldContentRuntime,
    objectives: &mut ObjectiveManagerRes,
    commands: &mut Commands,
    ship_modifiers: &mut ShipModifiersParams,
    mut pending_layers: Option<&mut PendingWorldLayerChanges>,
    mut layer_map: Option<&mut WorldLayerMap>,
    mut next_state: Option<&mut NextState<GamePhase>>,
    mut game_over_reason: Option<&mut crate::server_app::GameOverReason>,
    faction_dispatch: &mut FactionDispatchParams,
    ai_query: &mut Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    // Balance telemetry: `ObjectiveCompleted` is emitted here, guarded on the
    // objective actually transitioning. `Option<&mut Messages<_>>` so callers
    // in bare-`App` fixtures (no registered message) can pass `None`.
    mut balance_events: Option<
        &mut bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>,
    >,
    trace_tick: u64,
    trace_source_path: Option<&str>,
    trace_source_line: Option<usize>,
    // The transient effect queues a name-resolved command lands on (issue
    // #1223): condition/capacity adjustments, civilian orders and scripted power
    // orders used to be `pending_*` fields on `runtime`; each is now its owning plugin's
    // `EffectQueue<T>` resource, lent here as plain `&mut Vec<T>`. The narrative
    // queue (issue #1338) joined them last.
    effects: &mut EffectQueuesOut,
) {
    let DispatchResult {
        commands: action_cmds,
        new_events,
        name_to_uuid_inserts,
        entity_group_inserts,
        warnings,
        override_failures,
    } = result;

    for warning in warnings {
        bevy::log::warn!("{log_ctx}: {warning}");
    }
    // Louder than `warnings` (issue #1048). Two producers now, both
    // `spawn_entity` and both leaving the world short of something it authored:
    // an override that could not be applied at all, and (issue #1046) a template
    // that did not resolve. See `DispatchResult::override_failures`'s doc for
    // why each warrants ERROR.
    for failure in override_failures {
        bevy::log::error!("{log_ctx}: {failure}");
    }

    events_out.extend(new_events);

    for cmd in action_cmds {
        match cmd {
            ActionCmd::Addressed {
                recipients,
                action,
                origin_layer,
            } => {
                let source_path = trace_source_path.map(str::to_string);
                commands.queue(move |world: &mut World| {
                    use bevy::ecs::system::RunSystemOnce;
                    match crate::recipients::resolve_in_world(world, &recipients) {
                        Ok(ships) if ships.is_empty() => {
                            bevy::log::warn!("Addressed action selected no current player ships");
                            crate::recipients::report(
                                world,
                                crate::recipients::RecipientDiagnostic {
                                    tick: trace_tick,
                                    source: source_path,
                                    line: trace_source_line,
                                    action: "addressed".into(),
                                    message:
                                        "No current player ships matched; action was not applied"
                                            .into(),
                                },
                            );
                        }
                        Ok(ships) => {
                            if let Err(error) = world.run_system_once_with(
                                apply_addressed_action,
                                (
                                    ships,
                                    *action,
                                    source_path,
                                    trace_source_line,
                                    trace_tick,
                                    origin_layer,
                                ),
                            ) {
                                bevy::log::error!("Addressed action application failed: {error}");
                            }
                        }
                        Err(error) => {
                            bevy::log::warn!("Addressed action refused: {error}");
                            crate::recipients::report(
                                world,
                                crate::recipients::RecipientDiagnostic {
                                    tick: trace_tick,
                                    source: source_path,
                                    line: trace_source_line,
                                    action: "addressed".into(),
                                    message: error.to_string(),
                                },
                            );
                        }
                    }
                });
            }
            ActionCmd::Presentation { ship, cue } => {
                commands.queue(move |world: &mut World| {
                    use bevy::ecs::system::RunSystemOnce;
                    let _ = world.run_system_once_with(
                        crate::gm::operations::presentation::apply_scenario_command,
                        (ship, cue),
                    );
                });
            }
            ActionCmd::SetContactInformation { observer, change } => {
                commands.queue(move |world: &mut World| {
                    if let Err(reason) = crate::gm::operations::information::apply_scenario_command(
                        world, &observer, &change,
                    ) {
                        bevy::log::warn!("Contact information action refused: {reason}");
                    }
                });
            }
            ActionCmd::SetNpcDoctrine { uuid, id } => {
                commands.queue(move |world: &mut World| {
                    use bevy::ecs::system::RunSystemOnce;
                    let _ = world.run_system_once_with(
                        crate::gm::operations::npc::apply_scenario_command,
                        (uuid, id),
                    );
                });
            }
            ActionCmd::ApprovePortOrder {
                visitor,
                provider,
                visit,
                lines,
            } => {
                let Some(visitor) = runtime.name_to_uuid.get(&visitor).cloned() else {
                    bevy::log::warn!("{log_ctx}: port order visitor name was not found");
                    continue;
                };
                let Some(provider) = runtime.name_to_uuid.get(&provider).cloned() else {
                    bevy::log::warn!("{log_ctx}: port order provider name was not found");
                    continue;
                };
                // The existing deferred World effect slot applies intent only.
                // No admission token or transient command buffer is fabricated.
                commands.queue(move |world: &mut World| {
                    if let Err(reason) = crate::resources::ports_server::approve_port_order(
                        world, &visitor, &provider, visit, lines,
                    ) {
                        bevy::log::warn!("Mission port order refused: {reason}");
                    }
                });
            }
            cmd @ (ActionCmd::AddObjective { .. }
            | ActionCmd::CompleteObjective { .. }
            | ActionCmd::FailObjective { .. }) => {
                crate::gm::operations::objective::apply_command(
                    &mut objectives.0,
                    cmd,
                    balance_events.as_deref_mut(),
                    layer_map.as_deref_mut(),
                );
            }
            cmd @ (ActionCmd::AddObjectiveInstance { .. }
            | ActionCmd::CompleteObjectiveInstance { .. }
            | ActionCmd::FailObjectiveInstance { .. }
            | ActionCmd::SetObjectiveInstanceProgress { .. }) => {
                let origin = crate::objective_instances::ObjectiveCommandOrigin {
                    source: trace_source_path.map(str::to_owned),
                    line: trace_source_line,
                    tick: trace_tick,
                };
                commands.queue(move |world: &mut World| {
                    crate::objective_instances::apply_command(world, cmd, &origin);
                });
            }

            ActionCmd::ResetTrigger { id } => {
                let n = runtime.triggers.reset_by_id(&id);
                if n == 0 {
                    bevy::log::warn!(
                        "{log_ctx}: ResetTrigger('{id}') matched no trigger with that id"
                    );
                }
            }

            // ── The authored mission timeline (issue #1338) ──────────────────
            //
            // Both arms only BUFFER. Nothing in the world moves, no state is
            // read back, and no message is written here — the applier holds no
            // message writers, and the three systems that call it are at Bevy's
            // parameter limit, which is exactly what the #1223 effect-queue
            // pattern exists for.
            // `narrative::emit_authored_and_marked_entity_narrative` turns each
            // request into its event on the same tick.
            ActionCmd::NarrativeBeat { id } => {
                effects
                    .narrative
                    .push(crate::core::narrative::NarrativeRequest::Authored {
                        kind: crate::core::narrative::NarrativeKind::BeatFired,
                        id,
                        entity_uuid: None,
                    });
            }

            ActionCmd::NarrativeOutcome { entity, outcome } => {
                // Resolved by NAME against the runtime's map, like every other
                // name-carrying command here. An unresolvable name is still
                // recorded — the authored id is the timeline's identity, and a
                // beat about an entity that has already despawned is precisely
                // the case an "escaped"/"destroyed" outcome is authored for.
                let entity_uuid = runtime.name_to_uuid.get(&entity).cloned();
                effects
                    .narrative
                    .push(crate::core::narrative::NarrativeRequest::Authored {
                        kind: outcome,
                        id: entity,
                        entity_uuid,
                    });
            }

            // Buffered exactly like the two arms above, and for the same
            // reason: no name resolution needed (a Station id is not an
            // entity name), and the applier holds no message writer to log
            // the shown/superseded pair itself.
            // `crate::narrative::tick_computer_message` turns this into the
            // authoritative `ActiveComputerMessage` state and its narrative
            // events on the same tick.
            ActionCmd::ShowComputerMessage {
                id,
                text,
                severity,
                duration_secs,
                station,
            } => {
                effects.computer_message.push(
                    crate::core::computer_message::ComputerMessageRequest {
                        id,
                        text,
                        severity,
                        duration_secs,
                        station,
                    },
                );
            }

            ActionCmd::ApplyModifier {
                uuid,
                tag,
                slot,
                bonus,
            } => {
                let Some(mut mods) = world_modifiers(
                    ship_modifiers,
                    uuid_to_entity,
                    &uuid,
                    log_ctx,
                    "ApplyModifier",
                ) else {
                    continue;
                };
                mods.add_or_update(crate::modifiers::Modifier {
                    source: world_modifier_source(tag),
                    slot,
                    bonus,
                });
            }

            ActionCmd::RemoveModifier { uuid, tag, slot } => {
                let Some(mut mods) = world_modifiers(
                    ship_modifiers,
                    uuid_to_entity,
                    &uuid,
                    log_ctx,
                    "RemoveModifier",
                ) else {
                    continue;
                };
                mods.remove(&world_modifier_source(tag), &slot);
            }

            ActionCmd::ApplyFlag { uuid, tag, kind } => {
                let Some(mut mods) =
                    world_modifiers(ship_modifiers, uuid_to_entity, &uuid, log_ctx, "ApplyFlag")
                else {
                    continue;
                };
                mods.add_flag(world_modifier_source(tag), kind);
            }

            ActionCmd::RemoveFlag { uuid, tag, kind } => {
                let Some(mut mods) =
                    world_modifiers(ship_modifiers, uuid_to_entity, &uuid, log_ctx, "RemoveFlag")
                else {
                    continue;
                };
                mods.remove_flag(world_modifier_source(tag), kind);
            }

            ActionCmd::ApplyIntModifier {
                uuid,
                tag,
                slot,
                bonus,
            } => {
                let Some(mut mods) = world_modifiers(
                    ship_modifiers,
                    uuid_to_entity,
                    &uuid,
                    log_ctx,
                    "ApplyIntModifier",
                ) else {
                    continue;
                };
                mods.add_or_update_int(crate::modifiers::IntModifier {
                    source: world_modifier_source(tag),
                    slot,
                    bonus,
                });
            }

            ActionCmd::RemoveIntModifier { uuid, tag, slot } => {
                let Some(mut mods) = world_modifiers(
                    ship_modifiers,
                    uuid_to_entity,
                    &uuid,
                    log_ctx,
                    "RemoveIntModifier",
                ) else {
                    continue;
                };
                mods.remove_int(&world_modifier_source(tag), &slot);
            }

            // Buffered, never written here: the applier holds no resources
            // (issue #1223). `mission_report::apply_report_rows` drains the
            // queue into `MissionReport` in queue order on the same tick.
            ActionCmd::SetReportRow(row) => {
                effects.report_rows.push(row);
            }

            // Always applied before `SetNextState` below —
            // `OnEnter(GamePhase::GameOver)` reads the reason resource.
            ActionCmd::SetGameOverReason { reason, outcome } => {
                if let Some(gr) = game_over_reason.as_deref_mut() {
                    gr.0 = Some(reason);
                    // Declared outcome (#843), or `None` for an undeclared
                    // scripted end — the headless classifier defaults that to
                    // victory.
                    gr.1 = outcome;
                }
            }

            ActionCmd::SetNextState { phase } => {
                if let Some(ns) = next_state.as_deref_mut() {
                    ns.set(phase);
                }
            }

            // Issue #1025. Name resolution here (the applier holds
            // `name_to_uuid`); the arithmetic and the flag edges happen in
            // `tick_infrastructure_condition`, which drains this queue in
            // `SimSet::Modifiers`.
            ActionCmd::ReplaceRegionField { entity, profile } => {
                if profile.validate().is_err() {
                    continue;
                }
                let Some(target) = runtime
                    .name_to_uuid
                    .get(&entity)
                    .and_then(|uuid| uuid_to_entity.get(uuid))
                    .copied()
                else {
                    continue;
                };
                commands.queue(move |world: &mut World| {
                    let tick = world
                        .get_resource::<crate::sim_tick::SimTick>()
                        .map_or(0, |tick| tick.0);
                    let dt = world
                        .get_resource::<Time<bevy::time::Fixed>>()
                        .map(|time| time.timestep().as_secs_f32())
                        .unwrap_or_else(|| {
                            Time::<bevy::time::Fixed>::default()
                                .timestep()
                                .as_secs_f32()
                        });
                    let pose = world
                        .get::<crate::regions::field::RegionFieldState>(target)
                        .and_then(|state| state.sample_pose(tick, dt).ok());
                    if let Some(pose) = pose {
                        if let Ok(state) = crate::regions::field::RegionFieldState::new(
                            profile,
                            tick,
                            pose.origin.to_array(),
                        ) {
                            let valid = world.get::<crate::entities::spawner::RegionShapeSection>(target)
                                .is_some_and(|shape| state.validate_environment(&shape.0,
                                    world.get::<crate::entities::spawner::RegionEffectsSection>(target)
                                        .map_or(&[], |effects| effects.0.as_slice())).is_ok());
                            if valid { world.entity_mut(target).insert(state); }
                        }
                    }
                });
            }
            ActionCmd::AdjustInfrastructureCondition { entity, delta } => {
                let Some(uuid) = runtime.name_to_uuid.get(&entity).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: AdjustInfrastructureCondition: no entity named '{entity}' \
                         in this world — ignoring"
                    );
                    continue;
                };
                effects
                    .condition_adjustments
                    .push(crate::infrastructure::ConditionAdjustment { uuid, delta });
            }

            // Issue #1042, and the same three lines because it is the same
            // shape: resolve the name here, queue the move, and let
            // `tick_infrastructure_condition` do the arithmetic and re-publish
            // the counter. A capacity the structure never declared is dropped
            // THERE, with a warning, because that check needs the component.
            ActionCmd::AdjustInfrastructureCapacity {
                entity,
                capacity,
                delta,
            } => {
                let Some(uuid) = runtime.name_to_uuid.get(&entity).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: AdjustInfrastructureCapacity: no entity named '{entity}' \
                         in this world — ignoring"
                    );
                    continue;
                };
                effects
                    .capacity_adjustments
                    .push(crate::infrastructure::CapacityAdjustment {
                        uuid,
                        capacity,
                        delta,
                    });
            }

            // Issue #1035. Nothing to resolve and nothing to queue: a
            // workforce is a party rather than an entity, and its register is
            // a field on the very runtime this applier already holds. The
            // mirror flag rides beside this command as an ordinary
            // `MutateFlag`, so it gets its transition event from the one path
            // that emits them.
            ActionCmd::SetWorkforceState { id, mutation } => {
                if runtime.workforce.apply(&id, mutation).is_none() {
                    bevy::log::debug!(
                        "{log_ctx}: SetWorkforceState: '{id}' is not a side this world \
                         declared, or was already in that state — nothing moved"
                    );
                }
            }

            // Issues #1041/#1398. The name is resolved here — the applier is
            // where `name_to_uuid` lives — and the order is queued for
            // `ship::power::drain_scripted_power_orders`, which is the one
            // system holding both an entity query and the reactor. The mirror
            // flag is NOT written here: `weapons_cold.*` is mirrored off the
            // reactor, so a scenario's order and an Engineering officer's order
            // produce the same transition event.
            ActionCmd::SetWeaponsRestraint { entity, held } => {
                let Some(uuid) = runtime.name_to_uuid.get(&entity).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: SetWeaponsRestraint: no entity named '{entity}' in this \
                         world — ignoring"
                    );
                    continue;
                };
                effects
                    .power_orders
                    .push(crate::modifiers::power_system::PendingGroupPower { uuid, held });
            }

            // Issue #1028, and the same shape for the same reason: the applier
            // holds `name_to_uuid`, so the name is resolved here and the order
            // is queued for `tick_civilian_traffic`, which is the one system
            // that owns the compliance state machine.
            ActionCmd::OrderCivilian { entity, order } => {
                if let Err(why) = order.validate() {
                    bevy::log::warn!("{log_ctx}: OrderCivilian for '{entity}': {why} — ignoring");
                    continue;
                }
                let Some(uuid) = runtime.name_to_uuid.get(&entity).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: OrderCivilian: no entity named '{entity}' in this \
                         world — ignoring"
                    );
                    continue;
                };
                effects
                    .civilian_orders
                    .push(crate::civilian::PendingCivilianOrder { uuid, order });
            }

            // Issue #1031. Name resolution here for the same reason as the four
            // above — the applier is where `name_to_uuid` lives — but the write
            // is IMMEDIATE rather than queued: an evidence entry is a record of
            // something that already happened, so no system owns an edge, a
            // threshold or a compliance machine that has to see it first.
            //
            // An unresolvable subject is a warned no-op (AC2). The name is
            // resolved a tick after the beat that wrote it, against a world that
            // may have moved on, and a scenario appending to a hull that has
            // been destroyed should lose the entry rather than the run.
            ActionCmd::RecordDossierEvidence {
                subject,
                text,
                provenance,
                gathered_at_tick,
                reference,
            } => {
                let Some(uuid) = runtime.name_to_uuid.get(&subject).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: RecordDossierEvidence: no entity named '{subject}' in \
                         this world — ignoring"
                    );
                    continue;
                };
                let appended = runtime.evidence.append_reference(
                    &uuid,
                    &text,
                    provenance,
                    gathered_at_tick,
                    reference,
                );
                let appended = match appended {
                    Ok(value) => value,
                    Err(error) => {
                        bevy::log::warn!("{log_ctx}: invalid dossier reference: {error}");
                        continue;
                    }
                };
                if !appended {
                    // AC3, and a no-op the author is told about rather than one
                    // that disappears: a scenario reaching the same finding
                    // twice is legitimate (a re-scan), so this is a debug line
                    // and not a warning.
                    bevy::log::debug!(
                        "{log_ctx}: RecordDossierEvidence: '{text}' ({}) is already on \
                         '{subject}'s file — keeping the first stamp",
                        provenance.as_str()
                    );
                }
            }

            ActionCmd::LoadWorld { path, loader_path } => {
                if let Some(lc) = pending_layers.as_deref_mut() {
                    lc.0.push(WorldLayerChange::Load { path, loader_path });
                }
            }

            ActionCmd::UnloadWorld { path } => {
                if let Some(lc) = pending_layers.as_deref_mut() {
                    lc.0.push(WorldLayerChange::Unload(path));
                }
            }

            // `target_layer` and `name` arrive already resolved: `parent:`
            // prefixes are stripped and walked, and the layer was proved to
            // exist against the same projection this applier writes to. The
            // transition event (if any) is already in `events_out`.
            ActionCmd::MutateFlag {
                target_layer,
                name,
                mutation,
            } => {
                let store = match &target_layer {
                    None => Some(&mut runtime.flags),
                    Some(path) => layer_map
                        .as_deref_mut()
                        .and_then(|lm| lm.0.get_mut(path))
                        .map(|wr| &mut wr.flags),
                };
                let Some(store) = store else {
                    bevy::log::warn!(
                        "{log_ctx}: MutateFlag: target layer {target_layer:?} missing from \
                         WorldLayerMap — ignoring '{name}'"
                    );
                    continue;
                };
                let (before, after) =
                    crate::world::dispatch::preview_mutation(store, &name, &mutation);
                if let Some(trace) = effects.test_trace.as_deref_mut() {
                    trace.push(
                        trace_tick,
                        trace_source_path,
                        trace_source_line,
                        crate::workshop::test_protocol::TestTraceKind::FlagMutation {
                            name: name.clone(),
                            before,
                            after,
                            layer: target_layer.clone(),
                        },
                    );
                }
                match mutation {
                    crate::world::dispatch::FlagMutation::Set => store.set_flag(&name),
                    crate::world::dispatch::FlagMutation::Clear => store.clear_flag(&name),
                    crate::world::dispatch::FlagMutation::Increment(by) => {
                        store.increment_flag(&name, by)
                    }
                    crate::world::dispatch::FlagMutation::SetValue(v) => {
                        store.set_flag_value(&name, v)
                    }
                };
            }

            ActionCmd::SpawnEntity {
                config,
                name,
                uuid,
                position,
                rotation,
                scale,
                layer_path,
                template_path,
                overrides,
            } => {
                // The template arrives already resolved and name-patched: the
                // pure layer loaded it via `DispatchContext::template_loader`
                // and gated the failure path (issue #715), so a command here
                // always spawns.
                let pos_vec = Vec3::new(position[0], position[1], position[2]);
                let spawned =
                    crate::entities::spawner::spawn_entity(commands, &config, pos_vec, uuid, None);

                // Issue #863: the ONE site a runtime spawn happens, and so the
                // one site that records what it was made from. Everything the
                // record needs is in hand here and nowhere afterwards — the
                // template path and the override document are both consumed by
                // the resolution above and leave no trace on the spawned
                // entity, which is exactly why a resume could not rebuild one.
                //
                // Stamped unconditionally rather than only for hostiles or only
                // for ships: what makes an entity worth recording is that a
                // *script* made it, not what it turned out to be.
                commands.entity(spawned).insert((
                    crate::entities::spawner::EntityTemplatePath::new(&template_path),
                    crate::entities::spawner::EntitySpawnOrigin(
                        crate::world::spawn_origin::SpawnOrigin {
                            template_path,
                            name,
                            position,
                            rotation,
                            scale,
                            overrides,
                            layer_path: layer_path.clone(),
                        },
                    ),
                ));

                // Apply optional rotation (XYZ Euler radians) and scale
                // (per-axis) via the canonical `TransformConfig` conversions —
                // the same parse-layer helpers the static `[[entity]]` schema
                // uses. `spawn_entity` only set translation; overwrite the
                // whole Transform when either is supplied.
                if rotation.is_some() || scale.is_some() {
                    let tc = crate::world::config::TransformConfig {
                        rotation,
                        scale,
                        ..Default::default()
                    };
                    commands.entity(spawned).insert(Transform {
                        translation: pos_vec,
                        rotation: tc.quat(),
                        scale: tc.scale_vec(),
                    });
                }

                // A hull's pose is `ShipPhysics`, not its `Transform`: the
                // spawner seeds `yaw: 0.0` and `integrate_ship_physics`
                // overwrites the Transform from it every tick, so an authored
                // rotation on a SHIP used to survive exactly until the first
                // fixed step. That is what a GM's dragged heading (issue #1305)
                // would have lost, and a scripted spawn's authored rotation
                // with it. Seeded here, once, through an entity command that
                // runs after the spawner's own inserts and only where the
                // component exists — a rotation-free spawn (every one every
                // shipped world makes) queues nothing and is byte-identical.
                //
                // `-rotation[1]` because the Transform Euler the physics writes
                // is `from_euler(YXZ, -yaw, 0, roll)`; the two spellings of one
                // pose must not disagree the moment the hull exists.
                if let Some([_, transform_yaw, _]) = rotation {
                    commands.entity(spawned).queue(
                        move |mut entity: bevy::ecs::world::EntityWorldMut| {
                            if let Some(mut physics) =
                                entity.get_mut::<crate::ship::state::ShipPhysics>()
                            {
                                physics.yaw = -transform_yaw;
                            }
                        },
                    );
                }

                // Attach to the authoring layer's spawned_entities so
                // `UnloadWorld` despawns the entity (base-world origin: the
                // entity just persists for the session), and stamp its
                // origin layer (issue #891 review finding 1) so
                // `entity_flag_chain` can read it in O(1) instead of scanning
                // `WorldLayerMap` for it later.
                if let (Some(path), Some(lm)) = (&layer_path, layer_map.as_deref_mut()) {
                    if let Some(layer) = lm.0.get_mut(path) {
                        layer.spawned_entities.push(spawned);
                        commands
                            .entity(spawned)
                            .insert(EntityOriginLayer(path.clone()));
                        crate::console::helm::transit::seed_layer_origin(
                            &mut commands.entity(spawned),
                            path,
                        );
                    }
                }
            }

            ActionCmd::DestroyEntity { uuid } => {
                let target_entity = uuid_to_entity.get(&uuid).copied();
                // The mission timeline's no-silent-vanish signal (issue #1338).
                // Queued for EVERY scripted removal because this applier cannot
                // see a `NarrativeMark` — it is lent plain `&mut Vec<_>` sinks
                // and no component query — so the mark gate lives in
                // `narrative::emit_authored_and_marked_entity_narrative`, which
                // already remembers every marked uuid it has seen and drops an
                // unmarked one. Deliberately NOT a `BalanceEvent`: a scripted
                // removal is an authorial act, and a rescue-by-despawn must
                // never count as a destruction in the combat ledger (PRD
                // #1337's "supplements the ledger, never changes it"). The
                // emitter turns it into `marked_entity_destroyed` only when the
                // author recorded no outcome of their own for that entity.
                effects
                    .narrative
                    .push(crate::core::narrative::NarrativeRequest::ScriptedRemoval {
                        entity_uuid: uuid.clone(),
                    });
                // The matching `WorldEvent::Destroyed` is already in
                // `events_out` so chained `on_destroyed` triggers fire.
                //
                // We deliberately do NOT use `MessageWriter<AiEntityDestroyed>`
                // directly: `tick_trigger_pipeline` already holds the matching
                // reader, which would trip Bevy's B0002 access check. Deferring
                // the write via a command runs it after the system exits, so
                // external consumers (telemetry, save/load, achievements)
                // observe script-killed entities the same as combat-killed ones.
                commands.queue(move |world: &mut World| {
                    if let Some(mut msgs) =
                        world.get_resource_mut::<Messages<crate::ai::server::AiEntityDestroyed>>()
                    {
                        msgs.write(crate::ai::server::AiEntityDestroyed { entity_uuid: uuid });
                    }
                });
                if let Some(ent) = target_entity {
                    commands.queue(move |world: &mut World| {
                        // The despawn inverse's capture (issue #1444), taken one
                        // instant before the entity stops existing and only when
                        // an accepted GM removal armed this uuid. Every ordinary
                        // scripted or combat destruction gets `None` here and
                        // pays for no capture walk.
                        let capture =
                            crate::gm::operations::despawn_undo::capture_before_removal(world, ent);
                        let cleared = crate::gm::operations::despawn::remove_entity(world, ent);
                        crate::gm::operations::despawn_undo::record_capture(
                            world, capture, cleared,
                        );
                    });
                }
            }

            ActionCmd::AddFactionEnemy {
                faction_uuid,
                enemy_uuid,
            } => {
                let Some(registry) = faction_dispatch.registry.as_deref_mut() else {
                    bevy::log::warn!(
                        "{log_ctx}: AddFactionEnemy skipped: FactionRegistryResource not present"
                    );
                    continue;
                };
                // Idempotent: returns false if `enemy_uuid` is already listed.
                // Either way no target re-validation is needed, because adding a
                // hostility cannot invalidate an existing engagement — the next
                // `enemy_in_range` tick organically picks the new relationship
                // up. `RemoveFactionEnemy` below is deliberately asymmetric.
                registry.0.add_enemy(faction_uuid, enemy_uuid);
            }

            ActionCmd::RemoveFactionEnemy {
                faction_uuid,
                enemy_uuid,
            } => {
                let Some(registry) = faction_dispatch.registry.as_deref_mut() else {
                    bevy::log::warn!(
                        "{log_ctx}: RemoveFactionEnemy skipped: FactionRegistryResource not present"
                    );
                    continue;
                };
                let removed = registry.0.remove_enemy(faction_uuid, enemy_uuid);
                if removed {
                    // Snapshot every AI controller's own faction BEFORE taking
                    // the &mut on the query for re-validation. `iter()` on a
                    // `&mut Query` yields immutable refs, so there is no borrow
                    // conflict with the subsequent `iter_mut()`.
                    let ai_factions: Vec<(uuid::Uuid, uuid::Uuid)> = ai_query
                        .iter()
                        .filter_map(|(uid, _, fc)| {
                            let self_uuid = uuid::Uuid::parse_str(&uid.0).ok()?;
                            fc.map(|fc| (self_uuid, fc.0))
                        })
                        .collect();
                    let uuid_to_faction =
                        build_uuid_to_faction(&faction_dispatch.non_ai_factions, &ai_factions);
                    revalidate_ai_targets_after_faction_change(
                        ai_query,
                        &registry.0,
                        &uuid_to_faction,
                    );
                }
            }
        }
    }

    // Both maps arrive already gated: a `SpawnEntity` whose template failed
    // to resolve returns a warning-only result with no inserts (issue #715
    // moved that gate into `dispatch_spawn_entity`), so everything here
    // applies unconditionally.
    for (name, uuid) in name_to_uuid_inserts {
        runtime.name_to_uuid.insert(name, uuid);
    }
    for (group, name) in entity_group_inserts {
        runtime.entity_groups.entry(group).or_default().insert(name);
    }
}

/// Build a `UUID → faction UUID` map from every entity that carries a
/// `FactionComponent`. Used by `revalidate_ai_targets_after_faction_change`
/// to resolve a controller's `blackboard.target` UUID back to a faction so
/// the new `is_enemy` relationship can be evaluated.
///
/// The two queries cover disjoint sets of entities: `non_ai_factions`
/// holds factioned entities without a `BehaviourSection` (player
/// ship, stations, factioned beacons) and the AI controllers themselves
/// (which may also carry a faction) are gathered from `ai_factions`.
pub(crate) fn build_uuid_to_faction(
    non_ai_factions: &Query<
        (&EntityUuid, &crate::entities::spawner::FactionComponent),
        Without<BehaviourSection>,
    >,
    ai_factions: &[(uuid::Uuid, uuid::Uuid)],
) -> std::collections::HashMap<uuid::Uuid, uuid::Uuid> {
    let mut map = std::collections::HashMap::new();
    for (uid, fc) in non_ai_factions.iter() {
        if let Ok(uuid) = uuid::Uuid::parse_str(&uid.0) {
            map.insert(uuid, fc.0);
        }
    }
    for (self_uuid, faction_uuid) in ai_factions {
        map.insert(*self_uuid, *faction_uuid);
    }
    map
}

/// Bundles the optional world-layer mutation resources used by
/// `handle_respond_to_message` and `tick_trigger_pipeline` into a single
/// `SystemParam` so both functions stay within Bevy's 16-parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct WorldLayerParams<'w> {
    pub pending_layers: Option<ResMut<'w, PendingWorldLayerChanges>>,
    pub layer_map: Option<ResMut<'w, WorldLayerMap>>,
    pub base_world_config: Option<Res<'w, crate::world::config::WorldConfig>>,
}

/// Bundle of per-entity `ShipModifiers` writers used by
/// `handle_respond_to_message` and `tick_trigger_pipeline` to route
/// `TriggerAction::{Apply,Remove}{Modifier,Flag,IntModifier}` actions to
/// the named target entity's Component (not the legacy global Resource).
///
/// Grouping the mutable query into a `SystemParam` keeps both handlers
/// under Bevy's 16-parameter limit. Every ship entity (player + NPC) is
/// spawned with a `ShipModifiers` Component (`crates/phoenix-simulation/src/entities/spawner.rs`
/// and `spawn_game_start_entities`), so `.get_mut(entity)` is the correct
/// primary write target after the name is resolved through
/// `WorldContentRuntime.name_to_uuid` → UUID → ECS `Entity`.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ShipModifiersParams<'w, 's> {
    pub components: Query<'w, 's, &'static mut crate::modifiers::ShipModifiers>,
}

/// The AI-plugin messages `collect_world_events` bridges into `WorldEvent`s.
///
/// Grouped into a `SystemParam` for the same reason as `ShipModifiersParams`:
/// it keeps `collect_world_events` clear of Bevy's 16-parameter limit, and
/// each new AI event source would otherwise eat one slot of that budget.
#[derive(bevy::ecs::system::SystemParam)]
pub struct AiEventReaders<'w, 's> {
    pub attacked: MessageReader<'w, 's, crate::ai::server::AiEntityAttacked>,
    pub destroyed: MessageReader<'w, 's, crate::ai::server::AiEntityDestroyed>,
    pub disabled: MessageReader<'w, 's, crate::ai::server::AiEntityDisabled>,
    pub waypoint_reached: MessageReader<'w, 's, crate::ai::server::AiWaypointReached>,
}

/// Bundle of system params used by the two trigger-dispatch sites for
/// the `add_faction_enemy` / `remove_faction_enemy` actions. Grouping
/// these keeps both `tick_trigger_pipeline` and `handle_respond_to_message`
/// under Bevy's per-system parameter cap (16).
///
/// `registry` is `Option<ResMut<_>>` so test apps that don't insert
/// `FactionRegistryResource` (most of `world::server::tests`) still load
/// the systems without a "resource does not exist" panic. Production
/// builds always insert the registry via `init_world_runtime`, so the
/// `None` branch is a test-only safety net that logs and skips the
/// action.
#[derive(bevy::ecs::system::SystemParam)]
pub struct FactionDispatchParams<'w, 's> {
    pub registry: Option<ResMut<'w, crate::entities::config_cache::FactionRegistryResource>>,
    pub non_ai_factions: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            &'static crate::entities::spawner::FactionComponent,
        ),
        Without<BehaviourSection>,
    >,
}

/// After a faction relationship is mutated, walk every AI controller and clear
/// its `TacticalRadarSelection` if the locked target's faction is no longer hostile to
/// the controller's own faction.
///
/// Required because `ai_target_selection`'s retention tier deliberately keeps an
/// established lock rather than re-deciding from scratch every tick — that
/// stickiness is what stops helm and weapons drifting onto different ships.
/// Retention only asks "is it alive and in radar range", never "is it still an
/// enemy", so a scenario that demotes a faction from hostile to neutral via
/// `remove_faction_enemy` would otherwise leave a ship engaging a now-friendly
/// target forever. Clearing the lock here drops it back to the tiers below,
/// which do consult the registry.
///
/// Post-#702 this clears `TacticalRadarSelection` — the ship's one authoritative lock —
/// rather than the private `ShipAiMemory.target` mirror it used to clear. That
/// mirror had already stopped being the firing path's input, so demoting a
/// faction stopped the ship *pursuing* its old enemy while it carried on
/// *shooting* it. One surface, one clear, both behaviours.
///
/// Controllers with no target, no faction, an unparseable target UUID, or a
/// target that has no faction (factionless entities like the starbase or an
/// asteroid) are left untouched.
pub(crate) fn revalidate_ai_targets_after_faction_change(
    ai_query: &mut Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    registry: &crate::ai::faction::FactionRegistry,
    uuid_to_faction: &std::collections::HashMap<uuid::Uuid, uuid::Uuid>,
) {
    for (_uid, weapons_target_opt, self_faction_comp) in ai_query.iter_mut() {
        let Some(mut weapons_target) = weapons_target_opt else {
            continue;
        };
        let Some(target_uuid) = weapons_target
            .0
            .as_deref()
            .and_then(|t| uuid::Uuid::parse_str(t).ok())
        else {
            continue;
        };
        let self_faction = self_faction_comp.map(|fc| fc.0);
        let target_faction = uuid_to_faction.get(&target_uuid).copied();
        if !crate::ai::faction::is_enemy(self_faction, target_faction, registry) {
            weapons_target.0 = None;
        }
    }
}

/// Addressed delivery reuses the ordinary dispatcher and applier. Resolution
/// happens in the enclosing exclusive command, so earlier instance mutations
/// are visible, and UUID iteration order is shared by every simulation peer.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn apply_addressed_action(
    In((ships, action, source_path, source_line, tick, origin_layer)): In<(
        Vec<String>,
        crate::world::config::TriggerAction,
        Option<String>,
        Option<usize>,
        u64,
        Option<String>,
    )>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut modifiers: ShipModifiersParams,
    mut factions: FactionDispatchParams,
    mut ai: Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    entities: Query<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>,
    mut layers: Option<ResMut<WorldLayerMap>>,
    mut balance: Option<ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>>,
    mut queues: EffectQueues,
) {
    let uuid_to_entity = entities
        .iter()
        .map(|(entity, uuid)| (uuid.0.clone(), entity))
        .collect();
    let mut events = Vec::new();
    for ship in ships {
        let mut names = runtime.name_to_uuid.clone();
        // Do not overwrite any authored name: it may also be a sound source or
        // AI target carried by the action.
        let mut alias = "addressed_recipient".to_string();
        while names.contains_key(&alias) {
            alias.push('_');
        }
        names.insert(alias.clone(), ship);
        let Ok(action) = crate::recipients::retarget_action(&action, &alias) else {
            return;
        };
        let layer_views = project_layer_views(layers.as_deref());
        let result = dispatch_action(
            &action,
            &DispatchContext {
                origin_layer: origin_layer.clone(),
                entity_name: None,
                name_to_uuid: &names,
                base_flags: &runtime.flags,
                layers: &layer_views,
                base_anchors: &HashMap::new(),
                factions: factions.registry.as_deref().map(|registry| &registry.0),
                uuid_source: &|| unreachable!("addressed actions cannot spawn entities"),
                template_loader: &crate::entities::loader::WasmTemplateLoader,
            },
        );
        apply_dispatch_result(
            result,
            "addressed action",
            &mut events,
            &uuid_to_entity,
            &mut runtime,
            &mut objectives,
            &mut commands,
            &mut modifiers,
            None,
            layers.as_deref_mut(),
            None,
            None,
            &mut factions,
            &mut ai,
            balance.as_deref_mut(),
            tick,
            source_path.as_deref(),
            source_line,
            &mut queues.out(),
        );
    }
    runtime.pending_world_events.extend(events);
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
