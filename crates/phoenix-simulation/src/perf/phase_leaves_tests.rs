use super::*;
use crate::perf::phase::Interval;
use crate::sim_sets::SimSet;
use bevy::prelude::*;

#[derive(Resource, Default)]
struct Calls(u32);
fn first(mut calls: ResMut<Calls>) {
    calls.0 += 1;
}
fn second(mut calls: ResMut<Calls>) {
    calls.0 += 10;
}
fn duplicate() {}

#[test]
fn typed_leaf_membership_is_observed_only_after_normal_execution_without_mutation() {
    let mut app = App::new();
    app.init_resource::<Calls>()
        .init_schedule(FixedUpdate)
        .configure_sets(FixedUpdate, FixedStep::TickMounts.in_set(SimSet::Physics))
        .add_systems(
            FixedUpdate,
            (
                first.in_set(FixedStep::TickMounts),
                second
                    .in_set(FixedStep::TickMounts)
                    .in_set(FixedStep::TickCommsRadio),
                duplicate.in_set(FixedStep::TickMounts),
                duplicate.in_set(FixedStep::TickCommsRadio),
            ),
        );
    assert!(observe(&app).is_err());
    app.world_mut().run_schedule(FixedUpdate);
    let before = crate::perf::phase_trace::observe_schedule(&app).unwrap();
    let rows = observe(&app).unwrap();
    let map = attribution(&rows);
    let row = rows
        .iter()
        .find(|row| row.name.ends_with("::first"))
        .unwrap();
    assert_eq!(row.fixed_steps, ["TickMounts"]);
    assert_eq!(map[&row.name].as_deref(), Some("TickMounts"));
    let row = rows
        .iter()
        .find(|row| row.name.ends_with("::second"))
        .unwrap();
    assert_eq!(row.fixed_steps, ["TickCommsRadio", "TickMounts"]);
    assert_eq!(map[&row.name], None);
    let duplicates: Vec<_> = rows
        .iter()
        .filter(|row| row.name.ends_with("::duplicate"))
        .collect();
    assert_eq!(duplicates.len(), 2);
    assert_ne!(duplicates[0].node, duplicates[1].node);
    assert_eq!(map[&duplicates[0].name], None);
    assert_eq!(app.world().resource::<Calls>().0, 11);
    assert_eq!(
        before,
        crate::perf::phase_trace::observe_schedule(&app).unwrap()
    );
    assert_eq!(rows, observe(&app).unwrap());
    let mut coverage = LeafCoverage::default();
    coverage.record(&app, &[], &BTreeMap::new()).unwrap();
    assert_eq!(coverage.schedule.as_ref(), Some(&rows));
    assert!(coverage.samples.is_empty());
    let name = duplicates[0].name.clone();
    let raw = RawInvocation {
        id: 0,
        interval: interval(0, 2),
        systems: vec![
            super::super::RawSystem {
                name: name.clone(),
                interval: interval(0, 1),
                unattributed: false,
                ambiguous: true,
            },
            super::super::RawSystem {
                name: "condition-with-no-graph-node".into(),
                interval: interval(1, 2),
                unattributed: true,
                ambiguous: false,
            },
        ],
    };
    coverage
        .record(&app, &[raw], &BTreeMap::from([(name.clone(), 2)]))
        .unwrap();
    let metadata = &coverage.unattributed_name_metadata[&name];
    assert_eq!(metadata.schedule_nodes.len(), 2);
    assert_eq!(metadata.fixed_step_memberships.len(), 2);
    assert!(
        coverage.unattributed_name_metadata["condition-with-no-graph-node"]
            .schedule_nodes
            .is_empty()
    );
    assert_eq!(
        serde_json::to_value(&coverage).unwrap()["unattributed_timing_kind"],
        "NAMEGROUP"
    );
    assert_eq!(coverage.samples.len(), 1);

    // A real graph change is refused after its normal initialization too.
    app.add_systems(FixedUpdate, duplicate.in_set(FixedStep::TickCommsRadio));
    app.world_mut().run_schedule(FixedUpdate);
    assert!(coverage.record(&app, &[], &BTreeMap::new()).is_err());
    assert_eq!(coverage.schedule.as_ref(), Some(&rows));
}

