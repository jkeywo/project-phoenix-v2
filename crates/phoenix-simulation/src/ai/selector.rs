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
#[path = "selector_tests.rs"]
mod tests;
