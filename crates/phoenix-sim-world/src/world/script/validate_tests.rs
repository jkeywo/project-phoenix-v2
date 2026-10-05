use super::*;

fn reg(event: &str, handler: &str, path: &str) -> Registration {
    Registration {
        event: event.to_string(),
        handler: handler.to_string(),
        source_path: path.to_string(),
    }
}

fn defined(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_resolved_handler_produces_no_finding() {
    let regs = vec![reg("flag_set:armed", "handle_armed", "w.toml#script.s")];
    let findings = validate_registrations(&regs, &defined(&["handle_armed"]));
    assert!(findings.is_empty());
}

#[test]
fn an_unresolved_handler_is_an_error_finding() {
    let regs = vec![reg("flag_set:armed", "handle_armed", "w.toml#script.s")];
    let findings = validate_registrations(&regs, &defined(&["something_else"]));
    assert_eq!(findings.len(), 1);
    let f = &findings[0];
    assert!(f.is_error());
    assert_eq!(f.category, UNRESOLVED_SCRIPT_FN);
    assert_eq!(f.source.file, "w.toml#script.s");
    assert_eq!(f.source.reference, "handle_armed");
    assert!(crate::world::validate::has_error(&findings));
}

#[test]
fn each_unresolved_handler_reports_separately() {
    let regs = vec![
        reg("a", "handler_a", "p1"),
        reg("b", "handler_b", "p2"),
        reg("c", "handler_c", "p3"),
    ];
    // Only `handler_b` is defined.
    let findings = validate_registrations(&regs, &defined(&["handler_b"]));
    assert_eq!(findings.len(), 2);
}

// ── Rhai trigger front-end handler resolution (issue #980) ────────────────

fn script_trigger(handler: &str, path: &str) -> ScriptTrigger {
    ScriptTrigger {
        trigger: crate::world::config::scripted_trigger(
            crate::world::config::TriggerCondition::OnWorldLoaded,
        ),
        handler: handler.to_string(),
        source_path: path.to_string(),
    }
}

#[test]
fn a_resolved_scripted_trigger_handler_is_clean() {
    let sts = vec![script_trigger("on_loaded", "w.toml#script.s")];
    assert!(validate_script_triggers(&sts, &defined(&["on_loaded"])).is_empty());
}

#[test]
fn an_unresolved_scripted_trigger_handler_blocks_activation() {
    let sts = vec![script_trigger("missing", "w.toml#script.s")];
    let findings = validate_script_triggers(&sts, &defined(&["something_else"]));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
    assert_eq!(findings[0].source.reference, "missing");
    assert!(crate::world::validate::has_error(&findings));
}

// ── GM-operable event identity (issue #1301) ─────────────────────────────

fn gm_event_trigger(id: &str, label: &str, path: &str) -> ScriptTrigger {
    let mut trigger =
        crate::world::config::scripted_trigger(crate::world::config::TriggerCondition::Manual);
    trigger.id = Some(id.to_string());
    trigger.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
        id.to_string(),
        label.to_string(),
    ));
    ScriptTrigger {
        trigger,
        handler: "h".to_string(),
        source_path: path.to_string(),
    }
}

#[test]
fn distinct_gm_event_ids_are_clean_and_ordinary_triggers_are_ignored() {
    let sts = vec![
        gm_event_trigger("breach", "world.gm.event.breach", "w.toml#script.s"),
        gm_event_trigger("sweep", "world.gm.event.sweep", "w.toml#script.s"),
        script_trigger("on_loaded", "w.toml#script.s"),
    ];
    assert!(validate_gm_events(&sts).is_empty());
}

/// Deliberately stricter than `ResetTrigger`'s tolerant `Trigger::id`
/// lookup, which re-arms EVERY trigger sharing an id on purpose: a GM
/// action names exactly one event and must get exactly one handler run.
#[test]
fn a_duplicate_gm_event_id_blocks_activation() {
    let sts = vec![
        gm_event_trigger("breach", "world.gm.event.breach", "w.toml#script.a"),
        gm_event_trigger("breach", "world.gm.event.other", "w.toml#script.b"),
    ];
    let findings = validate_gm_events(&sts);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, INVALID_GM_EVENT);
    assert_eq!(findings[0].source.reference, "breach");
    assert_eq!(findings[0].source.file, "w.toml#script.b");
    assert!(crate::world::validate::has_error(&findings));
}

