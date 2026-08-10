//! The Rhai trigger front-end (issue #980, milestone M2).
//!
//! One registered *loading-engine* host function per
//! [`TriggerCondition`](crate::world::config::TriggerCondition) variant. A unit's
//! top level calls them to author triggers in script:
//!
//! ```rhai
//! on_destroyed("raider", "on_raider_dead");
//! on_all_destroyed("wave_1", 5, "wave_one_cleared");
//! on_world_loaded("arm_the_rivalry");
//! ```
//!
//! Each call builds the *same* [`Trigger`](crate::world::config::Trigger) struct
//! the TOML `[[trigger]]` front-end builds — through the one shared
//! [`scripted_trigger`](crate::world::config::scripted_trigger) constructor — and
//! records the named handler fn that supplies its effects. The built triggers
//! feed the existing evaluator ([`crate::world::content`]) and pipeline
//! (`tick_trigger_pipeline`) exactly as TOML-authored ones do: **one evaluator,
//! two front-ends** (settled decision 5). Nothing here executes a handler — that
//! is the runtime host's job ([`super::engine::RuntimeHost`]); the loading engine
//! only *collects* what a top level authored (`Engine::run_ast` never runs fn
//! bodies), so a handler's effect calls compile here and resolve at runtime.
//!
//! # Integer-only surface (`no_float`)
//!
//! The whole script API is integer-only. The two conditions with a float field —
//! `OnTimer { after_secs }` and `OnAllDestroyed { after_secs }` — take an `INT`
//! (seconds) at the host-fn boundary and convert to `f32` there, exactly as the
//! roadmap's "floats convert at the host-fn boundary" rule requires. Authored
//! `after_secs` in TOML are whole seconds in every shipped world, so a scripted
//! trigger and its TOML equivalent build the identical `f32`.

use std::sync::{Arc, Mutex};

use rhai::{Engine, ImmutableString};

use crate::world::config::{scripted_trigger, TriggerCondition};
use crate::world::script::engine::{BuilderState, ScriptTrigger};

/// Record one script-authored trigger against the unit currently running.
fn push_trigger(state: &Mutex<BuilderState>, condition: TriggerCondition, handler: &str) {
    let mut s = state.lock().expect("builder state lock");
    let source_path = s.current_path.clone();
    s.script_triggers.push(ScriptTrigger {
        trigger: scripted_trigger(condition),
        handler: handler.to_string(),
        source_path,
    });
}

