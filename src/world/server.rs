use bevy::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use rhai::{Map, AST};

use crate::lobby::{Target, WorldResource};
use crate::messages::{GamePhase, ServerMessage};
use crate::objectives::ObjectiveManager;
use crate::simulation::SimOutbox;
#[cfg(test)]
use crate::world::content::TriggerAction;
use crate::world::content::{TriggerState, WorldEvent};
use crate::world::delayed::{partition_delayed_actions, DelayedAction};
use crate::world::dispatch::{
    dispatch_action, ActionCmd, DispatchContext, DispatchResult, LayerView,
    WORLD_MODIFIER_SOURCE_ID,
};
use crate::world::layers::{evaluate_layer_load, LayerLoadOutcome};
use crate::world::scenario::evaluate_scenario_load;
use crate::world::script::effects::BufferedEffect;
use crate::world::script::engine::{RuntimeHost, ScriptTrigger};
use crate::world::script::schedule::{PendingCallbacks, SchedClock, TickBudget};

// -- Resources --------------------------------------------------------------

/// Server-side runtime state for the currently active world content.
///
/// Populated at `Startup` from the unified `WorldConfig` resource (which is
/// inserted by `insert_world_config_resource` when the JS bridge has called
/// `wasm_load_world`). When no world is loaded all vecs/maps are empty and
/// the trigger systems are no-ops. The comms half of this state (template /
/// dialogue / contact / range tracking) lives in
/// `crate::comms::server::CommsRuntime` (issue #816).
#[derive(Resource, Default)]
pub struct WorldContentRuntime {
    /// Mutable per-trigger runtime state (fired flag).
    pub trigger_states: Vec<TriggerState>,
    /// Named-entity → UUID mapping (populated from `WorldConfig.name_to_uuid`).
    pub name_to_uuid: HashMap<String, String>,
    /// Paths of world TOML files already merged into this runtime, used to
    /// de-duplicate additive world loads (no-op if path already active).
    pub loaded_scenario_paths: HashSet<String>,
    /// World flag / counter store consumed by predicate-gated triggers
    /// (`when = "..."`) and mutated by `set_flag` / `clear_flag` /
    /// `increment_flag` / `set_flag_value` trigger actions. Mutations are
    /// observed inside `tick_trigger_pipeline`, which emits `FlagSet` /
    /// `FlagCleared` `WorldEvent`s on transitions and re-evaluates the
    /// trigger table in the same tick so chained `on_flag_set` /
    /// `on_flag_cleared` triggers fire as part of the same Bevy frame.
    pub flags: crate::world::flags::FlagStore,
    /// Queue of synthesised `WorldEvent`s to be drained by `collect_world_events`
    /// into `WorldEventBuffer` on the next Update tick. Used by
    /// `init_world_runtime` (base-world Startup) and
    /// `apply_world_layer_changes` (sub-world Load) to inject
    /// `WorldEvent::WorldLoaded` into the trigger evaluation pipeline
    /// without duplicating the dispatch logic that lives inside
    /// `tick_trigger_pipeline`.
    pub pending_world_events: Vec<WorldEvent>,
    /// Last aggregate hull fraction observed for every live entity. A damaged
    /// entity only emits an event on a downward crossing; healing never re-arms
    /// a scenario's single-shot `on_hull_below` templates.
    pub observed_hull_fractions: HashMap<String, f32>,
    /// Time zero for the mission clock: the `Time::elapsed_secs()` reading taken
    /// on the first simulation tick of `GamePhase::InProgress`. `on_timer`
    /// triggers fire when `time.elapsed_secs() - mission_clock_anchor_secs >=
    /// after_secs`, and `action_delays` schedule `fire_at_elapsed` against the
    /// same origin. (#475, re-anchored in #960)
    ///
    /// `None` means "not anchored yet" — no world loaded at all, or a mission
    /// that has not started. `collect_world_events` emits no `TimerElapsed`
    /// while it is `None`, and `tick_delayed_actions` dispatches nothing.
    ///
    /// **Written by [`anchor_mission_clock`], cleared by [`arm_mission_clock`],
    /// and by nothing else.** It is deliberately NOT stamped at `Startup`: the
    /// world is loaded then, but `Time` keeps running through the whole lobby,
    /// so a boot anchor made `after_secs` an offset from app launch rather than
    /// from mission start — see [`anchor_mission_clock`] for what that cost.
    pub mission_clock_anchor_secs: Option<f32>,
    /// Maps named groups to the set of entity names currently in that group.
    pub entity_groups: HashMap<String, HashSet<String>>,
    /// Actions queued for deferred dispatch (via `action_delays` on triggers).
    pub pending_delayed_actions: Vec<DelayedAction>,
    /// Named mission deadlines (issue #1024): the live state of every
    /// `[[deadline]]` the world authored.
    ///
    /// A *record*, not a queue — read [`crate::world::deadlines`] before adding
    /// anything to it. Each deadline's firing is one ordinary `ScheduledCall` on
    /// [`WorldScriptRuntime::pending_callbacks`], drained by the callback system
    /// that already exists; what lives here is the name, label, visibility and
    /// mutable due tick that the raw `(fire_tick, script_path, fn_name)` key
    /// cannot carry. Armed once by [`arm_mission_deadlines`] on the first
    /// simulation tick of the mission — the same tick [`anchor_mission_clock`]
    /// stamps, so `due_secs` measures from mission start rather than from boot.
    ///
    /// It sits on this resource rather than becoming a resource of its own so
    /// every site that already borrows the content runtime to apply a call's
    /// effects can apply its deadline mutations too, and so the state census
    /// (`tests/authoritative_state_enumeration.rs`) sees no new registration —
    /// the same shape `WorldScriptRuntime::pending_callbacks` has.
    pub deadlines: crate::world::deadlines::DeadlineTable,
    /// The promises this run has made (issue #1029): every
    /// `ctx.commitments.record(…)` a dialogue beat wrote, and whether each ended
    /// up kept or broken.
    ///
    /// A pure record with no queue and no evaluator — read
    /// [`crate::world::commitments`] before adding anything to it. Nothing arms
    /// it, nothing scans it, and no system this slice adds runs per tick: a
    /// promise is written when a script says so and settled when a script says
    /// so, and the campaign flag a resolution writes travels as an ordinary
    /// `MutateFlag` on the effect buffer that already existed.
    ///
    /// It sits on this resource beside `deadlines`, and for the same reason:
    /// every site that already borrows the content runtime to apply a call's
    /// effects can apply its commitment mutations too, and the state census
    /// (`tests/authoritative_state_enumeration.rs`) sees no new registration.
    pub commitments: crate::world::commitments::CommitmentLedger,
    /// Infrastructure condition adjustments queued this tick by a scripted
    /// `repair_infrastructure` / `damage_infrastructure` effect (issue #1025),
    /// already resolved to the target's UUID.
    ///
    /// Drained every tick by
    /// [`crate::infrastructure::tick_infrastructure_condition`] in
    /// `SimSet::Modifiers`, so it is empty at every tick boundary — including
    /// the one a snapshot is taken at. It exists so that the one system that
    /// owns threshold edges is also the one system that moves condition:
    /// an effect writing the component directly would cross a threshold with
    /// nobody listening, and the world flag store would never hear about it.
    pub pending_condition_adjustments: Vec<crate::infrastructure::ConditionAdjustment>,
    /// Named infrastructure capacities a completed `transfer` moved this tick
    /// (issue #1027), already resolved to each end's UUID.
    ///
    /// Drained by the same
    /// [`crate::infrastructure::tick_infrastructure_condition`] on the same
    /// terms and for the same reason as `pending_condition_adjustments` above:
    /// that system is the one place a structure's published numbers move, so it
    /// is the one place that can re-publish the counter a scenario predicate
    /// reads. A transfer writing the component itself would move the goods and
    /// leave every `counter(depot_transfer_throughput)` test reading the figure
    /// the depot was authored with.
    pub pending_capacity_adjustments: Vec<crate::infrastructure::CapacityAdjustment>,
    /// External operations a scripted effect asked to start this tick (issue
    /// #1026), already resolved to the performing ship's and the target's UUIDs.
    ///
    /// Drained every tick by [`crate::operations::tick_operations`] in
    /// `SimSet::Modifiers`, so it is empty at every tick boundary — including
    /// the one a snapshot is taken at. Queued rather than applied where it is
    /// authored for `pending_condition_adjustments`' reason turned around:
    /// the applier holds `name_to_uuid` and nothing else, while opening a hold
    /// needs the ship's capability table, its power grid and the target's
    /// position.
    pub pending_operation_starts: Vec<crate::operations::PendingOperationStart>,
    /// Civilian orders queued this tick by a scripted `order_civilian_*` effect
    /// (issue #1028), already resolved to the target's UUID.
    ///
    /// Drained every tick by [`crate::civilian::tick_civilian_traffic`] in
    /// `SimSet::Input`, so it is empty at every tick boundary — including the
    /// one a snapshot is taken at. It exists for the same reason the condition
    /// queue above does: one system owns the compliance state machine, and an
    /// effect writing the component directly would skip the acknowledgement the
    /// crew is meant to watch for.
    pub pending_civilian_orders: Vec<crate::civilian::PendingCivilianOrder>,
}

/// Bevy resource wrapping the server-side objective manager.
#[derive(Resource, Default)]
pub struct ObjectiveManagerRes(pub ObjectiveManager);

/// Queue of world TOML paths to load additively into the live `WorldContentRuntime`.
///
/// The `apply_pending_scenario_loads` system drains it each frame, parses the
/// TOML, and merges the new triggers/comms into the runtime. (Currently no
/// trigger action enqueues into this; retained as the merge-side plumbing.)
#[derive(Resource, Default)]
pub struct PendingScenarioLoad(pub Vec<String>);

/// Serialisable runtime snapshot for one additively-loaded sub-world.
///
/// Keyed by world TOML path in `WorldLayerMap`. Tracks the ECS entity handles
/// spawned from the sub-world's `[[entity]]` blocks so they can be despawned
/// when `UnloadWorld` fires, plus the anchors and flag store those entities
/// resolve against.
///
/// It also snapshotted the trigger states the layer contributed, so `UnloadWorld`
/// could take exactly them back out. Issue #985 deleted the `[[trigger]]` parser
/// — the only way a layer could author one — so there is nothing to snapshot
/// until script-in-layers (#1045).
#[derive(Clone, Debug, Default)]
pub struct WorldRuntime {
    /// ECS entity handles spawned when this layer was loaded.
    pub spawned_entities: Vec<Entity>,
    /// Anchor table from the layer's `WorldConfig`. Used by `spawn_entity`
    /// trigger actions (issue #417) to resolve `anchor = "..."` action
    /// fields when this layer authored the trigger.
    pub anchors: HashMap<String, [f32; 3]>,
    /// Per-layer world flag store (PRD #397 fix 1). Mutations from this
    /// layer's triggers default to this store; `parent:` prefixes on
    /// flag-mutation actions walk up via `loader_path`.
    pub flags: crate::world::flags::FlagStore,
    /// Path of the layer whose trigger called `LoadWorld(path)` to bring
    /// this layer in. `None` = loaded at startup (base world's
    /// `extra_worlds`) ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â the loader is the base world itself, so
    /// `parent:` from this layer walks straight to the base
    /// `WorldContentRuntime.flags` store.
    pub loader_path: Option<String>,
    /// Objective ids added by this layer's triggers (issue #751). Recorded as
    /// `AddObjective` commands are applied so `UnloadWorld` removes exactly
    /// this layer's objectives from the shared `ObjectiveManager`.
    pub owned_objective_ids: Vec<String>,
    /// Authored policy for this layer's in-flight delayed actions on unload
    /// (issue #751). `true` = resolve (dispatch immediately), `false` =
    /// cancel (drop). Snapshotted from the layer's `WorldConfig` at load.
    pub delayed_unload_resolve: bool,
}

/// Map of `path ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ WorldRuntime` for sub-worlds loaded via `LoadWorld` / `extra_worlds`.
///
/// Each entry is keyed by the world TOML path so `UnloadWorld` can remove it by
/// the same path. Stored as a Bevy `Resource`; an empty map is the initial state.
#[derive(Resource, Default)]
pub struct WorldLayerMap(pub HashMap<String, WorldRuntime>);

/// Marker component recording which loaded world layer spawned this entity
/// (perf fix, issue #891 review finding 1). Stamped exactly once, at the two
/// sites that add an entity to a `WorldRuntime::spawned_entities` list — the
/// `SpawnEntity` trigger action and the bulk layer-load spawn in
/// `apply_world_layer_changes` — so [`entity_flag_chain`] can read a ship's
/// origin layer in O(1) (a `Query::get`) instead of the O(layers) scan
/// `entity_origin_layer` used to run on every call, including per-claim
/// inside `handle_torpedo_magazine_inter_system`.
///
/// Absent on a base-world (or otherwise unrecorded) entity — exactly the
/// entities the old scan resolved to `None` — so a missing component keeps
/// meaning "anchored at the base world", not "not spawned yet".
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct EntityOriginLayer(pub String);

/// The flag-store-only half of the layered walk (PRD #397 fix 1, split out by
/// the issue #891 review finding 2): `chain[0]` is the origin layer's own
/// store, each `loader_path` hop appends the next-outer layer, and the base
/// `WorldContentRuntime` store (`base_flags`) terminates the chain. A `parent:`
/// prefix on a flag name steps one entry outward
/// (`crate::world::flags::resolve_chain`). An origin naming a layer missing
/// from the map degrades to the base store alone (shouldn't happen in normal
/// flow).
///
/// Shared by [`entity_flag_chain`] (every AI policy/selector host) and by
/// [`layered_flag_chain_with_paths`] (`tick_trigger_pipeline`). It used to
/// also build a parallel `Vec<Option<String>>` layer-path chain with a
/// `String` clone per hop — but only the trigger pipeline ever read that
/// half; every AI host discarded it on every call, so pairing the two meant a
/// throwaway allocation on every AI decision this crate makes. That half now
/// lives only in `layered_flag_chain_with_paths`, the one reader that wants it.
pub fn layered_flag_chain<'a>(
    origin: Option<&str>,
    base_flags: &'a crate::world::flags::FlagStore,
    layer_map: Option<&'a WorldLayerMap>,
) -> Vec<&'a crate::world::flags::FlagStore> {
    let mut flag_chain: Vec<&crate::world::flags::FlagStore> = Vec::new();
    let mut cur = origin;
    loop {
        match cur {
            Some(p) => {
                if let Some(wr) = layer_map.and_then(|lm| lm.0.get(p)) {
                    flag_chain.push(&wr.flags);
                    cur = wr.loader_path.as_deref();
                } else {
                    // Layer missing from the map — treat as empty.
                    // (Shouldn't happen in normal flow.)
                    flag_chain.push(base_flags);
                    break;
                }
            }
            None => {
                flag_chain.push(base_flags);
                break;
            }
        }
    }
    flag_chain
}

/// `tick_trigger_pipeline`'s own wrapper around [`layered_flag_chain`] (issue
/// #891 review finding 2): the SAME store walk, plus the layer-PATH chain
/// `evaluate_single_trigger` / `DispatchContext` need to resolve `parent:`
/// against the right outer layer and to stamp `origin_layer` on dispatched
/// actions. No other reader wants the path chain, so it is derived here
/// rather than threaded through the shared walk every caller pays for.
pub fn layered_flag_chain_with_paths<'a>(
    origin: Option<&str>,
    base_flags: &'a crate::world::flags::FlagStore,
    layer_map: Option<&'a WorldLayerMap>,
) -> (Vec<&'a crate::world::flags::FlagStore>, Vec<Option<String>>) {
    let flag_chain = layered_flag_chain(origin, base_flags, layer_map);
    let mut layer_chain: Vec<Option<String>> = Vec::new();
    let mut cur = origin.map(str::to_string);
    loop {
        layer_chain.push(cur.clone());
        match &cur {
            Some(p) => match layer_map.and_then(|lm| lm.0.get(p)) {
                Some(wr) => cur = wr.loader_path.clone(),
                None => break,
            },
            None => break,
        }
    }
    (flag_chain, layer_chain)
}

/// The world-flag chain one entity's AI policy/selector guards evaluate against
/// (issue #891 stage 2): anchored at the layer that spawned the entity and
/// climbing `loader_path` to the base store, exactly as a trigger authored in
/// that layer reads. A base-world (or unrecorded) entity reads the base store
/// alone; `parent:` prefixes climb outward from wherever the entity is
/// anchored.
///
/// `origin` is the entity's own [`EntityOriginLayer`] component (read by the
/// caller via a `Query`; `None` for a base-world entity — an O(1) read since
/// the issue #891 review perf fix, replacing a `WorldLayerMap` scan).
/// `runtime` is `Option` because every AI host takes `Option<Res<_>>` for
/// bare-`App` fixtures: absent, the chain is empty and `flag()`/`counter()`
/// guards read false.
pub fn entity_flag_chain<'a>(
    origin: Option<&EntityOriginLayer>,
    runtime: Option<&'a WorldContentRuntime>,
    layer_map: Option<&'a WorldLayerMap>,
) -> Vec<&'a crate::world::flags::FlagStore> {
    match runtime {
        Some(rt) => layered_flag_chain(origin.map(|o| o.0.as_str()), &rt.flags, layer_map),
        None => Vec::new(),
    }
}

/// Queue of `LoadWorld` / `UnloadWorld` actions to execute on the next frame.
///
/// `tick_trigger_pipeline` pushes path-keyed commands here; `apply_world_layer_changes`
/// drains it and mutates `WorldLayerMap` + `WorldContentRuntime` accordingly.
#[derive(Resource, Default)]
pub struct PendingWorldLayerChanges(pub Vec<WorldLayerChange>);

/// A single pending world-layer command.
#[derive(Clone, Debug)]
pub enum WorldLayerChange {
    /// Load a sub-world. `loader_path` is the layer whose trigger called
    /// `LoadWorld(path)` to enqueue this ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â `None` for startup-time loads
    /// (base world's `extra_worlds`). Recorded on the new
    /// `WorldRuntime.loader_path` so `parent:` walks from the loaded
    /// layer reach the right outer flag store (PRD #397 fix 1).
    Load {
        path: String,
        loader_path: Option<String>,
    },
    Unload(String),
}

/// Per-tick buffer of the externally-sourced `WorldEvent`s observed this tick.
///
/// Producer: `collect_world_events` (drains the `AiEventReaders` messages and
/// `WorldContentRuntime::pending_world_events`, and synthesises the per-tick
/// `TimerElapsed` event). Consumer: `tick_trigger_pipeline`, which seeds its
/// trigger-chaining loop from it. `inject_comms_templates` was the second
/// consumer until issue #985 deleted the `[[comms]]` front-end it fired.
///
/// The chaining loop's internally-produced events (`FlagSet`, `FlagCleared`,
/// `Destroyed` from a `DestroyEntity` action) stay LOCAL to the pipeline and
/// are never written here, so the buffer stays what its name says: the
/// EXTERNALLY-sourced events of this tick. Contents are valid for one tick:
/// `collect_world_events` rebuilds the buffer every run, so stale events
/// never leak into the next tick.
#[derive(Resource, Default)]
pub struct WorldEventBuffer(pub Vec<WorldEvent>);

// ── Scripting seam (issue #984, Rhai M6 phase 2a) ───────────────────────────

/// The raw world source a session loaded: its path plus the world TOML as a
/// `Value`.
///
/// `WorldConfig` drops the raw `[script]` / `script` keys the Rhai loader needs,
/// so this carries the whole TOML alongside its path. Populated on both
/// targets — headless inserts it directly in `build_headless_app`, the browser
/// via `insert_raw_world_source_resource` reading `server::bridge::
/// get_raw_world_source()` at `Startup` — and read once by
/// `compile_world_scripts`.
///
/// "Raw" is about SHAPE, not provenance: unparsed TOML with nothing dropped,
/// which is not the same as untouched. On a harnessed duel run
/// (`--side-a`/`--side-b`) what lands here is the world TOML *after*
/// `headless::duel::apply_duel_sides` has regenerated the slot roster inside its
/// `[script]` source — deliberately, since this resource is what
/// `compile_world_scripts` compiles.
#[derive(Resource, Clone, Debug)]
pub struct RawWorldSource {
    /// The world TOML's path (its content-ledger / snapshot-boundary key).
    pub path: String,
    /// The world TOML as loaded — after any headless duel-side transform — still
    /// carrying any `[script]` / `script` key.
    pub toml: toml::Value,
}

/// A reference to the script handler fn that supplies one scripted trigger's
/// effects at runtime.
///
/// Held in [`WorldScriptRuntime::handlers`] parallel to
/// `WorldContentRuntime.trigger_states`. `(script_path, fn_name)` is everything
/// `RuntimeHost::call` needs to resolve the fn against the right unit's AST.
#[derive(Clone, Debug)]
pub struct ScriptHandlerRef {
    /// Content-relative (or virtual) path of the unit whose AST defines the fn.
    pub script_path: String,
    /// The handler fn to call when the trigger fires.
    pub fn_name: String,
}

/// Runtime state for a world that authors Rhai scripts (issue #984, Rhai M6
/// phase 2a).
///
/// Inserted at `Startup` by [`compile_world_scripts`] ONLY when the loaded world
/// compiled at least one script AST with no error; **absent** for every
/// script-free world (the entire shipped set), so the scripted-handler branch of
/// [`tick_trigger_pipeline`] is skipped and behaviour there is byte-identical to
/// before scripting existed. Its lifecycle mirrors [`WorldContentRuntime`]:
/// created at world load, persisting for the session.
///
/// [`pending_callbacks`](Self::pending_callbacks) is the serialisable
/// future-work queue that deferred `after(n, |ctx| …)` callbacks land on
/// (issue #984, Rhai M6 phase 2b): [`tick_trigger_pipeline`]'s scripted
/// handlers and [`tick_script_callbacks`]'s own callbacks EXTEND it, and
/// [`tick_script_callbacks`] drains the due entries each tick.
#[derive(Resource)]
pub struct WorldScriptRuntime {
    /// The runtime host that runs retained handler fns.
    pub host: RuntimeHost,
    /// Retained ASTs keyed by content-relative (or virtual) path.
    pub asts: BTreeMap<String, AST>,
    /// The compiled script triggers, consumed once by [`init_world_runtime`]'s
    /// merge ([`merge_script_triggers`]) to append trigger states and build
    /// [`handlers`](Self::handlers).
    pub triggers: Vec<ScriptTrigger>,
    /// Parallel to `WorldContentRuntime.trigger_states`: `None` for every
    /// declarative trigger index, `Some` for each appended scripted one. Filled
    /// by [`init_world_runtime`]; empty until the merge runs.
    pub handlers: Vec<Option<ScriptHandlerRef>>,
    /// The per-tick operation/call budget, shared across every script call in a
    /// tick and reset when [`budget_tick`](Self::budget_tick) falls behind the
    /// current `SimTick`.
    pub budget: TickBudget,
    /// The `SimTick` the current [`budget`](Self::budget) was created for.
    pub budget_tick: u64,
    /// Content hash of the compiled script set (the #988 save-binding input).
    pub content_hash: u64,
    /// Serialisable queue of deferred `after(n, |ctx| …)` callbacks awaiting
    /// their fire tick (issue #984, Rhai M6 phase 2b). Populated by every
    /// scripted handler / callback that schedules one, drained in authored
    /// order by [`tick_script_callbacks`] once `now_tick >= fire_tick`. Live
    /// authoritative future work — it belongs in the same digest fold as
    /// [`WorldContentRuntime`]'s own deferred state.
    pub pending_callbacks: PendingCallbacks,
    /// Queue of scripted `ctx.effects.open_comms(#{…})` requests awaiting a comms
    /// system to materialise them into threads (issue #984). Sibling of
    /// [`pending_callbacks`](Self::pending_callbacks) and populated the same way:
    /// every script call site extends it with its [`CallEffects`] `comms_opens`.
    ///
    /// It lives here, not on a comms resource, for the same reason the callback
    /// queue does — this is the resource the script systems already borrow, so
    /// routing costs one line per call site and a script-free world (every shipped
    /// one) has no `WorldScriptRuntime` at all, hence no queue and no behaviour.
    /// The request itself is comms vocabulary from `comms::content`, keeping the
    /// #816 split intact: world runtime holds the strings, the comms module owns
    /// what they mean.
    pub pending_comms_opens: Vec<crate::comms::content::OpenCommsRequest>,
    /// `on_deadline("id", "handler")` declarations collected at load (issue
    /// #1024), pairing each authored `[[deadline]]` with the fn it runs and the
    /// unit that said so. Read once, by [`arm_mission_deadlines`]; the unit path
    /// is half of the `ScheduledCall` key each deadline arms with.
    pub deadline_handlers: Vec<crate::world::deadlines::DeadlineHandler>,
}

/// The two script-related reads [`tick_trigger_pipeline`] needs, bundled into a
/// single [`SystemParam`] so the pipeline stays under Bevy's 16-parameter limit.
///
/// Both `Option` so every bare-`App` fixture (and every script-free world) takes
/// the `None` arm and the scripted-handler branch is a no-op.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ScriptRuntimeParams<'w> {
    pub runtime: Option<ResMut<'w, WorldScriptRuntime>>,
    pub sim_tick: Option<Res<'w, crate::sim_tick::SimTick>>,
}

/// Set fresh by [`compile_world_scripts`] at every world load: `true` when the
/// world's scripts failed to compile/validate. Read by
/// [`world_activation_blocked`] so a script-error world spawns zero entities,
/// atomically with the composition gate.
///
/// A module-level `AtomicBool` static — NOT a Bevy resource — so the two
/// `Startup` spawn systems' access sets (and therefore their scheduling) are
/// untouched: making the gate a resource the spawn systems must borrow would add
/// ordering edges and perturb Startup determinism for the entire script-free
/// shipped set. The static keeps the flag out of the ECS access set entirely.
///
/// It must be thread-*safe*, not merely a `thread_local!`: `compile_world_scripts`
/// and the spawn systems run on Bevy's multithreaded native executor and can land
/// on different worker threads, so a `true` written on worker A would be invisible
/// to a `thread_local!` read on worker B — the gate could read `false` and spawn
/// despite a script error, and (worse) the spawn decision would become
/// non-deterministic across lockstep peers (which worker runs which system). The
/// atomic makes the write visible across workers. The `.chain()` ordering in
/// `WorldPlugin` (with finding-1's matching `.after` on `setup_world`) sequences
/// `compile_world_scripts` before both spawn systems within the Startup run, and
/// the `Release`/`Acquire` pairing publishes that write to the reads.
///
/// `compile_world_scripts` writes it `false` UNCONDITIONALLY at the top of every
/// world load (before the `script`-key check), so a script-free world — and any
/// app, e.g. a bare-`App` fixture, that never runs the system — reads `false`.
static SCRIPT_ACTIVATION_BLOCKED: AtomicBool = AtomicBool::new(false);

/// Record whether the just-loaded world's scripts blocked activation. Written
/// only by [`compile_world_scripts`], once per load. `Release` so the write is
/// published to the `Acquire` read in [`script_activation_blocked`] on any worker.
fn set_script_activation_blocked(blocked: bool) {
    SCRIPT_ACTIVATION_BLOCKED.store(blocked, Ordering::Release);
}

/// Whether the current world's scripts blocked activation (see
/// [`SCRIPT_ACTIVATION_BLOCKED`]). `Acquire` to observe `compile_world_scripts`'
/// `Release` write across worker threads.
fn script_activation_blocked() -> bool {
    SCRIPT_ACTIVATION_BLOCKED.load(Ordering::Acquire)
}

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        // The comms half of the pre-#816 WorldPlugin lives in
        // `CommsWorldPlugin`. Added here so every app that installs the
        // world also gets comms, and so the cross-plugin ordering
        // constraints (`init_comms_runtime` after `init_world_runtime`;
        // `open_scripted_comms_threads` between `tick_script_callbacks` and
        // `tick_delayed_actions`; `broadcast_objective_summary` after
        // `broadcast_comms_state`) all resolve against systems guaranteed to be
        // registered.
        // Infrastructure condition (issue #1025) is added here for the same
        // reason comms is: it writes `WorldContentRuntime`'s flag store and
        // world-event queue, so it has no meaning in an app that has no world.
        // External operations (issue #1026) join for the same reason again: a
        // script start arrives on `WorldContentRuntime`'s queue and a completion
        // pays into the infrastructure queue beside it, so the plugin is
        // meaningless in an app with no world — and its tick is explicitly
        // ordered before `InfrastructurePlugin`'s, which needs both registered.
        // Civilian traffic (issue #1028) is added here for the same reason:
        // routes are world data, the order queue is a field on
        // `WorldContentRuntime`, and a dock target is resolved through its name
        // table — none of which exist in an app with no world.
        app.add_plugins(crate::comms::CommsWorldPlugin)
            .add_plugins(crate::infrastructure::InfrastructurePlugin)
            .add_plugins(crate::operations::OperationsPlugin)
            .add_plugins(crate::civilian::CivilianPlugin)
            // Dossiers (issue #1030) join them for the same reason: the
            // commitments a fact sheet lists are a field on
            // `WorldContentRuntime`, the comms standing it reports is
            // `CommsRuntime`'s, and every subject on its roster came out of a
            // world file. The plugin registers a publisher and nothing else.
            .add_plugins(crate::dossier::DossierPlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<PendingScenarioLoad>()
            .init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .init_resource::<WorldEventBuffer>()
            .add_systems(
                Startup,
                (
                    insert_world_config_resource,
                    // The scripting seam (issue #984, Rhai M6 phase 2a): source
                    // the raw world TOML and compile its scripts BEFORE the
                    // spawn pass (so a script error blocks it, atomically with
                    // the composition gate) and before `init_world_runtime` (so
                    // its trigger-merge sees the compiled `ScriptTrigger`s).
                    insert_raw_world_source_resource,
                    compile_world_scripts,
                    spawn_world_entities,
                    init_world_runtime,
                    load_extra_worlds,
                )
                    .chain(),
            )
            .add_systems(
                FixedUpdate,
                broadcast_objective_summary
                    .in_set(crate::sim_sets::SimSet::Broadcast)
                    .after(crate::comms::server::broadcast_comms_state),
            )
            // The comms half of the Physics set is registered by
            // `CommsWorldPlugin`, ordered against these systems from that side.
            // It was a four-system `.chain()` (#718/#719) until issue #985
            // deleted `tick_pending_follow_ups` and `inject_comms_templates`;
            // `open_scripted_comms_threads` is what remains, and it sits after
            // the callback drain rather than around the event collector.
            // The mission clock (#960). `SimSet::Physics` is gated on
            // `GamePhase::InProgress`, so the first run of
            // `anchor_mission_clock` is the first simulation tick of the
            // mission; `arm_mission_clock` re-opens it for a second round.
            .add_systems(OnEnter(GamePhase::InProgress), arm_mission_clock)
            .add_systems(
                FixedUpdate,
                (
                    anchor_mission_clock,
                    // Immediately after the anchor and before anything reads the
                    // table: a `[[deadline]]` is due N seconds into the MISSION,
                    // so it is keyed off the same first-InProgress-tick moment
                    // (issue #1024). Runs its body exactly once per mission.
                    arm_mission_deadlines,
                    collect_world_events,
                    tick_trigger_pipeline,
                )
                    .chain()
                    .in_set(crate::sim_sets::SimSet::Physics),
            )
            // Cursor advancement is a `Modifiers` evaluator: `Physics` has
            // finished moving every ship by then, so waypoint arrival is
            // judged against this tick's final positions. It emits
            // `AiWaypointReached`, which `collect_world_events` turns into a
            // `WorldEvent::WaypointReached` on the next tick (the same
            // one-tick event bridge `AiEntityAttacked` already uses).
            .add_systems(
                FixedUpdate,
                crate::ai_plugin::advance_objective_cursors
                    .in_set(crate::sim_sets::SimSet::Modifiers),
            )
            // The scripted-callback drain (issue #984, Rhai M6 phase 2b):
            // `after(n, |ctx| …)` callbacks that scripted handlers scheduled are
            // drained here once due. Ordered AFTER `tick_trigger_pipeline` (so it
            // shares that system's per-tick budget reset, and sees this tick's
            // freshly-scheduled callbacks) and BEFORE `tick_delayed_actions` (so a
            // callback's own `in_seconds` effect reaches the delayed queue in the
            // same tick a trigger's would). A no-op for every script-free world:
            // no `WorldScriptRuntime` → early return before any `DerefMut`.
            .add_systems(
                FixedUpdate,
                tick_script_callbacks
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(tick_trigger_pipeline)
                    .before(tick_delayed_actions),
            )
            .add_systems(
                FixedUpdate,
                tick_delayed_actions
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(tick_trigger_pipeline),
            )
            .add_systems(
                FixedUpdate,
                apply_pending_scenario_loads.in_set(crate::sim_sets::SimSet::Physics),
            )
            .add_systems(
                FixedUpdate,
                apply_world_layer_changes.in_set(crate::sim_sets::SimSet::Physics),
            )
            .add_observer(handle_region_entered_event)
            .add_observer(handle_region_exited_event);
    }
}

/// Observer: bridge `RegionEntered` (player ship boundary crossing into a
/// region) into a queued `WorldEvent::EnteredRegion` so `collect_world_events`
/// can buffer it for the trigger pipeline on the next tick.
///
/// Looks up the region entity's UUID via `RegionMembership.region_uuids`
/// (populated each tick by `update_region_membership`, and persisted after
/// the entity despawns). Drops the event silently if no UUID is cached
/// (e.g. a region entity spawned without an `EntityUuid` component ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â not
/// expected in production paths but possible in narrow unit tests).
///
/// Single-fire-per-transition is provided by the region containment
/// system itself: `update_region_membership` uses set differences between
/// the previous and current "inside" sets, so it only triggers the
/// observer event once per boundary crossing. Staying inside on the next
/// tick produces no further `RegionEntered` events.
///
/// After PRD #597 PR 9, `update_region_membership` tracks region membership
/// for every ship (player + NPCs). World-scenario triggers, however, remain
/// player-driven: only crossings by the `LocalShip` are bridged into
/// `pending_world_events`.
fn handle_region_entered_event(
    trigger: On<crate::regions::server::RegionEntered>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
    local_ship_q: Query<(), With<crate::simulation::LocalShip>>,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    // World triggers fire only on player-ship boundary crossings; NPC ships
    // (also tracked in RegionMembership after PRD #597 PR 9) are silently
    // dropped here â€” they still receive region effects via the other
    // observers/systems.
    if local_ship_q.get(ev.subject).is_err() {
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
/// region entity is despawned while the ship is inside. Filters on
/// `LocalShip` for the same reason: world-scenario triggers are player-driven.
fn handle_region_exited_event(
    trigger: On<crate::regions::server::RegionExited>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    runtime: Option<ResMut<WorldContentRuntime>>,
    local_ship_q: Query<(), With<crate::simulation::LocalShip>>,
) {
    let (Some(membership), Some(mut runtime)) = (membership, runtime) else {
        return;
    };
    let ev = trigger.event();
    if local_ship_q.get(ev.subject).is_err() {
        return;
    }
    let Some(uuid) = membership.region_uuids.get(&ev.region_entity).cloned() else {
        return;
    };
    runtime
        .pending_world_events
        .push(WorldEvent::ExitedRegion { uuid });
}

/// Startup system: copy the unified `WorldConfig` from the WASM-side
/// thread-local cache into a Bevy `Resource` so downstream systems
/// (`spawn_world_entities`, `ai::server::tick_ai_controllers`) can read it
/// via `Res<WorldConfig>`.
///
/// On native (no WASM bridge) `get_world_config()` returns `None` and this
/// system is a no-op; downstream systems that iterate world entities
/// simply see an empty world (native unit tests only â€” production always
/// loads a world TOML through the WASM bridge).
pub(crate) fn insert_world_config_resource(mut commands: Commands) {
    if let Some(world_config) = crate::config_cache::get_world_config() {
        commands.insert_resource(world_config);
    }
}

/// `Startup` system: populate [`RawWorldSource`] from the browser bridge's
/// stashed raw world TOML (issue #984, Rhai M6 phase 2a).
///
/// The script-loader's twin of [`insert_world_config_resource`]: `WorldConfig`
/// has dropped the raw `[script]` / `script` keys, so the Rhai loader needs the
/// untouched TOML text. On native `get_raw_world_source()` has no equivalent
/// (headless inserts `RawWorldSource` directly in `build_headless_app`), so this
/// system only ever inserts on wasm; on native it is a no-op and never disturbs
/// the resource the headless path already inserted.
#[cfg_attr(
    not(all(target_arch = "wasm32", feature = "server")),
    allow(unused_mut, unused_variables)
)]
pub(crate) fn insert_raw_world_source_resource(mut commands: Commands) {
    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    if let Some((path, toml_str)) = crate::server::bridge::get_raw_world_source() {
        // `toml::Value`'s `FromStr` parses a single VALUE EXPRESSION (`1`,
        // `"x"`, `[1, 2]`), not a document — so it rejects every world file
        // ("unexpected content, expected nothing"). This wasm-only arm shipped
        // with that misuse in #984 P2a and stayed invisible while every world
        // was script-free: the error logged, and there were no scripts to
        // lose. `toml::from_str` is the document parser the rest of the crate
        // uses (and the same route `parse_world` takes, which is why the world
        // itself loaded while its scripts vanished).
        match toml::from_str::<toml::Value>(&toml_str) {
            Ok(toml) => commands.insert_resource(RawWorldSource { path, toml }),
            Err(e) => bevy::log::error!(
                target: "world",
                "insert_raw_world_source_resource: world TOML at {path} failed to re-parse: {e}"
            ),
        }
    }
}

