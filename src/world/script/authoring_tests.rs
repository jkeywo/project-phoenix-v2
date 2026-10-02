use super::*;
use std::collections::BTreeSet;

/// The `(receiver, name, params)` the editor autocomplete is meant to
/// expose — the curated subset of the registered vocabulary, with overloads
/// collapsed to one descriptor. A verb that is registered but deliberately
/// NOT offered (the `flt(…)` marker, the name-resolving `spawn_entity` /
/// `add_objective` family, the read/write `deadlines` / `commitments`
/// handles, `dossier.holds`, the delayed `destroy_entity` twin, …) is absent
/// here, exactly as it was absent from the former hand-maintained mirror.
///
/// This is the repointed drift guard (issue #1238). The old test pinned a
/// HAND-MAINTAINED `HOST_FNS` slice against the registration sites and then
/// re-checked every entry for a "phantom" (a descriptor the engine never
/// registered). Both concerns are now structural: `host_fns()` is DERIVED by
/// running the same registration the engines run (see
/// [`collect_host_fn_descriptors`]) and taking the descriptors the `host_fn!`
/// sites emitted, so a phantom is impossible and an exposed verb cannot be
/// registered without its descriptor. What still deserves a test is that the
/// exposed SET is exactly this intended list, with the right arities:
/// exposing a currently-hidden verb, or dropping an exposed one, must be a
/// deliberate edit here rather than an accident. The tuples below are the
/// former mirror's `(receiver, name, params)`, so this doubles as the
/// "same set unchanged" proof the derivation had to preserve.
const EXPECTED_EXPOSED: &[(&str, &str, &[&str])] = &[
    // Loading engine: `on` + one per TriggerCondition variant, then the two
    // trigger-level modifiers.
    ("", "on", &["event", "handler"]),
    ("", "on_destroyed", &["entity", "handler"]),
    ("", "on_all_destroyed", &["group", "handler"]),
    ("", "on_attacked", &["entity", "handler"]),
    ("", "on_timer", &["after_secs", "handler"]),
    ("", "on_hailed", &["entity", "handler"]),
    ("", "on_flag_set", &["name", "handler"]),
    ("", "on_flag_cleared", &["name", "handler"]),
    ("", "on_world_loaded", &["handler"]),
    ("", "on_entered_region", &["entity", "handler"]),
    ("", "on_exited_region", &["entity", "handler"]),
    ("", "on_waypoint_reached", &["entity", "handler"]),
    ("", "on_hull_below", &["entity", "threshold", "handler"]),
    // The manual-only GM authoring shorthand and its lifecycle modifier
    // (issue #1301). Deliberately exposed: deciding which moments a Game
    // Master can reach for is the author's judgement, and the mission panel
    // can only list what a scenario declares.
    ("", "gm_event", &["id", "label", "handler"]),
    // The GM operability declaration on an ORDINARY event (issue #1302).
    // Exposed for the same reason: which automatic moments a Game Master
    // may reach for is the author's judgement, and the mission panel can
    // only list what a scenario declares.
    ("trigger", "gm_controls", &["id", "label"]),
    // The Pause lever on a declared control set (issue #1303). Sibling to
    // `gm_controls`, and exposed for its reason: whether a Game Master may
    // suspend an automatic moment is the author's judgement.
    ("trigger", "pauseable", &[]),
    // The Skip-next lever on that declaration (issue #1304). Exposed for
    // the same reason again: which occurrences a Game Master may quietly
    // spend is the author's judgement, and the mission panel can only offer
    // what a scenario declares.
    ("trigger", "skip", &[]),
    // The GM-attention band of that declaration (issue #1434). Exposed for
    // the same reason once more: how loudly an eligible beat asks for a
    // Game Master's attention is the author's judgement, and the attention
    // queue can only band what a scenario declares.
    ("trigger", "attention_band", &["band"]),
    ("trigger", "when", &["predicate"]),
    ("trigger", "repeat", &[]),
    ("trigger", "repeatable", &[]),
    // ctx.flags.*
    ("flags", "increment", &["name", "by"]),
    // ctx.effects.*
    ("effects", "complete_objective", &["id", "instance_id"]),
    ("effects", "fail_objective", &["id", "instance_id"]),
    (
        "effects",
        "set_objective_progress",
        &["id", "instance_id", "progress"],
    ),
    // The mission-timeline vocabulary (issue #1338). Deliberately exposed:
    // marking a beat and judging a marked entity's outcome are things only
    // a scenario author can do, so they belong in the autocomplete set.
    ("effects", "narrative_beat", &["id"]),
    ("effects", "narrative_outcome", &["entity", "outcome"]),
    // The post-mission report (issue #1344). Exposed for the same reason:
    // deciding what a mission was ABOUT, and what each of those things
    // ended in, is the author's judgement and nobody else's.
    ("effects", "report_row", &["spec"]),
    ("effects", "reset_trigger", &["id"]),
    ("effects", "set_npc_doctrine", &["entity", "id"]),
    (
        "effects",
        "set_ghost_contact",
        &["observer", "id", "palette", "position_mm"],
    ),
    ("effects", "remove_ghost_contact", &["observer", "id"]),
    (
        "effects",
        "set_contact_report",
        &[
            "observer",
            "target",
            "delay_ticks",
            "position_step_mm",
            "hide_identity",
        ],
    ),
    ("effects", "clear_contact_report", &["observer", "target"]),
    ("effects", "load_world", &["path"]),
    ("effects", "unload_world", &["path"]),
    ("effects", "game_over", &["reason"]),
    ("effects", "repair_infrastructure", &["entity", "points"]),
    ("effects", "damage_infrastructure", &["entity", "points"]),
    ("effects", "order_hold", &["entity"]),
    ("effects", "order_divert_route", &["entity", "route"]),
    ("effects", "order_divert_anchor", &["entity", "anchor"]),
    ("effects", "order_dock", &["entity", "structure"]),
    ("effects", "open_comms", &["spec"]),
    // Timed per-ship Viewscreen staging (issue #1468), exposed through
    // the same action owner as the GM presentation controls.
    ("effects", "force_view", &["ship", "mode", "duration_ticks"]),
    (
        "effects",
        "title_card",
        &["ship", "title", "subtitle", "duration_ticks"],
    ),
    (
        "effects",
        "incoming_comms",
        &["ship", "message", "duration_ticks"],
    ),
    ("effects", "clear_presentation", &["ship", "part"]),
    ("effects", "sound", &["ship", "id", "source"]),
    // The Viewscreen computer-message vocabulary (issue #1342): a scenario
    // author schedules a timed, severity-graded message the same way they
    // author any other effect.
    (
        "effects",
        "show_message",
        &["id", "text", "severity", "duration_secs"],
    ),
    // ctx.schedule.* and the in_seconds(n).<verb> delay builder.
    ("schedule", "in_seconds", &["secs"]),
    ("schedule", "after", &["secs", "callback"]),
    ("delay", "complete_objective", &["id", "instance_id"]),
    ("delay", "fail_objective", &["id", "instance_id"]),
    ("delay", "reset_trigger", &["id"]),
    ("delay", "load_world", &["path"]),
    ("delay", "unload_world", &["path"]),
    ("delay", "game_over", &["reason"]),
    // ctx.dossier.*
    ("dossier", "append", &["spec"]),
];

