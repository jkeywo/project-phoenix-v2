//! Read-only decision-trace projection for the `ai` log category (issue #1146).
//!
//! The `ai` log category (`crate::logging::LogCat::Ai`) shipped built and tested
//! but with essentially no emitters, so `--log ai=debug` printed nothing. This
//! module supplies the *pure* half of the fix: Bevy-free functions that turn the
//! authoritative scored-objective pool into the exact field VALUES the doctrine,
//! target and console-AI emitters log. The systems themselves (`ai::server`,
//! `console::weapons`, `console::captain`) hold the thin emit sites that gate on
//! [`LogFilterConfig`](crate::logging::LogFilterConfig) and call the `plog!`
//! macros with these values.
//!
//! # Why the logic lives here rather than inline at the emit site
//!
//! `cargo test` installs no `tracing` subscriber, so the emitted log line's text
//! and fields cannot be captured in a unit test (see the note in
//! `crate::logging::macros`). The codebase's answer — used by
//! `command_admission::router`'s unrouted-lint tests — is to test the *decision*
//! that drives the event through a pure function, not the tracing output. So the
//! structured directive-change event is built here as a [`DirectiveChange`]
//! value by [`directive_change`], and the doctrine emitter merely logs that
//! value's fields. A test asserts the value; the emitter is a one-liner over it.
//!
//! # Determinism
//!
//! Every function here is a read-only projection of already-authoritative state
//! (the [`ScoredObjective`] pool the doctrine aggregator computes each tick). It
//! never touches `SimRng`, never mutates the world, and is only ever *called*
//! from inside a log-level gate at the emit site — so with `ai` logging off the
//! doctrine hot path does not even format a label, and the seeded digest is
//! byte-identical whether `ai=debug` is on or off. `tests/ai_decision_log.rs`
//! proves that directly.

use crate::core::messages::{AiDirective, ScoredObjective};

/// The upper bound on candidates named in a [`format_pool`] scoring trace.
///
/// A doctrine pool is small (a handful of authored objectives), but a mission
/// pool merged onto a player ship can run longer; the trace names the top few by
/// score and counts the rest, so one `debug` line stays one line.
const POOL_TRACE_LIMIT: usize = 6;

/// A compact, stable label for a directive kind and the entity/anchor it names.
///
/// This is the `prev`/`new` field value carried by a directive-change event and
/// the key that change detection compares on: two directives with the same kind
/// but a different target produce different labels, so a Destroy retargeting is
/// a directive change, not a silent no-op.
pub fn directive_label(directive: &AiDirective) -> String {
    match directive {
        AiDirective::None => "none".to_string(),
        AiDirective::Destroy { target } => format!("Destroy({target})"),
        AiDirective::Patrol { anchors, loop_path } => {
            let route = anchors.join(">");
            if *loop_path {
                format!("Patrol({route} loop)")
            } else {
                format!("Patrol({route})")
            }
        }
        AiDirective::Reach { anchor } => format!("Reach({anchor})"),
        AiDirective::Hail { target } => format!("Hail({target})"),
        AiDirective::Order { target, route } => format!("Order({target} -> {route})"),
        AiDirective::Scan { target } => format!("Scan({target})"),
        AiDirective::Retreat { anchor } => format!("Retreat({anchor})"),
        AiDirective::Dock { target } => format!("Dock({target})"),
        AiDirective::Tow { target } => format!("Tow({target})"),
        AiDirective::Stabilise { target } => format!("Stabilise({target})"),
        AiDirective::Escort { target } => format!("Escort({target})"),
        AiDirective::Transfer { target } => format!("Transfer({target})"),
        AiDirective::FieldRepair { target } => format!("FieldRepair({target})"),
        AiDirective::Secure { target } => format!("Secure({target})"),
        AiDirective::Rescue { target } => format!("Rescue({target})"),
    }
}

/// The target/anchor name a directive names, for the event's `target` field.
///
/// `None` for the target-less directives (`Patrol` with no anchors, and the
/// human-facing `None`). `Patrol` reports its first anchor — the waypoint the
/// ship is heading for now — which is what a reader wants next to a Patrol
/// timeline entry.
pub fn directive_target(directive: &AiDirective) -> Option<&str> {
    match directive {
        AiDirective::Destroy { target }
        | AiDirective::Hail { target }
        | AiDirective::Scan { target }
        | AiDirective::Dock { target }
        | AiDirective::Tow { target }
        | AiDirective::Stabilise { target }
        | AiDirective::Escort { target }
        | AiDirective::Transfer { target }
        | AiDirective::FieldRepair { target }
        | AiDirective::Secure { target }
        | AiDirective::Rescue { target } => Some(target.as_str()),
        AiDirective::Order { target, .. } => Some(target.as_str()),
        AiDirective::Reach { anchor } | AiDirective::Retreat { anchor } => Some(anchor.as_str()),
        AiDirective::Patrol { anchors, .. } => anchors.first().map(String::as_str),
        AiDirective::None => None,
    }
}

