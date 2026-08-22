use bevy::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use rhai::{Map, AST};

use crate::core::messages::{GamePhase, ServerMessage};
use crate::effect_queue::EffectQueue;
use crate::lobby::{Target, WorldResource};
use crate::objectives::ObjectiveManager;
use crate::server_app::SimOutbox;
#[cfg(test)]
use crate::world::content::TriggerAction;
use crate::world::content::{TriggerState, WorldEvent};
use crate::world::delayed::{partition_delayed_actions, DelayedAction};
use crate::world::dispatch::{
    dispatch_action, ActionCmd, DispatchContext, DispatchResult, LayerView,
    WORLD_MODIFIER_SOURCE_ID,
};
use crate::world::layers::{evaluate_layer_load, LayerLoadOutcome};
use crate::world::load::{load, LoadError, LoadPolicy, LoadRequest, MemoryReader};
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
    /// effects can apply its deadline mutations too — the same shape
    /// `WorldScriptRuntime::pending_callbacks` has.
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
    /// effects can apply its commitment mutations too.
    pub commitments: crate::world::commitments::CommitmentLedger,
    /// What this run's crew have found out (issue #1031): every
    /// `ctx.dossier.append(…)` a scan handler or a dialogue `on_pick` wrote,
    /// with the provenance that says how they learned it.
    ///
    /// Append-only, deduplicated on `(subject, provenance, text)`, and read back
    /// by the dossier projection and nothing else — read
    /// [`crate::dossier::evidence`] before adding anything to it. Like the ledger
    /// above it there is no queue and no evaluator: an entry is written when a
    /// script says the crew learned something, and nothing scans for findings
    /// that have quietly become available.
    ///
    /// It is the ONE input to a dossier that is not recomputed every tick, which
    /// is why it sits here rather than in `src/dossier/`: state belongs beside
    /// the other scenario state already on this resource, so every site that
    /// borrows the content runtime to apply a call's effects can append to it.
    pub evidence: crate::dossier::evidence::EvidenceLog,
    /// The sides of this world's labour dispute (issue #1035): whether each is
    /// out, and what each makes of the crew.
    ///
    /// A record with no evaluator and no queue — read
    /// [`crate::world::workforce`] before adding anything to it. Armed once by
    /// [`arm_mission_workforces`] from the world's `[[workforce]]` blocks, on
    /// the same tick the deadline table is armed and for the same reason;
    /// after that it moves only when a script says so, and its state is
    /// mirrored to a world flag that triggers read. Nothing scans it per tick.
    ///
    /// It sits on this resource beside `deadlines` and `commitments`, and for
    /// their reason: every site that already borrows the content runtime to
    /// apply a call's effects can apply a settlement too.
    pub workforce: crate::world::workforce::WorkforceRegister,
    // The four transient per-tick effect queues that used to sit here —
    // `pending_condition_adjustments`, `pending_capacity_adjustments`,
    // `pending_civilian_orders` and `pending_weapons_holds` — were extracted to
    // their own per-owner `crate::effect_queue::EffectQueue<T>` resources (issue
    // #1223), registered and drained by the plugin that owns each edge
    // (Infrastructure / Civilian / captain). They lived here partly so the
    // authoritative-state census "saw no new registration"; #1220–#1222 gave the
    // census a real declaration registry, so each queue is now declared
    // `ClearedAtFold` at its owning `build()` instead. `pending_world_events` and
    // `pending_delayed_actions` stay: unlike those four they are NOT empty at a
    // tick boundary (they are snapshotted / carried across ticks), so they are
    // deferred state rather than a transient inter-system queue.
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
/// so this carries the whole TOML alongside its path. Since issue #1214 it is the
/// **browser's** route only: `insert_raw_world_source_resource` reads the
/// `server::bridge::BridgeWorldSource` Resource the wasm bridge inserts at
/// `wasm_init` (issue #1181) at `Startup` and inserts it, and
/// `compile_world_scripts` reads it once. Headless no longer inserts it — it
/// compiles the world's scripts once in `world::load::load` and hands the result
/// to `compile_world_scripts` as [`PreCompiledScripts`], so a headless run does
/// not parse this raw value or compile a second time.
///
/// "Raw" is about SHAPE, not provenance: unparsed TOML with nothing dropped,
/// which is not the same as untouched. On a harnessed duel run
/// (`--side-a`/`--side-b`) the duel transform runs as `world::load`'s
/// `raw_transform` hook, regenerating the slot roster inside the `[script]`
/// source before it is compiled — so the compiled set headless hands over already
/// reflects the roster, exactly as the value that used to land here did.
#[derive(Resource, Clone, Debug)]
pub struct RawWorldSource {
    /// The world TOML's path (its content-ledger / snapshot-boundary key).
    pub path: String,
    /// The world TOML as loaded — after any headless duel-side transform — still
    /// carrying any `[script]` / `script` key.
    pub toml: toml::Value,
}

