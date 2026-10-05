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
            progress: None,
            unassigned: false,
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
