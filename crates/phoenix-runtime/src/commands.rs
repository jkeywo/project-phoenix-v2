//! Canonical delayed input, independent of the game's admission and payload types.
use std::collections::BTreeMap;

use crate::{CommandOrder, HostSlot};

#[derive(Debug)]
pub struct CommandQueue<T> {
    origin: HostSlot,
    next_seq: u64,
    queue: BTreeMap<(u64, CommandOrder), T>,
}

impl<T> Default for CommandQueue<T> {
    fn default() -> Self {
        Self {
            origin: HostSlot::SOLO,
            next_seq: 0,
            queue: BTreeMap::new(),
        }
    }
}

impl<T> CommandQueue<T> {
    pub fn set_origin(&mut self, origin: HostSlot) {
        self.origin = origin;
    }

    pub fn origin(&self) -> HostSlot {
        self.origin
    }

    pub fn next_sequence(&self) -> u64 {
        self.next_seq
    }

    /// Restore the issuer at an agreed checkpoint before populating pending input.
    pub fn restore_sequence(&mut self, next_sequence: u64) {
        assert!(
            self.queue.is_empty(),
            "restore the issuer before pending input"
        );
        self.next_seq = next_sequence;
    }

    /// Mint only for locally admitted input. Peer input keeps its original key.
    pub fn next_order(&mut self) -> CommandOrder {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        CommandOrder::new(self.origin, seq)
    }

    /// A retransmission with the same canonical key replaces that pending entry.
    pub fn insert(&mut self, key: (u64, CommandOrder), value: T) -> Option<T> {
        self.queue.insert(key, value)
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.queue.values()
    }

    /// Retain the established tick partition, including its saturating clock bound.
    pub fn drain_due(&mut self, now: u64) -> impl Iterator<Item = T> {
        let later = self
            .queue
            .split_off(&(now.saturating_add(1), CommandOrder::default()));
        std::mem::replace(&mut self.queue, later).into_values()
    }

    /// A new run clears input and sequence; the participant retains its identity.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.next_seq = 0;
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
