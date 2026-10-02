use crate::world::config::{Trigger, TriggerCondition};
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

/// Assert a registration fn builds exactly the expected `Trigger`, and that
/// it recorded the handler name.
///
/// The expectation used to be computed by parsing the equivalent
/// `[[trigger]] script = "h"` block — the "one evaluator, two front-ends"
/// structural-equality guarantee. Issue #985 deleted that second front-end,
/// so the expectation is written out instead, which is the stronger
/// assertion: it pins the condition each registration builds rather than
/// comparing two parsers against each other.
fn assert_builds(script_call: &str, expected: TriggerCondition) {
    assert_builds_with(
        script_call,
        crate::world::config::scripted_trigger(expected),
    );
}

/// [`assert_builds`] for a registration that also sets a lifecycle field
/// (`.when(…)`, `.repeat()`, …), where the whole `Trigger` is the claim.
fn assert_builds_with(script_call: &str, expected: Trigger) {
    let regs = script_triggers(&format!("{script_call};\nfn h(ctx) {{ }}"));
    assert_eq!(regs.len(), 1, "exactly one trigger from `{script_call}`");
    assert_eq!(regs[0].handler, "h");
    assert_eq!(
        regs[0].trigger, expected,
        "script `{script_call}` must build the expected Trigger"
    );
}

// ── one test per TriggerCondition variant (all 11) ────────────────────────

