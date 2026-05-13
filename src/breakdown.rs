use crate::messages::{Console, Shape};
use rand::Rng;

/// All consoles that can receive a breakdown assignment.
const ALL_CONSOLES: [Console; 5] = [
    Console::CaptainChair,
    Console::Helm,
    Console::Tactical,
    Console::Repair,
    Console::Power,
];

/// One entry in the breakdown queue: a console that must perform a repair
/// and a randomly-assigned shape for the repair mini-game.
#[derive(Debug, Clone, PartialEq)]
pub struct BreakdownEntry {
    pub console: Console,
    pub shape: Shape,
}

/// FIFO queue of breakdown entries.
///
/// Each entry names the console whose player must perform the repair action
/// and carries a shape for the mini-game.
/// The front of the queue is the *active* (authorized) repair target.
#[derive(Debug, Clone, Default)]
pub struct BreakdownQueue {
    queue: std::collections::VecDeque<BreakdownEntry>,
    last_picked: Option<Console>,
}

impl BreakdownQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pick a random console (never the same as the previous pick), assign a
    /// random shape, and push the entry to the back of the queue.
    ///
    /// Uses `rng` so callers can supply a seeded generator for deterministic tests.
    pub fn push_random<R: Rng>(&mut self, rng: &mut R) {
        let candidates: Vec<&Console> = ALL_CONSOLES
            .iter()
            .filter(|c| Some(*c) != self.last_picked.as_ref())
            .collect();
        let idx = rng.random_range(0..candidates.len());
        let console = candidates[idx].clone();
        let shape = match rng.random_range(0..3) {
            0 => Shape::Square,
            1 => Shape::Triangle,
            _ => Shape::Circle,
        };
        self.last_picked = Some(console.clone());
        self.queue.push_back(BreakdownEntry { console, shape });
    }

    /// The current authorized repair target — the front of the queue.
    pub fn front(&self) -> Option<&BreakdownEntry> {
        self.queue.front()
    }

    /// Remove the front entry (called after a successful repair).
    pub fn pop_front(&mut self) -> Option<BreakdownEntry> {
        self.queue.pop_front()
    }

    /// Number of pending breakdowns.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Compute how many new breakdowns to enqueue from a damage event.
///
/// Tracks "every full 10 HP of *cumulative* damage taken = one breakdown".
/// `damage_taken_before` is total damage accumulated before this event;
/// `damage_taken_after` is the total after. Returns the number of new complete
/// 10-HP buckets crossed.
///
/// Example: 0→9 = 0, 0→10 = 1, 9→25 = 1 (crosses only the 10 boundary),
/// 0→25 = 2 (crosses 10 and 20).
pub fn breakdowns_from_damage(damage_taken_before: i32, damage_taken_after: i32) -> u32 {
    if damage_taken_after <= damage_taken_before {
        return 0;
    }
    let buckets_before = damage_taken_before / 10;
    let buckets_after = damage_taken_after / 10;
    (buckets_after - buckets_before).max(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    // ── BreakdownQueue ────────────────────────────────────────────────────

    #[test]
    fn new_queue_is_empty() {
        assert!(BreakdownQueue::new().is_empty());
    }

    #[test]
    fn push_random_enqueues_one_breakdown() {
        let mut q = BreakdownQueue::new();
        let mut rng = SmallRng::seed_from_u64(0);
        q.push_random(&mut rng);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn front_returns_first_entry() {
        let mut q = BreakdownQueue::new();
        let mut rng = SmallRng::seed_from_u64(0);
        q.push_random(&mut rng);
        let entry = q.front().unwrap();
        // Entry carries both console and shape.
        let _console = &entry.console;
        let _shape = entry.shape;
    }

    #[test]
    fn pop_front_removes_first_entry_and_exposes_next() {
        let mut q = BreakdownQueue::new();
        let mut rng = SmallRng::seed_from_u64(42);
        q.push_random(&mut rng);
        q.push_random(&mut rng);
        let first = q.front().unwrap().clone();
        q.pop_front();
        let second = q.front().unwrap().clone();
        assert_ne!(first, second, "second entry should differ from first");
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn push_random_assigns_one_of_three_shapes() {
        let mut q = BreakdownQueue::new();
        let mut rng = SmallRng::seed_from_u64(0);
        let mut seen = std::collections::HashSet::new();
        // Push enough entries to see all three shapes.
        for _ in 0..30 {
            q.push_random(&mut rng);
            let entry = q.queue.back().unwrap();
            seen.insert(entry.shape);
        }
        assert!(seen.contains(&Shape::Square), "should see Square");
        assert!(seen.contains(&Shape::Triangle), "should see Triangle");
        assert!(seen.contains(&Shape::Circle), "should see Circle");
    }

    #[test]
    fn shape_stays_same_across_reads() {
        let mut q = BreakdownQueue::new();
        let mut rng = SmallRng::seed_from_u64(42);
        q.push_random(&mut rng);
        let s1 = q.front().unwrap().shape;
        let s2 = q.front().unwrap().shape;
        assert_eq!(s1, s2, "shape must not change between reads");
    }

    #[test]
    fn pop_front_returns_entry() {
        let mut q = BreakdownQueue::new();
        let mut rng = SmallRng::seed_from_u64(0);
        q.push_random(&mut rng);
        let entry = q.pop_front();
        assert!(entry.is_some());
        assert!(q.is_empty());
    }

    #[test]
    fn picker_never_returns_same_console_twice_in_a_row() {
        let mut q = BreakdownQueue::new();
        let mut rng = SmallRng::seed_from_u64(1234);
        for _ in 0..100 {
            let prev_back = q.queue.back().map(|e| e.console.clone());
            q.push_random(&mut rng);
            let new_back = q.queue.back().unwrap().console.clone();
            if let Some(prev) = prev_back {
                assert_ne!(
                    prev, new_back,
                    "picker should never repeat the same console consecutively"
                );
            }
        }
    }

    // ── breakdowns_from_damage ────────────────────────────────────────────
    // Note: arguments are *cumulative damage taken* (0 = full health, 25 = 25 HP lost)

    #[test]
    fn no_breakdowns_when_no_damage() {
        assert_eq!(breakdowns_from_damage(0, 0), 0);
    }

    #[test]
    fn damage_within_first_bucket_gives_no_breakdown() {
        // 0 → 9 damage taken: hasn't filled 10 HP bucket yet
        assert_eq!(breakdowns_from_damage(0, 9), 0);
    }

    #[test]
    fn exactly_10_damage_gives_one_breakdown() {
        assert_eq!(breakdowns_from_damage(0, 10), 1);
    }

    #[test]
    fn partial_extra_damage_does_not_add_breakdown() {
        // 9 → 15: crosses the 10-mark once
        assert_eq!(breakdowns_from_damage(9, 15), 1);
    }

    #[test]
    fn taking_25_damage_from_zero_gives_two_breakdowns() {
        // 0 → 25: completes 10-HP buckets at 10 and 20; remainder 5 not counted
        assert_eq!(breakdowns_from_damage(0, 25), 2);
    }

    #[test]
    fn crossing_two_more_buckets_mid_game() {
        // Already taken 5 damage; now take 20 more (total 25). Crosses 10 and 20.
        assert_eq!(breakdowns_from_damage(5, 25), 2);
    }
}
