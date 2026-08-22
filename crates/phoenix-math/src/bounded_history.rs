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
    fn net_change_is_the_newest_sample_minus_the_oldest() {
        let mut w = BoundedHistory::new(3);
        w.push(10.0);
        w.push(20.0);
        w.push(40.0);
        assert_eq!(w.net_change(), Some(30.0));
        // A reading that goes the OTHER way is a negative net change, not an
        // absolute distance: the sign is the whole point of a trend.
        w.push(5.0);
        assert_eq!(w.net_change(), Some(-15.0), "20.0 is now the oldest sample");
    }

    #[test]
    fn net_change_is_none_until_the_window_is_full() {
        let mut w = BoundedHistory::new(3);
        w.push(0.0);
        assert_eq!(
            w.net_change(),
            None,
            "a one-sample window has no span to measure across"
        );
        w.push(100.0);
        assert_eq!(
            w.net_change(),
            None,
            "two samples is still a SHORTER span than the authored three: answering \
             here would report progress over a window nobody authored"
        );
        w.push(200.0);
        assert_eq!(w.net_change(), Some(200.0));
    }

    /// The same property `all_at_least` has: a cleared window is not a full one,
    /// so the trend goes unavailable until it has been re-earned.
    #[test]
    fn clearing_makes_the_net_change_unavailable_again() {
        let mut w = BoundedHistory::new(2);
        w.push(1.0);
        w.push(9.0);
        assert_eq!(w.net_change(), Some(8.0));
        w.clear();
        assert_eq!(w.net_change(), None);
    }

    #[test]
    fn a_zero_capacity_window_never_reports_a_net_change() {
        let mut w = BoundedHistory::new(0);
        w.push(1.0);
        w.push(2.0);
        assert_eq!(w.net_change(), None);
    }

    #[test]
    fn a_flat_window_reports_no_progress_rather_than_nothing() {
        let mut w = BoundedHistory::new(3);
        for _ in 0..3 {
            w.push(50.0);
        }
        assert_eq!(
            w.net_change(),
            Some(0.0),
            "a reading that has not moved is a MEASURED zero, not an absent answer"
        );
    }

    #[test]
    fn setting_the_capacity_to_zero_empties_the_window() {
        let mut w = BoundedHistory::new(3);
        w.push(1.0);
        w.set_capacity(0);
        assert!(w.is_empty());
        assert!(!w.is_full());
    }

    // ── BoundedRing<T> (issue #1151) ──────────────────────────────────────────
    //
    // The generic ring the trigger-fire recorder keeps per trigger. The same
    // bound-is-the-point properties `BoundedHistory` has, over an arbitrary
    // record type (here `&'static str`, standing in for a fire record).

    #[test]
    fn a_ring_fills_to_capacity_then_evicts_the_oldest() {
        let mut r: BoundedRing<&str> = BoundedRing::new(2);
        assert!(r.is_empty());
        r.push("a");
        r.push("b");
        assert!(r.is_full());
        assert_eq!(r.len(), 2);
        r.push("c");
        assert_eq!(r.len(), 2, "the ring must never exceed its capacity");
        assert_eq!(
            r.iter().copied().collect::<Vec<_>>(),
            vec!["b", "c"],
            "the oldest record aged out; the last `capacity` remain, oldest first"
        );
        assert_eq!(r.last(), Some(&"c"));
    }

    #[test]
    fn a_zero_capacity_ring_retains_nothing() {
        let mut r: BoundedRing<&str> = BoundedRing::new(0);
        r.push("a");
        r.push("b");
        assert!(r.is_empty());
        assert!(!r.is_full());
        assert_eq!(r.last(), None);
    }

    #[test]
    fn shrinking_a_ring_discards_the_oldest_records() {
        let mut r: BoundedRing<i32> = BoundedRing::new(4);
        for n in 1..=4 {
            r.push(n);
        }
        r.set_capacity(2);
        assert_eq!(r.iter().copied().collect::<Vec<_>>(), vec![3, 4]);
        assert!(r.is_full());
    }

    #[test]
    fn re_authoring_the_same_ring_capacity_does_not_reset_it() {
        let mut r: BoundedRing<i32> = BoundedRing::new(3);
        for n in 0..3 {
            r.push(n);
        }
        for _ in 0..10 {
            r.set_capacity(3);
        }
        assert_eq!(
            r.len(),
            3,
            "an idempotent re-author must not clear the ring"
        );
    }

    #[test]
    fn clearing_a_ring_drops_records_but_keeps_capacity() {
        let mut r: BoundedRing<i32> = BoundedRing::new(2);
        r.push(1);
        r.push(2);
        r.clear();
        assert!(r.is_empty());
        assert_eq!(r.capacity(), 2);
    }

    #[test]
    fn a_default_ring_retains_nothing_until_a_capacity_is_authored() {
        // The recorder builds records with an explicit capacity; a `default()`
        // ring is the degenerate zero-length one, matching `BoundedHistory`.
        let mut r: BoundedRing<i32> = BoundedRing::default();
        assert_eq!(r.capacity(), 0);
        r.push(1);
        assert!(r.is_empty());
        r.set_capacity(2);
        r.push(1);
        r.push(2);
        assert_eq!(r.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
    }
}
