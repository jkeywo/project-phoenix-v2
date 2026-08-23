use super::*;
use crate::ai::server::{AiEntityAttacked, AiEntityDestroyed};
use crate::comms::server::{CommsChannel2Event, CommsInboxRes, CommsRuntime};
use crate::console::comms::server::handle_comms_channel2;
use crate::core::messages::*;
use crate::lobby::LobbyPlugin;
use crate::world::content::TriggerCondition;

// ── Embedded Rhai script fixtures (issues #984/#1033, hoisted #1191) ──────
//
// Each constant is the exact literal a test used to author inline —
// unmoved, unedited, just named and lifted out of the test body it was
// authored beside. Two fixtures (`rhai_chain_link`, `script_when_gated_spawn`)
// are named helper FNS instead of consts, because their call sites build the
// string with `format!`, whose template must be a literal at that call site —
// see each fn's own doc comment.

/// `scripted_in_seconds_effect_routes_through_tick_delayed_actions`: a bare
/// Rhai handler fn, compiled directly (no `[script]` TOML wrapper).
const RHAI_IN_SECONDS_COMPLETE_OBJECTIVE: &str = r#"fn on_x(ctx) {
                ctx.schedule.in_seconds(0).complete_objective("aphelion_secured");
            }"#;

/// `scripted_on_destroyed_completes_objective_through_the_live_pipeline`.
const SCRIPT_ON_DESTROYED_COMPLETES_OBJECTIVE: &str = r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.effects.complete_objective("obj"); }'
"#;

/// `scripted_open_comms_queues_on_the_runtime_through_the_live_pipeline`.
const SCRIPT_ON_DESTROYED_OPENS_COMMS: &str = r#"[script]
setup = 'on_destroyed("raider", "hail"); fn hail(ctx) { ctx.effects.open_comms(#{ from: "axiom", node_fn: "hail_axiom", display_name: "Axiom Control", urgent: true }); } fn hail_axiom(ctx) { #{ message: "Axiom Station, go ahead.", responses: [] } }'
"#;

/// `scripted_flag_write_chains_a_declarative_on_flag_set`.
const SCRIPT_ON_DESTROYED_SETS_FLAG: &str = r#"[script]
setup = 'on_destroyed("raider", "arm"); fn arm(ctx) { ctx.flags.armed = 1; }'
"#;

/// `scripted_flag_clear_chains_a_declarative_on_flag_cleared`.
const SCRIPT_ON_DESTROYED_CLEARS_FLAG: &str = r#"[script]
setup = 'on_destroyed("raider", "disarm"); fn disarm(ctx) { ctx.flags.shields_up = 0; }'
"#;

/// `scripted_flag_increment_chains_a_declarative_on_flag_set`.
const SCRIPT_ON_DESTROYED_INCREMENTS_FLAG: &str = r#"[script]
setup = 'on_destroyed("raider", "bump"); fn bump(ctx) { ctx.flags.increment("wave", 1); }'
"#;

/// `scripted_and_dispatched_spawn_mint_the_same_entity_uuid`.
const SCRIPT_ON_DESTROYED_SPAWNS_WAVE: &str = r#"[script]
setup = 'on_destroyed("raider", "spawn_wave"); fn spawn_wave(ctx) { ctx.effects.spawn_entity(#{ template_path: "fixture/harrow_mint.toml", name: "wave", position: [100, 0, 0], groups: ["hostiles"] }); }'
"#;

/// `a_scripted_destroy_chains_on_destroyed_in_the_same_tick`.
const SCRIPT_ON_DESTROYED_DESTROYS_SKYHOOK: &str = r#"[script]
setup = 'on_destroyed("raider", "collapse"); fn collapse(ctx) { ctx.effects.destroy_entity("skyhook"); }'
"#;

/// `a_scripted_destroy_of_the_last_group_member_fires_on_all_destroyed`.
const SCRIPT_ON_DESTROYED_KILLS_GROUP_MEMBERS: &str = r#"[script]
setup = '''
on_destroyed("cue_a", "kill_a");
on_destroyed("cue_b", "kill_b");
fn kill_a(ctx) { ctx.effects.destroy_entity("band_a"); }
fn kill_b(ctx) { ctx.effects.destroy_entity("band_b"); }
'''
"#;