/// `Startup` system: compile the loaded world's Rhai scripts and insert the
/// [`WorldScriptRuntime`] (issue #984, Rhai M6 phase 2a).
///
/// Ordered after [`insert_world_config_resource`] /
/// [`insert_raw_world_source_resource`] and before `spawn_world_entities` /
/// [`init_world_runtime`]. A world with no `script` key short-circuits before
/// building any engine, so a script-free world is a true no-op — no
/// `WorldScriptRuntime`, no content-ledger records, nothing that could move a
/// digest.
///
/// Findings fold into the SAME atomic activation gate the composition findings
/// use: on a script error this records the block (read by
/// [`world_activation_blocked`]) so a script-error world spawns zero entities,
/// and inserts no runtime. Headless additionally hard-fails the build for a
/// script error (see `build_headless_app`), so a broken script never reaches a
/// running authoritative host.
pub(crate) fn compile_world_scripts(mut commands: Commands, raw: Option<Res<RawWorldSource>>) {
    // Reset the per-load gate: every world load writes it fresh, so a script-free
    // world (or an app that never runs this system) reads `false`.
    set_script_activation_blocked(false);

    let Some(raw) = raw else {
        return;
    };
    // No `script` key → nothing to compile. Short-circuit before building any
    // Rhai engine so the entire shipped (script-free) set pays nothing and can
    // never record into the content ledger.
    if raw.toml.get("script").is_none() {
        return;
    }

    let resolver = crate::config_cache::production_script_resolver();
    let compiled = crate::world::script::load::load_world_scripts(&raw.path, &raw.toml, &resolver);

    if crate::world::validate::has_error(&compiled.findings) {
        for f in compiled.findings.iter().filter(|f| f.is_error()) {
            bevy::log::error!(
                target: "world",
                "compile_world_scripts: script [error] {}: {}",
                f.category, f.message
            );
        }
        // Block activation atomically with the composition gate — spawn nothing.
        set_script_activation_blocked(true);
        return;
    }

    // No scripts actually compiled (e.g. an empty `[script]` table) — nothing to
    // run, so insert no runtime and leave behaviour identical to a script-free
    // world.
    if compiled.asts.is_empty() {
        return;
    }

    commands.insert_resource(WorldScriptRuntime {
        host: RuntimeHost::new(),
        asts: compiled.asts,
        triggers: compiled.script_triggers,
        handlers: Vec::new(),
        budget: TickBudget::new(),
        budget_tick: 0,
        content_hash: compiled.content_hash,
        pending_callbacks: PendingCallbacks::new(),
        pending_comms_opens: Vec::new(),
        deadline_handlers: compiled.deadline_handlers,
    });
}

/// `OnEnter(GamePhase::InProgress)` system: seeds the `ship_power` counter in
/// the world flag store from the selected ship's `power_rating`.
///
/// This runs before `spawn_game_start_entities` so that `when` predicates on
/// `[[entity]]` entries with `spawn_on = "GameStart"` can gate spawns on
/// `counter(ship_power) >= N`.
///
/// If `ShipClientConfigResource.power_rating` is `None` (the ship TOML omits
/// the field), no counter is written and `ship_power` defaults to `0`.
pub fn seed_ship_power_counter(
    ship_client_config: Res<crate::lobby::server::ShipClientConfigResource>,
    runtime: Option<ResMut<WorldContentRuntime>>,
) {
    let Some(mut runtime) = runtime else {
        return;
    };
    if let Some(rating) = ship_client_config.0.power_rating {
        runtime.flags.set_flag_value("ship_power", rating as i64);
    }
}

/// Startup system: spawn `[[entity]]` instances owned by the unified
/// `WorldConfig` pipeline.
///
/// The unified pipeline owns both asteroid-field templates AND any
/// `[[entity]]` carrying a `name` field. The complementary `setup_world`
/// in `server_app.rs` handles anonymous non-asteroid immediate entries
/// (stars, planets); the shared `is_owned_by_unified_pipeline` helper
/// guarantees no entry is spawned twice.
///
/// For named entries the UUID is read from `WorldConfig.name_to_uuid`
/// (populated by an earlier assign-uuid pass in this same system), so the
/// spawned `EntityUuid` component matches the UUID that trigger / comms
/// lookups resolve to. For asteroid-field entries a fresh UUID is allocated.
pub(crate) fn spawn_world_entities(
    mut commands: Commands,
    world_config: Option<ResMut<crate::world::config::WorldConfig>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
) {
    let Some(mut world_config) = world_config else {
        return; // No unified WorldConfig (native tests, hardcoded fallback).
    };

    // First pass (PRD #339 slice 2): assign UUIDs to every named [[entity]]
    // entry and register them in `WorldConfig.name_to_uuid` (and mirror
    // into `WorldContentRuntime.name_to_uuid` if present so trigger / comms
    // lookup paths see the same names). This pass runs independently of
    // template resolution so it works even when the config cache is empty
    // (e.g. in unit tests).
    let new_names = crate::world::config::assign_named_entity_uuids(&world_config.entities, || {
        crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity)
    });
    for (name, uuid) in &new_names {
        world_config.name_to_uuid.insert(name.clone(), uuid.clone());
    }
    if let Some(runtime) = runtime.as_mut() {
        for (name, uuid) in &new_names {
            runtime.name_to_uuid.insert(name.clone(), uuid.clone());
        }
    }

    let config_cache = crate::config_cache::get_config_cache();
    let world_snapshot = world_config.clone();
    // `ship_power` is seeded on `OnEnter(InProgress)` â€” not available at
    // Startup. Pass the runtime flags so any Immediate-path predicates that
    // don't depend on ship_power still evaluate correctly.
    let flags = runtime.as_ref().map(|r| &r.flags);
    let _spawned = spawn_immediate_entities_internal(
        &mut commands,
        &world_snapshot,
        &config_cache,
        flags,
        id_mint.as_deref(),
    );
}

/// The `Startup` atomic-activation gate, shared by **both** immediate-spawn
/// systems: `spawn_world_entities` (asteroid fields + named entries) and
/// `setup_world` in `server_app.rs` (the anonymous non-asteroid remainder —
/// stars, planets, nebulae).
///
/// Returns `true` when this world must spawn nothing, having logged every
/// blocking finding. `system` names the caller so the log says which half was
/// stopped; the answer itself is identical for both, because it reads only the
/// parsed [`crate::world::config::WorldConfig`].
///
/// Both callers matter. The two systems are registered independently with no
/// ordering relationship between them, and each answers a failed entity
/// resolution by logging and moving on. Gating only one converts "the world is
/// missing one entity" into "the world is missing one half", which is a worse
/// failure than the one being fixed and breaks the atomicity
/// `world-content-lifecycle-state` promises. See
/// [`crate::world::validate::activation_findings`] for what is checked.
///
/// `config_cache` is the cache the gated spawn will resolve templates from —
/// the global one at `Startup`, a layer's own when a layer is spawning. It is
/// threaded in (issue #973) so the template-resolution check asks the *same*
/// question the spawn is about to ask, rather than a filesystem-backed
/// approximation of it that can pass where the spawn then fails.
///
/// # A note for whoever writes the next bare-`App` fixture
///
/// [`crate::entity_loader::SpawnTemplateLoader`] takes its authority from the
/// host behind the cache, so on native this gate is authoritative about paths
/// like `"fixture/station.toml"` that no filesystem holds. That is right — the
/// host really can decide them, and the answer really is "absent" — but it
/// means **a fixture that adds an `[[entity]]` and forgets the matching
/// `ConfigCache` entry gets zero spawns, not one.** The diagnostic naming the
/// entity and its template goes through `bevy::log::error!`, and a bare `App`
/// installs no `tracing` subscriber, so without help that failure reads as
/// "spawning is broken" rather than "your fixture is incomplete". The
/// `cfg(test)` mirror below is that help: unit tests, the only population at
/// risk, see the findings on stderr. Production keeps the single `bevy::log`
/// channel.
pub fn world_activation_blocked(
    world_config: &crate::world::config::WorldConfig,
    config_cache: &crate::config_cache::ConfigCache,
    system: &str,
) -> bool {
    let templates = crate::entity_loader::SpawnTemplateLoader {
        cache: config_cache,
        host: &crate::entity_loader::WasmTemplateLoader,
    };
    let findings = crate::world::validate::activation_findings(
        world_config,
        &crate::entity_includes::HostFragmentSource,
        &templates,
    );
    let errors = findings.iter().filter(|f| f.is_error()).count();
    // The scripting seam (issue #984, Rhai M6 phase 2a) folds into the SAME
    // atomic gate: a world whose scripts failed to compile/validate must spawn
    // nothing either. `compile_world_scripts` runs earlier in the `Startup`
    // chain and sets this flag fresh per load; a script-free world never trips
    // it, so this branch is inert for the whole shipped set.
    let script_blocked = script_activation_blocked();
    if errors == 0 && !script_blocked {
        return false;
    }
    if script_blocked {
        bevy::log::error!(
            target: "world",
            "{system}: spawn blocked: world scripts failed activation; spawning zero entities"
        );
    }
    for f in findings.iter().filter(|f| f.is_error()) {
        bevy::log::error!(target: "world", "world validation [error] {}: {}", f.category, f.message);
    }
    if errors > 0 {
        bevy::log::error!(
            target: "world",
            "{system}: spawn blocked: world composition invalid ({errors} error(s)); spawning zero entities"
        );
    }
    // See the doc above: a bare `App` has no `tracing` subscriber, so every
    // line emitted above is dropped and a fixture with an incomplete
    // `ConfigCache` looks like a broken spawner. Test builds only — integration
    // tests under `tests/` link the lib without `cfg(test)` and run a real app
    // with `LogPlugin`, so they already see the lines above.
    #[cfg(test)]
    {
        for f in findings.iter().filter(|f| f.is_error()) {
            eprintln!(
                "{system}: world validation [error] {}: {}",
                f.category, f.message
            );
        }
    }
    true
}

/// Spawn the unified-pipeline-owned immediate `[[entity]]` instances.
///
/// Returns the list of spawned `Entity` handles in spawn order
/// (asteroid fields first, then named non-asteroid entries). Callers must
/// flush commands (e.g. via `app.update()`) before querying commands.
///
/// Extracted from `spawn_world_entities` so the spawn logic is testable
/// on native: tests pass a fixture `ConfigCache` (plain `HashMap`) directly
/// instead of relying on the WASM-only `CONFIG_CACHE` thread-local.
///
/// `flags` is the world flag/counter store used to evaluate optional `when`
/// predicates on entity entries. Pass `None` (or a store where `ship_power`
/// is unset) at Startup time â€” `ship_power` is seeded on
/// `OnEnter(GamePhase::InProgress)` before `spawn_game_start_entities`, so
/// `Immediate` entries evaluated here will see `ship_power = 0`.
pub fn spawn_immediate_entities_internal(
    commands: &mut Commands,
    world_config: &crate::world::config::WorldConfig,
    config_cache: &crate::config_cache::ConfigCache,
    flags: Option<&crate::world::flags::FlagStore>,
    id_mint: Option<&crate::world_id::WorldIdMint>,
) -> Vec<Entity> {
    // Atomic-activation guard (issues #750/#752/#906/#969/#973): if this world's
    // composition is invalid, spawn NOTHING — a composition error must never
    // leave partial root-world content active. The headless build path aborts
    // earlier on the full composition; this seam is the last-resort gate for
    // the Bevy `Startup` spawn. `setup_world` in `server_app.rs` owns the OTHER
    // half of that spawn and consults the same gate, so a rejected world loses
    // both halves together.
    if world_activation_blocked(world_config, config_cache, "spawn_world_entities") {
        return Vec::new();
    }

    // The routing predicate asks the SAME lookup the spawn below performs
    // (issue #973 review): cache first, then the host loader. A cache-only
    // predicate answering `false` for a field template that is on disk but
    // uncached would push the entry into `setup_world`'s anonymous bucket,
    // where it spawns without its `[asteroid_field] anchor` resolved — a belt
    // silently sitting at the world origin. See
    // `entity_loader::template_is_asteroid_field`.
    let (fields, named, _anon) =
        crate::world::config::partition_immediate_entities_three_way(world_config, |path| {
            crate::entity_loader::template_is_asteroid_field(
                path,
                config_cache,
                &crate::entity_loader::WasmTemplateLoader,
            )
        });

    // Pre-resolve named-entity positions so `relative_to` references can be
    // looked up during spawn (PRD #337).
    let named_positions = crate::world::config::build_named_entity_positions(world_config);

    let mut spawned = Vec::with_capacity(fields.len() + named.len());

    // Helper: evaluate an optional `when` predicate against the flag store.
    let predicate_allows = |entity_inst: &crate::world::config::WorldEntity| -> bool {
        match &entity_inst.when_predicate {
            None => true,
            Some(pred) => {
                let empty = crate::world::flags::FlagStore::new();
                let store = flags.unwrap_or(&empty);
                pred.evaluate(&[store])
            }
        }
    };

    // Asteroid-field entries get a fresh UUID (they have no name to anchor to).
    for entity_inst in fields {
        if !predicate_allows(entity_inst) {
            continue;
        }
        let mut config = match crate::entity_loader::resolve_entity_via(
            entity_inst,
            config_cache,
            &crate::entity_loader::WasmTemplateLoader,
        ) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "spawn_world_entities: failed to resolve asteroid field '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };
        // Resolve optional `anchor` reference into a concrete world-space offset
        // applied to the streaming spawner. Missing anchor ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ warn + fall back
        // to world origin so a typo never silently relocates the field.
        if let Some(field) = config.asteroid_field.as_mut() {
            if let Some(anchor_name) = field.anchor.as_ref() {
                match world_config.anchors.get(anchor_name) {
                    Some(pos) => field.anchor_offset = *pos,
                    None => {
                        bevy::log::warn!(
                            "spawn_world_entities: asteroid field '{}' references unknown anchor '{}' ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â falling back to world origin",
                            entity_inst.template_path, anchor_name
                        );
                        field.anchor_offset = [0.0, 0.0, 0.0];
                    }
                }
            }
        }
        let uuid = crate::world_id::mint_id_with(id_mint, crate::world_id::IdNamespace::Entity);
        let pos = match resolve_position(entity_inst, &world_config.anchors, &named_positions) {
            Ok(p) => p,
            Err(e) => {
                bevy::log::error!("spawn_world_entities: {e}");
                continue;
            }
        };
        let entity = crate::entity_spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );
        spawned.push(entity);
    }

    // Named non-asteroid entries MUST use the UUID already registered in
    // `world_config.name_to_uuid` so triggers / comms resolve to a real
    // entity. A missing registration is a programmer error â€” log and skip
    // rather than allocate a fresh UUID (which would silently desync).
    for entity_inst in named {
        if !predicate_allows(entity_inst) {
            continue;
        }
        let name = entity_inst
            .name
            .as_ref()
            .expect("partition guarantees Some");
        let uuid = match world_config.name_to_uuid.get(name) {
            Some(u) => u.clone(),
            None => {
                bevy::log::error!(
                    "spawn_world_entities: named entity '{}' has no UUID in WorldConfig.name_to_uuid ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â skipping",
                    name
                );
                continue;
            }
        };
        let config = match crate::entity_loader::resolve_entity_via(
            entity_inst,
            config_cache,
            &crate::entity_loader::WasmTemplateLoader,
        ) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "spawn_world_entities: failed to resolve named entity '{}' ({}): {}",
                    name,
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };
        let pos = match resolve_position(entity_inst, &world_config.anchors, &named_positions) {
            Ok(p) => p,
            Err(e) => {
                bevy::log::error!("spawn_world_entities: named entity '{name}': {e}");
                continue;
            }
        };
        let mut config = config;
        if entity_inst.name.is_some() {
            config.name = entity_inst.name.clone();
        }
        let entity = crate::entity_spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );
        spawned.push(entity);
    }

    spawned
}

/// Resolve an `[[entity]]` instance's spawn position via the pure
/// `world::config::resolve_entity_position` helper, then widen to a Bevy `Vec3`.
///
/// Centralises position resolution for the unified pipeline so anchor-named
/// entries (PRD #337 slice 3) share the same code path as inline-position
/// entries.
fn resolve_position(
    entity_inst: &crate::world::config::WorldEntity,
    anchors: &HashMap<String, [f32; 3]>,
    entities_by_name: &HashMap<String, [f32; 3]>,
) -> Result<Vec3, String> {
    let pos =
        crate::world::config::resolve_entity_position_with(entity_inst, anchors, entities_by_name)?;
    Ok(Vec3::new(pos[0], pos[1], pos[2]))
}

// -- Startup systems ---------------------------------------------------------

/// Startup system: initialise `WorldContentRuntime` and `WorldResource`
/// from the loaded `WorldConfig` (if any).
///
/// This is the post-PRD-#341 sole runtime-init entry point: the legacy
/// scenario / map split is gone. The comms half (`CommsRuntime`,
/// `CommsInboxRes`) is initialised by `comms::server::init_comms_runtime`,
/// which runs after this system in the Startup schedule. When no
/// `WorldConfig` resource is present (native unit tests) this is a
/// no-op and downstream trigger systems remain quiet.
pub(crate) fn init_world_runtime(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut world_resource: ResMut<WorldResource>,
    mut script_runtime: Option<ResMut<WorldScriptRuntime>>,
) {
    let Some(world_config) = world_config else {
        return;
    };

    // The mission clock is NOT anchored here. Loading the world and starting
    // the mission are different moments - the lobby sits between them - and
    // every `after_secs` in a world TOML is authored against the second one.
    // `anchor_mission_clock` stamps `mission_clock_anchor_secs` on the first
    // simulation tick of `GamePhase::InProgress`; see that system for what a
    // boot anchor cost.

    // Populate scenario metadata so the lobby title/description render correctly.
    world_resource.0.scenario_title = world_config.global.title.clone().unwrap_or_default();
    world_resource.0.scenario_description =
        world_config.global.description.clone().unwrap_or_default();

    // `spawn_world_entities` ran earlier in the Startup chain and already
    // populated `runtime.name_to_uuid` for named [[entity]] instances. Fold
    // in any additional names from `WorldConfig.name_to_uuid` WITHOUT
    // overwriting: existing entries win (single source of truth from the
    // spawn pass).
    for (name, uuid) in &world_config.name_to_uuid {
        runtime
            .name_to_uuid
            .entry(name.clone())
            .or_insert_with(|| uuid.clone());
    }

    // The trigger table starts EMPTY. It used to be derived from the parsed
    // world's `[[trigger]]` blocks and the scripted states were appended after
    // them; issue #985 deleted that parser, so scripts are the only source and
    // `WorldScriptRuntime.handlers` is parallel to a table it fills alone. A
    // script-free world keeps an empty table, exactly as before.
    runtime.trigger_states.clear();

    // Merge script-authored triggers (issue #984, Rhai M6 phase 2a).
    if let Some(script_runtime) = script_runtime.as_deref_mut() {
        merge_script_triggers(&mut runtime.trigger_states, script_runtime);
    }

    // Issue #415: emit a WorldLoaded event so `on_world_loaded` triggers
    // declared in the base world fire on the first Update tick. Pushed onto
    // the pending queue (rather than evaluated here) so the dispatch logic
    // inside `tick_trigger_pipeline` is the single owner of trigger action
    // execution.
    runtime.pending_world_events.push(WorldEvent::WorldLoaded);
}

/// Append one [`TriggerState`] per compiled [`ScriptTrigger`] to `trigger_states`,
/// and build `script_runtime.handlers` parallel to it — one `Some` per appended
/// state (issue #984, Rhai M6 phase 2a).
///
/// It appended AFTER a table `init_world_runtime` had already filled from the
/// world's `[[trigger]]` blocks, and `handlers` carried a `None` for each of
/// those declarative indices. Issue #985 deleted that front-end, so the table
/// starts empty and every index is a scripted one; the `handlers` vec is
/// built parallel rather than assumed dense so the two stay index-aligned by
/// construction.
///
/// An appended state feeds `evaluate_single_trigger` like any other — the
/// evaluator never knew where a trigger came from — and the handler resolved
/// through the parallel `handlers` entry is what supplies the effects when it
/// fires.
pub(crate) fn merge_script_triggers(
    trigger_states: &mut Vec<TriggerState>,
    script_runtime: &mut WorldScriptRuntime,
) {
    let mut handlers: Vec<Option<ScriptHandlerRef>> = vec![None; trigger_states.len()];
    // This one-time merge is the only reader of `triggers`, so `take` it rather
    // than borrow-and-clone: the compiled `ScriptTrigger`s are consumed here and
    // not retained for the world's lifetime (finding 5), and taking ownership lets
    // each field move into the appended state/handler instead of cloning.
    for st in std::mem::take(&mut script_runtime.triggers) {
        trigger_states.push(TriggerState {
            trigger: st.trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        });
        handlers.push(Some(ScriptHandlerRef {
            script_path: st.source_path,
            fn_name: st.handler,
        }));
    }
    script_runtime.handlers = handlers;
}

/// Startup system: queue all `extra_worlds` paths from the loaded `WorldConfig`
/// as `LoadWorld` commands so they are merged into the runtime on the first frame.
///
/// Runs after `init_world_runtime` in the Startup chain. Each path is pushed
/// into `PendingWorldLayerChanges` rather than applied directly so the same
/// `apply_world_layer_changes` path handles both startup and trigger-fired loads.
fn load_extra_worlds(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut pending: ResMut<PendingWorldLayerChanges>,
) {
    let Some(world_config) = world_config else {
        return;
    };
    for path in &world_config.extra_worlds {
        pending.0.push(WorldLayerChange::Load {
            path: path.clone(),
            loader_path: None,
        });
    }
}

// -- Update systems ----------------------------------------------------------