/// A control set that reaches this pass from anywhere but the `gm_event`
/// host fn — issue #1302's `gm_controls` on an ordinary trigger — meets the
/// same identity rule.
#[test]
fn a_malformed_gm_event_identity_blocks_activation_at_the_validation_pass_too() {
    for (id, label) in [("a::b", "world.gm.event.a"), ("ok", "")] {
        let findings = validate_gm_events(&[gm_event_trigger(id, label, "w.toml#script.s")]);
        assert_eq!(findings.len(), 1, "{id:?}/{label:?} must be refused");
        assert_eq!(findings[0].category, INVALID_GM_EVENT);
        assert!(crate::world::validate::has_error(&findings));
    }
}

/// Issue #1304: a Skip lever on a manual event is refused by THIS pass too,
/// not only by the `.skip()` host fn -- a control set that reaches here from
/// anywhere meets the same rule, which is what keeps the two from drifting.
///
/// The same manual event WITHOUT the lever stays clean: the finding is
/// about a lever that could never be consumed, not about manual events.
#[test]
fn a_skip_lever_on_a_manual_event_blocks_activation_at_the_validation_pass_too() {
    let mut skippable_manual =
        gm_event_trigger("breach", "world.gm.event.breach", "w.toml#script.s");
    skippable_manual
        .trigger
        .gm_controls
        .as_mut()
        .expect("controls")
        .skip = true;
    let findings = validate_gm_events(&[skippable_manual.clone()]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, INVALID_GM_EVENT);
    assert_eq!(findings[0].source.reference, "breach");
    assert!(crate::world::validate::has_error(&findings));

    // The same lever on an event that HAS occurrences is clean.
    let mut automatic = skippable_manual;
    automatic.trigger.condition = crate::world::config::TriggerCondition::OnDestroyed {
        entity_name: "courier".to_string(),
    };
    assert!(validate_gm_events(&[automatic]).is_empty());

    // And a manual event that declares no Skip is clean, as it always was.
    assert!(validate_gm_events(&[gm_event_trigger(
        "breach",
        "world.gm.event.breach",
        "w.toml#script.s"
    )])
    .is_empty());
}

/// Issue #1434: an invented GM-attention band is refused by THIS pass too,
/// not only by the `.attention_band(…)` host fn, so a control set that
/// reaches here from anywhere meets the same rule. A band nobody can parse
/// would otherwise land the beat silently in the default one — a priority
/// the author did not choose, on a live facilitator's desk.
#[test]
fn an_invented_attention_band_blocks_activation_at_the_validation_pass_too() {
    let mut invented = gm_event_trigger("breach", "world.gm.event.breach", "w.toml#script.s");
    invented
        .trigger
        .gm_controls
        .as_mut()
        .expect("controls")
        .attention_band = Some("critical".to_string());
    let findings = validate_gm_events(&[invented.clone()]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, INVALID_GM_EVENT);
    assert_eq!(findings[0].source.reference, "breach");
    // The refusal names what to write instead.
    assert!(
        findings[0].message.contains("'urgent'")
            && findings[0].message.contains("'attention'")
            && findings[0].message.contains("'background'"),
        "{}",
        findings[0].message
    );
    assert!(crate::world::validate::has_error(&findings));

    // Each of the three authored spellings is clean, and so is no band.
    for band in ["urgent", "attention", "background"] {
        let mut authored = invented.clone();
        authored
            .trigger
            .gm_controls
            .as_mut()
            .expect("controls")
            .attention_band = Some(band.to_string());
        assert!(validate_gm_events(&[authored]).is_empty(), "{band}");
    }
    assert!(validate_gm_events(&[gm_event_trigger(
        "breach",
        "world.gm.event.breach",
        "w.toml#script.s"
    )])
    .is_empty());
}

/// Issue #1302: the id space is ONE space across both authoring surfaces.
/// A manual `gm_event` and an automatic `gm_controls` declaration that
/// collide are the same ambiguity as two manual ones — a GM action naming
/// `breach` would have two handlers it could run — so the same pass refuses
/// it, and it does not matter which surface authored which row.
#[test]
fn a_manual_and_an_automatic_event_may_not_share_one_authored_id() {
    let mut automatic = gm_event_trigger("breach", "world.gm.event.breach", "w.toml#script.b");
    automatic.trigger.condition = crate::world::config::TriggerCondition::OnDestroyed {
        entity_name: "courier".to_string(),
    };
    let findings = validate_gm_events(&[
        gm_event_trigger("breach", "world.gm.event.breach", "w.toml#script.a"),
        automatic,
    ]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, INVALID_GM_EVENT);
    assert_eq!(findings[0].source.reference, "breach");
    assert!(crate::world::validate::has_error(&findings));

    // Two DISTINCT ids on the two surfaces are perfectly ordinary.
    let mut automatic = gm_event_trigger("evac", "world.gm.event.evac", "w.toml#script.b");
    automatic.trigger.condition = crate::world::config::TriggerCondition::OnDestroyed {
        entity_name: "courier".to_string(),
    };
    assert!(validate_gm_events(&[
        gm_event_trigger("breach", "world.gm.event.breach", "w.toml#script.a"),
        automatic,
    ])
    .is_empty());
}

