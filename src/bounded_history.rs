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

use std::collections::VecDeque;

/// A ring of at most `capacity` recent `f64` samples, oldest evicted first.
///
/// `capacity == 0` is legal and degenerate: nothing is ever retained and the
/// window is never full, so every window predicate answers `false`. That is the
/// safe reading of "the designer authored a zero-length window" — it disables
/// the decision rather than making it trivially true.
#[derive(Clone, Debug, Default, PartialEq)]
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_window_is_empty_and_not_full() {
        let w = BoundedHistory::new(3);
        assert!(w.is_empty());
        assert!(!w.is_full());
        assert_eq!(w.len(), 0);
        assert_eq!(w.capacity(), 3);
        assert_eq!(w.min(), None);
        assert_eq!(w.max(), None);
        assert_eq!(w.last(), None);
    }

    #[test]
    fn it_fills_to_capacity_and_then_stops_growing() {
        let mut w = BoundedHistory::new(3);
        for n in 0..100 {
            w.push(n as f64);
            assert!(w.len() <= 3, "the window must never exceed its capacity");
        }
        assert!(w.is_full());
        assert_eq!(w.len(), 3);
        // The retained samples are the LAST three, oldest first.
        assert_eq!(w.iter().collect::<Vec<_>>(), vec![97.0, 98.0, 99.0]);
        assert_eq!(w.last(), Some(99.0));
    }

    #[test]
    fn the_oldest_sample_is_the_one_evicted() {
        let mut w = BoundedHistory::new(2);
        w.push(1.0);
        w.push(2.0);
        assert_eq!(w.min(), Some(1.0));
        w.push(5.0);
        assert_eq!(
            w.min(),
            Some(2.0),
            "1.0 aged out of the window, so it must stop counting"
        );
        assert_eq!(w.max(), Some(5.0));
    }

    /// The property a running aggregate cannot provide: one bad sample stops
    /// mattering once it ages out. A running minimum would be stuck at 1.0.
    #[test]
    fn a_stale_bad_sample_ages_out_instead_of_poisoning_the_window() {
        let mut w = BoundedHistory::new(3);
        w.push(1.0);
        w.push(10.0);
        w.push(10.0);
        assert!(
            !w.all_at_least(5.0),
            "the bad sample is still in the window"
        );
        w.push(10.0);
        assert!(
            w.all_at_least(5.0),
            "it has aged out; the window is now clean"
        );
    }

    #[test]
    fn all_at_least_is_false_until_the_window_is_full() {
        let mut w = BoundedHistory::new(3);
        w.push(100.0);
        assert!(
            !w.all_at_least(5.0),
            "one good sample is not a maintained distance"
        );
        w.push(100.0);
        assert!(!w.all_at_least(5.0));
        w.push(100.0);
        assert!(w.all_at_least(5.0));
    }

    #[test]
    fn all_at_least_is_inclusive_at_the_threshold() {
        let mut w = BoundedHistory::new(2);
        w.push(5.0);
        w.push(5.0);
        assert!(w.all_at_least(5.0));
        assert!(!w.all_at_least(5.000_001));
    }

    #[test]
    fn a_zero_capacity_window_retains_nothing_and_never_answers_held() {
        let mut w = BoundedHistory::new(0);
        w.push(100.0);
        w.push(100.0);
        assert!(w.is_empty());
        assert!(!w.is_full());
        assert!(!w.all_at_least(0.0));
    }

    #[test]
    fn clearing_drops_the_samples_but_keeps_the_capacity() {
        let mut w = BoundedHistory::new(2);
        w.push(1.0);
        w.push(2.0);
        assert!(w.is_full());
        w.clear();
        assert!(w.is_empty());
        assert_eq!(w.capacity(), 2);
        assert!(!w.all_at_least(0.0), "a cleared window is not a full one");
    }

    #[test]
    fn shrinking_the_capacity_discards_the_oldest_samples() {
        let mut w = BoundedHistory::new(4);
        for n in 1..=4 {
            w.push(n as f64);
        }
        w.set_capacity(2);
        assert_eq!(w.iter().collect::<Vec<_>>(), vec![3.0, 4.0]);
        assert!(w.is_full());
    }

    #[test]
    fn growing_the_capacity_keeps_what_is_there_but_un_fulls_the_window() {
        let mut w = BoundedHistory::new(2);
        w.push(1.0);
        w.push(2.0);
        assert!(w.is_full());
        w.set_capacity(4);
        assert_eq!(w.len(), 2);
        assert!(
            !w.is_full(),
            "a grown window needs new samples before it is full again"
        );
        assert_eq!(w.iter().collect::<Vec<_>>(), vec![1.0, 2.0]);
    }

    #[test]
    fn re_authoring_the_same_capacity_does_not_reset_the_window() {
        let mut w = BoundedHistory::new(3);
        for n in 0..3 {
            w.push(n as f64);
        }
        for _ in 0..10 {
            w.set_capacity(3);
        }
        assert!(
            w.is_full(),
            "an idempotent re-author must not clear the history"
        );
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn setting_the_capacity_to_zero_empties_the_window() {
        let mut w = BoundedHistory::new(3);
        w.push(1.0);
        w.set_capacity(0);
        assert!(w.is_empty());
        assert!(!w.is_full());
    }
}
