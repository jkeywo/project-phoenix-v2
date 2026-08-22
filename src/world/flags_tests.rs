use super::*;

// --- Predicate::render (issue #1152) -----------------------------------

#[test]
fn render_round_trips_a_fact_atom_with_a_param_operand() {
    let p = parse_predicate("fact(range_to_target) < param(orbit_range)").unwrap();
    assert_eq!(p.render(), "fact(range_to_target) < param(orbit_range)");
}

#[test]
fn render_names_each_context_and_the_state_time_atom() {
    assert_eq!(
        parse_predicate("memory(engagements) >= 3")
            .unwrap()
            .render(),
        "memory(engagements) >= 3"
    );
    // `state_time` takes no argument and renders bare.
    assert_eq!(
        parse_predicate("state_time >= param(dwell)")
            .unwrap()
            .render(),
        "state_time >= param(dwell)"
    );
    assert_eq!(
        parse_predicate("flag(general_quarters)").unwrap().render(),
        "flag(general_quarters)"
    );
    assert_eq!(
        parse_predicate("counter(kills) > 2").unwrap().render(),
        "counter(kills) > 2"
    );
}

#[test]
fn render_parenthesises_composed_guards() {
    // A three-way `and` is a readable, unambiguous expression once rendered;
    // the surface only needs the guard back verbatim enough to read.
    let p = parse_predicate(
        "fact(hazard_urgency) > param(surge) and fact(boost_available) > 0 \
         and memory(engagements) < param(cap)",
    )
    .unwrap();
    let rendered = p.render();
    assert!(
        rendered.contains("fact(hazard_urgency) > param(surge)"),
        "{rendered}"
    );
    assert!(rendered.contains("fact(boost_available) > 0"), "{rendered}");
    assert!(
        rendered.contains("memory(engagements) < param(cap)"),
        "{rendered}"
    );
    assert!(rendered.contains(" and "), "{rendered}");
}

// --- FlagStore basics --------------------------------------------------

#[test]
fn unset_flag_reads_false_and_zero() {
    let store = FlagStore::new();
    assert!(!store.flag("missing"));
    assert_eq!(store.counter("missing"), 0);
}

#[test]
fn set_flag_then_read_true() {
    let mut store = FlagStore::new();
    let (before, after) = store.set_flag("a");
    assert_eq!(before, 0);
    assert_eq!(after, 1);
    assert!(store.flag("a"));
    assert_eq!(store.counter("a"), 1);
}

#[test]
fn clear_flag_resets_to_false() {
    let mut store = FlagStore::new();
    store.set_flag("a");
    let (before, after) = store.clear_flag("a");
    assert_eq!(before, 1);
    assert_eq!(after, 0);
    assert!(!store.flag("a"));
}

#[test]
fn increment_flag_accumulates() {
    let mut store = FlagStore::new();
    let (_, a1) = store.increment_flag("k", 3);
    let (b2, a2) = store.increment_flag("k", 4);
    assert_eq!(a1, 3);
    assert_eq!(b2, 3);
    assert_eq!(a2, 7);
    assert_eq!(store.counter("k"), 7);
    // Boolean view: any non-zero counter is true.
    assert!(store.flag("k"));
}

#[test]
fn nonzero_counter_reads_true_via_flag() {
    let mut store = FlagStore::new();
    store.set_flag_value("c", 5);
    assert!(store.flag("c"));
    assert_eq!(store.counter("c"), 5);
}

#[test]
fn set_flag_value_zero_clears_storage() {
    let mut store = FlagStore::new();
    store.set_flag_value("c", 5);
    let (before, after) = store.set_flag_value("c", 0);
    assert_eq!(before, 5);
    assert_eq!(after, 0);
    assert!(!store.flag("c"));
}

// --- Parent chain ------------------------------------------------------

#[test]
fn parent_prefix_walks_one_layer_up() {
    let mut parent = FlagStore::new();
    parent.set_flag("a");
    let child = FlagStore::new();
    assert!(flag_in_chain(&[&child, &parent], "parent:a"));
    // No prefix → looks in current (child), which is unset.
    assert!(!flag_in_chain(&[&child, &parent], "a"));
}

#[test]
fn parent_prefix_walks_multiple_layers_up() {
    let mut root = FlagStore::new();
    root.set_flag_value("k", 7);
    let mid = FlagStore::new();
    let child = FlagStore::new();
    let chain: &[&FlagStore] = &[&child, &mid, &root];
    assert_eq!(counter_in_chain(chain, "parent:parent:k"), 7);
    assert!(flag_in_chain(chain, "parent:parent:k"));
}

#[test]
fn parent_past_root_resolves_as_unset() {
    let store = FlagStore::new();
    let chain: &[&FlagStore] = &[&store];
    assert!(!flag_in_chain(chain, "parent:parent:anything"));
    assert_eq!(counter_in_chain(chain, "parent:parent:anything"), 0);
}

// --- Predicate parsing & evaluation ------------------------------------

fn store_with(pairs: &[(&str, i64)]) -> FlagStore {
    let mut s = FlagStore::new();
    for (k, v) in pairs {
        s.set_flag_value(k, *v);
    }
    s
}

#[test]
fn parse_flag_atom() {
    let pred = parse_predicate("flag(a)").unwrap();
    assert_eq!(pred, Predicate::Flag { name: "a".into() });
}

