/// Default hull-fraction threshold below which retreat scoring begins (30%).
/// Used as the fallback when no config-provided threshold is available.
pub const DEFAULT_RETREAT_THRESHOLD: f32 = 0.3;

/// Compute a retreat utility score from the given hull fraction and threshold.
///
/// Returns a score in [0.0, 1.0]:
/// - `hull_fraction >= threshold` → 0.0 (no need to retreat)
/// - `hull_fraction < threshold` → rises linearly from 0.0 at threshold
///   to 1.0 at zero hull.
///
/// Monotonic: score strictly decreases as hull_fraction increases.
pub fn score_retreat(hull_fraction: f32, threshold: f32) -> f32 {
    if hull_fraction >= threshold || threshold <= 0.0 {
        0.0
    } else {
        let t = (threshold - hull_fraction) / threshold;
        t.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #1: Full hull (1.0) with default threshold (0.3) → 0.0
    #[test]
    fn full_hull_scores_zero() {
        let score = score_retreat(1.0, 0.3);
        assert_eq!(score, 0.0);
    }

    /// #2: Hull exactly at threshold → 0.0
    #[test]
    fn hull_at_threshold_scores_zero() {
        let score = score_retreat(0.3, 0.3);
        assert_eq!(score, 0.0);
    }

    /// #3: Hull just below threshold (0.29) → positive, > 0.0
    #[test]
    fn hull_below_threshold_scores_positive() {
        let score = score_retreat(0.29, 0.3);
        assert!(score > 0.0);
    }

    /// #4: Hull at 0.0 → 1.0
    #[test]
    fn zero_hull_scores_one() {
        let score = score_retreat(0.0, 0.3);
        assert_eq!(score, 1.0);
    }

    /// #5: Hull at 0.5 * threshold (sharp rise band) → 0.5
    #[test]
    fn hull_at_half_threshold_scores_half() {
        let score = score_retreat(0.15, 0.3);
        assert!((score - 0.5).abs() < f32::EPSILON);
    }

    /// #6: Hull at 0.1 scores higher than hull at 0.29 (monotonic near threshold)
    #[test]
    fn lower_hull_scores_higher_near_threshold() {
        let s_low = score_retreat(0.1, 0.3);
        let s_high = score_retreat(0.29, 0.3);
        assert!(s_low > s_high);
    }

    /// #7: Hull at 0.0 with threshold=0.5 → 1.0
    #[test]
    fn zero_hull_with_higher_threshold_scores_one() {
        let score = score_retreat(0.0, 0.5);
        assert_eq!(score, 1.0);
    }

    /// #8: Hull at 1.0 with threshold=0.5 → 0.0
    #[test]
    fn full_hull_with_higher_threshold_scores_zero() {
        let score = score_retreat(1.0, 0.5);
        assert_eq!(score, 0.0);
    }

    /// #9: Monotonicity check across full range
    #[test]
    fn monotonic_across_full_range() {
        let samples = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];
        let mut prev = f32::MAX;
        for &h in &samples {
            let s = score_retreat(h, 0.3);
            assert!(
                s <= prev,
                "score_retreat({}, 0.3) = {}, but previous was {} (must be monotonic)",
                h,
                s,
                prev
            );
            prev = s;
        }
    }

    /// #10: Edge: threshold=0.0 → 0.0 always
    #[test]
    fn zero_threshold_always_zero() {
        assert_eq!(score_retreat(0.0, 0.0), 0.0);
        assert_eq!(score_retreat(0.5, 0.0), 0.0);
        assert_eq!(score_retreat(1.0, 0.0), 0.0);
    }

    /// #11: Edge: hull_fraction slightly negative → clamped to 1.0
    #[test]
    fn negative_hull_scores_one() {
        let score = score_retreat(-0.1, 0.3);
        assert_eq!(score, 1.0);
    }

    /// #12: Edge: hull_fraction > 1.0 → treated as >= threshold → 0.0
    #[test]
    fn hull_above_one_scores_zero() {
        let score = score_retreat(1.5, 0.3);
        assert_eq!(score, 0.0);
    }

    /// #13: Zero hull with threshold=0.0 → 0.0
    #[test]
    fn zero_hull_with_zero_threshold_scores_zero() {
        let score = score_retreat(0.0, 0.0);
        assert_eq!(score, 0.0);
    }
}