#[test]
fn signature_formats_top_level_and_receiver_calls() {
    let fns = host_fns();
    let on_destroyed = fns.iter().find(|h| h.name == "on_destroyed").unwrap();
    assert_eq!(on_destroyed.signature(), "on_destroyed(entity, handler)");
    let complete = fns
        .iter()
        .find(|h| h.name == "complete_objective" && h.receiver == "effects")
        .unwrap();
    assert_eq!(
        complete.signature(),
        "effects.complete_objective(id, instance_id)"
    );
}

#[test]
fn derived_registry_exposes_exactly_the_intended_vocabulary() {
    // The DERIVED list (from the `host_fn!` sites, harvested by
    // `collect_host_fn_descriptors`) must expose exactly the curated set,
    // with the same arities — no more (a hidden verb accidentally described),
    // no less (an exposed verb whose `host_fn!` was dropped), and no changed
    // parameter list.
    let derived: BTreeSet<(&str, &str, Vec<&str>)> = host_fns()
        .iter()
        .map(|h| (h.receiver, h.name, h.params.to_vec()))
        .collect();
    let expected: BTreeSet<(&str, &str, Vec<&str>)> = EXPECTED_EXPOSED
        .iter()
        .map(|(receiver, name, params)| (*receiver, *name, params.to_vec()))
        .collect();
    assert_eq!(
        derived, expected,
        "the derived autocomplete vocabulary drifted from the intended set \
             (receiver, name, params)"
    );
    // Overloads collapse to one descriptor (game_over, on_all_destroyed,
    // repair_infrastructure, …), so the derived length is the pair-set length
    // — a second descriptor for the same (receiver, name) would trip this.
    assert_eq!(
        host_fns().len(),
        EXPECTED_EXPOSED.len(),
        "an overloaded verb emitted more than one descriptor"
    );
}