#[test]
fn parse_counter_with_ge() {
    let pred = parse_predicate("counter(x) >= 5").unwrap();
    assert_eq!(
        pred,
        Predicate::Counter {
            name: "x".into(),
            op: CmpOp::Ge,
            rhs: 5
        }
    );
}

#[test]
fn parse_all_comparison_operators() {
    for (src, op) in [
        ("counter(x) >= 1", CmpOp::Ge),
        ("counter(x) > 1", CmpOp::Gt),
        ("counter(x) == 1", CmpOp::Eq),
        ("counter(x) != 1", CmpOp::Ne),
        ("counter(x) <= 1", CmpOp::Le),
        ("counter(x) < 1", CmpOp::Lt),
    ] {
        let p = parse_predicate(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(
            p,
            Predicate::Counter {
                name: "x".into(),
                op,
                rhs: 1
            }
        );
    }
}

#[test]
fn parse_negative_integer_in_counter_comparison() {
    let pred = parse_predicate("counter(x) >= -3").unwrap();
    assert_eq!(
        pred,
        Predicate::Counter {
            name: "x".into(),
            op: CmpOp::Ge,
            rhs: -3
        }
    );
}

#[test]
fn parse_parent_prefix_in_name() {
    let pred = parse_predicate("flag(parent:parent:goal)").unwrap();
    assert_eq!(
        pred,
        Predicate::Flag {
            name: "parent:parent:goal".into()
        }
    );
}

#[test]
fn parse_and_precedence_higher_than_or() {
    // a or b and c → a or (b and c)
    let pred = parse_predicate("flag(a) or flag(b) and flag(c)").unwrap();
    match pred {
        Predicate::Or(lhs, rhs) => {
            assert_eq!(*lhs, Predicate::Flag { name: "a".into() });
            assert!(matches!(*rhs, Predicate::And(_, _)));
        }
        other => panic!("expected Or at root, got {other:?}"),
    }
}

#[test]
fn parse_not_binds_tightest() {
    // not a and b → (not a) and b
    let pred = parse_predicate("not flag(a) and flag(b)").unwrap();
    match pred {
        Predicate::And(lhs, _) => assert!(matches!(*lhs, Predicate::Not(_))),
        other => panic!("expected And at root, got {other:?}"),
    }
}

#[test]
fn parens_override_precedence() {
    // (a or b) and c
    let pred = parse_predicate("(flag(a) or flag(b)) and flag(c)").unwrap();
    match pred {
        Predicate::And(lhs, _) => assert!(matches!(*lhs, Predicate::Or(_, _))),
        other => panic!("expected And at root, got {other:?}"),
    }
}

#[test]
fn evaluate_flag_atom_against_store() {
    let s = store_with(&[("a", 1)]);
    let pred = parse_predicate("flag(a)").unwrap();
    assert!(pred.evaluate(&[&s]));
    let s2 = FlagStore::new();
    assert!(!pred.evaluate(&[&s2]));
}

#[test]
fn evaluate_counter_comparison() {
    let s = store_with(&[("kills", 4)]);
    assert!(parse_predicate("counter(kills) >= 4")
        .unwrap()
        .evaluate(&[&s]));
    assert!(!parse_predicate("counter(kills) > 4")
        .unwrap()
        .evaluate(&[&s]));
    assert!(parse_predicate("counter(kills) == 4")
        .unwrap()
        .evaluate(&[&s]));
    assert!(parse_predicate("counter(missing) == 0")
        .unwrap()
        .evaluate(&[&s]));
}

#[test]
fn evaluate_compound_expression() {
    let s = store_with(&[("a", 1), ("b", 0), ("c", 1)]);
    // (a and not b) or c
    let p = parse_predicate("(flag(a) and not flag(b)) or flag(c)").unwrap();
    assert!(p.evaluate(&[&s]));
}

#[test]
fn evaluate_compound_short_circuit_or_with_false_lhs() {
    let s = store_with(&[("b", 1)]);
    let p = parse_predicate("flag(a) or flag(b)").unwrap();
    assert!(p.evaluate(&[&s]));
}

#[test]
fn evaluate_parent_reference_in_predicate() {
    let mut parent = FlagStore::new();
    parent.set_flag("phase_done");
    let child = FlagStore::new();
    let p = parse_predicate("flag(parent:phase_done)").unwrap();
    assert!(p.evaluate(&[&child, &parent]));
}

#[test]
fn round_trip_representative_expressions() {
    for src in [
        "flag(a)",
        "not flag(a)",
        "flag(a) and flag(b)",
        "flag(a) or flag(b)",
        "(flag(a) or flag(b)) and not flag(c)",
        "counter(x) >= 5 and counter(y) < 10",
        "flag(parent:phase) or counter(parent:parent:kills) > 0",
    ] {
        parse_predicate(src).unwrap_or_else(|e| panic!("'{src}' parse failed: {e}"));
    }
}

// --- Parser error diagnostics -----------------------------------------

#[test]
fn parse_empty_predicate_errors() {
    let err = parse_predicate("").unwrap_err();
    assert!(err.to_lowercase().contains("empty"), "got: {err}");
}

#[test]
fn parse_unbalanced_paren_errors_without_panic() {
    let err = parse_predicate("(flag(a)").unwrap_err();
    assert!(
        err.contains("')'") || err.contains("end of predicate"),
        "got: {err}"
    );
}

#[test]
fn parse_missing_atom_errors() {
    let err = parse_predicate("flag(a) and").unwrap_err();
    assert!(err.contains("end of predicate"), "got: {err}");
}

#[test]
fn parse_unknown_character_reports_position() {
    let err = parse_predicate("flag(a) & flag(b)").unwrap_err();
    assert!(err.contains("position"), "got: {err}");
    assert!(
        err.contains('&'),
        "error must mention offending char: {err}"
    );
}

#[test]
fn parse_trailing_garbage_errors() {
    let err = parse_predicate("flag(a) flag(b)").unwrap_err();
    assert!(err.to_lowercase().contains("trailing"), "got: {err}");
}

#[test]
fn parse_counter_without_operator_errors() {
    let err = parse_predicate("counter(a)").unwrap_err();
    assert!(err.contains("comparison operator"), "got: {err}");
}

#[test]
fn parse_counter_with_non_integer_rhs_errors() {
    let err = parse_predicate("counter(a) >= foo").unwrap_err();
    assert!(err.contains("integer"), "got: {err}");
}

#[test]
fn parse_bare_equals_errors_with_hint() {
    let err = parse_predicate("counter(a) = 1").unwrap_err();
    assert!(err.contains("=="), "should hint at ==, got: {err}");
}

// --- Typed AI facts + named parameters (issue #775) --------------------

fn facts_with(pairs: &[(&str, f64)]) -> AiFacts {
    let mut f = AiFacts::new();
    for (k, v) in pairs {
        f.set(k, *v);
    }
    f
}

fn params_with(pairs: &[(&str, f64)]) -> AiParams {
    let mut p = AiParams::new();
    for (k, v) in pairs {
        p.set(k, *v);
    }
    p
}

#[test]
fn parse_fact_atom_with_numeric_rhs() {
    let pred = parse_predicate("fact(secs_since_combat) < 10.0").unwrap();
    assert_eq!(
        pred,
        Predicate::Fact {
            context: FactContext::SelfCtx,
            name: "secs_since_combat".into(),
            op: CmpOp::Lt,
            rhs: Operand::Number(10.0),
        }
    );
}

#[test]
fn parse_fact_atom_with_param_rhs() {
    let pred = parse_predicate("fact(secs_since_combat) < param(combat_window_secs)").unwrap();
    assert_eq!(
        pred,
        Predicate::Fact {
            context: FactContext::SelfCtx,
            name: "secs_since_combat".into(),
            op: CmpOp::Lt,
            rhs: Operand::Param("combat_window_secs".into()),
        }
    );
}

#[test]
fn parse_true_and_false_literals() {
    assert_eq!(parse_predicate("true").unwrap(), Predicate::Bool(true));
    assert_eq!(parse_predicate("false").unwrap(), Predicate::Bool(false));
}

#[test]
fn evaluate_fact_against_named_parameter_table() {
    let p = parse_predicate("fact(secs_since_combat) < param(combat_window_secs)").unwrap();
    let params = params_with(&[("combat_window_secs", 10.0)]);
    let no_flags: &[&FlagStore] = &[];
    // In combat 3s ago → within the 10s window → true.
    assert!(p.evaluate_with(
        &facts_with(&[("secs_since_combat", 3.0)]),
        &params,
        no_flags
    ));
    // 12s ago → outside the window → false.
    assert!(!p.evaluate_with(
        &facts_with(&[("secs_since_combat", 12.0)]),
        &params,
        no_flags
    ));
    // Exactly at the boundary → strict `<` is false.
    assert!(!p.evaluate_with(
        &facts_with(&[("secs_since_combat", 10.0)]),
        &params,
        no_flags
    ));
}

#[test]
fn absent_fact_evaluates_false_never_panics() {
    let p = parse_predicate("fact(secs_since_combat) < param(w)").unwrap();
    let params = params_with(&[("w", 10.0)]);
    // No `secs_since_combat` reading at all → false (not in combat).
    assert!(!p.evaluate_with(&AiFacts::new(), &params, &[]));
}

#[test]
fn unresolved_parameter_evaluates_false_never_panics() {
    let p = parse_predicate("fact(x) < param(missing)").unwrap();
    // Parameter `missing` is not authored → comparison is false.
    assert!(!p.evaluate_with(&facts_with(&[("x", 1.0)]), &AiParams::new(), &[]));
}

#[test]
fn bool_literal_evaluates_to_itself() {
    assert!(parse_predicate("true")
        .unwrap()
        .evaluate_with(&AiFacts::new(), &AiParams::new(), &[]));
    assert!(!parse_predicate("false").unwrap().evaluate_with(
        &AiFacts::new(),
        &AiParams::new(),
        &[]
    ));
}

#[test]
fn facts_compose_with_flags_and_boolean_operators() {
    let s = store_with(&[("shooting", 1)]);
    let p = parse_predicate("flag(shooting) or fact(secs_since_combat) < param(w)").unwrap();
    let params = params_with(&[("w", 10.0)]);
    // flag(shooting) is true → whole OR true even with no fact.
    assert!(p.evaluate_with(&AiFacts::new(), &params, &[&s]));
}

#[test]
fn world_flag_evaluate_ignores_facts() {
    // The world-trigger `evaluate` entry point still works and reads flags.
    let s = store_with(&[("a", 1)]);
    assert!(parse_predicate("flag(a)").unwrap().evaluate(&[&s]));
}

#[test]
fn parse_fact_without_operand_errors_without_panic() {
    let err = parse_predicate("fact(x) <").unwrap_err();
    assert!(err.contains("end of predicate"), "got: {err}");
}

#[test]
fn parse_fact_with_bad_operand_errors_without_panic() {
    let err = parse_predicate("fact(x) < flag(y)").unwrap_err();
    assert!(err.contains("number or param"), "got: {err}");
}

#[test]
fn parse_integer_rhs_for_fact_is_widened() {
    // A fact comparison may use a bare integer; it widens to f64.
    let p = parse_predicate("fact(x) >= 5").unwrap();
    assert_eq!(
        p,
        Predicate::Fact {
            context: FactContext::SelfCtx,
            name: "x".into(),
            op: CmpOp::Ge,
            rhs: Operand::Number(5.0),
        }
    );
    assert!(p.evaluate_with(&facts_with(&[("x", 5.0)]), &AiParams::new(), &[]));
}

// --- Three-context selector grammar (issue #776) -----------------------

fn factset(
    self_pairs: &[(&str, f64)],
    candidate_pairs: &[(&str, f64)],
    target_pairs: &[(&str, f64)],
) -> AiFactSet {
    AiFactSet {
        self_facts: facts_with(self_pairs),
        candidate_facts: facts_with(candidate_pairs),
        target_facts: facts_with(target_pairs),
    }
}

#[test]
fn parse_bare_fact_is_self_context() {
    // #775 back-compat: bare `fact(...)` still parses as the SELF context.
    let p = parse_predicate("fact(secs_since_combat) < 10").unwrap();
    assert!(matches!(
        p,
        Predicate::Fact {
            context: FactContext::SelfCtx,
            ..
        }
    ));
}

#[test]
fn parse_three_context_fact_keywords() {
    for (src, ctx) in [
        ("self_fact(power_rating) >= 5", FactContext::SelfCtx),
        ("candidate_fact(hostile) > 0", FactContext::Candidate),
        ("target_fact(distance) < 100", FactContext::Target),
    ] {
        let p = parse_predicate(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(p, Predicate::Fact { context, .. } if context == ctx),
            "{src} should parse in context {ctx:?}"
        );
    }
}

#[test]
fn selector_reads_each_context_independently() {
    // The same fact name resolves to a different value per context.
    let p = parse_predicate("candidate_fact(dist) < self_fact(dist)");
    // `self_fact(dist)` is not a valid operand rhs — operands are numbers or
    // param(...). So compare candidate vs a param instead.
    assert!(p.is_err(), "a fact atom is not a valid comparison operand");

    let p =
        parse_predicate("candidate_fact(hostile) > 0 and candidate_fact(dist) < target_fact(dist)");
    assert!(p.is_err(), "target_fact is likewise not a valid operand");

    // The supported shape: each context compared to a literal / param.
    let p = parse_predicate(
        "self_fact(power_rating) >= param(min_rating) and candidate_fact(hostile) > 0 and target_fact(locked) > 0",
    )
    .unwrap();
    let params = params_with(&[("min_rating", 4.0)]);
    // All three contexts satisfy their clause.
    assert!(p.evaluate_selector(
        &factset(
            &[("power_rating", 5.0)],
            &[("hostile", 1.0)],
            &[("locked", 1.0)]
        ),
        &params,
        &[],
    ));
    // Candidate not hostile → whole conjunction false.
    assert!(!p.evaluate_selector(
        &factset(
            &[("power_rating", 5.0)],
            &[("hostile", 0.0)],
            &[("locked", 1.0)]
        ),
        &params,
        &[],
    ));
    // Self power rating below the authored floor → false.
    assert!(!p.evaluate_selector(
        &factset(
            &[("power_rating", 3.0)],
            &[("hostile", 1.0)],
            &[("locked", 1.0)]
        ),
        &params,
        &[],
    ));
}

#[test]
fn absent_candidate_and_target_facts_evaluate_false() {
    // A candidate/target atom with no reading in that context is false,
    // never a panic — the same contract SELF facts already carry.
    let p = parse_predicate("candidate_fact(hidden) > 0").unwrap();
    assert!(!p.evaluate_selector(&AiFactSet::default(), &AiParams::new(), &[]));
    let p = parse_predicate("target_fact(anything) >= 1").unwrap();
    assert!(!p.evaluate_selector(&AiFactSet::default(), &AiParams::new(), &[]));
}

#[test]
fn evaluate_with_treats_bare_fact_as_self_and_ignores_other_contexts() {
    // The #775 entry point still evaluates bare `fact(...)` as SELF and a
    // stray candidate atom (no candidate context supplied) reads false.
    let p = parse_predicate("fact(x) >= 1 and not candidate_fact(y) > 0").unwrap();
    assert!(p.evaluate_with(&facts_with(&[("x", 2.0)]), &AiParams::new(), &[]));
}

#[test]
fn context_fact_keywords_usable_as_flag_names() {
    // The new keywords remain legal flag/counter names (unquoted).
    let s = store_with(&[("candidate_fact", 1)]);
    assert!(parse_predicate("flag(candidate_fact)")
        .unwrap()
        .evaluate(&[&s]));
}

// ── Private memory + state time atoms (issue #882) ───────────────────────

fn memory_with(pairs: &[(&str, f64)], state_time: f64) -> AiPolicyMemory {
    let mut m = AiPolicyMemory::new();
    for (k, v) in pairs {
        m.set(k, *v);
    }
    m.set_state_time_secs(state_time);
    m
}

#[test]
fn memory_atom_reads_the_owning_systems_bag() {
    let p = parse_predicate("memory(engagements) >= param(limit)").unwrap();
    let mut params = AiParams::new();
    params.set("limit", 2.0);
    assert!(p.evaluate_stateful(
        &AiFacts::new(),
        &memory_with(&[("engagements", 3.0)], 0.0),
        &params,
        &[]
    ));
    assert!(!p.evaluate_stateful(
        &AiFacts::new(),
        &memory_with(&[("engagements", 1.0)], 0.0),
        &params,
        &[]
    ));
}

#[test]
fn state_time_atom_reads_the_state_clock() {
    let p = parse_predicate("state_time >= 3.0").unwrap();
    assert!(!p.evaluate_stateful(
        &AiFacts::new(),
        &memory_with(&[], 2.9),
        &AiParams::new(),
        &[]
    ));
    assert!(p.evaluate_stateful(
        &AiFacts::new(),
        &memory_with(&[], 3.0),
        &AiParams::new(),
        &[]
    ));
}

#[test]
fn absent_memory_evaluates_false_and_never_panics() {
    // The absent-→-false / no-panic contract, on the private atoms too.
    let p = parse_predicate("memory(never_written) > 0").unwrap();
    assert!(!p.evaluate_stateful(
        &AiFacts::new(),
        &AiPolicyMemory::new(),
        &AiParams::new(),
        &[]
    ));
    // And through the STATELESS entry point, where no bag exists at all:
    // a stateless policy can never read private state (content validation
    // rejects that authoring outright — this is the belt to that braces).
    assert!(!p.evaluate_with(&AiFacts::new(), &AiParams::new(), &[]));
    assert!(!parse_predicate("state_time > 0").unwrap().evaluate_with(
        &AiFacts::new(),
        &AiParams::new(),
        &[]
    ));
}

#[test]
fn memory_is_not_a_fact_and_a_fact_is_not_memory() {
    // The two namespaces are disjoint: seeding `x` as a world fact does not
    // satisfy `memory(x)`, and vice versa.
    let mem_pred = parse_predicate("memory(x) > 0").unwrap();
    let fact_pred = parse_predicate("fact(x) > 0").unwrap();
    let facts = facts_with(&[("x", 5.0)]);
    let memory = memory_with(&[("x", 5.0)], 0.0);
    assert!(!mem_pred.evaluate_stateful(&facts, &AiPolicyMemory::new(), &AiParams::new(), &[]));
    assert!(!fact_pred.evaluate_stateful(&AiFacts::new(), &memory, &AiParams::new(), &[]));
    assert!(mem_pred.evaluate_stateful(&AiFacts::new(), &memory, &AiParams::new(), &[]));
    assert!(fact_pred.evaluate_stateful(&facts, &AiPolicyMemory::new(), &AiParams::new(), &[]));
}

#[test]
fn private_atom_references_are_reportable_for_content_validation() {
    let p = parse_predicate("memory(a) > 0 and (state_time < 5 or memory(b) == 1)").unwrap();
    let mut refs = Vec::new();
    p.referenced_memory(&mut refs);
    refs.sort();
    assert_eq!(refs, vec!["a".to_string(), "b".to_string()]);
    assert!(p.references_state_time());

    let stateless = parse_predicate("fact(x) > 0 and flag(y)").unwrap();
    let mut refs = Vec::new();
    stateless.referenced_memory(&mut refs);
    assert!(refs.is_empty());
    assert!(!stateless.references_state_time());
}

#[test]
fn private_keywords_remain_usable_as_flag_and_fact_names() {
    // `memory` / `state_time` were legal identifiers before #882; a world
    // trigger or fact that used them must keep parsing.
    let s = store_with(&[("memory", 1), ("state_time", 1)]);
    assert!(parse_predicate("flag(memory)").unwrap().evaluate(&[&s]));
    assert!(parse_predicate("flag(state_time)").unwrap().evaluate(&[&s]));
    assert!(parse_predicate("fact(state_time) > 0")
        .unwrap()
        .evaluate_with(&facts_with(&[("state_time", 4.0)]), &AiParams::new(), &[]));
}

// ── Authored history operators (issue #890) ─────────────────────────────

/// Drive one policy's history bag with a series of readings for one fact,
/// one reading per shared tick, and return the bag.
fn folded(fact: &str, ticks: usize, samples: &[Option<f64>]) -> AiPolicyMemory {
    let spec = HistorySpec {
        fact: fact.to_string(),
        ticks,
    };
    let mut memory = AiPolicyMemory::new();
    for sample in samples {
        let mut facts = AiFacts::new();
        if let Some(v) = sample {
            facts.set(fact, *v);
        }
        memory.fold_history(std::slice::from_ref(&spec), &facts);
    }
    memory
}

fn history_says(src: &str, memory: &AiPolicyMemory, params: &AiParams) -> bool {
    parse_predicate(src)
        .expect("history expression parses")
        .evaluate_stateful(&AiFacts::new(), memory, params, &[])
}

#[test]
fn a_history_atom_parses_with_a_literal_and_with_a_param_window() {
    let literal = parse_predicate("history(min, range_to_target, 8) >= 40").unwrap();
    let Predicate::History {
        reducer,
        window,
        op,
        rhs,
    } = literal
    else {
        panic!("a history atom must parse to Predicate::History");
    };
    assert_eq!(reducer, HistoryReducer::Min);
    assert_eq!(window.fact, "range_to_target");
    assert_eq!(window.ticks, Operand::Number(8.0));
    assert_eq!(op, CmpOp::Ge);
    assert_eq!(rhs, Operand::Number(40.0));

    let authored =
        parse_predicate("history(net_change, range_to_target, param(w)) > param(gain)").unwrap();
    let Predicate::History { window, rhs, .. } = authored else {
        panic!("a history atom must parse to Predicate::History");
    };
    assert_eq!(window.ticks, Operand::Param("w".into()));
    assert_eq!(rhs, Operand::Param("gain".into()));
}

/// AC: the THRESHOLD-OVER-WINDOW shape — "every sample in the window
/// satisfies a predicate" — is authorable, and reads false until the window
/// has actually been held for the authored span.
#[test]
fn threshold_over_window_answers_held_only_once_the_whole_window_qualifies() {
    let params = params_with(&[("standoff_ticks", 3.0), ("safe_range", 40.0)]);
    let guard = "history(min, range_to_target, param(standoff_ticks)) >= param(safe_range)";

    // Two good samples of an authored three: not held yet.
    let two = folded("range_to_target", 3, &[Some(50.0), Some(50.0)]);
    assert!(
        !history_says(guard, &two, &params),
        "a partly-filled window is not a maintained distance"
    );

    let three = folded("range_to_target", 3, &[Some(50.0); 3]);
    assert!(history_says(guard, &three, &params));

    // One breach anywhere in the window answers no...
    let breached = folded("range_to_target", 3, &[Some(50.0), Some(10.0), Some(50.0)]);
    assert!(!history_says(guard, &breached, &params));
    // ...and stops mattering once it ages out, which is the property a
    // running minimum could never give.
    let recovered = folded(
        "range_to_target",
        3,
        &[Some(50.0), Some(10.0), Some(50.0), Some(50.0), Some(50.0)],
    );
    assert!(
        history_says(guard, &recovered, &params),
        "a stale breach must age out of a bounded window"
    );
}

/// The equivalence that justifies expressing "has held" as a reducer plus a
/// comparison rather than inventing a `held(...)` predicate: `min` over a
/// full window compared `>=` IS `BoundedHistory::all_at_least`.
#[test]
fn min_over_a_full_window_is_exactly_all_at_least() {
    let params = params_with(&[("w", 4.0), ("t", 12.5)]);
    let guard = "history(min, x, param(w)) >= param(t)";
    for series in [
        vec![20.0, 30.0, 40.0, 50.0],
        vec![12.5, 12.5, 12.5, 12.5],
        vec![12.4, 99.0, 99.0, 99.0],
        vec![99.0, 99.0, 99.0, 12.4],
    ] {
        let mut window = BoundedHistory::new(4);
        for v in &series {
            window.push(*v);
        }
        let bag = folded("x", 4, &series.iter().map(|v| Some(*v)).collect::<Vec<_>>());
        assert_eq!(
            history_says(guard, &bag, &params),
            window.all_at_least(12.5),
            "history(min, …) >= t must agree with all_at_least(t) for {series:?}"
        );
    }
}

/// AC: the NET-CHANGE-OVER-WINDOW shape — "is the quantity trending" — is
/// authorable, keeps its sign, and is absent (so `false`) until the window
/// spans the authored length.
#[test]
fn net_change_over_window_answers_the_trend_with_its_sign() {
    let params = params_with(&[("w", 3.0), ("min_progress", 5.0)]);
    let opening = "history(net_change, range_to_target, param(w)) > param(min_progress)";
    let closing = "history(net_change, range_to_target, param(w)) < 0";

    let short = folded("range_to_target", 3, &[Some(10.0), Some(40.0)]);
    assert!(
        !history_says(opening, &short, &params),
        "a window shorter than the authored span measures a span nobody authored"
    );

    let opened = folded("range_to_target", 3, &[Some(10.0), Some(20.0), Some(40.0)]);
    assert!(history_says(opening, &opened, &params));
    assert!(!history_says(closing, &opened, &params));

    let closed = folded("range_to_target", 3, &[Some(40.0), Some(20.0), Some(10.0)]);
    assert!(!history_says(opening, &closed, &params));
    assert!(
        history_says(closing, &closed, &params),
        "a shrinking gap is a NEGATIVE net change, not an absolute distance"
    );
}

/// The shape #789 needed and could not express: two windows over the SAME
/// reading with independent authored lengths, tuned for different questions.
#[test]
fn two_windows_over_one_fact_keep_independent_authored_lengths() {
    let specs = vec![
        HistorySpec {
            fact: "range".into(),
            ticks: 2,
        },
        HistorySpec {
            fact: "range".into(),
            ticks: 5,
        },
    ];
    let mut memory = AiPolicyMemory::new();
    for v in [10.0, 20.0, 30.0] {
        let mut facts = AiFacts::new();
        facts.set("range", v);
        memory.fold_history(&specs, &facts);
    }
    assert_eq!(memory.history().len(), 2, "two windows, not one shared one");
    assert_eq!(
        memory
            .history()
            .reduce(&specs[0], HistoryReducer::NetChange),
        Some(10.0),
        "the short window spans the last two readings"
    );
    assert_eq!(
        memory
            .history()
            .reduce(&specs[1], HistoryReducer::NetChange),
        None,
        "the long window is not full yet, so it has no span to measure"
    );
}

/// An absent reading is a HOLE, not a skipped tick: a window that closed
/// over one would span more real time than its authored length while
/// claiming not to.
#[test]
fn an_absent_reading_clears_the_window_instead_of_spanning_the_gap() {
    let params = params_with(&[("w", 3.0)]);
    let guard = "history(min, x, param(w)) >= 0";
    let spanning = folded("x", 3, &[Some(1.0), Some(1.0), None, Some(1.0)]);
    assert!(
        !history_says(guard, &spanning, &params),
        "the window must restart after a tick with no reading"
    );
    let refilled = folded("x", 3, &[Some(1.0), None, Some(1.0), Some(1.0), Some(1.0)]);
    assert!(history_says(guard, &refilled, &params));
}

/// No unbounded buffers: the window keeps exactly its authored capacity
/// however long the scenario runs, and the SET of windows is the authored
/// set rather than everything ever asked for.
#[test]
fn a_folded_window_never_grows_past_its_authored_capacity() {
    let spec = HistorySpec {
        fact: "x".into(),
        ticks: 4,
    };
    let mut memory = AiPolicyMemory::new();
    for n in 0..10_000 {
        let mut facts = AiFacts::new();
        facts.set("x", n as f64);
        memory.fold_history(std::slice::from_ref(&spec), &facts);
        assert!(memory.history().window(&spec).unwrap().len() <= 4);
    }
    assert_eq!(memory.history().len(), 1);

    // Re-authoring to a different set drops what is no longer asked for.
    let other = HistorySpec {
        fact: "y".into(),
        ticks: 2,
    };
    memory.fold_history(std::slice::from_ref(&other), &AiFacts::new());
    assert_eq!(memory.history().len(), 1);
    assert!(memory.history().window(&spec).is_none());
}

/// A history atom outside a bag that anything folds is `false`, never a
/// panic — the same absent-reading contract `fact(...)` carries. The load
/// error that stops content getting here lives in `entities::config` /
/// `entities::ai_flag_hosts`.
#[test]
fn an_unfolded_history_atom_is_false_and_never_panics() {
    let params = params_with(&[("w", 3.0)]);
    let p = parse_predicate("history(min, x, param(w)) >= 0").unwrap();
    assert!(!p.evaluate(&[]));
    assert!(!p.evaluate_with(&AiFacts::new(), &params, &[]));
    assert!(!p.evaluate_stateful(&AiFacts::new(), &AiPolicyMemory::new(), &params, &[]));
    // An undeclared window param resolves to no window at all.
    let bag = folded("x", 3, &[Some(1.0); 3]);
    assert!(!history_says(
        "history(min, x, param(missing)) >= 0",
        &bag,
        &AiParams::new()
    ));
}

#[test]
fn history_atoms_compose_with_the_rest_of_the_grammar() {
    let params = params_with(&[("w", 2.0)]);
    let bag = folded("x", 2, &[Some(9.0), Some(9.0)]);
    assert!(history_says(
        "history(min, x, param(w)) > 5 and not history(max, x, param(w)) > 100",
        &bag,
        &params
    ));
    assert!(history_says(
        "flag(nope) or history(min, x, param(w)) > 5",
        &bag,
        &params
    ));
}

// ── Malformed history expressions are load errors that name the problem ──

#[test]
fn an_unknown_reducer_is_a_parse_error_naming_the_valid_ones() {
    let err = parse_predicate("history(average, x, 4) > 0").unwrap_err();
    assert!(err.contains("average"), "{err}");
    assert!(err.contains("min") && err.contains("net_change"), "{err}");
}

#[test]
fn a_history_window_must_be_a_positive_whole_number_of_ticks() {
    let fractional = parse_predicate("history(min, x, 2.5) > 0").unwrap_err();
    assert!(fractional.contains("WHOLE number"), "{fractional}");
    assert!(fractional.contains("2.5"), "{fractional}");

    for src in ["history(min, x, 0) > 0", "history(min, x, -4) > 0"] {
        let err = parse_predicate(src).unwrap_err();
        assert!(err.contains("positive whole number"), "{src}: {err}");
    }
}

#[test]
fn a_malformed_history_atom_names_what_was_missing() {
    // Missing the window argument entirely.
    let err = parse_predicate("history(min, x) > 0").unwrap_err();
    assert!(err.contains("window length"), "{err}");
    // Missing the separator between reducer and fact.
    let err = parse_predicate("history(min x, 4) > 0").unwrap_err();
    assert!(err.contains("','"), "{err}");
    // No comparison at all: a window reduces to a scalar, it is not itself
    // a boolean.
    let err = parse_predicate("history(min, x, 4)").unwrap_err();
    assert!(err.contains("comparison operator"), "{err}");
    // Nothing to compare against.
    let err = parse_predicate("history(min, x, 4) >").unwrap_err();
    assert!(err.contains("reached end of predicate"), "{err}");
    // A window that is neither a literal nor a param.
    let err = parse_predicate("history(min, x, fact(y)) > 0").unwrap_err();
    assert!(err.contains("window length"), "{err}");
}

#[test]
fn history_remains_usable_as_a_flag_and_fact_name() {
    // `history` was a legal identifier before #890; existing content that
    // used it as a name must keep parsing.
    let s = store_with(&[("history", 1)]);
    assert!(parse_predicate("flag(history)").unwrap().evaluate(&[&s]));
    assert!(parse_predicate("fact(history) > 0").unwrap().evaluate_with(
        &facts_with(&[("history", 1.0)]),
        &AiParams::new(),
        &[]
    ));
}

#[test]
fn history_references_are_reportable_for_content_validation() {
    let p = parse_predicate(
        "history(min, a, param(w)) >= 1 and not history(net_change, b, 7) < param(t)",
    )
    .unwrap();
    let mut refs = Vec::new();
    p.referenced_history(&mut refs);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].render(), "history(min, a, param(w))");
    assert_eq!(refs[1].render(), "history(net_change, b, 7)");
    assert_eq!(
        p.history_atom().unwrap().render(),
        "history(min, a, param(w))"
    );

    // BOTH operands are declaration-checked references.
    let mut params = Vec::new();
    p.referenced_params(&mut params);
    params.sort();
    assert_eq!(params, vec!["t".to_string(), "w".to_string()]);

    // And an expression with no window reports none.
    let plain = parse_predicate("fact(x) > 0").unwrap();
    assert!(plain.history_atom().is_none());
}

