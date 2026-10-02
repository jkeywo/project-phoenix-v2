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