/// Broadcast `ObjectiveSummary` when objectives change.
pub(crate) fn broadcast_objective_summary(
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {
    if !objectives.0.is_dirty() {
        return;
    }

    let objectives_snap = objectives.0.sorted_snapshots();

    outbox.0.push((
        Target::All,
        ServerMessage::ObjectiveSummary {
            objectives: objectives_snap,
        },
    ));

    objectives.0.mark_clean();
}

// -- Mission clock -----------------------------------------------------------

/// `OnEnter(GamePhase::InProgress)` system: disarm the mission clock so the
/// next simulation tick re-stamps it.
///
/// One line, and it is the whole multi-round half of the fix. A session can
/// reach `InProgress` more than once (`ReturnToLobby` from the game-over screen
/// puts the crew back in the lobby and a second round starts from there, which
/// is why `reset_command_log` and `reset_broadcast_caches_on_start` sit in the
/// same `OnEnter` chain). Without this, round two would measure `after_secs`
/// from round one's start and arrive with its whole schedule already expired.
///
/// It writes `None` rather than a reading of its own because it does not run
/// in a fixed step. Bevy applies a `NextState<GamePhase>` write at whichever
/// `StateTransition` site comes first, and the two production start paths use
/// different ones: the lobby countdown and headless auto-start write from
/// `FixedUpdate` (the fixed-schedule site `register_fixed_state_transition`
/// installs), while `auto_transition_from_loading` writes from `Update` (the
/// frame-level site). `Time` resolves to `Time<Fixed>` at the first and
/// `Time<Virtual>` at the second, and those two clocks disagree by up to one
/// timestep, so a reading taken here would make the schedule a function of
/// which path started the mission and of frame pacing. Deferring the reading
/// to [`anchor_mission_clock`], which only ever runs inside a fixed step,
/// keeps it on one clock.
pub(crate) fn arm_mission_clock(mut runtime: ResMut<WorldContentRuntime>) {
    runtime.mission_clock_anchor_secs = None;
}

/// Stamp time zero for the mission clock on the first simulation tick of the
/// mission.
///
/// # The bug this closes (latent since #475, lethal since #960)
///
/// `after_secs` used to be measured from a `Time::elapsed_secs()` reading taken
/// by `init_world_runtime`, a `Startup` system. `GamePhase` defaults to
/// `Lobby`, the world is loaded at `Startup`, and `Time<Virtual>` (and through
/// it `Time<Fixed>`) runs the whole time the crew is picking stations. The
/// `SimSet` chain is gated on `in_state(GamePhase::InProgress)`, so no trigger
/// was *evaluated* during the lobby - but the clock they would be evaluated
/// against kept running. After a 90-second lobby the first `InProgress` tick
/// therefore emitted `TimerElapsed { elapsed_secs: 90 }`, and every trigger
/// authored at 0, 45 and 90 fired in one dispatch batch. In `combat_test` that
/// is three waves and four comms bursts landing together on tick one; at a
/// five-minute lobby the entire eight-wave raid arrives at once and the victory
/// trigger is armed before the player has moved.
///
/// Nothing caught it because the only automated driver is headless, which
/// auto-starts on the first fixed step with nobody connected - elapsed is
/// approximately zero at `InProgress`, so the boot anchor and the mission
/// anchor agree to within a tick. It takes a lobby to tell them apart, and
/// `combat_test_wave_clock_measures_from_mission_start_not_app_boot`
/// (`tests/headless_runner.rs`) supplies one.
///
/// # Why here
///
/// `SimSet::Physics` is gated on `InProgress`, so "the first tick this system
/// runs" IS "the first simulation tick of the mission" - the gate does the
/// work, and there is no second predicate to keep in step with it. Running
/// inside the fixed schedule also means `Time` is `Time<Fixed>`, the same clock
/// `collect_world_events` and `tick_delayed_actions` read the anchor back
/// against, and the same clock two hosts would agree on: the reading is a whole
/// number of sim ticks, not a frame-pacing artifact.
///
/// Ordered before `collect_world_events` so a mission whose first wave is
/// authored at `after_secs = 0` still gets its `TimerElapsed { 0.0 }` on that
/// very tick rather than one tick late.
///
/// # What it does not do
///
/// Nothing re-anchors while a mission is running. In particular
/// `apply_pending_scenario_loads` does not: that applier MERGES a world TOML
/// into a live runtime (it appends trigger states, it does not replace them),
/// so re-anchoring there would rewind the clock the base world's own in-flight
/// `on_timer` triggers and `action_delays` are already scheduled against. A
/// genuinely new scenario arrives the other way - back to the lobby and in
/// again - and that path re-arms through [`arm_mission_clock`].
///
/// `world_config` gates the stamp for the same reason `init_world_runtime`
/// gates on it: an app with no world (native unit-test fixtures) must go on
/// seeing no `TimerElapsed` events at all, or `collect_world_events` would
/// start writing `WorldEventBuffer` every tick in apps that previously left it
/// untouched.
pub(crate) fn anchor_mission_clock(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
    time: Option<Res<bevy::time::Time>>,
) {
    // Both reads go through an immutable deref, so an already-anchored tick
    // does not mark `WorldContentRuntime` changed.
    if world_config.is_none() || runtime.mission_clock_anchor_secs.is_some() {
        return;
    }
    // `Time` is optional so test apps without `TimePlugin` keep working: they
    // never anchor, and therefore never see `TimerElapsed` - same as before.
    if let Some(t) = time {
        runtime.mission_clock_anchor_secs = Some(t.elapsed_secs());
    }
}

/// Arm every `[[deadline]]` this world authored, on the first simulation tick of
/// the mission (issue #1024).
///
/// Chained immediately after [`anchor_mission_clock`] and gated the same way, so
/// "the tick this runs for the first time" IS "the first simulation tick of the
/// mission": `SimSet::Physics` is gated on `InProgress`, and `due_secs` therefore
/// measures from mission start rather than from app boot. That is the #960 fix
/// applied to this vocabulary from the outset — a ninety-second lobby must not
/// retire a mission's deadlines before the crew has the con.
///
/// # It arms; it does not tick
///
/// The only thing arming *does* is push one ordinary
/// [`ScheduledCall`](crate::world::script::schedule::ScheduledCall) per deadline
/// onto `WorldScriptRuntime::pending_callbacks` — the queue
/// [`tick_script_callbacks`] already drains. No system introduced by issue #1024
/// walks the deadline table looking for due work, and this one runs its body
/// exactly once per mission. See [`crate::world::deadlines`] for why that is the
/// whole point.
///
/// # Determinism
///
/// A no-op for every world that authors no deadline: the early returns happen
/// before any `DerefMut`, so no change-detection tick flips and a deadline-free
/// run is byte-identical to one from before this system existed. Where deadlines
/// ARE authored, every fire tick is `now_tick + seconds_to_ticks(due_secs, hz)`
/// over values two peers both read from the world file, so both peers arm the
/// same ticks.
pub(crate) fn arm_mission_deadlines(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut script: Option<ResMut<WorldScriptRuntime>>,
) {
    // Immutable reads only on the already-armed path, so an armed mission does
    // not mark `WorldContentRuntime` changed every tick.
    let Some(world_config) = world_config else {
        return;
    };
    if runtime.deadlines.armed || world_config.deadlines.is_empty() {
        return;
    }
    // A deadline fires a script fn, so a world with deadlines and no script
    // runtime has nothing to arm against. `validate_deadline_handlers` already
    // blocks such a world at load; this is the belt to that brace.
    let Some(script) = script.as_deref_mut() else {
        return;
    };
    let now_tick = sim_tick.map(|t| t.0).unwrap_or(0);
    let queued = runtime.deadlines.arm(
        &world_config.deadlines,
        &script.deadline_handlers,
        now_tick,
        world_config.global.sim_tick_hz,
    );
    // THE reuse, in one line: a deadline's firing is an entry on the EXISTING
    // deferred-callback queue.
    script.pending_callbacks.extend(queued);
}

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
/// Free function rather than a system because it is called from all four sites
/// that consume a call's [`CallEffects`] — the trigger pipeline, the callback
/// drain, and the two comms answer paths — each of which already holds both
/// borrows.
pub(crate) fn apply_deadline_changes(
    changes: &[crate::world::deadlines::DeadlineChange],
    deadlines: &mut crate::world::deadlines::DeadlineTable,
    pending_callbacks: &mut PendingCallbacks,
    now_tick: u64,
    tick_hz: f32,
) {
    for change in changes {
        let Some(edit) = deadlines.apply(change, now_tick, tick_hz) else {
            continue;
        };
        if let Some(stale) = edit.retract {
            pending_callbacks.retract(&stale);
        }
        if let Some(fresh) = edit.push {
            pending_callbacks.push(fresh);
        }
    }
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
/// `FlagSet` it produces is queued as a `WorldEvent` and evaluated by the
/// trigger pipeline on a LATER tick, so an `on_flag_set` handler reading
/// `ctx.commitments.state(…)` sees the settled promise.
///
/// A duplicate id is logged rather than propagated. The script surface already
/// raised on it — dropping that call's whole buffer under settled decision 10,
/// which is where the author is told — so reaching here means the live ledger
/// disagreed with the per-call snapshot the raise was decided against, which
/// only two calls resolving the same new id in one tick can produce. Refusing
/// the second is the same answer the snapshot would have given.
///
/// Free function rather than a system, for [`apply_deadline_changes`]' reason:
/// it is called from all four sites that consume a call's [`CallEffects`].
pub(crate) fn apply_commitment_changes(
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
///    measured from `mission_clock_anchor_secs` - which [`anchor_mission_clock`]
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
    hulls: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntitySystemHull,
    )>,
) {
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
    for (uuid, hull) in &hulls {
        let max = hull.0.total_max();
        if max <= 0.0 {
            continue;
        }
        let current_fraction = (hull.0.total_current() / max).clamp(0.0, 1.0);
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
        world_events.append(&mut runtime.pending_world_events);
    }
    let elapsed_secs = time.as_ref().and_then(|t| {
        runtime
            .mission_clock_anchor_secs
            .map(|loaded_at| (t.elapsed_secs() - loaded_at).max(0.0))
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

/// Upper bound on `tick_trigger_pipeline`'s within-tick trigger-chaining passes.
///
/// A `set_flag` action emits a `FlagSet` event that a downstream `on_flag_set`
/// trigger can react to in the same Bevy frame, which can in turn set another
/// flag. The cap stops a pathological feedback loop from hanging the frame.
const MAX_CHAIN_PASSES: i32 = 16;

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
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: FactionDispatchParams,
    time: Option<Res<bevy::time::Time>>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
    // The scripting seam (issue #984, Rhai M6 phase 2a). Both `Option`, so a
    // script-free world (no `WorldScriptRuntime`) and every bare-`App` fixture
    // take the `None` arm and the scripted-handler branch below is skipped
    // entirely — behaviour there is byte-identical to before scripting existed.
    mut script: ScriptRuntimeParams,
) {
    let empty_anchors: HashMap<String, [f32; 3]> = HashMap::new();
    // Seeded UUID source for `SpawnEntity` dispatch. Bound once per system run
    // because `DispatchContext::uuid_source` is a `&dyn Fn`.
    let uuid_source =
        || crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);
    // Template source for `SpawnEntity` dispatch (issue #715), built once per
    // system run. `WasmTemplateLoader` unconditionally: it serves the
    // preloaded config cache first and, on native, falls back to the
    // filesystem — reproducing the old cfg-split inline block on both targets.
    let template_loader = crate::entity_loader::WasmTemplateLoader;
    if buffer.0.is_empty() && runtime.pending_delayed_actions.is_empty() {
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
            .mission_clock_anchor_secs
            .map(|loaded_at| (t.elapsed_secs() - loaded_at).max(0.0))
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
        if sr.budget_tick != now_tick {
            sr.budget = TickBudget::new();
            sr.budget_tick = now_tick;
        }
    }

    // Reborrow the `ResMut` as a plain `&mut` so the evaluation loop below can
    // split disjoint field borrows (`&runtime.flags` for condition chains while
    // `&mut runtime.trigger_states[idx]` is handed to the evaluator) — a smart
    // pointer cannot split, a plain reference can. Placed after the early
    // return so change detection still only marks the resource on ticks that
    // actually process events (the pre-existing behaviour: every path past
    // this point mutated through the `ResMut` anyway).
    let runtime = &mut *runtime;

    let name_to_uuid = runtime.name_to_uuid.clone();

    // Build UUID â†’ ECS Entity map once per tick so the six per-entity
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
        // Per-trigger evaluation: build chain from origin_layer up. The
        // chains borrow the LIVE stores (`runtime.flags` and `layer_map`'s
        // per-layer stores) rather than per-pass clones: evaluation is safe
        // against them because the pass is two-phase — every condition is
        // evaluated before any fired action is dispatched, so no store
        // mutates while these borrows are alive.
        // Carries the trigger-states index alongside each fired trigger so the
        // dispatch loop can look up its scripted handler (issue #984, Rhai M6
        // phase 2a) in `WorldScriptRuntime.handlers[idx]`.
        let mut fired: Vec<(usize, crate::world::content::FiredTrigger)> = Vec::new();
        // We have to clone the origin_layer slice up front: the evaluator
        // below takes `&mut runtime.trigger_states[idx]`, so nothing may
        // hold a borrow on `runtime.trigger_states` across the loop.
        let trigger_origins: Vec<Option<String>> = runtime
            .trigger_states
            .iter()
            .map(|s| s.origin_layer.clone())
            .collect();
        let entity_groups = runtime.entity_groups.clone();
        for (idx, origin) in trigger_origins.iter().enumerate() {
            // Build the flag-store and layer-path chains for this trigger.
            // The store half is the ONE shared layered walk
            // (`layered_flag_chain`), also read through by every AI
            // policy/selector host (issue #891 stage 2) via
            // `entity_flag_chain`; the path half is this pipeline's own
            // wrapper (issue #891 review finding 2) since nothing else needs it.
            let (flag_chain, layer_chain) = layered_flag_chain_with_paths(
                origin.as_deref(),
                &runtime.flags,
                world_layers.layer_map.as_deref(),
            );
            let result = crate::world::content::evaluate_single_trigger(
                &mut runtime.trigger_states[idx],
                &current_events,
                &name_to_uuid,
                &flag_chain,
                &layer_chain,
                &entity_groups,
                current_elapsed,
            );
            if let Some(ft) = result {
                fired.push((idx, ft));
            }
        }

        if fired.is_empty() {
            break;
        }

        let mut next_events: Vec<WorldEvent> = Vec::new();
        for (idx, ft) in fired {
            // The handler for this trigger (IP-2, issue #984, Rhai M6 phase 2a).
            // A per-action dispatch loop for the fired trigger's own
            // `[[trigger.action]]` array used to run first; issue #985 deleted
            // the parser that filled it, so this is the whole of a fire. The
            // handler runs on the runtime host and its result goes through the
            // SAME apply path, so a scripted flag write chains into the next
            // pass exactly as a declarative `set_flag` used to
            // (`apply_script_commands`).
            if let Some(sr) = script.runtime.as_deref_mut() {
                let handler = sr.handlers.get(idx).and_then(|h| h.clone());
                if let Some(h) = handler {
                    // Split `WorldScriptRuntime` into disjoint field borrows so
                    // the one `&self` call takes `&mut budget` and `&ast` at once,
                    // while `&runtime.flags` (a DISJOINT resource) is the base the
                    // flag overlay snapshots. `call` returns owned `CallEffects`,
                    // so no `WorldScriptRuntime` borrow survives into the apply.
                    let effects = {
                        let WorldScriptRuntime {
                            host, asts, budget, ..
                        } = &mut *sr;
                        match asts.get(&h.script_path) {
                            Some(ast) => Some(host.call(
                                budget,
                                &script_clock,
                                ast,
                                &h.script_path,
                                &h.fn_name,
                                &runtime.flags,
                                &runtime.deadlines,
                                &runtime.commitments,
                                Map::new(),
                            )),
                            None => {
                                bevy::log::warn!(
                                    "tick_trigger_pipeline: scripted handler '{}' names a \
                                     missing unit '{}'",
                                    h.fn_name,
                                    h.script_path
                                );
                                None
                            }
                        }
                    };
                    if let Some(effects) = effects {
                        apply_script_commands(
                            effects.commands,
                            "tick_trigger_pipeline (script)",
                            &mut next_events,
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
                            ft.origin_layer.clone(),
                            ft.entity_name.clone(),
                        );
                        // Script-scheduled delayed effects join the SAME queue a
                        // TOML `action_delays` entry uses; dropped when the
                        // mission clock is unanchored, matching the declarative
                        // path above.
                        if elapsed_secs.is_some() {
                            runtime.pending_delayed_actions.extend(effects.delayed);
                        }
                        // Deferred `after(..)` callbacks join the runtime's
                        // serialisable future-work queue (issue #984, phase 2b);
                        // `tick_script_callbacks` drains the due ones each tick.
                        // 2a dropped these — now they are retained.
                        sr.pending_callbacks.extend(effects.callbacks);
                        // A scripted `open_comms` queues for the comms module to
                        // materialise; empty for every world that authors none.
                        sr.pending_comms_opens.extend(effects.comms_opens);
                        // And its named-deadline mutations (issue #1024): applied
                        // against the live table, each taking its edit to the SAME
                        // callback queue above — which is what stops a slipped
                        // deadline also firing at its old tick.
                        apply_deadline_changes(
                            &effects.deadline_changes,
                            &mut runtime.deadlines,
                            &mut sr.pending_callbacks,
                            script_clock.tick,
                            script_clock.tick_hz,
                        );
                        apply_commitment_changes(
                            &effects.commitment_changes,
                            &mut runtime.commitments,
                            script_clock.tick,
                        );
                    }
                }
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

/// Project the live `WorldLayerMap` into the read-only `LayerView`s that
/// `dispatch_action` reads.
///
/// Called once per action rather than once per pass: `LayerView::flags` must be
/// the *live* per-layer store so that a flag mutation applied earlier in this
/// same pass is visible to the next action's before/after preview. See
/// `DispatchContext::base_flags` for why that matters.
pub(crate) fn project_layer_views(layer_map: Option<&WorldLayerMap>) -> HashMap<String, LayerView> {
    layer_map
        .map(|lm| {
            lm.0.iter()
                .map(|(path, wr)| {
                    (
                        path.clone(),
                        LayerView {
                            flags: wr.flags.clone(),
                            loader_path: wr.loader_path.clone(),
                            anchors: wr.anchors.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve a dispatched UUID to its target entity's `ShipModifiers` component.
///
/// The pure layer resolves entity *name* -> *UUID* and stops there (writing a
/// component is irreducibly impure), so the last two hops — UUID -> `Entity` ->
/// component — land here. `what` names the calling action for the warn text.
fn world_modifiers<'a>(
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
fn world_modifier_source(tag: String) -> crate::messages::ModifierSource {
    crate::messages::ModifierSource::World {
        id: WORLD_MODIFIER_SOURCE_ID.to_string(),
        tag,
    }
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
pub(crate) fn apply_script_commands(
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
    mut game_over_reason: Option<&mut crate::simulation::GameOverReason>,
    faction_dispatch: &mut FactionDispatchParams,
    ai_query: &mut Query<
        (
            &EntityUuid,
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    mut balance_events: Option<&mut bevy::ecs::message::Messages<crate::balance::BalanceEvent>>,
    // The dispatch context a name-resolving `BufferedEffect::Action` needs (issue
    // #984, Rhai M6). `uuid_source` is the SAME closure `tick_trigger_pipeline`
    // binds — so a scripted `spawn_entity` mints its `EntityUuid` inside
    // `dispatch_spawn_entity` from the real `WorldIdMint`, in the same order as
    // the declarative twin (never at the effects.rs boundary, never a fallback
    // mint). `origin_layer`/`entity_name` come from the fired trigger (the
    // callback path passes `None`/`None`).
    uuid_source: &dyn Fn() -> String,
    template_loader: &dyn crate::entity_loader::TemplateLoader,
    base_anchors: &HashMap<String, [f32; 3]>,
    origin_layer: Option<String>,
    entity_name: Option<String>,
) {
    for eff in commands_in {
        match eff {
            // A resolved command (the M1 set + flag writes): applied directly, as
            // before. A `MutateFlag` gets its transition event previewed here (it
            // was pushed onto the sink DIRECTLY by `ctx.flags.*`, bypassing
            // `dispatch_action`'s transition step), so a scripted flag write chains
            // a downstream `on_flag_set` exactly as a declarative `set_flag` does.
            BufferedEffect::Cmd(cmd) => {
                let mut new_events: Vec<WorldEvent> = Vec::new();
                if let ActionCmd::MutateFlag {
                    target_layer,
                    name,
                    mutation,
                } = &cmd
                {
                    // Resolve the store this write lands in (scripts emit base-layer
                    // writes — `target_layer: None` — but resolve `Some` too for
                    // forward-compat), preview the mutation against it, and push the
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
                );
            }
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
    mut game_over_reason: Option<&mut crate::simulation::GameOverReason>,
    faction_dispatch: &mut FactionDispatchParams,
    ai_query: &mut Query<
        (
            &EntityUuid,
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    // Balance telemetry: `ObjectiveCompleted` is emitted here, guarded on the
    // objective actually transitioning. `Option<&mut Messages<_>>` so callers
    // in bare-`App` fixtures (no registered message) can pass `None`.
    mut balance_events: Option<&mut bevy::ecs::message::Messages<crate::balance::BalanceEvent>>,
) {
    let DispatchResult {
        commands: action_cmds,
        new_events,
        name_to_uuid_inserts,
        entity_group_inserts,
        warnings,
    } = result;

    for warning in warnings {
        bevy::log::warn!("{log_ctx}: {warning}");
    }

    events_out.extend(new_events);

    for cmd in action_cmds {
        match cmd {
            ActionCmd::AddObjective {
                id,
                text,
                mandatory,
                targets,
                directive,
                utility,
                source,
                origin_layer,
            } => {
                let added = objectives.0.add_full(
                    id.clone(),
                    text,
                    mandatory,
                    targets,
                    directive,
                    utility,
                    source,
                );
                // Record layer ownership (issue #751) so UnloadWorld removes
                // exactly the objectives this layer's triggers added. Only on
                // a genuinely new insert, and only for layer-authored
                // triggers with a live layer-map entry.
                if added {
                    if let (Some(path), Some(lm)) = (origin_layer, layer_map.as_deref_mut()) {
                        if let Some(layer) = lm.0.get_mut(&path) {
                            layer.owned_objective_ids.push(id);
                        }
                    }
                }
            }

            ActionCmd::ResetTrigger { id } => {
                let n =
                    crate::world::content::reset_triggers_by_id(&mut runtime.trigger_states, &id);
                if n == 0 {
                    bevy::log::warn!(
                        "{log_ctx}: ResetTrigger('{id}') matched no trigger with that id"
                    );
                }
            }

            ActionCmd::CompleteObjective { id } => {
                // Guard the tracer on the actual transition to `Completed`, so
                // a re-issued CompleteObjective on an already-complete objective
                // does not double-emit (issue #841).
                if objectives.0.complete(&id) {
                    if let Some(msgs) = balance_events.as_deref_mut() {
                        msgs.write(crate::balance::BalanceEvent::ObjectiveCompleted {
                            objective_id: id.clone(),
                        });
                    }
                }
            }

            ActionCmd::FailObjective { id } => {
                objectives.0.fail(&id);
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
            ActionCmd::AdjustInfrastructureCondition { entity, delta } => {
                let Some(uuid) = runtime.name_to_uuid.get(&entity).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: AdjustInfrastructureCondition: no entity named '{entity}' \
                         in this world — ignoring"
                    );
                    continue;
                };
                runtime
                    .pending_condition_adjustments
                    .push(crate::infrastructure::ConditionAdjustment { uuid, delta });
            }

            // Issue #1026, on exactly the same terms: the applier resolves the
            // two names, and `tick_operations` — which can see the capability
            // table, the power grid and the target's position — decides whether
            // the hold opens.
            ActionCmd::StartOperation { ship, verb, target } => {
                let Some(ship_uuid) = runtime.name_to_uuid.get(&ship).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: StartOperation: no entity named '{ship}' in this world — \
                         ignoring"
                    );
                    continue;
                };
                let Some(target_uuid) = runtime.name_to_uuid.get(&target).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: StartOperation: no target named '{target}' in this world — \
                         ignoring"
                    );
                    continue;
                };
                runtime
                    .pending_operation_starts
                    .push(crate::operations::PendingOperationStart {
                        ship_uuid,
                        verb,
                        target_uuid,
                    });
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
                runtime
                    .pending_civilian_orders
                    .push(crate::civilian::PendingCivilianOrder { uuid, order });
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
                name: _,
                uuid,
                position,
                rotation,
                scale,
                layer_path,
                overrides: _,
            } => {
                // The template arrives already resolved and name-patched: the
                // pure layer loaded it via `DispatchContext::template_loader`
                // and gated the failure path (issue #715), so a command here
                // always spawns.
                let pos_vec = Vec3::new(position[0], position[1], position[2]);
                let spawned =
                    crate::entity_spawner::spawn_entity(commands, &config, pos_vec, uuid, None);

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
                    }
                }
            }

            ActionCmd::DestroyEntity { uuid } => {
                let target_entity = uuid_to_entity.get(&uuid).copied();
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
                        world.get_resource_mut::<Messages<crate::ai_plugin::AiEntityDestroyed>>()
                    {
                        msgs.write(crate::ai_plugin::AiEntityDestroyed { entity_uuid: uuid });
                    }
                });
                if let Some(ent) = target_entity {
                    commands.entity(ent).try_despawn();
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

/// Drain actions from `pending_delayed_actions` whose `fire_at_elapsed` has
/// elapsed and dispatch them through the same `world::dispatch` table
/// `tick_trigger_pipeline` uses.
///
/// Registered after `tick_trigger_pipeline` in `SimSet::Physics` so that it sees
/// the same tick's `mission_clock_anchor_secs` anchor.
pub(crate) fn tick_delayed_actions(
    mut runtime: ResMut<WorldContentRuntime>,
    time: Option<Res<bevy::time::Time>>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut pending_layers: Option<ResMut<PendingWorldLayerChanges>>,
    mut layer_map: Option<ResMut<WorldLayerMap>>,
    base_world_config: Option<Res<crate::world::config::WorldConfig>>,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
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
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    let Some(elapsed) = time.as_ref().and_then(|t| {
        runtime
            .mission_clock_anchor_secs
            .map(|loaded| (t.elapsed_secs() - loaded).max(0.0))
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
    let template_loader = crate::entity_loader::WasmTemplateLoader;
    let uuid_source =
        || crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);

    // Ready/still-pending is a pure decision (`world::delayed`); only the
    // elapsed-clock read above and the dispatch below touch Bevy.
    let queued = std::mem::take(&mut runtime.pending_delayed_actions);
    let schedule = partition_delayed_actions(queued, elapsed);
    runtime.pending_delayed_actions = schedule.still_pending;

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
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: FactionDispatchParams,
    time: Option<Res<bevy::time::Time>>,
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    // The mint a callback-scheduled `spawn_entity` draws its `EntityUuid` from
    // (issue #984, Rhai M6). Added so the callback path builds the IDENTICAL
    // `uuid_source` the trigger path holds — else a spawn from an `after(..)`
    // callback would fall back to the process-global mint and diverge (R2). `None`
    // for a bare-`App` fixture, exactly like `tick_delayed_actions`.
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
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
    if sr.budget_tick != now_tick {
        sr.budget = TickBudget::new();
        sr.budget_tick = now_tick;
    }

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
            .mission_clock_anchor_secs
            .map(|loaded| (t.elapsed_secs() - loaded).max(0.0))
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
    let template_loader = crate::entity_loader::WasmTemplateLoader;
    let uuid_source =
        || crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);

    // Reborrow the `WorldContentRuntime` `ResMut` as a plain `&mut` so `runtime.flags`
    // (the flag overlay base) and `&mut runtime` (the apply path) can be borrowed
    // in sequence — the same disjoint-field split `tick_trigger_pipeline` uses.
    let runtime = &mut *runtime;

    for call in due {
        // Split `WorldScriptRuntime` into disjoint field borrows so the one
        // `&self` call takes `&mut budget` and `&ast` at once, while
        // `&runtime.flags` (a DISJOINT resource) is the overlay base. `call`
        // returns owned `CallEffects`, so no `WorldScriptRuntime` borrow survives
        // into the apply below.
        let effects = {
            let WorldScriptRuntime {
                host, asts, budget, ..
            } = &mut *sr;
            match asts.get(&call.script_path) {
                Some(ast) => Some(host.call(
                    budget,
                    &script_clock,
                    ast,
                    &call.script_path,
                    &call.fn_name,
                    &runtime.flags,
                    &runtime.deadlines,
                    &runtime.commitments,
                    Map::new(),
                )),
                None => {
                    bevy::log::warn!(
                        "tick_script_callbacks: callback '{}' names a missing unit '{}'",
                        call.fn_name,
                        call.script_path
                    );
                    None
                }
            }
        };
        let Some(effects) = effects else {
            continue;
        };

        // A callback's chaining events queue for the NEXT tick, exactly as
        // `tick_delayed_actions` does — this system runs after
        // `tick_trigger_pipeline` has already drained `pending_world_events`.
        let mut out_events: Vec<WorldEvent> = Vec::new();
        apply_script_commands(
            effects.commands,
            "tick_script_callbacks",
            &mut out_events,
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
            // A callback is authored at base scope (no origin layer / trigger
            // entity to thread), so `None`/`None` — matching the effect sink's and
            // schedule sink's `origin_layer: None` note.
            &uuid_source,
            &template_loader,
            world_layers
                .base_world_config
                .as_ref()
                .map(|wc| &wc.anchors)
                .unwrap_or(&empty_anchors),
            None,
            None,
        );
        runtime.pending_world_events.extend(out_events);
        // A callback-scheduled `in_seconds` effect joins the SAME delayed queue,
        // dropped when the mission clock is unanchored (matching the trigger path).
        if elapsed_secs.is_some() {
            runtime.pending_delayed_actions.extend(effects.delayed);
        }
        // A callback that scheduled another callback re-queues it for a future
        // tick (drained the next time this system observes it due).
        sr.pending_callbacks.extend(effects.callbacks);
        // A callback that opened a comms thread queues it for the comms module.
        sr.pending_comms_opens.extend(effects.comms_opens);
        // A callback that slipped or cancelled a deadline re-keys it here
        // (issue #1024) — including a deadline's OWN fire handler, which may
        // legitimately arm the next one in a chain.
        apply_deadline_changes(
            &effects.deadline_changes,
            &mut runtime.deadlines,
            &mut sr.pending_callbacks,
            script_clock.tick,
            script_clock.tick_hz,
        );
        apply_commitment_changes(
            &effects.commitment_changes,
            &mut runtime.commitments,
            script_clock.tick,
        );
    }
}

/// Build a `UUID Ã¢â€ â€™ faction UUID` map from every entity that carries a
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
/// spawned with a `ShipModifiers` Component (`src/entities/spawner.rs`
/// and `spawn_game_start_entities`), so `.get_mut(entity)` is the correct
/// primary write target after the name is resolved through
/// `WorldContentRuntime.name_to_uuid` â†’ UUID â†’ ECS `Entity`.
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
    pub attacked: MessageReader<'w, 's, crate::ai_plugin::AiEntityAttacked>,
    pub destroyed: MessageReader<'w, 's, crate::ai_plugin::AiEntityDestroyed>,
    pub waypoint_reached: MessageReader<'w, 's, crate::ai_plugin::AiWaypointReached>,
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
    pub registry: Option<ResMut<'w, crate::config_cache::FactionRegistryResource>>,
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
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    registry: &crate::faction::FactionRegistry,
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
        if !crate::faction::is_enemy(self_faction, target_faction, registry) {
            weapons_target.0 = None;
        }
    }
}

use crate::entity_spawner::BehaviourSection;
use crate::entity_spawner::EntityUuid;

// ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ Pending scenario load system ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬

/// Bevy system: drain `PendingScenarioLoad` and merge each world TOML into the
/// live `WorldContentRuntime` (trigger states).
///
/// On WASM the TOML string is not available at runtime (JS pre-fetches only the
/// initial world), so we push paths into the WASM-side pending-world queue and
/// the implementation returns early until the JS bridge delivers the TOML via
/// `wasm_push_world_toml`. On native targets `std::fs::read_to_string` is used.
///
/// It does NOT re-anchor the mission clock. This is a MERGE into a live
/// runtime - it extends `trigger_states`, it does not replace them - so the
/// base world's own in-flight `on_timer` triggers and `action_delays` are still
/// scheduled against the running clock, and rewinding it here would postpone
/// every one of them by however long the mission had been going. A genuinely
/// fresh scenario is reached through the lobby instead, and
/// `arm_mission_clock` re-anchors on that path.
fn apply_pending_scenario_loads(
    mut pending: ResMut<PendingScenarioLoad>,
    mut runtime: ResMut<WorldContentRuntime>,
) {
    if pending.0.is_empty() {
        return;
    }

    let paths: Vec<String> = pending.0.drain(..).collect();

    for path in paths {
        // Decisions (dedup / requeue / parse handling) are pure
        // (`world::scenario`); this applier resolves the TOML (I/O) and
        // performs the merges. The dedup check happens before the TOML read
        // so a duplicate never touches the WASM fetch queue.
        let already_loaded = runtime.loaded_scenario_paths.contains(&path);
        let toml_str = if already_loaded {
            None
        } else {
            load_scenario_toml(&path)
        };
        let result = evaluate_scenario_load(&path, already_loaded, toml_str.as_deref());
        for warning in &result.warnings {
            bevy::log::error!("apply_pending_scenario_loads: {warning}");
        }
        if result.requeue {
            // WASM: TOML not yet available; re-queue for the next frame.
            pending.0.push(path);
            continue;
        }
        if result.mark_loaded {
            runtime.loaded_scenario_paths.insert(path);
        }
    }
}

// ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ World layer system (LoadWorld / UnloadWorld) ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬

/// Build a `ConfigCache` suitable for spawning entities from a world layer.
///
/// On WASM the global config cache (pre-loaded by the JS bridge) is returned
/// unchanged.  On native the global cache is always empty (no WASM pre-load
/// step), so we fall back to reading each template file from disk so that
/// `spawn_immediate_entities_internal` can resolve them.
///
/// # What this walk does NOT cover, and the I/O that leaves (issue #973)
///
/// It walks `_world_config.entities` — the layer's static `[[entity]]` blocks —
/// and nothing else. A `spawn_entity` **trigger action**'s `template_path` is
/// therefore not in the cache this returns, while
/// [`crate::world::validate::activation_findings`] checks every spawned
/// instance, triggers included. So on native, the activation gate resolves each
/// distinct trigger template through the filesystem at layer load.
///
/// That is real I/O on a runtime transition that previously did none. It is
/// bounded — once per distinct template per layer load, never per frame, and
/// `#[cfg(target_arch = "wasm32")]` builds never touch a filesystem at all —
/// and it does not move the content digest, because
/// [`crate::content_ledger`] is keyed by canonical path and records the same
/// bytes the eager walk already recorded. Widening this walk to trigger
/// templates would trade the I/O here for the same I/O one step earlier, so it
/// is recorded rather than pre-emptively "fixed".
fn build_layer_config_cache(
    _world_config: &crate::world::config::WorldConfig,
) -> crate::config_cache::ConfigCache {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut cache = crate::config_cache::get_config_cache();
        for entity in &_world_config.entities {
            if cache.contains_key(&entity.template_path) {
                continue;
            }
            if std::fs::metadata(&entity.template_path).is_err()
                && crate::config_cache::mod_pack_overlay_get(&entity.template_path).is_none()
            {
                // Template not on disk (e.g. test fixture); skip silently.
                // spawn_immediate_entities_internal logs and continues for
                // missing templates.
                continue;
            }
            // The `includes` closure resolves here too (issue #869), so a
            // composed hull referenced by a world reaches the layer cache fully
            // merged — the same single document the browser preload assembles.
            match crate::entity_includes::load_entity_config(&entity.template_path) {
                Ok(cfg) => {
                    cache.insert(entity.template_path.clone(), cfg);
                }
                Err(e) => {
                    // Warn only: this builds a cache, it does not decide
                    // whether the world activates. Since #906 the decision
                    // belongs to `validate_template_composition`, which
                    // `spawn_immediate_entities_internal` consults before it
                    // spawns anything — so a composition failure now blocks the
                    // whole world instead of quietly costing it one entity.
                    bevy::log::warn!(
                        "build_layer_config_cache: failed to resolve '{}': {e}",
                        entity.template_path
                    );
                }
            }
        }
        cache
    }

    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::get_config_cache()
    }
}

/// Bevy system: drain `PendingWorldLayerChanges` and apply each `LoadWorld` or
/// `UnloadWorld` command to `WorldLayerMap` and `WorldContentRuntime`.
///
/// `LoadWorld` parses the TOML, merges triggers/comms into the live runtime, and
/// stores a `WorldRuntime` snapshot keyed by path so `UnloadWorld` can reverse it.
///
/// `UnloadWorld` removes the stored snapshot and retains only triggers/comms
/// states that do not belong to the unloaded world (matched by pointer equality
/// of the underlying `Trigger`/`CommsTemplate` clone identity ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â we use indices
/// tracked in the snapshot length at load time).
fn apply_world_layer_changes(
    mut commands: Commands,
    mut pending: ResMut<PendingWorldLayerChanges>,
    mut layer_map: ResMut<WorldLayerMap>,
    mut runtime: ResMut<WorldContentRuntime>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
    // Layer-owned objective cleanup on unload (issue #751). `Option` so bare
    // `App` fixtures without an `ObjectiveManagerRes` still run the loader.
    mut objectives: Option<ResMut<ObjectiveManagerRes>>,
    // Related runtime state to prune when a layer's objectives are removed
    // (issue #752): a captain priority boost pointing at a removed objective,
    // and any per-ship route cursor keyed to it.
    mut captain_boost: Option<ResMut<crate::server_app::CaptainPriorityBoost>>,
    mut objective_cursors_q: Query<&mut crate::ai::server::ObjectiveCursors>,
) {
    if pending.0.is_empty() {
        return;
    }

    let changes: Vec<WorldLayerChange> = pending.0.drain(..).collect();

    for change in changes {
        match change {
            WorldLayerChange::Load { path, loader_path } => {
                // Decisions (dedup / requeue / parse handling / origin
                // tagging / name→UUID assignment) are pure (`world::layers`);
                // this applier resolves the TOML (I/O), spawns entities,
                // merges comms, and mutates the layer map. The dedup check
                // happens before the TOML read so a duplicate never touches
                // the WASM fetch queue.
                let already_loaded = layer_map.0.contains_key(&path);
                let toml_str = if already_loaded {
                    None
                } else {
                    load_scenario_toml(&path)
                };
                let result =
                    evaluate_layer_load(&path, already_loaded, toml_str.as_deref(), || {
                        crate::world_id::mint_id_with(
                            id_mint.as_deref(),
                            crate::world_id::IdNamespace::Entity,
                        )
                    });
                for warning in &result.warnings {
                    bevy::log::error!("apply_world_layer_changes: {warning}");
                }
                match result.outcome {
                    LayerLoadOutcome::AlreadyLoaded => {
                        // De-duplicate, no-op.
                        continue;
                    }
                    LayerLoadOutcome::TomlUnavailable => {
                        // WASM: re-queue until the fetch completes.
                        pending.0.push(WorldLayerChange::Load { path, loader_path });
                    }
                    LayerLoadOutcome::ParseFailed => {
                        // Insert an empty entry so we don't retry a broken file.
                        layer_map.0.insert(path, WorldRuntime::default());
                    }
                    LayerLoadOutcome::Loaded {
                        name_to_uuid_inserts,
                        scenario_config,
                        emit_world_loaded,
                    } => {
                        // Register the layer's named entities in the live
                        // name_to_uuid map. A layer contributes ENTITIES and
                        // nothing else since issue #985 — see `world::layers`.
                        for (name, uuid) in name_to_uuid_inserts {
                            runtime.name_to_uuid.insert(name, uuid);
                        }

                        // Spawn the layer's [[entity]] blocks into the ECS.
                        // On native the global config cache is always empty (no WASM
                        // pre-load step), so we build a local cache by reading each
                        // referenced template from disk.  WASM uses the pre-loaded
                        // global cache as normal.
                        let config_cache = build_layer_config_cache(&scenario_config);
                        let spawned_entities = spawn_immediate_entities_internal(
                            &mut commands,
                            &scenario_config,
                            &config_cache,
                            Some(&runtime.flags),
                            id_mint.as_deref(),
                        );
                        // Stamp each entity's origin layer (issue #891 review
                        // finding 1) so `entity_flag_chain` can read it in
                        // O(1) instead of scanning `WorldLayerMap` for it
                        // later — the second of the two spawn sites.
                        for &spawned in &spawned_entities {
                            commands
                                .entity(spawned)
                                .insert(EntityOriginLayer(path.clone()));
                        }

                        // Issue #415: emit a WorldLoaded event so
                        // `on_world_loaded` triggers declared inside
                        // this sub-world (and merged into the live
                        // runtime above) fire on the next Update
                        // tick via `tick_trigger_pipeline`.
                        if emit_world_loaded {
                            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
                        }

                        let delayed_unload_resolve = matches!(
                            scenario_config.delayed_unload_policy,
                            crate::world::config::DelayedUnloadPolicy::Resolve
                        );
                        layer_map.0.insert(
                            path,
                            WorldRuntime {
                                spawned_entities,
                                anchors: scenario_config.anchors.clone(),
                                flags: crate::world::flags::FlagStore::new(),
                                loader_path,
                                owned_objective_ids: Vec::new(),
                                delayed_unload_resolve,
                            },
                        );
                    }
                }
            }
            WorldLayerChange::Unload(path) => {
                let Some(layer) = layer_map.0.remove(&path) else {
                    continue; // Not loaded - no-op.
                };

                // Despawn ECS entities that were spawned when this layer loaded.
                // Use try_despawn: entities may have already died (e.g. hull = 0) before the layer unloads.
                for entity in &layer.spawned_entities {
                    commands.entity(*entity).try_despawn();
                }

                // No trigger states to remove: a layer has contributed none
                // since issue #985 deleted the `[[trigger]]` parser, which was
                // the only way one could. Script-in-layers (#1045) is where the
                // removal set comes back.

                // Remove objectives this layer's triggers added (issue #751)
                // and prune the runtime state that referenced them (issue #752):
                // a captain priority boost pointing at a removed objective, and
                // any per-ship route cursor keyed to it. Otherwise a stale boost
                // would keep re-scoring a gone objective and a re-added same-id
                // objective would inherit the old cursor's waypoint index.
                for id in &layer.owned_objective_ids {
                    if let Some(obj) = objectives.as_deref_mut() {
                        obj.0.remove(id);
                    }
                    if let Some(boost) = captain_boost.as_deref_mut() {
                        boost.prune_objective(id);
                    }
                    for mut cursors in objective_cursors_q.iter_mut() {
                        cursors.0.retain(|c| &c.objective_id != id);
                    }
                }

                // Cancel or resolve this layer's pending delayed actions by
                // the authored policy (issue #751). Pure partition; the
                // resolved actions are pulled to fire immediately on the next
                // `tick_delayed_actions`.
                let queued = std::mem::take(&mut runtime.pending_delayed_actions);
                runtime.pending_delayed_actions =
                    crate::world::delayed::partition_delayed_actions_on_unload(
                        queued,
                        &path,
                        layer.delayed_unload_resolve,
                    );
            }
        }
    }
}

/// Load a world TOML string for the given path.
///
/// - **Native**: uses `std::fs::read_to_string` (for tests and dev builds).
/// - **WASM**: checks the pending world TOML queue populated by JS via
///   `wasm_push_world_toml`; returns `None` if the fetch is not yet complete.
fn load_scenario_toml(path: &str) -> Option<String> {
    // Issue #935: layer worlds loaded through this function (extra worlds,
    // additive layers) are authored content too — a designer editing one
    // moves nothing in the content digest unless it is recorded. This does
    // NOT reset the ledger: a layer load is additive to the same run, not a
    // new scenario/world load (see `content_ledger`'s reset-semantics docs).
    let text = load_scenario_toml_text(path);
    if let Some(text) = &text {
        crate::content_ledger::record(path, text);
    }
    text
}

fn load_scenario_toml_text(path: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::config_cache::pop_pending_world_toml(path).or_else(|| {
            // Fire a JS fetch request if we haven't already.
            crate::config_cache::request_world_fetch(path.to_string());
            None
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ai_plugin::{AiEntityAttacked, AiEntityDestroyed};
    use crate::comms::server::{CommsChannel2Event, CommsInboxRes, CommsRuntime};
    use crate::console::comms::server::handle_comms_channel2;
    use crate::lobby::LobbyPlugin;
    use crate::messages::*;
    use crate::world::content::TriggerCondition;

    // ── The shared layered flag chain (issue #891 stage 2) ───────────────────

    /// A layer map with one loaded sub-world whose `loader_path` is the base
    /// world, carrying one flag of its own.
    fn one_layer_map(path: &str, flag: &str) -> WorldLayerMap {
        let mut layer = WorldRuntime::default();
        layer.flags.set_flag(flag);
        let mut map = WorldLayerMap::default();
        map.0.insert(path.to_string(), layer);
        map
    }

    #[test]
    fn layered_flag_chain_walks_loader_path_innermost_first() {
        let mut base = crate::world::flags::FlagStore::default();
        base.set_flag("base_flag");
        let mut inner = WorldRuntime {
            loader_path: Some("assets/worlds/outer.toml".into()),
            ..Default::default()
        };
        inner.flags.set_flag("inner_flag");
        let mut outer = WorldRuntime::default();
        outer.flags.set_flag("outer_flag");
        let mut map = WorldLayerMap::default();
        map.0.insert("assets/worlds/inner.toml".into(), inner);
        map.0.insert("assets/worlds/outer.toml".into(), outer);

        // The store-only walk (`layered_flag_chain`) — the shape every AI
        // host reads through `entity_flag_chain`.
        let flags = layered_flag_chain(Some("assets/worlds/inner.toml"), &base, Some(&map));
        // Innermost-first: an unprefixed name reads the origin layer; each
        // `parent:` steps one entry outward — the reader's own scope first,
        // never a flattened union.
        assert!(crate::world::flags::flag_in_chain(&flags, "inner_flag"));
        assert!(!crate::world::flags::flag_in_chain(&flags, "outer_flag"));
        assert!(crate::world::flags::flag_in_chain(
            &flags,
            "parent:outer_flag"
        ));
        assert!(crate::world::flags::flag_in_chain(
            &flags,
            "parent:parent:base_flag"
        ));

        // The with-paths wrapper (`layered_flag_chain_with_paths`) —
        // `tick_trigger_pipeline`'s own shape — carries the identical store
        // chain plus the layer-path chain climbing to the base world.
        let (flags2, layers) =
            layered_flag_chain_with_paths(Some("assets/worlds/inner.toml"), &base, Some(&map));
        assert_eq!(
            layers,
            vec![
                Some("assets/worlds/inner.toml".to_string()),
                Some("assets/worlds/outer.toml".to_string()),
                None
            ],
            "the layer-path chain climbs loader_path to the base world"
        );
        assert!(crate::world::flags::flag_in_chain(&flags2, "inner_flag"));
    }

    #[test]
    fn layered_flag_chain_from_the_base_world_is_the_base_store_alone() {
        let mut base = crate::world::flags::FlagStore::default();
        base.set_flag("base_flag");
        let flags = layered_flag_chain(None, &base, None);
        assert_eq!(flags.len(), 1);
        assert!(crate::world::flags::flag_in_chain(&flags, "base_flag"));

        let (flags2, layers) = layered_flag_chain_with_paths(None, &base, None);
        assert_eq!(layers, vec![None]);
        assert_eq!(flags2.len(), 1);
        assert!(crate::world::flags::flag_in_chain(&flags2, "base_flag"));
    }

    /// The AI-host entry point: a ship carrying an `EntityOriginLayer`
    /// component is anchored at that layer, exactly as a trigger authored
    /// there; a ship with no component is anchored at the base.
    #[test]
    fn entity_flag_chain_anchors_at_the_spawning_layer() {
        let mut app = App::new();
        let layer_ship = app
            .world_mut()
            .spawn(EntityOriginLayer("assets/worlds/sub.toml".to_string()))
            .id();
        let base_ship = app.world_mut().spawn_empty().id();

        let mut runtime = WorldContentRuntime::default();
        runtime.flags.set_flag("base_flag");
        let map = one_layer_map("assets/worlds/sub.toml", "layer_flag");

        let origin = app.world().get::<EntityOriginLayer>(layer_ship);
        let chain = entity_flag_chain(origin, Some(&runtime), Some(&map));
        assert!(
            crate::world::flags::flag_in_chain(&chain, "layer_flag"),
            "a layer-spawned ship reads its own layer's store first"
        );
        assert!(
            crate::world::flags::flag_in_chain(&chain, "parent:base_flag"),
            "and climbs to the base store through `parent:`"
        );

        let origin = app.world().get::<EntityOriginLayer>(base_ship);
        let chain = entity_flag_chain(origin, Some(&runtime), Some(&map));
        assert!(
            crate::world::flags::flag_in_chain(&chain, "base_flag"),
            "an unrecorded ship is anchored at the base world"
        );
        assert!(!crate::world::flags::flag_in_chain(&chain, "layer_flag"));

        // No runtime (bare-`App` fixtures): the chain is empty and every read
        // is false.
        let chain = entity_flag_chain(origin, None, Some(&map));
        assert!(chain.is_empty());
    }

    // -- AI-event trigger tests -----------------------------------------------

    /// Build a minimal test app that includes just what tick_trigger_pipeline needs.
    pub(crate) fn ai_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::ai_plugin::AiPlugin)
            .insert_resource(crate::config_cache::FactionRegistryResource(
                crate::config_cache::get_faction_registry(),
            ))
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .init_resource::<WorldEventBuffer>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (
                    collect_world_events,
                    tick_trigger_pipeline,
                    handle_comms_channel2,
                )
                    .chain(),
            );
        // Set phase to InProgress
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app
    }

    /// Queue `actions` on the delayed-action queue, due immediately, and step
    /// one tick so `tick_delayed_actions` dispatches them in queue order.
    ///
    /// This is where the action-dispatch tests below are driven from. Issue #985
    /// deleted the `[[trigger]]` action array, so a fired trigger dispatches
    /// nothing of its own any more and the delayed-action queue is the surviving
    /// [`TriggerAction`] consumer. `origin_layer` and `entity_name` are the two
    /// context fields a firing trigger used to supply; they now travel on the
    /// [`DelayedAction`] itself and are read straight back out by
    /// `tick_delayed_actions` when it builds each `DispatchContext`.
    fn dispatch_delayed_actions(
        app: &mut App,
        origin_layer: Option<&str>,
        entity_name: Option<&str>,
        actions: Vec<TriggerAction>,
    ) {
        app.add_systems(Update, tick_delayed_actions.after(tick_trigger_pipeline));
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.mission_clock_anchor_secs = Some(0.0);
            for action in actions {
                runtime.pending_delayed_actions.push(DelayedAction {
                    action,
                    origin_layer: origin_layer.map(str::to_string),
                    entity_name: entity_name.map(str::to_string),
                    fire_at_elapsed: 0.0,
                });
            }
        }
        app.update();
    }

    /// The [`dispatch_delayed_actions`] shape for a test that needs nothing from
    /// the app before the dispatch: builds the fixture app, dispatches, and hands
    /// the stepped app back so the test can read the mutated resources.
    fn dispatch_delayed_actions_in_new_app(actions: Vec<TriggerAction>) -> App {
        let mut app = ai_trigger_test_app();
        dispatch_delayed_actions(&mut app, None, None, actions);
        app
    }

    /// Issue #710: the `DispatchContext` flag stores must be the LIVE ones,
    /// never a copy taken before dispatch began. `dispatch_action` computes
    /// before/after against them to decide whether a transition event fires at
    /// all, so a snapshot silently drops transitions.
    ///
    /// Set-then-clear within one drain is the discriminating case: against the
    /// live store the clear reads before = 1 and emits `FlagCleared`; against a
    /// snapshot it reads before = 0, sees no change, and emits nothing.
    ///
    /// The pair used to be two triggers firing in a single `tick_trigger_pipeline`
    /// pass; issue #985 deleted the `[[trigger]]` action array they dispatched
    /// through, so they are queued on the delayed-action path instead — which
    /// re-projects the same live stores once per action, for the same reason.
    #[test]
    fn flag_stores_handed_to_dispatch_are_live_within_one_drain() {
        let mut app = ai_trigger_test_app();
        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![
                TriggerAction::SetWorldFlag { name: "a".into() },
                TriggerAction::ClearWorldFlag { name: "a".into() },
            ],
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.flags.counter("a"),
            0,
            "set-then-clear must end with the flag cleared"
        );
        // Both transitions, in order. Against a per-action snapshot the clear
        // would read before = 0, emit nothing, and only the `FlagSet` would be
        // here.
        assert_eq!(
            runtime.pending_world_events,
            vec![
                WorldEvent::FlagSet {
                    name: "a".into(),
                    origin_layer: None,
                },
                WorldEvent::FlagCleared {
                    name: "a".into(),
                    origin_layer: None,
                },
            ],
            "clear_flag must emit FlagCleared against the live store"
        );
    }

    /// Issue #710: `tick_delayed_actions` now shares the `world::dispatch`
    /// table with `tick_trigger_pipeline`. The routing of `new_events` is the one
    /// thing that must stay different: a delayed action's events queue onto
    /// `pending_world_events` for the NEXT tick, because this system runs after
    /// `tick_trigger_pipeline` has already drained that queue for this one.
    #[test]
    fn delayed_action_dispatches_and_queues_event_for_next_tick() {
        let mut app = ai_trigger_test_app();
        app.add_systems(Update, tick_delayed_actions.after(tick_trigger_pipeline));

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.mission_clock_anchor_secs = Some(0.0);
            runtime.pending_delayed_actions.push(DelayedAction {
                action: TriggerAction::SetWorldFlag {
                    name: "aphelion_armed".to_string(),
                },
                origin_layer: None,
                entity_name: None,
                fire_at_elapsed: 0.0,
            });
        }

        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.flags.counter("aphelion_armed"),
            1,
            "delayed set_flag must mutate the live base flag store"
        );
        assert!(
            runtime.pending_delayed_actions.is_empty(),
            "the fired delayed action must be drained"
        );
        assert_eq!(
            runtime.pending_world_events,
            vec![WorldEvent::FlagSet {
                name: "aphelion_armed".to_string(),
                origin_layer: None,
            }],
            "a delayed action's new_events must queue for the NEXT tick"
        );
    }

    /// Issue #981 (Rhai M3): a scripted `in_seconds(n).<verb>(…)` delayed effect
    /// routes through the EXISTING `tick_delayed_actions` queue — the builder
    /// hands the host a `DelayedAction` (the same struct a TOML `action_delays`
    /// entry produces), and once on `pending_delayed_actions` it dispatches
    /// exactly as a declaratively-delayed action does. No second scheduler.
    #[test]
    fn scripted_in_seconds_effect_routes_through_tick_delayed_actions() {
        use crate::world::script::engine::RuntimeHost;
        use crate::world::script::schedule::{SchedClock, TickBudget};
        use rhai::Map;

        // Run a handler that schedules a delayed `complete_objective`, and collect
        // the `DelayedAction`s it produced.
        let host = RuntimeHost::new();
        let ast = host
            .engine()
            .compile(
                r#"fn on_x(ctx) {
                    ctx.schedule.in_seconds(0).complete_objective("aphelion_secured");
                }"#,
            )
            .expect("compiles");
        let mut budget = TickBudget::new();
        // A zero clock: the delay of 0s makes the action due immediately
        // (`fire_at_elapsed = 0`).
        let clock = SchedClock {
            tick: 0,
            elapsed_secs: 0.0,
            tick_hz: 60.0,
        };
        let effects = host.call(
            &mut budget,
            &clock,
            &ast,
            "world.toml#script.setup",
            "on_x",
            &crate::world::flags::FlagStore::new(),
            &crate::world::deadlines::DeadlineTable::default(),
            &crate::world::commitments::CommitmentLedger::default(),
            Map::new(),
        );
        assert_eq!(effects.delayed.len(), 1, "one delayed effect was scheduled");

        // Wire the real system and seed the objective the delayed effect targets.
        let mut app = ai_trigger_test_app();
        app.add_systems(Update, tick_delayed_actions.after(tick_trigger_pipeline));
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "aphelion_secured",
            "secure the aphelion",
            true,
            vec![],
        );
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.mission_clock_anchor_secs = Some(0.0);
            // The route: the script-scheduled DelayedAction goes onto the SAME
            // `pending_delayed_actions` queue a TOML `action_delays` entry uses.
            runtime.pending_delayed_actions.extend(effects.delayed);
        }

        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_delayed_actions.is_empty(),
            "the due script-scheduled action must be drained by tick_delayed_actions"
        );
        let objectives = app.world().resource::<ObjectiveManagerRes>();
        let snapshot = objectives
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "aphelion_secured")
            .expect("the objective exists");
        assert_eq!(
            snapshot.status,
            ObjectiveStatus::Completed,
            "tick_delayed_actions must dispatch the scripted delayed complete_objective"
        );
    }

    /// A test resolver that reads no sibling files — inline `[script]` blocks
    /// are lifted from the TOML directly and never consult a resolver.
    struct NoScriptResolver;
    impl crate::world::script::load::ScriptResolver for NoScriptResolver {
        fn read(&self, _path: &str) -> Option<String> {
            None
        }
    }

    /// Build a `WorldScriptRuntime` from an inline-`[script]` fixture world the
    /// SAME way `compile_world_scripts` does in production.
    fn compile_fixture_scripts(world_toml: &str) -> WorldScriptRuntime {
        let value: toml::Value = toml::from_str(world_toml).expect("valid fixture toml");
        let compiled = crate::world::script::load::load_world_scripts(
            "fixture/scripted.toml",
            &value,
            &NoScriptResolver,
        );
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "fixture scripts must compile clean: {:?}",
            compiled.findings
        );
        WorldScriptRuntime {
            host: RuntimeHost::new(),
            asts: compiled.asts,
            triggers: compiled.script_triggers,
            handlers: Vec::new(),
            budget: TickBudget::new(),
            budget_tick: 0,
            content_hash: compiled.content_hash,
            pending_callbacks: PendingCallbacks::new(),
            pending_comms_opens: Vec::new(),
            deadline_handlers: Vec::new(),
        }
    }

    /// Issue #984 (Rhai M6 phase 2a), smallest-slice proof: a scripted trigger
    /// authored inline fires through the LIVE `tick_trigger_pipeline` (not a
    /// direct `RuntimeHost` call) and its handler's `complete_objective` effect
    /// reaches the objective manager. Drives the real `ai_trigger_test_app`
    /// harness with the same merge (`merge_script_triggers`) + dispatch path
    /// production uses.
    #[test]
    fn scripted_on_destroyed_completes_objective_through_the_live_pipeline() {
        // One inline `[script]` trigger: on the raider's death, complete "obj".
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.effects.complete_objective("obj"); }'
"#,
        );
        assert_eq!(sr.triggers.len(), 1, "one scripted trigger authored");

        let mut app = ai_trigger_test_app();
        let raider_uuid = "raider-uuid-984a";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            // No declarative triggers; merge appends the scripted one and builds
            // the parallel handler table.
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
            assert_eq!(runtime.trigger_states.len(), 1);
        }
        assert_eq!(sr.handlers.len(), 1);
        assert!(
            sr.handlers[0].is_some(),
            "the scripted index carries a handler"
        );
        app.world_mut().insert_resource(sr);
        // Seed the objective the scripted handler completes.
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "obj",
            "hold the line",
            true,
            vec![],
        );

        // Drive the live pipeline: emit the destruction event and step once.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        let objectives = app.world().resource::<ObjectiveManagerRes>();
        let snap = objectives
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "obj")
            .expect("the objective exists");
        assert_eq!(
            snap.status,
            ObjectiveStatus::Completed,
            "a scripted on_destroyed handler must complete the objective through the LIVE pipeline"
        );
    }

    /// Issue #984: a scripted handler's `ctx.effects.open_comms(#{…})` queues on
    /// `WorldScriptRuntime::pending_comms_opens` through the LIVE
    /// `tick_trigger_pipeline` — the routing line beside `pending_callbacks`,
    /// proven end-to-end rather than by calling the host directly. The comms
    /// module has nothing draining that queue yet (the scripted-thread system is
    /// the next slice), so this pins the wiring, not the thread.
    #[test]
    fn scripted_open_comms_queues_on_the_runtime_through_the_live_pipeline() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "hail"); fn hail(ctx) { ctx.effects.open_comms(#{ from: "axiom", node_fn: "hail_axiom", display_name: "Axiom Control", urgent: true }); } fn hail_axiom(ctx) { #{ message: "Axiom Station, go ahead.", responses: [] } }'
"#,
        );

        let mut app = ai_trigger_test_app();
        let raider_uuid = "raider-uuid-984c";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        assert!(
            sr.pending_comms_opens.is_empty(),
            "nothing queued before the trigger fires"
        );
        app.world_mut().insert_resource(sr);

        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        let sr = app.world().resource::<WorldScriptRuntime>();
        assert_eq!(
            sr.pending_comms_opens.len(),
            1,
            "the fired handler's open must reach the runtime queue"
        );
        let open = &sr.pending_comms_opens[0];
        assert_eq!(open.from, "axiom");
        assert_eq!(open.root_fn, "hail_axiom");
        assert_eq!(open.display_name.as_deref(), Some("Axiom Control"));
        assert_eq!(open.thread_id, None);
        assert!(open.urgent);
        assert_eq!(
            open.script_path, "fixture/scripted.toml#script.setup",
            "the host stamps the running unit's path, so the node fn resolves \
             against the right AST"
        );
    }

    /// Issue #984 (Rhai M6 phase 2a), flag-chaining proof: a scripted handler's
    /// `ctx.flags` write emits `ActionCmd::MutateFlag` directly, bypassing the
    /// `push_flag_transition` a declarative `set_flag` gets. `apply_script_commands`
    /// restores the transition event by previewing the write against the live
    /// store, so a downstream declarative `on_flag_set` trigger chains in the
    /// next pass. Without the preview the write would apply silently and the
    /// watcher would never fire.
    ///
    /// The watcher is observed through its own `fired` latch: issue #985 deleted
    /// the `[[trigger]]` action array this used to read a chained objective out
    /// of, and a trigger's latch is now the whole of what a declarative-shaped
    /// trigger records when it fires.
    #[test]
    fn scripted_flag_write_chains_a_declarative_on_flag_set() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "arm"); fn arm(ctx) { ctx.flags.armed = 1; }'
