// Frozen selector entry and recursive evaluator from 30ad2c1d.
use super::*;

impl Predicate {
    fn owned_evaluate_selector(
        &self,
        facts: &AiFactSet,
        params: &AiParams,
        chain: &[&FlagStore],
    ) -> bool {
        self.owned_evaluate_ctx(
            &facts.self_facts,
            &facts.candidate_facts,
            &facts.target_facts,
            &AiPolicyMemory::default(),
            params,
            chain,
        )
    }

    fn owned_evaluate_ctx(
        &self,
        self_facts: &AiFacts,
        candidate_facts: &AiFacts,
        target_facts: &AiFacts,
        memory: &AiPolicyMemory,
        params: &AiParams,
        chain: &[&FlagStore],
    ) -> bool {
        match self {
            Predicate::Flag { name } => flag_in_chain(chain, name),
            Predicate::Counter { name, op, rhs } => op.apply(counter_in_chain(chain, name), *rhs),
            Predicate::Fact {
                context,
                name,
                op,
                rhs,
            } => {
                // The private contexts (issue #882) read the owning system's
                // own memory bag; the three world contexts read their fact set.
                let lhs = match context {
                    FactContext::SelfCtx => self_facts.get(name),
                    FactContext::Candidate => candidate_facts.get(name),
                    FactContext::Target => target_facts.get(name),
                    FactContext::Memory => memory.get(name),
                    FactContext::StateTime => Some(memory.state_time_secs()),
                };
                match (lhs, rhs.resolve(params)) {
                    (Some(lhs), Some(rhs)) => op.apply(lhs, rhs),
                    // Absent fact or unresolved parameter → false, never panic.
                    _ => false,
                }
            }
            Predicate::History {
                reducer,
                window,
                op,
                rhs,
            } => {
                // An unresolvable window (unknown param, or a length content
                // validation would have rejected) reads as no window at all,
                // which makes the comparison false — the same absent-reading
                // contract every other atom keeps.
                let Some(spec) = window.resolve(params) else {
                    return false;
                };
                match (
                    memory.history().reduce(&spec, *reducer),
                    rhs.resolve(params),
                ) {
                    (Some(lhs), Some(rhs)) => op.apply(lhs, rhs),
                    _ => false,
                }
            }
            Predicate::Bool(b) => *b,
            Predicate::Not(inner) => !inner.owned_evaluate_ctx(
                self_facts,
                candidate_facts,
                target_facts,
                memory,
                params,
                chain,
            ),
            Predicate::And(a, b) => {
                a.owned_evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                ) && b.owned_evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                )
            }
            Predicate::Or(a, b) => {
                a.owned_evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                ) || b.owned_evaluate_ctx(
                    self_facts,
                    candidate_facts,
                    target_facts,
                    memory,
                    params,
                    chain,
                )
            }
        }
    }
}

fn reading(value: f64) -> AiFacts {
    let mut facts = AiFacts::new();
    facts.set("shared", value);
    facts
}

#[test]
fn borrowed_selector_contexts_match_frozen_owned_evaluation_without_mutating_inputs() {
    let expressions = [
        "true",
        "false",
        "fact(shared) == 3",
        "self_fact(shared) == 3",
        "candidate_fact(shared) == 7",
        "target_fact(shared) == 11",
        "candidate_fact(shared) >= param(limit)",
        "target_fact(shared) != param(missing)",
        "self_fact(absent) != 0",
        "not candidate_fact(absent) > 0",
        "flag(ready)",
        "flag(parent:ready)",
        "flag(parent:parent:ready)",
        "counter(count) >= 2",
        "counter(parent:count) < 0",
        "counter(parent:parent:count) == 0",
        "memory(shared) != 0",
        "state_time == 0",
        "state_time > param(limit)",
        "history(min, shared, 4) >= 0",
        "history(max, shared, param(missing)) > 0",
        "self_fact(shared) > 0 and (candidate_fact(shared) > 4 or target_fact(shared) > 8)",
        "flag(ready) or (flag(parent:ready) and not counter(count) > 0)",
        "not (target_fact(absent) > 0 or memory(shared) > 0) and state_time == 0",
    ];
    let expressions: Vec<_> = expressions
        .into_iter()
        .map(|expression| (expression, parse_predicate(expression).unwrap()))
        .collect();
    let mut inner = FlagStore::new();
    inner.set_flag_value("count", 2);
    let mut parent = FlagStore::new();
    parent.set_flag("ready");
    parent.set_flag_value("count", -3);
    let mut outer = FlagStore::new();
    outer.set_flag("ready");
    let stores = [inner, parent, outer];
    for mask in 0..8 {
        let facts = AiFactSet {
            self_facts: if mask & 1 != 0 {
                reading(3.0)
            } else {
                AiFacts::new()
            },
            candidate_facts: if mask & 2 != 0 {
                reading(7.0)
            } else {
                AiFacts::new()
            },
            target_facts: if mask & 4 != 0 {
                reading(11.0)
            } else {
                AiFacts::new()
            },
        };
        for with_param in [false, true] {
            let mut params = AiParams::new();
            if with_param {
                params.set("limit", 5.0);
            }
            for chain_len in 0..=stores.len() {
                let chain: Vec<_> = stores[..chain_len].iter().collect();
                let before = (facts.clone(), params.clone(), stores.clone());
                for (expression, predicate) in &expressions {
                    let expected = predicate.owned_evaluate_selector(&facts, &params, &chain);
                    assert_eq!(
                        predicate.evaluate_selector(&facts, &params, &chain),
                        expected,
                        "owned {expression}"
                    );
                    assert_eq!(
                        predicate.evaluate_selector_borrowed(
                            &facts.self_facts,
                            &facts.candidate_facts,
                            &facts.target_facts,
                            &params,
                            &chain
                        ),
                        expected,
                        "borrowed {expression}, mask {mask}"
                    );
                }
                assert_eq!((facts.clone(), params.clone(), stores.clone()), before);
            }
        }
    }
    let facts = AiFactSet {
        self_facts: reading(3.0),
        candidate_facts: reading(7.0),
        target_facts: reading(11.0),
    };
    for expression in [
        "self_fact(shared) == 3",
        "candidate_fact(shared) == 7",
        "target_fact(shared) == 11",
    ] {
        assert!(parse_predicate(expression)
            .unwrap()
            .evaluate_selector_borrowed(
                &facts.self_facts,
                &facts.candidate_facts,
                &facts.target_facts,
                &AiParams::new(),
                &[]
            ));
    }
}

#[test]
fn borrowed_selector_preserves_nonfinite_comparisons() {
    for value in [-0.0, 0.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let facts = AiFactSet {
            self_facts: reading(value),
            candidate_facts: reading(value),
            target_facts: reading(value),
        };
        for expression in [
            "fact(shared) == 0",
            "candidate_fact(shared) != 0",
            "target_fact(shared) < 0",
            "self_fact(shared) >= 0",
            "not target_fact(shared) > 0",
        ] {
            let predicate = parse_predicate(expression).unwrap();
            assert_eq!(
                predicate.evaluate_selector_borrowed(
                    &facts.self_facts,
                    &facts.candidate_facts,
                    &facts.target_facts,
                    &AiParams::new(),
                    &[]
                ),
                predicate.owned_evaluate_selector(&facts, &AiParams::new(), &[])
            );
        }
    }
}
