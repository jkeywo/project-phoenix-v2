// Frozen complete owned-facts selection oracle from 30ad2c1d.
use super::*;
use crate::world::flags::AiFactSet;

impl TargetSelector {
    /// Select this system's target from the unioned candidate sources.
    ///
    /// Returns the chosen candidate's UUID, or `None` when no candidate is
    /// eligible this tick — in which case the host drops any current selection.
    /// Pure: the same inputs always yield the same output, with deterministic
    /// tie-breaking, so it is safe on the fixed sim tick and in P2P lockstep.
    fn select_owned_oracle(
        &self,
        self_ctx: &SelfContext,
        candidates: &[SelectorCandidate],
        current: Option<&str>,
        flags: &[&FlagStore],
    ) -> Option<String> {
        // 1. Union + dedup by entity identity, keeping the first occurrence and
        //    folding later sources' facts in (mirrors validate_phaser_banks's
        //    HashSet dedup by id).
        let mut seen: HashSet<&str> = HashSet::new();
        let mut unique: Vec<SelectorCandidate> = Vec::with_capacity(candidates.len());
        for cand in candidates {
            if seen.insert(cand.uuid.as_str()) {
                unique.push(cand.clone());
            } else if let Some(existing) = unique.iter_mut().find(|c| c.uuid == cand.uuid) {
                existing.merge_facts(&cand.facts);
            }
        }

        // 2. Horizon filter (planar squared distance, matching the sensors
        //    in_range_pos convention: x/z only).
        let horizon_sq = (self.horizon as f64) * (self.horizon as f64);
        unique.retain(|c| planar_dist_sq(self_ctx.position, c.position) <= horizon_sq);

        // The currently-retained target's CANDIDATE facts become the shared
        // TARGET context for every candidate's eligibility/score evaluation.
        let target_facts: AiFacts = current
            .and_then(|cur| unique.iter().find(|c| c.uuid == cur))
            .map(|c| c.facts.clone())
            .unwrap_or_default();

        // 3 + 4. Keep eligible candidates and score each additively.
        let mut scored: Vec<(&SelectorCandidate, f64)> = Vec::new();
        for cand in &unique {
            let facts = AiFactSet {
                self_facts: self_ctx.facts.clone(),
                candidate_facts: cand.facts.clone(),
                target_facts: target_facts.clone(),
            };
            if !self
                .eligibility
                .evaluate_selector(&facts, &self.params, flags)
            {
                continue;
            }
            let mut score = 0.0;
            for term in &self.score {
                if term.when.evaluate_selector(&facts, &self.params, flags) {
                    score += term.weight;
                }
            }
            scored.push((cand, score));
        }

        if scored.is_empty() {
            return None;
        }

        // Best candidate: highest score, ties broken by smallest UUID string
        // (AC3 — deterministic, query-order-independent).
        let best = scored
            .iter()
            .max_by(|(ca, sa), (cb, sb)| {
                sa.partial_cmp(sb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    // Reverse UUID so the *smallest* UUID is the maximum on a score tie.
                    .then_with(|| cb.uuid.cmp(&ca.uuid))
            })
            .map(|(c, s)| (c.uuid.clone(), *s))
            .expect("scored is non-empty");

        // 5. Hysteresis: retain the current target while it is still eligible
        //    and within the authored switch margin of the best score.
        if let Some(cur) = current {
            if let Some((_, cur_score)) = scored.iter().find(|(c, _)| c.uuid == cur) {
                if *cur_score >= best.1 - self.switch_margin as f64 {
                    return Some(cur.to_string());
                }
            }
        }

        Some(best.0)
    }
}

fn check_owned_oracle(
    selector: &TargetSelector,
    self_ctx: &SelfContext,
    candidates: &[SelectorCandidate],
    current: Option<&str>,
    chain: &[&FlagStore],
) -> Option<String> {
    let before = (selector.clone(), self_ctx.clone(), candidates.to_vec());
    let expected = selector.select_owned_oracle(self_ctx, candidates, current, chain);
    let actual = selector.select(self_ctx, candidates, current, chain);
    assert_eq!(
        actual, expected,
        "current={current:?}, horizon={}, margin={}",
        selector.horizon, selector.switch_margin
    );
    assert_eq!(
        (selector.clone(), self_ctx.clone(), candidates.to_vec()),
        before
    );
    actual
}

#[test]
fn borrowed_selection_matches_complete_owned_oracle_for_order_merge_horizon_and_hysteresis() {
    let orders = [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]];
    let rows = [
        cand(
            "alpha",
            10.0,
            0.0,
            &[("hostile", 1.0), ("detectable", 1.0), ("source_radar", 1.0)],
        ),
        cand(
            "alpha",
            900.0,
            0.0,
            &[("hostile", 0.0), ("source_combat_lock", 1.0)],
        ),
        cand(
            "beta",
            20.0,
            0.0,
            &[
                ("hostile", 1.0),
                ("detectable", 1.0),
                ("source_objective", 1.0),
            ],
        ),
        cand(
            "gamma",
            0.0,
            1000.0,
            &[
                ("hostile", 1.0),
                ("detectable", 0.0),
                ("source_combat_lock", 1.0),
            ],
        ),
    ];
    let mut inner = FlagStore::new();
    inner.set_flag("enabled");
    inner.set_flag_value("count", 3);
    let mut parent = FlagStore::new();
    parent.set_flag("enabled");
    parent.set_flag_value("count", -2);
    let chains: [Vec<&FlagStore>; 4] = [
        vec![],
        vec![&inner],
        vec![&parent, &inner],
        vec![&inner, &parent],
    ];
    let selectors = [
        priority_selector(),
        TargetSelector {
            eligibility: parse_predicate(
                "candidate_fact(hostile) > 0 and (flag(enabled) or flag(parent:enabled))",
            )
            .unwrap(),
            ..priority_selector()
        },
        TargetSelector {
            eligibility: parse_predicate(
                "self_fact(power_rating) >= param(minimum) and candidate_fact(detectable) > 0",
            )
            .unwrap(),
            ..priority_selector()
        },
        TargetSelector {
            eligibility: parse_predicate("candidate_fact(absent) != 0").unwrap(),
            ..priority_selector()
        },
    ];
    for mut selector in selectors {
        selector.params.set("minimum", 5.0);
        // TARGET facts are taken after merge/horizon but BEFORE eligibility.
        selector.score.push(ScoreTerm {
            when: parse_predicate(
                "target_fact(source_combat_lock) > 0 and candidate_fact(source_objective) > 0",
            )
            .unwrap(),
            weight: 1500.0,
        });
        selector.score.push(ScoreTerm {
            when: parse_predicate("counter(parent:count) < 0").unwrap(),
            weight: -0.25,
        });
        for order in orders {
            let candidates: Vec<_> = order.into_iter().map(|index| rows[index].clone()).collect();
            for horizon in [0.0, 10.0, 20.0, -20.0, 1000.0] {
                selector.horizon = horizon;
                for margin in [0.0, 100.0, 2000.0] {
                    selector.switch_margin = margin;
                    for current in [
                        None,
                        Some("alpha"),
                        Some("beta"),
                        Some("gamma"),
                        Some("removed"),
                    ] {
                        for self_rating in [None, Some(4.0), Some(8.0)] {
                            let mut self_ctx = SelfContext::default();
                            if let Some(rating) = self_rating {
                                self_ctx.facts.set("power_rating", rating);
                            }
                            for chain in &chains {
                                check_owned_oracle(
                                    &selector,
                                    &self_ctx,
                                    &candidates,
                                    current,
                                    chain,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn borrowed_target_context_keeps_first_duplicate_pose_and_fact_then_merges_missing_facts() {
    let mut selector = priority_selector();
    selector.horizon = 50.0;
    let first = cand(
        "alpha",
        10.0,
        0.0,
        &[("hostile", 1.0), ("detectable", 1.0), ("source_radar", 1.0)],
    );
    let duplicate = cand(
        "alpha",
        900.0,
        0.0,
        &[("hostile", 0.0), ("source_combat_lock", 1.0)],
    );
    let other = cand(
        "beta",
        20.0,
        0.0,
        &[
            ("hostile", 1.0),
            ("detectable", 1.0),
            ("source_objective", 1.0),
        ],
    );
    assert_eq!(
        check_owned_oracle(
            &selector,
            &SelfContext::default(),
            &[first.clone(), duplicate.clone(), other.clone()],
            None,
            &[]
        )
        .as_deref(),
        Some("alpha")
    );
    assert_eq!(
        check_owned_oracle(
            &selector,
            &SelfContext::default(),
            &[duplicate, first.clone(), other.clone()],
            None,
            &[]
        )
        .as_deref(),
        Some("beta")
    );
    selector.eligibility = parse_predicate("candidate_fact(hostile) > 0").unwrap();
    selector.score = vec![ScoreTerm {
        when: parse_predicate("target_fact(signal) > 0 and candidate_fact(source_objective) > 0")
            .unwrap(),
        weight: 5.0,
    }];
    let rejected = cand("current", 30.0, 0.0, &[("hostile", 0.0), ("signal", 1.0)]);
    assert_eq!(
        check_owned_oracle(
            &selector,
            &SelfContext::default(),
            &[rejected, first, other],
            Some("current"),
            &[]
        )
        .as_deref(),
        Some("beta"),
        "an ineligible current target still contributes its pre-eligibility TARGET facts"
    );
}

#[test]
fn borrowed_selection_retains_term_order_ties_and_current_identity_over_successive_calls() {
    let mut selector = priority_selector();
    selector.eligibility = parse_predicate("candidate_fact(hostile) > 0").unwrap();
    let mut current = None;
    for step in 0..16 {
        selector.switch_margin = if step % 2 == 0 { 0.5 } else { 2.0 };
        let candidates = vec![
            cand(
                "smallest",
                10.0,
                0.0,
                &[
                    ("hostile", 1.0),
                    ("strong", if step % 3 == 0 { 1.0 } else { 0.0 }),
                ],
            ),
            cand(
                "rival",
                20.0,
                0.0,
                &[
                    ("hostile", 1.0),
                    ("strong", if step % 3 == 1 { 1.0 } else { 0.0 }),
                ],
            ),
        ];
        selector.score = vec![
            ScoreTerm {
                when: Predicate::Bool(true),
                weight: 1e16,
            },
            ScoreTerm {
                when: Predicate::Bool(true),
                weight: -1e16,
            },
            ScoreTerm {
                when: parse_predicate("candidate_fact(strong) > 0").unwrap(),
                weight: 1.0,
            },
        ];
        current = check_owned_oracle(
            &selector,
            &SelfContext::default(),
            &candidates,
            current.as_deref(),
            &[],
        );
        assert!(current.is_some());
    }
    let candidates = [
        cand("z", 10.0, 0.0, &[("hostile", 1.0)]),
        cand("a", 10.0, 0.0, &[("hostile", 1.0)]),
    ];
    selector.score.clear();
    assert_eq!(
        check_owned_oracle(&selector, &SelfContext::default(), &candidates, None, &[]).as_deref(),
        Some("a")
    );
    assert_eq!(
        check_owned_oracle(&selector, &SelfContext::default(), &[], Some("a"), &[]),
        None
    );
}

#[test]
fn borrowed_selection_preserves_nonfinite_score_and_absent_target_behavior() {
    // Existing runtime accepts these values; this change must not add validation
    // or normalize them while removing fact copies.
    for weight in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, 0.0] {
        let selector = TargetSelector {
            horizon: 10.0,
            eligibility: Predicate::Bool(true),
            score: vec![ScoreTerm {
                when: Predicate::Bool(true),
                weight,
            }],
            ..Default::default()
        };
        let candidates = [cand("b", 0.0, 0.0, &[]), cand("a", 0.0, 0.0, &[])];
        for current in [None, Some("a"), Some("missing")] {
            // Do not compare NaN-containing owners with PartialEq as a mutation
            // witness: retain their exact bits and input facts independently.
            let before: Vec<_> = selector
                .score
                .iter()
                .map(|term| term.weight.to_bits())
                .collect();
            assert_eq!(
                selector.select(&SelfContext::default(), &candidates, current, &[]),
                selector.select_owned_oracle(&SelfContext::default(), &candidates, current, &[])
            );
            assert_eq!(
                selector
                    .score
                    .iter()
                    .map(|term| term.weight.to_bits())
                    .collect::<Vec<_>>(),
                before
            );
        }
    }
}

#[test]
fn borrowed_selector_preserves_owned_fact_set_wire_and_save_shape() {
    let owned = AiFactSet {
        self_facts: facts(&[("shared", 3.0)]),
        candidate_facts: facts(&[("shared", 7.0)]),
        target_facts: facts(&[("shared", 11.0)]),
    };
    let value = serde_json::to_value(&owned).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "self_facts":{"values":{"shared":3.0}},
            "candidate_facts":{"values":{"shared":7.0}},
            "target_facts":{"values":{"shared":11.0}},
        })
    );
    assert_eq!(serde_json::from_value::<AiFactSet>(value).unwrap(), owned);
    let saved = ron::to_string(&owned).unwrap();
    assert_eq!(ron::from_str::<AiFactSet>(&saved).unwrap(), owned);
}