"#,
        );

        let mut app = ai_trigger_test_app();
        let raider_uuid = "raider-uuid-984b";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            // Declarative watcher on_flag_set("armed"); the scripted
            // on_destroyed trigger is appended AFTER it by the merge, so the
            // watcher keeps index 0.
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagSet {
                        name: "armed".to_string(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);

        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.trigger_states[0].fired,
            "a scripted flag write must emit FlagSet so the declarative on_flag_set \
             trigger chains in the next pass (the apply_script_commands preview)"
        );
        assert_eq!(
            runtime.flags.counter("armed"),
            1,
            "the scripted flag write itself must land on the live store"
        );
    }

    /// Issue #984 (Rhai M6 phase 2a), flag-chaining proof for a CLEARED
    /// transition: a scripted handler writes `ctx.flags.shields_up = 0` (drains an
    /// absolute `SetValue(0)`) over a pre-set flag. `apply_script_commands`
    /// previews that against the live store — before 1, after 0 — so
    /// `push_flag_transition`'s boolean flip emits `FlagCleared`, and a downstream
    /// declarative `on_flag_cleared` trigger chains in the next pass. Without the
    /// preview the write would apply silently and the watcher would never fire.
    ///
    /// Observed through the watcher's own `fired` latch — issue #985 deleted the
    /// `[[trigger]]` action array the chained objective used to come from.
    #[test]
    fn scripted_flag_clear_chains_a_declarative_on_flag_cleared() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "disarm"); fn disarm(ctx) { ctx.flags.shields_up = 0; }'
"#,
        );

        let mut app = ai_trigger_test_app();
        let raider_uuid = "raider-uuid-984c";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // Pre-set so the scripted write is a true 1 -> 0 transition.
            runtime.flags.set_flag("shields_up");
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            // Declarative watcher on_flag_cleared("shields_up"); the scripted
            // on_destroyed trigger is appended AFTER it by the merge, so the
            // watcher keeps index 0.
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagCleared {
                        name: "shields_up".to_string(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);

        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.trigger_states[0].fired,
            "a scripted flag clear must emit FlagCleared so the declarative \
             on_flag_cleared trigger chains in the next pass (the \
             apply_script_commands preview)"
        );
        assert_eq!(
            runtime.flags.counter("shields_up"),
            0,
            "the scripted flag clear itself must land on the live store"
        );
    }

    /// Issue #984 (Rhai M6 phase 2a), flag-chaining proof for an INCREMENT
    /// transition: a scripted handler calls `ctx.flags.increment("wave", 1)`
    /// (drains a relative `Increment(1)`) over an unset counter.
    /// `apply_script_commands` previews that against the live store — before 0,
    /// after 1 — so `push_flag_transition`'s boolean flip emits `FlagSet`, and a
    /// downstream declarative `on_flag_set` trigger chains in the next pass.
    /// Without the preview the increment would apply silently and the watcher
    /// would never fire.
    ///
    /// Observed through the watcher's own `fired` latch — issue #985 deleted the
    /// `[[trigger]]` action array the chained objective used to come from.
    #[test]
    fn scripted_flag_increment_chains_a_declarative_on_flag_set() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "bump"); fn bump(ctx) { ctx.flags.increment("wave", 1); }'
"#,
        );

        let mut app = ai_trigger_test_app();
        let raider_uuid = "raider-uuid-984d";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // `wave` starts unset (counter 0), so the +1 is a true 0 -> 1
            // transition (0 is "not set", 1 is "set").
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            // Declarative watcher on_flag_set("wave"); the scripted on_destroyed
            // trigger is appended AFTER it by the merge, so the watcher keeps
            // index 0.
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagSet {
                        name: "wave".to_string(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);

        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.trigger_states[0].fired,
            "a scripted flag increment from 0 must emit FlagSet so the declarative \
             on_flag_set trigger chains in the next pass (the apply_script_commands \
             preview)"
        );
        assert_eq!(
            runtime.flags.counter("wave"),
            1,
            "the scripted flag increment itself must land on the live store"
        );
    }

    /// Issue #984 (Rhai M6), the DIGEST-CRITICAL guard: a scripted `spawn_entity`
    /// and the same `TriggerAction::SpawnEntity` dispatched through the host, each
    /// driven with a fresh, identically-seeded `WorldIdMint`, mint the IDENTICAL
    /// `EntityUuid`. This is what a converted world's authoritative digest (#894)
    /// rides on, and it catches the P2a-class divergence risks: minting at the
    /// effects.rs boundary or via the process-global fallback (R1), a stubbed uuid
    /// source rather than the real one (R2), or dropping the name→uuid insert by
    /// applying only `.commands` instead of the whole `DispatchResult` (R3) would
    /// each break the equality below.
    ///
    /// The non-scripted half used to be an authored `[[trigger.action]]` twin;
    /// issue #985 deleted that front-end, so it is dispatched through
    /// `tick_delayed_actions` instead — which binds the SAME `uuid_source` closure
    /// over the same `WorldIdMint`, so it is still the mint path a declarative
    /// action took.
    #[test]
    fn scripted_and_dispatched_spawn_mint_the_same_entity_uuid() {
        use crate::entity_config::EntityConfig;

        // A trivial template both paths spawn, served from the native config cache
        // so the live `WasmTemplateLoader` resolves it with no files on disk.
        crate::config_cache::insert_native_config(
            "fixture/harrow_mint.toml".to_string(),
            EntityConfig::from_toml("").unwrap(),
        );

        let raider_uuid = "raider-uuid-mint";

        // ---- Scripted: the handler spawns "wave" from a `#{ … }` map. ----
        let scripted_uuid = {
            let mut sr = compile_fixture_scripts(
                r#"[script]
setup = 'on_destroyed("raider", "spawn_wave"); fn spawn_wave(ctx) { ctx.effects.spawn_entity(#{ template_path: "fixture/harrow_mint.toml", name: "wave", position: [100, 0, 0], groups: ["hostiles"] }); }'
"#,
            );
            let mut app = ai_trigger_test_app();
            // A fresh mint (tick 0, every sequence 0): the spawn is the first Entity
            // id minted, so its sequence is deterministic and shared by both paths.
            app.world_mut()
                .insert_resource(crate::world_id::WorldIdMint::default());
            {
                let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
                runtime
                    .name_to_uuid
                    .insert("raider".to_string(), raider_uuid.to_string());
                runtime.trigger_states = Vec::new();
                merge_script_triggers(&mut runtime.trigger_states, &mut sr);
            }
            app.world_mut().insert_resource(sr);
            app.world_mut()
                .resource_mut::<Messages<AiEntityDestroyed>>()
                .write(AiEntityDestroyed {
                    entity_uuid: raider_uuid.into(),
                });
            app.update();
            app.world()
                .resource::<WorldContentRuntime>()
                .name_to_uuid
                .get("wave")
                .cloned()
        };

        // ---- Dispatched twin: the SAME spawn as a `TriggerAction`. ----
        let dispatched_uuid = {
            let mut app = ai_trigger_test_app();
            app.world_mut()
                .insert_resource(crate::world_id::WorldIdMint::default());
            dispatch_delayed_actions(
                &mut app,
                None,
                Some("raider"),
                vec![TriggerAction::SpawnEntity {
                    template_path: "fixture/harrow_mint.toml".to_string(),
                    name: "wave".to_string(),
                    anchor: None,
                    position: Some([100.0, 0.0, 0.0]),
                    rotation: None,
                    scale: None,
                    groups: vec!["hostiles".to_string()],
                    overrides: None,
                }],
            );
            app.world()
                .resource::<WorldContentRuntime>()
                .name_to_uuid
                .get("wave")
                .cloned()
        };

        assert!(
            scripted_uuid.is_some(),
            "the scripted spawn must register a name→uuid entry — proving the whole \
             DispatchResult (with its inserts) was applied, not just .commands (R3)"
        );
        assert_eq!(
            scripted_uuid, dispatched_uuid,
            "a scripted spawn must mint the SAME EntityUuid as the dispatched \
             TriggerAction — the digest-critical mint-order parity (R1/R2/R3)"
        );
    }

    // ── Scripted destruction (issue #1033) ───────────────────────────────────

    /// Spawn `name` as a live ECS entity carrying `uuid`, registered in
    /// `name_to_uuid` — the shape a world-spawned entity has by the time a script
    /// can address it by name.
    fn register_named_entity(app: &mut App, name: &str, uuid: &str) -> Entity {
        use crate::entities::spawner::EntityUuid;
        let entity = app
            .world_mut()
            .spawn((
                EntityUuid(uuid.to_string()),
                bevy::prelude::Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert(name.to_string(), uuid.to_string());
        entity
    }

    /// Issue #1033, THE acceptance: a scripted `destroy_entity` chains its
    /// `WorldEvent::Destroyed` into the SAME tick, so a downstream `on_destroyed`
    /// trigger fires off a scripted removal exactly as it does off a combat kill.
    ///
    /// The chain is the whole point of buffering a `TriggerAction` instead of a
    /// resolved command. `dispatch_destroy_entity` returns the event on
    /// `DispatchResult::new_events` beside the despawn command;
    /// `apply_script_commands` feeds the WHOLE result to `apply_dispatch_result`,
    /// whose `events_out` is `tick_trigger_pipeline`'s `next_events` — the current
    /// tick's next chaining pass. So the watcher below fires inside the single
    /// `app.update()` that ran the handler, with no second tick to hide behind.
    ///
    /// Three things are asserted together because a partial implementation can
    /// deliver any one of them alone: the entity is really gone, the chained
    /// trigger really fired, and the `AiEntityDestroyed` message really reached
    /// external consumers (telemetry, save/load) the way a combat kill's does.
    #[test]
    fn a_scripted_destroy_chains_on_destroyed_in_the_same_tick() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "collapse"); fn collapse(ctx) { ctx.effects.destroy_entity("skyhook"); }'
"#,
        );

        let mut app = ai_trigger_test_app();
        let raider_uuid = "raider-uuid-1033a";
        let skyhook_uuid = "skyhook-uuid-1033a";
        let skyhook = register_named_entity(&mut app, "skyhook", skyhook_uuid);
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            // The witness, at index 0: a declarative `on_destroyed` watching the
            // SKYHOOK — an entity nothing shoots at. Only the scripted destruction
            // can make this fire.
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "skyhook".into(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);

        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        assert!(
            app.world().get_entity(skyhook).is_err(),
            "the scripted destroy must despawn the named entity"
        );
        assert!(
            trigger_fired(&app, 0),
            "the chained on_destroyed must fire in the SAME tick — the scripted \
             destroy's Destroyed event rides `new_events` into this tick's next \
             chaining pass, exactly as a combat kill's does"
        );
        let msgs = app
            .world()
            .resource::<Messages<crate::ai_plugin::AiEntityDestroyed>>();
        let mut cursor = msgs.get_cursor();
        let emitted: Vec<String> = cursor.read(msgs).map(|m| m.entity_uuid.clone()).collect();
        assert!(
            emitted.iter().any(|u| u == skyhook_uuid),
            "external consumers must see a scripted kill as they see a combat one, \
             got {emitted:?}"
        );
    }

    /// Issue #1033, the group half of the acceptance: a script destroys group
    /// members one at a time, and the group's `OnAllDestroyed` fires when — and
    /// only when — the LAST one goes.
    ///
    /// This is the assertion that catches a cleanup that looks tidy and is not.
    /// `OnAllDestroyed` resolves each `Destroyed` event back to a member NAME
    /// through `name_to_uuid`, and reads the membership out of `entity_groups`;
    /// removing the destroyed entity from either — the intuitive "clean up on
    /// removal" — makes the last kill unresolvable and the group never fires. A
    /// combat kill removes neither, so neither does a scripted one, and the two
    /// paths stay identical by doing the same nothing.
    ///
    /// The negative half (one down, group silent) is what makes the positive half
    /// mean anything: a trigger that fired on the FIRST kill would satisfy a
    /// fires-at-the-end assertion just as well.
    #[test]
    fn a_scripted_destroy_of_the_last_group_member_fires_on_all_destroyed() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = '''
on_destroyed("cue_a", "kill_a");
on_destroyed("cue_b", "kill_b");
fn kill_a(ctx) { ctx.effects.destroy_entity("band_a"); }
fn kill_b(ctx) { ctx.effects.destroy_entity("band_b"); }
'''
"#,
        );

        let mut app = ai_trigger_test_app();
        let band_a = register_named_entity(&mut app, "band_a", "band-a-uuid-1033");
        let band_b = register_named_entity(&mut app, "band_b", "band-b-uuid-1033");
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("cue_a".to_string(), "cue-a-uuid-1033".to_string());
            runtime
                .name_to_uuid
                .insert("cue_b".to_string(), "cue-b-uuid-1033".to_string());
            // The storm band, as a spawned group would register it.
            runtime.entity_groups.insert(
                "band".to_string(),
                ["band_a".to_string(), "band_b".to_string()]
                    .into_iter()
                    .collect(),
            );
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAllDestroyed {
                        group: "band".into(),
                        after_secs: 0.0,
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);

        // First member down.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: "cue-a-uuid-1033".into(),
            });
        app.update();
        assert!(
            app.world().get_entity(band_a).is_err(),
            "the first scripted destroy must despawn its target"
        );
        assert!(
            !trigger_fired(&app, 0),
            "one of two members down is not the whole group — OnAllDestroyed must \
             stay silent"
        );

        // Last member down.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: "cue-b-uuid-1033".into(),
            });
        app.update();
        assert!(app.world().get_entity(band_b).is_err());
        assert!(
            trigger_fired(&app, 0),
            "the LAST scripted destroy must fire the group's OnAllDestroyed — the \
             storm-band teardown this effect exists for"
        );
    }

    /// Issue #1033: destroying a name nothing registered is a warning-only no-op —
    /// no panic, no despawn, and NO `Destroyed` event for a chained trigger to
    /// react to. Matching `dispatch_spawn_entity`'s unresolvable-template
    /// contingency, and for the same reason: a phantom event would fire
    /// `on_destroyed` for something that never existed.
    ///
    /// Driven through the scripted path (not `dispatch_action` directly) so it also
    /// covers the raise-vs-warn choice: an unknown name is a RUNTIME miss the
    /// dispatcher warns about, not a malformed call the host fn rejects, so the
    /// rest of the handler's effects still apply.
    #[test]
    fn a_scripted_destroy_of_an_unknown_name_warns_and_keeps_the_rest_of_the_call() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.effects.destroy_entity("no_such_entity"); ctx.effects.complete_objective("obj"); }'
"#,
        );

        let mut app = ai_trigger_test_app();
        let raider_uuid = "raider-uuid-1033c";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "no_such_entity".into(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "obj",
            "hold the line",
            true,
            vec![],
        );

        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        assert!(
            !trigger_fired(&app, 0),
            "an unresolvable destroy must emit no Destroyed event at all"
        );
        assert!(
            objective_is_completed(&app, "obj"),
            "the miss is a dispatch warning, not a raise: the effects authored \
             after it still apply"
        );
    }

    /// Issue #1033 AC4: a name freed by a destroy is reusable — a later
    /// `spawn_entity` under the same name resolves to the NEW entity, with no
    /// stale entry in the way.
    ///
    /// Both effects run in one handler, in authored order, which is the sharpest
    /// version: the spawn's `name_to_uuid` insert lands after the destroy in the
    /// same buffer drain. It works because the map is written by INSERT (an
    /// overwrite), so the stale entry is replaced rather than needing removal —
    /// which is what lets the destroy leave the entry alone for the chaining pass
    /// above to resolve against.
    #[test]
    fn a_name_freed_by_a_scripted_destroy_is_reusable_by_a_later_spawn() {
        use crate::entity_config::EntityConfig;
        crate::config_cache::insert_native_config(
            "fixture/reuse_1033.toml".to_string(),
            EntityConfig::from_toml("").unwrap(),
        );

        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = '''
on_destroyed("cue", "recycle");
fn recycle(ctx) {
    ctx.effects.destroy_entity("band");
    ctx.effects.spawn_entity(#{ template_path: "fixture/reuse_1033.toml", name: "band", position: [10, 0, 0] });
}
'''
"#,
        );

        let mut app = ai_trigger_test_app();
        app.world_mut()
            .insert_resource(crate::world_id::WorldIdMint::default());
        let old_uuid = "band-old-uuid-1033";
        let old = register_named_entity(&mut app, "band", old_uuid);
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("cue".to_string(), "cue-uuid-1033d".to_string());
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);

        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: "cue-uuid-1033d".into(),
            });
        app.update();

        assert!(
            app.world().get_entity(old).is_err(),
            "the old entity is gone"
        );
        let now = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("band")
            .cloned()
            .expect("the name is still registered — to the NEW entity");
        assert_ne!(
            now, old_uuid,
            "reusing a destroyed entity's name must resolve to the freshly minted \
             uuid, not the stale one"
        );
    }

    /// Issue #984 (Rhai M6 phase 2a), Startup-wiring proof: `compile_world_scripts`
    /// inserts `WorldScriptRuntime`, and `init_world_runtime` merges the scripted
    /// trigger into `trigger_states` and builds `handlers` parallel to it — the
    /// same `Startup` chain production runs (minus the spawn), driven here without
    /// a full headless app.
    ///
    /// The fixture carried a `[[trigger]]` block as well, to pin that the scripted
    /// state was APPENDED after the declarative ones. Issue #985 deleted that
    /// parser (a world authoring one now fails to parse at all), so scripts are the
    /// only source and the parallel `handlers` table has no `None` entries left to
    /// check.
    #[test]
    fn compile_and_init_wire_a_scripted_trigger_into_the_runtime() {
        let world_toml = r#"
[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.effects.complete_objective("obj"); }'
"#;
        let world_config = crate::world::config::parse_world(world_toml).expect("world parses");
        let raw_value: toml::Value = toml::from_str(world_toml).expect("valid toml");

        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>()
            .init_resource::<WorldResource>()
            .insert_resource(world_config)
            .insert_resource(RawWorldSource {
                path: "fixture/scripted.toml".to_string(),
                toml: raw_value,
            })
            .add_systems(Startup, (compile_world_scripts, init_world_runtime).chain());
        app.update();

        // compile_world_scripts inserted the runtime; init_world_runtime's
        // one-time merge then drained `triggers` (finding 5) — it is not retained
        // for the world's lifetime, the compiled trigger now lives in
        // `trigger_states` and the parallel `handlers` table.
        let sr = app.world().resource::<WorldScriptRuntime>();
        assert!(
            sr.triggers.is_empty(),
            "merge_script_triggers drains the retained triggers"
        );
        // handlers stay parallel to trigger_states — one entry, carrying the
        // compiled handler fn.
        assert_eq!(sr.handlers.len(), 1, "one scripted trigger");
        assert_eq!(
            sr.handlers[0].as_ref().map(|h| h.fn_name.as_str()),
            Some("k"),
            "the scripted index carries its handler fn"
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.trigger_states.len(), 1);
        assert_eq!(
            runtime.trigger_states[0].trigger.condition,
            TriggerCondition::OnDestroyed {
                entity_name: "raider".to_string()
            }
        );
    }

    /// Whether the objective manager reports `id` as `Completed`.
    fn objective_is_completed(app: &App, id: &str) -> bool {
        app.world()
            .resource::<ObjectiveManagerRes>()
            .0
            .sorted_snapshots()
            .into_iter()
            .any(|o| o.id == id && o.status == ObjectiveStatus::Completed)
    }

    /// Issue #984 (Rhai M6 phase 2b), scheduled-callback drain proof: a scripted
    /// `on_destroyed` handler calls `ctx.schedule.after(2, |ctx| …)`, and the
    /// callback runs through the LIVE pipeline only once its fire tick arrives —
    /// NOT on the tick it was scheduled. At `sim_tick_hz = 1` the 2-second delay
    /// is 2 ticks, and `SimTick` is driven by hand so the drain boundary is exact.
    #[test]
    fn scripted_callback_fires_after_the_delay_not_immediately() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("obj"); }); }'
"#,
        );

        let mut app = ai_trigger_test_app();
        // Drains the callbacks the handler schedules, after the pipeline that
        // schedules them — the same order production wires.
        app.add_systems(Update, tick_script_callbacks.after(tick_trigger_pipeline));
        // sim_tick_hz = 1 → `after(2 s)` == fire_tick + 2 ticks; SimTick is driven
        // by hand below so the drain lands on an exact, readable tick.
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.sim_tick_hz = 1.0;
        app.world_mut().insert_resource(cfg);
        app.world_mut().insert_resource(crate::sim_tick::SimTick(0));

        let raider_uuid = "raider-uuid-984-cb1";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "obj",
            "hold the line",
            true,
            vec![],
        );

        // Tick 0: the raider dies → handler `k` schedules the callback (fire_tick
        // 0 + 2 = 2). The callback must NOT run on the tick it was scheduled.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();
        assert!(
            !objective_is_completed(&app, "obj"),
            "the callback must not fire on the tick it was scheduled"
        );
        assert_eq!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_callbacks
                .len(),
            1,
            "the scheduled callback must be queued, not dropped (2a dropped it)"
        );

        // Tick 1: still before the fire tick — nothing drains.
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
        app.update();
        assert!(
            !objective_is_completed(&app, "obj"),
            "the callback must not fire before its delay elapses"
        );

        // Tick 2: now due — drained and run through the live apply path.
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
        app.update();
        assert!(
            objective_is_completed(&app, "obj"),
            "tick_script_callbacks must run the due callback through the live pipeline"
        );
        assert!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_callbacks
                .is_empty(),
            "the fired callback must be drained out of the queue"
        );
    }

    /// Issue #984 (Rhai M6 phase 2b), re-queue proof: a callback that itself calls
    /// `ctx.schedule.after(..)` re-queues the new callback onto `pending_callbacks`
    /// for a FUTURE tick. The first callback (2 ticks out) completes "first" and
    /// schedules a second (2 ticks further); the second completes "second".
    #[test]
    fn scripted_callback_can_reschedule_another_callback() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("first"); ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("second"); }); }); }'
