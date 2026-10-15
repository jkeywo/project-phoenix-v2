//! State.
use super::*;

// -- Resources --------------------------------------------------------------

/// A process-local fixed-time anchor plus an exact previously elapsed age.
/// Only the age is persisted; a fresh host may have a younger fixed clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MissionClock {
    anchored_at: std::time::Duration,
    prior_elapsed: std::time::Duration,
}
impl MissionClock {
    pub fn new(now: std::time::Duration) -> Self {
        Self::resume(now, std::time::Duration::ZERO)
    }
    pub fn resume(now: std::time::Duration, elapsed: std::time::Duration) -> Self {
        Self {
            anchored_at: now,
            prior_elapsed: elapsed,
        }
    }
    pub fn elapsed(self, now: std::time::Duration) -> std::time::Duration {
        self.prior_elapsed
            .saturating_add(now.saturating_sub(self.anchored_at))
    }
    pub fn elapsed_secs(self, now: std::time::Duration) -> f32 {
        self.elapsed(now).as_secs_f32()
    }
}

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
    pub opportunities: Vec<crate::world::opportunities::OpportunityRecord>,
    /// Scenario-private bindings/evidence; no physical machinery copies.
    pub situations: phoenix_sim_world::world::activities::SituationInstances,
    /// Immutable authored definitions, rebuilt from accepted content.
    pub situation_definitions: Vec<phoenix_sim_world::world::activities::SituationDefinition>,
    pub situation_configs: Vec<phoenix_sim_world::world::activities::SituationConfig>,
    /// Sole saved method/release frontier; graph derives from accepted sources.
    pub plan_choice: Option<phoenix_sim_world::world::planning::PlanChoice>,
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
    /// Exact mission age rebased against this process's fixed clock. The public
    /// seconds used by authored timers are projected only after integer elapsed
    /// arithmetic, so save/resume cannot change their rounding phase.
    /// Anchored once by `anchor_mission_clock`, cleared by `arm_mission_clock`.
    pub mission_clock: Option<MissionClock>,
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
    /// [`crate::gm::actions::action::apply_due_actions`] arms it in `PreUpdate` at the
    /// grant's exact apply tick, while a `when` predicate or a cooldown can
    /// legitimately hold the entry for many ticks before the pipeline can honour
    /// it. So, like `pending_world_events` and `pending_delayed_actions`, it
    /// lives here and is snapshotted.
    pub pending_gm_event_fires: std::collections::BTreeSet<String>,
    /// The scenario-authored GM spawn palette, copied from `WorldConfig` at
    /// load (issue #1305).
    ///
    /// AUTHORED CONTENT, not run state: it lives here for `triggers`'
    /// reason — the deterministic apply-tick reducer and the trigger pipeline
    /// both need it, and neither holds `WorldConfig` — and, like a trigger's
    /// authored condition, it is answered for by `snapshot::content_digest`
    /// rather than captured or folded. A resumed world rebuilds it by replaying
    /// the same load.
    pub gm_palette: Vec<crate::world::config::GmPaletteEntry>,
    pub gm_objective_palette: Vec<crate::gm::operations::objective::ObjectivePaletteEntry>,
    pub gm_npc_doctrine_palette: Vec<crate::gm::operations::npc::NpcDoctrinePaletteEntry>,
    /// GM placements that have crossed their canonical apply boundary and are
    /// waiting for the trigger pipeline to spawn them (issue #1305).
    ///
    /// A `Vec` in canonical grant order rather than a set: two identical
    /// placements of the same palette entry are two entities, not one, and the
    /// ORDER decides which draws which uuid from the `WorldIdMint`. Genuinely
    /// cross-tick for `pending_gm_event_fires`' reason — armed in `PreUpdate`,
    /// drained in `FixedUpdate`, which a paused session never reaches — so it is
    /// snapshotted and folded.
    pub pending_gm_spawns: Vec<crate::gm::operations::spawn::PendingGmSpawn>,
    /// Canonically ordered UUID removals accepted before the fixed pipeline.
    pub pending_gm_despawns: Vec<String>,
    /// The bounded inverse data for allowed GM removals (issue #1444), and the
    /// two cross-tick arms that produce and consume it.
    ///
    /// It lives here rather than in a resource of its own because both of its
    /// arms are already this runtime's own cross-tick pattern: an accepted
    /// removal is armed in `PreUpdate` beside `pending_gm_despawns` and the
    /// capture is taken when the fixed pipeline destroys the entity, and an
    /// accepted inverse is armed the same way and executed by the same
    /// pipeline's command queue. Captured and folded with its siblings.
    pub gm_despawn_captures: crate::gm::operations::despawn_undo::GmDespawnCaptures,
    pub contact_overrides: crate::gm::operations::contact::ContactOverrides,
    pub contact_classifications: crate::gm::operations::contact::ContactClassifications,
    pub presentation: crate::gm::operations::presentation::PresentationState,
    pub contact_information: crate::gm::operations::information::ContactInformation,
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