/// `a_scripted_destroy_of_an_unknown_name_warns_and_keeps_the_rest_of_the_call`.
const SCRIPT_ON_DESTROYED_DESTROYS_UNKNOWN_NAME: &str = r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.effects.destroy_entity("no_such_entity"); ctx.effects.complete_objective("obj"); }'
"#;

/// `a_name_freed_by_a_scripted_destroy_is_reusable_by_a_later_spawn`.
const SCRIPT_ON_DESTROYED_RECYCLES_NAME: &str = r#"[script]
setup = '''
on_destroyed("cue", "recycle");
fn recycle(ctx) {
ctx.effects.destroy_entity("band");
ctx.effects.spawn_entity(#{ template_path: "fixture/reuse_1033.toml", name: "band", position: [10, 0, 0] });
}
'''
"#;

/// `compile_and_init_wire_a_scripted_trigger_into_the_runtime`. Same handler
/// as `SCRIPT_ON_DESTROYED_COMPLETES_OBJECTIVE` but wrapped as a whole world
/// TOML (parsed with `parse_world` / `toml::from_str`, not
/// `compile_fixture_scripts`) — kept as its own constant rather than shared,
/// so the two call sites' framing stays independent.
const WORLD_TOML_SCRIPTED_ON_DESTROYED_COMPLETES_OBJECTIVE: &str = r#"
[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.effects.complete_objective("obj"); }'
"#;

/// `scripted_callback_fires_after_the_delay_not_immediately`.
const SCRIPT_ON_DESTROYED_SCHEDULES_CALLBACK: &str = r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("obj"); }); }'
"#;

/// `scripted_callback_can_reschedule_another_callback`.
const SCRIPT_CALLBACK_RESCHEDULES_ANOTHER: &str = r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("first"); ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("second"); }); }); }'
"#;

/// `scripted_callback_rescheduled_at_delay_zero_defers_to_the_next_tick`.
const SCRIPT_CALLBACK_RESCHEDULE_AT_DELAY_ZERO: &str = r#"[script]
setup = 'on_destroyed("raider", "k"); fn k(ctx) { ctx.schedule.after(2, |ctx| { ctx.effects.complete_objective("first"); ctx.schedule.after(0, |ctx| { ctx.effects.complete_objective("second"); }); }); }'
"#;

/// `trigger_chain_exceeding_max_passes_stops_at_the_cap`: the seed link,
/// fired by the external `WorldLoaded` event.
const RHAI_CHAIN_SEED_LINK: &str =
    r#"on_world_loaded("h0"); fn h0(ctx) { ctx.flags.chain_1 = 1; }"#;

/// `trigger_chain_exceeding_max_passes_stops_at_the_cap`: the per-link Rhai
/// fragment appended `chain_len - 1` times — link `i` fires on `chain_i` and
/// sets `chain_{i + 1}`. A helper fn rather than a `const`, since `format!`'s
/// template must be a literal at its own call site — the template text still
/// lives here, named, instead of inline in the test.
fn rhai_chain_link(i: usize) -> String {
    format!(
        r#" on_flag_set("chain_{i}", "h{i}"); fn h{i}(ctx) {{ ctx.flags.chain_{next} = 1; }}"#,
        next = i + 1
    )
}