// ── `flags` compound-assignment lint (issue #994) ─────────────────────────

fn src(path: &str, source: &str) -> ScriptSource {
    ScriptSource {
        path: path.to_string(),
        source: source.to_string(),
    }
}

#[test]
fn named_function_lines_ignore_comments_and_strings() {
    let source = "// fn phantom(ctx) {}\nlet text = `fn nope(ctx) {}`;\n\nprivate fn actual(ctx) {\n  ctx.flags.ready = 1;\n}\n";
    assert_eq!(
        named_function_lines(source),
        BTreeMap::from([("actual".to_string(), 4)])
    );
}

#[test]
fn a_flag_plus_equals_is_a_blocking_finding() {
    let sources = vec![src(
        "w.toml#script.s",
        r#"fn on_x(ctx) { flags.score += 50; }"#,
    )];
    let findings = validate_flag_opassign(&sources);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let f = &findings[0];
    assert!(f.is_error());
    assert_eq!(f.category, FLAG_OPASSIGN_NOT_COMPOSABLE);
    assert_eq!(f.source.file, "w.toml#script.s");
    assert_eq!(f.source.reference, "flags.score");
    assert_eq!(f.source.line, Some(1));
    assert!(f.message.contains("flags.increment"));
    assert!(crate::world::validate::has_error(&findings));
}

#[test]
fn the_real_ctx_flags_idiom_is_also_caught() {
    // The shipped spelling is `ctx.flags.x += n`, where `flags` sits mid-chain.
    let sources = vec![src(
        "w.toml#script.s",
        r#"fn on_x(ctx) { ctx.flags.score += 50; }"#,
    )];
    let findings = validate_flag_opassign(&sources);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, FLAG_OPASSIGN_NOT_COMPOSABLE);
    assert_eq!(findings[0].source.reference, "flags.score");
}

#[test]
fn every_compound_operator_is_rejected() {
    // Each of the compound-assignment operators degrades identically.
    for op in [
        "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", "&=", "|=", "^=",
    ] {
        let source = format!("fn on_x(ctx) {{ ctx.flags.score {op} 1; }}");
        let findings = validate_flag_opassign(&[src("s", &source)]);
        assert_eq!(findings.len(), 1, "operator {op} should fire: {findings:?}");
        assert_eq!(findings[0].category, FLAG_OPASSIGN_NOT_COMPOSABLE);
    }
}

#[test]
fn the_flag_index_form_is_rejected() {
    let sources = vec![src(
        "w.toml#script.s",
        r#"fn on_x(ctx) { ctx.flags["kills"] += 1; }"#,
    )];
    let findings = validate_flag_opassign(&sources);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, FLAG_OPASSIGN_NOT_COMPOSABLE);
    assert_eq!(findings[0].source.reference, "flags[…]");
}

#[test]
fn the_increment_verb_is_clean() {
    // The composable verb — the whole point of M3 — must not be flagged.
    let sources = vec![src(
        "s",
        r#"fn on_x(ctx) { ctx.flags.increment("score", 50); }"#,
    )];
    assert!(validate_flag_opassign(&sources).is_empty());
}

#[test]
fn a_plain_absolute_assign_is_clean() {
    // `flags.x = v` (and the index form) stay absolute and are allowed.
    let sources = vec![src(
        "s",
        r#"fn on_x(ctx) { ctx.flags.armed = 1; ctx.flags["kills"] = 3; }"#,
    )];
    assert!(validate_flag_opassign(&sources).is_empty());
}

#[test]
fn a_non_flags_opassign_is_not_flagged() {
    // A `+=` on a plain local must never be flagged (no false positives).
    let sources = vec![src(
        "s",
        r#"fn on_x(ctx) { let x = 0; x += 1; let score = 0; score += 5; }"#,
    )];
    assert!(validate_flag_opassign(&sources).is_empty());
}