// --- serde round-trips (issue #862) -------------------------------------
//
// Round-tripped through RON specifically, because that is the text format
// `vellum-save`'s browser backend stores strings as — a value that only
// round-trips through `serde_json` or in-memory would prove nothing about
// the snapshot path these types actually travel.

#[test]
fn flag_store_round_trips_through_ron() {
    let mut store = FlagStore::new();
    store.set_flag("alpha");
    store.set_flag_value("beta", -7);
    store.increment_flag("gamma", 42);

    let text = ron::to_string(&store).expect("FlagStore should serialise");
    let restored: FlagStore = ron::from_str(&text).expect("FlagStore should parse");

    assert_eq!(restored, store);
    assert!(restored.flag("alpha"));
    assert_eq!(restored.counter("beta"), -7);
    assert_eq!(restored.counter("gamma"), 42);
}

#[test]
fn ai_facts_round_trip_through_ron() {
    let mut facts = AiFacts::new();
    facts.set("secs_since_combat", 12.5);
    facts.set("power_rating", 0.75);

    let text = ron::to_string(&facts).expect("AiFacts should serialise");
    let restored: AiFacts = ron::from_str(&text).expect("AiFacts should parse");

    assert_eq!(restored, facts);
    assert_eq!(restored.get("secs_since_combat"), Some(12.5));
    assert_eq!(restored.get("power_rating"), Some(0.75));
}