"#,
        );

        let mut app = ai_trigger_test_app();
        app.add_systems(Update, tick_script_callbacks.after(tick_trigger_pipeline));
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.sim_tick_hz = 1.0;
        app.world_mut().insert_resource(cfg);
        app.world_mut().insert_resource(crate::sim_tick::SimTick(0));

        let raider_uuid = "raider-uuid-984-cb2";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);
        {
            let mut objectives = app.world_mut().resource_mut::<ObjectiveManagerRes>();
            objectives.0.add("first", "first beat", true, vec![]);
            objectives.0.add("second", "second beat", true, vec![]);
        }

        // Tick 0: schedule the first callback (fire_tick 2).
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        // Tick 2: the first callback fires, completing "first" and scheduling the
        // second (fire_tick 2 + 2 = 4). "second" is queued, not yet run.
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
        app.update();
        assert!(
            objective_is_completed(&app, "first"),
            "the first callback must complete its objective on its fire tick"
        );
        assert!(
            !objective_is_completed(&app, "second"),
            "the re-queued callback must not fire on the tick that scheduled it"
        );
        assert_eq!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_callbacks
                .len(),
            1,
            "the callback scheduled from inside a callback must be re-queued"
        );

        // Tick 4: the re-queued callback is now due and completes "second".
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 4;
        app.update();
        assert!(
            objective_is_completed(&app, "second"),
            "a callback re-queued by a callback must fire on its own later tick"
        );
        assert!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_callbacks
                .is_empty(),
            "both callbacks must be drained once fired"
        );
    }

    /// Issue #984 (Rhai M6 phase 2b), delay-0 re-queue proof (the busy-loop edge):
    /// a callback that reschedules another at `after(0)` gives the new callback
    /// `fire_tick == the current tick`. The `mem::take` snapshot `drain_due` takes
    /// BEFORE iterating means that delay-0 callback lands on `pending_callbacks`
    /// AFTER the snapshot, so it does NOT fire re-entrantly in the same drain — it
    /// fires on the NEXT tick. This is what makes an `after(0)` self-reschedule
    /// impossible to spin within a single tick.
    #[test]
    fn scripted_callback_rescheduled_at_delay_zero_defers_to_the_next_tick() {
        let mut sr = compile_fixture_scripts(
            r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("first"); ctx.schedule.after(0, |ctx| { ctx.effects.complete_objective("second"); }); }); }'
