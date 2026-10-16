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
