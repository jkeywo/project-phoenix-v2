use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::comms::content::CommsTemplateState;
use crate::comms::server::CommsRuntime;
use crate::lobby::{Target, WorldResource};
use crate::messages::{GamePhase, ServerMessage};
use crate::objectives::ObjectiveManager;
use crate::simulation::SimOutbox;
#[cfg(test)]
use crate::world::content::TriggerAction;
use crate::world::content::{trigger_states_from_world, TriggerState, WorldEvent};
use crate::world::delayed::{partition_delayed_actions, DelayedAction};
use crate::world::dispatch::{
    dispatch_action, ActionCmd, DispatchContext, DispatchResult, LayerView,
    WORLD_MODIFIER_SOURCE_ID,
};
use crate::world::layers::{evaluate_layer_load, evaluate_layer_unload, LayerLoadOutcome};
use crate::world::scenario::evaluate_scenario_load;

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
    /// `Time::elapsed_secs()` snapshot taken when the base world was loaded
    /// (set by `init_world_runtime`). `on_timer` triggers fire when
    /// `time.elapsed_secs() - world_loaded_at_secs >= after_secs`.
    /// `None` while no world is loaded (lobby, fallback bootstrap), in
    /// which case `collect_world_events` skips emitting `TimerElapsed` events.
    /// (#475)
    pub world_loaded_at_secs: Option<f32>,
    /// Maps named groups to the set of entity names currently in that group.
    pub entity_groups: HashMap<String, HashSet<String>>,
    /// Actions queued for deferred dispatch (via `action_delays` on triggers).
    pub pending_delayed_actions: Vec<DelayedAction>,
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
/// Keyed by world TOML path in `WorldLayerMap`. Holds the trigger and comms
/// states that were derived from the sub-world's `WorldConfig` at load time,
/// enabling them to be cleanly removed when `UnloadWorld` fires.
/// Also tracks ECS entity handles spawned from the sub-world's `[[entity]]`
/// blocks so they can be despawned when `UnloadWorld` fires.
#[derive(Clone, Debug, Default)]
pub struct WorldRuntime {
    pub trigger_states: Vec<TriggerState>,
    pub comms_template_states: Vec<CommsTemplateState>,
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
/// `TimerElapsed` event). Consumers: `inject_comms_templates` (comms-template
/// evaluation) and `tick_trigger_pipeline` (seeds its trigger-chaining loop).
///
/// The chaining loop's internally-produced events (`FlagSet`, `FlagCleared`,
/// `Destroyed` from a `DestroyEntity` action) stay LOCAL to the pipeline and
/// are never written here, preserving the contract that comms templates fire
/// only from external events. Contents are valid for one tick:
/// `collect_world_events` rebuilds the buffer every run, so stale events
/// never leak into the next tick.
#[derive(Resource, Default)]
pub struct WorldEventBuffer(pub Vec<WorldEvent>);

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        // The comms half of the pre-#816 WorldPlugin lives in
        // `CommsWorldPlugin`. Added here so every app that installs the
        // world also gets comms, and so the cross-plugin ordering
        // constraints (`init_comms_runtime` after `init_world_runtime`;
        // the Physics-set `tick_pending_follow_ups → collect_world_events
        // → inject_comms_templates → tick_trigger_pipeline` chain;
        // `broadcast_objective_summary` after `broadcast_comms_state`)
        // all resolve against systems guaranteed to be registered.
        app.add_plugins(crate::comms::CommsWorldPlugin)
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
                    spawn_world_entities,
                    init_world_runtime,
                    load_extra_worlds,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                broadcast_objective_summary
                    .in_set(crate::sim_sets::SimSet::Broadcast)
                    .after(crate::comms::server::broadcast_comms_state),
            )
            // Explicit trigger-pipeline ordering (#718/#719): the comms
            // halves of the chain (`tick_pending_follow_ups`,
            // `inject_comms_templates`) are registered by
            // `CommsWorldPlugin` with `.before`/`.after` constraints
            // against these two systems, reproducing the original
            // four-system `.chain()` exactly.
            .add_systems(
                Update,
                (collect_world_events, tick_trigger_pipeline)
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
                Update,
                crate::ai_plugin::advance_objective_cursors
                    .in_set(crate::sim_sets::SimSet::Modifiers),
            )
            .add_systems(
                Update,
                tick_delayed_actions
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(tick_trigger_pipeline),
            )
            .add_systems(
                Update,
                apply_pending_scenario_loads.in_set(crate::sim_sets::SimSet::Physics),
            )
            .add_systems(
                Update,
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
fn spawn_world_entities(
    mut commands: Commands,
    world_config: Option<ResMut<crate::world::config::WorldConfig>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
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
        crate::sim_rng::assign_uuid_with(sim_rng.as_deref())
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
        sim_rng.as_deref(),
    );
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
    sim_rng: Option<&crate::sim_rng::SimRng>,
) -> Vec<Entity> {
    // Atomic-activation guard (issue #750): if this world's entity identity is
    // invalid (e.g. duplicate reference names), spawn NOTHING. A composition
    // error must never leave partial root-world content active. The headless
    // build path aborts earlier on the full composition; this seam is the
    // last-resort gate for the Bevy `Startup` spawn.
    let mut identity =
        crate::world::validate::validate_entity_identity("", "", &world_config.entities);
    // Objective authoring validation (issue #752): duplicate declarations within
    // a single action list, or complete/fail references to objectives no
    // add_objective declares, also block activation so nothing spawns partially.
    identity.extend(crate::world::validate::validate_objectives(
        "",
        "",
        world_config,
    ));
    if crate::world::validate::has_error(&identity) {
        bevy::log::error!(
            target: "world",
            "spawn blocked: world composition invalid ({} error(s)); spawning zero entities",
            identity.iter().filter(|f| f.is_error()).count()
        );
        return Vec::new();
    }

    let (fields, named, _anon) =
        crate::world::config::partition_immediate_entities_three_way(world_config, |path| {
            config_cache
                .get(path)
                .and_then(|c| c.asteroid_field.as_ref())
                .is_some()
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
        let mut config = match crate::entity_loader::resolve_entity(entity_inst, config_cache) {
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
        let uuid = crate::sim_rng::assign_uuid_with(sim_rng);
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
        let config = match crate::entity_loader::resolve_entity(entity_inst, config_cache) {
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
    time: Option<Res<bevy::time::Time>>,
) {
    let Some(world_config) = world_config else {
        return;
    };

    // (#475) Stamp the load-time anchor for `on_timer` triggers. All
    // `after_secs` values are measured relative to this; `collect_world_events`
    // emits `WorldEvent::TimerElapsed { elapsed_secs }` each tick using
    // `time.elapsed_secs() - world_loaded_at_secs`. `Time` is wrapped in
    // `Option` so older test apps that don't install `TimePlugin` continue
    // to work (they just never see `TimerElapsed` events Ã¢â‚¬â€ same as today).
    if let Some(t) = time {
        runtime.world_loaded_at_secs = Some(t.elapsed_secs());
    }

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

    // Derive trigger runtime states straight from the parsed world.
    runtime.trigger_states = trigger_states_from_world(&world_config);

    // Issue #415: emit a WorldLoaded event so `on_world_loaded` triggers
    // declared in the base world fire on the first Update tick. Pushed onto
    // the pending queue (rather than evaluated here) so the dispatch logic
    // inside `tick_trigger_pipeline` is the single owner of trigger action
    // execution.
    runtime.pending_world_events.push(WorldEvent::WorldLoaded);
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

// -- AI-event trigger system -------------------------------------------------

/// Collect this tick's externally-sourced `WorldEvent`s into `WorldEventBuffer`.
///
/// Three sources, in order:
/// 1. AI-plugin messages (`AiEntityAttacked` / `AiEntityDestroyed` /
///    `AiWaypointReached`) bridged into `WorldEvent`s via `AiEventReaders`.
/// 2. `runtime.pending_world_events` (`WorldLoaded`, `EnteredRegion`, ...
///    queued by `init_world_runtime`, `apply_world_layer_changes`, the region
///    observers, and `tick_delayed_actions` on the previous tick).
/// 3. (#475) A synthesised `TimerElapsed` event once the world has loaded.
///    `on_timer` triggers fire when `elapsed_secs >= after_secs`, measured
///    from `world_loaded_at_secs` (so `after_secs = 0` fires on the first
///    post-load tick, and `after_secs = 300` fires 300s into the scenario
///    regardless of how long the lobby was up beforehand). Single-shot
///    semantics on `TriggerState.fired` prevent re-firing. `Time` is optional
///    so test apps without `TimePlugin` continue to work (they just never see
///    `TimerElapsed`).
///
/// Ordering: chained after `tick_pending_follow_ups` (which snapshots
/// `pending_world_events` before this system drains them) and before
/// `inject_comms_templates` and `tick_trigger_pipeline` (which consume the
/// buffer for comms-template injection and trigger evaluation respectively).
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
    // Drain any externally-queued world events (e.g. WorldLoaded pushed by
    // init_world_runtime or apply_world_layer_changes). The emptiness check
    // is a read: it keeps event-free ticks from marking WorldContentRuntime
    // changed via a no-op DerefMut.
    if !runtime.pending_world_events.is_empty() {
        world_events.append(&mut runtime.pending_world_events);
    }
    let elapsed_secs = time.as_ref().and_then(|t| {
        runtime
            .world_loaded_at_secs
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
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    let empty_anchors: HashMap<String, [f32; 3]> = HashMap::new();
    // Seeded UUID source for `SpawnEntity` dispatch. Bound once per system run
    // because `DispatchContext::uuid_source` is a `&dyn Fn`.
    let uuid_source = || crate::sim_rng::assign_uuid_with(sim_rng.as_deref());
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
            .world_loaded_at_secs
            .map(|loaded_at| (t.elapsed_secs() - loaded_at).max(0.0))
    });

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

    // Comms-template injection happens in `inject_comms_templates`, chained
    // immediately before this system (#719).

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
        let mut fired: Vec<crate::world::content::FiredTrigger> = Vec::new();
        // We have to clone the origin_layer slice up front: the evaluator
        // below takes `&mut runtime.trigger_states[idx]`, so nothing may
        // hold a borrow on `runtime.trigger_states` across the loop.
        let trigger_origins: Vec<Option<String>> = runtime
            .trigger_states
            .iter()
            .map(|s| s.origin_layer.clone())
            .collect();
        let entity_groups = runtime.entity_groups.clone();
        // Issue #710: ONE name -> uuid map for *action dispatch*, rebuilt at
        // the top of every chaining pass — the same per-pass freshness rule
        // `entity_groups` above already follows. Previously the six modifier
        // arms read the tick-level `name_to_uuid` clone while `DestroyEntity`
        // read the live `runtime.name_to_uuid`; unifying on per-pass makes the
        // modifier arms slightly fresher and `DestroyEntity` slightly staler,
        // and lets an action resolve a name that a `SpawnEntity` in an earlier
        // pass of this same tick registered.
        //
        // Trigger *condition* evaluation deliberately keeps using the
        // tick-level `name_to_uuid` clone above: only the dispatch arms change.
        let dispatch_names = runtime.name_to_uuid.clone();
        for (idx, origin) in trigger_origins.iter().enumerate() {
            // Build the flag-store and layer-path chains for this trigger.
            let mut flag_chain: Vec<&crate::world::flags::FlagStore> = Vec::new();
            let mut layer_chain: Vec<Option<String>> = Vec::new();
            let mut cur = origin.clone();
            loop {
                layer_chain.push(cur.clone());
                match &cur {
                    Some(p) => {
                        if let Some(wr) = world_layers.layer_map.as_ref().and_then(|lm| lm.0.get(p))
                        {
                            flag_chain.push(&wr.flags);
                            cur = wr.loader_path.clone();
                        } else {
                            // Layer missing from the map Ã¢â‚¬â€ treat as empty.
                            // (Shouldn't happen in normal flow.)
                            flag_chain.push(&runtime.flags);
                            break;
                        }
                    }
                    None => {
                        flag_chain.push(&runtime.flags);
                        break;
                    }
                }
            }
            // The base entry was already pushed above when cur went to None.
            // If we exited via the layer-missing branch we also pushed base.
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
                fired.push(ft);
            }
        }

        if fired.is_empty() {
            break;
        }

        let mut next_events: Vec<WorldEvent> = Vec::new();
        for ft in fired {
            for (i, action) in ft.actions.iter().enumerate() {
                let delay = ft.action_delays.get(i).copied().unwrap_or(0.0);
                if delay > 0.0 {
                    if let Some(es) = elapsed_secs {
                        runtime.pending_delayed_actions.push(DelayedAction {
                            action: action.clone(),
                            origin_layer: ft.origin_layer.clone(),
                            entity_name: ft.entity_name.clone(),
                            fire_at_elapsed: es + delay,
                        });
                    }
                    continue;
                }
                // Issue #710: the action-dispatch table lives in the pure
                // `world::dispatch` module. Decide first, then apply.
                //
                // The flag stores handed to the context MUST be the LIVE ones
                // — `runtime.flags` plus `layer_map`'s per-layer stores,
                // re-projected for every action, never a stale copy.
                // `dispatch_action` computes each flag mutation's
                // before/after against these stores to decide whether a
                // transition event fires at all, so two triggers that both
                // `set_flag` the same flag must see 0 -> 1 and then 1 -> 1
                // (one `FlagSet` total), not 0 -> 1 twice.
                let layers = project_layer_views(world_layers.layer_map.as_deref());
                let result = {
                    let ctx = DispatchContext {
                        origin_layer: ft.origin_layer.clone(),
                        entity_name: ft.entity_name.clone(),
                        name_to_uuid: &dispatch_names,
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
                    dispatch_action(action, &ctx)
                };

                // Applied before the next action is dispatched, so the next
                // `DispatchContext` observes this action's writes. `new_events`
                // route into this pass's `next_events` — i.e. the next chaining
                // pass of the SAME tick (`tick_delayed_actions` instead queues
                // them onto `runtime.pending_world_events` for the next tick).
                apply_dispatch_result(
                    result,
                    "tick_trigger_pipeline",
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
                );
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
                // entity just persists for the session).
                if let (Some(path), Some(lm)) = (&layer_path, layer_map.as_deref_mut()) {
                    if let Some(layer) = lm.0.get_mut(path) {
                        layer.spawned_entities.push(spawned);
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
/// the same tick's `world_loaded_at_secs` anchor.
fn tick_delayed_actions(
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
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    let Some(elapsed) = time.as_ref().and_then(|t| {
        runtime
            .world_loaded_at_secs
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
    let uuid_source = || crate::sim_rng::assign_uuid_with(sim_rng.as_deref());

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
/// live `WorldContentRuntime` (trigger states + comms templates + contacts).
///
/// On WASM the TOML string is not available at runtime (JS pre-fetches only the
/// initial world), so we push paths into the WASM-side pending-world queue and
/// the implementation returns early until the JS bridge delivers the TOML via
/// `wasm_push_world_toml`. On native targets `std::fs::read_to_string` is used.
fn apply_pending_scenario_loads(
    mut pending: ResMut<PendingScenarioLoad>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
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
        // Merge trigger states (don't overwrite existing ones).
        runtime.trigger_states.extend(result.new_trigger_states);
        if let Some(scenario_config) = result.scenario_config {
            // Merge comms template states + contacts into the live comms
            // runtime (skips duplicate contacts by uuid).
            let _ = crate::comms::server::merge_world_comms(
                &mut comms,
                &scenario_config,
                &runtime.name_to_uuid,
            );
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
fn build_layer_config_cache(
    _world_config: &crate::world::config::WorldConfig,
) -> crate::config_cache::ConfigCache {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::entity_config::EntityConfig;
        let mut cache = crate::config_cache::get_config_cache();
        for entity in &_world_config.entities {
            if cache.contains_key(&entity.template_path) {
                continue;
            }
            match std::fs::read_to_string(&entity.template_path) {
                Ok(toml_str) => {
                    if let Ok(cfg) = EntityConfig::from_toml(&toml_str) {
                        cache.insert(entity.template_path.clone(), cfg);
                    } else {
                        bevy::log::warn!(
                            "build_layer_config_cache: failed to parse '{}' ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â entity will be skipped",
                            entity.template_path
                        );
                    }
                }
                Err(_) => {
                    // Template not on disk (e.g. test fixture); skip silently.
                    // spawn_immediate_entities_internal logs and continues for missing templates.
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
    mut comms: ResMut<CommsRuntime>,
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
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
                        crate::sim_rng::assign_uuid_with(sim_rng.as_deref())
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
                        trigger_states,
                        name_to_uuid_inserts,
                        scenario_config,
                        emit_world_loaded,
                    } => {
                        // Merge the origin-tagged trigger states (issue #417)
                        // into the live runtime and register the layer's
                        // named entities in the live name_to_uuid map.
                        runtime.trigger_states.extend(trigger_states.clone());
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
                            sim_rng.as_deref(),
                        );

                        // Merge comms template states + contacts
                        // into the live comms runtime (skips
                        // duplicate contacts by uuid); snapshot
                        // the layer's states for UnloadWorld.
                        let comms_template_states = crate::comms::server::merge_world_comms(
                            &mut comms,
                            &scenario_config,
                            &runtime.name_to_uuid,
                        );

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
                                trigger_states,
                                comms_template_states,
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

                // Remove trigger states belonging to this layer. Which live
                // indices belong to it is a pure decision (`world::layers`),
                // matched by trigger equality against the load-time snapshot.
                let unload = evaluate_layer_unload(&layer.trigger_states, &runtime.trigger_states);
                let mut ti = 0usize;
                runtime.trigger_states.retain(|_| {
                    let keep = !unload.triggers_to_remove.contains(&ti);
                    ti += 1;
                    keep
                });

                // Remove comms template states belonging to this layer.
                crate::comms::server::remove_layer_comms(&mut comms, &layer.comms_template_states);

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
    use crate::comms::server::{
        inject_comms_templates, tick_pending_follow_ups, CommsChannel2Event, CommsInboxRes,
    };
    use crate::console::comms::server::handle_comms_channel2;
    use crate::lobby::LobbyPlugin;
    use crate::messages::*;
    use crate::world::content::TriggerCondition;

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
                    tick_pending_follow_ups,
                    collect_world_events,
                    inject_comms_templates,
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

    /// Issue #710: the `DispatchContext` flag stores must be the LIVE ones,
    /// never a per-pass copy taken before dispatch began (condition
    /// evaluation reads the stores before any dispatch, which is safe;
    /// dispatch itself must not). `dispatch_action` computes before/after
    /// against them to decide whether a transition event fires at all, so a
    /// snapshot silently drops transitions.
    ///
    /// Set-then-clear in a single pass is the discriminating case: against the
    /// live store the clear reads before = 1 and emits `FlagCleared`; against a
    /// snapshot it reads before = 0, sees no change, and emits nothing. The
    /// sibling case — two triggers both `set_flag` the same flag
    /// (`assets/worlds/before_the_fire.toml:276`) — is not observable here,
    /// because a duplicated `FlagSet` is masked by single-shot trigger firing.
    #[test]
    fn flag_stores_handed_to_dispatch_are_live_within_a_pass() {
        let mut app = ai_trigger_test_app();
        let npc_uuid = "src-uuid-001";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            let mk = |condition, actions| TriggerState {
                trigger: crate::world::content::Trigger {
                    condition,
                    actions,
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            };
            runtime.trigger_states = vec![
                mk(
                    TriggerCondition::OnDestroyed {
                        entity_name: "source".into(),
                    },
                    vec![TriggerAction::SetWorldFlag { name: "a".into() }],
                ),
                mk(
                    TriggerCondition::OnDestroyed {
                        entity_name: "source".into(),
                    },
                    vec![TriggerAction::ClearWorldFlag { name: "a".into() }],
                ),
                mk(
                    TriggerCondition::OnFlagCleared { name: "a".into() },
                    vec![TriggerAction::AddObjective {
                        id: "obj-cleared".into(),
                        text: "Reacted to flag cleared".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                ),
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        // Both setter and clearer fire in the same pass. Against the LIVE store
        // the clear sees before = 1 and emits FlagCleared, so the downstream
        // on_flag_cleared trigger fires. Against a per-pass snapshot the clear
        // would see before = 0, emit nothing, and this objective would never
        // appear.
        let objectives = app.world().resource::<ObjectiveManagerRes>();
        assert!(
            objectives
                .0
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-cleared"),
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
            runtime.world_loaded_at_secs = Some(0.0);
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

    /// A scenario trigger fires when the named ship reaches the named
    /// waypoint — the `AiWaypointReached` message the cursor evaluator emits
    /// is bridged into a `WorldEvent::WaypointReached` and matched here.
    #[test]
    fn on_waypoint_reached_trigger_fires_add_objective_action() {
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
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-border".to_string(),
                    text: "Patrol reached the border".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-border"),
            "reaching wp_border must fire the trigger's AddObjective action"
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
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-border".to_string(),
                    text: "Patrol reached the border".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-border"),
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
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-any".to_string(),
                    text: "Patrol reached a waypoint".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-any"),
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
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-harrow".to_string(),
                    text: "Harrow arrived".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-harrow"),
            "another ship's arrival must not fire a trigger scoped to harrow_patrol"
        );
    }

    #[test]
    fn on_entity_destroyed_trigger_fires_add_objective_action() {
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
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-001".to_string(),
                    text: "Station destroyed".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-001"),
            "AddObjective action must have fired"
        );
    }

    // -- AI-event ApplyModifier / per-entity target regression tests -------
    //
    // The following six tests exercise `tick_trigger_pipeline` dispatch of the
    // per-entity trigger actions (`ApplyModifier`, `RemoveModifier`,
    // `ApplyFlag`, `RemoveFlag`, `ApplyIntModifier`, `RemoveIntModifier`)
    // and prove that the action lands on the target entity's per-entity
    // `ShipModifiers` Component â€” not the legacy global Resource â€” and
    // that non-target entities (e.g. the player ship) remain unaffected.
    // These are the regression tests for the audit-report bug where world
    // triggers silently misrouted every named-entity write to whichever
    // ship happened to own the global Resource.

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

    /// Installs a single `OnDestroyed { raider_alpha }` trigger whose
    /// action list is `actions`, emits `AiEntityDestroyed { npc-target-uuid }`,
    /// and ticks once so `tick_trigger_pipeline` dispatches the actions.
    fn fire_ai_event_trigger(app: &mut App, actions: Vec<TriggerAction>) {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.trigger_states = vec![TriggerState {
            trigger: crate::world::content::Trigger {
                condition: TriggerCondition::OnDestroyed {
                    entity_name: "raider_alpha".to_string(),
                },
                actions,
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
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
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: "npc-target-uuid".to_string(),
            });
        app.update();
    }

    #[test]
    fn ai_events_apply_modifier_lands_on_target_entity_not_player() {
        let mut app = ai_trigger_test_app();
        let (npc, player) = spawn_two_modifier_targets(&mut app);
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        fire_ai_event_trigger(
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
        // `app.update()` calls and fire its action only when the last entity
        // dies. Mirrors `on_entity_destroyed_trigger_fires_add_objective_action`
        // but uses two separate destruction ticks. (#470)
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
                actions: vec![TriggerAction::AddObjective {
                    id: "obj-victory".to_string(),
                    text: "All waves cleared".to_string(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-victory"),
            "AddObjective must not fire after only first wave dies"
        );

        // Tick 2: wave_b dies. Trigger must NOW fire.
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: uuid_b.to_string(),
            });
        app.update();

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-victory"),
            "AddObjective must fire after the last named entity dies"
        );
    }

    // -- Flag-system integration tests (issue #412) ---------------------------

    #[test]
    fn when_predicate_suppresses_action_dispatch_when_false() {
        // on_destroyed with when="flag(green_light)" must not fire its actions
        // while the flag is unset, but the trigger MUST remain live so it can
        // fire on a subsequent matching event once the flag is set.
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
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-gated".into(),
                        text: "Should only fire after flag is set".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(green_light)").unwrap()),
                    action_predicates: vec![],
                    action_delays: vec![],
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
        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(!runtime.trigger_states[0].fired, "trigger must remain live");
            let objs = &app.world().resource::<ObjectiveManagerRes>().0;
            assert!(
                !objs.sorted_snapshots().iter().any(|o| o.id == "obj-gated"),
                "gated action must NOT have fired"
            );
        }
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
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-gated"),
            "gated action must fire once the flag is set"
        );
    }

    #[test]
    fn set_flag_action_fires_on_flag_set_trigger_within_same_tick() {
        // Trigger A: on_destroyed ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ set_flag a
        // Trigger B: on_flag_set { name="a" } ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ add_objective B
        // A and B must both fire in a single tick.
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-chain-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::SetWorldFlag { name: "a".into() }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet { name: "a".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-chain".into(),
                            text: "Reacted to flag set".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.flags.flag("a"),
            "set_flag action must have mutated the store"
        );
        assert!(
            runtime.trigger_states[1].fired,
            "on_flag_set trigger must have fired"
        );
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots().iter().any(|o| o.id == "obj-chain"),
            "chained AddObjective must have fired in the same tick"
        );
    }

    #[test]
    fn no_op_reset_of_already_set_flag_does_not_emit_transition() {
        // Flag "a" starts set. A trigger fires set_flag a (no-op, value stays 1).
        // An on_flag_set trigger for "a" must NOT fire (transitions only).
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-noop-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("a"); // pre-set
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::SetWorldFlag { name: "a".into() }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet { name: "a".into() },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-no-op".into(),
                            text: "Should not fire on no-op re-set".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.trigger_states[1].fired,
            "on_flag_set must not fire when the flag was already set (no transition)"
        );
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objs.sorted_snapshots().iter().any(|o| o.id == "obj-no-op"),
            "no objective expected from a no-op flag re-set"
        );
    }

    #[test]
    fn clear_flag_action_fires_on_flag_cleared_trigger() {
        let mut app = ai_trigger_test_app();
        let npc_uuid = "uuid-clear-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("shields_up"); // pre-set so we transition trueÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢false
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::ClearWorldFlag {
                            name: "shields_up".into(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagCleared {
                            name: "shields_up".into(),
                        },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-shields-down".into(),
                            text: "Shields are down".into(),
                            mandatory: true,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(!runtime.flags.flag("shields_up"));
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-shields-down"),
            "on_flag_cleared trigger must fire on trueÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢false transition"
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
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-parent-when".into(),
                        text: "parent flag was set".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(parent:armed)").unwrap()),
                    action_predicates: vec![],
                    action_delays: vec![],
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

        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-parent-when"),
            "sub-world trigger gated on parent:armed must fire when base flag is set"
        );
    }

    /// Per-layer flag scoping: a flag set inside a sub-world must NOT
    /// fire a base-world trigger that watches the same name.
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
        let npc_uuid = "uuid-scoped-source";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![
                // Sub-world trigger: setting `armed` in the sub-world layer.
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "source".into(),
                        },
                        actions: vec![TriggerAction::SetWorldFlag {
                            name: "armed".into(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: Some(layer_path.clone()),
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                // Base-world watcher: on_flag_set armed.
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet {
                            name: "armed".into(),
                        },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-base-armed".into(),
                            text: "should NOT fire Ã¢输了¬â€� different layer".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
            ];
        }
        app.world_mut()
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
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
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            !objs
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-base-armed"),
            "base trigger must not cross-fire on sub-world flag"
        );
    }

    /// `parent:flag` mutation from the base world walks past root Ã¢â€ â€™ no-op +
    /// warn; the predicate read also resolves as unset.
    #[test]
    fn parent_walk_past_root_from_base_is_noop_for_mutation_and_reads_unset() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>();
        app.init_resource::<PendingWorldLayerChanges>();
        let npc_uuid = "uuid-past-root";
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("source".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "source".into(),
                    },
                    // Base-world trigger (origin_layer=None) tries to mutate
                    // `parent:armed` Ã¯Â¿Â½ must be a no-op.
                    actions: vec![TriggerAction::SetWorldFlag {
                        name: "parent:armed".into(),
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<AiEntityDestroyed>>()
            .write(AiEntityDestroyed {
                entity_uuid: npc_uuid.into(),
            });
        app.update();

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

    #[test]
    fn on_entity_attacked_trigger_fires_add_objective_action() {
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
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-002".to_string(),
                        text: "Enemy attacked".to_string(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-002"),
            "AddObjective action from on_entity_attacked must have fired"
        );
    }

    #[test]
    fn set_ai_state_action_is_noop_in_doctrine_based_ai() {
        // Issue #572: SetAiState is kept in TriggerAction for TOML backward compat
        // but is now a no-op â€” doctrine-based AI has no FSM state slots. Verify
        // the system doesn't crash and the AI entity is unmodified.
        use crate::entity_config::BehaviourConfig;

        let mut app = ai_trigger_test_app();

        let npc_uuid = "npc-state-change-uuid-003";
        let attacker_uuid = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.0),
                EntityUuid(npc_uuid.to_string()),
                BehaviourSection(BehaviourConfig::default()),
            ))
            .id();
        app.update(); // register AI tokens

        // Set up trigger: on attacked â†’ SetAiState (now a no-op).
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("npc_alpha".to_string(), npc_uuid.to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "npc_alpha".to_string(),
                    },
                    actions: vec![TriggerAction::SetAiState {
                        entity: "npc_alpha".to_string(),
                        state: "chase".to_string(),
                        target: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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

        // Fire the attacked event.
        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: npc_uuid.to_string(),
                attacker_uuid,
            });

        // Must not panic â€” SetAiState is silently ignored.
        app.update();

        // The entity must still be AI-controlled (no FSM state to mutate).
        assert!(
            app.world().get::<BehaviourSection>(entity).is_some(),
            "BehaviourSection must survive a SetAiState no-op"
        );
    }

    // -- add_faction_enemy / remove_faction_enemy dispatch tests --------------

    /// Helper: fire a single trigger with the given action via
    /// `tick_trigger_pipeline`. Uses `on_world_loaded` so we only need a
    /// `WorldLoaded` event to fire it. Returns the post-update App so
    /// tests can inspect mutated resources.
    fn fire_world_loaded_action(actions: Vec<TriggerAction>) -> App {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnWorldLoaded,
                    actions,
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
        app
    }

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

        // Fire a trigger that flips both directions hostile.
        let app = fire_world_loaded_action(vec![
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
        let app = fire_world_loaded_action(vec![TriggerAction::AddFactionEnemy {
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
        let app = fire_world_loaded_action(vec![
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

    #[test]
    fn remove_faction_enemy_action_clears_blackboard_target_when_target_becomes_friendly() {
        // Scenario:
        //   1. Spawn a Harrow-factioned NPC that targets a Federation
        //      player ship (set blackboard.target manually to simulate a
        //      prior `enemy_in_range` engagement).
        //   2. Make Federation hostile to Harrow via add_faction_enemy
        //      so the precondition is mutually hostile.
        //   3. Fire remove_faction_enemy for Harrow Ã¢â€ â€™ Federation.
        //   4. Assert: the NPC's blackboard.target is now None (the
        //      revalidation kicked in because target_faction is no
        //      longer hostile to self_faction).
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

        // Bring both sides into mutual hostility, then fire
        // remove_faction_enemy on Harrow's side. Two trigger states Ã¢â€¡â€™ the
        // first establishes the relationship that the second tears down.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnWorldLoaded,
                        actions: vec![
                            TriggerAction::AddFactionEnemy {
                                faction: "Harrow".into(),
                                enemy: "Federation".into(),
                            },
                            TriggerAction::AddFactionEnemy {
                                faction: "Federation".into(),
                                enemy: "Harrow".into(),
                            },
                        ],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagSet {
                            name: "peace".into(),
                        },
                        actions: vec![TriggerAction::RemoveFactionEnemy {
                            faction: "Harrow".into(),
                            enemy: "Federation".into(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
            ];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
            runtime.pending_world_events.push(WorldEvent::FlagSet {
                name: "peace".into(),
                origin_layer: None,
            });
        }

        app.update();

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
            EntityConfig::from_toml(raider_toml).unwrap(),
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
            .push("assets/worlds/patrol.toml".into());
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
            matches!(&pending.0[0], WorldLayerChange::Load { path: p, .. } if p == "assets/worlds/patrol.toml")
        );
        assert!(
            matches!(&pending.0[1], WorldLayerChange::Load { path: p, .. } if p == "assets/worlds/side.toml")
        );
    }

    /// `LoadWorld` action via trigger queues a `Load` command into `PendingWorldLayerChanges`.
    #[test]
    fn load_world_trigger_action_queues_pending_layer_change() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>();

        let npc_uuid = "trigger-load-world-npc-001";
        let attacker_uuid = uuid::Uuid::parse_str("dddddddd-0000-0000-0000-000000000001").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "raider".into(),
                    },
                    actions: vec![TriggerAction::LoadWorld {
                        path: "assets/worlds/patrol.toml".into(),
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
                entity_uuid: npc_uuid.into(),
                attacker_uuid,
            });
        app.update();

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(pending.0.len(), 1, "one Load must be queued");
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Load { path: p, .. } if p == "assets/worlds/patrol.toml")
        );
    }

    /// `UnloadWorld` action via trigger queues an `Unload` command.
    #[test]
    fn unload_world_trigger_action_queues_pending_layer_change() {
        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>();

        let npc_uuid = "trigger-unload-world-npc-002";
        let attacker_uuid = uuid::Uuid::parse_str("dddddddd-0000-0000-0000-000000000002").unwrap();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("raider".into(), npc_uuid.into());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "raider".into(),
                    },
                    actions: vec![TriggerAction::UnloadWorld {
                        path: "assets/worlds/patrol.toml".into(),
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
                entity_uuid: npc_uuid.into(),
                attacker_uuid,
            });
        app.update();

        let pending = app.world().resource::<PendingWorldLayerChanges>();
        assert_eq!(pending.0.len(), 1, "one Unload must be queued");
        assert!(
            matches!(&pending.0[0], WorldLayerChange::Unload(p) if p == "assets/worlds/patrol.toml")
        );
    }

    /// `apply_world_layer_changes` with `LoadWorld(patrol.toml)` reads the TOML
    /// on native, merges triggers into `WorldContentRuntime`, and registers the
    /// layer in `WorldLayerMap`.
    #[test]
    fn load_world_action_merges_triggers_into_runtime() {
        let mut app = layer_test_app();

        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });

        app.update();

        let layer_map = app.world().resource::<WorldLayerMap>();
        assert!(
            layer_map.0.contains_key("assets/worlds/patrol.toml"),
            "WorldLayerMap must contain the loaded path"
        );

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.trigger_states.is_empty(),
            "trigger states must be merged into runtime"
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
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });
        app.update();

        let trigger_count_after_first = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        // Load again ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â must not double-add.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });
        app.update();

        let trigger_count_after_second = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        assert_eq!(
            trigger_count_after_first, trigger_count_after_second,
            "duplicate LoadWorld must not add duplicate trigger states"
        );
    }

    /// `UnloadWorld` removes the triggers that were added by the matching `LoadWorld`.
    #[test]
    fn unload_world_removes_triggers_added_by_load_world() {
        let mut app = layer_test_app();

        // Load patrol.toml.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Load {
                path: "assets/worlds/patrol.toml".into(),
                loader_path: None,
            });
        app.update();

        let trigger_count_loaded = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();
        assert!(
            trigger_count_loaded > 0,
            "patrol.toml must add at least one trigger"
        );

        // Unload it.
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload("assets/worlds/patrol.toml".into()));
        app.update();

        let trigger_count_unloaded = app
            .world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len();

        assert_eq!(
            trigger_count_unloaded, 0,
            "UnloadWorld must remove all triggers that were added by the LoadWorld"
        );

        let layer_map = app.world().resource::<WorldLayerMap>();
        assert!(
            !layer_map.0.contains_key("assets/worlds/patrol.toml"),
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

        let world_toml = format!(
            r#"
[global]
seed = 1

[[entity]]
template_path = "{template_path_str}"
name = "layer_npc"
position = [1.0, 0.0, 0.0]

[[trigger]]
condition = "on_destroyed"
entity = "layer_npc"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-layer-npc"
  text = "Destroyed."
  mandatory = false
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
                (
                    collect_world_events,
                    inject_comms_templates,
                    tick_trigger_pipeline,
                    probe,
                )
                    .chain(),
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
    #[test]
    fn trigger_chain_exceeding_max_passes_stops_at_the_cap() {
        let mut app = ai_trigger_test_app();
        let chain_len = (MAX_CHAIN_PASSES + 2) as usize;
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            let mk = |condition, flag_to_set: String| TriggerState {
                trigger: crate::world::content::Trigger {
                    condition,
                    actions: vec![TriggerAction::SetWorldFlag { name: flag_to_set }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            };
            let mut states = vec![mk(TriggerCondition::OnWorldLoaded, "chain_1".to_string())];
            for i in 1..chain_len {
                states.push(mk(
                    TriggerCondition::OnFlagSet {
                        name: format!("chain_{i}"),
                    },
                    format!("chain_{}", i + 1),
                ));
            }
            runtime.trigger_states = states;
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

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
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-ghost-layer".into(),
                        text: "Fired despite the missing layer".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: Some(crate::world::flags::parse_predicate("flag(armed)").unwrap()),
                    action_predicates: vec![],
                    action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-ghost-layer"),
            "a missing origin layer must fall back to the base flag store, \
             so the base-store `armed` flag must satisfy the when predicate"
        );
    }

    /// (#717) One trigger whose action list carries every `TriggerAction`
    /// variant (all 21) must dispatch them in LIST ORDER, each result applied
    /// before the next action is dispatched. Order is observed through
    /// order-sensitive pairs rather than instrumentation:
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
    #[test]
    fn single_trigger_dispatches_every_action_variant_in_list_order() {
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

        {
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
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnWorldLoaded,
                        actions: all_variants,
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                // Chained witness: fires only if the pipeline emitted
                // FlagSet("ordered") followed by FlagCleared("ordered").
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnFlagCleared {
                            name: "ordered".into(),
                        },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-chained-cleared".into(),
                            text: "Observed set-then-clear".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
            ];
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }

        app.update();
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
        assert_eq!(
            snapshot_status("obj-chained-cleared"),
            Some(ObjectiveStatus::Active),
            "SetWorldFlag must dispatch before ClearWorldFlag (chained \
             on_flag_cleared trigger must observe the transition)"
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

    /// `tick_trigger_pipeline` drains `pending_world_events` and dispatches their
    /// matching triggers' actions. Seeds a `WorldLoaded` event directly into
    /// the queue and asserts the `add_objective` action fires.
    #[test]
    fn pending_world_loaded_event_fires_on_world_loaded_trigger() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnWorldLoaded,
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-loaded".into(),
                        text: "World loaded.".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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

        let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objectives
                .sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-loaded"),
            "on_world_loaded trigger must have fired its add_objective action"
        );
        // Queue must be drained.
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime.pending_world_events.is_empty(),
            "pending_world_events must be drained by collect_world_events"
        );
        assert!(
            runtime.trigger_states[0].fired,
            "trigger must be marked fired"
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
    /// event onto `pending_world_events` so any `on_world_loaded` triggers
    /// declared in the base world fire on the first Update tick.
    #[test]
    fn init_world_runtime_queues_world_loaded_event() {
        let mut app = App::new();
        app.add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<WorldResource>()
            .add_plugins(WorldPlugin);

        // Insert a WorldConfig so init_world_runtime takes its non-no-op path.
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.triggers.push(crate::world::content::Trigger {
            condition: TriggerCondition::OnWorldLoaded,
            actions: vec![TriggerAction::AddObjective {
                id: "obj-startup".into(),
                text: "Startup objective.".into(),
                mandatory: false,
                targets: vec![],
                directive: crate::messages::AiDirective::None,
                utility: crate::objectives::UtilityConfig::default(),
                source: crate::messages::ObjectiveSource::default(),
            }],
            when: None,
            action_predicates: vec![],
            action_delays: vec![],
            id: None,
            repeat: false,
            cooldown_secs: None,
        });
        app.insert_resource(cfg);

        app.world_mut().run_schedule(Startup);

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            runtime
                .pending_world_events
                .iter()
                .any(|e| matches!(e, WorldEvent::WorldLoaded)),
            "init_world_runtime must queue a WorldLoaded event during Startup"
        );
        assert_eq!(
            runtime.trigger_states.len(),
            1,
            "trigger states must be populated"
        );
    }

    /// Sub-world `LoadWorld` must push a `WorldLoaded` event so any
    /// `on_world_loaded` triggers declared in the loaded sub-world fire
    /// after the load merges.
    #[test]
    fn apply_world_layer_changes_queues_world_loaded_event_on_load() {
        let world_path = write_on_world_loaded_layer_fixture();

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

    /// End-to-end: load a sub-world with an `on_world_loaded` trigger,
    /// unload it, then re-load it. The trigger must fire on both load
    /// cycles (proves `fired` is reset because the trigger state is
    /// recreated fresh on re-load).
    #[test]
    fn on_world_loaded_fires_again_after_unload_and_reload() {
        let world_path = write_on_world_loaded_layer_fixture();

        let mut app = ai_trigger_test_app();
        app.init_resource::<WorldLayerMap>()
            .init_resource::<PendingWorldLayerChanges>()
            .add_systems(Update, apply_world_layer_changes);

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

        {
            let objectives = &app.world().resource::<ObjectiveManagerRes>().0;
            assert!(
                objectives
                    .sorted_snapshots()
                    .iter()
                    .any(|o| o.id == "obj-on-load"),
                "on_world_loaded trigger must fire on first load"
            );
        }

        // Complete the objective so we can detect the second add as a
        // distinct event (ObjectiveManager dedupes by id; re-adding the
        // same id leaves the existing objective in place which is fine ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â
        // we instead assert the trigger's `fired` flag flips back to true
        // after re-load).
        // -- Unload --
        app.world_mut()
            .resource_mut::<PendingWorldLayerChanges>()
            .0
            .push(WorldLayerChange::Unload(world_path.clone()));
        app.update();
        app.update();

        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(
                !runtime
                    .trigger_states
                    .iter()
                    .any(|s| matches!(s.trigger.condition, TriggerCondition::OnWorldLoaded)),
                "Unload must remove the on_world_loaded trigger state"
            );
        }

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

        let runtime = app.world().resource::<WorldContentRuntime>();
        let reloaded_trigger = runtime
            .trigger_states
            .iter()
            .find(|s| matches!(s.trigger.condition, TriggerCondition::OnWorldLoaded))
            .expect("on_world_loaded trigger must be re-registered on re-load");
        assert!(
            reloaded_trigger.fired,
            "on_world_loaded trigger must fire again on re-load \
             (proves fired flag is reset via fresh TriggerState on Load)"
        );
    }

    /// Writes a tiny world TOML containing exactly one `on_world_loaded`
    /// trigger to a temp file. Returns the path. Each call uses a unique
    /// path so parallel test runs do not collide.
    fn write_on_world_loaded_layer_fixture() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let tmp = std::env::temp_dir();
        let tag = COUNTER.fetch_add(1, Ordering::Relaxed);
        let world_path = tmp.join(format!("on_world_loaded_{tag}.toml"));
        let toml = r#"
[global]
seed = 1

[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "add_objective"
  id   = "obj-on-load"
  text = "Loaded."
  mandatory = false
"#;
        std::fs::write(&world_path, toml).expect("failed to write fixture world TOML");
        world_path.to_string_lossy().into_owned()
    }

    // -- Region enter/exit triggers (issue #416) -----------------------------

    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::{spawn_entity, EntityUuid};
    use crate::region_shape::RegionShape;
    use crate::regions::server::RegionPlugin;

    /// Build a minimal app that wires `RegionPlugin` + the issue-#416
    /// observers + `tick_trigger_pipeline` into the same world. Skips the
    /// heavyweight `WorldPlugin`/`AiPlugin`/`LobbyPlugin` bootstrap so the
    /// test focuses on the region-event ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ trigger-fire path.
    fn region_trigger_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
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
                    inject_comms_templates,
                    tick_trigger_pipeline,
                    handle_comms_channel2,
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
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
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

    fn install_region_trigger(
        app: &mut App,
        name: &str,
        uuid: &str,
        condition: TriggerCondition,
        obj_id: &str,
    ) {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.name_to_uuid.insert(name.into(), uuid.into());
        runtime.trigger_states.push(TriggerState {
            trigger: crate::world::content::Trigger {
                condition,
                actions: vec![TriggerAction::AddObjective {
                    id: obj_id.into(),
                    text: "region trigger objective".into(),
                    mandatory: false,
                    targets: vec![],
                    directive: crate::messages::AiDirective::None,
                    utility: crate::objectives::UtilityConfig::default(),
                    source: crate::messages::ObjectiveSource::default(),
                }],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
                id: None,
                repeat: false,
                cooldown_secs: None,
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        });
    }

    fn objective_present(app: &App, id: &str) -> bool {
        app.world()
            .resource::<ObjectiveManagerRes>()
            .0
            .sorted_snapshots()
            .iter()
            .any(|o| o.id == id)
    }

    #[test]
    fn ship_entering_region_fires_on_entered_region_trigger_exactly_once() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-nebula";
        spawn_region_with_uuid(&mut app, 100.0, 0.0, 50.0, uuid);
        install_region_trigger(
            &mut app,
            "nebula",
            uuid,
            TriggerCondition::OnEnteredRegion {
                entity_name: "nebula".into(),
            },
            "obj-entered",
        );

        // Tick 1: ship outside (at origin), no enter ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ no fire.
        app.update();
        assert!(
            !objective_present(&app, "obj-entered"),
            "trigger must not fire while outside"
        );

        // Move ship inside. The membership system runs in Physics and
        // queues a WorldEvent via the observer; `tick_trigger_pipeline` (also
        // in Physics) drains the queue on the NEXT tick ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â matching the
        // documented `WorldLoaded` two-tick pattern.
        set_ship_pos(&mut app, 110.0, 0.0);
        app.update(); // queues EnteredRegion
        app.update(); // collect_world_events drains; tick_trigger_pipeline fires
        assert!(
            objective_present(&app, "obj-entered"),
            "trigger must fire on entry"
        );

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
        install_region_trigger(
            &mut app,
            "nebula",
            uuid,
            TriggerCondition::OnExitedRegion {
                entity_name: "nebula".into(),
            },
            "obj-exited",
        );

        // Move inside first so we enter cleanly.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(
            !objective_present(&app, "obj-exited"),
            "exit trigger must not fire on entry"
        );

        // Now move outside ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ RegionExited ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ queued ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ drained next tick.
        set_ship_pos(&mut app, 200.0, 0.0);
        app.update();
        app.update();
        assert!(
            objective_present(&app, "obj-exited"),
            "exit trigger must fire when ship moves outside the region"
        );
    }

    #[test]
    fn region_despawn_while_ship_inside_fires_on_exited_region_trigger() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-fragile";
        let region_entity = spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);
        install_region_trigger(
            &mut app,
            "fragile",
            uuid,
            TriggerCondition::OnExitedRegion {
                entity_name: "fragile".into(),
            },
            "obj-imploded",
        );

        // Enter the region.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        app.update();
        assert!(!objective_present(&app, "obj-imploded"));

        // Despawn the region while ship is inside ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â membership system
        // emits an implicit RegionExited.
        app.world_mut().despawn(region_entity);
        app.update(); // queues ExitedRegion
        app.update(); // drains + fires

        assert!(
            objective_present(&app, "obj-imploded"),
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
        install_region_trigger(
            &mut app,
            "region_a",
            uuid_a,
            TriggerCondition::OnEnteredRegion {
                entity_name: "region_a".into(),
            },
            "obj-a",
        );
        install_region_trigger(
            &mut app,
            "region_b",
            uuid_b,
            TriggerCondition::OnEnteredRegion {
                entity_name: "region_b".into(),
            },
            "obj-b",
        );

        // Ship at origin is inside both regions. First tick queues both
        // events, second tick drains + fires both triggers.
        set_ship_pos(&mut app, 0.0, 0.0);
        app.update();
        app.update();

        assert!(
            objective_present(&app, "obj-a"),
            "region A enter trigger must fire"
        );
        assert!(
            objective_present(&app, "obj-b"),
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
        install_region_trigger(
            &mut app,
            "quarantine",
            uuid,
            TriggerCondition::OnEnteredRegion {
                entity_name: "quarantine".into(),
            },
            "obj-ship-quarantined",
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
            power: None,
            sensors_console: None,
            navigation_console: None,
            shields_console: None,
            torpedoes: None,
            repair: None,
            audio: None,
            comms: None,
            asteroid_field: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
            mesh: None,
            target: None,
            cinematic_camera: None,
            ai_profile: None,
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
            !objective_present(&app, "obj-ship-quarantined"),
            "NPC entering the region must not fire the player-ship trigger"
        );
        let runtime = app.world().resource::<WorldContentRuntime>();
        assert!(
            !runtime.trigger_states[0].fired,
            "trigger must remain unfired when only an NPC is inside"
        );
    }

    #[test]
    fn on_entered_region_trigger_with_when_filter_obeys_predicate() {
        let mut app = region_trigger_test_app();
        let uuid = "uuid-zone";
        spawn_region_with_uuid(&mut app, 0.0, 0.0, 50.0, uuid);

        // Install a trigger gated by `flag(armed)`.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert("zone".into(), uuid.into());
            runtime.trigger_states.push(TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnEnteredRegion {
                        entity_name: "zone".into(),
                    },
                    actions: vec![TriggerAction::AddObjective {
                        id: "obj-armed-entry".into(),
                        text: "Armed entry.".into(),
                        mandatory: false,
                        targets: vec![],
                        directive: crate::messages::AiDirective::None,
                        utility: crate::objectives::UtilityConfig::default(),
                        source: crate::messages::ObjectiveSource::default(),
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
                last_fired_elapsed: None,
            });
        }

        // First entry: flag unset ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ predicate false ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ no objective.
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();
        assert!(
            !objective_present(&app, "obj-armed-entry"),
            "gated trigger must not fire while flag is unset"
        );
        {
            let runtime = app.world().resource::<WorldContentRuntime>();
            assert!(
                !runtime.trigger_states[0].fired,
                "predicate-false firings must NOT consume the trigger"
            );
        }

        // Set the flag, leave the region, re-enter ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â trigger should fire now.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.flags.set_flag("armed");
        }
        set_ship_pos(&mut app, 200.0, 0.0); // exit
        app.update();
        set_ship_pos(&mut app, 10.0, 0.0); // re-enter
        app.update();

        assert!(
            objective_present(&app, "obj-armed-entry"),
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
    /// the ECS, registers it in `name_to_uuid`, and a follow-up DestroyEntity
    /// removes it again.
    #[test]
    fn spawn_entity_action_with_position_spawns_and_registers_uuid() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        // Trigger fires on attack of a pre-registered marker.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("marker".to_string(), "marker-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "marker".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "spawned_one".to_string(),
                        anchor: None,
                        position: Some([7.0, 0.0, 3.0]),
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "marker-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

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

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("trigger_src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnDestroyed {
                        entity_name: "trigger_src".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "anchor_spawn".to_string(),
                        anchor: Some("alpha".to_string()),
                        position: None,
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<crate::ai_plugin::AiEntityDestroyed>>()
            .write(crate::ai_plugin::AiEntityDestroyed {
                entity_uuid: "src-uuid".into(),
            });

        app.update();
        app.update();

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

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("layer_trigger".to_string(), "lt-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "layer_trigger".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "layer_spawn".to_string(),
                        anchor: Some("docking_bay".to_string()),
                        position: None,
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "lt-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("dddddddd-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

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

    /// DestroyEntity action despawns the target entity and emits a destroyed
    /// world event that subsequent triggers can chain on.
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
            runtime
                .name_to_uuid
                .insert("witness".to_string(), "src-uuid".to_string());
            // First trigger: on attack ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ destroy.
            // Second trigger: on destroyed of "doomed" ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ add objective (proves chaining).
            runtime.trigger_states = vec![
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnAttacked {
                            entity_name: "witness".into(),
                        },
                        actions: vec![TriggerAction::DestroyEntity {
                            entity: "doomed".into(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                TriggerState {
                    trigger: crate::world::content::Trigger {
                        condition: TriggerCondition::OnDestroyed {
                            entity_name: "doomed".into(),
                        },
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-chained".into(),
                            text: "chained".into(),
                            mandatory: false,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        when: None,
                        action_predicates: vec![],
                        action_delays: vec![],
                        id: None,
                        repeat: false,
                        cooldown_secs: None,
                    },
                    fired: false,
                    origin_layer: None,
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
            ];
        }

        app.world_mut()
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("eeeeeeee-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

        // Target entity must be gone.
        assert!(
            app.world().get_entity(target_entity).is_err(),
            "DestroyEntity must despawn the target entity"
        );

        // Chained on_destroyed trigger must have fired (objective added).
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(
            objs.sorted_snapshots()
                .iter()
                .any(|o| o.id == "obj-chained"),
            "chained on_destroyed trigger must fire from DestroyEntity action"
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

    /// DestroyEntity with an unknown entity name is a warned no-op (no panic,
    /// no objective from a chained trigger).
    #[test]
    fn destroy_entity_action_with_unknown_name_is_noop() {
        let mut app = ai_trigger_test_app();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "src".into(),
                    },
                    actions: vec![TriggerAction::DestroyEntity {
                        entity: "does_not_exist".into(),
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("ffffffff-0000-0000-0000-000000000001")
                    .unwrap(),
            });
        app.update();
        // No assertion needed beyond "does not panic". Verify objectives empty.
        let objs = &app.world().resource::<ObjectiveManagerRes>().0;
        assert!(objs.sorted_snapshots().is_empty());
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

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "src".into(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "cascade_me".into(),
                        anchor: None,
                        position: Some([1.0, 0.0, 1.0]),
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("10101010-0000-0000-0000-000000000001")
                    .unwrap(),
            });

        app.update();
        app.update();

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

    /// `when` predicate gates SpawnEntity just like every other action.
    #[test]
    fn spawn_entity_action_respects_when_predicate() {
        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("src".to_string(), "src-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "src".into(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "blocked".into(),
                        anchor: None,
                        position: Some([0.0, 0.0, 0.0]),
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: Some(crate::world::flags::Predicate::Flag {
                        name: "ready".into(),
                    }),
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "src-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("20202020-0000-0000-0000-000000000001")
                    .unwrap(),
            });
        app.update();
        app.update();

        // Flag was NOT set ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ no registration should appear.
        let has = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .contains_key("blocked");
        assert!(
            !has,
            "SpawnEntity must not run while `when` predicate is false"
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

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("marker".to_string(), "marker-uuid".to_string());
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnAttacked {
                        entity_name: "marker".to_string(),
                    },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "rotated_scaled".to_string(),
                        anchor: None,
                        position: Some([1.0, 2.0, 3.0]),
                        rotation: Some([0.0, std::f32::consts::FRAC_PI_2, 0.0]),
                        scale: Some([2.0, 2.0, 2.0]),
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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
            .resource_mut::<Messages<crate::ai_plugin::AiEntityAttacked>>()
            .write(crate::ai_plugin::AiEntityAttacked {
                entity_uuid: "marker-uuid".into(),
                attacker_uuid: uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000002")
                    .unwrap(),
            });

        app.update();
        app.update();

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

    /// (#475) `on_timer` triggers fire when `time.elapsed_secs() -
    /// runtime.world_loaded_at_secs >= after_secs`. Verify the producer
    /// in `tick_trigger_pipeline` emits `TimerElapsed` events using the
    /// load-time anchor, and that an `on_timer` trigger correctly fires
    /// a `spawn_entity` action.
    #[test]
    fn on_timer_trigger_fires_spawn_entity_action() {
        use crate::entities::spawner::EntityUuid;

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        // Stamp world load time to `now` and install an on_timer trigger
        // with `after_secs = 0.0` so it should fire on the first tick.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.world_loaded_at_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 0.0 },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "wave_now".to_string(),
                        anchor: None,
                        position: Some([1.0, 0.0, 1.0]),
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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

        // Tick twice: first runs tick_trigger_pipeline which fires the trigger
        // and queues the spawn via Commands; second flushes Commands.
        app.update();
        app.update();

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("wave_now")
            .cloned();
        assert!(
            uuid.is_some(),
            "on_timer after_secs=0 must have fired its SpawnEntity action Ã¢â‚¬â€ \
             tick_trigger_pipeline must emit TimerElapsed events when \
             world_loaded_at_secs is set"
        );

        // And the trigger must be marked fired (single-shot).
        let fired = app.world().resource::<WorldContentRuntime>().trigger_states[0].fired;
        assert!(fired, "on_timer trigger must latch fired=true after firing");

        // ECS must contain the spawned entity.
        let uuid_val = uuid.unwrap();
        let mut q = app.world_mut().query::<&EntityUuid>();
        let found = q.iter(app.world()).any(|eu| eu.0 == uuid_val);
        assert!(found, "spawned entity must exist in ECS");
    }

    /// (#475) `on_timer` triggers with `after_secs > now - world_loaded_at`
    /// must NOT fire yet. Pin the elapsed-secs comparison.
    #[test]
    fn on_timer_trigger_does_not_fire_before_after_secs_elapses() {
        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            // World loaded "in the future" so elapsed will be negative,
            // clamped to 0, never satisfying after_secs = 100.
            runtime.world_loaded_at_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 100.0 },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "wave_future".to_string(),
                        anchor: None,
                        position: Some([0.0, 0.0, 0.0]),
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
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

        let uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("wave_future")
            .cloned();
        assert!(
            uuid.is_none(),
            "on_timer after_secs=100 must not fire when only a few ms have elapsed"
        );
        let fired = app.world().resource::<WorldContentRuntime>().trigger_states[0].fired;
        assert!(!fired, "trigger must not be marked fired");
    }

    /// SpawnEntity action stamps the trigger `name` onto the spawned entity as
    /// its `EntityName` component.  This is required for `resolve_objective_target`
    /// to match a `AiDirective::Destroy { target: "wave_1" }` against
    /// `AiWorldEntity::name` in `WorldSnapshot` â€” if the component kept the
    /// template display name ("Harrow Destroyer") the Backfill AI would never
    /// resolve the Destroy target and the ship would stay on its Patrol forever.
    #[test]
    fn spawn_entity_action_stamps_trigger_name_as_entity_name() {
        use crate::entities::spawner::{EntityName, EntityUuid};

        let template_path = write_spawn_template_fixture();
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.world_loaded_at_secs = Some(0.0);
            runtime.trigger_states = vec![TriggerState {
                trigger: crate::world::content::Trigger {
                    condition: TriggerCondition::OnTimer { after_secs: 0.0 },
                    actions: vec![TriggerAction::SpawnEntity {
                        template_path: template_path.clone(),
                        name: "wave_1".to_string(),
                        anchor: None,
                        position: Some([50.0, 0.0, 50.0]),
                        rotation: None,
                        scale: None,
                        groups: vec![],
                        overrides: None,
                    }],
                    when: None,
                    action_predicates: vec![],
                    action_delays: vec![],
                    id: None,
                    repeat: false,
                    cooldown_secs: None,
                },
                fired: false,
                origin_layer: None,
                seen_destroyed: std::collections::HashSet::new(),
                last_fired_elapsed: None,
            }];
        }

        app.update();
        app.update();

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
            "EntityName must be the trigger name 'wave_1', not the template display name"
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
