use super::*;
use crate::world::flags::parse_predicate;

fn facts(pairs: &[(&str, f64)]) -> AiFacts {
    let mut f = AiFacts::new();
    for (k, v) in pairs {
        f.set(k, *v);
    }
    f
}

fn cand(uuid: &str, x: f32, z: f32, pairs: &[(&str, f64)]) -> SelectorCandidate {
    SelectorCandidate {
        uuid: uuid.into(),
        position: [x, 0.0, z],
        facts: facts(pairs),
    }
}

/// A selector that ranks hostile, detectable candidates by an additive
/// source-priority utility (combat-lock ≫ objective ≫ radar), reproducing
/// the Sensors tier order with a large horizon and no hysteresis.
fn priority_selector() -> TargetSelector {
    let mut params = AiParams::new();
    params.set("combat_lock_weight", 1000.0);
    params.set("objective_weight", 100.0);
    params.set("radar_weight", 1.0);
    TargetSelector {
        params,
        sources: vec![
            "combat-lock".into(),
            "objective-destroy".into(),
            "radar-contacts".into(),
        ],
        horizon: 1000.0,
        switch_margin: 0.0,
        eligibility: parse_predicate(
            "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0",
        )
        .unwrap(),
        score: vec![
            ScoreTerm {
                when: parse_predicate("candidate_fact(source_combat_lock) > 0").unwrap(),
                weight: 1000.0,
            },
            ScoreTerm {
                when: parse_predicate("candidate_fact(source_objective) > 0").unwrap(),
                weight: 100.0,
            },
            ScoreTerm {
                when: parse_predicate("candidate_fact(source_radar) > 0").unwrap(),
                weight: 1.0,
            },
        ],
    }
}

fn detectable_hostile<'a>(extra: &[(&'a str, f64)]) -> Vec<(&'a str, f64)> {
    let mut v = vec![("detectable", 1.0), ("hostile", 1.0)];
    v.extend_from_slice(extra);
    v
}

// ── AC1: union + dedup by identity ──────────────────────────────────────

#[test]
fn unions_and_deduplicates_candidates_by_identity() {
    let sel = priority_selector();
    // The same UUID appears from two sources; dedup keeps one entry and
    // folds facts so BOTH source markers count toward its score.
    let candidates = vec![
        cand(
            "enemy",
            10.0,
            0.0,
            &detectable_hostile(&[("source_radar", 1.0)]),
        ),
        cand(
            "enemy",
            10.0,
            0.0,
            &detectable_hostile(&[("source_combat_lock", 1.0)]),
        ),
    ];
    let picked = sel.select(&SelfContext::default(), &candidates, None, &[]);
    assert_eq!(picked.as_deref(), Some("enemy"));
}

// ── AC2: contexts + power rating drive eligibility/score ────────────────

#[test]
fn eligibility_reads_self_power_rating_and_candidate_context() {
    // Only engage when the ship out-rates the candidate's authored threat.
    let mut params = AiParams::new();
    params.set("min_rating", 5.0);
    let sel = TargetSelector {
        params,
        sources: vec!["radar-contacts".into()],
        horizon: 1000.0,
        switch_margin: 0.0,
        eligibility: parse_predicate(
            "self_fact(power_rating) >= param(min_rating) and candidate_fact(hostile) > 0",
        )
        .unwrap(),
        score: vec![ScoreTerm {
            when: parse_predicate("true").unwrap(),
            weight: 1.0,
        }],
    };
    let candidates = vec![cand("enemy", 10.0, 0.0, &[("hostile", 1.0)])];

    // Under-rated ship → not eligible → no selection.
    let weak = SelfContext {
        position: [0.0, 0.0, 0.0],
        facts: facts(&[("power_rating", 3.0)]),
    };
    assert_eq!(sel.select(&weak, &candidates, None, &[]), None);

    // Sufficiently-rated ship → eligible.
    let strong = SelfContext {
        position: [0.0, 0.0, 0.0],
        facts: facts(&[("power_rating", 6.0)]),
    };
    assert_eq!(
        sel.select(&strong, &candidates, None, &[]).as_deref(),
        Some("enemy")
    );
}

#[test]
fn additive_score_prefers_higher_priority_source() {
    let sel = priority_selector();
    let candidates = vec![
        cand(
            "locked",
            50.0,
            0.0,
            &detectable_hostile(&[("source_combat_lock", 1.0)]),
        ),
        cand(
            "objective",
            20.0,
            0.0,
            &detectable_hostile(&[("source_objective", 1.0)]),
        ),
        cand(
            "radar",
            10.0,
            0.0,
            &detectable_hostile(&[("source_radar", 1.0)]),
        ),
    ];
    // Combat-lock (1000) beats objective (100) beats radar (1).
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, None, &[])
            .as_deref(),
        Some("locked")
    );
}