#[test]
fn on_destroyed_builds_its_condition() {
    assert_builds(
        r#"on_destroyed("raider", "h")"#,
        TriggerCondition::OnDestroyed {
            entity_name: "raider".into(),
        },
    );
    // Spot-check the condition through the public accessor too.
    let regs = script_triggers(r#"on_destroyed("raider", "h"); fn h(ctx) { }"#);
    assert_eq!(
        regs[0].trigger.condition,
        TriggerCondition::OnDestroyed {
            entity_name: "raider".into()
        }
    );
}

#[test]
fn on_all_destroyed_default_gate_builds_its_condition() {
    assert_builds(
        r#"on_all_destroyed("wave_1", "h")"#,
        TriggerCondition::OnAllDestroyed {
            group: "wave_1".into(),
            after_secs: 0.0,
        },
    );
}

#[test]
fn on_all_destroyed_with_gate_builds_its_condition() {
    assert_builds(
        r#"on_all_destroyed("wave_1", 5, "h")"#,
        TriggerCondition::OnAllDestroyed {
            group: "wave_1".into(),
            after_secs: 5.0,
        },
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
fn on_attacked_builds_its_condition() {
    assert_builds(
        r#"on_attacked("escort", "h")"#,
        TriggerCondition::OnAttacked {
            entity_name: "escort".into(),
        },
    );
}

#[test]
fn on_timer_builds_its_condition() {
    assert_builds(
        r#"on_timer(45, "h")"#,
        TriggerCondition::OnTimer { after_secs: 45.0 },
    );
    let regs = script_triggers(r#"on_timer(45, "h"); fn h(ctx) { }"#);
    assert_eq!(
        regs[0].trigger.condition,
        TriggerCondition::OnTimer { after_secs: 45.0 }
    );
}

#[test]
fn on_hailed_builds_its_condition() {
    assert_builds(
        r#"on_hailed("relay", "h")"#,
        TriggerCondition::OnHailed {
            entity_name: "relay".into(),
        },
    );
}

#[test]
fn on_flag_set_builds_its_condition() {
    assert_builds(
        r#"on_flag_set("armed", "h")"#,
        TriggerCondition::OnFlagSet {
            name: "armed".into(),
        },
    );
}

#[test]
fn on_flag_cleared_builds_its_condition() {
    assert_builds(
        r#"on_flag_cleared("armed", "h")"#,
        TriggerCondition::OnFlagCleared {
            name: "armed".into(),
        },
    );
}

#[test]
fn on_world_loaded_builds_its_condition() {
    assert_builds(r#"on_world_loaded("h")"#, TriggerCondition::OnWorldLoaded);
    let regs = script_triggers(r#"on_world_loaded("h"); fn h(ctx) { }"#);
    assert_eq!(regs[0].trigger.condition, TriggerCondition::OnWorldLoaded);
}

#[test]
fn on_entered_region_builds_its_condition() {
    assert_builds(
        r#"on_entered_region("nebula", "h")"#,
        TriggerCondition::OnEnteredRegion {
            entity_name: "nebula".into(),
        },
    );
}

#[test]
fn on_exited_region_builds_its_condition() {
    assert_builds(
        r#"on_exited_region("nebula", "h")"#,
        TriggerCondition::OnExitedRegion {
            entity_name: "nebula".into(),
        },
    );
}

#[test]
fn on_waypoint_reached_any_builds_its_condition() {
    assert_builds(
        r#"on_waypoint_reached("courier", "h")"#,
        TriggerCondition::OnWaypointReached {
            entity_name: "courier".into(),
            waypoint: None,
        },
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
fn on_waypoint_reached_specific_builds_its_condition() {
    assert_builds(
        r#"on_waypoint_reached("courier", "beacon_3", "h")"#,
        TriggerCondition::OnWaypointReached {
            entity_name: "courier".into(),
            waypoint: Some("beacon_3".into()),
        },
    );
    let regs = script_triggers(r#"on_waypoint_reached("courier", "beacon_3", "h"); fn h(ctx) { }"#);
    assert_eq!(
        regs[0].trigger.condition,
        TriggerCondition::OnWaypointReached {
            entity_name: "courier".into(),
            waypoint: Some("beacon_3".into())
        }
    );
}

#[test]
fn on_hull_below_builds_its_condition() {
    // The one condition with a FRACTIONAL field, so the one registration
    // taking a `flt(…)` marker rather than an INT (issue #984).
    assert_builds(
        r#"on_hull_below("station", flt("0.75"), "h")"#,
        TriggerCondition::OnHullBelow {
            entity_name: "station".into(),
            threshold: 0.75,
        },
    );
    let regs = script_triggers(r#"on_hull_below("station", flt("0.5"), "h"); fn h(ctx) { }"#);
    assert_eq!(
        regs[0].trigger.condition,
        TriggerCondition::OnHullBelow {
            entity_name: "station".into(),
            threshold: 0.5
        }
    );
}

#[test]
fn on_hull_below_rejects_a_threshold_outside_the_authored_range() {
    // Mirrors the declarative front-end's `(0, 1]` check, so the two
    // front-ends refuse the same content.
    for bad in ["0.0", "1.5", "-0.25"] {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: format!("on_hull_below(\"s\", flt(\"{bad}\"), \"h\"); fn h(ctx) {{ }}"),
        }]);
        assert!(
            !compiled.findings.is_empty(),
            "threshold {bad} must be refused at load"
        );
    }
}

// ── the trigger-level `when` modifier (issue #984) ────────────────────────

#[test]
fn when_builds_the_same_predicate_the_declarative_field_does() {
    let mut expected = crate::world::config::scripted_trigger(TriggerCondition::OnAllDestroyed {
        group: "hostiles".into(),
        after_secs: 0.0,
    });
    expected.when = Some(
        crate::world::flags::parse_predicate("counter(waves_spawned) >= 8")
            .expect("the predicate parses"),
    );
    assert_builds_with(
        r#"on_all_destroyed("hostiles", "h").when("counter(waves_spawned) >= 8")"#,
        expected,
    );
}

#[test]
fn when_applies_to_the_registration_it_is_chained_onto() {
    // Two registrations, one guarded: the modifier must land on ITS OWN
    // trigger, which is what the returned handle is for.
    let regs = script_triggers(
        r#"
            on_world_loaded("a");
            on_destroyed("x", "b").when("flag(armed)");
            on_timer(5, "c");
            fn a(ctx) { }
            fn b(ctx) { }
            fn c(ctx) { }
            "#,
    );
    let guarded: Vec<&str> = regs
        .iter()
        .filter(|r| r.trigger.when.is_some())
        .map(|r| r.handler.as_str())
        .collect();
    assert_eq!(guarded, vec!["b"]);
}

#[test]
fn when_rejects_a_malformed_predicate_and_a_world_history_atom() {
    for bad in [
        // Not a predicate at all.
        "counter(",
        // A bounded-history window, which a WORLD expression cannot fold —
        // refused by the same `reject_world_history` the declarative `when =`
        // field runs (issue #890).
        "history(hull_below, 5) >= 1",
    ] {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: format!("on_world_loaded(\"h\").when(\"{bad}\"); fn h(ctx) {{ }}"),
        }]);
        assert!(
            !compiled.findings.is_empty(),
            "predicate `{bad}` must be refused at load"
        );
    }
}

/// `.repeat()` marks exactly the registration it is chained onto, and it
/// composes with `.when(…)` in either order — the two are orthogonal
/// modifiers of one `Trigger`, not a sequence.
#[test]
fn repeat_marks_the_registration_it_is_chained_onto_and_composes_with_when() {
    let regs = script_triggers(
        r#"
            on_hailed("x", "a");
            on_hailed("x", "b").repeat();
            on_hailed("x", "c").repeat().when("flag(armed)");
            on_hailed("x", "d").when("flag(armed)").repeat();
            fn a(ctx) { }
            fn b(ctx) { }
            fn c(ctx) { }
            fn d(ctx) { }
            "#,
    );
    let repeating: Vec<&str> = regs
        .iter()
        .filter(|r| r.trigger.repeat)
        .map(|r| r.handler.as_str())
        .collect();
    assert_eq!(repeating, vec!["b", "c", "d"]);
    let guarded: Vec<&str> = regs
        .iter()
        .filter(|r| r.trigger.when.is_some())
        .map(|r| r.handler.as_str())
        .collect();
    assert_eq!(
        guarded,
        vec!["c", "d"],
        "both modifiers hand the handle straight back, so the two compose in \
             EITHER order and both land on the same registration — the guide, the \
             spec and this front-end's own summaries all promise that, and `d` is \
             the order the promise used to break in"
    );
    assert_eq!(
        regs.iter()
            .find(|r| r.handler == "c")
            .expect("the third registration")
            .trigger
            .cooldown_secs,
        None,
        "no cooldown twin is exposed on this front-end: an event-driven condition \
             fires once per occurrence and has nothing for a minimum spacing to \
             suppress"
    );
}

// ── the front-end also records the handler and defaults the lifecycle ─────

#[test]
fn a_scripted_trigger_defaults_every_lifecycle_field() {
    let regs = script_triggers(r#"on_destroyed("x", "h"); fn h(ctx) { }"#);
    let t = &regs[0].trigger;
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

// ── a scripted trigger's effects come from its handler (issue #980) ──────

/// A scripted world fires its trigger through the shared evaluator and then
/// runs the handler on the runtime host, landing on the one `ActionCmd`
/// boundary. Neither step touches `tick_trigger_pipeline`.
///
/// This was the strongest migration guard while there were TWO front-ends: it
/// built the same trigger declaratively, dispatched its `[[trigger.action]]`
/// array, and asserted the two `ActionCmd` sequences were identical. Issue
/// #985 deleted the declarative half — and with it `FiredTrigger::actions`,
/// which is why the "a fired trigger carries no action list" assertion below
/// is now a statement about the type rather than about a scripted trigger in
/// particular. What survives is the concrete expected sequence, which is what
/// pinned behaviour rather than equality-to-itself.
#[test]
fn a_scripted_trigger_fires_and_its_handler_emits_the_action_cmds() {
    use crate::world::content::{evaluate_triggers, TriggerState, WorldEvent};
    use crate::world::dispatch::ActionCmd;
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

    let mut states = vec![TriggerState {
        trigger: st.trigger.clone(),
        fired: false,
        origin_layer: None,
        seen_destroyed: HashSet::new(),
        last_fired_elapsed: None,
    }];
    let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
    assert_eq!(fired.len(), 1);

    let host = RuntimeHost::new();
    let ast = compiled.asts.get(path).expect("compiled ast");
    let cmds = host.call_immediate(ast, path, &st.handler, &FlagStore::new(), Map::new());

    assert_eq!(
        cmds,
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

// ── gm_event: the manual-only GM shorthand (issue #1301) ─────────────────

#[test]
fn gm_event_builds_a_manual_trigger_with_an_implied_fire_control() {
    let mut expected = crate::world::config::scripted_trigger(TriggerCondition::Manual);
    expected.id = Some("breach_alarm".into());
    expected.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
        "breach_alarm".into(),
        "world.gm.event.breach_alarm".into(),
    ));
    assert_builds_with(
        r#"gm_event("breach_alarm", "world.gm.event.breach_alarm", "h")"#,
        expected,
    );
}

#[test]
fn a_gm_event_is_one_shot_until_repeatable_says_otherwise() {
    let once = script_triggers(r#"gm_event("a", "world.gm.event.a", "h"); fn h(ctx) { }"#);
    assert!(!once[0].trigger.repeat, "gm_event is one-shot by default");

    let reusable =
        script_triggers(r#"gm_event("a", "world.gm.event.a", "h").repeatable(); fn h(ctx) { }"#);
    assert!(
        reusable[0].trigger.repeat,
        "repeatable() makes it a reusable quick tool"
    );
    // `repeatable()` sets the SAME lifecycle field `.repeat()` does, and the
    // two compose with `.when(…)` in either order.
    let chained = script_triggers(
        r#"gm_event("a", "world.gm.event.a", "h").repeatable().when("flag(ready)");
               fn h(ctx) { }"#,
    );
    assert!(chained[0].trigger.repeat);
    assert!(chained[0].trigger.when.is_some());
    let reversed = script_triggers(
        r#"gm_event("a", "world.gm.event.a", "h").when("flag(ready)").repeatable();
               fn h(ctx) { }"#,
    );
    assert_eq!(chained[0].trigger, reversed[0].trigger);
}

/// A malformed id or label is a LOAD-TIME finding, not an event that
/// silently never appears in the mission panel.
#[test]
fn a_malformed_gm_event_identity_is_a_blocking_finding() {
    for source in [
        r#"gm_event("", "world.gm.event.a", "h"); fn h(ctx) { }"#,
        r#"gm_event("a::b", "world.gm.event.a", "h"); fn h(ctx) { }"#,
        r#"gm_event("a b", "world.gm.event.a", "h"); fn h(ctx) { }"#,
        r#"gm_event("a", "", "h"); fn h(ctx) { }"#,
    ] {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: source.to_string(),
        }]);
        assert!(
            compiled
                .findings
                .iter()
                .any(|finding| finding.severity == crate::world::validate::Severity::Error),
            "`{source}` must be refused at load: {:?}",
            compiled.findings
        );
        assert!(
            compiled.script_triggers.is_empty(),
            "`{source}` must not register a half-built event"
        );
    }
}

// ── gm_controls: GM operability on an ORDINARY event (issue #1302) ────────

/// The identity rule is ONE rule: the same four malformed shapes
/// `gm_event` refuses are refused on this surface too, and the trigger the
/// registration already pushed is left carrying NO control set — the
/// half-built event the atomic activation gate then blocks the world over.
#[test]
fn a_malformed_gm_controls_identity_is_a_blocking_finding() {
    for source in [
        r#"on_world_loaded("h").gm_controls("", "world.gm.event.a"); fn h(ctx) { }"#,
        r#"on_world_loaded("h").gm_controls("a::b", "world.gm.event.a"); fn h(ctx) { }"#,
        r#"on_world_loaded("h").gm_controls("a b", "world.gm.event.a"); fn h(ctx) { }"#,
        r#"on_world_loaded("h").gm_controls("a", ""); fn h(ctx) { }"#,
    ] {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: source.to_string(),
        }]);
        assert!(
            compiled
                .findings
                .iter()
                .any(|finding| finding.severity == crate::world::validate::Severity::Error),
            "`{source}` must be refused at load: {:?}",
            compiled.findings
        );
        assert!(
            compiled
                .script_triggers
                .iter()
                .all(|st| st.trigger.gm_controls.is_none()),
            "`{source}` must not register a half-built control set"
        );
    }
}

/// The declaration keeps the automatic condition and adds exactly the Fire
/// lever — the same control set `gm_event` implies, under an authored id.
#[test]
fn gm_controls_keeps_the_condition_and_adds_an_authored_fire_control() {
    let mut expected = crate::world::config::scripted_trigger(TriggerCondition::OnHullBelow {
        entity_name: "courier".into(),
        threshold: 0.4,
    });
    expected.id = Some("breach_alarm".into());
    expected.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
        "breach_alarm".into(),
        "world.gm.event.breach_alarm".into(),
    ));
    assert_builds_with(
        r#"on_hull_below("courier", flt("0.4"), "h")
                   .gm_controls("breach_alarm", "world.gm.event.breach_alarm")"#,
        expected,
    );

    let controls = script_triggers(
        r#"on_hull_below("courier", flt("0.4"), "h")
                   .gm_controls("breach_alarm", "world.gm.event.breach_alarm");
               fn h(ctx) { }"#,
    )[0]
    .trigger
    .gm_controls
    .clone()
    .expect("a control set");
    assert!(controls.fire, "Fire is what this declaration turns on");
    assert!(
        !controls.pause && !controls.skip,
        "Pause (#1303) and Skip (#1304) are not declared here"
    );
}