"#,
        );

        let mut app = ai_trigger_test_app();
        app.add_systems(Update, tick_script_callbacks.after(tick_trigger_pipeline));
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.global.sim_tick_hz = 1.0;
        app.world_mut().insert_resource(cfg);
        app.world_mut().insert_resource(crate::sim_tick::SimTick(0));

        let raider_uuid = "raider-uuid-984-cb0";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".to_string(), raider_uuid.to_string());
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);
        {
            let mut objectives = app.world_mut().resource_mut::<ObjectiveManagerRes>();
            objectives.0.add("first", "first beat", true, vec![]);
            objectives.0.add("second", "second beat", true, vec![]);
        }

        // Tick 0: schedule the first callback (fire_tick 2).
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: raider_uuid.into(),
            });
        app.update();

        // Tick 2: the first callback fires and reschedules the second at `after(0)`,
        // so its fire_tick == 2 == the current tick. The snapshot boundary must keep
        // it OUT of this same drain — "second" stays queued, unrun, this tick.
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
        app.update();
        assert!(
            objective_is_completed(&app, "first"),
            "the first callback must fire on its own tick"
        );
        assert!(
            !objective_is_completed(&app, "second"),
            "a delay-0 callback scheduled during the drain must NOT fire re-entrantly this tick"
        );
        assert_eq!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_callbacks
                .len(),
            1,
            "the delay-0 re-queue must be parked for the next tick, not run now"
        );

        // Tick 3: the very next tick drains the delay-0 callback (fire_tick 2 <= 3).
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 3;
        app.update();
        assert!(
            objective_is_completed(&app, "second"),
            "the deferred delay-0 callback must fire on the next tick"
        );
        assert!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_callbacks
                .is_empty(),
            "both callbacks must be drained once fired"
        );
    }

    /// Whether the trigger at `index` has latched `fired`.
    ///
    /// The trigger latch is what a condition test reads since issue #985 deleted
    /// the `[[trigger]]` action array: a fire used to be observed through an
    /// action's side effect, and a trigger that fires now records nothing else.
    fn trigger_fired(app: &App, index: usize) -> bool {
        app.world().resource::<WorldContentRuntime>().trigger_states[index].fired
    }

    /// A scenario trigger fires when the named ship reaches the named
    /// waypoint — the `AiWaypointReached` message the cursor evaluator emits
    /// is bridged into a `WorldEvent::WaypointReached` and matched here.
    #[test]
    fn on_waypoint_reached_trigger_fires() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "patrol-npc-uuid-001";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("harrow_patrol".to_string(), npc_uuid.to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnWaypointReached {
                    entity_name: "harrow_patrol".to_string(),
                    waypoint: Some("wp_border".to_string()),
                },
                when: None,
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        }];

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiWaypointReached>>()
            .write(crate::ai_plugin::AiWaypointReached {
                entity_uuid: npc_uuid.to_string(),
                objective_id: "patrol".to_string(),
                waypoint: "wp_border".to_string(),
            });

        app.update();

        assert!(
            trigger_fired(&app, 0),
            "reaching wp_border must fire the trigger"
        );
    }

    /// A trigger naming a specific waypoint must ignore arrivals at the
    /// ship's other waypoints.
    #[test]
    fn on_waypoint_reached_trigger_ignores_a_different_waypoint() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "patrol-npc-uuid-002";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("harrow_patrol".to_string(), npc_uuid.to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnWaypointReached {
                    entity_name: "harrow_patrol".to_string(),
                    waypoint: Some("wp_border".to_string()),
                },
                when: None,
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        }];

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiWaypointReached>>()
            .write(crate::ai_plugin::AiWaypointReached {
                entity_uuid: npc_uuid.to_string(),
                objective_id: "patrol".to_string(),
                waypoint: "wp_home".to_string(),
            });

        app.update();

        assert!(
            !trigger_fired(&app, 0),
            "arriving at wp_home must not fire a trigger scoped to wp_border"
        );
    }

    /// Omitting `waypoint` scopes the trigger to the ship rather than to one
    /// stop on its route: any arrival fires it.
    #[test]
    fn on_waypoint_reached_without_waypoint_fires_on_any_waypoint() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "patrol-npc-uuid-003";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("harrow_patrol".to_string(), npc_uuid.to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnWaypointReached {
                    entity_name: "harrow_patrol".to_string(),
                    waypoint: None,
                },
                when: None,
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        }];

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiWaypointReached>>()
            .write(crate::ai_plugin::AiWaypointReached {
                entity_uuid: npc_uuid.to_string(),
                objective_id: "patrol".to_string(),
                waypoint: "wp_anywhere".to_string(),
            });

        app.update();

        assert!(
            trigger_fired(&app, 0),
            "an unscoped on_waypoint_reached trigger must fire on any arrival"
        );
    }

    /// A trigger naming a different ship must not fire for this ship's arrival.
    #[test]
    fn on_waypoint_reached_trigger_ignores_a_different_ship() {
        let mut app = ai_trigger_test_app();

        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("harrow_patrol".to_string(), "uuid-harrow".to_string());
        runtime
            .name_to_uuid
            .insert("other_ship".to_string(), "uuid-other".to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnWaypointReached {
                    entity_name: "harrow_patrol".to_string(),
                    waypoint: None,
                },
                when: None,
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        }];

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiWaypointReached>>()
            .write(crate::ai_plugin::AiWaypointReached {
                entity_uuid: "uuid-other".to_string(),
                objective_id: "patrol".to_string(),
                waypoint: "wp_border".to_string(),
            });

        app.update();

        assert!(
            !trigger_fired(&app, 0),
            "another ship's arrival must not fire a trigger scoped to harrow_patrol"
        );
    }

    /// The `AiEntityDestroyed` message is bridged into a `WorldEvent::Destroyed`
    /// and matched against the trigger's named entity.
    #[test]
    fn on_entity_destroyed_trigger_fires() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "dead-npc-uuid-001";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("station_alpha".to_string(), npc_uuid.to_string());
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnDestroyed {
                    entity_name: "station_alpha".to_string(),
                },
                when: None,
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        }];

        // Emit the AiEntityDestroyed message.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.to_string(),
            });

        app.update();

        assert!(
            trigger_fired(&app, 0),
            "the named entity's destruction must fire the trigger"
        );
    }

    // -- ApplyModifier / per-entity target regression tests -----------------
    //
    // The following six tests exercise dispatch of the per-entity trigger
    // actions (`ApplyModifier`, `RemoveModifier`, `ApplyFlag`, `RemoveFlag`,
    // `ApplyIntModifier`, `RemoveIntModifier`) and prove that the action lands
    // on the target entity's per-entity `ShipModifiers` Component â€” not the
    // legacy global Resource â€” and that non-target entities (e.g. the player
    // ship) remain unaffected. These are the regression tests for the
    // audit-report bug where world triggers silently misrouted every
    // named-entity write to whichever ship happened to own the global Resource.
    //
    // They fired the actions off an `on_destroyed` trigger until issue #985
    // deleted the `[[trigger]]` action array; the actions are dispatched through
    // the delayed-action queue now, which reaches the SAME `world::dispatch`
    // table and the same `apply_dispatch_result` applier.

    /// Spawns two entities (an NPC and a "player" ship) with distinct
    /// UUIDs and per-entity `ShipModifiers` components. Registers name
    /// mappings for both. Returns `(npc_entity, player_entity)`.
    fn spawn_two_modifier_targets(app: &mut App) -> (Entity, Entity) {
        let npc = app
            .world_mut()
            .spawn((
                EntityUuid("npc-target-uuid".to_string()),
                crate::modifiers::ShipModifiers::new(),
            ))
            .id();
        let player = app
            .world_mut()
            .spawn((
                EntityUuid("player-target-uuid".to_string()),
                crate::modifiers::ShipModifiers::new(),
            ))
            .id();
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("raider_alpha".into(), "npc-target-uuid".into());
        runtime
            .name_to_uuid
            .insert("player_ship".into(), "player-target-uuid".into());
        (npc, player)
    }

    /// Dispatches `actions`, in list order, naming `raider_alpha` as the
    /// resolving entity — the name a fired `OnDestroyed { raider_alpha }`
    /// trigger used to stamp onto the dispatch context before issue #985.
    fn dispatch_actions_for_npc_target(app: &mut App, actions: Vec<TriggerAction>) {
        dispatch_delayed_actions(app, None, Some("raider_alpha"), actions);
    }

    #[test]
    fn ai_events_apply_modifier_lands_on_target_entity_not_player() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        dispatch_actions_for_npc_target(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "raider_alpha".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 1.5,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .expect("NPC entity must carry ShipModifiers");
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .expect("player entity must carry ShipModifiers");
        assert!(
            npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) > 1.0,
            "ApplyModifier must land on the target NPC's per-entity component; got {}",
            npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed)
        );
        assert!(
            (player_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "player entity must be unaffected by an NPC-targeted ApplyModifier; got {}",
            player_mods.get(&crate::messages::ModifierSlot::MaxSpeed)
        );
    }

    #[test]
    fn ai_events_remove_modifier_undoes_only_the_target_entity() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        dispatch_actions_for_npc_target(
            &mut app,
            vec![
                TriggerAction::ApplyModifier {
                    entity: "raider_alpha".into(),
                    tag: "boost".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                    bonus: 2.0,
                },
                TriggerAction::RemoveModifier {
                    entity: "raider_alpha".into(),
                    tag: "boost".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                },
            ],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .expect("NPC entity must carry ShipModifiers");
        let value = npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed);
        assert!(
            (value - 1.0).abs() < 1e-3,
            "RemoveModifier must reverse the previously-applied bonus on the NPC's component; expected 1.0, got {value}"
        );
    }

    #[test]
    fn ai_events_apply_flag_lands_on_target_entity_not_player() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        dispatch_actions_for_npc_target(
            &mut app,
            vec![TriggerAction::ApplyFlag {
                entity: "raider_alpha".into(),
                tag: "jammer".into(),
                kind: crate::messages::FlagKind::CommsJammed,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            npc_mods.has_flag(&crate::messages::FlagKind::CommsJammed),
            "ApplyFlag must register on the target NPC's per-entity component"
        );
        assert!(
            !player_mods.has_flag(&crate::messages::FlagKind::CommsJammed),
            "player entity must be unaffected by an NPC-targeted ApplyFlag"
        );
    }

    #[test]
    fn ai_events_remove_flag_undoes_only_the_target_entity() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        dispatch_actions_for_npc_target(
            &mut app,
            vec![
                TriggerAction::ApplyFlag {
                    entity: "raider_alpha".into(),
                    tag: "jammer".into(),
                    kind: crate::messages::FlagKind::CommsJammed,
                },
                TriggerAction::RemoveFlag {
                    entity: "raider_alpha".into(),
                    tag: "jammer".into(),
                    kind: crate::messages::FlagKind::CommsJammed,
                },
            ],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            !npc_mods.has_flag(&crate::messages::FlagKind::CommsJammed),
            "RemoveFlag must un-register the flag on the NPC's per-entity component"
        );
    }

    #[test]
    fn ai_events_apply_int_modifier_lands_on_target_entity_not_player() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        dispatch_actions_for_npc_target(
            &mut app,
            vec![TriggerAction::ApplyIntModifier {
                entity: "raider_alpha".into(),
                tag: "extra_team".into(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
                bonus: 2,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert_eq!(
            npc_mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            2,
            "ApplyIntModifier must land on the target NPC's per-entity component"
        );
        assert_eq!(
            player_mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            0,
            "player entity must be unaffected by an NPC-targeted ApplyIntModifier"
        );
    }

    #[test]
    fn ai_events_remove_int_modifier_undoes_only_the_target_entity() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        dispatch_actions_for_npc_target(
            &mut app,
            vec![
                TriggerAction::ApplyIntModifier {
                    entity: "raider_alpha".into(),
                    tag: "extra_team".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                    bonus: 3,
                },
                TriggerAction::RemoveIntModifier {
                    entity: "raider_alpha".into(),
                    tag: "extra_team".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                },
            ],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert_eq!(
            npc_mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            0,
            "RemoveIntModifier must reverse the int modifier on the NPC's per-entity component"
        );
    }

    #[test]
    fn ai_events_apply_modifier_unknown_entity_name_is_ignored() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        dispatch_actions_for_npc_target(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "does_not_exist".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 5.0,
            }],
        );
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        let player_mods = app
            .world()
            .entity(player)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            (npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "unknown entity name must not touch any entity's per-entity component (NPC)"
        );
        assert!(
            (player_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "unknown entity name must not touch any entity's per-entity component (player)"
        );
    }

    #[test]
    fn ai_events_apply_modifier_registered_name_without_ecs_entity_is_ignored() {
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        // Register a phantom name â†’ UUID mapping with no matching ECS entity.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("phantom".into(), "phantom-uuid".into());
        }
        dispatch_actions_for_npc_target(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "phantom".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 5.0,
            }],
        );
        // No entity should have received the modifier.
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            (npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "registered name with no ECS entity must not silently misroute the modifier"
        );
    }

    #[test]
    fn ai_events_apply_modifier_target_entity_without_component_is_ignored() {
        // Guards the third defensive branch: name resolves + UUID resolves +
        // Entity exists but has no `ShipModifiers` Component â†’ warn+continue.
        let mut app = ai_trigger_test_app();
        let (npc, _player) = spawn_two_modifier_targets(&mut app);
        // Spawn an entity with a UUID but WITHOUT a ShipModifiers component.
        app.world_mut()
            .spawn(EntityUuid("componentless-uuid".to_string()));
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("componentless".into(), "componentless-uuid".into());
        }
        dispatch_actions_for_npc_target(
            &mut app,
            vec![TriggerAction::ApplyModifier {
                entity: "componentless".into(),
                tag: "boost".into(),
                slot: crate::messages::ModifierSlot::MaxSpeed,
                bonus: 5.0,
            }],
        );
        // The NPC should NOT have been affected â€” no silent misroute.
        let npc_mods = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap();
        assert!(
            (npc_mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "entity without ShipModifiers component must not misroute to other ships"
        );
    }

    #[test]
    fn on_all_destroyed_trigger_fires_after_all_named_entities_die_across_ticks() {
        // End-to-end Bevy runtime check: an `OnAllDestroyed` trigger over two
        // named entities must accumulate `seen_destroyed` across separate
        // `app.update()` calls and fire only when the last entity dies. Mirrors
        // `on_entity_destroyed_trigger_fires` but uses two separate destruction
        // ticks. (#470)
        let mut app = ai_trigger_test_app();

        let uuid_a = "wave-a-uuid";
        let uuid_b = "wave-b-uuid";
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("wave_a".to_string(), uuid_a.to_string());
        runtime
            .name_to_uuid
            .insert("wave_b".to_string(), uuid_b.to_string());
        runtime.entity_groups.insert(
            "waves".to_string(),
            ["wave_a".to_string(), "wave_b".to_string()]
                .into_iter()
                .collect(),
        );
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnAllDestroyed {
                    group: "waves".into(),
                    after_secs: 0.0,
                },
                when: None,
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        }];

        // Tick 1: only wave_a dies. Trigger must NOT fire yet.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: uuid_a.to_string(),
            });
        app.update();

        assert!(
            !trigger_fired(&app, 0),
            "the trigger must not fire after only the first wave dies"
        );

        // Tick 2: wave_b dies. Trigger must NOW fire.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: uuid_b.to_string(),
            });
        app.update();

        assert!(
            trigger_fired(&app, 0),
            "the trigger must fire after the last named entity dies"
        );
    }

    // -- Flag-system integration tests (issue #412) ---------------------------

    /// A `when` that reads false suppresses the firing WITHOUT consuming the
    /// trigger, so the same trigger still fires on a later matching event once
    /// the flag is set. That distinction is the whole point of a trigger-level
    /// gate — an in-handler `if` cannot express it, because by then the
    /// condition has fired and the trigger is spent.
    ///
    /// The suppression used to be read off a gated `add_objective`; issue #985
    /// deleted the `[[trigger]]` action array, so the observable is the
    /// trigger's own `fired` latch.
    #[test]
    fn when_predicate_suppresses_the_fire_but_keeps_the_trigger_live() {
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-destroyed-target";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("target".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "target".into(),
                    },
                    when: Some(crate::world::flags::parse_predicate("flag(green_light)").unwrap()),
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }
        // First firing: flag unset ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ no objective.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();
        assert!(
            !trigger_fired(&app, 0),
            "a false `when` must suppress the fire and leave the trigger live"
        );
        // Set the flag and re-fire.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("green_light");
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();
        assert!(
            trigger_fired(&app, 0),
            "the still-live trigger must fire once the flag is set"
        );
    }

    /// A `set_flag` action's `FlagSet` transition event chains a downstream
    /// `on_flag_set` trigger.
    ///
    /// It used to chain within a single tick, because the setter was an action on
    /// a trigger and the pipeline loops its chaining passes inside one tick. Issue
    /// #985 deleted that action array, so the setter is dispatched from the
    /// delayed-action queue instead — and `tick_delayed_actions` runs AFTER the
    /// pipeline has drained this tick's events, so its `FlagSet` queues onto
    /// `pending_world_events` and the watcher fires on the NEXT tick.
    #[test]
    fn delayed_set_flag_action_fires_on_flag_set_trigger_on_the_next_tick() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagSet { name: "a".into() },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }
        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::SetWorldFlag { name: "a".into() }],
        );

        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(
                runtime.flags.flag("a"),
                "set_flag action must have mutated the store"
            );
            assert!(
                !trigger_fired(&app, 0),
                "the transition is queued for the next tick, so nothing has \
                 chained yet"
            );
        }

        // Next tick: `collect_world_events` drains the queued `FlagSet` into the
        // buffer and the watcher fires.
        app.update();
        assert!(
            trigger_fired(&app, 0),
            "on_flag_set trigger must fire on the queued FlagSet transition"
        );
    }

    /// Flag "a" starts set, so a `set_flag a` is a no-op (the counter stays 1)
    /// and emits no transition at all — an `on_flag_set` watcher must stay
    /// unfired, because the condition matches transitions, not values.
    ///
    /// The setter is dispatched from the delayed-action queue: issue #985 deleted
    /// the `[[trigger]]` action array it used to ride on.
    #[test]
    fn no_op_reset_of_already_set_flag_does_not_emit_transition() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("a"); // pre-set
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagSet { name: "a".into() },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }
        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::SetWorldFlag { name: "a".into() }],
        );

        assert!(
            app.world()
                .resource::<WorldContentRuntime>()
                .pending_world_events
                .is_empty(),
            "a no-op re-set must emit no transition event at all"
        );

        // Step the tick that would have drained a transition, had one been
        // emitted.
        app.update();
        assert!(
            !trigger_fired(&app, 0),
            "on_flag_set must not fire when the flag was already set (no transition)"
        );
    }

    /// The mirror of the `set_flag` chain: a `clear_flag` over a set flag is a
    /// true → false transition, so it emits `FlagCleared` and an
    /// `on_flag_cleared` watcher fires on the next tick (the delayed-action
    /// queue's event lag — see the `set_flag` twin above).
    #[test]
    fn delayed_clear_flag_action_fires_on_flag_cleared_trigger_on_the_next_tick() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // Pre-set so the clear is a real true → false transition.
            runtime.flags.set_flag("shields_up");
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagCleared {
                        name: "shields_up".into(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }
        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::ClearWorldFlag {
                name: "shields_up".into(),
            }],
        );
        app.update();

        assert!(!app
            .world()
            .resource::<WorldContentRuntime>()
            .flags
            .flag("shields_up"));
        assert!(
            trigger_fired(&app, 0),
            "on_flag_cleared trigger must fire on the true → false transition"
        );
    }

    // -- PRD #397 fix 1: parent: walker in Bevy dispatch ----------------------

    /// A trigger in a sub-world layer can gate its `when` predicate on a
    /// flag in the loader (base) world via the `parent:` prefix.
    #[test]
    fn parent_prefix_in_when_predicate_reads_loader_layer_flag() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();

        let layer_path = "child.toml".to_string();
        // Pre-register a sub-world layer whose loader is the base world.
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                layer_path.clone(),
                WorldRuntime {
                    loader_path: None,
                    ..Default::default()
                },
            );
        }
        let npc_uuid = "uuid-parent-when-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // Base layer: flag `armed` is set.
            runtime.flags.set_flag("armed");
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            // Sub-world trigger: on_destroyed with when=flag(parent:armed).
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "source".into(),
                    },
                    when: Some(crate::world::flags::parse_predicate("flag(parent:armed)").unwrap()),
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: Some(layer_path.clone()),
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        assert!(
            trigger_fired(&app, 0),
            "sub-world trigger gated on parent:armed must fire when base flag is set"
        );
    }

    /// Per-layer flag scoping: a flag set inside a sub-world must NOT fire a
    /// base-world trigger that watches the same name. The mutation lands in the
    /// layer's own store and its `FlagSet` carries that layer path, which the
    /// base-world watcher's layer chain does not match.
    ///
    /// The sub-world `set_flag` came off a layer-origin trigger's action array
    /// until issue #985 deleted it; the same mutation is queued as a delayed
    /// action stamped with that layer as its `origin_layer`, which is the field
    /// `dispatch_action` reads to pick the target store either way.
    #[test]
    fn same_named_flag_in_sub_world_does_not_fire_base_world_on_flag_set() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();

        let layer_path = "child.toml".to_string();
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                layer_path.clone(),
                WorldRuntime {
                    loader_path: None,
                    ..Default::default()
                },
            );
        }
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // Base-world watcher: on_flag_set armed.
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagSet {
                        name: "armed".into(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }
        dispatch_delayed_actions(
            &mut app,
            Some(&layer_path),
            None,
            vec![TriggerAction::SetWorldFlag {
                name: "armed".into(),
            }],
        );
        // Second tick: the queued FlagSet reaches the pipeline, so the base
        // watcher gets its chance to (wrongly) match.
        app.update();

        // Sub-world layer's flag store got the mutation; base store did not.
        let lm = app.world().resource::<WorldLayerMap>();
        let layer_flags = &lm.0.get(&layer_path).expect("layer present").flags;
        assert!(
            layer_flags.flag("armed"),
            "mutation lands in sub-world layer"
        );
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(!runtime.flags.flag("armed"), "base store must remain empty");
        assert!(
            !runtime.trigger_states[0].fired,
            "base trigger must not cross-fire on sub-world flag"
        );
    }

    /// A `parent:flag` mutation from the BASE world walks past the root of the
    /// layer chain: no store to write, so it is a warned no-op — and in
    /// particular it must not fall back to writing the base store, nor write the
    /// literal prefixed name as if it were an ordinary flag.
    ///
    /// Driven from the delayed-action queue since issue #985 deleted the
    /// `[[trigger]]` action array the mutation used to be authored on.
    #[test]
    fn parent_walk_past_root_from_base_is_noop_for_mutation_and_reads_unset() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();
        dispatch_delayed_actions(
            &mut app,
            // Base world: the chain is one entry long, so one `parent:` step
            // already walks off the end.
            None,
            None,
            vec![TriggerAction::SetWorldFlag {
                name: "parent:armed".into(),
            }],
        );

        // Neither the base `armed` nor `parent:armed` should be set.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.flags.flag("armed"),
            "past-root mutation must not write to base"
        );
        assert!(
            !runtime.flags.flag("parent:armed"),
            "past-root mutation must not write the literal prefixed name"
        );
    }

    /// The `AiEntityAttacked` message is bridged into a `WorldEvent::Attacked`
    /// and matched against the trigger's named entity.
    #[test]
    fn on_entity_attacked_trigger_fires() {
        let mut app = ai_trigger_test_app();

        let npc_uuid = "attacked-npc-uuid-002";
        let attacker_uuid = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("enemy_ship".to_string(), npc_uuid.to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "enemy_ship".to_string(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: npc_uuid.to_string(),
                attacker_uuid,
            });

        app.update();

        assert!(
            trigger_fired(&app, 0),
            "an attack on the named entity must fire the trigger"
        );
    }

    /// Issue #572: `SetAiState` survives as a `TriggerAction` variant but is a
    /// no-op — doctrine-based AI has no FSM state slot to write. Dispatching one
    /// must neither panic nor disturb the target entity.
    ///
    /// Dispatched through the delayed-action queue: issue #985 deleted the
    /// `[[trigger]]` action array it used to be authored on.
    #[test]
    fn set_ai_state_action_is_noop_in_doctrine_based_ai() {
        use crate::entity_config::BehaviourConfig;

        let mut app = ai_trigger_test_app();

        let npc_uuid = "npc-state-change-uuid-003";
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid(npc_uuid.to_string()),
                BehaviourSection(BehaviourConfig::default()),
            ))
            .id();
        app.update(); // register AI tokens

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("npc_alpha".to_string(), npc_uuid.to_string());
        }

        // Must not panic — SetAiState is silently ignored.
        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::SetAiState {
                entity: "npc_alpha".to_string(),
                state: "chase".to_string(),
                target: None,
            }],
        );

        // The entity must still be AI-controlled (no FSM state to mutate).
        assert!(
            app.world().get::<BehaviourSection>(entity).is_some(),
            "BehaviourSection must survive a SetAiState no-op"
        );
    }

    // -- add_faction_enemy / remove_faction_enemy dispatch tests --------------

    // These fired their actions off an `on_world_loaded` trigger until issue
    // #985 deleted the `[[trigger]]` action array; they go through
    // `dispatch_delayed_actions_in_new_app`, which reaches the same
    // `world::dispatch` table with the same `FactionRegistry` in the context.

    /// Convenience: faction UUIDs from the bundled TOML asset files
    /// (loaded by `get_faction_registry`). Centralises the literal UUIDs
    /// so the tests can refer to them by symbolic name.
    fn fed_faction_uuid() -> uuid::Uuid {
        uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap()
    }
    fn harrow_faction_uuid() -> uuid::Uuid {
        uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap()
    }

    #[test]
    fn add_faction_enemy_action_makes_factions_mutually_hostile() {
        // Pre-condition: Harrow and Federation default to neutral.
        let app = ai_trigger_test_app();
        {
            let reg = &app
                .world()
                .resource::<crate::config_cache::FactionRegistryResource>()
                .0;
            assert!(
                !crate::faction::is_enemy(
                    Some(fed_faction_uuid()),
                    Some(harrow_faction_uuid()),
                    reg
                ),
                "precondition: Federation must not consider Harrow hostile by default"
            );
            assert!(
                !crate::faction::is_enemy(
                    Some(harrow_faction_uuid()),
                    Some(fed_faction_uuid()),
                    reg
                ),
                "precondition: Harrow must not consider Federation hostile by default"
            );
        }

        // Dispatch the pair that flips both directions hostile.
        let app = dispatch_delayed_actions_in_new_app(vec![
            TriggerAction::AddFactionEnemy {
                faction: "Harrow".into(),
                enemy: "Federation".into(),
            },
            TriggerAction::AddFactionEnemy {
                faction: "Federation".into(),
                enemy: "Harrow".into(),
            },
        ]);

        let reg = &app
            .world()
            .resource::<crate::config_cache::FactionRegistryResource>()
            .0;
        assert!(
            crate::faction::is_enemy(Some(fed_faction_uuid()), Some(harrow_faction_uuid()), reg),
            "Federation must consider Harrow hostile after add_faction_enemy"
        );
        assert!(
            crate::faction::is_enemy(Some(harrow_faction_uuid()), Some(fed_faction_uuid()), reg),
            "Harrow must consider Federation hostile after add_faction_enemy"
        );
    }

    #[test]
    fn add_faction_enemy_action_with_unknown_faction_name_is_noop() {
        // The Federation registry stays unchanged when the named faction
        // is missing. Verifies the warn-and-skip dispatch path.
        let app = dispatch_delayed_actions_in_new_app(vec![TriggerAction::AddFactionEnemy {
            faction: "Klingon".into(), // not a registered faction
            enemy: "Federation".into(),
        }]);

        let reg = &app
            .world()
            .resource::<crate::config_cache::FactionRegistryResource>()
            .0;
        // Federation's enemies list must still contain Pirate (its default)
        // and nothing else from the AddFactionEnemy dispatch.
        let fed = reg.get(&fed_faction_uuid()).expect("federation present");
        let pirate_uuid = uuid::Uuid::parse_str("bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb").unwrap();
        assert_eq!(
            fed.enemies,
            vec![pirate_uuid],
            "unknown faction name must not mutate any other faction's enemy list"
        );
    }

    #[test]
    fn remove_faction_enemy_action_removes_relationship() {
        // First add the relationship, then verify remove undoes it.
        let app = dispatch_delayed_actions_in_new_app(vec![
            TriggerAction::AddFactionEnemy {
                faction: "Harrow".into(),
                enemy: "Federation".into(),
            },
            TriggerAction::RemoveFactionEnemy {
                faction: "Harrow".into(),
                enemy: "Federation".into(),
            },
        ]);

        let reg = &app
            .world()
            .resource::<crate::config_cache::FactionRegistryResource>()
            .0;
        assert!(
            !crate::faction::is_enemy(Some(harrow_faction_uuid()), Some(fed_faction_uuid()), reg),
            "remove_faction_enemy must undo the prior add_faction_enemy"
        );
    }

    /// Scenario:
    ///   1. Spawn a Harrow-factioned NPC that has locked a Federation player
    ///      ship (the lock is seeded by hand to stand in for a prior
    ///      `enemy_in_range` engagement).
    ///   2. Make the two sides mutually hostile via `add_faction_enemy`, so the
    ///      precondition holds.
    ///   3. Dispatch `remove_faction_enemy` for Harrow → Federation.
    ///   4. Assert the NPC's lock is cleared — the revalidation kicked in
    ///      because the target's faction is no longer hostile to its own.
    ///
    /// The three actions rode on two `on_world_loaded` / `on_flag_set` triggers
    /// until issue #985 deleted the `[[trigger]]` action array; they are queued
    /// on the delayed-action queue in the same order now, and
    /// `tick_delayed_actions` carries the `ai_query` the revalidation needs.
    #[test]
    fn remove_faction_enemy_action_clears_blackboard_target_when_target_becomes_friendly() {
        use crate::entity_config::BehaviourConfig;

        let mut app = ai_trigger_test_app();

        // Prepare a Federation-factioned "player ship" entity.
        let player_uuid_str = "11111111-1111-1111-1111-111111111111";
        let player_uuid = uuid::Uuid::parse_str(player_uuid_str).unwrap();
        app.world_mut().spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid(player_uuid_str.to_string()),
            crate::entities::spawner::FactionComponent(fed_faction_uuid()),
        ));

        // Prepare a Harrow-factioned NPC entity with an AI behaviour.
        let npc_uuid_str = "22222222-2222-2222-2222-222222222222";
        let npc_entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(10.0, 0.0, 0.0),
                EntityUuid(npc_uuid_str.to_string()),
                BehaviourSection(BehaviourConfig::default()),
                crate::entities::spawner::FactionComponent(harrow_faction_uuid()),
                // The ship's authoritative Tactical lock. Post-#702 this is the
                // surface `revalidate_ai_targets_after_faction_change` clears;
                // it used to clear the private `ShipAiMemory.target` mirror,
                // which by then was no longer what the firing path read.
                crate::weapons_plugin::TacticalRadarSelection::default(),
            ))
            .id();

        // First update: register the AI token for the spawned NPC.
        app.update();

        // Manually seed the engagement: the NPC has locked the player.
        {
            let mut lock = app
                .world_mut()
                .get_mut::<crate::weapons_plugin::TacticalRadarSelection>(npc_entity)
                .expect("TacticalRadarSelection must be attached");
            lock.0 = Some(player_uuid.to_string());
        }

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![
                TriggerAction::AddFactionEnemy {
                    faction: "Harrow".into(),
                    enemy: "Federation".into(),
                },
                TriggerAction::AddFactionEnemy {
                    faction: "Federation".into(),
                    enemy: "Harrow".into(),
                },
                TriggerAction::RemoveFactionEnemy {
                    faction: "Harrow".into(),
                    enemy: "Federation".into(),
                },
            ],
        );

        // The NPC's lock must be cleared because Harrow no longer considers
        // Federation hostile. `ai_target_selection`'s retention tier would
        // otherwise hold the lock forever: it re-checks that the target is alive
        // and in radar range, never that it is still an enemy.
        let lock = app
            .world()
            .get::<crate::weapons_plugin::TacticalRadarSelection>(npc_entity)
            .unwrap();
        assert_eq!(
            lock.0, None,
            "remove_faction_enemy must clear TacticalRadarSelection when the target is no longer hostile"
        );
    }

    // -- Unified [[entity]] name ? uuid pipeline (PRD #337/#339 slice 2) -------

    #[test]
    fn spawn_world_entities_populates_name_to_uuid_for_named_entity() {
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;

        // Build a unified WorldConfig with one named entry (no template
        // resolution needed ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â the helper that mutates `name_to_uuid` runs
        // independently of the asteroid-field spawning path).
        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("starbase_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([500.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/star_sun.toml".into(),
            name: None,
            transform: Some(crate::world::config::TransformConfig {
                position: Some([0.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });

        let mut app = App::new();
        app.insert_resource(world_cfg);
        app.add_systems(Update, spawn_world_entities);
        app.update();

        let cfg = app.world().resource::<UnifiedWorldConfig>();
        assert_eq!(
            cfg.name_to_uuid.len(),
            1,
            "only named [[entity]] entries get a uuid"
        );
        let uuid = cfg
            .name_to_uuid
            .get("starbase_alpha")
            .expect("named entity must register");
        assert!(!uuid.is_empty(), "registered uuid must be non-empty");
    }

    #[test]
    fn spawn_world_entities_mirrors_names_into_world_content_runtime() {
        // PRD #337/#339 slice 2: trigger / comms lookup paths read from
        // `WorldContentRuntime.name_to_uuid`. The unified pipeline must
        // mirror its registrations into that map so the lookup path stays
        // a single source of truth during the transitional slices.
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "assets/entities/station_outpost.toml".into(),
            name: Some("earth".into()),
            ..Default::default()
        });

        let mut app = App::new();
        app.insert_resource(world_cfg);
        app.init_resource::<WorldContentRuntime>();
        // Pre-populate runtime with a legacy entry to prove merge (not overwrite).
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("legacy_spawn".into(), "legacy-uuid".into());

        app.add_systems(Update, spawn_world_entities);
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.name_to_uuid.contains_key("earth"),
            "unified pipeline must mirror named entries into runtime"
        );
        assert!(
            runtime.name_to_uuid.contains_key("legacy_spawn"),
            "pre-existing legacy entries must survive the mirror"
        );
    }

    #[test]
    fn init_world_runtime_preserves_existing_name_to_uuid() {
        // PRD #341: `spawn_world_entities` runs before `init_world_runtime`
        // and writes names from the unified [[entity]] pipeline into
        // `WorldContentRuntime.name_to_uuid`. `init_world_runtime` (which
        // folds `WorldConfig.name_to_uuid` in) must NOT overwrite those ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â
        // otherwise trigger and comms lookups for unified-pipeline names
        // would silently disappear.
        use crate::world::config::WorldConfig as UnifiedWorldConfig;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .name_to_uuid
            .insert("starbase_alpha".into(), "world-config-uuid".into());
        world_cfg
            .name_to_uuid
            .insert("only_in_world".into(), "world-only-uuid".into());

        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.init_resource::<CommsInboxRes>();
        app.insert_resource(WorldResource(crate::messages::WorldData::default()));
        app.insert_resource(world_cfg);

        // Pre-populate the runtime with a value that must survive the merge.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("starbase_alpha".into(), "unified-pipeline-uuid".into());

        app.add_systems(Update, init_world_runtime);
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime
                .name_to_uuid
                .get("starbase_alpha")
                .map(String::as_str),
            Some("unified-pipeline-uuid"),
            "init_world_runtime must preserve unified-pipeline registrations"
        );
        assert_eq!(
            runtime
                .name_to_uuid
                .get("only_in_world")
                .map(String::as_str),
            Some("world-only-uuid"),
            "names that exist only in WorldConfig.name_to_uuid still flow through"
        );
    }

    #[test]
    fn spawn_immediate_entities_spawns_named_non_asteroid_with_registered_uuid() {
        // PRD #339 slice 2 (rejection fix): named [[entity]] entries MUST be
        // spawned as real Bevy entities ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â otherwise triggers / comms resolve
        // to a UUID that has no Transform behind it. The spawned entity's
        // `EntityUuid` component must equal the UUID already registered in
        // `WorldConfig.name_to_uuid` for that name (single source of truth ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â
        // no fresh UUID allocation inside the spawn loop).
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::EntityUuid;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/station.toml".into(),
            name: Some("starbase_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([500.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });
        // An anonymous entry must NOT be spawned by the unified pipeline
        // (the complementary `setup_world` in `server_app.rs` owns it).
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/star.toml".into(),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([0.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });

        // Pre-populate name_to_uuid as `spawn_world_entities`'s
        // assign-uuid pass would have.
        world_cfg
            .name_to_uuid
            .insert("starbase_alpha".into(), "stable-station-uuid".into());

        // Build a fixture ConfigCache with the templates referenced above.
        // Empty EntityConfig is sufficient ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â no asteroid_field section, so
        // `is_owned_by_unified_pipeline` routes by `name.is_some()`.
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/station.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        m.insert(
            "fixture/star.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let world_cfg = app.world().resource::<UnifiedWorldConfig>().clone();
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, None, None)
        };
        app.update();

        // Exactly one entity from the unified pipeline.
        assert_eq!(spawned.len(), 1, "only the named entry must be spawned");

        // Its EntityUuid must equal the registered UUID ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â not a fresh one.
        let uuid_component = app
            .world()
            .get::<EntityUuid>(spawned[0])
            .expect("spawned entity must carry EntityUuid");
        assert_eq!(
            uuid_component.0, "stable-station-uuid",
            "spawned entity's UUID must match the one in WorldConfig.name_to_uuid"
        );

        // And it must have a Transform at the configured position so
        // trigger / comms position queries work.
        let transform = app
            .world()
            .get::<Transform>(spawned[0])
            .expect("spawned entity must have a Transform");
        assert_eq!(transform.translation, Vec3::new(500.0, 0.0, 0.0));
    }

    /// Issue #906: a template whose include closure is broken must BLOCK the
    /// spawn, not cost the world one entity.
    ///
    /// Before this, `build_layer_config_cache` warned and skipped, so the
    /// template simply never entered the cache and the world came up silently
    /// short. The fragment source is seeded through
    /// `config_cache::record_raw_template` — the same channel the browser
    /// preload delivers text on — so the test needs no files on disk.
    #[test]
    fn spawn_is_blocked_when_an_entity_template_cannot_be_composed() {
        use crate::entity_config::EntityConfig;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        crate::config_cache::clear_template_preload_state();
        crate::config_cache::record_raw_template(
            "fixture/broken.toml",
            "includes = [\"absent.toml\"]\n".to_string(),
        );

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/broken.toml".into(),
            name: Some("broken_one".into()),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([0.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/station.toml".into(),
            name: Some("innocent_bystander".into()),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([10.0, 0.0, 0.0]),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg
            .name_to_uuid
            .insert("broken_one".into(), "broken-uuid".into());
        world_cfg
            .name_to_uuid
            .insert("innocent_bystander".into(), "bystander-uuid".into());

        // Both templates are in the cache: the ONLY thing that can stop the
        // spawn is the composition finding, not a cache miss.
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/broken.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        m.insert(
            "fixture/station.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let spawn = |cfg: &UnifiedWorldConfig| -> usize {
            let mut app = App::new();
            app.add_plugins(bevy::time::TimePlugin);
            app.insert_resource(cfg.clone());
            let spawned: Vec<Entity> = {
                let cfg = app.world().resource::<UnifiedWorldConfig>().clone();
                let mut commands = app.world_mut().commands();
                spawn_immediate_entities_internal(&mut commands, &cfg, &cache, None, None)
            };
            app.update();
            spawned.len()
        };

        assert_eq!(
            spawn(&world_cfg),
            0,
            "a composition failure must block the WHOLE world — the bystander must \
             not spawn either, or activation was not atomic"
        );

        // Control: the same world, with the broken template's includes fixed,
        // spawns both. Without this the assertion above could pass for any
        // reason at all.
        crate::config_cache::record_raw_template("fixture/broken.toml", "class = \"ok\"\n".into());
        assert_eq!(spawn(&world_cfg), 2, "the control world must spawn both");

        crate::config_cache::clear_template_preload_state();
    }

    /// Issue #969, through the spawn path rather than the resolver: a moon
    /// authored against the planet's `id` reaches the world at planet+offset,
    /// and a moon authored against a name nothing declares takes the whole
    /// world down with it instead of quietly not existing.
    ///
    /// Shaped after `combat_test.toml`'s gas giant / ice moon pair, including
    /// its split identity — `id` is the short reference an author writes,
    /// `name` the strings.csv key — because that split is the defect.
    #[test]
    fn a_relative_to_moon_spawns_at_the_planet_and_a_typo_blocks_the_world() {
        use crate::entity_config::EntityConfig;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::{TransformConfig, WorldEntity};
        use std::collections::HashMap;

        let world_for = |reference: &str| -> UnifiedWorldConfig {
            let mut cfg = UnifiedWorldConfig::default();
            cfg.entities.push(WorldEntity {
                template_path: "fixture/planet.toml".into(),
                id: Some("gas-giant".into()),
                name: Some("world.entity.gas_giant.name".into()),
                transform: Some(TransformConfig {
                    position: Some([-1200.0, 0.0, 300.0]),
                    ..Default::default()
                }),
                ..Default::default()
            });
            cfg.entities.push(WorldEntity {
                template_path: "fixture/moon.toml".into(),
                id: Some("ice-moon".into()),
                name: Some("world.entity.ice_moon.name".into()),
                transform: Some(TransformConfig {
                    relative_to: Some(reference.into()),
                    offset: Some([125.0, 0.0, 40.0]),
                    ..Default::default()
                }),
                ..Default::default()
            });
            cfg.name_to_uuid
                .insert("world.entity.gas_giant.name".into(), "giant-uuid".into());
            cfg.name_to_uuid
                .insert("world.entity.ice_moon.name".into(), "moon-uuid".into());
            cfg
        };

        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/planet.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        m.insert(
            "fixture/moon.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let spawn = |cfg: &UnifiedWorldConfig| -> (usize, Option<Vec3>) {
            let mut app = App::new();
            app.add_plugins(bevy::time::TimePlugin);
            app.insert_resource(cfg.clone());
            let spawned: Vec<Entity> = {
                let cfg = app.world().resource::<UnifiedWorldConfig>().clone();
                let mut commands = app.world_mut().commands();
                spawn_immediate_entities_internal(&mut commands, &cfg, &cache, None, None)
            };
            app.update();
            let moon = spawned
                .iter()
                .find(|e| {
                    app.world()
                        .get::<EntityUuid>(**e)
                        .is_some_and(|u| u.0 == "moon-uuid")
                })
                .and_then(|e| app.world().get::<Transform>(*e))
                .map(|t| t.translation);
            (spawned.len(), moon)
        };

        // By the planet's authored `id` — what combat_test.toml writes.
        let (count, moon) = spawn(&world_for("gas-giant"));
        assert_eq!(count, 2, "both landmarks must spawn");
        assert_eq!(
            moon,
            Some(Vec3::new(-1075.0, 0.0, 340.0)),
            "the moon must sit at the gas giant plus its offset"
        );

        // By the planet's `name` — the documented reference id still works.
        let (count, moon) = spawn(&world_for("world.entity.gas_giant.name"));
        assert_eq!(count, 2);
        assert_eq!(moon, Some(Vec3::new(-1075.0, 0.0, 340.0)));

        // A reference nothing declares blocks the whole world. Before #969 the
        // count here was 1: the planet spawned, the moon did not, and only a
        // log line said so.
        let (count, moon) = spawn(&world_for("gas_giant"));
        assert_eq!(
            count, 0,
            "an unresolvable relative_to must block activation, not cost one entity"
        );
        assert_eq!(moon, None);
    }

    /// The other half of the immediate spawn, which the test above cannot see.
    ///
    /// `is_owned_by_unified_pipeline` keys on `name`, so an `id`-only entry —
    /// `default.toml`'s `nebula-1`, `combat_test.toml`'s dust cloud — belongs to
    /// `setup_world` in `server_app.rs`, a separate `Startup` system with no
    /// ordering relationship to `spawn_world_entities`. It answers a failed
    /// resolve the same way, by logging and moving on, so a gate on the unified
    /// half alone would turn one missing entity into *half a world*: the stars,
    /// planets and nebulae still there, every named entity and asteroid field
    /// gone. Both halves must refuse the same world.
    #[test]
    fn an_anonymous_entitys_broken_relative_to_blocks_both_immediate_spawn_halves() {
        use crate::entity_config::EntityConfig;
        use crate::lobby::server::WorldResource;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::{TransformConfig, WorldEntity};
        use std::collections::HashMap;

        let world_for = |reference: &str| -> UnifiedWorldConfig {
            let mut cfg = UnifiedWorldConfig::default();
            // Carries a `name` → owned by the unified pipeline.
            cfg.entities.push(WorldEntity {
                template_path: "fixture/planet.toml".into(),
                id: Some("gas-giant".into()),
                name: Some("world.entity.gas_giant.name".into()),
                transform: Some(TransformConfig {
                    position: Some([-1200.0, 0.0, 300.0]),
                    ..Default::default()
                }),
                ..Default::default()
            });
            // `id` but no `name` → anonymous, owned by `setup_world`, and the
            // holder of the broken reference.
            cfg.entities.push(WorldEntity {
                template_path: "fixture/nebula.toml".into(),
                id: Some("nebula-1".into()),
                name: None,
                transform: Some(TransformConfig {
                    relative_to: Some(reference.into()),
                    offset: Some([125.0, 0.0, 40.0]),
                    ..Default::default()
                }),
                ..Default::default()
            });
            cfg.name_to_uuid
                .insert("world.entity.gas_giant.name".into(), "giant-uuid".into());
            cfg
        };

        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/planet.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        m.insert(
            "fixture/nebula.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        // `(unified half, setup_world half)`.
        let halves = |cfg: &UnifiedWorldConfig| -> (usize, usize) {
            let mut app = App::new();
            app.add_plugins(bevy::time::TimePlugin);
            let unified = {
                let mut commands = app.world_mut().commands();
                spawn_immediate_entities_internal(&mut commands, cfg, &cache, None, None).len()
            };
            let mut world_res = WorldResource::default();
            let anonymous = {
                let mut commands = app.world_mut().commands();
                crate::server_app::spawn_anonymous_entities_internal(
                    &mut commands,
                    &mut world_res,
                    cfg,
                    &cache,
                    None,
                )
            };
            app.update();
            (unified, anonymous)
        };

        assert_eq!(
            halves(&world_for("gas-giant")),
            (1, 1),
            "the control world splits one entity to each half"
        );
        assert_eq!(
            halves(&world_for("gas_giant")),
            (0, 0),
            "an unresolvable relative_to must block BOTH immediate-spawn halves, \
             not just the one that owns the entity"
        );
    }

    // -- PRD #337 slice 3: NPCs through unified pipeline ------------------

    #[test]
    fn spawn_immediate_entities_resolves_anchor_position_for_named_entry() {
        // PRD #337 slice 3: a `[[entity]]` with `name = ...` AND
        // `anchor = "..."` (no inline `position`) must be spawned at the
        // anchor's coordinates. This is the migration path for the patrol
        // raider NPC moving off `[[spawn]]`.
        use crate::entity_config::EntityConfig;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .anchors
            .insert("patrol_alpha".into(), [300.0, 0.0, -300.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/raider.toml".into(),
            name: Some("raider_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                anchor: Some("patrol_alpha".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg
            .name_to_uuid
            .insert("raider_alpha".into(), "raider-uuid-001".into());

        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/raider.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, None, None)
        };
        app.update();

        assert_eq!(spawned.len(), 1, "exactly one named entry must spawn");

        let transform = app
            .world()
            .get::<Transform>(spawned[0])
            .expect("spawned entity must have Transform");
        assert_eq!(
            transform.translation,
            Vec3::new(300.0, 0.0, -300.0),
            "named entity with anchor must be positioned at the anchor"
        );
    }

    #[test]
    fn spawn_immediate_entities_wires_behaviour_for_npc_with_anchor() {
        // PRD #337 slice 3: a named [[entity]] whose template carries a
        // [behaviour] block must end up with a BehaviourSection ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â the
        // AiPlugin's `attach_controllers_on_spawn` system reads that to
        // wire the AiController. This guarantees NPCs migrated from
        // [[spawn]] to [[entity]] still get AI on spawn.
        use crate::entity_config::EntityConfig;
        use crate::entity_spawner::BehaviourSection;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let raider_toml = r#"
tags = ["ship","npc","enemy"]

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Destroy hostiles"
directive_kind = "Destroy"
base_priority = 35.0
"#;
        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .anchors
            .insert("patrol_alpha".into(), [300.0, 0.0, -300.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/raider.toml".into(),
            name: Some("raider_alpha".into()),
            transform: Some(crate::world::config::TransformConfig {
                anchor: Some("patrol_alpha".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        world_cfg
            .name_to_uuid
            .insert("raider_alpha".into(), "raider-uuid-002".into());

        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/raider.toml".into(),
            EntityConfig::from_toml_in_mode(
                raider_toml,
                crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
            )
            .unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, None, None)
        };
        app.update();

        assert_eq!(spawned.len(), 1);
        assert!(
            app.world().get::<BehaviourSection>(spawned[0]).is_some(),
            "NPC spawned through unified pipeline must carry BehaviourSection so AiPlugin registers its token"
        );
    }

    #[test]
    fn spawn_immediate_entities_resolves_anchor_offset_on_asteroid_field() {
        // PRD #397 fix 5: an asteroid_field template whose
        // `[asteroid_field] anchor = "name"` references a known world anchor
        // must have its `anchor_offset` populated with that anchor's world
        // position. The streaming spawner reads this offset to translate
        // the eligibility annulus and per-asteroid spawn positions.
        use crate::entity_config::{
            AsteroidFieldConfig, AsteroidFieldShape, EntityConfig, GridConfig,
        };
        use crate::entity_spawner::AsteroidFieldSection;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .anchors
            .insert("belt_origin".into(), [500.0, 0.0, -250.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/anchored_belt.toml".into(),
            ..Default::default()
        });

        // Build an EntityConfig in the cache with an asteroid_field that
        // references the anchor. Anchor offset starts at origin and must
        // be filled in by `spawn_immediate_entities_internal`.
        let mut field_template = EntityConfig::from_toml("").unwrap();
        field_template.asteroid_field = Some(AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 200.0,
            density: 0.005,
            weight: 1.0,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["a.toml".into()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(GridConfig {
                resolution: 15.0,
                fill_gameplay: 0.4,
                fill_cosmetic: 0.0,
                uniformity: 0.3,
                noise_freq: 0.02,
                noise_octaves: 1,
                density_noise_freq: 0.01,
                density_noise_octaves: 1,
                jitter: 0.0,
                cosmetic_y_offset: 0.0,
                gameplay_y_variance: 0.0,
                spawn_cells: 4,
                despawn_cells: 6,
            }),
            shield_pierce: 0.0,
            shape: Some(AsteroidFieldShape::Torus),
            anchor: Some("belt_origin".into()),
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        });
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert("fixture/anchored_belt.toml".into(), field_template);
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, None, None)
        };
        app.update();

        assert_eq!(
            spawned.len(),
            1,
            "exactly one asteroid_field entry must spawn"
        );
        let section = app
            .world()
            .get::<AsteroidFieldSection>(spawned[0])
            .expect("asteroid_field entry must carry an AsteroidFieldSection");
        assert_eq!(
            section.0.anchor_offset,
            [500.0, 0.0, -250.0],
            "spawn-time resolution must copy the anchor's world position into anchor_offset"
        );
        assert_eq!(
            section.0.anchor.as_deref(),
            Some("belt_origin"),
            "anchor name must be preserved alongside the resolved offset"
        );
    }

    /// Issue #973 review (F1): the ownership predicate must ask the same
    /// question the spawn asks.
    ///
    /// #973 widened spawning from cache-only to cache-then-host. While the
    /// three-way partition stayed cache-only, an asteroid-field template that
    /// was on disk but absent from the `ConfigCache` answered "not a field",
    /// fell into the anonymous bucket, and spawned through `setup_world` —
    /// which never runs the `[asteroid_field] anchor -> anchor_offset` block
    /// and *does* `upsert_world_entity`, which the field path deliberately
    /// omits. The world then comes up almost right, with its belts quietly at
    /// the world origin: a quieter failure than the blank world #973 replaced,
    /// which is the wrong direction.
    ///
    /// Reachable today, not theoretical: `headless::app::build_headless_app`
    /// derives the preload directory from `--ship`'s parent, so
    /// `--ship assets/entities/test/rng_coverage_lancer.toml
    /// --world assets/worlds/combat_test.toml` preloads only
    /// `assets/entities/test/` and both of `combat_test.toml`'s
    /// `asteroid_field_main.toml` belts miss the cache.
    #[test]
    fn a_disk_only_asteroid_field_still_routes_to_the_field_spawn_path() {
        use crate::entity_spawner::AsteroidFieldSection;
        use crate::lobby::server::WorldResource;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicU32, Ordering};

        // On disk and deliberately NOT in the cache — the exact shape the
        // headless preload leaves behind when the world's belts live outside
        // `--ship`'s directory.
        static C: AtomicU32 = AtomicU32::new(0);
        let tag = C.fetch_add(1, Ordering::Relaxed);
        let path_buf = std::env::temp_dir().join(format!("disk_only_belt_{tag}.toml"));
        std::fs::write(
            &path_buf,
            "[asteroid_field]\n\
             inner_radius = 100.0\n\
             outer_radius = 200.0\n\
             density = 0.005\n\
             anchor = \"belt_origin\"\n",
        )
        .expect("write the disk-only belt fixture");
        let path = path_buf.to_string_lossy().into_owned();

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg
            .anchors
            .insert("belt_origin".into(), [500.0, 0.0, -250.0]);
        // An `id` but no `name` — exactly how `combat_test.toml` authors its
        // two belts, so `name` cannot be what routes this entry.
        world_cfg.entities.push(WorldEntity {
            template_path: path.clone(),
            id: Some("inner-belt".into()),
            ..Default::default()
        });

        let cache = crate::config_cache::ConfigCache::from(HashMap::new());
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, None, None)
        };
        let mut world_res = WorldResource::default();
        let anonymous = {
            let mut commands = app.world_mut().commands();
            crate::server_app::spawn_anonymous_entities_internal(
                &mut commands,
                &mut world_res,
                &world_cfg,
                &cache,
                None,
            )
        };
        app.update();

        assert_eq!(
            spawned.len(),
            1,
            "the unified half owns asteroid fields, and a field it can spawn is \
             a field it must claim"
        );
        assert_eq!(
            anonymous, 0,
            "…and the `setup_world` half must not claim it as well"
        );
        let section = app
            .world()
            .get::<AsteroidFieldSection>(spawned[0])
            .expect("a field entry must carry an AsteroidFieldSection");
        assert_eq!(
            section.0.anchor_offset,
            [500.0, 0.0, -250.0],
            "only the field path resolves `[asteroid_field] anchor`; a mis-routed \
             belt sits silently at the world origin"
        );

        let _ = std::fs::remove_file(&path_buf);
    }

    #[test]
    fn duplicate_reference_name_spawns_zero_entities() {
        // Atomic activation (issue #750): a world whose entity identity is
        // invalid (two [[entity]] blocks sharing a reference name) must spawn
        // NOTHING — no partial root-world content.
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/a.toml".into(),
            name: Some("outpost".into()),
            ..Default::default()
        });
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/b.toml".into(),
            name: Some("outpost".into()),
            ..Default::default()
        });

        let cache = crate::config_cache::ConfigCache::from(HashMap::new());
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, None, None)
        };
        app.update();

        assert!(
            spawned.is_empty(),
            "a duplicate reference name is a composition error; zero entities must spawn"
        );
    }

    #[test]
    fn spawn_immediate_entities_falls_back_to_origin_when_anchor_unknown() {
        // PRD #397 fix 5: an asteroid_field referencing an anchor name that
        // is NOT present in `WorldConfig.anchors` must fall back to the
        // world origin (`anchor_offset = [0,0,0]`) rather than silently
        // relocating somewhere unexpected or refusing to spawn. Implementation
        // also logs a `warn!` so the typo is visible in console output.
        use crate::entity_config::{
            AsteroidFieldConfig, AsteroidFieldShape, EntityConfig, GridConfig,
        };
        use crate::entity_spawner::AsteroidFieldSection;
        use crate::world::config::WorldConfig as UnifiedWorldConfig;
        use crate::world::config::WorldEntity;
        use std::collections::HashMap;

        let mut world_cfg = UnifiedWorldConfig::default();
        // Note: NO anchor named "typo_anchor" ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â only "real_anchor".
        world_cfg
            .anchors
            .insert("real_anchor".into(), [999.0, 0.0, 999.0]);
        world_cfg.entities.push(WorldEntity {
            template_path: "fixture/typo_belt.toml".into(),
            ..Default::default()
        });

        let mut field_template = EntityConfig::from_toml("").unwrap();
        field_template.asteroid_field = Some(AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 200.0,
            density: 0.005,
            weight: 1.0,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["a.toml".into()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: Some(GridConfig {
                resolution: 15.0,
                fill_gameplay: 0.4,
                fill_cosmetic: 0.0,
                uniformity: 0.3,
                noise_freq: 0.02,
                noise_octaves: 1,
                density_noise_freq: 0.01,
                density_noise_octaves: 1,
                jitter: 0.0,
                cosmetic_y_offset: 0.0,
                gameplay_y_variance: 0.0,
                spawn_cells: 4,
                despawn_cells: 6,
            }),
            shield_pierce: 0.0,
            shape: Some(AsteroidFieldShape::Torus),
            anchor: Some("typo_anchor".into()),
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        });
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert("fixture/typo_belt.toml".into(), field_template);
        let cache = crate::config_cache::ConfigCache::from(m);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(world_cfg.clone());

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, None, None)
        };
        app.update();

        assert_eq!(
            spawned.len(),
            1,
            "unknown anchor must NOT block spawn ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â fallback to origin keeps the field alive"
        );
        let section = app
            .world()
            .get::<AsteroidFieldSection>(spawned[0])
            .expect("asteroid_field entry must still carry an AsteroidFieldSection");
        assert_eq!(
            section.0.anchor_offset,
            [0.0, 0.0, 0.0],
            "missing anchor must fall back to world origin, not the only other known anchor"
        );
    }

    // ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ extra_worlds + LoadWorld / UnloadWorld (issue #352) ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬

    /// Helper: build an `App` with `WorldLayerMap`, `WorldContentRuntime`, and
    /// the `apply_world_layer_changes` system wired in.  No LobbyPlugin needed.
    fn layer_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);
        app
    }

    /// `extra_worlds` on `WorldConfig` starts empty by default.
    #[test]
    fn world_config_extra_worlds_defaults_to_empty() {
        let cfg = crate::world::config::WorldConfig::default();
        assert!(cfg.extra_worlds.is_empty());
    }

    /// `load_extra_worlds` queues one `Load` command per `extra_worlds` entry.
    #[test]
    fn load_extra_worlds_startup_queues_pending_layer_changes() {
        let mut app = App::new();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<PendingWorldLayerChanges>();

        let mut world_cfg = crate::world::config::WorldConfig::default();
        world_cfg
            .extra_worlds
            .push("tests/fixtures/layer_entities.toml".into());
        world_cfg
            .extra_worlds
            .push("assets/worlds/side.toml".into());
        app.insert_resource(world_cfg);

        app.add_systems(Startup, load_extra_worlds);
        app.world_mut().run_schedule(Startup);

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(
            pending.0.len(),
            2,
            "one Load command per extra_worlds entry"
        );
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Load { path: p, .. } if p == "tests/fixtures/layer_entities.toml")
        );
        assert!(
            matches!(&pending.0[1], WorldLayerChange::Load { path: p, .. } if p == "assets/worlds/side.toml")
        );
    }

    /// A `LoadWorld` action queues a `Load` command into
    /// `PendingWorldLayerChanges` rather than loading inline, so the one
    /// `apply_world_layer_changes` applier owns every load.
    ///
    /// Dispatched through the delayed-action queue since issue #985 deleted the
    /// `[[trigger]]` action array it used to be authored on.
    #[test]
    fn load_world_action_queues_pending_layer_change() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>();

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::LoadWorld {
                path: "tests/fixtures/layer_entities.toml".into(),
            }],
        );

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(pending.0.len(), 1, "one Load must be queued");
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Load { path: p, .. } if p == "tests/fixtures/layer_entities.toml")
        );
    }

    /// The `UnloadWorld` mirror of `load_world_action_queues_pending_layer_change`.
    #[test]
    fn unload_world_action_queues_pending_layer_change() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>();

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::UnloadWorld {
                path: "tests/fixtures/layer_entities.toml".into(),
            }],
        );

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(pending.0.len(), 1, "one Unload must be queued");
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Unload(p) if p == "tests/fixtures/layer_entities.toml")
        );
    }

    /// How many ECS entities were spawned by the layer at `path`.
    ///
    /// The layer contract's one observable: a loaded layer contributes ENTITIES
    /// and — until script-in-layers (#1045) — nothing else, because issue #985
    /// deleted the `[[trigger]]` parser that was a layer's only way to author
    /// scenario logic. See `tests/fixtures/layer_entities.toml`.
    fn layer_entity_count(app: &App, path: &str) -> usize {
        app.world()
            .resource::<WorldLayerMap>()
            .0
            .get(path)
            .map(|layer| layer.spawned_entities.len())
            .unwrap_or(0)
    }

    /// `apply_world_layer_changes` with a `Load` reads the TOML on native,
    /// registers the layer in `WorldLayerMap`, spawns its `[[entity]]` blocks and
    /// registers their names in the live runtime.
    #[test]
    fn load_world_action_registers_the_layers_entities() {
        let mut app = layer_test_app();

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "tests/fixtures/layer_entities.toml".into(),
                loader_path: None,
            });

        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        assert!(
            layer_map
                .0
                .contains_key("tests/fixtures/layer_entities.toml"),
            "WorldLayerMap must contain the loaded path"
        );
        assert_eq!(
            layer_entity_count(&app, "tests/fixtures/layer_entities.toml"),
            1,
            "the layer's one [[entity]] block must spawn"
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime
                .name_to_uuid
                .contains_key("test.layer_fixture.raider"),
            "the layer's named entity must be registered for name lookups"
        );
    }

    /// A second `LoadWorld` for the same path is a no-op (de-duplicated).
    #[test]
    fn load_world_is_deduped_when_already_in_layer_map() {
        let mut app = layer_test_app();

        // Load once.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "tests/fixtures/layer_entities.toml".into(),
                loader_path: None,
            });
        app.update();

        let after_first = layer_entity_count(&app, "tests/fixtures/layer_entities.toml");
        assert_eq!(after_first, 1, "the first load spawns the layer's entity");

        // Load again — must not double-spawn.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "tests/fixtures/layer_entities.toml".into(),
                loader_path: None,
            });
        app.update();

        assert_eq!(
            after_first,
            layer_entity_count(&app, "tests/fixtures/layer_entities.toml"),
            "duplicate LoadWorld must not spawn the layer's entities twice"
        );
    }

    /// `UnloadWorld` despawns exactly the entities the matching `LoadWorld`
    /// spawned, and drops the layer from `WorldLayerMap`.
    ///
    /// It asserted on the layer's trigger states until issue #985 deleted the
    /// `[[trigger]]` parser; a layer now contributes only entities, so those are
    /// what the unload cascade has to take back out.
    #[test]
    fn unload_world_despawns_entities_added_by_load_world() {
        let mut app = layer_test_app();

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "tests/fixtures/layer_entities.toml".into(),
                loader_path: None,
            });
        app.update();

        let spawned: Vec<Entity> = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get("tests/fixtures/layer_entities.toml")
            .expect("layer present")
            .spawned_entities
            .clone();
        assert!(
            !spawned.is_empty(),
            "the fixture layer must spawn at least one entity"
        );

        // Unload it.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(
                "tests/fixtures/layer_entities.toml".into(),
            ));
        app.update();

        for entity in spawned {
            assert!(
                app.world().get_entity(entity).is_err(),
                "UnloadWorld must despawn every entity the load spawned"
            );
        }

        let layer_map = app.world().resource::<WorldLayerMap>();
        assert!(
            !layer_map
                .0
                .contains_key("tests/fixtures/layer_entities.toml"),
            "WorldLayerMap must no longer contain the unloaded path"
        );
    }

    /// Issue #751: unloading a layer removes exactly the objectives its
    /// triggers added (tracked in `WorldRuntime.owned_objective_ids`), leaving
    /// base-world objectives untouched.
    #[test]
    fn unload_world_removes_layer_owned_objectives() {
        let mut app = layer_test_app();
        app.init_resource::<ObjectiveManagerRes>();

        {
            let mut obj = app.world_mut().resource_mut::<ObjectiveManagerRes>();
            obj.0.add("base-obj", "Base objective", false, vec![]);
            obj.0.add("layer-obj", "Layer objective", false, vec![]);
        }
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                "sub.toml".to_string(),
                WorldRuntime {
                    owned_objective_ids: vec!["layer-obj".to_string()],
                    ..Default::default()
                },
            );
        }

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload("sub.toml".to_string()));
        app.update();

        let ids: Vec<String> = app
            .world()
            .resource::<ObjectiveManagerRes>()
            .0
            .sorted_snapshots()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            ids,
            vec!["base-obj".to_string()],
            "only the layer's objective is removed; the base objective survives"
        );
    }

    /// Issue #752: unloading a layer also prunes the runtime state that
    /// referenced its objectives — a captain priority boost pointing at a
    /// removed objective, and any per-ship route cursor keyed to it — while
    /// leaving unrelated boosts and cursors intact.
    #[test]
    fn unload_world_prunes_boost_and_cursor_for_removed_objectives() {
        use crate::ai::patrol_cursor::PatrolCursor;
        use crate::ai::server::ObjectiveCursors;
        use crate::server_app::CaptainPriorityBoost;

        let mut app = layer_test_app();
        app.init_resource::<ObjectiveManagerRes>();
        app.init_resource::<CaptainPriorityBoost>();

        {
            let mut obj = app.world_mut().resource_mut::<ObjectiveManagerRes>();
            obj.0.add("base-obj", "Base objective", false, vec![]);
            obj.0.add("layer-obj", "Layer objective", false, vec![]);
        }
        {
            let mut boost = app.world_mut().resource_mut::<CaptainPriorityBoost>();
            // A boost on the layer's objective (to be pruned) and one on the
            // base objective (must survive), in different ship scopes.
            boost.toggle("ship-a", "layer-obj");
            boost.toggle("ship-b", "base-obj");
        }
        // A ship carrying cursors for both objectives.
        app.world_mut().spawn(ObjectiveCursors(vec![
            PatrolCursor::new("layer-obj"),
            PatrolCursor::new("base-obj"),
        ]));
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                "sub.toml".to_string(),
                WorldRuntime {
                    owned_objective_ids: vec!["layer-obj".to_string()],
                    ..Default::default()
                },
            );
        }

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload("sub.toml".to_string()));
        app.update();

        let boost = app.world().resource::<CaptainPriorityBoost>();
        assert_eq!(
            boost.boosted_for("ship-a"),
            None,
            "the boost on the removed objective must be pruned"
        );
        assert_eq!(
            boost.boosted_for("ship-b"),
            Some("base-obj"),
            "the boost on the surviving objective is untouched"
        );

        let cursor_ids: Vec<String> = {
            let mut q = app.world_mut().query::<&ObjectiveCursors>();
            q.single(app.world())
                .unwrap()
                .0
                .iter()
                .map(|c| c.objective_id.clone())
                .collect()
        };
        assert_eq!(
            cursor_ids,
            vec!["base-obj".to_string()],
            "the cursor keyed to the removed objective is pruned; the other survives"
        );
    }

    /// Issue #751: with the default (`Cancel`) policy, unloading a layer drops
    /// its pending delayed actions and leaves other layers' actions intact.
    #[test]
    fn unload_world_cancels_layer_delayed_actions_by_default() {
        let mut app = layer_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.pending_delayed_actions = vec![
                DelayedAction {
                    action: TriggerAction::SetWorldFlag {
                        name: "base".into(),
                    },
                    origin_layer: None,
                    entity_name: None,
                    fire_at_elapsed: 5.0,
                },
                DelayedAction {
                    action: TriggerAction::SetWorldFlag { name: "sub".into() },
                    origin_layer: Some("sub.toml".into()),
                    entity_name: None,
                    fire_at_elapsed: 5.0,
                },
            ];
        }
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                "sub.toml".to_string(),
                WorldRuntime {
                    delayed_unload_resolve: false,
                    ..Default::default()
                },
            );
        }

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload("sub.toml".to_string()));
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.pending_delayed_actions.len(),
            1,
            "the layer's delayed action is cancelled; the base action remains"
        );
        assert!(runtime.pending_delayed_actions[0].origin_layer.is_none());
    }

    /// Issue #751: with the `Resolve` policy, unloading a layer keeps its
    /// pending delayed actions but pulls their fire time to 0 so the next
    /// delayed-action tick dispatches them immediately.
    #[test]
    fn unload_world_resolves_layer_delayed_actions_when_policy_is_resolve() {
        let mut app = layer_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.pending_delayed_actions = vec![DelayedAction {
                action: TriggerAction::SetWorldFlag { name: "sub".into() },
                origin_layer: Some("sub.toml".into()),
                entity_name: None,
                fire_at_elapsed: 100.0,
            }];
        }
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(
                "sub.toml".to_string(),
                WorldRuntime {
                    delayed_unload_resolve: true,
                    ..Default::default()
                },
            );
        }

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload("sub.toml".to_string()));
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.pending_delayed_actions.len(),
            1,
            "resolved actions are kept, not cancelled"
        );
        assert_eq!(
            runtime.pending_delayed_actions[0].fire_at_elapsed, 0.0,
            "resolved actions fire immediately on the next tick"
        );
    }

    /// Two `LoadWorld` commands for the same path queued within a single tick
    /// produce exactly one load ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â no duplicate entities, no duplicate trigger
    /// states, no duplicate `WorldLayerMap` entry (issue #413).
    #[test]
    fn two_load_world_same_path_same_tick_is_single_load() {
        let (world_path, _template_path) = write_layer_entity_fixtures();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        // Two triggers in the same world both request the same load in one tick.
        {
            let mut pending = app.world_mut().resource_mut::<PendingWorldLayerChanges>();
            pending.0.push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
            pending.0.push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        }
        // First update: apply_world_layer_changes drains both commands; the
        // second must be a no-op because the first already inserted into
        // WorldLayerMap.
        app.update();
        // Second update: Bevy flushes deferred spawns.
        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        let layer = layer_map
            .0
            .get(&world_path)
            .expect("WorldLayerMap must contain the loaded path");

        // Exactly one layer entry (a Vec-backed entity list would otherwise
        // hold double the entities).
        let entity_count = layer.spawned_entities.len();
        assert!(
            entity_count > 0,
            "precondition: world fixture must spawn at least one entity"
        );

        // Drop borrow before mutating pending again.
        let _ = layer_map;

        // Capture trigger/contact/name counts after the first-tick double-load.
        let runtime = app.world().resource::<WorldContentRuntime>();
        let triggers_after_double = runtime.trigger_states.len();
        let names_after_double = runtime.name_to_uuid.len();
        let contacts_after_double = app.world().resource::<CommsRuntime>().contacts.len();
        let _ = runtime;

        // Now load the same path AGAIN on a separate tick: must also be a
        // no-op (existing behaviour) and keep the same counts.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update();
        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        let layer = layer_map
            .0
            .get(&world_path)
            .expect("layer must still be present");
        assert_eq!(
            layer.spawned_entities.len(),
            entity_count,
            "second-tick duplicate LoadWorld must not double-spawn entities"
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.trigger_states.len(),
            triggers_after_double,
            "duplicate LoadWorld must not add duplicate trigger states"
        );
        assert_eq!(
            runtime.name_to_uuid.len(),
            names_after_double,
            "duplicate LoadWorld must not re-register named entities"
        );
        assert_eq!(
            app.world().resource::<CommsRuntime>().contacts.len(),
            contacts_after_double,
            "duplicate LoadWorld must not duplicate comms contacts"
        );
    }

    /// `UnloadWorld` for a path that was never loaded is a silent no-op.
    #[test]
    fn unload_world_unknown_path_is_noop() {
        let mut app = layer_test_app();

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(
                "assets/worlds/nonexistent.toml".into(),
            ));
        app.update(); // must not panic

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(runtime.trigger_states.is_empty());
    }

    // ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ Entity spawn / despawn via LoadWorld / UnloadWorld (issue #352) ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬

    /// Write a minimal world TOML and a stub entity template to temp files,
    /// return `(world_path, template_path)` as `String`s.
    ///
    /// The world has one named `[[entity]]` so we get a predictable spawn count
    /// without relying on the shipped `patrol.toml` config-cache.  Uses an
    /// atomic counter for unique paths so parallel test runs don't collide.
    fn write_layer_entity_fixtures() -> (String, String) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let tmp = std::env::temp_dir();
        let tag = COUNTER.fetch_add(1, Ordering::Relaxed);
        let template_path = tmp.join(format!("layer_test_npc_{tag}.toml"));
        let world_path = tmp.join(format!("layer_test_world_{tag}.toml"));

        let template_toml = r##"
