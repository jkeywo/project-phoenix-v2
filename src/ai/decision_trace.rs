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
        | AiDirective::FieldRepair { target } => Some(target.as_str()),
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
mod tests {
    use super::*;
    use crate::core::messages::{
        ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective, SystemAffinity,
    };
    use std::collections::BTreeMap;

    fn obj(id: &str, score: f32, directive: AiDirective) -> ScoredObjective {
        ScoredObjective {
            id: id.to_string(),
            score,
            directive,
            source: ObjectiveSource::Doctrine,
            relevance: vec![SystemAffinity::Helm],
            snapshot: ObjectiveSnapshot {
                id: id.to_string(),
                text: String::new(),
                text_params: BTreeMap::new(),
                mandatory: false,
                status: ObjectiveStatus::Active,
                targets: Vec::new(),
                source: ObjectiveSource::Doctrine,
            },
        }
    }

    #[test]
    fn directive_label_names_kind_and_target() {
        assert_eq!(directive_label(&AiDirective::None), "none");
        assert_eq!(
            directive_label(&AiDirective::Destroy {
                target: "Ashrender".into()
            }),
            "Destroy(Ashrender)"
        );
        assert_eq!(
            directive_label(&AiDirective::Reach {
                anchor: "picket".into()
            }),
            "Reach(picket)"
        );
        assert_eq!(
            directive_label(&AiDirective::Patrol {
                anchors: vec!["a".into(), "b".into()],
                loop_path: true,
            }),
            "Patrol(a>b loop)"
        );
        assert_eq!(
            directive_label(&AiDirective::Scan {
                target: "Ladder B".into()
            }),
            "Scan(Ladder B)"
        );
    }

    #[test]
    fn directive_target_reads_the_named_entity_or_anchor() {
        assert_eq!(
            directive_target(&AiDirective::Destroy {
                target: "Ashrender".into()
            }),
            Some("Ashrender")
        );
        assert_eq!(
            directive_target(&AiDirective::Patrol {
                anchors: vec!["first".into(), "second".into()],
                loop_path: false,
            }),
            Some("first")
        );
        assert_eq!(
            directive_target(&AiDirective::Scan {
                target: "Skyhook".into()
            }),
            Some("Skyhook")
        );
        assert_eq!(directive_target(&AiDirective::None), None);
    }

    #[test]
    fn top_directive_picks_highest_positive_real_directive() {
        let pool = vec![
            obj(
                "patrol",
                30.0,
                AiDirective::Patrol {
                    anchors: vec!["p".into()],
                    loop_path: true,
                },
            ),
            obj(
                "kill",
                38.0,
                AiDirective::Destroy {
                    target: "Ashrender".into(),
                },
            ),
            // A higher score, but human-facing (no directive): must be ignored.
            obj("brief", 99.0, AiDirective::None),
        ];
        assert_eq!(top_directive(&pool).unwrap().id, "kill");
    }

    #[test]
    fn top_directive_ignores_gated_out_and_empty_pools() {
        assert!(top_directive(&[]).is_none());
        let all_zero = vec![obj(
            "gated",
            0.0,
            AiDirective::Destroy { target: "x".into() },
        )];
        assert!(top_directive(&all_zero).is_none());
    }

    #[test]
    fn directive_change_reports_only_on_a_real_change() {
        let patrol = || {
            vec![obj(
                "patrol",
                30.0,
                AiDirective::Patrol {
                    anchors: vec!["p".into()],
                    loop_path: true,
                },
            )]
        };
        // Same top directive two ticks running → no event.
        assert!(directive_change(&patrol(), &patrol()).is_none());
    }

    #[test]
    fn directive_change_carries_the_new_directives_fields() {
        let prev = vec![obj(
            "patrol",
            30.0,
            AiDirective::Patrol {
                anchors: vec!["p".into()],
                loop_path: true,
            },
        )];
        let new = vec![obj(
            "kill",
            38.0,
            AiDirective::Destroy {
                target: "Ashrender".into(),
            },
        )];
        let change = directive_change(&prev, &new).expect("a change was expected");
        assert_eq!(change.prev, "Patrol(p loop)");
        assert_eq!(change.new, "Destroy(Ashrender)");
        assert_eq!(change.target, "Ashrender");
        assert_eq!(change.score, 38.0);
    }

    #[test]
    fn directive_change_opens_from_none_and_closes_to_none() {
        let kill = vec![obj(
            "kill",
            38.0,
            AiDirective::Destroy {
                target: "Ashrender".into(),
            },
        )];
        // First observation: none -> Destroy is the timeline's opening entry.
        let opened = directive_change(&[], &kill).expect("opening entry");
        assert_eq!(opened.prev, "none");
        assert_eq!(opened.new, "Destroy(Ashrender)");
        // Pool empties (everything gated out): Destroy -> none is a change too.
        let closed = directive_change(&kill, &[]).expect("closing entry");
        assert_eq!(closed.prev, "Destroy(Ashrender)");
        assert_eq!(closed.new, "none");
    }

    /// The acceptance criterion "a directive timeline for one ship can be
    /// reconstructed from the log stream" reduces to: feeding the per-tick pools
    /// through [`directive_change`] and keeping the `Some`s yields exactly the
    /// ship's ordered directive timeline, with unchanged ticks contributing
    /// nothing. This is the pure core the log stream carries.
    #[test]
    fn a_directive_timeline_is_the_sequence_of_changes() {
        let patrol = vec![obj(
            "patrol",
            30.0,
            AiDirective::Patrol {
                anchors: vec!["p".into()],
                loop_path: true,
            },
        )];
        let kill = vec![obj(
            "kill",
            38.0,
            AiDirective::Destroy {
                target: "Ashrender".into(),
            },
        )];
        let retreat = vec![obj(
            "run",
            100.0,
            AiDirective::Retreat {
                anchor: "haven".into(),
            },
        )];

        // Six ticks of pools; ticks 1,3,5 repeat the prior top directive.
        let per_tick = [&patrol, &patrol, &kill, &kill, &retreat, &retreat];
        let mut prev: &[ScoredObjective] = &[];
        let mut timeline: Vec<(String, String)> = Vec::new();
        for pool in per_tick {
            if let Some(change) = directive_change(prev, pool) {
                timeline.push((change.prev, change.new));
            }
            prev = pool;
        }

        assert_eq!(
            timeline,
            vec![
                ("none".to_string(), "Patrol(p loop)".to_string()),
                (
                    "Patrol(p loop)".to_string(),
                    "Destroy(Ashrender)".to_string()
                ),
                (
                    "Destroy(Ashrender)".to_string(),
                    "Retreat(haven)".to_string()
                ),
            ]
        );
    }

    #[test]
    fn format_pool_names_the_top_candidates_and_counts_the_rest() {
        let pool: Vec<ScoredObjective> = (0..8)
            .map(|i| {
                obj(
                    &format!("obj{i}"),
                    i as f32,
                    AiDirective::Reach {
                        anchor: format!("a{i}"),
                    },
                )
            })
            .collect();
        let s = format_pool(&pool);
        assert!(s.starts_with("8 candidates:"), "got {s}");
        // Highest score is listed first, lowest two are folded into the count.
        assert!(s.contains("obj7=7.0[Reach(a7)]"), "got {s}");
        assert!(s.contains("+2 more"), "got {s}");
    }
}