/// It is a trigger-level modifier like `.when(…)` and `.repeat()`, so the
/// three compose in any order and say the same sentence.
#[test]
fn gm_controls_composes_with_the_other_trigger_modifiers_in_any_order() {
    let one = script_triggers(
        r#"on_flag_set("alarm", "h")
                   .gm_controls("evac", "world.gm.event.evac").repeat().when("flag(ready)");
               fn h(ctx) { }"#,
    );
    let other = script_triggers(
        r#"on_flag_set("alarm", "h")
                   .when("flag(ready)").repeat().gm_controls("evac", "world.gm.event.evac");
               fn h(ctx) { }"#,
    );
    assert_eq!(one[0].trigger, other[0].trigger);
    assert!(one[0].trigger.repeat);
    assert!(one[0].trigger.when.is_some());
    assert_eq!(
        one[0].trigger.condition,
        TriggerCondition::OnFlagSet {
            name: "alarm".into()
        },
        "the automatic condition is untouched by the declaration"
    );
}

/// Issue #1303: `.pauseable()` adds the Pause lever to a control set either
/// surface declared, composes with every other modifier in any order, and
/// changes nothing else about the trigger.
#[test]
fn pauseable_adds_the_pause_lever_to_either_authoring_surface() {
    for source in [
        r#"on_destroyed("courier", "h")
                   .gm_controls("evac", "world.gm.event.evac").pauseable();
               fn h(ctx) { }"#,
        r#"gm_event("evac", "world.gm.event.evac", "h").pauseable();
               fn h(ctx) { }"#,
    ] {
        let controls = script_triggers(source)[0]
            .trigger
            .gm_controls
            .clone()
            .expect("a control set");
        assert!(controls.pause, "`{source}` declares the Pause lever");
        assert!(controls.fire, "and leaves the Fire lever alone");
        assert!(!controls.skip, "Skip (#1304) is still not declared");
        assert_eq!(controls.id, "evac");
    }

    // Order-independent, like every other trigger-level modifier, and a
    // second `.pauseable()` is idempotent rather than a load-time error:
    // unlike a second control set it cannot re-identify the event.
    let one = script_triggers(
        r#"on_flag_set("alarm", "h")
                   .gm_controls("evac", "world.gm.event.evac").pauseable().repeat()
                   .when("flag(ready)");
               fn h(ctx) { }"#,
    );
    let other = script_triggers(
        r#"on_flag_set("alarm", "h")
                   .when("flag(ready)").repeat()
                   .gm_controls("evac", "world.gm.event.evac").pauseable().pauseable();
               fn h(ctx) { }"#,
    );
    assert_eq!(one[0].trigger, other[0].trigger);
    assert!(one[0].trigger.repeat && one[0].trigger.when.is_some());
    assert_eq!(
        one[0].trigger.condition,
        TriggerCondition::OnFlagSet {
            name: "alarm".into()
        },
        "the automatic condition is untouched by the declaration"
    );
}