#[test]
fn every_derived_descriptor_carries_a_summary_and_named_params() {
    // A shape check on the derived descriptors, so the wasm bridge never
    // ships a blank completion: every entry has a summary, a name, and
    // non-blank parameter names (`on_world_loaded` is the one no-arg entry).
    for hf in host_fns() {
        assert!(!hf.name.is_empty());
        assert!(
            !hf.summary.is_empty(),
            "{}.{} has no summary",
            hf.receiver,
            hf.name
        );
        // A wrapped summary must use the `\` string continuation, which
        // eats the newline AND the following indentation. Writing the
        // wrap without it bakes the source indentation into the text the
        // scenario editor shows an author, so forbid the run of spaces
        // that can only come from that mistake.
        assert!(
            !hf.summary.contains("  "),
            "{}.{} summary has baked-in wrap indentation: {:?}",
            hf.receiver,
            hf.name,
            hf.summary
        );
        for param in hf.params {
            assert!(
                !param.is_empty(),
                "{}.{} has a blank parameter name",
                hf.receiver,
                hf.name
            );
        }
    }
}

#[test]
fn clean_script_has_no_diagnostics() {
    let src = r#"
            on_destroyed("raider", "on_raider_dead");
            fn on_raider_dead(ctx) {
                ctx.effects.complete_objective("obj-x");
            }
        "#;
    assert!(script_diagnostics(src, 0).is_empty());
}

#[test]
fn a_syntax_error_lands_on_its_line() {
    // The `let x = ;` is on line 3 (1-based) of the buffer.
    let src = "fn a(ctx) {\n    let ok = 1;\n    let x = ;\n}\n";
    let diags = script_diagnostics(src, 0);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].line, 3, "diagnostic must land on the error line");
    assert_eq!(diags[0].severity, "error");
    assert!(!diags[0].message.is_empty());
}

#[test]
fn line_offset_shifts_an_inline_block_to_the_document_line() {
    // The identical error, but the block starts at document line 12 (11
    // lines precede its content), so the diagnostic must read line 14.
    let src = "fn a(ctx) {\n    let ok = 1;\n    let x = ;\n}\n";
    let diags = script_diagnostics(src, 11);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].line, 3 + 11);
}

#[test]
fn a_top_level_call_to_an_undefined_fn_is_a_runtime_diagnostic() {
    // `run_ast` executes the top level; calling a function that neither the
    // engine nor the unit defines is a runtime error the editor should show.
    let src = "no_such_builder(\"x\", \"h\");\n";
    let diags = script_diagnostics(src, 0);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].line, 1);
    // No redundant position suffix left in the message.
    assert!(
        !diags[0].message.ends_with(')') || !diags[0].message.contains("(line "),
        "position suffix should be stripped: {:?}",
        diags[0].message
    );
}

#[test]
fn a_registered_trigger_builder_call_resolves_clean() {
    // `on_timer` is a registered loading-engine host fn, so a top-level call
    // to it (with a defined handler) runs without a diagnostic — proving the
    // diagnostics pass uses the same vocabulary the loader does.
    let src = "on_timer(30, \"tick\");\nfn tick(ctx) { }\n";
    assert!(script_diagnostics(src, 0).is_empty());
}