/// `when_predicate_suppresses_a_scripted_spawn`: formatted with the fixture
/// template's own temp-file path. A helper fn for the same `format!`-literal
/// reason as [`rhai_chain_link`].
fn script_when_gated_spawn(template_path: &str) -> String {
    format!(
        r#"[script]
setup = 'on_attacked("src", "spawn_it").when("flag(ready)"); fn spawn_it(ctx) {{ ctx.effects.spawn_entity(#{{ template_path: "{template_path}", name: "blocked", position: [0, 0, 0] }}); }}'
"#
    )
}

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
        .add_plugins(crate::ai::server::AiPlugin)
        .insert_resource(crate::entities::config_cache::FactionRegistryResource(
            crate::entities::config_cache::get_faction_registry(),
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
        .compile(RHAI_IN_SECONDS_COMPLETE_OBJECTIVE)
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
        &crate::dossier::evidence::EvidenceLog::default(),
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

/// Build a `WorldScriptRuntime` from an inline-`[script]` fixture world the
/// SAME way `compile_world_scripts` does in production, through the single
/// fixture compiler (`world::script::fixture`, issue #1215).
fn compile_fixture_scripts(world_toml: &str) -> WorldScriptRuntime {
    crate::world::script::fixture::compile_world_runtime("fixture/scripted.toml", world_toml)
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_COMPLETES_OBJECTIVE);
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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_OPENS_COMMS);

    let mut app = ai_trigger_test_app();
    let raider_uuid = "raider-uuid-984c";
    {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime
            .name_to_uuid
            .insert("raider".to_string(), raider_uuid.to_string());
        runtime.trigger_states = Vec::new();
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_SETS_FLAG);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_CLEARS_FLAG);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_INCREMENTS_FLAG);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    use crate::entities::config::EntityConfig;

    // A trivial template both paths spawn, served from the native config cache
    // so the live `WasmTemplateLoader` resolves it with no files on disk.
    crate::entities::config_cache::insert_native_config(
        "fixture/harrow_mint.toml".to_string(),
        EntityConfig::from_toml("").unwrap(),
    );

    let raider_uuid = "raider-uuid-mint";

    // ---- Scripted: the handler spawns "wave" from a `#{ … }` map. ----
    let scripted_uuid = {
        let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_SPAWNS_WAVE);
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
            merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_DESTROYS_SKYHOOK);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
        .resource::<Messages<crate::ai::server::AiEntityDestroyed>>();
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_KILLS_GROUP_MEMBERS);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_DESTROYS_UNKNOWN_NAME);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    use crate::entities::config::EntityConfig;
    crate::entities::config_cache::insert_native_config(
        "fixture/reuse_1033.toml".to_string(),
        EntityConfig::from_toml("").unwrap(),
    );

    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_RECYCLES_NAME);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let world_toml = WORLD_TOML_SCRIPTED_ON_DESTROYED_COMPLETES_OBJECTIVE;
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
    let mut sr = compile_fixture_scripts(SCRIPT_ON_DESTROYED_SCHEDULES_CALLBACK);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_CALLBACK_RESCHEDULES_ANOTHER);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
    let mut sr = compile_fixture_scripts(SCRIPT_CALLBACK_RESCHEDULE_AT_DELAY_ZERO);

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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
        .resource_mut::<Messages<crate::ai::server::AiWaypointReached>>()
        .write(crate::ai::server::AiWaypointReached {
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
        .resource_mut::<Messages<crate::ai::server::AiWaypointReached>>()
        .write(crate::ai::server::AiWaypointReached {
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
        .resource_mut::<Messages<crate::ai::server::AiWaypointReached>>()
        .write(crate::ai::server::AiWaypointReached {
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
        .resource_mut::<Messages<crate::ai::server::AiWaypointReached>>()
        .write(crate::ai::server::AiWaypointReached {
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
            slot: crate::core::messages::ModifierSlot::MaxSpeed,
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
        npc_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed) > 1.0,
        "ApplyModifier must land on the target NPC's per-entity component; got {}",
        npc_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed)
    );
    assert!(
        (player_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
        "player entity must be unaffected by an NPC-targeted ApplyModifier; got {}",
        player_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed)
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
                slot: crate::core::messages::ModifierSlot::MaxSpeed,
                bonus: 2.0,
            },
            TriggerAction::RemoveModifier {
                entity: "raider_alpha".into(),
                tag: "boost".into(),
                slot: crate::core::messages::ModifierSlot::MaxSpeed,
            },
        ],
    );
    let npc_mods = app
        .world()
        .entity(npc)
        .get::<crate::modifiers::ShipModifiers>()
        .expect("NPC entity must carry ShipModifiers");
    let value = npc_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed);
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
            kind: crate::core::messages::FlagKind::CommsJammed,
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
        npc_mods.has_flag(&crate::core::messages::FlagKind::CommsJammed),
        "ApplyFlag must register on the target NPC's per-entity component"
    );
    assert!(
        !player_mods.has_flag(&crate::core::messages::FlagKind::CommsJammed),
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
                kind: crate::core::messages::FlagKind::CommsJammed,
            },
            TriggerAction::RemoveFlag {
                entity: "raider_alpha".into(),
                tag: "jammer".into(),
                kind: crate::core::messages::FlagKind::CommsJammed,
            },
        ],
    );
    let npc_mods = app
        .world()
        .entity(npc)
        .get::<crate::modifiers::ShipModifiers>()
        .unwrap();
    assert!(
        !npc_mods.has_flag(&crate::core::messages::FlagKind::CommsJammed),
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
            slot: crate::core::messages::ModifierSlot::MaxSpeed,
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
        (npc_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
        "unknown entity name must not touch any entity's per-entity component (NPC)"
    );
    assert!(
        (player_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
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
            slot: crate::core::messages::ModifierSlot::MaxSpeed,
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
        (npc_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
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
            slot: crate::core::messages::ModifierSlot::MaxSpeed,
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
        (npc_mods.get(&crate::core::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
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
    use crate::entities::config::BehaviourConfig;

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
            .resource::<crate::entities::config_cache::FactionRegistryResource>()
            .0;
        assert!(
            !crate::ai::faction::is_enemy(
                Some(fed_faction_uuid()),
                Some(harrow_faction_uuid()),
                reg
            ),
            "precondition: Federation must not consider Harrow hostile by default"
        );
        assert!(
            !crate::ai::faction::is_enemy(
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
        .resource::<crate::entities::config_cache::FactionRegistryResource>()
        .0;
    assert!(
        crate::ai::faction::is_enemy(Some(fed_faction_uuid()), Some(harrow_faction_uuid()), reg),
        "Federation must consider Harrow hostile after add_faction_enemy"
    );
    assert!(
        crate::ai::faction::is_enemy(Some(harrow_faction_uuid()), Some(fed_faction_uuid()), reg),
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
        .resource::<crate::entities::config_cache::FactionRegistryResource>()
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
        .resource::<crate::entities::config_cache::FactionRegistryResource>()
        .0;
    assert!(
        !crate::ai::faction::is_enemy(Some(harrow_faction_uuid()), Some(fed_faction_uuid()), reg),
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
    use crate::entities::config::BehaviourConfig;

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
            crate::console::weapons::TacticalRadarSelection::default(),
        ))
        .id();

    // First update: register the AI token for the spawned NPC.
    app.update();

    // Manually seed the engagement: the NPC has locked the player.
    {
        let mut lock = app
            .world_mut()
            .get_mut::<crate::console::weapons::TacticalRadarSelection>(npc_entity)
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
        .get::<crate::console::weapons::TacticalRadarSelection>(npc_entity)
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
    app.insert_resource(WorldResource(crate::core::messages::WorldData::default()));
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
    use crate::entities::config::EntityConfig;
    use crate::entities::spawner::EntityUuid;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
    use crate::entities::config::EntityConfig;
    use crate::world::config::WorldConfig as UnifiedWorldConfig;
    use crate::world::config::WorldEntity;
    use std::collections::HashMap;

    crate::entities::config_cache::clear_template_preload_state();
    crate::entities::config_cache::record_raw_template(
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
    crate::entities::config_cache::record_raw_template(
        "fixture/broken.toml",
        "class = \"ok\"\n".into(),
    );
    assert_eq!(spawn(&world_cfg), 2, "the control world must spawn both");

    crate::entities::config_cache::clear_template_preload_state();
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
    use crate::entities::config::EntityConfig;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
    use crate::entities::config::EntityConfig;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
    use crate::entities::config::EntityConfig;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
    use crate::entities::config::EntityConfig;
    use crate::entities::spawner::BehaviourSection;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
    use crate::entities::config::{
        AsteroidFieldConfig, AsteroidFieldShape, EntityConfig, GridConfig,
    };
    use crate::entities::spawner::AsteroidFieldSection;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
    use crate::entities::spawner::AsteroidFieldSection;
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

    let cache = crate::entities::config_cache::ConfigCache::from(HashMap::new());
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

    let cache = crate::entities::config_cache::ConfigCache::from(HashMap::new());
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
    use crate::entities::config::{
        AsteroidFieldConfig, AsteroidFieldShape, EntityConfig, GridConfig,
    };
    use crate::entities::spawner::AsteroidFieldSection;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
/// One half of the layer contract's observables: a loaded layer contributes
/// ENTITIES (this) and, since script-in-layers (#1045), the trigger states and
/// handlers its `[script]` block compiles to. See
/// `tests/fixtures/layer_entities.toml`.
fn layer_entity_count(app: &App, path: &str) -> usize {
    app.world()
        .resource::<WorldLayerMap>()
        .0
        .get(path)
        .map(|layer| layer.spawned_entities.len())
        .unwrap_or(0)
}

// ── Script-in-layers (issue #1045) ───────────────────────────────────────────

const LAYER_A: &str = "tests/fixtures/layer_scripted_a.toml";
const LAYER_B: &str = "tests/fixtures/layer_scripted_b.toml";

/// `layer_test_app` plus the trigger pipeline, so a layer's merged scripted
/// trigger can actually be seen to FIRE rather than merely to be present.
///
/// `apply_world_layer_changes` is ordered before `collect_world_events` so a
/// layer that lands this tick has its `WorldLoaded` drained in the same tick it
/// was queued.
fn scripted_layer_test_app() -> App {
    let mut app = ai_trigger_test_app();
    app.init_resource::<WorldLayerMap>()
        .init_resource::<PendingWorldLayerChanges>()
        .add_systems(
            Update,
            apply_world_layer_changes.before(collect_world_events),
        );
    app
}

/// Queue a `Load` for `path` and step until it lands in the layer map.
///
/// Two ticks rather than one for the FIRST scripted layer of a session: the
/// applier inserts an empty `WorldScriptRuntime` and re-queues, so the merge (and
/// the `WorldLoaded` it fires from) happens on the tick after. See
/// `apply_world_layer_changes`' docs;
/// `the_first_scripted_layer_lands_a_tick_after_it_is_queued` pins that directly.
fn load_layer(app: &mut App, path: &str) {
    app.world_mut()
        .resource_mut::<PendingWorldLayerChanges>()
        .0
        .push(WorldLayerChange::Load {
            path: path.into(),
            loader_path: None,
        });
    for _ in 0..3 {
        app.update();
        if app.world().resource::<WorldLayerMap>().0.contains_key(path) {
            return;
        }
    }
    panic!("layer {path} never landed in WorldLayerMap");
}

/// Queue an `Unload` for `path` and step one tick.
fn unload_layer(app: &mut App, path: &str) {
    app.world_mut()
        .resource_mut::<PendingWorldLayerChanges>()
        .0
        .push(WorldLayerChange::Unload(path.into()));
    app.update();
}

/// The base-world flag store's value for `name` (scripted writes land here —
/// a script emits `MutateFlag { target_layer: None }`).
fn base_counter(app: &App, name: &str) -> i64 {
    app.world()
        .resource::<WorldContentRuntime>()
        .flags
        .counter(name)
}

/// The acceptance case (issue #1045): a layer that authors a `[script]` block
/// has it COMPILED at load, merged into the live runtime, and its
/// `on_world_loaded` handler fires from the `WorldLoaded` the load queues —
/// exactly as the same script would on the standalone/base-world path.
///
/// Before this issue the compiled set reached the applier and was dropped, so
/// the flag below stayed 0 and a scripted layer was silently inert.
#[test]
fn a_layer_script_compiles_and_its_trigger_fires_after_load() {
    let mut app = scripted_layer_test_app();
    load_layer(&mut app, LAYER_A);

    assert_eq!(
        base_counter(&app, "layer_a_loaded"),
        1,
        "the layer's on_world_loaded handler must run"
    );

    let sr = app.world().resource::<WorldScriptRuntime>();
    assert!(
        sr.asts.contains_key(&format!("{LAYER_A}#script.setup")),
        "the layer's unit is retained under its virtual path: {:?}",
        sr.asts.keys().collect::<Vec<_>>()
    );
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(
        runtime.trigger_states.len(),
        2,
        "on_world_loaded + on_flag_set"
    );
    assert!(
        runtime
            .trigger_states
            .iter()
            .all(|s| s.origin_layer.as_deref() == Some(LAYER_A)),
        "each merged state is tagged with the layer that brought it"
    );
    assert_eq!(
        sr.handlers.len(),
        runtime.trigger_states.len(),
        "handlers stays index-aligned with trigger_states"
    );
}

/// A script-free base world has no `WorldScriptRuntime` at all, so the first
/// scripted layer to arrive inserts an empty one and waits a tick to merge into
/// it (issue #1045).
///
/// The wait is the design, not an accident: a `Commands` insert lands at the
/// schedule's sync point, so merging in the same body would publish the layer's
/// trigger states and its `WorldLoaded` a tick before the `handlers` describing
/// them — and the pipeline would latch each `on_world_loaded` as fired against an
/// absent runtime, losing the layer's opening handler outright.
#[test]
fn the_first_scripted_layer_lands_a_tick_after_it_is_queued() {
    let mut app = scripted_layer_test_app();
    assert!(
        app.world().get_resource::<WorldScriptRuntime>().is_none(),
        "a script-free world starts with no script runtime"
    );
    app.world_mut()
        .resource_mut::<PendingWorldLayerChanges>()
        .0
        .push(WorldLayerChange::Load {
            path: LAYER_A.into(),
            loader_path: None,
        });

    app.update();
    assert!(
        app.world().get_resource::<WorldScriptRuntime>().is_some(),
        "tick one inserts the landing pad"
    );
    assert!(
        !app.world()
            .resource::<WorldLayerMap>()
            .0
            .contains_key(LAYER_A),
        "and re-queues the layer rather than merging it"
    );
    assert_eq!(base_counter(&app, "layer_a_loaded"), 0);

    app.update();
    assert!(
        app.world()
            .resource::<WorldLayerMap>()
            .0
            .contains_key(LAYER_A),
        "tick two merges it"
    );
    assert_eq!(
        base_counter(&app, "layer_a_loaded"),
        1,
        "and its opening handler runs on the same tick its states appear"
    );
}

/// Two scripted layers queued in ONE batch both merge, and into the same runtime.
#[test]
fn two_scripted_layers_in_one_batch_both_merge() {
    let mut app = scripted_layer_test_app();
    {
        let mut pending = app.world_mut().resource_mut::<PendingWorldLayerChanges>();
        pending.0.push(WorldLayerChange::Load {
            path: LAYER_A.into(),
            loader_path: None,
        });
        pending.0.push(WorldLayerChange::Load {
            path: LAYER_B.into(),
            loader_path: None,
        });
    }
    // Tick one inserts the landing pad and re-queues both; tick two merges them.
    app.update();
    app.update();

    let sr = app.world().resource::<WorldScriptRuntime>();
    assert_eq!(sr.handlers.len(), 4, "two triggers from each layer");
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len(),
        4
    );
    assert_eq!(base_counter(&app, "layer_a_loaded"), 1);
    assert_eq!(base_counter(&app, "layer_b_loaded"), 1);
}

/// THE parallel-vec guard (issue #1045 acceptance): unloading a layer whose
/// trigger states sit BEFORE another layer's must take their `handlers` entries
/// with them.
///
/// Filtering `trigger_states` alone leaves `handlers` one entry too long and
/// shifted, so the surviving trigger at index 0 resolves the UNLOADED layer's
/// handler — `layer_a_probed` would be written instead of `layer_b_probed`.
/// Both the structural check and the behavioural probe below catch that.
#[test]
fn unloading_a_layer_keeps_handlers_aligned_for_the_layers_that_remain() {
    let mut app = scripted_layer_test_app();
    load_layer(&mut app, LAYER_A);
    load_layer(&mut app, LAYER_B);
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .trigger_states
            .len(),
        4,
        "two triggers per layer"
    );

    unload_layer(&mut app, LAYER_A);

    // Structural: the two vecs are the same length, and every survivor is B's.
    let sr = app.world().resource::<WorldScriptRuntime>();
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.trigger_states.len(), 2, "only B's triggers remain");
    assert_eq!(
        sr.handlers.len(),
        runtime.trigger_states.len(),
        "handlers must shrink WITH trigger_states, not stay behind"
    );
    assert!(
        runtime
            .trigger_states
            .iter()
            .all(|s| s.origin_layer.as_deref() == Some(LAYER_B)),
        "A's states are gone"
    );
    assert!(
        sr.handlers.iter().all(|h| h
            .as_ref()
            .is_some_and(|h| h.fn_name.starts_with("layer_b_"))),
        "and so are A's handlers: {:?}",
        sr.handlers
    );
    assert!(
        !sr.asts.contains_key(&format!("{LAYER_A}#script.setup")),
        "A's unit is retracted too"
    );
    assert!(
        sr.asts.contains_key(&format!("{LAYER_B}#script.setup")),
        "B's is not"
    );

    // Behavioural: probe the surviving trigger and see WHICH handler runs. A
    // desynced `handlers` would run A's here.
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .pending_world_events
        .push(WorldEvent::FlagSet {
            name: "probe".into(),
            origin_layer: None,
        });
    app.update();

    assert_eq!(
        base_counter(&app, "layer_b_probed"),
        1,
        "the surviving trigger must resolve its OWN handler"
    );
    assert_eq!(
        base_counter(&app, "layer_a_probed"),
        0,
        "and never the unloaded layer's"
    );
}

/// The PURE half of the same guard, over the two functions that own both vecs:
/// a BASE world's triggers, then two layers', then a removal of the MIDDLE one.
///
/// The App test above cannot reach this shape — its base world is script-free —
/// and it is the shape that matters most: base-world entries are the ones no
/// unload can ever retract, so a filter that shifted them would silently repoint
/// the base world's own handlers.
#[test]
fn removing_a_layers_triggers_keeps_every_survivors_own_handler() {
    fn staged(source: &str, handler: &str) -> ScriptTrigger {
        ScriptTrigger {
            trigger: crate::world::config::scripted_trigger(TriggerCondition::OnWorldLoaded),
            handler: handler.to_string(),
            source_path: source.to_string(),
        }
    }
    fn handler_names(sr: &WorldScriptRuntime) -> Vec<&str> {
        sr.handlers
            .iter()
            .map(|h| {
                h.as_ref()
                    .expect("every index is scripted")
                    .fn_name
                    .as_str()
            })
            .collect()
    }

    let mut states: Vec<TriggerState> = Vec::new();
    let mut sr = WorldScriptRuntime::empty();

    sr.triggers = vec![
        staged("base.toml#script.setup", "base_a"),
        staged("base.toml#script.setup", "base_b"),
    ];
    merge_script_triggers(&mut states, &mut sr, None);
    sr.triggers = vec![staged("l1.toml#script.setup", "l1_a")];
    merge_script_triggers(&mut states, &mut sr, Some("l1.toml"));
    sr.triggers = vec![staged("l2.toml#script.setup", "l2_a")];
    merge_script_triggers(&mut states, &mut sr, Some("l2.toml"));

    assert_eq!(states.len(), 4);
    assert_eq!(sr.handlers.len(), 4, "one handler per appended state");

    assert_eq!(
        remove_layer_script_triggers(&mut states, &mut sr.handlers, "l1.toml"),
        1
    );
    assert_eq!(states.len(), 3);
    assert_eq!(
        sr.handlers.len(),
        3,
        "handlers shrinks with the table, not after it"
    );
    assert_eq!(
        handler_names(&sr),
        vec!["base_a", "base_b", "l2_a"],
        "every survivor kept the handler it arrived with"
    );
    assert_eq!(
        states
            .iter()
            .map(|s| s.origin_layer.as_deref())
            .collect::<Vec<_>>(),
        vec![None, None, Some("l2.toml")]
    );

    // Unloading a path that contributed nothing takes nothing.
    assert_eq!(
        remove_layer_script_triggers(&mut states, &mut sr.handlers, "never_loaded.toml"),
        0
    );
    assert_eq!(states.len(), 3);
    assert_eq!(sr.handlers.len(), 3);
}

/// The shipped-set guard: a script-free layer — `reinforcements.toml` and every
/// other layer any shipped world loads — conjures no script runtime and
/// contributes no trigger state, exactly as before issue #1045.
#[test]
fn a_scriptless_layer_still_merges_nothing() {
    let mut app = scripted_layer_test_app();
    load_layer(&mut app, "tests/fixtures/layer_entities.toml");
    assert!(
        app.world().get_resource::<WorldScriptRuntime>().is_none(),
        "no [script] block, no runtime"
    );
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .trigger_states
        .is_empty());
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
        .add_message::<crate::ai::server::AiEntityAttacked>()
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_message::<crate::ai::server::AiWaypointReached>()
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
    let mut setup = String::from(RHAI_CHAIN_SEED_LINK);
    for i in 1..chain_len {
        setup.push_str(&rhai_chain_link(i));
    }
    let mut sr = compile_fixture_scripts(&format!("[script]\nsetup = '{setup}'\n"));
    assert_eq!(sr.triggers.len(), chain_len, "one trigger per chain link");

    let mut app = ai_trigger_test_app();
    {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        runtime.trigger_states = Vec::new();
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
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
        .init_resource::<crate::server_app::GameOverReason>();

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
                text_params: Default::default(),
                mandatory: false,
                targets: vec![],
                directive: crate::core::messages::AiDirective::None,
                utility: crate::objectives::UtilityConfig::default(),
                source: crate::core::messages::ObjectiveSource::default(),
                command_stance: None,
            },
            TriggerAction::CompleteObjective {
                id: "obj-alpha".into(),
            },
            TriggerAction::AddObjective {
                id: "obj-beta".into(),
                text: "Add then fail".into(),
                text_params: Default::default(),
                mandatory: false,
                targets: vec![],
                directive: crate::core::messages::AiDirective::None,
                utility: crate::objectives::UtilityConfig::default(),
                source: crate::core::messages::ObjectiveSource::default(),
                command_stance: None,
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
                slot: crate::core::messages::ModifierSlot::MaxSpeed,
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
                kind: crate::core::messages::FlagKind::CommsJammed,
            },
            TriggerAction::RemoveModifier {
                entity: "target_ship".into(),
                tag: "boost".into(),
                slot: crate::core::messages::ModifierSlot::MaxSpeed,
            },
            TriggerAction::RemoveIntModifier {
                entity: "target_ship".into(),
                tag: "crew".into(),
                slot: crate::modifiers::IntModifierSlot::RepairTeams,
            },
            TriggerAction::RemoveFlag {
                entity: "target_ship".into(),
                tag: "jammer".into(),
                kind: crate::core::messages::FlagKind::CommsJammed,
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
        (mods.get(&crate::core::messages::ModifierSlot::MaxSpeed) - 1.0).abs() < 1e-3,
        "RemoveModifier must undo the earlier ApplyModifier"
    );
    assert_eq!(
        mods.get_int(&crate::modifiers::IntModifierSlot::RepairTeams),
        0,
        "RemoveIntModifier must undo the earlier ApplyIntModifier"
    );
    assert!(
        !mods.has_flag(&crate::core::messages::FlagKind::CommsJammed),
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
        .resource::<crate::entities::config_cache::FactionRegistryResource>()
        .0;
    assert!(
        !crate::ai::faction::is_enemy(
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
            .resource::<crate::server_app::GameOverReason>()
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

use crate::entities::config::EntityConfig;
use crate::entities::spawner::{spawn_entity, EntityUuid};
use crate::regions::shape::RegionShape;

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
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        Transform::default(),
        crate::ship::state::ShipPhysics::default(),
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::ship_plugin::ShipSystemControlSources::default(),
        crate::modifiers::ShipModifiers::new(),
    ));
    app
}

fn spawn_region_with_uuid(app: &mut App, x: f32, z: f32, radius: f32, uuid: &str) -> Entity {
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
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
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
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
        .query_filtered::<&mut crate::ship::state::ShipPhysics, With<crate::server_app::LocalShip>>(
        );
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
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
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
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
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
        .resource::<Messages<crate::ai::server::AiEntityDestroyed>>();
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
    let mut sr =
        compile_fixture_scripts(&script_when_gated_spawn(&template_path.replace('\\', "/")));
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
        merge_script_triggers(&mut runtime.trigger_states, &mut sr, None);
    }
    app.world_mut().insert_resource(sr);

    app.world_mut()
        .resource_mut::<Messages<crate::ai::server::AiEntityAttacked>>()
        .write(crate::ai::server::AiEntityAttacked {
            entity_uuid: "src-uuid".into(),
            attacker_uuid: uuid::Uuid::parse_str("20202020-0000-0000-0000-000000000001").unwrap(),
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
fn single_template_cache(path: &str) -> crate::entities::config_cache::ConfigCache {
    use crate::entities::config::EntityConfig;
    use std::collections::HashMap;
    let mut m: HashMap<String, EntityConfig> = HashMap::new();
    m.insert(path.into(), EntityConfig::from_toml("").unwrap());
    crate::entities::config_cache::ConfigCache::from(m)
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
    use crate::entities::spawner::EntityUuid;
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

    use crate::entities::config::EntityConfig;
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
    let cache = crate::entities::config_cache::ConfigCache::from(m);

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
        .get::<crate::entities::spawner::EntityUuid>(spawned_low[0])
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
        .get::<crate::entities::spawner::EntityUuid>(spawned_high[0])
        .unwrap();
    assert_eq!(uuid_high.0, "heavy-uuid");
}

/// Composed predicate: `counter(ship_power) >= 200 or flag(always_spawn)`
/// — entity spawns when either condition is true.
#[test]
fn spawn_game_start_entity_composed_predicate() {
    use crate::entities::spawner::EntityUuid;
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