/// `.pauseable()` on a trigger that declares no control set is a load-time
/// error: there is no id to address it by, no label to render and no
/// mission-panel row to hang a toggle on, so it can only be a mistake.
#[test]
fn pauseable_without_a_control_set_is_a_blocking_finding() {
    let compiled = compile_scripts(&[ScriptSource {
        path: "w.toml#script.setup".to_string(),
        source: r#"on_world_loaded("h").pauseable(); fn h(ctx) { }"#.to_string(),
    }]);
    assert!(
        compiled
            .findings
            .iter()
            .any(|finding| finding.severity == crate::world::validate::Severity::Error),
        "a pauseable() with no gm_controls must be refused at load: {:?}",
        compiled.findings
    );
    assert!(compiled
        .script_triggers
        .iter()
        .all(|st| st.trigger.gm_controls.is_none()));
}

/// One trigger, one control set. Declaring twice is a load-time error
/// rather than a last-writer-wins overwrite that would publish a panel row
/// under an id the author did not expect.
#[test]
fn a_second_gm_controls_declaration_on_one_trigger_is_a_blocking_finding() {
    for source in [
        r#"on_world_loaded("h").gm_controls("a", "world.gm.event.a")
                   .gm_controls("b", "world.gm.event.b"); fn h(ctx) { }"#,
        // `gm_event` already implied Fire under its OWN authored id.
        r#"gm_event("a", "world.gm.event.a", "h")
                   .gm_controls("b", "world.gm.event.b"); fn h(ctx) { }"#,
    ] {
        let compiled = compile_scripts(&[ScriptSource {
            path: "w.toml#script.setup".to_string(),
            source: source.to_string(),
        }]);
        assert!(
            compiled
                .findings
                .iter()
                .any(|finding| finding.severity == crate::world::validate::Severity::Error),
            "`{source}` must be refused at load: {:?}",
            compiled.findings
        );
    }

    // The FIRST declaration survives untouched — the refusal is not a
    // half-applied overwrite.
    let compiled = compile_scripts(&[ScriptSource {
        path: "w.toml#script.setup".to_string(),
        source: r#"on_world_loaded("h").gm_controls("a", "world.gm.event.a")
                           .gm_controls("b", "world.gm.event.b"); fn h(ctx) { }"#
            .to_string(),
    }]);
    assert_eq!(
        compiled.script_triggers[0]
            .trigger
            .gm_controls
            .as_ref()
            .map(|c| c.id.as_str()),
        Some("a"),
    );
}