/// The ship's current top directive: the highest positively-scored objective
/// that actually carries an AI directive.
///
/// Mirrors the filter `ai::server::active_destroy_target` /
/// `active_waypoint_route` already apply — `score > 0.0`, and a real directive
/// (never [`AiDirective::None`], which is human-facing only) — so the "current
/// directive" a trace names is the same one the helm and weapons act on. `None`
/// when the pool is empty or everything gated out to zero.
pub fn top_directive(scored: &[ScoredObjective]) -> Option<&ScoredObjective> {
    scored
        .iter()
        .filter(|o| o.score > 0.0 && !matches!(o.directive, AiDirective::None))
        .max_by(|a, b| a.score.total_cmp(&b.score))
}

/// The structured directive-change event's payload — the fields the `ai`-log
/// event carries.
///
/// Built by [`directive_change`] as a pure function of the two pools, so the
/// exact field VALUES the log line emits are unit-testable without a tracing
/// subscriber. The doctrine emitter logs these as individual `tracing` fields
/// (`prev`, `new`, `target`, `score`) alongside `tick` and `ship`, so a per-ship
/// directive timeline is `grep`-able out of a run's log stream.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectiveChange {
    /// The previous top-directive label, or `"none"` on the first observation /
    /// after the pool emptied.
    pub prev: String,
    /// The new top-directive label, or `"none"` if the ship now has no scored
    /// directive.
    pub new: String,
    /// The entity/anchor the new directive names, or empty for a target-less
    /// directive.
    pub target: String,
    /// The new top directive's utility score (`0.0` when there is none).
    pub score: f32,
}

/// `Some(change)` when the top directive of `new_pool` differs from that of
/// `prev_pool`; `None` when it is unchanged (no event this tick).
///
/// `prev_pool` is last tick's still-present scored pool read off the ship's
/// viewscreen blackboard *before* the doctrine aggregator overwrites it, so the
/// comparison uses only authoritative state and needs no cross-tick tracking
/// resource. The first observation (an empty `prev_pool`) reports a change from
/// `"none"`, which is the timeline's opening entry rather than a suppressed one.
pub fn directive_change(
    prev_pool: &[ScoredObjective],
    new_pool: &[ScoredObjective],
) -> Option<DirectiveChange> {
    let prev = top_directive(prev_pool)
        .map(|o| directive_label(&o.directive))
        .unwrap_or_else(|| "none".to_string());
    let new_top = top_directive(new_pool);
    let new = new_top
        .map(|o| directive_label(&o.directive))
        .unwrap_or_else(|| "none".to_string());
    if prev == new {
        return None;
    }
    Some(DirectiveChange {
        prev,
        new,
        target: new_top
            .and_then(|o| directive_target(&o.directive))
            .unwrap_or("")
            .to_string(),
        score: new_top.map(|o| o.score).unwrap_or(0.0),
    })
}

/// A one-line summary of a scored pool for the per-tick `debug` scoring trace.
///
/// Names the top [`POOL_TRACE_LIMIT`] candidates by score — `id=score[label]` —
/// and counts the rest, so the trace shows *why* the top directive won without
/// flooding the log. A read-only view: it sorts a borrow, never the pool.
pub fn format_pool(scored: &[ScoredObjective]) -> String {
    let mut view: Vec<&ScoredObjective> = scored.iter().collect();
    view.sort_by(|a, b| b.score.total_cmp(&a.score));
    let shown: Vec<String> = view
        .iter()
        .take(POOL_TRACE_LIMIT)
        .map(|o| format!("{}={:.1}[{}]", o.id, o.score, directive_label(&o.directive)))
        .collect();
    if view.len() > POOL_TRACE_LIMIT {
        format!(
            "{} candidates: {} +{} more",
            view.len(),
            shown.join(", "),
            view.len() - POOL_TRACE_LIMIT
        )
    } else {
        format!("{} candidates: {}", view.len(), shown.join(", "))
    }
}

#[cfg(test)]
#[path = "decision_trace_tests.rs"]
mod tests;
