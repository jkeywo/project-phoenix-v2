//! Read-only execution evidence for a disposable Workshop Test.
//!
//! This resource is installed only by the browser iframe and native Test child.
//! It observes the existing script applier; it is never declared authoritative,
//! snapshotted, digested, or offered back to Rhai.

use std::collections::VecDeque;

use bevy::prelude::Resource;

use super::test_protocol::{TestTraceKind, TestTraceRecord, TestTraceSource};

pub const TEST_TRACE_CAPACITY: usize = 256;

#[derive(Resource, Debug)]
pub struct TestTrace {
    records: VecDeque<TestTraceRecord>,
    last_tick: Option<u64>,
    next_order: u32,
}

impl Default for TestTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::with_capacity(TEST_TRACE_CAPACITY),
            last_tick: None,
            next_order: 0,
        }
    }
}

impl TestTrace {
    pub fn records(&self) -> Vec<TestTraceRecord> {
        self.records.iter().cloned().collect()
    }

    pub fn push(
        &mut self,
        tick: u64,
        source_path: Option<&str>,
        line: Option<usize>,
        kind: TestTraceKind,
    ) {
        if self.last_tick != Some(tick) {
            self.last_tick = Some(tick);
            self.next_order = 0;
        }
        let record = TestTraceRecord {
            tick,
            order: self.next_order,
            source: TestTraceSource {
                path: source_path.map(str::to_owned),
                line,
            },
            kind,
        };
        self.next_order = self.next_order.saturating_add(1);
        if self.records.len() == TEST_TRACE_CAPACITY {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }
}

#[cfg(test)]
mod tests {
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
}