// -- skip(): the Skip-next lever (issue #1304) ----------------------------

/// `.skip()` turns on exactly one field of the control set the line before
/// it declared, and leaves the condition, the lifecycle and Fire alone.
#[test]
fn skip_adds_one_lever_to_the_declaration_before_it() {
    let controls = script_triggers(
        r#"on_destroyed("courier", "h")
                   .gm_controls("evac", "world.gm.event.evac").skip();
               fn h(ctx) { }"#,
    )[0]
    .trigger
    .gm_controls
    .clone()
    .expect("a control set");
    assert!(controls.skip, "Skip is what this modifier turns on");
    assert!(controls.fire, "and it leaves the declared Fire alone");
    assert!(!controls.pause, "Pause (#1303) is not declared here");

    let mut expected = crate::world::config::scripted_trigger(TriggerCondition::OnDestroyed {
        entity_name: "courier".into(),
    });
    expected.id = Some("evac".into());
    let mut with_skip = crate::world::config::GmEventControls::fire_only(
        "evac".into(),
        "world.gm.event.evac".into(),
    );
    with_skip.skip = true;
    expected.gm_controls = Some(with_skip);
    assert_builds_with(
        r#"on_destroyed("courier", "h")
                   .gm_controls("evac", "world.gm.event.evac").skip()"#,
        expected,
    );
}