/// Explicitly named multi-ship Objective instances. Kept beside the legacy
/// manager so worlds without instance syntax retain byte-identical behaviour.
#[derive(Resource, Default)]
pub struct ObjectiveInstanceManagerRes(pub crate::objective_instances::ObjectiveInstanceManager);

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
    /// Derived accepted authored-origin permission; rebuilt with content, never
    /// captured/folded as mutable current location or read from Live inspection.
    pub transit_origin: Option<phoenix_model::transit::TransitWorldBinding>,
    /// Parsed effective definition retained for read-only Live inspection.
    /// Runtime authority continues to live in the specialised fields below;
    /// this copy is never read by simulation systems.
    pub inspection_config: Option<Box<crate::world::config::WorldConfig>>,
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
    /// Authored names remain valid before their instance is activated.
    pub recipient_declarations: BTreeSet<crate::objective_instances::ObjectiveInstanceKey>,
    /// The runtime host that runs retained handler fns.
    pub(super) host: RuntimeHost,
    /// Retained ASTs keyed by content-relative (or virtual) path.
    pub asts: BTreeMap<String, AST>,
    /// Owners retaining each AST unit. `None` denotes the root world; layer
    /// paths are explicit so shared sibling scripts survive either unload.
    pub ast_owners: BTreeMap<String, BTreeSet<Option<String>>>,
    /// Named-function definition lines derived from the exact compiled source.
    pub function_lines: BTreeMap<String, BTreeMap<String, usize>>,
    /// Compiled registrations consumed by the trigger registry at activation.
    pub triggers: Vec<ScriptTrigger>,
    /// The per-tick operation/call budget, shared across every script call in a
    /// tick and reset when [`budget_tick`](Self::budget_tick) falls behind the
    /// current `SimTick`.
    pub(super) budget: TickBudget,
    /// The `SimTick` the current [`budget`](Self::budget) was created for.
    pub(super) budget_tick: u64,
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
            recipient_declarations: compiled.recipient_references.declarations,
            host: RuntimeHost::new(),
            asts: compiled.asts,
            ast_owners,
            function_lines: compiled.function_lines,
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
            recipient_declarations: BTreeSet::new(),
            host: RuntimeHost::new(),
            asts: BTreeMap::new(),
            ast_owners: BTreeMap::new(),
            function_lines: BTreeMap::new(),
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

/// Per-world script validation result, rebuilt at every materialization pass.
/// The shared materialization chain publishes this resource before both spawn
/// systems, including when they run on different Bevy workers. Keeping the gate
/// on the App prevents another world's compile from clearing or setting it.
/// Bare spawn helpers validate their supplied composition without borrowing a
/// different App's script result.
#[derive(Resource, Default)]
pub(crate) struct ScriptActivationGate {
    pub(super) blocked: bool,
}

impl ScriptActivationGate {
    pub(crate) fn blocks_spawn(&self, system: &str) -> bool {
        if self.blocked {
            bevy::log::error!(
                target: "world",
                "{system}: spawn blocked: world scripts failed activation; spawning zero entities"
            );
        }
        self.blocked
    }
}

/// Upper bound on `tick_trigger_pipeline`'s within-tick trigger-chaining passes.
///
/// A `set_flag` action emits a `FlagSet` event that a downstream `on_flag_set`
/// trigger can react to in the same Bevy frame, which can in turn set another
/// flag. The cap stops a pathological feedback loop from hanging the frame.
pub(super) const MAX_CHAIN_PASSES: i32 = 16;

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
                            transit_origin_supported: wr.transit_origin.is_some(),
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

/// How many consecutive drains a layer may answer `TomlUnavailable` before it is
/// refused outright (issue #1045).
///
/// Generous on purpose: the legitimate case is a browser fetch in flight, which
/// resolves in a handful of frames, so this is a backstop against a source that
/// can never arrive rather than a timeout anyone should reach. At the fixed
/// timestep it is a few seconds of retrying before the log says so.
pub(super) const MAX_TOML_RETRIES: u32 = 300;

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