tags = ["npc"]

[appearance]
colour = "#888888"
size_min = 1.0
size_max = 2.0
"##;
        std::fs::write(&template_path, template_toml).expect("failed to write stub template");

        let template_path_str = template_path.to_string_lossy().replace('\\', "/");

        // Entities only: the world carried an `on_destroyed` `[[trigger]]` block
        // as well, but issue #985 deleted that parser and a world still
        // authoring one no longer parses at all — which would leave these
        // fixtures loading nothing.
        let world_toml = format!(
            r#"
[global]
seed = 1

[[entity]]
template_path = "{template_path_str}"
name = "layer_npc"
position = [1.0, 0.0, 0.0]
"#,
        );

        std::fs::write(&world_path, &world_toml).expect("failed to write layer world TOML");

        (
            world_path.to_string_lossy().into_owned(),
            template_path.to_string_lossy().into_owned(),
        )
    }

    /// `LoadWorld` spawns the world's `[[entity]]` blocks into the ECS and
    /// records them in `WorldLayerMap`.
    #[test]
    fn load_world_spawns_entities_into_ecs() {
        let (world_path, _template_path) = write_layer_entity_fixtures();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });

        // First update: commands are queued by apply_world_layer_changes.
        app.update();
        // Second update: Bevy flushes deferred commands, entities become real.
        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        let layer = layer_map
            .0
            .get(&world_path)
            .expect("WorldLayerMap must contain the loaded path");

        assert!(
            !layer.spawned_entities.is_empty(),
            "LoadWorld must record spawned entity handles in WorldLayerMap"
        );

        // Every recorded entity must actually exist in the ECS.
        for &entity in &layer.spawned_entities {
            assert!(
                app.world().get_entity(entity).is_ok(),
                "entity {entity:?} recorded in WorldLayerMap must exist in ECS after LoadWorld"
            );
        }
    }

    /// `UnloadWorld` despawns the ECS entities that were spawned by the
    /// matching `LoadWorld`.
    #[test]
    fn unload_world_despawns_entities_from_ecs() {
        let (world_path, _template_path) = write_layer_entity_fixtures();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        // Load first.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update();
        app.update();

        // Capture the spawned entity handles before unload.
        let spawned_before: Vec<Entity> = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get(&world_path)
            .expect("must be loaded")
            .spawned_entities
            .clone();

        assert!(
            !spawned_before.is_empty(),
            "precondition: LoadWorld must have spawned at least one entity"
        );

        // Now unload.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(world_path.clone()));
        app.update();
        app.update();

        // Each entity spawned by LoadWorld must now be gone.
        for entity in spawned_before {
            assert!(
                app.world().get_entity(entity).is_err(),
                "entity {entity:?} must be despawned after UnloadWorld"
            );
        }

        // WorldLayerMap entry must be removed.
        assert!(
            !app.world()
                .resource::<WorldLayerMap>()
                .0
                .contains_key(&world_path),
            "WorldLayerMap must not contain the path after UnloadWorld"
        );
    }

    // -- Issue #717: trigger-pipeline edge cases ------------------------------

    /// (#717) An empty world — no triggers, no entities, no queued events —
    /// must flow through the collect → inject → pipeline chain as a pure
    /// no-op: nothing panics, no state appears, and (pinning the #716/#718
    /// change-detection discipline) neither `WorldContentRuntime` nor
    /// `WorldEventBuffer` is marked changed on an event-free tick.
    #[test]
    fn tick_trigger_pipeline_is_a_noop_on_an_empty_world() {
        #[derive(Resource, Default)]
        struct ChangeProbe {
            runtime_changed: bool,
            buffer_changed: bool,
        }

        fn probe(
            runtime: Res<WorldContentRuntime>,
            buffer: Res<WorldEventBuffer>,
            mut out: ResMut<ChangeProbe>,
        ) {
            out.runtime_changed = runtime.is_changed();
            out.buffer_changed = buffer.is_changed();
        }

        // Deliberately bare — no TimePlugin (so no `TimerElapsed` synthesis)
        // and no Lobby/Ai plugin systems that might touch the probed
        // resources. This is the "minimal test apps without TimePlugin" shape
        // the pipeline's docs promise keeps working.
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<WorldEventBuffer>()
            .init_resource::<ChangeProbe>()
            .add_message::<CommsChannel2Event>()
            .add_message::<crate::ai_plugin::AiEntityAttacked>()
            .add_message::<crate::ai_plugin::AiEntityDestroyed>()
            .add_message::<crate::ai_plugin::AiWaypointReached>()
            .add_systems(
                Update,
                (collect_world_events, tick_trigger_pipeline, probe).chain(),
            );

        // First tick: the probe sees everything "changed" because the
        // resources were freshly inserted — it only proves nothing panics.
        app.update();
        // Second tick is the discriminating one: with no events and no
        // triggers none of the three systems may mutably dereference the
        // resources, so the probe must see them unchanged.
        app.update();

        let probe_out = app.world().resource::<ChangeProbe>();
        assert!(
            !probe_out.runtime_changed,
            "an event-free tick must not mark WorldContentRuntime changed"
        );
        assert!(
            !probe_out.buffer_changed,
            "an event-free tick must not mark WorldEventBuffer changed"
        );
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.trigger_states.is_empty(),
            "no trigger state may appear"
        );
        assert!(
            runtime.pending_world_events.is_empty(),
            "no world events may be synthesised from nothing"
        );
        assert!(
            app.world().resource::<WorldEventBuffer>().0.is_empty(),
            "the buffer must stay empty"
        );
        assert!(
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .is_empty(),
            "no objectives may appear"
        );
    }

    /// (#717) A trigger chain longer than `MAX_CHAIN_PASSES` must be cut off
    /// at the cap (with a warning) instead of hanging the frame. Builds a
    /// linear chain of `MAX_CHAIN_PASSES + 2` single-shot triggers: the seed
    /// fires on the external `WorldLoaded` event and sets `chain_1`; link `i`
    /// fires on `on_flag_set chain_i` and sets `chain_{i+1}`. Each pass of
    /// the within-tick chaining loop advances the chain by exactly one link,
    /// so the observable cut is: `chain_1..=chain_MAX` set (that much
    /// dispatch work happened), `chain_{MAX+1}` never set (the cap broke the
    /// loop before pass MAX+1), and the corresponding trigger never consumed.
    ///
    /// Each link is a SCRIPTED trigger. It was a declarative `[[trigger]]` chain
    /// until issue #985 deleted the action array, and a scripted handler's
    /// `ctx.flags` write is what still chains within a tick — `apply_script_commands`
    /// previews the transition and pushes the `FlagSet` into the pass's
    /// `next_events`, exactly where a declarative `set_flag` used to put it.
    #[test]
    fn trigger_chain_exceeding_max_passes_stops_at_the_cap() {
        let chain_len = (MAX_CHAIN_PASSES + 2) as usize;
        // Seed link plus one link per chain step: link i fires on chain_i and
        // sets chain_{i+1}.
        let mut setup =
            String::from(r#"on_world_loaded("h0"); fn h0(ctx) { ctx.flags.chain_1 = 1; }"#);
        for i in 1..chain_len {
            setup.push_str(&format!(
                r#" on_flag_set("chain_{i}", "h{i}"); fn h{i}(ctx) {{ ctx.flags.chain_{next} = 1; }}"#,
                next = i + 1
            ));
        }
        let mut sr = compile_fixture_scripts(&format!("[script]\nsetup = '{setup}'\n"));
        assert_eq!(sr.triggers.len(), chain_len, "one trigger per chain link");

        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }
        app.world_mut().insert_resource(sr);

        // A single update must terminate — the cap is what guarantees it.
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        let max = MAX_CHAIN_PASSES as usize;
        for i in 1..=max {
            assert_eq!(
                runtime.flags.counter(&format!("chain_{i}")),
                1,
                "pass {i} of the chaining loop must set chain_{i}"
            );
        }
        assert_eq!(
            runtime.flags.counter(&format!("chain_{}", max + 1)),
            0,
            "the MAX_CHAIN_PASSES cap must stop the chain before pass {}",
            max + 1
        );
        assert!(
            !runtime.trigger_states[max].fired,
            "the link past the cap must never fire (its flag never got set)"
        );
    }

    /// (#717) The per-trigger flag-store chain that `tick_trigger_pipeline`
    /// builds for condition evaluation must fall back to the BASE store when
    /// the trigger's `origin_layer` is missing from `WorldLayerMap`. The
    /// `when` predicate here only passes if the chain's first entry is the
    /// base store — an empty stand-in store would read `armed` as unset and
    /// suppress the trigger. The layer map is deliberately non-empty so the
    /// fallback is proven per-entry, not merely "map absent".
    ///
    /// The pure dispatch-layer twins of this behaviour (the `parent:` walk
    /// inside `dispatch_world_flag_action`) are
    /// `dispatch::tests::flag_layer_missing_mid_walk_is_silent_and_treated_as_base`
    /// / `world_flag_layer_missing_mid_walk_is_silent_and_treated_as_base_directly`,
    /// and the final-lookup-missing abort is
    /// `flag_target_layer_missing_from_map_warns_and_emits_nothing`. This
    /// test drives the Bevy-side chain builder those tests cannot reach.
    #[test]
    fn trigger_from_layer_missing_in_layer_map_reads_base_flags() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        {
            let mut layer_map = app.world_mut().resource_mut::<WorldLayerMap>();
            layer_map.0.insert(
                "assets/worlds/unrelated.toml".to_string(),
                WorldRuntime::default(),
            );
        }
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("armed"); // set in the BASE store only
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnWorldLoaded,
                    when: Some(crate::world::flags::parse_predicate("flag(armed)").unwrap()),
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                // Not present in WorldLayerMap — e.g. state surviving an
                // unload. The chain builder must treat it as the base world.
                origin_layer: Some("assets/worlds/ghost.toml".to_string()),
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();

        assert!(
            trigger_fired(&app, 0),
            "a missing origin layer must fall back to the base flag store, \
             so the base-store `armed` flag must satisfy the when predicate"
        );
    }

    /// (#717) A queue carrying every `TriggerAction` variant (all 21) must
    /// dispatch them in QUEUE ORDER, each result applied before the next action
    /// is dispatched. Order is observed through order-sensitive pairs rather
    /// than instrumentation:
    ///
    /// * `AddObjective` → `CompleteObjective`/`FailObjective`: complete/fail
    ///   only transition Active objectives, so the final statuses prove the
    ///   adds ran first.
    /// * `Apply*` → `Remove*` (float / int / flag modifiers): the removes
    ///   only find what the applies wrote, so baseline end values prove
    ///   apply-before-remove.
    /// * `SetWorldFlag` → `ClearWorldFlag` on one flag: against the live
    ///   store this emits `FlagSet` then `FlagCleared`; the chained
    ///   `on_flag_cleared` trigger firing proves the order (a clear-first
    ///   order would emit no `FlagCleared` transition at all).
    /// * `SetWorldFlagValue(40)` → `IncrementWorldFlag(+2)`: final counter 42
    ///   (the reverse order would end at 40).
    /// * `AddFactionEnemy` → `RemoveFactionEnemy` of the same pair: final
    ///   relationship is neutral (the reverse order would leave it hostile).
    /// * `LoadWorld` → `UnloadWorld`: `PendingWorldLayerChanges` preserves
    ///   push order positionally.
    /// * `SpawnEntity` / `DestroyEntity` / `SetAiState` (warn no-op) and the
    ///   trailing `GameOver` are asserted by their individual effects.
    ///
    /// The list was one trigger's `[[trigger.action]]` array until issue #985
    /// deleted it; the delayed-action queue is the surviving ordered consumer,
    /// and `partition_delayed_actions` is what now has to preserve the order.
    /// One consequence shows in the chained witness: a delayed action's events
    /// queue onto `pending_world_events` for the NEXT tick, so the
    /// `on_flag_cleared` witness fires a tick later than it used to.
    #[test]
    fn delayed_queue_dispatches_every_action_variant_in_queue_order() {
        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .init_resource::<crate::simulation::GameOverReason>();

        // Target for the six per-entity modifier actions.
        let target = app
            .world_mut()
            .spawn((
                EntityUuid("allvar-target-uuid".to_string()),
                crate::modifiers::ShipModifiers::new(),
            ))
            .id();
        // Victim for DestroyEntity.
        let victim = app
            .world_mut()
            .spawn((
                EntityUuid("allvar-victim-uuid".to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        let all_variants = {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("target_ship".into(), "allvar-target-uuid".into());
            runtime
                .name_to_uuid
                .insert("victim".into(), "allvar-victim-uuid".into());
            let all_variants = vec![
                TriggerAction::AddObjective {
                    id: "obj-alpha".into(),
                    text: "Add then complete".into(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                },
                TriggerAction::CompleteObjective {
                    id: "obj-alpha".into(),
                },
                TriggerAction::AddObjective {
                    id: "obj-beta".into(),
                    text: "Add then fail".into(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                },
                TriggerAction::FailObjective {
                    id: "obj-beta".into(),
                },
                TriggerAction::SetAiState {
                    entity: "target_ship".into(),
                    state: "attack".into(),
                    target: None,
                },
                TriggerAction::ApplyModifier {
                    entity: "target_ship".into(),
                    tag: "boost".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                    bonus: 2.0,
                },
                TriggerAction::ApplyIntModifier {
                    entity: "target_ship".into(),
                    tag: "crew".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                    bonus: 3,
                },
                TriggerAction::ApplyFlag {
                    entity: "target_ship".into(),
                    tag: "jammer".into(),
                    kind: crate::messages::FlagKind::CommsJammed,
                },
                TriggerAction::RemoveModifier {
                    entity: "target_ship".into(),
                    tag: "boost".into(),
                    slot: crate::messages::ModifierSlot::MaxSpeed,
                },
                TriggerAction::RemoveIntModifier {
                    entity: "target_ship".into(),
                    tag: "crew".into(),
                    slot: crate::modifiers::IntModifierSlot::RepairTeams,
                },
                TriggerAction::RemoveFlag {
                    entity: "target_ship".into(),
                    tag: "jammer".into(),
                    kind: crate::messages::FlagKind::CommsJammed,
                },
                TriggerAction::SetWorldFlag {
                    name: "ordered".into(),
                },
                TriggerAction::ClearWorldFlag {
                    name: "ordered".into(),
                },
                TriggerAction::SetWorldFlagValue {
                    name: "counter".into(),
                    value: 40,
                },
                TriggerAction::IncrementWorldFlag {
                    name: "counter".into(),
                    by: 2,
                },
                TriggerAction::SpawnEntity {
                    template_path: template_path.clone(),
                    name: "allvar_spawned".into(),
                    anchor: None,
                    position: Some([5.0, 0.0, 5.0]),
                    rotation: None,
                    scale: None,
                    groups: vec![],
                    overrides: None,
                },
                TriggerAction::DestroyEntity {
                    entity: "victim".into(),
                },
                TriggerAction::AddFactionEnemy {
                    faction: "Harrow".into(),
                    enemy: "Federation".into(),
                },
                TriggerAction::RemoveFactionEnemy {
                    faction: "Harrow".into(),
                    enemy: "Federation".into(),
                },
                TriggerAction::LoadWorld {
                    path: "assets/worlds/allvar_load.toml".into(),
                },
                TriggerAction::UnloadWorld {
                    path: "assets/worlds/allvar_unload.toml".into(),
                },
                TriggerAction::GameOver {
                    message: Some("all variants dispatched".into()),
                    outcome: None,
                },
            ];
            // Chained witness: fires only if the queue emitted FlagSet("ordered")
            // followed by FlagCleared("ordered").
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnFlagCleared {
                        name: "ordered".into(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            all_variants
        };

        dispatch_delayed_actions(&mut app, None, None, all_variants);
        // Second update: the queued transitions reach the pipeline (so the
        // witness can fire) and the `GameOver` state transition lands.
        app.update();

        // Objectives: add-before-complete / add-before-fail.
        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        let snapshot_status = |id: &str| {
            objectives
                .sorted_snapshots()
                .iter()
                .find(|o| o.id == id)
                .map(|o| o.status.clone())
        };
        assert_eq!(
            snapshot_status("obj-alpha"),
            Some(ObjectiveStatus::Completed),
            "AddObjective must dispatch before CompleteObjective"
        );
        assert_eq!(
            snapshot_status("obj-beta"),
            Some(ObjectiveStatus::Failed),
            "AddObjective must dispatch before FailObjective"
        );
        assert!(
            app.world().resource::<WorldContentRuntime>().trigger_states[0].fired,
            "SetWorldFlag must dispatch before ClearWorldFlag (the chained \
             on_flag_cleared witness must observe the transition)"
        );

        // Per-entity modifiers: apply-before-remove nets out to baselines.
        let mods = app
            .world()
            .entity(target)
            .get::<crate::modifiers::ShipModifiers>()
            .expect("target must keep its ShipModifiers component");
        assert!(
            (mods.get(&crate::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
            "RemoveModifier must undo the earlier ApplyModifier"
        );
        assert_eq!(
            mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
            0,
            "RemoveIntModifier must undo the earlier ApplyIntModifier"
        );
        assert!(
            !mods.has_flag(&crate::messages::FlagKind::CommsJammed),
            "RemoveFlag must undo the earlier ApplyFlag"
        );

        // World flags: set-then-clear ends cleared; value-then-increment ends 42.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(runtime.flags.counter("ordered"), 0);
        assert_eq!(
            runtime.flags.counter("counter"),
            42,
            "SetWorldFlagValue(40) must dispatch before IncrementWorldFlag(+2)"
        );

        // SpawnEntity: registered and present in the ECS.
        let spawned_uuid = runtime
            .name_to_uuid
            .get("allvar_spawned")
            .cloned()
            .expect("SpawnEntity must register name_to_uuid");
        let mut q = app.world_mut().query::<&EntityUuid>();
        assert!(
            q.iter(app.world()).any(|eu| eu.0 == spawned_uuid),
            "SpawnEntity must spawn the templated entity"
        );

        // DestroyEntity: the victim is gone.
        assert!(
            app.world().get_entity(victim).is_err(),
            "DestroyEntity must despawn the victim"
        );

        // Factions: add-before-remove nets out to neutral.
        let registry = &app
            .world()
            .resource::<crate::config_cache::FactionRegistryResource>()
            .0;
        assert!(
            !crate::faction::is_enemy(
                Some(harrow_faction_uuid()),
                Some(fed_faction_uuid()),
                registry
            ),
            "RemoveFactionEnemy must undo the earlier AddFactionEnemy"
        );

        // Layer changes: pushed in list order.
        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(pending.0.len(), 2, "exactly one Load and one Unload");
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Load { path, .. } if path == "assets/worlds/allvar_load.toml"),
            "LoadWorld must be queued before UnloadWorld, got {:?}",
            pending.0
        );
        assert!(
            matches!(&pending.0[1], WorldLayerChange::Unload(path) if path == "assets/worlds/allvar_unload.toml"),
            "UnloadWorld must be queued after LoadWorld, got {:?}",
            pending.0
        );

        // GameOver: reason recorded, phase transitioned (second update let
        // the queued NextState take effect).
        assert_eq!(
            app.world()
                .resource::<crate::simulation::GameOverReason>()
                .0
                .as_deref(),
            Some("all variants dispatched"),
            "GameOver must record its reason"
        );
        assert_eq!(
            *app.world().resource::<State<GamePhase>>().get(),
            GamePhase::GameOver,
            "GameOver must queue the phase transition"
        );
    }

    // -- on_world_loaded (issue #415) ----------------------------------------

    /// `collect_world_events` drains `pending_world_events` into the per-tick
    /// buffer and `tick_trigger_pipeline` matches the triggers against it. Seeds
    /// a `WorldLoaded` event directly into the queue and asserts the trigger
    /// latches.
    #[test]
    fn pending_world_loaded_event_fires_on_world_loaded_trigger() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnWorldLoaded,
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.trigger_states[0].fired,
            "on_world_loaded trigger must have fired"
        );
        // Queue must be drained.
        assert!(
            runtime.pending_world_events.is_empty(),
            "pending_world_events must be drained by collect_world_events"
        );
    }

    /// (#718) Pins the follow-ups -> collect -> pipeline chain ordering:
    /// (#718) `WorldEventBuffer` is per-tick state: `collect_world_events`
    /// rebuilds it every run. An event queued for tick N flows through the
    /// buffer to the pipeline, and tick N+1 (with no new sources) must leave
    /// the buffer empty — stale events must not leak into later ticks.
    #[test]
    fn world_event_buffer_holds_only_current_tick_events() {
        let mut app = ai_trigger_test_app();

        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_world_events
            .push(WorldEvent::WorldLoaded);

        app.update();
        {
            let buffer = app.world().resource::<WorldEventBuffer>();
            assert_eq!(
                buffer.0,
                vec![WorldEvent::WorldLoaded],
                "collect_world_events must publish this tick's external events"
            );
        }

        app.update();
        let buffer = app.world().resource::<WorldEventBuffer>();
        assert!(
            buffer.0.is_empty(),
            "stale events must not leak into the next tick's buffer"
        );
    }

    /// Base-world Startup: `init_world_runtime` must push a `WorldLoaded`
    /// event onto `pending_world_events` so any `on_world_loaded` trigger the
    /// base world authored fires on the first Update tick.
    ///
    /// The `WorldConfig` used to carry the world's parsed `[[trigger]]` blocks
    /// and this asserted they reached `trigger_states`; issue #985 deleted both
    /// the field and the parser, so the table starts EMPTY for a script-free
    /// world and only `merge_script_triggers` can fill it — which is what the
    /// second assertion now pins.
    #[test]
    fn init_world_runtime_queues_world_loaded_event() {
        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<WorldResource>()
            .add_plugins(WorldPlugin);

        // Insert a WorldConfig so init_world_runtime takes its non-no-op path.
        app.insert_resource(crate::world::config::WorldConfig::default());

        app.world_mut().run_schedule(Startup);

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime
                .pending_world_events
                .iter()
                .any(|e| matches!(e, WorldEvent::WorldLoaded)),
            "init_world_runtime must queue a WorldLoaded event during Startup"
        );
        assert!(
            runtime.trigger_states.is_empty(),
            "a script-free world contributes no trigger states"
        );
    }

    /// Sub-world `LoadWorld` must push a `WorldLoaded` event so a base-world
    /// `on_world_loaded` handler can react to the layer arriving.
    #[test]
    fn apply_world_layer_changes_queues_world_loaded_event_on_load() {
        let world_path = write_entity_layer_fixture();

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldLayerMap>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime
                .pending_world_events
                .iter()
                .any(|e| matches!(e, WorldEvent::WorldLoaded)),
            "apply_world_layer_changes Load branch must queue a WorldLoaded event"
        );
    }

    /// End-to-end: load a sub-world, unload it, then re-load it. Each load cycle
    /// must emit its own `WorldLoaded`, so a base-world `on_world_loaded` handler
    /// reacts to the layer arriving BOTH times — a single event on the first load
    /// would leave a re-loaded layer silently unannounced.
    ///
    /// This used to place the `on_world_loaded` trigger INSIDE the layer and read
    /// the second fire off a freshly-created `TriggerState`. Issue #985 deleted
    /// the `[[trigger]]` parser, which was a layer's only way to author scenario
    /// logic (scripts compile on the base-world path only, until #1045), so the
    /// handler is a base-world trigger that survives both cycles — hence `repeat`,
    /// and hence `last_fired_elapsed` rather than `fired` as the "it fired again"
    /// observable.
    #[test]
    fn on_world_loaded_fires_again_after_unload_and_reload() {
        let world_path = write_entity_layer_fixture();

        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            // A fixed step makes the mission clock advance by a readable amount
            // per tick, so the two fires are distinguishable by their stamps.
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs(1),
            ))
            .add_systems(Update, apply_world_layer_changes);
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.mission_clock_anchor_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnWorldLoaded,
                    when: None,
                    id: None,
                    repeat: true,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }

        // -- First load cycle --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update(); // applies load + queues WorldLoaded
        app.update(); // collect_world_events drains pending event; pipeline fires trigger

        let first_fire = app.world().resource::<WorldContentRuntime>().trigger_states[0]
            .last_fired_elapsed
            .expect("on_world_loaded trigger must fire on first load");

        // -- Unload --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(world_path.clone()));
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<WorldContentRuntime>().trigger_states[0].last_fired_elapsed,
            Some(first_fire),
            "an unload emits no WorldLoaded, so the handler must not fire again"
        );

        // -- Second load cycle --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: world_path.clone(),
                loader_path: None,
            });
        app.update(); // applies load + queues WorldLoaded
        app.update(); // drain + dispatch

        let second_fire = app.world().resource::<WorldContentRuntime>().trigger_states[0]
            .last_fired_elapsed
            .expect("the trigger has fired at least once");
        assert!(
            second_fire > first_fire,
            "a re-loaded layer must emit its own WorldLoaded, so the handler fires \
             again at a later mission time ({second_fire} vs {first_fire})"
        );
    }

    /// Writes a tiny loadable world TOML to a temp file and returns the path.
    /// Each call uses a unique path so parallel test runs do not collide.
    ///
    /// It carried one `on_world_loaded` `[[trigger]]`; issue #985 deleted that
    /// parser, and a world that still authors the block now fails to parse
    /// outright — so the fixture is the minimum a layer can be.
    fn write_entity_layer_fixture() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let tmp = std::env::temp_dir();
        let tag = COUNTER.fetch_add(1, Ordering::Relaxed);
        let world_path = tmp.join(format!("on_world_loaded_{tag}.toml"));
        let toml = r#"