/// It composes with the other trigger modifiers in any order, exactly as
/// they compose with each other -- with the ONE ordering constraint the
/// lever's meaning imposes: it must follow the declaration it belongs to.
#[test]
fn skip_composes_with_the_other_modifiers_and_needs_its_declaration_first() {
    let one = script_triggers(
        r#"on_flag_set("alarm", "h")
                   .gm_controls("evac", "world.gm.event.evac").skip().repeat();
               fn h(ctx) { }"#,
    );
    let other = script_triggers(
        r#"on_flag_set("alarm", "h")
                   .repeat().gm_controls("evac", "world.gm.event.evac").skip();
               fn h(ctx) { }"#,
    );
    assert_eq!(one[0].trigger, other[0].trigger);
    assert!(one[0].trigger.repeat);
    assert!(one[0]
        .trigger
        .gm_controls
        .as_ref()
        .is_some_and(|controls| controls.skip));

    // A lever with no declaration to belong to names no event at all, so it
    // is a load-time error rather than a trigger with a silent Skip.
    let compiled = compile_scripts(&[ScriptSource {
        path: "w.toml#script.setup".to_string(),
        source: r#"on_flag_set("alarm", "h").skip(); fn h(ctx) { }"#.to_string(),
    }]);
    assert!(
        compiled
            .findings
            .iter()
            .any(|finding| finding.severity == crate::world::validate::Severity::Error),
        "a Skip with no gm_controls must be refused at load: {:?}",
        compiled.findings
    );
}

/// A manual `gm_event` has no automatic occurrence, so a Skip on it could
/// never be consumed: a mission-panel button an operator can press for ever
/// with no possible effect. Refused at load rather than shipped inert.
#[test]
fn skip_on_a_manual_gm_event_is_a_blocking_finding() {
    let compiled = compile_scripts(&[ScriptSource {
        path: "w.toml#script.setup".to_string(),
        source: r#"gm_event("a", "world.gm.event.a", "h").skip(); fn h(ctx) { }"#.to_string(),
    }]);
    assert!(
        compiled
            .findings
            .iter()
            .any(|finding| finding.severity == crate::world::validate::Severity::Error),
        "a Skip on a manual event must be refused at load: {:?}",
        compiled.findings
    );
    assert!(
        compiled.script_triggers.iter().all(|st| st
            .trigger
            .gm_controls
            .as_ref()
            .is_none_or(|controls| !controls.skip)),
        "and must not register a half-built lever"
    );
}
