//! Test-local state breakpoints evaluated at the completed fixed-tick boundary.
//!
//! This observes the ordinary world Flag stores after all fixed work. It does
//! not alter scenario state, expose script internals, or enter snapshots and
//! digests.

use bevy::{prelude::*, time::TimeUpdateStrategy};

use super::{
    test_clock::TestClock,
    test_protocol::{
        TestBreakpoint, TestBreakpointHit, TestTraceKind, TestTraceRecord, TestTraceSource,
    },
    test_trace::TestTrace,
};

#[derive(Resource, Debug, Default)]
pub struct TestBreakpointState {
    pub configured: Option<TestBreakpoint>,
    pub hit: Option<TestBreakpointHit>,
    was_true: bool,
}

impl TestBreakpointState {
    pub fn configured(breakpoint: Option<TestBreakpoint>) -> Self {
        Self {
            configured: breakpoint,
            hit: None,
            was_true: false,
        }
    }

    pub(crate) fn release(&mut self) {
        self.hit = None;
    }
}

fn adjacent_trace(
    records: &[TestTraceRecord],
    breakpoint: &TestBreakpoint,
) -> (TestTraceSource, Vec<TestTraceRecord>) {
    let selected = records.iter().rposition(|record| {
        matches!(&record.kind,
        TestTraceKind::FlagMutation { name, layer, .. }
            if name == breakpoint.name() && layer == &breakpoint.layer)
    });
    let Some(index) = selected else {
        return (
            TestTraceSource {
                path: None,
                line: None,
            },
            records
                .iter()
                .rev()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
        );
    };
    let start = index.saturating_sub(2);
    let end = (index + 3).min(records.len());
    (records[index].source.clone(), records[start..end].to_vec())
}

pub(crate) fn evaluate_breakpoint(
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    trace: Res<TestTrace>,
    tick: Res<crate::sim_tick::SimTick>,
    mut state: ResMut<TestBreakpointState>,
    mut clock: ResMut<TestClock>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut fixed_time: ResMut<Time<Fixed>>,
    mut paused: ResMut<crate::gm_action::SimulationPaused>,
) {
    let Some(breakpoint) = state.configured.as_ref() else {
        return;
    };
    let current = match breakpoint.layer.as_deref() {
        Some(path) => layers
            .as_ref()
            .and_then(|map| map.0.get(path))
            .map(|layer| layer.flags.counter(breakpoint.name())),
        None => runtime
            .as_ref()
            .map(|runtime| runtime.flags.counter(breakpoint.name())),
    };
    let Some(current) = current else {
        state.was_true = false;
        return;
    };
    let matches = breakpoint.matches(current);
    if matches && !state.was_true {
        let breakpoint = breakpoint.clone();
        let records = trace.records();
        let (source, adjacent_trace) = adjacent_trace(&records, &breakpoint);
        state.hit = Some(TestBreakpointHit {
            breakpoint,
            current,
            tick: tick.0,
            source,
            adjacent_trace,
        });
        clock.paused = true;
        clock.steps = 0;
        clock.stepping = false;
        paused.0 = true;
        virtual_time.pause();
        *strategy = TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO);
        let remainder = fixed_time.overstep();
        fixed_time.discard_overstep(remainder);
    }
    state.was_true = matches;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workshop::test_protocol::{TestBreakpointComparison, TestBreakpointCondition};

    #[test]
    fn breakpoint_state_is_digest_neutral() {
        let mut world = World::new();
        let before = crate::sim_digest::world_digest(&world);
        world.insert_resource(TestBreakpointState::configured(Some(TestBreakpoint {
            layer: None,
            condition: TestBreakpointCondition::Counter {
                name: "alarm".into(),
                comparison: TestBreakpointComparison::Ge,
                value: 2,
            },
        })));
        assert_eq!(crate::sim_digest::world_digest(&world), before);
    }

    #[test]
    fn hit_source_and_adjacent_records_follow_the_matching_flag_mutation() {
        let breakpoint = TestBreakpoint {
            layer: Some("assets/worlds/layer.toml".into()),
            condition: TestBreakpointCondition::Flag {
                name: "ready".into(),
                value: true,
            },
        };
        let records = (0..5)
            .map(|order| TestTraceRecord {
                tick: 4,
                order,
                source: TestTraceSource {
                    path: (order == 2).then(|| "assets/worlds/layer.rhai".into()),
                    line: (order == 2).then_some(7),
                },
                kind: TestTraceKind::FlagMutation {
                    name: if order == 2 { "ready" } else { "other" }.into(),
                    before: 0,
                    after: 1,
                    layer: Some("assets/worlds/layer.toml".into()),
                },
            })
            .collect::<Vec<_>>();
        let (source, adjacent) = adjacent_trace(&records, &breakpoint);
        assert_eq!(source.path.as_deref(), Some("assets/worlds/layer.rhai"));
        assert_eq!(source.line, Some(7));
        assert_eq!(adjacent, records);
    }
}