#[test]
fn ai_fact_set_round_trips_through_ron() {
    let mut set = AiFactSet::default();
    set.self_facts.set("hull_pct", 0.4);
    set.candidate_facts.set("range", 1200.0);
    set.target_facts.set("shield_pct", 0.9);

    let text = ron::to_string(&set).expect("AiFactSet should serialise");
    let restored: AiFactSet = ron::from_str(&text).expect("AiFactSet should parse");

    assert_eq!(restored, set);
    assert_eq!(restored.self_facts.get("hull_pct"), Some(0.4));
    assert_eq!(restored.candidate_facts.get("range"), Some(1200.0));
    assert_eq!(restored.target_facts.get("shield_pct"), Some(0.9));
}

/// Exercises every field, including the nested [`AiHistory`] /
/// [`BoundedHistory`] windows the memory bag carries (issue #890), since
/// those are the transitive serde dependency that makes `AiPolicyMemory`
/// derivable at all.
#[test]
fn ai_policy_memory_round_trips_through_ron_including_history_windows() {
    let mut memory = AiPolicyMemory::new();
    memory.set("threat_level", 3.0);
    memory.set_state_time_secs(9.5);
    let specs = vec![HistorySpec {
        fact: "range_to_target".to_string(),
        ticks: 4,
    }];
    let mut facts = AiFacts::new();
    for reading in [500.0, 480.0, 460.0, 440.0] {
        facts.set("range_to_target", reading);
        memory.fold_history(&specs, &facts);
    }

    let text = ron::to_string(&memory).expect("AiPolicyMemory should serialise");
    let restored: AiPolicyMemory = ron::from_str(&text).expect("AiPolicyMemory should parse");

    assert_eq!(restored, memory);
    assert_eq!(restored.get("threat_level"), Some(3.0));
    assert_eq!(restored.state_time_secs(), 9.5);
    assert_eq!(
        restored.history().reduce(&specs[0], HistoryReducer::Min),
        Some(440.0)
    );
    assert_eq!(
        restored
            .history()
            .reduce(&specs[0], HistoryReducer::NetChange),
        Some(-60.0)
    );
}