/// A world's scripts, compiled ONCE at build time and handed to
/// [`compile_world_scripts`] for the runtime insertion (issue #1214, Track 2 A2).
///
/// The headless path (`build_headless_app`) runs the world through
/// `world::load::load` a single time — the same pass that feeds the build-time
/// fail-fast gate — and inserts the compiled result here. [`compile_world_scripts`]
/// then consumes it (`Option<ResMut>` + `.take()`) instead of re-reading
/// [`RawWorldSource`] and compiling a second time, so a headless run parses and
/// compiles a world's scripts only once. The browser still arrives via
/// [`RawWorldSource`] (populated by [`insert_raw_world_source_resource`] on wasm);
/// both targets converge on the identical `WorldScriptRuntime` construction. The
/// inner value is `None` for a script-free world (`world::load` returns no
/// scripts), which short-circuits exactly as the absent-`script`-key arm of the
/// `RawWorldSource` path does.
#[derive(Resource)]
pub struct PreCompiledScripts(pub Option<crate::world::script::load::CompiledScripts>);

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

impl WorldScriptRuntime {
    /// Build the live runtime from a compiled script set, or `None` when the set
    /// has no runnable AST — a script-free world, or an empty `[script]` table.
    ///
    /// The one place the `CompiledScripts → WorldScriptRuntime` construction
    /// lives: [`compile_world_scripts`] (both the pre-compiled headless path and
    /// the browser's `RawWorldSource` compile) and every `#[cfg(test)]` fixture
    /// compiler (`world::script::fixture`, and through it the `comms::scripted`
    /// dialogue fixtures) route through here rather than hand-rolling the literal.
    /// Returning `None` for an empty set lets a caller insert no resource, leaving
    /// behaviour identical to a world that never authored a script.
    pub fn from_compiled(compiled: crate::world::script::load::CompiledScripts) -> Option<Self> {
        if compiled.asts.is_empty() {
            return None;
        }
        Some(WorldScriptRuntime {
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
        })
    }
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
            // The tractor beam (issue #1156). Added here alongside its sibling
            // because a coupled target is moved through the same after-integration
            // `SimSet::Modifiers` window the rig uses.
            .add_plugins(crate::tractor::server::TractorPlugin)
            // Helm docking (issue #1159) stands beside the tractor for the same
            // reason: the own ship is flown onto its mate through the same
            // after-integration `SimSet::Modifiers` window the tractor rig uses,
            // and it is ordered after that rig so a hull that is both a docking
            // ship and a tractor target has a deterministic last writer.
            .add_plugins(crate::dock::server::DockPlugin)
            // The transfer umbilical (issue #1160) stands beside the dock it gates
            // on: its flow tick runs after the dock tick (so it reads the fresh
            // docked state) and before the infrastructure tick (so the capacity it
            // queues moves the same tick), the ordering the tractor's arrest keeps.
            .add_plugins(crate::umbilical::server::UmbilicalPlugin)
            .add_plugins(crate::civilian::CivilianPlugin)
            // Dossiers (issue #1030) join them for the same reason: the
            // commitments a fact sheet lists are a field on
            // `WorldContentRuntime`, the comms standing it reports is
            // `CommsRuntime`'s, and every subject on its roster came out of a
            // world file. The plugin registers a publisher and nothing else.
            .add_plugins(crate::dossier::DossierPlugin)
            // The science scan (issue #1032) for the same reason again: what it
            // reads is an authored structure's condition track, so it is
            // meaningless in an app with no world — and its tick is explicitly
            // ordered AFTER `InfrastructurePlugin`'s, so a scan taken on the
            // tick a repair lands reads the repaired number.
            .add_plugins(crate::science::SciencePlugin)
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
                    // Beside the deadline arm, and for its reason: a
                    // `[[workforce]]`'s authored strike status is the situation
                    // the crew ARRIVE INTO, so it must be true before the first
                    // handler runs and before any operation is offered
                    // (issue #1035). Runs its body exactly once per mission.
                    arm_mission_workforces,
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
                crate::ai::server::advance_objective_cursors
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
            // The two world-layer mutators (issue #1183). Both were `.in_set`
            // only, with NO edge relative to each other or to
            // `tick_trigger_pipeline` — yet all three write
            // `ResMut<WorldContentRuntime>` (and `apply_world_layer_changes`
            // additionally shares `ObjectiveManagerRes`, `WorldLayerMap` and
            // `PendingWorldLayerChanges` with the pipeline), so no two of them
            // can run in parallel and the executor was picking their order by
            // Bevy's `--deterministic` registration tie-break alone — the same
            // silent P2P-lockstep hazard (#854) the registration-shuffle guard
            // exists to close. These edges pin it.
            //
            // # Order: tick_trigger_pipeline -> scenario_loads -> layer_changes
            //
            // * AFTER `tick_trigger_pipeline` because the pipeline is the
            //   PRODUCER: a fired `load_world`/`unload_world` trigger action
            //   pushes a `WorldLayerChange` onto `PendingWorldLayerChanges` this
            //   tick (see `apply_dispatch_result`), which `apply_world_layer_changes`
            //   then drains — so it must observe the pipeline's same-tick output.
            //   `apply_pending_scenario_loads` names the pipeline too so BOTH
            //   world-runtime merges land at a FIXED position relative to the
            //   pipeline's own `WorldContentRuntime` writes: any world merged
            //   this tick first becomes live for trigger evaluation NEXT tick,
            //   the same one-tick cadence `collect_world_events` already imposes.
            // * scenario_loads BEFORE layer_changes because both APPEND to
            //   `WorldContentRuntime.trigger_states`, and that append order fixes
            //   the trigger-evaluation order (hence the digest). Pinning scenario
            //   first also keeps a layer's stored snapshot length (which
            //   `UnloadWorld` reverses by index) taken with scenario merges
            //   already in place, so an unload retains the scenario-merged
            //   triggers below its snapshot boundary.
            //
            // # Determinism
            // This pinned order EQUALS the order the schedule already resolved to
            // (insertion-order tie-break), so it moves no digest and no replay
            // fingerprint — it only removes the executor's freedom to pick a
            // different one. `registration_order_determinism` guards it: without
            // these edges the three form ambiguous, `WorldContentRuntime`-
            // conflicting pairs in `ScheduleGraph::conflicting_systems()`; with
            // them they are ordered and drop out.
            .add_systems(
                FixedUpdate,
                apply_pending_scenario_loads
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(tick_trigger_pipeline),
            )
            .add_systems(
                FixedUpdate,
                apply_world_layer_changes
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(tick_trigger_pipeline)
                    .after(apply_pending_scenario_loads),
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
    local_ship_q: Query<(), With<crate::server_app::LocalShip>>,
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
    local_ship_q: Query<(), With<crate::server_app::LocalShip>>,
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
    if let Some(world_config) = crate::entities::config_cache::get_world_config() {
        commands.insert_resource(world_config);
    }
}

/// `Startup` system: populate [`RawWorldSource`] from the browser bridge's
/// stashed raw world TOML (issue #984, Rhai M6 phase 2a).
///
/// The script-loader's twin of [`insert_world_config_resource`]: `WorldConfig`
/// has dropped the raw `[script]` / `script` keys, so the Rhai loader needs the
/// untouched TOML text. Reads the [`crate::server::bridge::BridgeWorldSource`]
/// Resource the wasm bridge inserts at `wasm_init` (issue #1181, replacing the
/// former `get_raw_world_source()` ambient free-function read). On native that
/// Resource is never inserted — headless compiles its scripts once in
/// `world::load::load` and hands them to `compile_world_scripts` as
/// [`PreCompiledScripts`] (issue #1214) — so `bridge` is `None` and this system
/// is a no-op there, exactly as its wasm-gated body used to be.
pub(crate) fn insert_raw_world_source_resource(
    mut commands: Commands,
    bridge: Option<Res<crate::server::bridge::BridgeWorldSource>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    // `toml::Value`'s `FromStr` parses a single VALUE EXPRESSION (`1`, `"x"`,
    // `[1, 2]`), not a document — so it rejects every world file ("unexpected
    // content, expected nothing"). This arm shipped with that misuse in #984 P2a
    // and stayed invisible while every world was script-free: the error logged,
    // and there were no scripts to lose. `toml::from_str` is the document parser
    // the rest of the crate uses (and the same route `parse_world` takes, which
    // is why the world itself loaded while its scripts vanished).
    match toml::from_str::<toml::Value>(&bridge.toml) {
        Ok(toml) => commands.insert_resource(RawWorldSource {
            path: bridge.path.clone(),
            toml,
        }),
        Err(e) => bevy::log::error!(
            target: "world",
            "insert_raw_world_source_resource: world TOML at {} failed to re-parse: {e}",
            bridge.path
        ),
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
/// # One compile per load (issue #1214)
///
/// Two sources feed this system, checked in order:
///
/// * [`PreCompiledScripts`] — the headless path. `build_headless_app` compiled
///   the world's scripts once in `world::load::load` (recording their content
///   into the not-yet-frozen content ledger there) and hands the result over, so
///   this system takes ownership and builds the runtime WITHOUT recompiling. This
///   is what collapses the old headless double-compile.
/// * [`RawWorldSource`] — the browser path. On wasm the raw world TOML is
///   inserted at `Startup` and this system compiles it here, as before.
///
/// Either way the construction below is identical; the only difference is where
/// the `CompiledScripts` came from.
///
/// Findings fold into the SAME atomic activation gate the composition findings
/// use: on a script error this records the block (read by
/// [`world_activation_blocked`]) so a script-error world spawns zero entities,
/// and inserts no runtime. Headless additionally hard-fails the build for a
/// script error (see `build_headless_app`), so a broken script never reaches a
/// running authoritative host.
pub(crate) fn compile_world_scripts(
    mut commands: Commands,
    pre: Option<ResMut<PreCompiledScripts>>,
    raw: Option<Res<RawWorldSource>>,
) {
    // Reset the per-load gate: every world load writes it fresh, so a script-free
    // world (or an app that never runs this system) reads `false`.
    set_script_activation_blocked(false);

    // Prefer scripts already compiled at build time (issue #1214): the headless
    // path runs the world through `world::load::load` once and hands the result
    // over as `PreCompiledScripts`, so this builds the runtime from it rather than
    // re-reading `RawWorldSource` and compiling a second time. `.take()` leaves the
    // resource holding `None`; a script-free `world::load` already stored `None`
    // there, which short-circuits exactly as the absent-`script`-key arm below.
    // The browser (no `PreCompiledScripts`) falls through to the `RawWorldSource`
    // compile it always did.
    let compiled = if let Some(mut pre) = pre {
        let Some(compiled) = pre.0.take() else {
            return;
        };
        compiled
    } else {
        let Some(raw) = raw else {
            return;
        };
        // No `script` key → nothing to compile. Short-circuit before building any
        // Rhai engine so the entire shipped (script-free) set pays nothing and can
        // never record into the content ledger.
        if raw.toml.get("script").is_none() {
            return;
        }
        let resolver = crate::entities::config_cache::production_script_resolver();
        crate::world::script::load::load_world_scripts(&raw.path, &raw.toml, &resolver)
    };

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

    // No scripts actually compiled (e.g. an empty `[script]` table) — `from_compiled`
    // returns `None`, so insert no runtime and leave behaviour identical to a
    // script-free world.
    if let Some(runtime) = WorldScriptRuntime::from_compiled(compiled) {
        commands.insert_resource(runtime);
    }
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

    let config_cache = crate::entities::config_cache::get_config_cache();
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
/// [`crate::entities::loader::SpawnTemplateLoader`] takes its authority from the
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
    config_cache: &crate::entities::config_cache::ConfigCache,
    system: &str,
) -> bool {
    let templates = crate::entities::loader::SpawnTemplateLoader {
        cache: config_cache,
        host: &crate::entities::loader::WasmTemplateLoader,
    };
    let findings = crate::world::validate::activation_findings(
        world_config,
        &crate::entities::include_resolve::HostFragmentSource,
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
    config_cache: &crate::entities::config_cache::ConfigCache,
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
            crate::entities::loader::template_is_asteroid_field(
                path,
                config_cache,
                &crate::entities::loader::WasmTemplateLoader,
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
        let mut config = match crate::entities::loader::resolve_entity_via(
            entity_inst,
            config_cache,
            &crate::entities::loader::WasmTemplateLoader,
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
        let entity = crate::entities::spawner::spawn_entity(
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
        let config = match crate::entities::loader::resolve_entity_via(
            entity_inst,
            config_cache,
            &crate::entities::loader::WasmTemplateLoader,
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
        let entity = crate::entities::spawner::spawn_entity(
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

/// Build the live workforce register from the world's `[[workforce]]` blocks,
/// mirroring each side's opening state into the flag store (issue #1035).
///
/// The twin of [`arm_mission_deadlines`] and deliberately smaller: a workforce
/// queues nothing, so there is no `ScheduledCall` to push and nothing to
/// retract later. What it does have to be is **early** — the register is what
/// decides whether a depot refuses a transfer, so it must hold the authored
/// answer before the first script handler runs rather than one tick after.
///
/// # Why the mirror flags are written straight into the store
///
/// Every *later* move of a workforce writes its flag as an ordinary
/// [`ActionCmd::MutateFlag`] through the trigger pipeline, so an
/// `on_flag_cleared` chains off it. The opening state deliberately does not: a
/// transition event on tick one would announce "the strike just started" for a
/// strike that was already happening when the crew arrived, and would fire
/// every trigger authored to watch for the settlement's opposite. This is the
/// same reading `InfrastructureState::from_config` takes when it level-evaluates
/// a degraded structure's thresholds instead of flipping them on tick one.
///
/// # Determinism
///
/// A no-op for every world that authors no workforce: the early returns happen
/// before any `DerefMut`, so no change-detection tick flips and a
/// workforce-free run is byte-identical to one from before this system existed.
pub(crate) fn arm_mission_workforces(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut runtime: ResMut<WorldContentRuntime>,
) {
    // Immutable reads only on the already-armed path, so an armed mission does
    // not mark `WorldContentRuntime` changed every tick.
    let Some(world_config) = world_config else {
        return;
    };
    if runtime.workforce.armed || world_config.workforces.is_empty() {
        return;
    }
    let mirror = runtime.workforce.arm(&world_config.workforces);
    for write in mirror {
        runtime.flags.set_flag_value(&write.name, write.value);
    }
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
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::server_app::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: FactionDispatchParams,
    time: Option<Res<bevy::time::Time>>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
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
    let uuid_source =
        || crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);
    // Template source for `SpawnEntity` dispatch (issue #715), built once per
    // system run. `WasmTemplateLoader` unconditionally: it serves the
    // preloaded config cache first and, on native, falls back to the
    // filesystem — reproducing the old cfg-split inline block on both targets.
    let template_loader = crate::entities::loader::WasmTemplateLoader;
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
                                &runtime.evidence,
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
                            &mut effect_queues.out(),
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
fn world_modifier_source(tag: String) -> crate::core::messages::ModifierSource {
    crate::core::messages::ModifierSource::World {
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
    // mint). `origin_layer`/`entity_name` come from the fired trigger (the
    // callback path passes `None`/`None`).
    uuid_source: &dyn Fn() -> String,
    template_loader: &dyn crate::entities::loader::TemplateLoader,
    base_anchors: &HashMap<String, [f32; 3]>,
    origin_layer: Option<String>,
    entity_name: Option<String>,
    // The per-owner effect queues (issue #1223), threaded through to the shared
    // `apply_dispatch_result` below unchanged.
    effects: &mut EffectQueuesOut,
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
                    effects,
                );
            }
        }
    }
}

/// The four transient per-tick effect queues an applied dispatch can enqueue
/// (issue #1223), borrowed as plain `&mut Vec<T>` so [`apply_dispatch_result`]
/// and [`apply_script_commands`] stay Bevy-agnostic — the same shape their
/// `runtime: &mut WorldContentRuntime` parameter already has. An effect-applying
/// SYSTEM holds the four `EffectQueue<T>` resources (via [`EffectQueues`]) and
/// lends them here with [`EffectQueues::out`]; a bare-`App` test lends four local
/// `Vec`s instead.
pub(crate) struct EffectQueuesOut<'a> {
    /// Drained by `crate::infrastructure::tick_infrastructure_condition`.
    pub condition_adjustments: &'a mut Vec<crate::infrastructure::ConditionAdjustment>,
    /// Drained by the same infrastructure tick.
    pub capacity_adjustments: &'a mut Vec<crate::infrastructure::CapacityAdjustment>,
    /// Drained by `crate::civilian::tick_civilian_traffic`.
    pub civilian_orders: &'a mut Vec<crate::civilian::PendingCivilianOrder>,
    /// Drained by `crate::console::captain::server::apply_scripted_weapons_holds`,
    /// as `(ship uuid, held)`.
    pub weapons_holds: &'a mut Vec<(String, bool)>,
}

/// The four per-owner [`EffectQueue`] resources an effect-applying SYSTEM needs,
/// bundled as one `SystemParam` (issue #1223) so a dispatch system gains one
/// parameter rather than four. Each resource is registered and declared
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
    condition: Option<ResMut<'w, EffectQueue<crate::infrastructure::ConditionAdjustment>>>,
    capacity: Option<ResMut<'w, EffectQueue<crate::infrastructure::CapacityAdjustment>>>,
    civilian_orders: Option<ResMut<'w, EffectQueue<crate::civilian::PendingCivilianOrder>>>,
    weapons_holds: Option<ResMut<'w, EffectQueue<(String, bool)>>>,
    condition_fallback: Local<'s, Vec<crate::infrastructure::ConditionAdjustment>>,
    capacity_fallback: Local<'s, Vec<crate::infrastructure::CapacityAdjustment>>,
    civilian_orders_fallback: Local<'s, Vec<crate::civilian::PendingCivilianOrder>>,
    weapons_holds_fallback: Local<'s, Vec<(String, bool)>>,
}

impl EffectQueues<'_, '_> {
    /// Borrow the four queues as an [`EffectQueuesOut`] to lend to the applier,
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
            weapons_holds: match &mut self.weapons_holds {
                Some(q) => &mut q.0,
                None => &mut self.weapons_holds_fallback,
            },
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
    // The four transient effect queues a name-resolved command lands on (issue
    // #1223): condition/capacity adjustments, civilian orders and weapons holds
    // used to be `pending_*` fields on `runtime`; each is now its owning plugin's
    // `EffectQueue<T>` resource, lent here as plain `&mut Vec<T>`.
    effects: &mut EffectQueuesOut,
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
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
                origin_layer,
            } => {
                let added = objectives.0.add_full_with_params(
                    id.clone(),
                    text,
                    text_params,
                    mandatory,
                    targets,
                    directive,
                    utility,
                    source,
                    command_stance,
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
                        msgs.write(crate::core::balance::BalanceEvent::ObjectiveCompleted {
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

            // Issue #1041. The name is resolved here — the applier is where
            // `name_to_uuid` lives — and the order is queued for
            // `apply_scripted_weapons_holds`, which is the one system holding
            // an entity query. The mirror flag is NOT written here: it is
            // mirrored off the authoritative component, so a scenario's order
            // and a captain's press produce the same transition event.
            ActionCmd::SetWeaponsHold { entity, held } => {
                let Some(uuid) = runtime.name_to_uuid.get(&entity).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: SetWeaponsHold: no entity named '{entity}' in this \
                         world — ignoring"
                    );
                    continue;
                };
                effects.weapons_holds.push((uuid, held));
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
            } => {
                let Some(uuid) = runtime.name_to_uuid.get(&subject).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: RecordDossierEvidence: no entity named '{subject}' in \
                         this world — ignoring"
                    );
                    continue;
                };
                if !runtime
                    .evidence
                    .append(&uuid, &text, provenance, gathered_at_tick)
                {
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
                commands
                    .entity(spawned)
                    .insert(crate::entities::spawner::EntitySpawnOrigin(
                        crate::world::spawn_origin::SpawnOrigin {
                            template_path,
                            name,
                            position,
                            rotation,
                            scale,
                            overrides,
                            layer_path: layer_path.clone(),
                        },
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
                        world.get_resource_mut::<Messages<crate::ai::server::AiEntityDestroyed>>()
                    {
                        msgs.write(crate::ai::server::AiEntityDestroyed { entity_uuid: uuid });
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
    mut game_over_reason: Option<ResMut<crate::server_app::GameOverReason>>,
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
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
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
    let template_loader = crate::entities::loader::WasmTemplateLoader;
    let uuid_source =
        || crate::world_id::mint_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity);

    // Ready/still-pending is a pure decision (`world::delayed`); only the
    // elapsed-clock read above and the dispatch below touch Bevy.
    let queued = std::mem::take(&mut runtime.pending_delayed_actions);
    let schedule = partition_delayed_actions(queued, elapsed);
    runtime.pending_delayed_actions = schedule.still_pending;

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
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
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
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
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
    let template_loader = crate::entities::loader::WasmTemplateLoader;
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
                    &runtime.evidence,
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
            &mut effect_queues.out(),
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
    pub attacked: MessageReader<'w, 's, crate::ai::server::AiEntityAttacked>,
    pub destroyed: MessageReader<'w, 's, crate::ai::server::AiEntityDestroyed>,
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

use crate::entities::spawner::BehaviourSection;
use crate::entities::spawner::EntityUuid;

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
        // Dedup before any TOML read so a duplicate never touches the WASM fetch
        // queue — and is never re-marked, since it is already recorded in
        // `loaded_scenario_paths`. (Absorbed from the former `world::scenario`
        // decision layer, issue #1215.)
        if runtime.loaded_scenario_paths.contains(&path) {
            continue;
        }
        // Resolve the TOML (I/O; also records the text into the content ledger).
        // `None` means the WASM fetch is still in flight — re-queue for the next
        // frame without marking the path loaded.
        let Some(toml_str) = load_scenario_toml(&path) else {
            pending.0.push(path);
            continue;
        };
        // Route the parse (and the merged world's script-carry, dropped here) through
        // the one world-load sequence under `Merge`. A `MemoryReader` seeded with the
        // text `load_scenario_toml` already read — and already recorded — keeps the
        // decision pure; the returned `LedgerPlan` is dropped (the ledger recording
        // is owned above) and a layered `[script]` taking effect stays a ledger-freeze
        // policy question for #1045 (see `world::layers`), so its resolver is
        // `NoSiblingScripts` and the compiled set is discarded.
        let reader = MemoryReader::new([(path.clone(), toml_str)]);
        let request = LoadRequest::new(
            path.as_str(),
            &reader,
            &crate::world::script::load::NoSiblingScripts,
            LoadPolicy::Merge,
        );
        match load(request) {
            Ok(_loaded) => {}
            Err(LoadError::ParseFailed { message, .. }) => {
                bevy::log::error!(
                    "apply_pending_scenario_loads: failed to parse {path}: {message}"
                );
            }
            // Unreachable with a MemoryReader under Merge (see `world::layers`), but
            // mapped defensively rather than panicking.
            Err(other) => {
                bevy::log::error!("apply_pending_scenario_loads: failed to load {path}: {other}");
            }
        }
        // Mark loaded on success AND on parse failure — a broken file must not be
        // retried frame after frame.
        runtime.loaded_scenario_paths.insert(path);
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
) -> crate::entities::config_cache::ConfigCache {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut cache = crate::entities::config_cache::get_config_cache();
        for entity in &_world_config.entities {
            if cache.contains_key(&entity.template_path) {
                continue;
            }
            if std::fs::metadata(&entity.template_path).is_err()
                && crate::entities::config_cache::mod_pack_overlay_get(&entity.template_path)
                    .is_none()
            {
                // Template not on disk (e.g. test fixture); skip silently.
                // spawn_immediate_entities_internal logs and continues for
                // missing templates.
                continue;
            }
            // The `includes` closure resolves here too (issue #869), so a
            // composed hull referenced by a world reaches the layer cache fully
            // merged — the same single document the browser preload assembles.
            match crate::entities::include_resolve::load_entity_config(&entity.template_path) {
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
        crate::entities::config_cache::get_config_cache()
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
                        scripts,
                    } => {
                        // Drop the layer's compiled `[script]` set: a supporting
                        // world's script does not yet take effect. The route that
                        // delivers it here is present (issue #1215); making it FIRE
                        // is a ledger-freeze policy question for #1045 — see
                        // `world::layers`. `None` for the entire shipped set.
                        let _ = scripts;

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
        crate::entities::config_cache::pop_pending_world_toml(path).or_else(|| {
            // Fire a JS fetch request if we haven't already.
            crate::entities::config_cache::request_world_fetch(path.to_string());
            None
        })
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
pub(crate) mod tests;
