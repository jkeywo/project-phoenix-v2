use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use rhai::{Map, AST};

use crate::core::messages::{GamePhase, ServerMessage};
use crate::effect_queue::EffectQueue;
use crate::lobby::{Target, WorldResource};
use crate::objectives::ObjectiveManager;
use crate::server_app::SimOutbox;
#[cfg(test)]
use crate::world::content::TriggerAction;
#[cfg(test)]
use crate::world::content::TriggerState;
use crate::world::content::WorldEvent;
use crate::world::delayed::{partition_delayed_actions, DelayedAction};
use crate::world::dispatch::{
    dispatch_action, ActionCmd, DispatchContext, DispatchResult, LayerView,
    WORLD_MODIFIER_SOURCE_ID,
};
use crate::world::layers::{evaluate_layer_load, LayerLoadOutcome, LayerValidationContext};
use crate::world::load::{load, LoadError, LoadPolicy, LoadRequest, MemoryReader};
use crate::world::script::effects::BufferedEffect;
use crate::world::script::engine::{RuntimeHost, ScriptTrigger};
use crate::world::script::schedule::{CallEffects, PendingCallbacks, SchedClock, TickBudget};

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
    /// Ordered continuation/handler pairs and their observer generation.
    pub triggers: crate::world::trigger_registry::WorldTriggerRegistry,
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
    // their own per-owner `crate::effect_queue::EffectQueue<T>` resources
    // (issue #1223), registered and drained by the plugin that owns each edge
    // (Infrastructure / Civilian / Power — the last was the captain's until
    // #1398 turned the scripted weapons hold into a reactor order). They lived
    // here partly so the authoritative-state census "saw no new registration";
    // #1220–#1222 gave the census a real declaration registry, so each queue is
    // now declared `ClearedAtFold` at its owning `build()` instead.
    // `pending_world_events` and `pending_delayed_actions` stay: unlike those
    // four they are NOT empty at a tick boundary (they are snapshotted /
    // carried across ticks), so they are deferred state rather than a transient
    // inter-system queue.
    /// Layer-qualified ids of GM `Fire` requests that have crossed their
    /// canonical apply boundary and are waiting for the trigger pipeline to
    /// run their handler (issue #1301).
    ///
    /// A `BTreeSet` rather than a `HashSet` or a `Vec`: it is authoritative
    /// state that is captured, folded and replayed, so its iteration order must
    /// be identical on every peer and its contents must not carry a duplicate
    /// (a second Fire of an already-armed event is a No-op, not a second run).
    ///
    /// It is genuinely cross-tick rather than a transient inter-system queue:
    /// [`crate::gm_action::apply_due_actions`] arms it in `PreUpdate` at the
    /// grant's exact apply tick, while a `when` predicate or a cooldown can
    /// legitimately hold the entry for many ticks before the pipeline can honour
    /// it. So, like `pending_world_events` and `pending_delayed_actions`, it
    /// lives here and is snapshotted.
    pub pending_gm_event_fires: std::collections::BTreeSet<String>,
    /// The scenario-authored GM spawn palette, copied from `WorldConfig` at
    /// load (issue #1305).
    ///
    /// AUTHORED CONTENT, not run state: it lives here for `trigger_states`'
    /// reason — the deterministic apply-tick reducer and the trigger pipeline
    /// both need it, and neither holds `WorldConfig` — and, like a trigger's
    /// authored condition, it is answered for by `snapshot::content_digest`
    /// rather than captured or folded. A resumed world rebuilds it by replaying
    /// the same load.
    pub gm_palette: Vec<crate::world::config::GmPaletteEntry>,
    pub gm_objective_palette: Vec<crate::gm_objective::ObjectivePaletteEntry>,
    pub gm_npc_doctrine_palette: Vec<crate::gm_npc::NpcDoctrinePaletteEntry>,
    /// GM placements that have crossed their canonical apply boundary and are
    /// waiting for the trigger pipeline to spawn them (issue #1305).
    ///
    /// A `Vec` in canonical grant order rather than a set: two identical
    /// placements of the same palette entry are two entities, not one, and the
    /// ORDER decides which draws which uuid from the `WorldIdMint`. Genuinely
    /// cross-tick for `pending_gm_event_fires`' reason — armed in `PreUpdate`,
    /// drained in `FixedUpdate`, which a paused session never reaches — so it is
    /// snapshotted and folded.
    pub pending_gm_spawns: Vec<crate::gm_spawn::PendingGmSpawn>,
    /// Canonically ordered UUID removals accepted before the fixed pipeline.
    pub pending_gm_despawns: Vec<String>,
    pub contact_overrides: crate::gm_contact::ContactOverrides,
    /// Layer-qualified ids of GM-operable events a Game Master has PAUSED
    /// (issue #1303).
    ///
    /// A `BTreeSet` for [`Self::pending_gm_event_fires`]' reasons, and captured
    /// and folded beside it. What it means is narrower than it looks: while an
    /// id is in here, `tick_trigger_pipeline` does not EVALUATE that trigger's
    /// automatic condition at all — it never reaches
    /// `trigger_fires_for_events`, so no `seen_destroyed` name is accumulated,
    /// no `fired` latch is set and no cooldown is stamped. That is what "no
    /// missed edge is captured" has to mean mechanically: an `OnAllDestroyed`
    /// that merely skipped its firing would still have banked the destructions
    /// and would fire the instant it resumed.
    ///
    /// It gates ONLY the automatic pass. An armed Fire is honoured while
    /// paused, because Fire is the lever a GM reaches for precisely when the
    /// automatic opportunity has gone.
    pub paused_gm_events: std::collections::BTreeSet<String>,
    /// Layer-qualified ids of GM `Skip` requests that have crossed their
    /// canonical apply boundary and are waiting for the AUTOMATIC occurrence
    /// they will stand in front of (issue #1304).
    ///
    /// A `BTreeSet` for [`Self::pending_gm_event_fires`]' reasons, and cross-tick
    /// far more emphatically than that set is: a Fire arm is normally consumed
    /// the tick it lands, while a Skip arm waits for the world to produce an
    /// occurrence that may be many minutes away or may never come at all. So it
    /// is captured, folded and replayed exactly as the Fire set is.
    ///
    /// The two sets are read and consumed independently, which is what makes
    /// "Fire does not consume an armed Skip" true by construction rather than
    /// by a rule somebody has to remember: the manual pass in
    /// [`tick_trigger_pipeline`] touches only the Fire set, and the automatic
    /// evaluation loop touches only this one.
    pub pending_gm_event_skips: std::collections::BTreeSet<String>,
}

/// Bevy resource wrapping the server-side objective manager.
#[derive(Resource, Default)]
pub struct ObjectiveManagerRes(pub ObjectiveManager);

/// Queue of world TOML paths to load additively into the live `WorldContentRuntime`.
///
/// **Nothing enqueues into it, and draining it merges nothing.** The
/// `apply_pending_scenario_loads` system reads each path, records its TOML into
/// the content ledger, parses it to prove it parses, and marks it loaded — the
/// trigger/comms merge it once described was deleted with the `[[trigger]]` /
/// `[[comms]]` front-ends (issue #985), and no `TriggerAction` has ever pushed a
/// path here. What survives is the de-duplicating `loaded_scenario_paths`
/// bookkeeping.
///
/// A supporting world that wants to CONTRIBUTE anything goes through
/// `WorldLayerChange` and `apply_world_layer_changes` instead: that path has a
/// `WorldLayerMap` entry to hang the layer's entities and scripts off, and so can
/// take them back out at `UnloadWorld`. This one cannot (see
/// `apply_pending_scenario_loads`).
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
/// — the only way a layer could author one — and issue #1045 gave the capability
/// back through `[script]`. The states themselves are no longer snapshotted here:
/// each is origin-tagged with this layer's path in the live table, which is what
/// [`crate::world::trigger_registry::WorldTriggerRegistry::remove_layer`] matches on at unload.
#[derive(Clone, Debug, Default)]
pub struct WorldRuntime {
    /// `true` only after a layer completed atomic activation. Failed-load
    /// sentinels deliberately remain `false`: they occupy the path to suppress
    /// retry loops, but are not part of the active composition a snapshot must
    /// recreate. A successfully loaded content-empty layer is still `true`.
    pub is_active: bool,
    /// Relative atomic-activation order among the active layers. Snapshot
    /// topology sorts on this before restoring index-aligned trigger state.
    /// Failed sentinels keep the default zero; a reloaded layer receives an
    /// ordinal after every survivor.
    pub activation_order: u64,
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
    /// `extra_worlds`) — the loader is the base world itself, so
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
    /// Script units (AST keys) this layer's `[script]` block ADDED to the live
    /// [`WorldScriptRuntime::asts`] (issue #1045).
    ///
    /// Every compiled unit this layer owns, including a sibling `.rhai` shared
    /// with another layer. The live AST is retained once, while `ast_owners`
    /// records each owning layer; unload removes this layer's ownership and only
    /// drops the AST after the final owner leaves. Empty for a script-free layer.
    pub script_units: Vec<String>,
}

