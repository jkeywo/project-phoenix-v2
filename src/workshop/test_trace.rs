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
#[path = "test_trace_tests.rs"]
mod tests;
