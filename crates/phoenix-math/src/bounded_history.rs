//! A fixed-capacity history window (issue #788).
//!
//! Pure Rust, no Bevy imports, no domain types: samples are plain `f64` and the
//! capacity is a plain `usize` the caller sources from authored data. Fully
//! unit-testable on native, in the same shape as `asteroids::window`.
//!
//! # Why this exists
//!
//! A decision like "has this ship *held* a distance, not merely touched it once"
//! cannot be made from a single-tick fact, and cannot be made from a running
//! aggregate either — a running minimum never recovers once a single bad sample
//! folds into it. It needs the last N readings and nothing older.
//!
//! The bound is the point. A `Vec` that only grows is a leak in a simulation
//! that runs for hours, and a growing window silently changes the meaning of
//! "recently" as the run goes on. [`BoundedHistory`] overwrites in place: memory
//! is `capacity` samples for ever, and the window always means exactly the last
//! `capacity` readings.
//!
//! # Not full until it is full
//!
//! [`BoundedHistory::is_full`] is separate from [`BoundedHistory::len`] on
//! purpose. A predicate like "every sample in the window is above X" is
//! vacuously true over an empty window, so a caller that forgets to check
//! fullness would answer "yes, held" on the very first tick. Callers are
//! expected to gate on `is_full()`; [`BoundedHistory::all_at_least`] does that
//! for them.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// A ring of at most `capacity` recent `f64` samples, oldest evicted first.
///
/// `capacity == 0` is legal and degenerate: nothing is ever retained and the
/// window is never full, so every window predicate answers `false`. That is the
/// safe reading of "the designer authored a zero-length window" — it disables
/// the decision rather than making it trivially true.
///
/// Serialisable because it is a field of `world::flags::AiHistory`, which is
/// itself a field of `world::flags::AiPolicyMemory` — serde for the #862
/// snapshot payload; the payload boundary is the #894 record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BoundedHistory {
    capacity: usize,
    samples: VecDeque<f64>,
}

impl BoundedHistory {
    /// An empty window of the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            samples: VecDeque::with_capacity(capacity),
        }
    }

    /// The authored window length.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many samples are currently retained (never more than `capacity`).
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// `true` once the window holds a full `capacity` samples. Always `false`
    /// for a zero capacity.
    pub fn is_full(&self) -> bool {
        self.capacity > 0 && self.samples.len() >= self.capacity
    }

    /// Re-author the window length, discarding the oldest samples if the new
    /// length is shorter.
    ///
    /// Exists because the capacity comes from authored data the host may only
    /// learn about after the window was constructed (a ship's config resolves
    /// at spawn, the component default does not). Re-authoring to the SAME
    /// capacity is a no-op, so calling it every tick is free and cannot reset
    /// the window.
    pub fn set_capacity(&mut self, capacity: usize) {
        if capacity == self.capacity {
            return;
        }
        self.capacity = capacity;
        self.trim();
    }

    /// Record one sample, evicting the oldest when the window is full.
    pub fn push(&mut self, sample: f64) {
        if self.capacity == 0 {
            self.samples.clear();
            return;
        }
        self.samples.push_back(sample);
        self.trim();
    }

    /// Drop every retained sample, keeping the capacity. Used when the thing
    /// being measured changes identity, so the new measurement never inherits
    /// the old one's history.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Evict from the front until the window fits its capacity.
    fn trim(&mut self) {
        while self.samples.len() > self.capacity {
            self.samples.pop_front();
        }
    }

    /// The smallest retained sample, or `None` when empty.
    pub fn min(&self) -> Option<f64> {
        self.samples.iter().copied().fold(None, |acc, v| {
            Some(match acc {
                Some(m) => v.min(m),
                None => v,
            })
        })
    }

    /// The largest retained sample, or `None` when empty.
    pub fn max(&self) -> Option<f64> {
        self.samples.iter().copied().fold(None, |acc, v| {
            Some(match acc {
                Some(m) => v.max(m),
                None => v,
            })
        })
    }

    /// The most recently pushed sample.
    pub fn last(&self) -> Option<f64> {
        self.samples.back().copied()
    }

    /// Iterate the retained samples, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = f64> + '_ {
        self.samples.iter().copied()
    }

    /// `true` when the window is FULL and every retained sample is `>=
    /// threshold`.
    ///
    /// The fullness half is not optional: over a partly-filled window this
    /// would answer "held" from a single good sample, which is the exact
    /// opposite of what a "has it been maintained" question is asking.
    pub fn all_at_least(&self, threshold: f64) -> bool {
        self.is_full() && self.samples.iter().all(|v| *v >= threshold)
    }

    /// The NET change across a FULL window — newest sample minus oldest — or
    /// `None` until the window is full (issue #789).
    ///
    /// The sibling of [`Self::all_at_least`], and the difference between them is
    /// the difference between a *level* question and a *trend* one.
    /// `all_at_least` answers "has every reading stayed past a line"; this
    /// answers "which way, and how far, has the reading moved over the authored
    /// span". A caller asking whether something is getting better or worse
    /// cannot get that from a minimum, a maximum, or a single reading — it needs
    /// the two ends of a bounded window.
    ///
    /// The fullness gate is not optional here either, and for a sharper reason
    /// than `all_at_least`'s: over a partly-filled window this measures a
    /// SHORTER span than the one the designer authored, so the answer would be
    /// smaller in magnitude simply because less time had passed. A decision
    /// taken from it would fire early, on less evidence than it asked for, and
    /// would do so most reliably right after a `clear()` — exactly when the
    /// caller knows least.
    ///
    /// Sign is the caller's to interpret: positive means the newest reading is
    /// larger than the oldest.
    pub fn net_change(&self) -> Option<f64> {
        if !self.is_full() {
            return None;
        }
        Some(self.samples.back()? - self.samples.front()?)
    }
}