/// Map of `path → WorldRuntime` for sub-worlds loaded via `LoadWorld` / `extra_worlds`.
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
    (flag_chain, layer_path_chain(origin, layer_map))
}

/// The layer-PATH chain alone, innermost first: `chain[0]` is `origin` itself
/// (`None` = base world), each later entry its loader, terminating at `None`.
///
/// The half of [`layered_flag_chain_with_paths`] that `parent:` resolution needs
/// and nothing else does. Split out (issue #1045) because
/// [`apply_script_commands`] resolves a scripted flag write's target layer while
/// holding `&mut WorldContentRuntime` — it cannot also borrow the flag stores the
/// other half returns, and does not want them.
pub fn layer_path_chain(
    origin: Option<&str>,
    layer_map: Option<&WorldLayerMap>,
) -> Vec<Option<String>> {
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
    layer_chain
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
///
/// NOT `Clone`: [`DeferredApply`](Self::DeferredApply) carries a
/// [`LoadedLayer`](crate::world::layers::LoadedLayer), whose compiled Rhai ASTs
/// cannot be cloned. Nothing copies a queued change — the applier drains the
/// queue by value — so the bound was never load-bearing.
#[derive(Debug)]
pub enum WorldLayerChange {
    /// Load a sub-world. `loader_path` is the layer whose trigger called
    /// `LoadWorld(path)` to enqueue this — `None` for startup-time loads
    /// (base world's `extra_worlds`). Recorded on the new
    /// `WorldRuntime.loader_path` so `parent:` walks from the loaded
    /// layer reach the right outer flag store (PRD #397 fix 1).
    Load {
        path: String,
        loader_path: Option<String>,
    },
    /// The layer TOML has arrived and is retained here while its declared
    /// sibling `.rhai` fetch is pending. Neither source is re-requested and the
    /// layer is not parsed/compiled/minted until the sibling reaches a terminal
    /// state.
    AwaitingScript {
        path: String,
        loader_path: Option<String>,
        toml: String,
        script_path: String,
    },
    /// A `Load` that was already EVALUATED on an earlier tick and is waiting for
    /// the `WorldScriptRuntime` that tick inserted to become visible (issue
    /// #1045). The applier merges and spawns straight from `layer`.
    ///
    /// It carries the evaluated decision rather than re-running the evaluation
    /// because re-evaluation would reparse the layer and mint different entity
    /// UUIDs. Carrying the outcome makes native and wasm identical and ensures
    /// the eventual activation is exactly the decision evaluated on this tick.
    DeferredApply {
        path: String,
        loader_path: Option<String>,
        layer: Box<crate::world::layers::LoadedLayer>,
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

/// The world this session loaded — `(path, TOML text)` — handed from the wasm
/// edge into the `World` (issue #1181, replacing the `get_raw_world_source()`
/// ambient read [`insert_raw_world_source_resource`] used).
///
/// `wasm_init` inserts this from the `SNAPSHOT_WORLD` edge stash before the app
/// runs; the browser's `Startup` re-parse then reads it as an ordinary
/// `Option<Res<BridgeWorldSource>>` instead of reaching back through a bridge
/// free function. Never inserted on native (that path uses [`PreCompiledScripts`]),
/// so the read is a no-op there exactly as the old wasm-gated body was.
///
/// Lives here — beside its [`RawWorldSource`] consumer — rather than in
/// `crate::server::bridge` (issue #1194): it is sim-visible state read by the
/// always-compiled [`insert_raw_world_source_resource`], so the `--server`
/// feature gate must not be able to compile it out. The wasm bridge only inserts
/// it (`wasm_init`), through this path.
#[derive(Resource, Clone, Debug)]
pub struct BridgeWorldSource {
    /// The world TOML's path (`Run::scenario` / content-ledger key).
    pub path: String,
    /// The untouched world TOML text.
    pub toml: String,
}

/// The raw world source a session loaded: its path plus the world TOML as a
/// `Value`.
///
/// `WorldConfig` drops the raw `[script]` / `script` keys the Rhai loader needs,
/// so this carries the whole TOML alongside its path. Since issue #1214 it is the
/// **browser's** route only: `insert_raw_world_source_resource` reads the
/// [`BridgeWorldSource`] Resource the wasm bridge inserts at
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

/// Runtime state for a world that authors Rhai scripts (issue #984, Rhai M6
/// phase 2a).
///
/// Inserted at `Startup` by [`compile_world_scripts`] when the root world has a
/// runnable AST, or just before applying the first scripted child layer. Its
/// lifecycle mirrors [`WorldContentRuntime`]; child units are reference-counted
/// by explicit owners and may enter or leave while the root runtime persists.
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
    /// Owners retaining each AST unit. `None` denotes the root world; layer
    /// paths are explicit so shared sibling scripts survive either unload.
    pub ast_owners: BTreeMap<String, BTreeSet<Option<String>>>,
    /// Compiled registrations consumed by the trigger registry at activation.
    pub triggers: Vec<ScriptTrigger>,
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
    /// [`apply_script_call`] extends it from the call's [`CallEffects`].
    ///
    /// It lives here, not on a comms resource, for the same reason the callback
    /// queue does — this is the resource the script systems already borrow, so
    /// routing costs one line per call site and a wholly script-free composition
    /// has no `WorldScriptRuntime` at all, hence no queue and no behaviour.
    /// The request itself is comms vocabulary from `comms::content`, keeping the
    /// #816 split intact: world runtime holds the strings, the comms module owns
    /// what they mean.
    pub pending_comms_opens: Vec<crate::comms::content::OpenCommsRequest>,
    /// `on_deadline("id", "handler")` declarations collected at load (issue
    /// #1024), pairing each authored `[[deadline]]` with the fn it runs and the
    /// unit that said so. Root declarations are read by
    /// [`arm_mission_deadlines`]; child declarations are consumed atomically by
    /// `apply_loaded_layer` at that layer's activation tick. The unit path is
    /// part of the `ScheduledCall` key each deadline arms with.
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
        let ast_owners = compiled
            .asts
            .keys()
            .cloned()
            .map(|path| (path, BTreeSet::from([None])))
            .collect();
        Some(WorldScriptRuntime {
            host: RuntimeHost::new(),
            asts: compiled.asts,
            ast_owners,
            triggers: compiled.script_triggers,
            budget: TickBudget::new(),
            budget_tick: 0,
            content_hash: compiled.content_hash,
            pending_callbacks: PendingCallbacks::new(),
            pending_comms_opens: Vec::new(),
            deadline_handlers: compiled.deadline_handlers,
        })
    }

    /// An empty runtime with no compiled content, for the one case a script
    /// runtime has to exist without a base-world script set: a script-free base
    /// world whose LAYER brings the session's first `[script]` block (issue
    /// #1045).
    ///
    /// Inserted by `apply_world_layer_changes` as a LANDING PAD one tick before
    /// the layer merges into it (see that system's docs for why the merge waits),
    /// so [`merge_layer_scripts`] has exactly one shape to fill — a live `ResMut`
    /// — rather than a create path and a merge path that could drift.
    /// [`content_hash`](Self::content_hash) stays `0`: the field is the BASE
    /// world's script-set hash, and a layer's set binds a save through its own
    /// `<layer path>#scripts` content-ledger record (written by
    /// `load_world_scripts`), not through this number.
    pub(crate) fn empty() -> Self {
        WorldScriptRuntime {
            host: RuntimeHost::new(),
            asts: BTreeMap::new(),
            ast_owners: BTreeMap::new(),
            triggers: Vec::new(),
            budget: TickBudget::new(),
            budget_tick: 0,
            content_hash: 0,
            pending_callbacks: PendingCallbacks::new(),
            pending_comms_opens: Vec::new(),
            deadline_handlers: Vec::new(),
        }
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
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<ObjectiveManagerRes>(StateClass::Folded, "objective-runtime-state");
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
            // Security teams (issue #1346) stand beside them for the same reason:
            // the targets a team crosses to are world furniture spawned from world
            // files, and the consequence a completed action raises is a flag on
            // this plugin's own `WorldContentRuntime` store, so a scenario trigger
            // fires the tick the work lands.
            .add_plugins(crate::security::SecurityPlugin)
            // The rescue transporter (issue #1348) stands beside them: what it
            // recovers is the civilians an authored contact carries, its discovery
            // reads the `scan.<id>.taken` flag on this plugin's own
            // `WorldContentRuntime` store, and the completion/casualty facts it
            // raises are flags on that same store, so an authored beat fires the
            // tick the rescue lands or a carrier is lost.
            .add_plugins(crate::transporter::server::TransporterPlugin)
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
            // Debris hazards (issue #1347) stand beside the science plugin, and
            // are ordered after it inside `SimSet::Modifiers`: what promotes a
            // drifting rock to a confirmed threat is a scan, so the tick that
            // latches an assessment has to run after the tick that takes one.
            .add_plugins(crate::debris::DebrisPlugin)
            // Controlled demolition (issue #1350) stands beside Security, whose
            // command target it borrows and whose team states it reads: the
            // obstruction it clears is world furniture spawned from a world file,
            // and the four outcomes it decides raise flags on this plugin's own
            // `WorldContentRuntime` store, so a scenario trigger fires the tick the
            // charges go off.
            .add_plugins(crate::demolition::DemolitionPlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<PendingScenarioLoad>()
            .init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .init_resource::<WorldEventBuffer>();
        super::materialization::register(app, Startup);
        app.add_systems(
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
            crate::ai::server::advance_objective_cursors.in_set(crate::sim_sets::SimSet::Modifiers),
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
            apply_world_layer_changes
                .in_set(crate::sim_sets::SimSet::Physics)
                .before(collect_world_events)
                .before(tick_script_callbacks),
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
    // dropped here — they still receive region effects via the other
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
/// simply see an empty world (native unit tests only — production always
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
/// untouched TOML text. Reads the [`BridgeWorldSource`]
/// Resource the wasm bridge inserts at `wasm_init` (issue #1181, replacing the
/// former `get_raw_world_source()` ambient free-function read). On native that
/// Resource is never inserted — headless compiles its scripts once in
/// `world::load::load` and hands them to `compile_world_scripts` as
/// [`PreCompiledScripts`] (issue #1214) — so `bridge` is `None` and this system
/// is a no-op there, exactly as its wasm-gated body used to be.
pub(crate) fn insert_raw_world_source_resource(
    mut commands: Commands,
    bridge: Option<Res<BridgeWorldSource>>,
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
///
/// The browser's [`RawWorldSource`] arm also validates the exact compiled
/// script-spawn references for template composition/resolution and doctrine,
/// because browser boot does not pass through `LoadPolicy::Activate`. The
/// [`PreCompiledScripts`] arm deliberately does not repeat that gate: native
/// Activate already consumed the same compiled references before handing them
/// here.
pub(crate) fn compile_world_scripts(
    mut commands: Commands,
    pre: Option<ResMut<PreCompiledScripts>>,
    raw: Option<Res<RawWorldSource>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
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
    let mut resolved_spawn_findings = Vec::new();
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
        let compiled =
            crate::world::script::load::load_world_scripts(&raw.path, &raw.toml, &resolver);
        // The compiled set's digest used to be written from inside the loader;
        // since issue #1241 it comes back as data and its caller applies it. This
        // is the BROWSER's caller, so the write lands exactly where it always did
        // — the same statement, the same thread, the same tick. The ledger is
        // already frozen by `wasm_init` on this target, so a browser save binds
        // through the world TOML's inline blocks rather than through this record;
        // that was true before the lift and is true after it.
        if let Some(digest) = &compiled.ledger_digest {
            digest.apply();
        }
        // Browser boot does not pass through `LoadPolicy::Activate`, so its
        // already-compiled exact source set needs the root script-spawn half of
        // the composition gate here, before any runtime or spawn can land.
        // Deliberately confined to RawWorldSource: PreCompiledScripts came from
        // the native Activate gate and must not be validated twice.
        if let Some(config) = world_config.as_deref() {
            let cache = crate::entities::config_cache::get_config_cache();
            let templates = crate::entities::loader::SpawnTemplateLoader {
                cache: &cache,
                host: &crate::entities::loader::WasmTemplateLoader,
            };
            resolved_spawn_findings = crate::world::validate::validate_resolved_script_spawns(
                config,
                &compiled.spawned_templates,
                &templates,
                &crate::entities::include_resolve::HostFragmentSource,
            );
        }
        compiled
    };

    let script_blocked = crate::world::validate::has_error(&compiled.findings);
    let spawn_blocked = crate::world::validate::has_error(&resolved_spawn_findings);
    if script_blocked || spawn_blocked {
        for f in compiled.findings.iter().filter(|f| f.is_error()) {
            bevy::log::error!(
                target: "world",
                "compile_world_scripts: script [error] {}: {}",
                f.category, f.message
            );
        }
        for f in resolved_spawn_findings.iter().filter(|f| f.is_error()) {
            match f.source.line {
                Some(line) => bevy::log::error!(
                    target: "world",
                    "compile_world_scripts: composition [error] {}:{} [{}] {}",
                    f.source.file,
                    line,
                    f.category,
                    f.message
                ),
                None => bevy::log::error!(
                    target: "world",
                    "compile_world_scripts: composition [error] {} [{}] {}",
                    f.source.file,
                    f.category,
                    f.message
                ),
            }
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
/// the world flag store from the fleet's authoritative hull `power_rating`.
///
/// This runs before `spawn_game_start_entities` so that `when` predicates on
/// `[[entity]]` entries with `spawn_on = "GameStart"` can gate spawns on
/// `counter(ship_power) >= N`.
///
/// The value must fold identically on every peer (issue #1116): the counter is
/// part of the scenario-flag digest, and a stationless GM owns no local ship
/// while two ship hosts each own a *different* one. So the rating is derived
/// from the frozen [`FleetRoster`]'s hull paths — resolved through the same
/// config cache on every peer — rather than from this peer's own
/// `ShipClientConfigResource`. A fleet ship whose `ship_path` is `None` is the
/// solo default ("the hull this peer's own lobby selected"), which is exactly
/// the local ship config; a solo run therefore keeps its pre-fleet value to the
/// byte. Multiple crewed hulls take the highest rating, so a mixed fleet is
/// gated by its heaviest ship rather than by whichever host happens to seed.
///
/// If no hull declares a `power_rating`, no counter is written and `ship_power`
/// defaults to `0`.
pub fn seed_ship_power_counter(
    fleet_roster: Option<Res<crate::lockstep::FleetRoster>>,
    ship_client_config: Res<crate::lobby::server::ShipClientConfigResource>,
    runtime: Option<ResMut<WorldContentRuntime>>,
) {
    let Some(mut runtime) = runtime else {
        return;
    };
    let cache = crate::entities::config_cache::get_config_cache();
    let rating = match fleet_roster.as_deref() {
        Some(roster) => roster
            .ships()
            .iter()
            .filter_map(|ship| match &ship.ship_path {
                Some(path) => cache.get(path).and_then(|config| config.power_rating),
                None => ship_client_config.0.power_rating,
            })
            .max(),
        None => ship_client_config.0.power_rating,
    };
    if let Some(rating) = rating {
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
    // `ship_power` is seeded on `OnEnter(InProgress)` — not available at
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
/// # What is gated on has widened past `[[entity]]` (issue #1046)
///
/// The findings this reads now cover the hulls a world's SCRIPTS spawn as well
/// as the ones it declares — a scripted `spawn_entity` naming a literal
/// `template_path` is template-resolved and doctrine-anchor checked like any
/// other. On native that has teeth here: `SpawnTemplateLoader` is authoritative
/// about absence, so a script wave pointed at a deleted hull now blanks BOTH
/// immediate-spawn halves at `Startup` instead of logging once, mid-mission,
/// when its timer fires.
///
/// That is the intended reading of atomicity rather than a side effect — a
/// world that cannot build one of its waves is as broken as one that cannot
/// build a planet, and finding out at `Startup` is the whole point of the gate.
/// It does mean a finding here can now name a spawn site that is not an
/// `[[entity]]` block, which is why script findings carry the script's own line
/// (`world::validate`'s `script_spawn_line`): a bare template path with no
/// authored `name` is otherwise very hard to place.
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
/// is unset) at Startup time — `ship_power` is seeded on
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
        // applied to the streaming spawner. Missing anchor → warn + fall back
        // to world origin so a typo never silently relocates the field.
        if let Some(field) = config.asteroid_field.as_mut() {
            if let Some(anchor_name) = field.anchor.as_ref() {
                match world_config.anchors.get(anchor_name) {
                    Some(pos) => field.anchor_offset = *pos,
                    None => {
                        bevy::log::warn!(
                            "spawn_world_entities: asteroid field '{}' references unknown anchor '{}' — falling back to world origin",
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
    // entity. A missing registration is a programmer error — log and skip
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
                    "spawn_world_entities: named entity '{}' has no UUID in WorldConfig.name_to_uuid — skipping",
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
        // The authored narrative mark (issue #1338). Attached here rather than
        // in a `SpawnSection` because the payload is the WORLD's authored
        // `name` — the unique reference id triggers, comms and objectives all
        // address this instance by — and the entity template knows nothing
        // about it. Only this named branch can mark: an anonymous or
        // asteroid-field instance has no name to carry.
        if entity_inst.narrative {
            commands
                .entity(entity)
                .insert(crate::core::narrative::NarrativeMark(name.clone()));
        }
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

    // Replace the authored GM palette on each world load (#1305).
    runtime.gm_palette = world_config.gm_palette.clone();
    runtime.gm_objective_palette = world_config.gm_objective_palette.clone();
    runtime.gm_npc_doctrine_palette = world_config.gm_npc_doctrine_palette.clone();

    // Reset the complete paired table; scripts are the production source.
    runtime.triggers.clear();

    // Merge script-authored triggers (issue #984, Rhai M6 phase 2a). `None`
    // origin: these are the BASE world's own, so they anchor at the base flag
    // store and no `UnloadWorld` can retract them.
    if let Some(script_runtime) = script_runtime.as_deref_mut() {
        merge_script_triggers(&mut runtime, script_runtime, None);
    }

    // Issue #415: emit a WorldLoaded event so `on_world_loaded` triggers
    // declared in the base world fire on the first Update tick. Pushed onto
    // the pending queue (rather than evaluated here) so the dispatch logic
    // inside `tick_trigger_pipeline` is the single owner of trigger action
    // execution.
    runtime.pending_world_events.push(WorldEvent::WorldLoaded);
}

/// Transfer staged registrations into the registry as one ordered layer.
pub(crate) fn merge_script_triggers(
    runtime: &mut WorldContentRuntime,
    script_runtime: &mut WorldScriptRuntime,
    origin_layer: Option<&str>,
) {
    runtime
        .triggers
        .append_scripted(std::mem::take(&mut script_runtime.triggers), origin_layer);
}

/// Merge one layer's compiled `[script]` set into the live script runtime
/// (issue #1045), returning the AST keys this load ADDED so the unload can
/// retract exactly them.
///
/// The one place a supporting world's scripts join a running session.
///
/// **ASTs** are inserted under their authored key. A key already present is left
/// alone and NOT recorded as this layer's: two worlds may name the same sibling
/// `.rhai`, and an unload must not pull a unit someone else still calls into.
///
/// **Triggers** are staged onto `triggers` and drained by
/// [`merge_script_triggers`] with this layer's path as their origin, so they
/// evaluate against the layer's flag chain and retract with it.
///
/// # Only what this layer ACTUALLY added is registered
///
/// A trigger is registered only when the unit that built it is one this load
/// inserted. The two go together: a registration is *derived from running a
/// unit's top level*, so a layer that names a `.rhai` the base world (or an
/// earlier layer) already loaded arrives carrying that unit's registrations a
/// second time. Appending them unconditionally would give the shared script two
/// `on_world_loaded` states — and the applier's own `WorldLoaded` push would fire
/// the intro twice on the tick the layer landed. Instead the AST is retained once
/// and the registrations are instantiated once per owning layer, with that layer
/// stamped on every trigger/callback so unload can retract the exact slice.
///
/// `registrations` (the generic `on(..)` set) has no runtime consumer — it exists
/// for the load-time cross-reference pass, which already ran. Deadline handlers
/// are consumed by the applier immediately before this merge, when it arms the
/// layer's deadline rows at the activation tick. `content_hash` stays whatever
/// the base world set because the layer's set binds a save through its own
/// `<layer path>#scripts` ledger record.
pub(crate) fn merge_layer_scripts(
    layer_path: &str,
    compiled: crate::world::script::load::CompiledScripts,
    runtime: &mut WorldContentRuntime,
    script_runtime: &mut WorldScriptRuntime,
) -> Vec<String> {
    let mut referenced_units: Vec<String> = Vec::new();
    for (unit_path, ast) in compiled.asts {
        referenced_units.push(unit_path.clone());
        script_runtime
            .ast_owners
            .entry(unit_path.clone())
            .or_default()
            .insert(Some(layer_path.to_string()));
        if let std::collections::btree_map::Entry::Vacant(slot) =
            script_runtime.asts.entry(unit_path)
        {
            slot.insert(ast);
        }
    }

    // Registrations belong to the world that declared the unit, not to the AST
    // path. Two layers sharing one sibling script receive distinct scoped
    // triggers while retaining one compiled AST.
    script_runtime.triggers.extend(compiled.script_triggers);
    merge_script_triggers(runtime, script_runtime, Some(layer_path));

    referenced_units
}

/// Startup system: queue all `extra_worlds` paths from the loaded `WorldConfig`
/// as `LoadWorld` commands so they are merged into the runtime on the first frame.
///
/// Runs after `init_world_runtime` in the Startup chain. Each path is pushed
/// into `PendingWorldLayerChanges` rather than applied directly so the same
/// `apply_world_layer_changes` path handles both startup and trigger-fired loads.
pub(crate) fn load_extra_worlds(
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
    local_ship: Query<&crate::entities::spawner::EntityUuid, With<crate::server_app::LocalShip>>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {
    if !objectives.0.is_dirty() {
        return;
    }

    let objectives_snap = objectives.0.snapshots_for(
        local_ship
            .iter()
            .next()
            .map(|uuid| uuid.0.as_str())
            .unwrap_or(""),
    );

    outbox.push_reliable((
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
/// It writes `None` rather than a reading of its own because it cannot know
/// which clock it is standing on. Bevy applies a `NextState<GamePhase>` write
/// at whichever `StateTransition` site comes first; `Time` resolves to
/// `Time<Fixed>` at the fixed-schedule site `register_fixed_state_transition`
/// installs and to `Time<Virtual>` at the frame-level one, and those two clocks
/// disagree by up to one timestep — so a reading taken here would make the
/// schedule a function of which path started the mission and of frame pacing.
/// Deferring the reading to [`anchor_mission_clock`], which only ever runs
/// inside a fixed step, keeps it on one clock.
///
/// Since issue #1121's fix round every *production* start path writes from
/// `FixedUpdate`: the lobby countdown, headless and native-host auto-start, and
/// `auto_transition_from_loading`, which moved there for the #907 reason this
/// paragraph describes. That makes the hazard harder to reach, not gone — a
/// bare-`App` fixture or a test driver writing the phase from a frame schedule
/// still lands on the frame-level site, and the deferral is what keeps the
/// mission clock right for those too.
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
/// Private to complete script-call application: the action dispatcher does not
/// own the callback queue, and callers must not replay this part separately.
fn apply_deadline_changes(
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
fn apply_commitment_changes(
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
    if buffer.0.is_empty()
        && runtime.pending_delayed_actions.is_empty()
        && runtime.pending_gm_event_fires.is_empty()
        && runtime.pending_gm_spawns.is_empty()
        && runtime.pending_gm_despawns.is_empty()
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
            let action = match crate::gm_spawn::palette_entry(&runtime.gm_palette, &pending.palette)
            {
                Some(entry) => crate::gm_spawn::spawn_action(&pending, entry),
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
        let fired = runtime.triggers.evaluate(
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
        );

        // An armed Skip whose event no longer exists cannot ever be honoured,
        // and authoritative state must not accumulate it — the Fire pass's rule
        // below, through the one `live_event_ids` both levers share. Nothing
        // else drops a Skip: a spent once-only event can be re-armed by
        // `reset_trigger`, and a `when` that reads false is a moment, not an
        // answer.
        if pass == 1 && !runtime.pending_gm_event_skips.is_empty() {
            let live = crate::gm_event::live_event_ids(&runtime.trigger_states);
            runtime
                .pending_gm_event_skips
                .retain(|id| live.contains(id));
        }

        // The GM's armed Fires (issue #1301). Only in the first pass: a Fire is
        // one authored occurrence, and letting it re-enter a chaining pass
        // would let one press run a repeatable handler several times in a tick.
        //
        // Iterated over `trigger_states` in table order rather than over the
        // pending set, so the intra-tick order of a manual fire is the same
        // authored order an automatic one has, on every peer. Entries leave the
        // set when they are consumed by an actual firing, or when the event
        // they name can no longer take one — a spent once-only trigger, or an
        // id whose layer has been unloaded. Everything else (a `when` that
        // currently reads false, a cooldown that has not elapsed) deliberately
        // KEEPS the arm, so a Fire during a suppressed moment lands when the
        // moment arrives instead of being silently dropped.
        if pass == 1 && !runtime.pending_gm_event_fires.is_empty() {
            let mut consumed: Vec<String> = Vec::new();
            for (idx, origin) in trigger_origins.iter().enumerate() {
                let Some(event_id) = crate::gm_event::state_event_id(&runtime.trigger_states[idx])
                else {
                    continue;
                };
                if !runtime.pending_gm_event_fires.contains(&event_id) {
                    continue;
                }
                if !crate::world::content::manual_fire_is_still_live(&runtime.trigger_states[idx]) {
                    consumed.push(event_id);
                    continue;
                }
                let (flag_chain, _) = layered_flag_chain_with_paths(
                    origin.as_deref(),
                    &runtime.flags,
                    world_layers.layer_map.as_deref(),
                );
                if let Some(ft) = crate::world::content::fire_manual_trigger(
                    &mut runtime.trigger_states[idx],
                    &flag_chain,
                    current_elapsed,
                ) {
                    consumed.push(event_id);
                    fired.push((idx, ft));
                }
            }
            // An armed id that names no live trigger at all cannot ever be
            // honoured, and authoritative state must not accumulate it.
            let live = crate::gm_event::live_event_ids(&runtime.trigger_states);
            runtime
                .pending_gm_event_fires
                .retain(|id| live.contains(id) && !consumed.contains(id));
        }

        if fired.is_empty() {
            break;
        }

        let mut next_events: Vec<WorldEvent> = Vec::new();
        for fired in fired {
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
                    // The store chain THIS handler reads through (issue #1045):
                    // its own layer first, then outward to the base world — the
                    // same walk its `when` predicates evaluate against and the
                    // same one `scope_scripted_flag_write` resolves its writes
                    // through, so a handler cannot write somewhere it cannot read
                    // back from. Snapshotted by value because the borrow of
                    // `layer_map` must not survive into `apply_script_commands`,
                    // which takes it mutably; one entry (`[base]`) for a
                    // base-world handler, which is what every shipped world has.
                    let handler_flag_chain: Vec<crate::world::flags::FlagStore> =
                        layered_flag_chain(
                            ft.origin_layer.as_deref(),
                            &runtime.flags,
                            world_layers.layer_map.as_deref(),
                        )
                        .into_iter()
                        .cloned()
                        .collect();
                    // Split `WorldScriptRuntime` into disjoint field borrows so
                    // the one `&self` call takes `&mut budget` and `&ast` at once.
                    // `call` returns owned `CallEffects`, so no
                    // `WorldScriptRuntime` borrow survives into the apply.
                    let effects = {
                        let WorldScriptRuntime {
                            host, asts, budget, ..
                        } = &mut *sr;
                        match asts.get(&h.script_path) {
                            Some(ast) => Some(host.call_scoped(
                                budget,
                                &script_clock,
                                ast,
                                &h.script_path,
                                &h.fn_name,
                                &handler_flag_chain,
                                &runtime.deadlines,
                                &runtime.commitments,
                                &runtime.evidence,
                                ft.origin_layer.as_deref(),
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
                        apply_script_call(
                            effects,
                            ScriptCallContext {
                                log_ctx: "tick_trigger_pipeline (script)",
                                clock: script_clock,
                                mission_clock_anchored: elapsed_secs.is_some(),
                                origin_layer: ft.origin_layer.clone(),
                                entity_name: ft.entity_name.clone(),
                            },
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
fn scope_scripted_flag_write(
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
        commands: immediate,
        delayed,
        callbacks,
        comms_opens,
        deadline_changes,
        commitment_changes,
    } = call;
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
    );
    if queue_events {
        runtime.pending_world_events.extend(pending_events);
    }
    if context.mission_clock_anchored {
        runtime.pending_delayed_actions.extend(delayed);
    }
    script_runtime.pending_callbacks.extend(callbacks);
    script_runtime.pending_comms_opens.extend(comms_opens);
    apply_deadline_changes(
        &deadline_changes,
        &mut runtime.deadlines,
        &mut script_runtime.pending_callbacks,
        context.clock.tick,
        context.clock.tick_hz,
    );
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
fn apply_script_commands(
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
) {
    for eff in commands_in {
        match eff {
            // A resolved command (the M1 set + flag writes): applied directly, as
            // before. A `MutateFlag` gets its transition event previewed here (it
            // was pushed onto the sink DIRECTLY by `ctx.flags.*`, bypassing
            // `dispatch_action`'s transition step), so a scripted flag write chains
            // a downstream `on_flag_set` exactly as a declarative `set_flag` does.
            BufferedEffect::Cmd(mut cmd) => {
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
    condition: Option<ResMut<'w, EffectQueue<crate::infrastructure::ConditionAdjustment>>>,
    capacity: Option<ResMut<'w, EffectQueue<crate::infrastructure::CapacityAdjustment>>>,
    civilian_orders: Option<ResMut<'w, EffectQueue<crate::civilian::PendingCivilianOrder>>>,
    power_orders:
        Option<ResMut<'w, EffectQueue<crate::modifiers::power_system::PendingGroupPower>>>,
    narrative: Option<ResMut<'w, EffectQueue<crate::core::narrative::NarrativeRequest>>>,
    computer_message:
        Option<ResMut<'w, EffectQueue<crate::core::computer_message::ComputerMessageRequest>>>,
    report_rows: Option<ResMut<'w, EffectQueue<crate::core::report::ReportRow>>>,
    condition_fallback: Local<'s, Vec<crate::infrastructure::ConditionAdjustment>>,
    capacity_fallback: Local<'s, Vec<crate::infrastructure::CapacityAdjustment>>,
    civilian_orders_fallback: Local<'s, Vec<crate::civilian::PendingCivilianOrder>>,
    power_orders_fallback: Local<'s, Vec<crate::modifiers::power_system::PendingGroupPower>>,
    narrative_fallback: Local<'s, Vec<crate::core::narrative::NarrativeRequest>>,
    computer_message_fallback:
        Local<'s, Vec<crate::core::computer_message::ComputerMessageRequest>>,
    report_rows_fallback: Local<'s, Vec<crate::core::report::ReportRow>>,
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
            ActionCmd::SetNpcDoctrine { uuid, id } => {
                commands.queue(move |world: &mut World| {
                    use bevy::ecs::system::RunSystemOnce;
                    let _ = world
                        .run_system_once_with(crate::gm_npc::apply_scenario_command, (uuid, id));
                });
            }
            cmd @ (ActionCmd::AddObjective { .. }
            | ActionCmd::CompleteObjective { .. }
            | ActionCmd::FailObjective { .. }) => {
                crate::gm_objective::apply_command(
                    &mut objectives.0,
                    cmd,
                    balance_events.as_deref_mut(),
                    layer_map.as_deref_mut(),
                );
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
            ActionCmd::SetGroupPower {
                entity,
                group,
                level,
            } => {
                let Some(uuid) = runtime.name_to_uuid.get(&entity).cloned() else {
                    bevy::log::warn!(
                        "{log_ctx}: SetGroupPower: no entity named '{entity}' in this \
                         world — ignoring"
                    );
                    continue;
                };
                effects
                    .power_orders
                    .push(crate::modifiers::power_system::PendingGroupPower { uuid, group, level });
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
                        crate::gm_despawn::remove_entity(world, ent);
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
        let callback_origin = call.origin_layer.clone();
        let callback_flag_chain: Vec<crate::world::flags::FlagStore> = layered_flag_chain(
            callback_origin.as_deref(),
            &runtime.flags,
            world_layers.layer_map.as_deref(),
        )
        .into_iter()
        .cloned()
        .collect();
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
                Some(ast) => Some(host.call_scoped(
                    budget,
                    &script_clock,
                    ast,
                    &call.script_path,
                    &call.fn_name,
                    // The callback's captured layer chain; root callbacks have
                    // the ordinary one-entry base chain.
                    &callback_flag_chain,
                    &runtime.deadlines,
                    &runtime.commitments,
                    &runtime.evidence,
                    callback_origin.as_deref(),
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
        apply_script_call(
            effects,
            ScriptCallContext {
                log_ctx: "tick_script_callbacks",
                clock: script_clock,
                mission_clock_anchored: elapsed_secs.is_some(),
                origin_layer: callback_origin.clone(),
                entity_name: None,
            },
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
/// spawned with a `ShipModifiers` Component (`src/entities/spawner.rs`
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

// ── Pending scenario load system ─────────────────────────────────────────────

/// Bevy system: drain `PendingScenarioLoad`, reading and recording each world
/// TOML — and merging NOTHING.
///
/// It once merged the additively-loaded world's trigger states and comms
/// templates into the live `WorldContentRuntime`. Issue #985 deleted both
/// front-ends, so what survives is: read the TOML (recording it into the content
/// ledger), parse it to prove it parses, and mark the path loaded so a duplicate
/// is not re-read. Nothing enqueues into the queue it drains, either — see
/// [`PendingScenarioLoad`], and `apply_world_layer_changes` for the path a
/// supporting world that wants to CONTRIBUTE something takes instead.
///
/// On WASM the TOML string is not available at runtime (JS pre-fetches only the
/// initial world), so we push paths into the WASM-side pending-world queue and
/// the implementation returns early until the JS bridge delivers the TOML via
/// `wasm_push_world_toml`. On native targets `std::fs::read_to_string` is used.
///
/// It does NOT re-anchor the mission clock, and would not even if it merged
/// again: this is additive to a LIVE runtime, so the base world's in-flight
/// `on_timer` triggers and `action_delays` are still scheduled against the
/// running clock, and rewinding it here would postpone every one of them by
/// however long the mission had been going. A genuinely fresh scenario is
/// reached through the lobby instead, and `arm_mission_clock` re-anchors there.
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
        // is owned above).
        //
        // The compiled set is DISCARDED, and stays discarded after issue #1045 gave
        // supporting worlds their scripts back: that slice wired the LAYER path
        // (`apply_world_layer_changes`), which has a `WorldLayerMap` entry to hang a
        // layer's ASTs and origin-tagged trigger states off and so can retract them
        // at `UnloadWorld`. This path has neither — a `PendingScenarioLoad` merge is
        // one-way and untracked — so merging a script here would be a set nothing
        // could ever take back out. Nothing enqueues into that queue today; if
        // something ever does, it wants the layer path, not a second merge.
        // `NoSiblingScripts` for the same reason: there is nothing here to resolve
        // a sibling FOR.
        let reader = MemoryReader::new([(path.clone(), toml_str)]);
        let request = LoadRequest::new(
            path.as_str(),
            &reader,
            &crate::world::script::load::NoSiblingScripts,
            LoadPolicy::Merge,
        );
        match load(request) {
            Ok(loaded) => {
                // The compiled set is discarded (see above), but its ledger digest
                // is not: `load_world_scripts` used to write that itself and now
                // returns it (issue #1241), so applying it here keeps the ledger
                // byte-identical to before the lift. The TOML `records` are
                // dropped for the reason the comment above gives — this function
                // already recorded that text through `load_scenario_toml`.
                for digest in &loaded.ledger.digests {
                    digest.apply();
                }
            }
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

// ── World layer system (LoadWorld / UnloadWorld) ──────────────────────────────

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

/// Bring one already-evaluated layer into the world: merge its scripts, register
/// its names, spawn its entities, and record it in the `WorldLayerMap`.
///
/// Shared by both routes into `apply_world_layer_changes`' Loaded case — the
/// same-tick one and the [`WorldLayerChange::DeferredApply`] one a tick later —
/// so a scripted layer and a scriptless one are brought in by identical code and
/// the deferral cannot drift into a second implementation.
///
/// `script_runtime` is `None` only for a layer that authored no script; the
/// callers refuse a scripted layer with no runtime before reaching here.
#[allow(clippy::too_many_arguments)]
fn apply_loaded_layer(
    path: &str,
    loader_path: Option<String>,
    layer: crate::world::layers::LoadedLayer,
    commands: &mut Commands,
    layer_map: &mut WorldLayerMap,
    runtime: &mut WorldContentRuntime,
    script_runtime: Option<&mut WorldScriptRuntime>,
    id_mint: Option<&crate::world_id::WorldIdMint>,
    now_tick: u64,
    root_tick_hz: f32,
) {
    let crate::world::layers::LoadedLayer {
        mut name_to_uuid_inserts,
        mut scenario_config,
        emit_world_loaded,
        scripts,
    } = layer;

    if scenario_config.gm_npc_doctrine_palette.iter().any(|entry| {
        runtime
            .gm_npc_doctrine_palette
            .iter()
            .any(|live| live.id == entry.id)
    }) {
        bevy::log::error!("layer {path} has a duplicate GM NPC doctrine palette id");
        return;
    }
    if scenario_config.gm_objective_palette.iter().any(|entry| {
        runtime
            .gm_objective_palette
            .iter()
            .any(|live| live.id == entry.id)
    }) {
        bevy::log::error!("layer {path} has a duplicate GM Objective palette id");
        return;
    }

    // A named layer entity is authored identity, not one incarnation of the
    // layer. Unload deliberately leaves the live name registry intact, so a
    // later load can restore the same UUID instead of exposing a removal and
    // reappearance as two unrelated contacts (issue #1296). Keep the parsed
    // config and the registrations in lockstep: the former is what spawning
    // reads, while the latter is what the live runtime records below. Anonymous
    // layer entities have no entry here and retain their mint-on-load policy.
    for (name, uuid) in &mut name_to_uuid_inserts {
        let Some(existing_uuid) = runtime.name_to_uuid.get(name).cloned() else {
            continue;
        };
        *uuid = existing_uuid.clone();
        scenario_config
            .name_to_uuid
            .insert(name.clone(), existing_uuid);
    }

    // Merge the layer's compiled `[script]` set into the live script runtime
    // (issue #1045). Script-free layers take the `None` branch without changing
    // the live runtime.
    let script_units = match (scripts, script_runtime) {
        (Some(compiled), Some(sr)) => {
            let calls = runtime.deadlines.arm_scoped(
                &scenario_config.deadlines,
                &compiled.deadline_handlers,
                now_tick,
                root_tick_hz,
                Some(path),
            );
            sr.pending_callbacks.extend(calls);
            merge_layer_scripts(path, compiled, runtime, sr)
        }
        (Some(_), None) => {
            // Refused by both callers before this point; belt to that brace.
            bevy::log::error!(
                target: "world",
                "apply_loaded_layer: layer {path} carries scripts with no runtime to \
                 merge into; dropping them"
            );
            Vec::new()
        }
        (None, _) => Vec::new(),
    };

    // Register the layer's named entities in the live name_to_uuid map.
    for (name, uuid) in name_to_uuid_inserts {
        runtime.name_to_uuid.insert(name, uuid);
    }

    // Spawn the layer's [[entity]] blocks into the ECS. On native the global
    // config cache is always empty (no WASM pre-load step), so we build a local
    // cache by reading each referenced template from disk. WASM uses the
    // pre-loaded global cache as normal.
    let config_cache = build_layer_config_cache(&scenario_config);
    let spawned_entities = spawn_immediate_entities_internal(
        commands,
        &scenario_config,
        &config_cache,
        Some(&runtime.flags),
        id_mint,
    );
    // Stamp each entity's origin layer (issue #891 review finding 1) so
    // `entity_flag_chain` can read it in O(1) instead of scanning `WorldLayerMap`
    // for it later — the second of the two spawn sites.
    for &spawned in &spawned_entities {
        commands
            .entity(spawned)
            .insert(EntityOriginLayer(path.to_string()));
    }

    let delayed_unload_resolve = matches!(
        scenario_config.delayed_unload_policy,
        crate::world::config::DelayedUnloadPolicy::Resolve
    );
    let activation_order = layer_map
        .0
        .values()
        .filter(|layer| layer.is_active)
        .map(|layer| layer.activation_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    layer_map.0.insert(
        path.to_string(),
        WorldRuntime {
            is_active: true,
            activation_order,
            spawned_entities,
            anchors: scenario_config.anchors.clone(),
            flags: crate::world::flags::FlagStore::new(),
            loader_path,
            owned_objective_ids: Vec::new(),
            delayed_unload_resolve,
            script_units,
        },
    );

    for mut entry in scenario_config.gm_objective_palette.clone() {
        entry.origin_layer = Some(path.to_string());
        runtime.gm_objective_palette.push(entry);
    }
    for mut entry in scenario_config.gm_npc_doctrine_palette.clone() {
        entry.origin_layer = Some(path.to_string());
        runtime.gm_npc_doctrine_palette.push(entry);
    }

    // Expose WorldLoaded only after every part of activation is live: ASTs,
    // handlers, deadlines, entities, flags and the layer map.
    if emit_world_loaded {
        runtime.pending_world_events.push(WorldEvent::WorldLoaded);
    }
}

/// How many consecutive drains a layer may answer `TomlUnavailable` before it is
/// refused outright (issue #1045).
///
/// Generous on purpose: the legitimate case is a browser fetch in flight, which
/// resolves in a handful of frames, so this is a backstop against a source that
/// can never arrive rather than a timeout anyone should reach. At the fixed
/// timestep it is a few seconds of retrying before the log says so.
const MAX_TOML_RETRIES: u32 = 300;

/// Snapshot the doctrine-anchor namespace visible to one layer evaluation.
///
/// Rebuilt immediately before every load decision, rather than once per drain:
/// a scriptless layer applied earlier in the same authored batch is already in
/// `layer_map` and may legitimately provide an anchor to the next layer. Failed
/// sentinels and inactive entries contribute nothing.
fn layer_validation_context<'a>(
    base_world: Option<&crate::world::config::WorldConfig>,
    layer_map: &WorldLayerMap,
    template_loader: &'a dyn crate::entities::loader::TemplateLoader,
    fragment_source: &'a dyn crate::entities::include_resolve::FragmentSource,
) -> LayerValidationContext<'a> {
    let root_anchors = base_world
        .into_iter()
        .flat_map(|world| world.anchors.keys().cloned());
    let layer_anchors = layer_map
        .0
        .values()
        .filter(|layer| layer.is_active)
        .flat_map(|layer| layer.anchors.keys().cloned());
    LayerValidationContext::new(
        template_loader,
        fragment_source,
        root_anchors.chain(layer_anchors),
    )
}

/// Bevy system: drain `PendingWorldLayerChanges` and apply each `LoadWorld` or
/// `UnloadWorld` command to `WorldLayerMap` and `WorldContentRuntime`.
///
/// `LoadWorld` parses the TOML, spawns the layer's entities, merges its compiled
/// `[script]` set into the live runtime, and stores a `WorldRuntime` snapshot
/// keyed by path so `UnloadWorld` can reverse it.
///
/// `UnloadWorld` removes the stored snapshot, despawns the layer's entities, and
/// retracts exactly the scenario logic it brought: the trigger states origin-tagged
/// with its path (each with its parallel `handlers` entry — see
/// [`crate::world::trigger_registry::WorldTriggerRegistry::remove_layer`]), the AST units it added, and any deadline
/// declaration or queued `after(..)` callback keyed to one of those units.
///
/// # The script runtime is created on demand, one tick early (issue #1045)
///
/// `WorldScriptRuntime` is absent for a script-free base world, so a layer that
/// brings the session's first `[script]` block has nowhere to merge into. This
/// system inserts an empty runtime and RE-QUEUES that load — the same answer the
/// `TomlUnavailable` arm gives a world that is not ready yet — rather than merging
/// into one it created on the spot.
///
/// The reason is ordering, not tidiness. A `Commands` insert lands at the
/// schedule's sync point, which may fall AFTER `tick_trigger_pipeline` has already
/// run this tick. Merging in the body would then publish the layer's trigger
/// states (on `WorldContentRuntime`, written directly) and its `WorldLoaded` event
/// a whole tick before the `handlers` that describe them — so the pipeline would
/// evaluate each `on_world_loaded` against an absent runtime, latch it `fired`,
/// and the layer's opening handler would never run at all. Waiting a tick means
/// the paired registrations become visible only with their execution runtime.
///
/// The re-queued item is a [`WorldLayerChange::DeferredApply`] carrying the
/// EVALUATED layer, not a fresh `Load`. Re-evaluating would reparse the layer and
/// mint different `[[entity]]` UUIDs. Carrying the outcome means those UUIDs are
/// minted exactly once, so the deferral costs a tick and nothing else.
fn apply_world_layer_changes(
    mut commands: Commands,
    mut pending: ResMut<PendingWorldLayerChanges>,
    mut layer_map: ResMut<WorldLayerMap>,
    mut runtime: ResMut<WorldContentRuntime>,
    // The live script runtime a layer's compiled set merges into, and retracts
    // from at unload (issue #1045). `Option` because a script-free world has
    // none — every shipped world, until one authors a scripted layer.
    mut script_runtime: Option<ResMut<WorldScriptRuntime>>,
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
    // Layer-owned objective cleanup on unload (issue #751). `Option` so bare
    // `App` fixtures without an `ObjectiveManagerRes` still run the loader.
    mut objectives: Option<ResMut<ObjectiveManagerRes>>,
    // Related runtime state to prune when a layer's objectives are removed
    // (issue #752): a captain priority boost pointing at a removed objective,
    // and any per-ship route cursor keyed to it.
    mut captain_boost: Option<ResMut<crate::server_app::CaptainPriorityBoost>>,
    mut objective_cursors_q: Query<&mut crate::ai::server::ObjectiveCursors>,
    // Consecutive `TomlUnavailable` answers per path, so a source that never
    // arrives is refused rather than re-queued forever (issue #1045). A `Local`
    // because it is scheduling bookkeeping, not world state: nothing reads it,
    // no digest folds it, and a fresh app starts it empty.
    mut toml_retries: Local<HashMap<String, u32>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    base_world: Option<Res<crate::world::config::WorldConfig>>,
    // A layer-owned dialogue is executable scenario state just like its queued
    // callback. Keep the comms surfaces optional for bare loader fixtures, but
    // retire all of them atomically when the owner unloads.
    mut comms_runtime: Option<ResMut<crate::comms::server::CommsRuntime>>,
    mut comms_inbox: Option<ResMut<crate::comms::server::CommsInboxRes>>,
    mut on_screen_message: Option<ResMut<crate::comms::server::OnScreenMessage>>,
) {
    if pending.0.is_empty() {
        return;
    }

    let changes: Vec<WorldLayerChange> = pending.0.drain(..).collect();
    let now_tick = sim_tick.as_deref().map_or(0, |tick| tick.0);
    let root_tick_hz = base_world
        .as_deref()
        .map_or(SchedClock::ZERO.tick_hz, |world| world.global.sim_tick_hz);

    // Resolves a layer's top-level `script = "wave.rhai"` sibling. Built once per
    // drain: `world::layers` keeps the read injected so the decision stays pure,
    // and this applier is the impure half that owns filesystem / bridge access.
    let script_resolver = crate::entities::config_cache::production_script_resolver();
    let template_loader = crate::entities::loader::WasmTemplateLoader;
    let fragment_source = crate::entities::include_resolve::HostFragmentSource;

    for change in changes {
        match change {
            WorldLayerChange::Load { path, loader_path } => {
                // Decisions (dedup / requeue / parse handling / origin
                // tagging / name→UUID assignment) are pure (`world::layers`);
                // this applier resolves the TOML (I/O), spawns entities,
                // merges comms, and mutates the layer map. The dedup check
                // happens before the TOML read so a duplicate never touches
                // the WASM fetch queue.
                // A layer whose apply is DEFERRED counts as loaded for dedup
                // purposes even though it has no map entry yet (issue #1045).
                // Without this, two `Load`s for the same path in one drain — a
                // duplicated `extra_worlds` entry, or a script calling
                // `load_world` twice in a tick — would each evaluate and each
                // defer, and the second apply would overwrite the first's map
                // entry, leaking its spawned entities. It also keeps the second
                // one off the world source, which on wasm can only be read once.
                let deferred_already = pending.0.iter().any(|queued| {
                    matches!(
                        queued,
                        WorldLayerChange::DeferredApply { path: p, .. }
                            | WorldLayerChange::AwaitingScript { path: p, .. }
                            if p == &path
                    )
                });
                let already_loaded = layer_map.0.contains_key(&path) || deferred_already;
                #[cfg(target_arch = "wasm32")]
                if let crate::entities::config_cache::WorldFetchState::Failed(message) =
                    crate::entities::config_cache::world_fetch_state(&path)
                {
                    bevy::log::error!(
                        target: "world",
                        "apply_world_layer_changes: refusing {path} atomically after fetch \
                         failure: {message}"
                    );
                    layer_map.0.insert(path, WorldRuntime::default());
                    continue;
                }
                let toml_str = if already_loaded {
                    None
                } else {
                    load_scenario_toml(&path)
                };
                #[cfg(target_arch = "wasm32")]
                if let Some(toml) = toml_str.as_deref() {
                    if let Some(script_path) =
                        crate::world::script::load::declared_sibling_script_path(&path, toml)
                    {
                        use crate::entities::config_cache::WorldFetchState;
                        match crate::entities::config_cache::world_fetch_state(&script_path) {
                            WorldFetchState::Ready(_) => {}
                            WorldFetchState::Failed(message) => {
                                bevy::log::error!(
                                    target: "world",
                                    "apply_world_layer_changes: refusing {path} atomically: \
                                     sibling script {script_path} failed to fetch: {message}"
                                );
                                layer_map.0.insert(path, WorldRuntime::default());
                                continue;
                            }
                            WorldFetchState::NotRequested | WorldFetchState::Pending => {
                                crate::entities::config_cache::request_world_fetch(
                                    script_path.clone(),
                                );
                                pending.0.push(WorldLayerChange::AwaitingScript {
                                    path,
                                    loader_path,
                                    toml: toml.to_string(),
                                    script_path,
                                });
                                continue;
                            }
                        }
                    }
                }
                let validation = layer_validation_context(
                    base_world.as_deref(),
                    &layer_map,
                    &template_loader,
                    &fragment_source,
                );
                let result = evaluate_layer_load(
                    &path,
                    already_loaded,
                    toml_str.as_deref(),
                    &script_resolver,
                    &validation,
                    || {
                        crate::world_id::mint_id_with(
                            id_mint.as_deref(),
                            crate::world_id::IdNamespace::Entity,
                        )
                    },
                );
                for warning in &result.warnings {
                    bevy::log::error!("apply_world_layer_changes: {warning}");
                }
                // The ledger writes the evaluation gathered (issue #1241): the
                // layer's compiled-script digest, which the loader used to write
                // itself. Applied HERE — right after the evaluation, before the
                // outcome is acted on and before any deferral — so it lands at the
                // same moment the eager write did, on every outcome the eager write
                // covered. Empty for a script-free layer and for every outcome
                // that never reached a compile.
                result.ledger.apply();
                match result.outcome {
                    LayerLoadOutcome::AlreadyLoaded => {
                        // De-duplicate, no-op.
                        toml_retries.remove(&path);
                        continue;
                    }
                    LayerLoadOutcome::TomlUnavailable => {
                        // WASM: re-queue until the fetch completes — but not
                        // forever. A world source can become permanently
                        // unreadable mid-session (the browser's pending map is
                        // CONSUMED by a read and its fetch guard refuses to
                        // re-ask, which a contrived `[Load A, Unload A, Load A]`
                        // in one drain reaches), and an unbounded re-queue turns
                        // that into a silent spin with nothing in the log. Refuse
                        // the layer loudly once the retries are plainly not going
                        // anywhere, the same way a broken file is refused.
                        let attempts = toml_retries.entry(path.clone()).or_insert(0);
                        *attempts += 1;
                        if *attempts > MAX_TOML_RETRIES {
                            bevy::log::error!(
                                target: "world",
                                "apply_world_layer_changes: giving up on {path} after \
                                 {MAX_TOML_RETRIES} drains with no world source; the layer \
                                 is refused and will not be retried"
                            );
                            toml_retries.remove(&path);
                            layer_map.0.insert(path, WorldRuntime::default());
                        } else {
                            pending.0.push(WorldLayerChange::Load { path, loader_path });
                        }
                    }
                    LayerLoadOutcome::ParseFailed => {
                        // Insert an empty entry so we don't retry a broken file.
                        toml_retries.remove(&path);
                        layer_map.0.insert(path, WorldRuntime::default());
                    }
                    LayerLoadOutcome::Loaded(layer) => {
                        toml_retries.remove(&path);
                        // A scripted layer needs a live `WorldScriptRuntime` to merge
                        // into, and a script-free base world has none. Insert an empty
                        // one and re-queue the EVALUATED layer rather than merging into
                        // one created on the spot — see the system docs for why the
                        // extra tick is the whole point, and why what is re-queued is
                        // the outcome rather than the load. Script-free layers skip
                        // this branch entirely.
                        if layer.scripts.is_some() && script_runtime.is_none() {
                            commands.insert_resource(WorldScriptRuntime::empty());
                            pending.0.push(WorldLayerChange::DeferredApply {
                                path,
                                loader_path,
                                layer,
                            });
                            continue;
                        }
                        apply_loaded_layer(
                            &path,
                            loader_path,
                            *layer,
                            &mut commands,
                            &mut layer_map,
                            &mut runtime,
                            script_runtime.as_deref_mut(),
                            id_mint.as_deref(),
                            now_tick,
                            root_tick_hz,
                        );
                    }
                }
            }
            WorldLayerChange::AwaitingScript {
                path,
                loader_path,
                toml,
                script_path,
            } => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let _ = (path, loader_path, toml, script_path);
                    unreachable!("AwaitingScript is produced only by the wasm fetch edge");
                }
                #[cfg(target_arch = "wasm32")]
                {
                    use crate::entities::config_cache::WorldFetchState;
                    match crate::entities::config_cache::world_fetch_state(&script_path) {
                        WorldFetchState::NotRequested | WorldFetchState::Pending => {
                            crate::entities::config_cache::request_world_fetch(script_path.clone());
                            pending.0.push(WorldLayerChange::AwaitingScript {
                                path,
                                loader_path,
                                toml,
                                script_path,
                            });
                        }
                        WorldFetchState::Failed(message) => {
                            bevy::log::error!(
                                target: "world",
                                "apply_world_layer_changes: refusing {path} atomically: sibling \
                                 script {script_path} failed to fetch: {message}"
                            );
                            layer_map.0.insert(path, WorldRuntime::default());
                        }
                        WorldFetchState::Ready(_) => {
                            let validation = layer_validation_context(
                                base_world.as_deref(),
                                &layer_map,
                                &template_loader,
                                &fragment_source,
                            );
                            let result = evaluate_layer_load(
                                &path,
                                layer_map.0.contains_key(&path),
                                Some(&toml),
                                &script_resolver,
                                &validation,
                                || {
                                    crate::world_id::mint_id_with(
                                        id_mint.as_deref(),
                                        crate::world_id::IdNamespace::Entity,
                                    )
                                },
                            );
                            for warning in &result.warnings {
                                bevy::log::error!("apply_world_layer_changes: {warning}");
                            }
                            result.ledger.apply();
                            match result.outcome {
                                LayerLoadOutcome::AlreadyLoaded => {}
                                LayerLoadOutcome::TomlUnavailable => {
                                    unreachable!("retained layer TOML cannot become unavailable")
                                }
                                LayerLoadOutcome::ParseFailed => {
                                    layer_map.0.insert(path, WorldRuntime::default());
                                }
                                LayerLoadOutcome::Loaded(layer) => {
                                    if layer.scripts.is_some() && script_runtime.is_none() {
                                        commands.insert_resource(WorldScriptRuntime::empty());
                                        pending.0.push(WorldLayerChange::DeferredApply {
                                            path,
                                            loader_path,
                                            layer,
                                        });
                                    } else {
                                        apply_loaded_layer(
                                            &path,
                                            loader_path,
                                            *layer,
                                            &mut commands,
                                            &mut layer_map,
                                            &mut runtime,
                                            script_runtime.as_deref_mut(),
                                            id_mint.as_deref(),
                                            now_tick,
                                            root_tick_hz,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            WorldLayerChange::DeferredApply {
                path,
                loader_path,
                layer,
            } => {
                // The empty `WorldScriptRuntime` queued alongside this on the previous
                // tick is visible now, so the merge and the `WorldLoaded` it fires from
                // land in this one body. Nothing is re-read or re-evaluated.
                if layer.scripts.is_some() && script_runtime.is_none() {
                    // Cannot happen: the insert and this queue entry were written in
                    // the same body, and commands are applied before the next run.
                    // Refuse the layer rather than drop its logic silently or defer
                    // again (which is how a queue starts spinning).
                    bevy::log::error!(
                        target: "world",
                        "apply_world_layer_changes: deferred layer {path} still has no \
                         WorldScriptRuntime to merge into; refusing it"
                    );
                    layer_map.0.insert(path, WorldRuntime::default());
                    continue;
                }
                apply_loaded_layer(
                    &path,
                    loader_path,
                    *layer,
                    &mut commands,
                    &mut layer_map,
                    &mut runtime,
                    script_runtime.as_deref_mut(),
                    id_mint.as_deref(),
                    now_tick,
                    root_tick_hz,
                );
            }
            WorldLayerChange::Unload(path) => {
                // Cancel any load for this path still sitting in the queue, so
                // `[Load A, Unload A]` in one drain nets to NOT LOADED (issue #1045).
                // Two kinds can be waiting: a `DeferredApply` this drain just queued
                // for a scripted layer, and a `Load` re-queued because its TOML has
                // not arrived. Without this the unload finds no map entry, no-ops,
                // and the load lands a tick later — an ordering inversion that turns
                // "load it then take it away" into "it is loaded".
                //
                // Cancelling a `DeferredApply` leaves the empty
                // `WorldScriptRuntime` its `Load` already queued for insertion
                // resident, deliberately: the insert is idempotent, an empty
                // runtime changes no behaviour (every read of it finds nothing),
                // and the next scripted layer merges straight into it instead of
                // waiting a tick. Chasing the `Commands` insert back would cost
                // more than it saves.
                pending.0.retain(|queued| match queued {
                    WorldLayerChange::Load { path: p, .. }
                    | WorldLayerChange::DeferredApply { path: p, .. }
                    | WorldLayerChange::AwaitingScript { path: p, .. } => p != &path,
                    WorldLayerChange::Unload(_) => true,
                });

                let Some(layer) = layer_map.0.remove(&path) else {
                    continue; // Not loaded - no-op.
                };

                // Retire every live comms surface owned by this layer before
                // the response handler can observe it. `script_path` cannot be
                // used as ownership: two loaded layers may share one sibling
                // AST, and the survivor must retain its own dialogues while the
                // departing owner's offered choices become unaddressable.
                if let Some(comms) = comms_runtime.as_deref_mut() {
                    crate::comms::server::retire_layer_owned_dialogues(
                        &path,
                        comms,
                        comms_inbox.as_deref_mut(),
                        on_screen_message.as_deref_mut(),
                    );
                }

                // Despawn ECS entities that were spawned when this layer loaded.
                // Use try_despawn: entities may have already died (e.g. hull = 0) before the layer unloads.
                for entity in &layer.spawned_entities {
                    commands.entity(*entity).try_despawn();
                }

                // Retraction pairs rows even in fixtures with no script runtime.
                let removed = runtime.triggers.remove_layer(&path);
                if removed > 0 {
                    bevy::log::debug!(target: "world",
                        "apply_world_layer_changes: unloaded {path} retracted {removed} trigger(s)");
                }
                if let Some(sr) = script_runtime.as_deref_mut() {
                    for call in runtime.deadlines.remove_origin(&path) {
                        sr.pending_callbacks.retract(&call);
                    }
                    sr.pending_callbacks
                        .0
                        .retain(|call| call.origin_layer.as_deref() != Some(path.as_str()));
                    sr.pending_comms_opens
                        .retain(|open| open.origin_layer.as_deref() != Some(path.as_str()));
                    for unit in &layer.script_units {
                        let remove_unit = sr.ast_owners.get_mut(unit).is_some_and(|owners| {
                            owners.remove(&Some(path.clone()));
                            owners.is_empty()
                        });
                        if remove_unit {
                            sr.ast_owners.remove(unit);
                            sr.asts.remove(unit);
                        }
                    }
                    // Every callback still waiting on its fire tick has already
                    // been retracted by exact `origin_layer`; that includes named
                    // deadlines, ordinary `after(..)` calls, and callbacks nested
                    // from either one. Shared ASTs remain until their last owner.
                }

                runtime
                    .gm_objective_palette
                    .retain(|entry| entry.origin_layer.as_deref() != Some(path.as_str()));
                runtime
                    .gm_npc_doctrine_palette
                    .retain(|entry| entry.origin_layer.as_deref() != Some(path.as_str()));
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
        crate::entities::config_cache::resolved_world_source(path).or_else(|| {
            // Fire a JS fetch request if we haven't already.
            crate::entities::config_cache::request_world_fetch(path.to_string());
            None
        })
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
pub(crate) mod tests;