// ── AC3: hysteresis + deterministic ties ────────────────────────────────

#[test]
fn retains_current_target_within_switch_margin() {
    let mut sel = priority_selector();
    sel.switch_margin = 50.0;
    // Two objective-tier candidates: current scores 100, a rival also 100
    // plus a tiny radar bonus (101). Within the 50 margin → keep current.
    let candidates = vec![
        cand(
            "current",
            10.0,
            0.0,
            &detectable_hostile(&[("source_objective", 1.0)]),
        ),
        cand(
            "rival",
            12.0,
            0.0,
            &detectable_hostile(&[("source_objective", 1.0), ("source_radar", 1.0)]),
        ),
    ];
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, Some("current"), &[])
            .as_deref(),
        Some("current"),
        "a rival within the switch margin must not steal the lock"
    );
}

#[test]
fn switches_when_rival_exceeds_switch_margin() {
    let mut sel = priority_selector();
    sel.switch_margin = 50.0;
    // Current is a radar contact (1); a combat lock (1000) blows past the
    // 50 margin → switch.
    let candidates = vec![
        cand(
            "current",
            10.0,
            0.0,
            &detectable_hostile(&[("source_radar", 1.0)]),
        ),
        cand(
            "locked",
            12.0,
            0.0,
            &detectable_hostile(&[("source_combat_lock", 1.0)]),
        ),
    ];
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, Some("current"), &[])
            .as_deref(),
        Some("locked")
    );
}

#[test]
fn final_tie_breaks_to_smallest_uuid() {
    let sel = priority_selector();
    // Two identically-scored radar hostiles, no current target: the smaller
    // UUID string wins, regardless of input order.
    let candidates = vec![
        cand(
            "bbb",
            10.0,
            0.0,
            &detectable_hostile(&[("source_radar", 1.0)]),
        ),
        cand(
            "aaa",
            11.0,
            0.0,
            &detectable_hostile(&[("source_radar", 1.0)]),
        ),
    ];
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, None, &[])
            .as_deref(),
        Some("aaa")
    );
    // Reversed input order yields the same deterministic winner.
    let reversed: Vec<_> = candidates.into_iter().rev().collect();
    assert_eq!(
        sel.select(&SelfContext::default(), &reversed, None, &[])
            .as_deref(),
        Some("aaa")
    );
}

// ── AC4: invalid / friendly / hidden / out-of-horizon dropped ───────────

#[test]
fn drops_friendly_and_hidden_candidates() {
    let sel = priority_selector();
    let candidates = vec![
        // Friendly (not hostile) — ineligible.
        cand(
            "ally",
            5.0,
            0.0,
            &[("detectable", 1.0), ("hostile", 0.0), ("source_radar", 1.0)],
        ),
        // Hidden (not detectable) — ineligible.
        cand(
            "cloaked",
            6.0,
            0.0,
            &[("detectable", 0.0), ("hostile", 1.0), ("source_radar", 1.0)],
        ),
        // Valid hostile.
        cand(
            "enemy",
            30.0,
            0.0,
            &detectable_hostile(&[("source_radar", 1.0)]),
        ),
    ];
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, None, &[])
            .as_deref(),
        Some("enemy")
    );
}

#[test]
fn drops_out_of_horizon_candidate() {
    let mut sel = priority_selector();
    sel.horizon = 100.0;
    let candidates = vec![cand(
        "far",
        500.0,
        0.0,
        &detectable_hostile(&[("source_radar", 1.0)]),
    )];
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, None, &[]),
        None
    );
}

#[test]
fn invalid_current_target_is_replaced_same_call() {
    let sel = priority_selector();
    // The current target is no longer among the candidates (destroyed /
    // despawned); a fresh eligible hostile replaces it in the same call.
    let candidates = vec![cand(
        "fresh",
        20.0,
        0.0,
        &detectable_hostile(&[("source_radar", 1.0)]),
    )];
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, Some("gone"), &[])
            .as_deref(),
        Some("fresh")
    );
}

#[test]
fn current_target_that_becomes_ineligible_is_dropped() {
    let sel = priority_selector();
    // The only candidate is the current target, now friendly → ineligible
    // → nothing eligible → dropped (None) in the same call.
    let candidates = vec![cand(
        "current",
        10.0,
        0.0,
        &[("detectable", 1.0), ("hostile", 0.0), ("source_radar", 1.0)],
    )];
    assert_eq!(
        sel.select(&SelfContext::default(), &candidates, Some("current"), &[]),
        None
    );
}

#[test]
fn no_candidates_selects_none() {
    let sel = priority_selector();
    assert_eq!(
        sel.select(&SelfContext::default(), &[], Some("prev"), &[]),
        None
    );
}