#[test]
fn a_flag_opassign_in_a_comment_or_string_is_ignored() {
    // The stripping pass must not see `+=` inside a comment or a string —
    // otherwise even the module docs (which spell out `flags.x += 50`) would
    // trip the lint.
    let sources = vec![src(
        "s",
        r#"fn on_x(ctx) {
                // do NOT write flags.score += 50 here
                /* nor flags.score += 1 in a block comment */
                let note = "flags.score += 99";
                ctx.flags.increment("score", 1);
            }"#,
    )];
    assert!(validate_flag_opassign(&sources).is_empty());
}

#[test]
fn a_flag_read_is_not_a_write() {
    // Reading a flag (`let s = flags.score;`) is not an assignment.
    let sources = vec![src("s", r#"fn on_x(ctx) { let s = ctx.flags.score; }"#)];
    assert!(validate_flag_opassign(&sources).is_empty());
}

#[test]
fn each_offending_source_reports_independently() {
    let sources = vec![
        src("a", r#"fn on_a(ctx) { ctx.flags.a += 1; }"#),
        src("b", r#"fn on_b(ctx) { ctx.flags.increment("b", 1); }"#),
        src("c", r#"fn on_c(ctx) { ctx.flags["c"] += 1; }"#),
    ];
    let findings = validate_flag_opassign(&sources);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert_eq!(findings[0].source.file, "a");
    assert_eq!(findings[1].source.file, "c");
}

// ── dialogue `on_pick` resolution (issue #984) ────────────────────────────

const TREE: &str = r#"
        fn hail_axiom(ctx) {
            #{ message: "Go ahead.", responses: [
                #{ text: "Acknowledge", on_pick: "on_ack" },
                #{ text: "Decline",     on_pick: "on_declien", important: true },
            ] }
        }
        fn on_ack(ctx)     { ctx.effects.complete_objective("reach_axiom"); }
        fn on_decline(ctx) { ctx.effects.fail_objective("reach_axiom"); }
    "#;

#[test]
fn a_resolved_on_pick_is_clean() {
    let sources = vec![src(
        "w.toml#script.s",
        r#"
            fn root(ctx) {
                #{ message: "Go ahead.", responses: [
                    #{ text: "Yes", on_pick: "on_yes" },
                    #{ text: "No",  on_pick: "on_no" },
                ] }
            }
            fn on_yes(ctx) { }
            fn on_no(ctx) { }
            "#,
    )];
    assert!(validate_on_pick_fns(&sources, &defined(&["root", "on_yes", "on_no"])).is_empty());
}

#[test]
fn a_typoed_on_pick_blocks_activation_naming_the_fn_and_file() {
    let sources = vec![src("w.toml#script.axiom", TREE)];
    let findings =
        validate_on_pick_fns(&sources, &defined(&["hail_axiom", "on_ack", "on_decline"]));
    assert_eq!(findings.len(), 1, "{findings:?}");
    let f = &findings[0];
    assert_eq!(f.category, UNRESOLVED_ON_PICK_FN);
    assert_eq!(f.source.reference, "on_declien", "the finding names the fn");
    assert_eq!(
        f.source.file, "w.toml#script.axiom",
        "and the file it is authored in"
    );
    assert!(f.source.line.is_some(), "and the line");
    assert!(f.message.contains("on_declien"));
    assert!(crate::world::validate::has_error(&findings));
}

#[test]
fn a_dynamically_built_on_pick_is_not_flagged() {
    // The documented limitation of a lexical pass: only a literal is
    // visible. A computed name is left to the runtime's `EnterError::
    // Unresolved` rather than guessed at — a false positive here would block
    // a legitimate world at load, which is strictly worse.
    let sources = vec![src(
        "s",
        r#"
            fn root(ctx) {
                let kind = "ack";
                #{ message: "Go ahead.", responses: [
                    #{ text: "Yes", on_pick: pick_for(kind) },
                    #{ text: "No",  on_pick: "on_" + kind },
                ] }
            }
            "#,
    )];
    assert!(validate_on_pick_fns(&sources, &defined(&["root"])).is_empty());
}

#[test]
fn an_on_pick_in_a_comment_or_string_is_ignored() {
    // The tokenizer lexes a literal's contents as DATA, so an `on_pick:` that
    // is itself inside a comment or a string is not a response.
    let sources = vec![src(
        "s",
        r#"
            fn root(ctx) {
                // author responses as on_pick: "handler_name"
                let doc = "on_pick: \"never_defined\"";
                #{ message: "x", responses: [] }
            }
            "#,
    )];
    assert!(validate_on_pick_fns(&sources, &defined(&["root"])).is_empty());
}