/// Register the typed trigger-builder host functions on a loading engine.
///
/// Called once from [`super::engine::loading_engine`]. Every closure captures a
/// clone of the shared [`BuilderState`] handle, so a top-level call attributes
/// its trigger to whichever unit the loader is currently running.
pub fn register_trigger_builders(engine: &mut Engine, state: Arc<Mutex<BuilderState>>) {
    // 1. OnDestroyed { entity_name }
    let s = state.clone();
    engine.register_fn(
        "on_destroyed",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnDestroyed {
                    entity_name: entity.to_string(),
                },
                &handler,
            );
        },
    );

    // 2. OnAllDestroyed { group, after_secs } — no-gate (after_secs = 0.0) …
    let s = state.clone();
    engine.register_fn(
        "on_all_destroyed",
        move |group: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnAllDestroyed {
                    group: group.to_string(),
                    after_secs: 0.0,
                },
                &handler,
            );
        },
    );
    // … and the gated form (integer seconds → f32 at the boundary).
    let s = state.clone();
    engine.register_fn(
        "on_all_destroyed",
        move |group: ImmutableString, after_secs: i64, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnAllDestroyed {
                    group: group.to_string(),
                    after_secs: after_secs as f32,
                },
                &handler,
            );
        },
    );

    // 3. OnAttacked { entity_name }
    let s = state.clone();
    engine.register_fn(
        "on_attacked",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnAttacked {
                    entity_name: entity.to_string(),
                },
                &handler,
            );
        },
    );

    // 4. OnTimer { after_secs } — integer seconds → f32 at the boundary.
    let s = state.clone();
    engine.register_fn(
        "on_timer",
        move |after_secs: i64, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnTimer {
                    after_secs: after_secs as f32,
                },
                &handler,
            );
        },
    );

    // 5. OnHailed { entity_name }
    let s = state.clone();
    engine.register_fn(
        "on_hailed",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnHailed {
                    entity_name: entity.to_string(),
                },
                &handler,
            );
        },
    );

    // 6. OnFlagSet { name }
    let s = state.clone();
    engine.register_fn(
        "on_flag_set",
        move |name: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnFlagSet {
                    name: name.to_string(),
                },
                &handler,
            );
        },
    );

    // 7. OnFlagCleared { name }
    let s = state.clone();
    engine.register_fn(
        "on_flag_cleared",
        move |name: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnFlagCleared {
                    name: name.to_string(),
                },
                &handler,
            );
        },
    );

    // 8. OnWorldLoaded (no condition params)
    let s = state.clone();
    engine.register_fn("on_world_loaded", move |handler: ImmutableString| {
        push_trigger(&s, TriggerCondition::OnWorldLoaded, &handler);
    });

    // 9. OnEnteredRegion { entity_name }
    let s = state.clone();
    engine.register_fn(
        "on_entered_region",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnEnteredRegion {
                    entity_name: entity.to_string(),
                },
                &handler,
            );
        },
    );

    // 10. OnExitedRegion { entity_name }
    let s = state.clone();
    engine.register_fn(
        "on_exited_region",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnExitedRegion {
                    entity_name: entity.to_string(),
                },
                &handler,
            );
        },
    );

    // 11. OnWaypointReached { entity_name, waypoint } — any-waypoint form …
    let s = state.clone();
    engine.register_fn(
        "on_waypoint_reached",
        move |entity: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnWaypointReached {
                    entity_name: entity.to_string(),
                    waypoint: None,
                },
                &handler,
            );
        },
    );
    // … and the specific-anchor form.
    let s = state.clone();
    engine.register_fn(
        "on_waypoint_reached",
        move |entity: ImmutableString, waypoint: ImmutableString, handler: ImmutableString| {
            push_trigger(
                &s,
                TriggerCondition::OnWaypointReached {
                    entity_name: entity.to_string(),
                    waypoint: Some(waypoint.to_string()),
                },
                &handler,
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::world::config::{parse_world, Trigger, TriggerCondition};
    use crate::world::script::load::compile_scripts;
    use vellum_script::ScriptSource;

    /// Compile a single inline script unit and return the triggers its top level
    /// authored, in registration order.
    fn script_triggers(source: &str) -> Vec<crate::world::script::engine::ScriptTrigger> {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: source.to_string(),
        }]);
        assert!(
            compiled.findings.is_empty(),
            "unexpected findings: {:?}",
            compiled.findings
        );
        compiled.script_triggers
    }

    /// The single `Trigger` the TOML front-end builds for `trigger_toml`, a
    /// `[[trigger]]` block body. Uses `script = "h"` so it takes the scripted
    /// path — the same construction the registration fns take.
    fn toml_trigger(trigger_toml: &str) -> Trigger {
        let world = format!("[[trigger]]\n{trigger_toml}\nscript = \"h\"\n");
        let cfg = parse_world(&world).expect("world parses");
        assert_eq!(cfg.triggers.len(), 1);
        cfg.triggers.into_iter().next().unwrap()
    }

    /// Assert a registration fn and its TOML equivalent build the identical
    /// `Trigger` (the "one evaluator, two front-ends" structural-equality
    /// guarantee), and that the registration recorded the handler name.
    fn assert_parity(script_call: &str, trigger_toml: &str) {
        let regs = script_triggers(&format!("{script_call};\nfn h(ctx) {{ }}"));
        assert_eq!(regs.len(), 1, "exactly one trigger from `{script_call}`");
        assert_eq!(regs[0].handler, "h");
        assert_eq!(
            regs[0].trigger,
            toml_trigger(trigger_toml),
            "script `{script_call}` must build the same Trigger as TOML `{trigger_toml}`"
        );
    }

    // ── one test per TriggerCondition variant (all 11) ────────────────────────

    #[test]
    fn on_destroyed_matches_toml() {
        assert_parity(
            r#"on_destroyed("raider", "h")"#,
            "condition = \"on_destroyed\"\nentity = \"raider\"",
        );
        // Spot-check the condition itself, not just equality-to-TOML.
        let regs = script_triggers(r#"on_destroyed("raider", "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnDestroyed {
                entity_name: "raider".into()
            }
        );
    }

    #[test]
    fn on_all_destroyed_default_gate_matches_toml() {
        assert_parity(
            r#"on_all_destroyed("wave_1", "h")"#,
            "condition = \"on_all_destroyed\"\ngroup = \"wave_1\"",
        );
    }

    #[test]
    fn on_all_destroyed_with_gate_matches_toml() {
        assert_parity(
            r#"on_all_destroyed("wave_1", 5, "h")"#,
            "condition = \"on_all_destroyed\"\ngroup = \"wave_1\"\nafter_secs = 5.0",
        );
        let regs = script_triggers(r#"on_all_destroyed("wave_1", 5, "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnAllDestroyed {
                group: "wave_1".into(),
                after_secs: 5.0
            }
        );
    }

    #[test]
    fn on_attacked_matches_toml() {
        assert_parity(
            r#"on_attacked("escort", "h")"#,
            "condition = \"on_attacked\"\nentity = \"escort\"",
        );
    }

    #[test]
    fn on_timer_matches_toml() {
        assert_parity(
            r#"on_timer(45, "h")"#,
            "condition = \"on_timer\"\nafter_secs = 45.0",
        );
        let regs = script_triggers(r#"on_timer(45, "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnTimer { after_secs: 45.0 }
        );
    }

    #[test]
    fn on_hailed_matches_toml() {
        assert_parity(
            r#"on_hailed("relay", "h")"#,
            "condition = \"on_hailed\"\nentity = \"relay\"",
        );
    }

    #[test]
    fn on_flag_set_matches_toml() {
        assert_parity(
            r#"on_flag_set("armed", "h")"#,
            "condition = \"on_flag_set\"\nname = \"armed\"",
        );
    }

    #[test]
    fn on_flag_cleared_matches_toml() {
        assert_parity(
            r#"on_flag_cleared("armed", "h")"#,
            "condition = \"on_flag_cleared\"\nname = \"armed\"",
        );
    }

    #[test]
    fn on_world_loaded_matches_toml() {
        assert_parity(r#"on_world_loaded("h")"#, "condition = \"on_world_loaded\"");
        let regs = script_triggers(r#"on_world_loaded("h"); fn h(ctx) { }"#);
        assert_eq!(regs[0].trigger.condition, TriggerCondition::OnWorldLoaded);
    }

    #[test]
    fn on_entered_region_matches_toml() {
        assert_parity(
            r#"on_entered_region("nebula", "h")"#,
            "condition = \"on_entered_region\"\nentity = \"nebula\"",
        );
    }

    #[test]
    fn on_exited_region_matches_toml() {
        assert_parity(
            r#"on_exited_region("nebula", "h")"#,
            "condition = \"on_exited_region\"\nentity = \"nebula\"",
        );
    }

    #[test]
    fn on_waypoint_reached_any_matches_toml() {
        assert_parity(
            r#"on_waypoint_reached("courier", "h")"#,
            "condition = \"on_waypoint_reached\"\nentity = \"courier\"",
        );
        let regs = script_triggers(r#"on_waypoint_reached("courier", "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnWaypointReached {
                entity_name: "courier".into(),
                waypoint: None
            }
        );
    }

    #[test]
    fn on_waypoint_reached_specific_matches_toml() {
        assert_parity(
            r#"on_waypoint_reached("courier", "beacon_3", "h")"#,
            "condition = \"on_waypoint_reached\"\nentity = \"courier\"\nwaypoint = \"beacon_3\"",
        );
        let regs =
            script_triggers(r#"on_waypoint_reached("courier", "beacon_3", "h"); fn h(ctx) { }"#);
        assert_eq!(
            regs[0].trigger.condition,
            TriggerCondition::OnWaypointReached {
                entity_name: "courier".into(),
                waypoint: Some("beacon_3".into())
            }
        );
    }

    // ── the front-end also records the handler and defaults the lifecycle ─────

    #[test]
    fn a_scripted_trigger_has_empty_actions_and_default_lifecycle() {
        let regs = script_triggers(r#"on_destroyed("x", "h"); fn h(ctx) { }"#);
        let t = &regs[0].trigger;
        assert!(t.actions.is_empty(), "the handler supplies the effects");
        assert!(t.action_predicates.is_empty());
        assert!(t.action_delays.is_empty());
        assert_eq!(t.when, None);
        assert_eq!(t.id, None);
        assert!(!t.repeat);
        assert_eq!(t.cooldown_secs, None);
    }

    #[test]
    fn multiple_registrations_are_collected_in_order() {
        let regs = script_triggers(
            r#"
            on_world_loaded("a");
            on_destroyed("x", "b");
            on_timer(10, "c");
            fn a(ctx) { }
            fn b(ctx) { }
            fn c(ctx) { }
            "#,
        );
        let handlers: Vec<&str> = regs.iter().map(|r| r.handler.as_str()).collect();
        assert_eq!(handlers, vec!["a", "b", "c"]);
    }

    // ── parity: scripted world == TOML world, same event stream (issue #980) ──

    /// A scripted world (trigger authored in Rhai, effects in a handler fn) and
    /// its declarative TOML equivalent produce the identical `ActionCmd` sequence
    /// for the same event stream — the strongest migration guard. The scripted
    /// path fires the trigger through the existing evaluator, then runs the
    /// handler on the runtime host; the TOML path fires the same trigger and
    /// dispatches its declarative actions. Both routes land on the one
    /// `ActionCmd` boundary, and neither touches `tick_trigger_pipeline`.
    #[test]
    fn scripted_and_toml_worlds_emit_identical_action_cmds() {
        use crate::world::content::{evaluate_triggers, TriggerState, WorldEvent};
        use crate::world::dispatch::{dispatch_action, ActionCmd, DispatchContext};
        use crate::world::flags::FlagStore;
        use crate::world::script::engine::RuntimeHost;
        use rhai::Map;
        use std::collections::{HashMap, HashSet};

        // Shared event stream: the entity "raider" is destroyed.
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".to_string(), "uuid-raider".to_string());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-raider".to_string(),
        }];

        let state_of = |trigger: Trigger| TriggerState {
            trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
            last_fired_elapsed: None,
        };

        // ---- Scripted front-end: trigger + handler authored in Rhai. ----
        let path = "w.toml#script.setup";
        let compiled = compile_scripts(&[ScriptSource {
            path: path.to_string(),
            source: r#"
                on_destroyed("raider", "on_raider_dead");
                fn on_raider_dead(ctx) {
                    ctx.effects.complete_objective("obj-x");
                    ctx.effects.fail_objective("obj-y");
                }
            "#
            .to_string(),
        }]);
        assert!(compiled.findings.is_empty(), "{:?}", compiled.findings);
        assert_eq!(compiled.script_triggers.len(), 1);
        let st = compiled.script_triggers[0].clone();

        let mut states = vec![state_of(st.trigger.clone())];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(
            fired[0].actions.is_empty(),
            "a scripted trigger's effects come from its handler, not an action list"
        );
        let host = RuntimeHost::new();
        let ast = compiled.asts.get(path).expect("compiled ast");
        let scripted_cmds = host.call(ast, path, &st.handler, &FlagStore::new(), Map::new());

        // ---- Declarative TOML front-end: the same trigger, actions inline. ----
        let cfg = parse_world(
            r#"
            [[trigger]]
            condition = "on_destroyed"
            entity = "raider"

            [[trigger.action]]
            type = "complete_objective"
            id = "obj-x"

            [[trigger.action]]
            type = "fail_objective"
            id = "obj-y"
            "#,
        )
        .expect("world parses");
        let mut states = vec![state_of(cfg.triggers[0].clone())];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);

        // A minimal dispatch context — the two objective actions are
        // context-free, so only the required borrows are populated.
        let empty_names = HashMap::new();
        let base_flags = FlagStore::new();
        let layers = HashMap::new();
        let anchors = HashMap::new();
        let uuid = || "uuid".to_string();
        let ctx = DispatchContext {
            origin_layer: None,
            entity_name: None,
            name_to_uuid: &empty_names,
            base_flags: &base_flags,
            layers: &layers,
            base_anchors: &anchors,
            factions: None,
            uuid_source: &uuid,
            template_loader: &crate::entity_loader::WasmTemplateLoader,
        };
        let toml_cmds: Vec<ActionCmd> = fired[0]
            .actions
            .iter()
            .flat_map(|a| dispatch_action(a, &ctx).commands)
            .collect();

        // Identical, and the expected sequence.
        assert_eq!(scripted_cmds, toml_cmds);
        assert_eq!(
            scripted_cmds,
            vec![
                ActionCmd::CompleteObjective {
                    id: "obj-x".to_string()
                },
                ActionCmd::FailObjective {
                    id: "obj-y".to_string()
                },
            ]
        );
    }
}