fn interval(start: u64, end: u64) -> Interval {
    Interval {
        start: Duration::from_millis(start),
        end: Duration::from_millis(end),
    }
}
#[test]
fn leaf_aggregation_unions_overlap_and_retains_every_ambiguous_interval() {
    let membership = BTreeMap::from([
        ("first".into(), Some("TickMounts".into())),
        ("peer".into(), Some("TickMounts".into())),
        ("multi".into(), None),
        ("nested".into(), Some("TickMounts".into())),
        ("duplicate".into(), Some("TickMounts".into())),
        ("closed-duplicate".into(), Some("TickMounts".into())),
    ]);
    let names = [
        "first",
        "peer",
        "multi",
        "missing",
        "nested",
        "duplicate",
        "closed-duplicate",
    ];
    let mut counts: BTreeMap<_, _> = names.iter().map(|name| ((*name).into(), 1)).collect();
    counts.insert("duplicate".into(), 2);
    let spans = [
        ("first", 1, 4, false, false),
        ("peer", 3, 6, false, false),
        ("first", 8, 9, false, false),
        ("multi", 2, 3, false, false),
        ("missing", 3, 4, false, false),
        ("nested", 4, 5, true, false),
        ("duplicate", 5, 6, false, false),
        ("closed-duplicate", 6, 7, false, true),
    ];
    let raw = RawInvocation {
        id: 42,
        interval: interval(0, 10),
        systems: spans
            .into_iter()
            .map(
                |(name, start, end, unattributed, ambiguous)| super::super::RawSystem {
                    name: name.into(),
                    interval: interval(start, end),
                    unattributed,
                    ambiguous,
                },
            )
            .collect(),
    };
    let output = reduce(&raw, &membership, &counts);
    assert_eq!(output.id, 42);
    assert_eq!(output.elapsed_ms, 10.0);
    let owned = output
        .groups
        .iter()
        .find(|row| row.fixed_step.as_deref() == Some("TickMounts"))
        .unwrap();
    assert_eq!(
        (owned.observed_ms, owned.envelope_ms, owned.intervals),
        (6.0, 8.0, 3)
    );
    let unknown = output
        .groups
        .iter()
        .find(|row| row.fixed_step.is_none())
        .unwrap();
    assert_eq!(
        (unknown.observed_ms, unknown.envelope_ms, unknown.intervals),
        (5.0, 5.0, 5)
    );
    assert_eq!(output.unattributed_names.len(), 5);
    assert_eq!(
        output.groups.iter().map(|row| row.intervals).sum::<usize>(),
        raw.systems.len()
    );
    // Same borrowed raw intervals remain available to the ordinary phase reducer.
    assert_eq!(raw.systems[0].interval, interval(1, 4));
    assert_eq!(raw.systems[7].interval, interval(6, 7));
}

#[test]
fn empty_or_ambiguous_metadata_never_invents_a_leaf() {
    let rows = vec![
        LeafScheduleRow {
            node: "1".into(),
            name: "same".into(),
            fixed_steps: vec!["TickMounts".into()],
        },
        LeafScheduleRow {
            node: "2".into(),
            name: "same".into(),
            fixed_steps: vec!["TickMounts".into()],
        },
        LeafScheduleRow {
            node: "3".into(),
            name: "empty".into(),
            fixed_steps: vec![],
        },
    ];
    assert_eq!(
        attribution(&rows),
        BTreeMap::from([("same".into(), None), ("empty".into(), None)])
    );
}

#[test]
fn unattributed_namegroup_unions_duplicate_and_nested_spans_without_claiming_node_costs() {
    let make = |name: &str, start, end, unattributed, ambiguous| super::super::RawSystem {
        name: name.into(),
        interval: interval(start, end),
        unattributed,
        ambiguous,
    };
    let raw = RawInvocation {
        id: 7,
        interval: interval(0, 12),
        systems: vec![
            make("known", 0, 1, false, false),
            make("duplicate-name", 1, 5, false, true),
            make("duplicate-name", 3, 7, true, true),
            make("duplicate-name", 9, 10, false, false),
            make("other-name", 4, 9, true, false),
            make("closed-span", 10, 11, false, true),
        ],
    };
    let membership = BTreeMap::from([
        ("known".into(), Some("TickMounts".into())),
        ("duplicate-name".into(), Some("TickMounts".into())),
    ]);
    let counts = BTreeMap::from([
        ("known".into(), 1),
        ("duplicate-name".into(), 2),
        ("other-name".into(), 1),
    ]);
    // Snapshot the independent existing phase reduction and its public schema.
    let ordinary = || {
        crate::perf::phase::reduce(&[crate::perf::phase::FixedInvocation {
            id: raw.id,
            interval: raw.interval,
            systems: raw
                .systems
                .iter()
                .map(|span| crate::perf::phase::ObservedSystem {
                    interval: span.interval,
                    phase: Some(SimSet::Input),
                })
                .collect(),
        }])
        .unwrap()
    };
    let phase_before = ordinary();
    let schema_before = serde_json::to_value(super::super::PhaseCoverage::default()).unwrap();
    let output = reduce(&raw, &membership, &counts);
    let name = &output.unattributed_name_groups["duplicate-name"];
    assert_eq!(
        (name.observed_ms, name.envelope_ms, name.intervals),
        (7.0, 9.0, 3)
    );
    assert_eq!(name.nested_or_auxiliary_intervals, 1);
    assert_eq!(name.ambiguous_at_entry_intervals, 2);
    assert_eq!(name.live_system_spans_at_drain, Some(2));
    let closed = &output.unattributed_name_groups["closed-span"];
    assert_eq!(closed.live_system_spans_at_drain, None);
    assert_eq!(closed.ambiguous_at_entry_intervals, 1);
    assert_eq!(output.unattributed_names["duplicate-name"], name.intervals);
    assert!(!output.unattributed_name_groups.contains_key("known"));
    let known = output
        .groups
        .iter()
        .find(|row| row.fixed_step.as_deref() == Some("TickMounts"))
        .unwrap();
    assert_eq!(
        (known.observed_ms, known.envelope_ms, known.intervals),
        (1.0, 1.0, 1)
    );
    let unknown = output
        .groups
        .iter()
        .find(|row| row.fixed_step.is_none())
        .unwrap();
    assert_eq!(
        (unknown.observed_ms, unknown.envelope_ms, unknown.intervals),
        (10.0, 10.0, 5)
    );
    assert!(
        output
            .unattributed_name_groups
            .values()
            .map(|row| row.observed_ms)
            .sum::<f64>()
            > unknown.observed_ms,
        "different name groups overlap; never add them as exclusive costs"
    );
    assert_eq!(ordinary(), phase_before);
    assert_eq!(
        serde_json::to_value(super::super::PhaseCoverage::default()).unwrap(),
        schema_before
    );
    assert!(schema_before.get("unattributed_name_groups").is_none());
}