#[test]
fn every_unresolved_on_pick_across_every_source_reports() {
    let sources = vec![
        src(
            "a",
            r#"fn a(ctx) { #{ responses: [ #{ on_pick: "gone_a" } ] } }"#,
        ),
        src(
            "b",
            r#"fn b(ctx) { #{ responses: [ #{ on_pick: "here_b" } ] } }"#,
        ),
        src(
            "c",
            r#"fn c(ctx) { #{ responses: [ #{ on_pick: "gone_c" } ] } }"#,
        ),
    ];
    let findings = validate_on_pick_fns(&sources, &defined(&["a", "b", "c", "here_b"]));
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert_eq!(findings[0].source.reference, "gone_a");
    assert_eq!(findings[1].source.reference, "gone_c");
}

// ── Named-deadline pairing (issue #1024) ─────────────────────────────────

fn deadline_world(ids: &[&str]) -> toml::Value {
    let blocks: String = ids
        .iter()
        .map(|id| format!("[[deadline]]\nid = \"{id}\"\ndue_secs = 60\n\n"))
        .collect();
    toml::from_str(&blocks).expect("the fixture world parses")
}

fn handler(deadline_id: &str, handler: &str) -> crate::world::deadlines::DeadlineHandler {
    crate::world::deadlines::DeadlineHandler {
        deadline_id: deadline_id.to_string(),
        handler: handler.to_string(),
        source_path: "w.toml#script.setup".to_string(),
    }
}

#[test]
fn a_paired_deadline_and_handler_produce_no_finding() {
    let defined: BTreeSet<String> = ["on_window".to_string()].into_iter().collect();
    let findings = validate_deadline_handlers(
        "w.toml",
        &deadline_world(&["window"]),
        &[handler("window", "on_window")],
        &defined,
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn an_on_deadline_naming_an_undefined_fn_is_an_error() {
    // The same cross-reference every other handler name gets.
    let findings = validate_deadline_handlers(
        "w.toml",
        &deadline_world(&["window"]),
        &[handler("window", "gone")],
        &BTreeSet::new(),
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
    assert_eq!(findings[0].source.reference, "gone");
    assert!(findings[0].is_error());
}

#[test]
fn an_on_deadline_naming_an_unauthored_deadline_is_an_error() {
    let defined: BTreeSet<String> = ["on_window".to_string()].into_iter().collect();
    let findings = validate_deadline_handlers(
        "w.toml",
        &deadline_world(&["window"]),
        &[handler("windwo", "on_window")],
        &defined,
    );
    // Two: the typo names no block, AND the real block is left unclaimed.
    // Both are reported, because a designer reading only the first would fix
    // half of a two-sided mistake.
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(findings.iter().all(|f| f.category == DEADLINE_NOT_PAIRED));
    assert_eq!(findings[0].source.reference, "windwo");
    assert_eq!(findings[1].source.reference, "window");
}

#[test]
fn an_authored_deadline_with_no_handler_is_an_error() {
    // Not "a deadline that does nothing" — a deadline that cannot be ARMED,
    // because arming it means queuing the call it runs. Left as a warning it
    // would surface mid-mission as a countdown reaching zero and nothing
    // happening, which no runtime check can report.
    let defined: BTreeSet<String> = ["on_window".to_string()].into_iter().collect();
    let findings = validate_deadline_handlers(
        "w.toml",
        &deadline_world(&["window", "collapse"]),
        &[handler("window", "on_window")],
        &defined,
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, DEADLINE_NOT_PAIRED);
    assert_eq!(findings[0].source.reference, "collapse");
    assert!(
        findings[0].message.contains("on_deadline"),
        "the message says how to fix it"
    );
    assert!(findings[0].is_error());
}

#[test]
fn a_world_with_neither_deadlines_nor_registrations_is_silent() {
    // Every shipped world today. The pass must be a no-op for them.
    let findings = validate_deadline_handlers(
        "w.toml",
        &toml::Value::Table(toml::map::Map::new()),
        &[],
        &BTreeSet::new(),
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn raw_lexer_handles_truncated_escape_at_end_of_source() {
    for quote in ['"', '\''] {
        let source = format!("ctx.spawn({quote}truncated\\");
        let tokens = lex_significant_raw(&source);
        assert!(!tokens.is_empty());
        assert!(tokens.last().unwrap().1.ends_with('\\'));
        assert!(named_function_lines(&source).is_empty());
    }
}