/// A fixed-capacity ring of at most `capacity` recent `T` values, oldest
/// evicted first (issue #1151).
///
/// The pure ring mechanic that [`BoundedHistory`] is the `f64`-window
/// specialisation of: identical eviction rule, identical `capacity == 0`
/// degenerate reading (nothing is ever retained), but generic over the sample
/// type and WITHOUT the numeric reducers (`min` / `max` / `net_change` /
/// `all_at_least`) that only mean anything over reals. `BoundedHistory` keeps
/// its own `VecDeque<f64>` storage rather than wrapping this, so its #862
/// snapshot wire shape is untouched; the two share a design, not a field.
///
/// # Why the trigger-fire recorder needs this and not [`BoundedHistory`]
///
/// A fire record (`crate::debug::payload::TriggerFire`, in the root crate) is a struct of a time
/// and a list of predicate values, not an `f64`, so it cannot go in the numeric
/// window at all. What the recorder actually reuses from `bounded_history` is
/// the *bound*: the ring is `capacity` records per trigger for ever, so a
/// session that runs for hours keeps the last `capacity` fires of each trigger
/// and nothing older — a `Vec` that only grows is a leak in exactly that run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundedRing<T> {
    capacity: usize,
    samples: VecDeque<T>,
}

impl<T> Default for BoundedRing<T> {
    /// A zero-capacity ring — the degenerate window that retains nothing, the
    /// same safe reading of "no length authored yet" [`BoundedHistory`] takes.
    /// Hand-written rather than derived so the bound is not `T: Default` (a
    /// record type need not be default-constructible to live in a ring of them).
    fn default() -> Self {
        Self {
            capacity: 0,
            samples: VecDeque::new(),
        }
    }
}

impl<T> BoundedRing<T> {
    /// An empty ring of the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            samples: VecDeque::with_capacity(capacity),
        }
    }

    /// The authored ring length.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many records are currently retained (never more than `capacity`).
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// `true` once the ring holds a full `capacity` records. Always `false` for
    /// a zero capacity.
    pub fn is_full(&self) -> bool {
        self.capacity > 0 && self.samples.len() >= self.capacity
    }

    /// Re-author the ring length, discarding the oldest records if the new
    /// length is shorter. Re-authoring to the SAME capacity is a no-op, so
    /// calling it every tick (the recorder does, to track a retuned config) is
    /// free and cannot reset the ring.
    pub fn set_capacity(&mut self, capacity: usize) {
        if capacity == self.capacity {
            return;
        }
        self.capacity = capacity;
        self.trim();
    }

    /// Record one value, evicting the oldest when the ring is full. A
    /// zero-capacity ring retains nothing.
    pub fn push(&mut self, sample: T) {
        if self.capacity == 0 {
            self.samples.clear();
            return;
        }
        self.samples.push_back(sample);
        self.trim();
    }

    /// Drop every retained record, keeping the capacity.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Evict from the front until the ring fits its capacity.
    fn trim(&mut self) {
        while self.samples.len() > self.capacity {
            self.samples.pop_front();
        }
    }

    /// The most recently pushed record.
    pub fn last(&self) -> Option<&T> {
        self.samples.back()
    }

    /// Iterate the retained records, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.samples.iter()
    }
}

#[cfg(test)]
#[path = "bounded_history_tests.rs"]
mod tests;
