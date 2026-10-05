use super::*;

#[test]
fn bounded_records_keep_runtime_order_and_tick_local_order() {
    let mut trace = TestTrace::default();
    for tick in 0..=TEST_TRACE_CAPACITY as u64 {
        trace.push(
            tick,
            Some("assets/worlds/test.rhai"),
            Some(4),
            TestTraceKind::HostCall {
                function: format!("call_{tick}"),
            },
        );
    }
    let records = trace.records();
    assert_eq!(records.len(), TEST_TRACE_CAPACITY);
    assert_eq!(records.first().unwrap().tick, 1);
    assert_eq!(records.last().unwrap().tick, TEST_TRACE_CAPACITY as u64);
    assert!(records.iter().all(|record| record.order == 0));

    trace.push(
        TEST_TRACE_CAPACITY as u64,
        Some("assets/worlds/test.rhai"),
        None,
        TestTraceKind::CallbackFired {
            function: "later".into(),
            scheduled_tick: TEST_TRACE_CAPACITY as u64,
        },
    );
    assert_eq!(trace.records().last().unwrap().order, 1);
}

#[test]
fn observations_are_outside_the_authoritative_digest() {
    let mut world = bevy::prelude::World::new();
    let before = crate::sim_digest::world_digest(&world);
    let mut trace = TestTrace::default();
    trace.push(
        9,
        Some("assets/worlds/test.rhai"),
        Some(2),
        TestTraceKind::HostCall {
            function: "observe".into(),
        },
    );
    world.insert_resource(trace);
    assert_eq!(crate::sim_digest::world_digest(&world), before);
}
