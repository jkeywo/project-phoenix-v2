//! Pure, Bevy-free per-system target *selector* runtime (issue #776).
//!
//! A [`TargetSelector`] answers a different question from the #775 channel
//! [`crate::ai::policy::AiPolicy`]: not "which typed verb do I emit on channel
//! C?" but "*which entity is my target?*". It is the reusable ranking spine
//! every AI-capable fine system that owns a target (Sensors first, Tactical and
//! Helm actuators later) shares.
//!
//! Given the operating ship (SELF context), a set of candidate contacts unioned
//! from several authored sources, and the currently-retained target, one pure
//! [`TargetSelector::select`] call:
//!
//!   1. unions + deduplicates candidates by entity identity (UUID),
//!   2. filters candidates outside the effective horizon (squared distance),
//!   3. keeps candidates whose authored `eligibility` predicate fires, read over
//!      explicit self / candidate / target fact contexts,
//!   4. sums each candidate's additive `score` from the authored score terms,
//!   5. retains the current target when it stays eligible and within the
//!      authored `switch_margin` of the best score (hysteresis, AC3),
//!   6. breaks final ties on stable entity identity (smallest UUID string).
//!
//! The selector never writes authoritative state: it returns the selected UUID
//! and the host emits the system's existing admitted command. Like `policy.rs`
//! this module owns only the *typed* selector; the TOML schema and content
//! validation live in `entities::config`, and the predicate grammar (including
//! the three fact contexts and authored ship power rating) lives in
//! `world::flags`.

use crate::world::flags::{AiFactSet, AiFacts, AiParams, FlagStore, Predicate};
use std::collections::HashSet;

/// The operating ship's context for one selection: its world position (for the
/// horizon filter) plus its SELF-context facts (faction, authored
/// `power_rating`, and any other self readings the eligibility/score
/// expressions read via `self_fact(...)`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelfContext {
    pub position: [f32; 3],
    pub facts: AiFacts,
}

/// One candidate contact fed to the selector from a registered source.
///
/// `uuid` is the entity identity used for union/dedup and tie-breaking;
/// `position` drives the horizon filter; `facts` are the CANDIDATE-context
/// readings (hostility, detectability, which source(s) surfaced it, objective
/// score, proximity, …) the eligibility/score expressions read via
/// `candidate_fact(...)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectorCandidate {
    pub uuid: String,
    pub position: [f32; 3],
    pub facts: AiFacts,
}

impl SelectorCandidate {
    /// Merge another source's facts for the same entity into this candidate.
    ///
    /// When the same UUID is surfaced by more than one source (e.g. Tactical's
    /// combat lock is also the nearest radar hostile), dedup keeps the first
    /// occurrence but folds later sources' facts in so a `candidate_fact(...)`
    /// marker set by any source is visible to the expressions.
    fn merge_facts(&mut self, other: &AiFacts) {
        for (k, v) in other.iter() {
            // Later sources never clobber an existing reading; a source marker
            // is monotonic (a fact present in either source stays present).
            if self.facts.get(k).is_none() {
                self.facts.set(k, v);
            }
        }
    }
}

/// One additive utility term: contributes `weight` to a candidate's score when
/// its `when` guard fires over the three fact contexts.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreTerm {
    /// Guard predicate; the term contributes only when it evaluates `true`.
    pub when: Predicate,
    /// Weight added to the candidate's score when the guard fires.
    pub weight: f64,
}

/// A resolved, typed per-system target selector (already parsed + validated).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetSelector {
    /// Authored named parameters referenced by the eligibility/score guards.
    pub params: AiParams,
    /// Registered candidate-source ids this selector unions (informational at
    /// runtime; validated against the system's known sources at content-load).
    pub sources: Vec<String>,
    /// Effective horizon: candidates farther than this (planar distance) are
    /// dropped before scoring. Hosts that own a live, damage-scaled horizon
    /// pre-filter candidates too; this is the selector's own authored bound.
    pub horizon: f32,
    /// Hysteresis margin: the current target is retained while its score is
    /// within `switch_margin` of the best candidate's score.
    pub switch_margin: f32,
    /// Candidate eligibility predicate over self/candidate/target contexts.
    pub eligibility: Predicate,
    /// Additive utility terms summed per eligible candidate.
    pub score: Vec<ScoreTerm>,
}

impl Default for TargetSelector {
    /// An inert selector that selects nothing (`eligibility = false`). Only ever
    /// reached by an `unwrap_or_default()` fallback whose `to_selector()` was
    /// already validated at content-load, so this never gates real gameplay.
    fn default() -> Self {
        Self {
            params: AiParams::new(),
            sources: Vec::new(),
            horizon: 0.0,
            switch_margin: 0.0,
            eligibility: Predicate::Bool(false),
            score: Vec::new(),
        }
    }
}

impl TargetSelector {
    /// Select this system's target from the unioned candidate sources.
    ///
    /// Returns the chosen candidate's UUID, or `None` when no candidate is
    /// eligible this tick — in which case the host drops any current selection.
    /// Pure: the same inputs always yield the same output, with deterministic
    /// tie-breaking, so it is safe on the fixed sim tick and in P2P lockstep.
    pub fn select(
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

/// Planar (x/z) squared distance between two world positions.
fn planar_dist_sq(a: [f32; 3], b: [f32; 3]) -> f64 {
    let dx = (a[0] - b[0]) as f64;
    let dz = (a[2] - b[2]) as f64;
    dx * dx + dz * dz
}

#[cfg(test)]
mod tests {
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
}