[global]
seed = 1
"#;
        std::fs::write(&world_path, toml).expect("failed to write fixture world TOML");
        world_path.to_string_lossy().into_owned()
    }

    // -- Region enter/exit triggers (issue #416) -----------------------------

    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::{spawn_entity, EntityUuid};
    use crate::region_shape::RegionShape;

    /// Build a minimal app that wires the region membership system + the issue-#416
    /// observers + `tick_trigger_pipeline` into the same world. Skips the
    /// heavyweight `WorldPlugin`/`AiPlugin`/`LobbyPlugin` bootstrap so the
    /// test focuses on the region-event ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ trigger-fire path.
    fn region_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            // The region membership system registered directly in `Update`
            // rather than via `RegionPlugin` (which schedules it on the fixed
            // logical tick since #895): this fixture drives per-update
            // semantics, the standard bare-`App` shape. Tail of the chain, so
            // a boundary crossing queues its world event for the NEXT
            // update's pipeline — the event lag this module's trigger tests
            // were written against.
            //
            // This one genuinely cannot switch to the real `RegionPlugin`
            // (re-review of issue #895): that plugin schedules
            // `update_region_membership` in `FixedUpdate`, which runs BEFORE
            // `Update` within the same `app.update()` call, so a boundary
            // crossing would be visible to `collect_world_events` on the SAME
            // frame instead of lagging by one — the exact cross-schedule
            // ordering these trigger tests assert on would collapse. Keeping
            // the system here is therefore load-bearing, not laziness, but it
            // IS a hand-copy of one `RegionPlugin::build` registration: if
            // that plugin ever changes how `update_region_membership` is
            // scheduled or gated (a new `.after()`/`.before()`, a run
            // condition, an added parameter), check whether this copy needs
            // to follow.
            .init_resource::<crate::regions::server::RegionMembership>()
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .init_resource::<WorldEventBuffer>()
            .add_message::<crate::ai::server::AiEntityAttacked>()
            .add_message::<crate::ai::server::AiEntityDestroyed>()
            .add_message::<crate::ai::server::AiWaypointReached>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (
                    collect_world_events,
                    tick_trigger_pipeline,
                    handle_comms_channel2,
                    crate::regions::server::update_region_membership,
                )
                    .chain(),
            )
            .add_observer(handle_region_entered_event)
            .add_observer(handle_region_exited_event);
        // Spawn the player ship (with a Transform so RegionPlugin's
        // membership query succeeds).
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::modifiers::ShipModifiers::new(),
        ));
        app
    }

    fn spawn_region_with_uuid(app: &mut App, x: f32, z: f32, radius: f32, uuid: &str) -> Entity {
        let config = EntityConfig {
            name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: None,
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            infrastructure: None,
            operations: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
        };
        let mut commands = app.world_mut().commands();
        spawn_entity(
            &mut commands,
            &config,
            Vec3::new(x, 0.0, z),
            uuid.to_string(),
            None,
        )
    }

    fn set_ship_pos(app: &mut App, x: f32, z: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_state::ShipPhysics, With<crate::simulation::LocalShip>>();
        if let Ok(mut p) = q.single_mut(app.world_mut()) {
            p.x = x;
            p.z = z;
        } else {
            // Fallback: the world test_app may not have a LocalShip entity yet.
            // No-op; world tests that call set_ship_pos must ensure a LocalShip entity exists.
        }
    }

    /// Register `name` → `uuid` and append a region trigger for `condition`,
    /// returning its index in `trigger_states` so the caller can read its latch
    /// through [`trigger_fired`].
    ///
    /// The trigger carried an `add_objective` the region tests read as their
    /// "it fired" signal; issue #985 deleted the `[[trigger]]` action array, so
    /// the latch is the signal and the objective id is gone with it.
    fn install_region_trigger(
        app: &mut App,
        name: &str,
        uuid: &str,
        condition: TriggerCondition,
    ) -> usize {
        install_region_trigger_gated(app, name, uuid, condition, None)
    }

    /// [`install_region_trigger`] with an optional trigger-level `when` gate.
    fn install_region_trigger_gated(
        app: &mut App,
        name: &str,
        uuid: &str,
        condition: TriggerCondition,
        when: Option<crate::world::flags::Predicate>,
    ) -> usize {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.name_to_uuid.insert(name.into(), uuid.into());
        runtime.trigger_states.push(TriggerState {
            trigger: crate::world::content::Trigger {
                condition,
                when,
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        });
        runtime.trigger_states.len() - 1
    }

    #[test]
    fn ship_entering_region_fires_on_entered_region_trigger_exactly_once() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-nebula";
        spawn_region_with_uuid(&mut app, 100.0, 0.0, 50.0, uuid);
        let idx = install_region_trigger(
            &mut app,
            "nebula",
            uuid,
            TriggerCondition::OnEnteredRegion {
                entity_name: "nebula".into(),
            },
        );

        // Tick 1: ship outside (at origin), no enter ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ no fire.
        app.update();
        assert!(
            !trigger_fired(&app, idx),
            "trigger must not fire while outside"
        );

        // Move ship inside. The membership system runs in Physics and
        // queues a WorldEvent via the observer; `tick_trigger_pipeline` (also
        // in Physics) drains the queue on the NEXT tick ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â matching the
        // documented `WorldLoaded` two-tick pattern.
        set_ship_pos(&mut app, 110.0, 0.0);
        app.update(); // queues EnteredRegion
        app.update(); // collect_world_events drains; tick_trigger_pipeline fires
        assert!(trigger_fired(&app, idx), "trigger must fire on entry");

        // Confirm single-shot: trigger is marked fired, queue is drained.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.trigger_states[0].fired,
            "trigger state must be marked fired after entry"
        );
        assert!(
            runtime.pending_world_events.is_empty(),
            "pending_world_events must be drained"
        );

        // Stay inside on subsequent ticks ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â membership system must not
        // re-emit `RegionEntered`, so no new events queue up.
        app.update();
        app.update();
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.is_empty(),
            "staying inside must not enqueue further EnteredRegion events"
        );
    }

    #[test]
    fn ship_exiting_region_fires_on_exited_region_trigger() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-nebula";
        spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);
        let idx = install_region_trigger(
            &mut app,
            "nebula",
            uuid,
            TriggerCondition::OnExitedRegion {
                entity_name: "nebula".into(),
            },
        );

        // Move inside first so we enter cleanly.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(
            !trigger_fired(&app, idx),
            "exit trigger must not fire on entry"
        );

        // Now move outside ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ RegionExited ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ queued ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ drained next tick.
        set_ship_pos(&mut app, 200.0, 0.0);
        app.update();
        app.update();
        assert!(
            trigger_fired(&app, idx),
            "exit trigger must fire when ship moves outside the region"
        );
    }

    #[test]
    fn region_despawn_while_ship_inside_fires_on_exited_region_trigger() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-fragile";
        let region_entity = spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);
        let idx = install_region_trigger(
            &mut app,
            "fragile",
            uuid,
            TriggerCondition::OnExitedRegion {
                entity_name: "fragile".into(),
            },
        );

        // Enter the region.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(!trigger_fired(&app, idx));

        // Despawn the region while ship is inside ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â membership system
        // emits an implicit RegionExited.
        app.world_mut().despawn(region_entity);
        app.update(); // queues ExitedRegion
        app.update(); // drains + fires

        assert!(
            trigger_fired(&app, idx),
            "exit trigger must fire when the region is despawned while ship is inside"
        );
    }

    #[test]
    fn overlapping_regions_fire_independent_enter_triggers() {
        let mut app = region_trigger_test_app();
        let uuid_a = "uuid-region-a";
        let uuid_b = "uuid-region-b";
        // Both regions cover the origin.
        spawn_region_with_uuid(&mut app, 0.0, 0.0, 80.0, uuid_a);
        spawn_region_with_uuid(&mut app, 20.0, 0.0, 80.0, uuid_b);
        let idx_a = install_region_trigger(
            &mut app,
            "region_a",
            uuid_a,
            TriggerCondition::OnEnteredRegion {
                entity_name: "region_a".into(),
            },
        );
        let idx_b = install_region_trigger(
            &mut app,
            "region_b",
            uuid_b,
            TriggerCondition::OnEnteredRegion {
                entity_name: "region_b".into(),
            },
        );

        // Ship at origin is inside both regions. First tick queues both
        // events, second tick drains + fires both triggers.
        set_ship_pos(&mut app, 0.0, 0.0);
        app.update();
        app.update();

        assert!(
            trigger_fired(&app, idx_a),
            "region A enter trigger must fire"
        );
        assert!(
            trigger_fired(&app, idx_b),
            "region B enter trigger must fire"
        );
    }

    #[test]
    fn npc_entering_region_does_not_fire_trigger() {
        // Region membership is tracked per-ship (PRD #597 PR 9), so an NPC ship
        // does now enter `RegionMembership.inside` when it crosses a region
        // boundary. World-scenario triggers, however, remain player-driven:
        // `handle_region_entered_event` filters on `LocalShip`, so an NPC
        // crossing does not fire an `OnEnteredRegion` trigger.
        //
        // This test uses a bare entity (no `Ship` marker) to keep the setup
        // narrow. See `npc_ship_in_damage_zone_takes_hull_damage` in
        // `src/regions/server.rs` for the equivalent test with a full NPC ship.
        let mut app = region_trigger_test_app();
        let uuid = "uuid-quarantine";
        // Region at (100, 0); player ship stays at origin (outside).
        spawn_region_with_uuid(&mut app, 100.0, 0.0, 50.0, uuid);
        let idx = install_region_trigger(
            &mut app,
            "quarantine",
            uuid,
            TriggerCondition::OnEnteredRegion {
                entity_name: "quarantine".into(),
            },
        );

        // Spawn an "NPC" entity inside the region by placing a generic
        // entity (no Ship marker) at (110, 0). The membership system
        // ignores it because the only Ship is the player ship at origin.
        let npc_config = EntityConfig {
            name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            css: None,
            light: Vec::new(),
            ship_config: None,
            shield_arcs: Vec::new(),
            tags: vec!["npc".into()],
            shape: None,
            effects: None,
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            helm_capability: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            comms_console: None,
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            infrastructure: None,
            operations: None,
            civilian: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
            lod_bubble: None,
        };
        {
            let mut commands = app.world_mut().commands();
            let _npc = spawn_entity(
                &mut commands,
                &npc_config,
                Vec3::new(110.0, 0.0, 0.0),
                "uuid-npc".into(),
                None,
            );
        }

        // Tick a few times; player ship stays at origin (outside).
        app.update();
        app.update();

        assert!(
            !trigger_fired(&app, idx),
            "trigger must remain unfired when only an NPC is inside"
        );
    }

    /// A region trigger's `when` gate behaves like every other trigger's: a
    /// false predicate suppresses the entry without consuming the trigger, so a
    /// later entry with the flag set still fires it.
    ///
    /// The gate was authored as `when: None` here, which meant the first half of
    /// the test could only ever pass on the region pipeline's one-tick event lag.
    /// Both halves are real now: each crossing is stepped twice (queue, then
    /// drain), so the suppressed entry is genuinely evaluated against the unset
    /// flag.
    #[test]
    fn on_entered_region_trigger_with_when_filter_obeys_predicate() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-zone";
        spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);

        let idx = install_region_trigger_gated(
            &mut app,
            "zone",
            uuid,
            TriggerCondition::OnEnteredRegion {
                entity_name: "zone".into(),
            },
            Some(crate::world::flags::parse_predicate("flag(armed)").unwrap()),
        );

        // First entry: the crossing queues an EnteredRegion, the next tick
        // drains it into the pipeline — where the unset flag suppresses it.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update(); // queues EnteredRegion
        app.update(); // drains it; the predicate reads false
        assert!(
            !trigger_fired(&app, idx),
            "predicate-false firings must NOT consume the trigger"
        );

        // Set the flag, leave the region, re-enter — the trigger fires now.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("armed");
        }
        set_ship_pos(&mut app, 200.0, 0.0); // exit
        app.update();
        app.update();
        set_ship_pos(&mut app, 10.0, 0.0); // re-enter
        app.update();
        app.update();

        assert!(
            trigger_fired(&app, idx),
            "gated trigger must fire once the flag is set and ship re-enters"
        );
    }

    // -- Issue #417: spawn_entity / destroy_entity trigger actions ---------

    /// Writes a minimal NPC template to a temp file and returns its path.
    pub(crate) fn write_spawn_template_fixture() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static C: AtomicU32 = AtomicU32::new(0);
        let tag = C.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("spawn_action_template_{tag}.toml"));
        std::fs::write(
            &p,
            r##"
tags = ["npc"]

[appearance]
colour = "#ff8800"
size_min = 1.0
size_max = 2.0
"##,
        )
        .expect("write template");
        p.to_string_lossy().into_owned()
    }

    /// SpawnEntity action with an explicit `position` spawns a new entity into
    /// the ECS at those coordinates and registers it in `name_to_uuid`.
    ///
    /// This section's actions were authored on `[[trigger.action]]` arrays until
    /// issue #985 deleted them; they are dispatched from the delayed-action
    /// queue, which reaches the same `world::dispatch` table.
    #[test]
    fn spawn_entity_action_with_position_spawns_and_registers_uuid() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::SpawnEntity {
                template_path: template_path.clone(),
                name: "spawned_one".to_string(),
                anchor: None,
                position: Some([7.0, 0.0, 3.0]),
                rotation: None,
                scale: None,
                groups: vec![],
                overrides: None,
            }],
        );
        app.update(); // flush the queued spawn Commands

        // name_to_uuid must contain the new entity.
        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("spawned_one")
            .cloned();
        assert!(uuid.is_some(), "SpawnEntity must register name_to_uuid");

        // The ECS must contain an entity with that UUID.
        let uuid_value = uuid.unwrap();
        let mut found = false;
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid_value {
                found = true;
                assert!((t.translation.x - 7.0).abs() < 1e-3);
                assert!((t.translation.z - 3.0).abs() < 1e-3);
            }
        }
        assert!(found, "spawned entity with the new UUID must exist in ECS");
    }

    /// SpawnEntity action with an `anchor` resolves the anchor against the
    /// base-world `WorldConfig` resource.
    #[test]
    fn spawn_entity_action_with_anchor_resolves_against_base_world() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        // Insert a WorldConfig with a known anchor.
        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("alpha".to_string(), [42.0, 0.0, -5.0]);
        app.world_mut().insert_resource(wc);

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::SpawnEntity {
                template_path: template_path.clone(),
                name: "anchor_spawn".to_string(),
                anchor: Some("alpha".to_string()),
                position: None,
                rotation: None,
                scale: None,
                groups: vec![],
                overrides: None,
            }],
        );
        app.update(); // flush the queued spawn Commands

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("anchor_spawn")
            .cloned()
            .expect("anchor spawn must register name_to_uuid");

        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        let mut found = false;
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid {
                found = true;
                assert!((t.translation.x - 42.0).abs() < 1e-3);
                assert!((t.translation.z - -5.0).abs() < 1e-3);
            }
        }
        assert!(found, "anchor-resolved spawn must exist at anchor coords");
    }

    /// SpawnEntity action with an `anchor` resolves against the originating
    /// sub-world layer's anchor table (not the base world).
    #[test]
    fn spawn_entity_action_resolves_anchor_against_origin_layer() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();

        // Register a fake layer with a custom anchor.
        let layer_path = "fake_layer_path.toml".to_string();
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            let mut wr = WorldRuntime::default();
            wr.anchors
                .insert("docking_bay".to_string(), [11.0, 0.0, 22.0]);
            lm.0.insert(layer_path.clone(), wr);
        }

        // Also seed a different anchor with the same name in the base world to
        // prove the layer-local one wins.
        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors
            .insert("docking_bay".to_string(), [-99.0, 0.0, -99.0]);
        app.world_mut().insert_resource(wc);

        dispatch_delayed_actions(
            &mut app,
            Some(&layer_path),
            None,
            vec![TriggerAction::SpawnEntity {
                template_path: template_path.clone(),
                name: "layer_spawn".to_string(),
                anchor: Some("docking_bay".to_string()),
                position: None,
                rotation: None,
                scale: None,
                groups: vec![],
                overrides: None,
            }],
        );
        app.update(); // flush the queued spawn Commands

        // The layer's spawned_entities list must now have the new entity, and
        // that entity must be at the layer-local anchor.
        let layer = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get(&layer_path)
            .expect("layer present")
            .clone();
        assert_eq!(
            layer.spawned_entities.len(),
            1,
            "layer-originated SpawnEntity must attach to the parent layer for cascade unload"
        );

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("layer_spawn")
            .cloned()
            .expect("layer spawn must register name_to_uuid");
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        let mut found = false;
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid {
                found = true;
                assert!(
                    (t.translation.x - 11.0).abs() < 1e-3,
                    "must use layer anchor (11), not base anchor (-99); got {}",
                    t.translation.x
                );
                assert!((t.translation.z - 22.0).abs() < 1e-3);
            }
        }
        assert!(found);
    }

    /// DestroyEntity action despawns the target entity and emits a `Destroyed`
    /// world event that a downstream `on_destroyed` trigger chains on.
    ///
    /// The chain is a tick longer than it used to be: the action rode on a
    /// trigger's own array before issue #985 deleted it, and a delayed action's
    /// events queue onto `pending_world_events` for the NEXT tick rather than
    /// into the current pass.
    #[test]
    fn destroy_entity_action_despawns_and_emits_chained_event() {
        use crate::entities::spawner::EntityUuid;

        let mut app = ai_trigger_test_app();

        // Spawn a target entity with a known UUID.
        let target_uuid = "doomed-uuid";
        let target_entity = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid.into()),
                bevy::prelude::Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("doomed".to_string(), target_uuid.into());
            // Witness: on destroyed of "doomed" — proves the chaining.
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "doomed".into(),
                    },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::DestroyEntity {
                entity: "doomed".into(),
            }],
        );
        app.update(); // the queued Destroyed event reaches the pipeline

        // Target entity must be gone.
        assert!(
            app.world().get_entity(target_entity).is_err(),
            "DestroyEntity must despawn the target entity"
        );

        // Chained on_destroyed trigger must have fired.
        assert!(
            trigger_fired(&app, 0),
            "chained on_destroyed trigger must fire from the DestroyEntity action"
        );

        // External consumers must also see the message: DestroyEntity action
        // must emit AiEntityDestroyed via the deferred Commands::queue path,
        // matching combat-induced destruction.
        let msgs = app
            .world()
            .resource::<Messages<crate::ai_plugin::AiEntityDestroyed>>();
        let mut cursor = msgs.get_cursor();
        let emitted: Vec<String> = cursor.read(msgs).map(|m| m.entity_uuid.clone()).collect();
        assert!(
            emitted.iter().any(|u| u == target_uuid),
            "DestroyEntity action must emit AiEntityDestroyed for '{target_uuid}', got {emitted:?}"
        );
    }

    /// DestroyEntity with an unknown entity name is a warned no-op: it must not
    /// panic, and it must emit no `Destroyed` event for a chained trigger to
    /// react to.
    #[test]
    fn destroy_entity_action_with_unknown_name_is_noop() {
        let mut app = ai_trigger_test_app();
        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::DestroyEntity {
                entity: "no_such_entity".into(),
            }],
        );
        assert!(
            app.world()
                .resource::<WorldContentRuntime>()
                .pending_world_events
                .is_empty(),
            "an unresolvable DestroyEntity must emit nothing at all"
        );
    }

    /// UnloadWorld cascades through entities spawned by a layer-origin
    /// SpawnEntity action (not just those spawned at LoadWorld time).
    #[test]
    fn unload_world_cascades_through_spawn_entity_action_results() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();

        let layer_path = "cascade_layer.toml".to_string();
        {
            let mut lm = app.world_mut().resource_mut::<WorldLayerMap>();
            lm.0.insert(layer_path.clone(), WorldRuntime::default());
        }

        dispatch_delayed_actions(
            &mut app,
            Some(&layer_path),
            None,
            vec![TriggerAction::SpawnEntity {
                template_path: template_path.clone(),
                name: "cascade_me".into(),
                anchor: None,
                position: Some([1.0, 0.0, 1.0]),
                rotation: None,
                scale: None,
                groups: vec![],
                overrides: None,
            }],
        );
        app.update(); // flush the queued spawn Commands

        let spawned: Vec<bevy::prelude::Entity> = app
            .world()
            .resource::<WorldLayerMap>()
            .0
            .get(&layer_path)
            .expect("layer present")
            .spawned_entities
            .clone();
        assert_eq!(spawned.len(), 1, "spawn must attach to the layer entry");

        // Now register unload handler + drive an UnloadWorld through it.
        app.add_systems(Update, apply_world_layer_changes);
        app.init_resource::<PendingWorldLayerChanges>();
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(layer_path.clone()));
        app.update();
        app.update();

        for e in spawned {
            assert!(
                app.world().get_entity(e).is_err(),
                "entity from SpawnEntity action must be despawned by UnloadWorld"
            );
        }
        let _ = std::any::type_name::<EntityUuid>(); // touch import
    }

    /// A trigger-level `when` gate suppresses the whole firing, so the scripted
    /// handler hanging off it never runs and its `spawn_entity` never happens.
    ///
    /// The gate used to be tested against a `[[trigger.action]]` spawn. Issue
    /// #985 deleted that array, so the spawn lives in a scripted handler — which
    /// is the arrangement the gate has to protect now, and the interesting one:
    /// the effect is skipped because the trigger never fired at all, not because
    /// anything inside the handler checked.
    #[test]
    fn when_predicate_suppresses_a_scripted_spawn() {
        let template_path = write_spawn_template_fixture();
        let mut sr = compile_fixture_scripts(&format!(
            r#"[script]
setup = 'on_attacked("src", "spawn_it").when("flag(ready)"); fn spawn_it(ctx) {{ ctx.effects.spawn_entity(#{{ template_path: "{template_path}", name: "blocked", position: [0, 0, 0] }}); }}'
"#,
            template_path = template_path.replace('\\', "/")
        ));
        assert!(
            sr.triggers[0].trigger.when.is_some(),
            "the scripted `.when(..)` must reach the compiled trigger"
        );

        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = Vec::new();
            merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("20202020-0000-0000-0000-000000000001")
                    .unwrap(),
            });
        app.update();
        app.update();

        // Flag was NOT set, so the trigger never fired and nothing spawned.
        assert!(
            !trigger_fired(&app, 0),
            "a false `when` must suppress the firing without consuming the trigger"
        );
        assert!(
            !app.world()
                .resource::<WorldContentRuntime>()
                .name_to_uuid
                .contains_key("blocked"),
            "the handler's spawn_entity must not run while `when` reads false"
        );
    }

    /// SpawnEntity action applies optional `rotation` (XYZ Euler radians) and
    /// `scale` (per-axis) to the spawned entity's Transform, mirroring the
    /// static `[[entity]]` TransformConfig semantics.
    #[test]
    fn spawn_entity_action_applies_rotation_and_scale() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::SpawnEntity {
                template_path: template_path.clone(),
                name: "rotated_scaled".to_string(),
                anchor: None,
                position: Some([1.0, 2.0, 3.0]),
                rotation: Some([0.0, std::f32::consts::FRAC_PI_2, 0.0]),
                scale: Some([2.0, 2.0, 2.0]),
                groups: vec![],
                overrides: None,
            }],
        );
        app.update(); // flush the queued spawn Commands

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("rotated_scaled")
            .cloned()
            .expect("SpawnEntity must register name_to_uuid");

        let expected_quat = Quat::from_euler(EulerRot::XYZ, 0.0, std::f32::consts::FRAC_PI_2, 0.0);
        let expected_scale = Vec3::new(2.0, 2.0, 2.0);
        let expected_translation = Vec3::new(1.0, 2.0, 3.0);

        let mut found = false;
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &bevy::prelude::Transform)>();
        for (eu, t) in q.iter(app.world()) {
            if eu.0 == uuid {
                found = true;
                assert!(
                    t.translation.abs_diff_eq(expected_translation, 1e-4),
                    "translation mismatch: got {:?}, expected {:?}",
                    t.translation,
                    expected_translation
                );
                assert!(
                    t.rotation.abs_diff_eq(expected_quat, 1e-4),
                    "rotation mismatch: got {:?}, expected {:?}",
                    t.rotation,
                    expected_quat
                );
                assert!(
                    t.scale.abs_diff_eq(expected_scale, 1e-4),
                    "scale mismatch: got {:?}, expected {:?}",
                    t.scale,
                    expected_scale
                );
            }
        }
        assert!(
            found,
            "spawned entity must exist in ECS with the registered UUID"
        );
    }

    /// (#960) The mission clock stamps ONCE per mission, off the clock as it
    /// reads when the mission starts - and `arm_mission_clock` is what lets a
    /// second round start its own clock instead of inheriting the first's.
    ///
    /// The lobby half of the same fix is flown end-to-end by
    /// `combat_test_wave_clock_measures_from_mission_start_not_app_boot`
    /// (`tests/headless_runner.rs`), which holds a real app out of `InProgress`
    /// for 90 s and requires the authored schedule to survive it. This one
    /// covers what that run cannot reach: the no-world case, the idempotence
    /// that keeps a running mission's clock from creeping forward every tick,
    /// and the re-arm a `ReturnToLobby` round depends on.
    #[test]
    fn mission_clock_anchors_once_per_mission_and_rearms_for_the_next() {
        use bevy::ecs::system::RunSystemOnce;

        let step = std::time::Duration::from_secs(10);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(step))
            .init_resource::<WorldContentRuntime>()
            .add_systems(Update, anchor_mission_clock);

        let anchor = |app: &App| {
            app.world()
                .resource::<WorldContentRuntime>()
                .mission_clock_anchor_secs
        };

        // No world loaded: nothing anchors, so `collect_world_events` goes on
        // emitting no `TimerElapsed` at all in a fixture that never asked for a
        // scenario.
        app.update();
        assert_eq!(
            anchor(&app),
            None,
            "an app with no `WorldConfig` must not anchor a clock it has no \
             triggers to measure"
        );

        app.insert_resource(crate::world::config::WorldConfig::default());
        app.update();
        let first = anchor(&app).expect("a world is loaded, so the mission clock anchors");

        // Idempotent: a running mission's time zero must not creep forward with
        // every tick, or `after_secs` would never be reached.
        app.update();
        assert_eq!(
            anchor(&app),
            Some(first),
            "the anchor must be stamped once per mission, not re-stamped every \
             tick - a clock that follows `now` never elapses anything"
        );

        // Round two. `OnEnter(GamePhase::InProgress)` runs this on every start,
        // including the one a `ReturnToLobby` leads back to.
        app.world_mut()
            .run_system_once(arm_mission_clock)
            .expect("arm_mission_clock should run");
        app.update();
        let second = anchor(&app).expect("the second mission anchors too");
        assert!(
            second > first,
            "round two must measure from ITS own start ({second}), not inherit \
             round one's ({first}) - inheriting means every trigger the first \
             round outlived has already expired when the second begins"
        );
    }

    /// (#475) `on_timer` triggers fire when `time.elapsed_secs() -
    /// runtime.mission_clock_anchor_secs >= after_secs`. Pins the producer in
    /// `tick_trigger_pipeline`: it must emit `TimerElapsed` events against the
    /// mission-clock anchor, or an `after_secs = 0` trigger would never fire.
    ///
    /// The fire used to be read off the trigger's `spawn_entity` action; issue
    /// #985 deleted the `[[trigger]]` action array, so the latch and its
    /// mission-clock stamp are the observable.
    #[test]
    fn on_timer_trigger_fires() {
        let mut app = ai_trigger_test_app();

        // Stamp world load time to `now` and install an on_timer trigger
        // with `after_secs = 0.0` so it should fire on the first tick.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.mission_clock_anchor_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 0.0 },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }

        app.update();

        let state = &app.world().resource::<WorldContentRuntime>().trigger_states[0];
        assert!(
            state.fired,
            "on_timer after_secs=0 must fire — tick_trigger_pipeline must emit \
             TimerElapsed events when mission_clock_anchor_secs is set"
        );
        assert!(
            state.last_fired_elapsed.is_some(),
            "the fire must be stamped against the mission clock"
        );
    }

    /// (#475) `on_timer` triggers with `after_secs > now - world_loaded_at`
    /// must NOT fire yet. Pin the elapsed-secs comparison.
    #[test]
    fn on_timer_trigger_does_not_fire_before_after_secs_elapses() {
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.mission_clock_anchor_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 100.0 },
                    when: None,
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            }];
        }

        app.update();
        app.update();

        let state = &app.world().resource::<WorldContentRuntime>().trigger_states[0];
        assert!(
            !state.fired,
            "on_timer after_secs=100 must not fire when only a few ms have elapsed"
        );
        assert!(state.last_fired_elapsed.is_none());
    }

    /// SpawnEntity action stamps its own `name` onto the spawned entity as the
    /// `EntityName` component. This is required for `resolve_objective_target`
    /// to match an `AiDirective::Destroy { target: "wave_1" }` against
    /// `AiWorldEntity::name` in `WorldSnapshot` — if the component kept the
    /// template display name ("Harrow Destroyer") the Backfill AI would never
    /// resolve the Destroy target and the ship would stay on its Patrol forever.
    ///
    /// The name travels on the action itself; the *resolving* context name (the
    /// one an `add_objective` links its target by) is the `DelayedAction`'s own
    /// `entity_name` field since issue #985, rather than being re-derived from
    /// the condition of the trigger that dispatched it.
    #[test]
    fn spawn_entity_action_stamps_its_name_as_entity_name() {
        use crate::entities::spawner::{EntityName, EntityUuid};

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        dispatch_delayed_actions(
            &mut app,
            None,
            None,
            vec![TriggerAction::SpawnEntity {
                template_path: template_path.clone(),
                name: "wave_1".to_string(),
                anchor: None,
                position: Some([50.0, 0.0, 50.0]),
                rotation: None,
                scale: None,
                groups: vec![],
                overrides: None,
            }],
        );
        app.update(); // flush the queued spawn Commands

        // Find the spawned entity by UUID and confirm its EntityName is "wave_1".
        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("wave_1")
            .cloned()
            .expect("SpawnEntity must register name_to_uuid for wave_1");

        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, Option<&EntityName>)>();
        let entity_name = q
            .iter(app.world())
            .find_map(|(eu, name)| (eu.0 == uuid).then(|| name.map(|n| n.0.clone())));

        assert!(
            entity_name.is_some(),
            "spawned entity with UUID for 'wave_1' must exist in ECS"
        );
        assert_eq!(
            entity_name.unwrap().as_deref(),
            Some("wave_1"),
            "EntityName must be the action's name 'wave_1', not the template display name"
        );
    }

    // ── Issue #629: ship_power counter seeding and predicate-gated spawns ─────

    /// `seed_ship_power_counter` logic: verify that writing `power_rating` into
    /// `FlagStore.set_flag_value("ship_power", rating)` produces the correct
    /// counter value (tests the seeding path in isolation).
    #[test]
    fn seed_ship_power_counter_writes_to_flags() {
        use crate::world::flags::FlagStore;

        let mut flags = FlagStore::new();
        // Simulate the body of seed_ship_power_counter for power_rating = Some(300).
        let rating: i32 = 300;
        flags.set_flag_value("ship_power", rating as i64);

        assert_eq!(
            flags.counter("ship_power"),
            300,
            "ship_power counter must equal power_rating"
        );
    }

    /// When `power_rating` is `None` the seeding system writes nothing, so
    /// `ship_power` defaults to 0.
    #[test]
    fn seed_ship_power_counter_absent_when_power_rating_is_none() {
        use crate::world::flags::FlagStore;

        let flags = FlagStore::new();
        // Simulate seed_ship_power_counter with power_rating = None — nothing written.

        assert_eq!(
            flags.counter("ship_power"),
            0,
            "ship_power must remain 0 when power_rating is None"
        );
    }

    // Helper: build a minimal ConfigCache with a single blank template at `path`.
    fn single_template_cache(path: &str) -> crate::config_cache::ConfigCache {
        use crate::entity_config::EntityConfig;
        use std::collections::HashMap;
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(path.into(), EntityConfig::from_toml("").unwrap());
        crate::config_cache::ConfigCache::from(m)
    }

    // Helper: build a minimal WorldConfig with one named GameStart entity.
    fn gamestart_world_cfg_with_predicate(
        when_predicate: Option<crate::world::flags::Predicate>,
    ) -> crate::world::config::WorldConfig {
        use crate::world::config::{WorldConfig, WorldEntity, WorldEntitySpawnOn};
        let mut cfg = WorldConfig::default();
        cfg.entities.push(WorldEntity {
            template_path: "fixture/frigate.toml".into(),
            name: Some("gated_ship".into()),
            // Use Immediate so spawn_immediate_entities_internal picks it up
            // (the predicate evaluation code path is shared with GameStart via
            // the same `when_predicate` field; see spawn_game_start_entities).
            spawn_on: WorldEntitySpawnOn::Immediate,
            when_predicate,
            ..Default::default()
        });
        cfg.name_to_uuid
            .insert("gated_ship".into(), "gated-ship-uuid".into());
        cfg
    }

    /// A `GameStart` entity with `counter(ship_power) >= 200` spawns when the
    /// flag store has `ship_power = 200`.
    #[test]
    fn spawn_game_start_entity_gated_on_ship_power_spawns_when_met() {
        use crate::entity_spawner::EntityUuid;
        use crate::world::flags::{CmpOp, FlagStore, Predicate};

        let pred = Predicate::Counter {
            name: "ship_power".into(),
            op: CmpOp::Ge,
            rhs: 200,
        };
        let world_cfg = gamestart_world_cfg_with_predicate(Some(pred));
        let cache = single_template_cache("fixture/frigate.toml");

        let mut flags = FlagStore::new();
        flags.set_flag_value("ship_power", 200);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, Some(&flags), None)
        };
        app.update();

        assert_eq!(
            spawned.len(),
            1,
            "entity must spawn when predicate is satisfied"
        );
        let uuid = app.world().get::<EntityUuid>(spawned[0]).unwrap();
        assert_eq!(uuid.0, "gated-ship-uuid");
    }

    /// A `GameStart` entity with `counter(ship_power) >= 200` is skipped when
    /// `ship_power = 150` (predicate not met).
    #[test]
    fn spawn_game_start_entity_gated_on_ship_power_skips_when_not_met() {
        use crate::world::flags::{CmpOp, FlagStore, Predicate};

        let pred = Predicate::Counter {
            name: "ship_power".into(),
            op: CmpOp::Ge,
            rhs: 200,
        };
        let world_cfg = gamestart_world_cfg_with_predicate(Some(pred));
        let cache = single_template_cache("fixture/frigate.toml");

        let mut flags = FlagStore::new();
        flags.set_flag_value("ship_power", 150);

        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);

        let spawned: Vec<Entity> = {
            let mut commands = app.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &world_cfg, &cache, Some(&flags), None)
        };
        app.update();

        assert_eq!(
            spawned.len(),
            0,
            "entity must be skipped when predicate is not met"
        );
    }

    /// Complementary pair: exactly one of two mutually-exclusive entities
    /// spawns depending on `ship_power`.
    #[test]
    fn spawn_game_start_entity_complementary_pair() {
        use crate::world::config::{WorldConfig, WorldEntity, WorldEntitySpawnOn};
        use crate::world::flags::{CmpOp, FlagStore, Predicate};

        let mut cfg = WorldConfig::default();
        // Heavy variant: ship_power >= 200
        cfg.entities.push(WorldEntity {
            template_path: "fixture/heavy.toml".into(),
            name: Some("heavy".into()),
            spawn_on: WorldEntitySpawnOn::Immediate,
            when_predicate: Some(Predicate::Counter {
                name: "ship_power".into(),
                op: CmpOp::Ge,
                rhs: 200,
            }),
            ..Default::default()
        });
        // Scout variant: ship_power < 200
        cfg.entities.push(WorldEntity {
            template_path: "fixture/scout.toml".into(),
            name: Some("scout".into()),
            spawn_on: WorldEntitySpawnOn::Immediate,
            when_predicate: Some(Predicate::Counter {
                name: "ship_power".into(),
                op: CmpOp::Lt,
                rhs: 200,
            }),
            ..Default::default()
        });
        cfg.name_to_uuid.insert("heavy".into(), "heavy-uuid".into());
        cfg.name_to_uuid.insert("scout".into(), "scout-uuid".into());

        use crate::entity_config::EntityConfig;
        use std::collections::HashMap;
        let mut m: HashMap<String, EntityConfig> = HashMap::new();
        m.insert(
            "fixture/heavy.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        m.insert(
            "fixture/scout.toml".into(),
            EntityConfig::from_toml("").unwrap(),
        );
        let cache = crate::config_cache::ConfigCache::from(m);

        // Low power: only scout spawns.
        let mut flags_low = FlagStore::new();
        flags_low.set_flag_value("ship_power", 150);

        let mut app_low = App::new();
        app_low.add_plugins(bevy::time::TimePlugin);
        let spawned_low: Vec<Entity> = {
            let mut commands = app_low.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &cfg, &cache, Some(&flags_low), None)
        };
        app_low.update();
        assert_eq!(spawned_low.len(), 1, "only scout should spawn at low power");
        let uuid_low = app_low
            .world()
            .get::<crate::entity_spawner::EntityUuid>(spawned_low[0])
            .unwrap();
        assert_eq!(uuid_low.0, "scout-uuid");

        // High power: only heavy spawns.
        let mut flags_high = FlagStore::new();
        flags_high.set_flag_value("ship_power", 250);

        let mut app_high = App::new();
        app_high.add_plugins(bevy::time::TimePlugin);
        let spawned_high: Vec<Entity> = {
            let mut commands = app_high.world_mut().commands();
            spawn_immediate_entities_internal(&mut commands, &cfg, &cache, Some(&flags_high), None)
        };
        app_high.update();
        assert_eq!(
            spawned_high.len(),
            1,
            "only heavy should spawn at high power"
        );
        let uuid_high = app_high
            .world()
            .get::<crate::entity_spawner::EntityUuid>(spawned_high[0])
            .unwrap();
        assert_eq!(uuid_high.0, "heavy-uuid");
    }

    /// Composed predicate: `counter(ship_power) >= 200 or flag(always_spawn)`
    /// — entity spawns when either condition is true.
    #[test]
    fn spawn_game_start_entity_composed_predicate() {
        use crate::entity_spawner::EntityUuid;
        use crate::world::flags::{CmpOp, FlagStore, Predicate};

        let pred = Predicate::Or(
            Box::new(Predicate::Counter {
                name: "ship_power".into(),
                op: CmpOp::Ge,
                rhs: 200,
            }),
            Box::new(Predicate::Flag {
                name: "always_spawn".into(),
            }),
        );
        let world_cfg = gamestart_world_cfg_with_predicate(Some(pred));
        let cache = single_template_cache("fixture/frigate.toml");

        // Case A: low power, flag not set → skipped.
        let flags_neither = FlagStore::new();
        let mut app_a = App::new();
        app_a.add_plugins(bevy::time::TimePlugin);
        let spawned_a: Vec<Entity> = {
            let mut commands = app_a.world_mut().commands();
            spawn_immediate_entities_internal(
                &mut commands,
                &world_cfg,
                &cache,
                Some(&flags_neither),
                None,
            )
        };
        app_a.update();
        assert_eq!(spawned_a.len(), 0, "neither condition met → skip");

        // Case B: flag set, low power → spawns.
        let mut flags_flag = FlagStore::new();
        flags_flag.set_flag("always_spawn");
        let mut app_b = App::new();
        app_b.add_plugins(bevy::time::TimePlugin);
        let spawned_b: Vec<Entity> = {
            let mut commands = app_b.world_mut().commands();
            spawn_immediate_entities_internal(
                &mut commands,
                &world_cfg,
                &cache,
                Some(&flags_flag),
                None,
            )
        };
        app_b.update();
        assert_eq!(spawned_b.len(), 1, "flag set → spawn even with low power");
        let uuid_b = app_b.world().get::<EntityUuid>(spawned_b[0]).unwrap();
        assert_eq!(uuid_b.0, "gated-ship-uuid");

        // Case C: high power, flag not set → spawns.
        let mut flags_power = FlagStore::new();
        flags_power.set_flag_value("ship_power", 300);
        let mut app_c = App::new();
        app_c.add_plugins(bevy::time::TimePlugin);
        let spawned_c: Vec<Entity> = {
            let mut commands = app_c.world_mut().commands();
            spawn_immediate_entities_internal(
                &mut commands,
                &world_cfg,
                &cache,
                Some(&flags_power),
                None,
            )
        };
        app_c.update();
        assert_eq!(spawned_c.len(), 1, "high power → spawn even without flag");
    }
}
