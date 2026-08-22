//! Pure math behind the automatic LOD switch-range tuner (`tune-lods` bin).
//!
//! The tuner renders each adjacent pair of LOD levels (fine A, coarse B) at a
//! swept series of camera distances and asks: how different do A and B look on
//! screen from here? Two pure functions answer that, and both live here — free
//! of Bevy, the GPU and the `capture` feature — so the *decisions* the tuner
//! makes are unit-testable without a render pass:
//!
//!   * [`image_diff_rms`] — an alpha-aware image difference. The capture target
//!     is transparent (background alpha 0), so a silhouette that A draws and B
//!     does not must register as a large difference, not be averaged away
//!     against a black background. Premultiplied-alpha RMS does exactly that.
//!   * [`find_knee`] — the knee of the difference-vs-distance curve. The curve
//!     falls monotonically-ish (near: A and B differ a lot; far: both shrink to
//!     a few pixels and the difference vanishes), so its literal minimum is at
//!     infinity. The *knee* — the diminishing-returns point past which keeping
//!     the expensive fine level buys almost nothing — is where the switch
//!     boundary belongs.
//!
//! Both are deliberately small and swappable: the metric is one function and the
//! knee rule is another, so a better one drops in without touching the render
//! plumbing.
//!
//! Offline tuning math, run at build time by the `tune-lods` bin, never in the
//! shipped simulation — so platform-varying std transcendentals are fine here
//! (issue #908, simmath.rs; same opt-out as src/viewer/camera.rs).
#![allow(clippy::disallowed_methods)]

/// Alpha-aware RMS difference between two RGBA8 images of the same dimensions,
/// normalised to `0.0..=1.0`.
///
/// Both images are premultiplied by their own alpha before differencing, so a
/// transparent pixel (alpha 0) contributes zero colour regardless of its RGB.
/// This is what makes the metric *silhouette-aware*: where A is opaque hull and
/// B is transparent background (or vice versa), the premultiplied channels
/// differ by the full colour, so a lost turret or a shrunken outline dominates
/// the score — exactly the differences a LOD switch must not happen too early
/// for. Shading differences inside the shared silhouette still count, weighted
/// naturally by how opaque both sides are.
///
/// All four channels (including alpha itself) enter the sum, so a difference in
/// coverage alone — same colour, different opacity — is still seen. Returns
/// `0.0` for empty or mismatched-length inputs (the caller treats an
/// unmeasurable pair as "no difference", which for tuning means "safe to
/// switch").
pub fn image_diff_rms(a: &[u8], b: &[u8]) -> f64 {
    if a.is_empty() || a.len() != b.len() || !a.len().is_multiple_of(4) {
        return 0.0;
    }
    let mut sum_sq = 0.0f64;
    let pixels = a.len() / 4;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let aa = pa[3] as f64 / 255.0;
        let ab = pb[3] as f64 / 255.0;
        // Premultiplied RGB: transparent background contributes nothing.
        for c in 0..3 {
            let ca = (pa[c] as f64 / 255.0) * aa;
            let cb = (pb[c] as f64 / 255.0) * ab;
            let d = ca - cb;
            sum_sq += d * d;
        }
        // Coverage difference in its own right.
        let da = aa - ab;
        sum_sq += da * da;
    }
    // Four channels per pixel — normalise so a fully-opaque-white vs fully
    // transparent field scores 1.0.
    (sum_sq / (pixels as f64 * 4.0)).sqrt()
}

/// The knee index of a curve given as parallel `xs`/`ys` samples, by the
/// Kneedle "maximum distance to the chord" rule.
///
/// `xs` must be sorted ascending (the tuner passes `ln(distance)`, so the knee
/// is judged in the log-distance space the sweep is spaced in). Both axes are
/// min-max normalised to `[0, 1]` before the chord is drawn from the first to
/// the last sample, so the two very different units (log distance vs. RMS
/// difference) are comparable. The returned index is the sample farthest from
/// that chord — the elbow of the curve.
///
/// Returns `None` for fewer than three samples (a knee needs an interior
/// point), or when every sample lies on the chord (a straight line has no
/// knee).
pub fn find_knee(xs: &[f64], ys: &[f64]) -> Option<usize> {
    let n = xs.len();
    if n < 3 || ys.len() != n {
        return None;
    }
    let (x0, xn) = (xs[0], xs[n - 1]);
    let (mut y_lo, mut y_hi) = (f64::MAX, f64::MIN);
    for &y in ys {
        y_lo = y_lo.min(y);
        y_hi = y_hi.max(y);
    }
    let x_span = xn - x0;
    let y_span = y_hi - y_lo;
    if x_span <= 0.0 || y_span <= 0.0 {
        return None;
    }
    // Normalised endpoints of the chord.
    let (nx0, ny0) = (0.0f64, (ys[0] - y_lo) / y_span);
    let (nxn, nyn) = (1.0f64, (ys[n - 1] - y_lo) / y_span);
    let (dx, dy) = (nxn - nx0, nyn - ny0);
    let chord_len = (dx * dx + dy * dy).sqrt();
    if chord_len <= 0.0 {
        return None;
    }

    let mut best_idx = 0usize;
    let mut best_dist = 0.0f64;
    for i in 1..n - 1 {
        let nx = (xs[i] - x0) / x_span;
        let ny = (ys[i] - y_lo) / y_span;
        // Perpendicular distance from (nx, ny) to the chord line.
        let dist = ((nx - nx0) * dy - (ny - ny0) * dx).abs() / chord_len;
        if dist > best_dist {
            best_dist = dist;
            best_idx = i;
        }
    }
    if best_idx == 0 {
        None
    } else {
        Some(best_idx)
    }
}

/// The knee index of an INCREASING cost curve — render-diff versus decimation
/// aggressiveness, where cutting more mesh only ever raises the diff.
///
/// This is the mirror of [`find_knee`], for the decimation tuner rather than the
/// range tuner. There the curve *falls* near→far and the knee is a
/// diminishing-returns elbow; here the curve *rises* as each candidate is
/// decimated harder, and the knee is the diminishing-*headroom* elbow — the
/// most-aggressive candidate before the diff takes off. The two shapes need two
/// rules: [`find_knee`] judges the bend by unsigned distance to the chord, which
/// on a rising convex curve would also flag any single sample poking ABOVE the
/// chord (a noisy dip toward the base). A cost curve that accelerates is convex
/// and its elbow sits BELOW the chord from the first to the last sample, so this
/// variant picks the sample of maximum drop below that chord and a spurious
/// upward blip can never be mistaken for the knee.
///
/// `xs` must be sorted ascending (the tuner passes the candidates in light→heavy
/// order, spaced however the driver chose). Both axes are min-max normalised
/// before the chord is drawn, so decimation aggressiveness and RMS difference
/// are comparable. The returned index is the candidate to choose — the most
/// aggressive simplification still perceptually close to the base.
///
/// Returns `None` for fewer than three samples, or when no interior sample lies
/// below the chord (a straight or concave rise has no such elbow — the caller
/// then keeps the authored parameters rather than guessing).
pub fn find_knee_increasing(xs: &[f64], ys: &[f64]) -> Option<usize> {
    let n = xs.len();
    if n < 3 || ys.len() != n {
        return None;
    }
    let (x0, xn) = (xs[0], xs[n - 1]);
    let (mut y_lo, mut y_hi) = (f64::MAX, f64::MIN);
    for &y in ys {
        y_lo = y_lo.min(y);
        y_hi = y_hi.max(y);
    }
    let x_span = xn - x0;
    let y_span = y_hi - y_lo;
    if x_span <= 0.0 || y_span <= 0.0 {
        return None;
    }
    // Normalised chord endpoints; the chord's height at any x is a lerp between
    // them, and the elbow is where the curve drops farthest below it.
    let ny0 = (ys[0] - y_lo) / y_span;
    let nyn = (ys[n - 1] - y_lo) / y_span;

    let mut best_idx = 0usize;
    let mut best_drop = 0.0f64;
    for i in 1..n - 1 {
        let nx = (xs[i] - x0) / x_span;
        let ny = (ys[i] - y_lo) / y_span;
        let chord_y = ny0 + (nyn - ny0) * nx;
        let drop = chord_y - ny;
        if drop > best_drop {
            best_drop = drop;
            best_idx = i;
        }
    }
    if best_idx == 0 {
        None
    } else {
        Some(best_idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_zero_difference() {
        let img = vec![10u8, 20, 30, 255, 40, 50, 60, 128];
        assert_eq!(image_diff_rms(&img, &img), 0.0);
    }

    #[test]
    fn transparent_rgb_is_ignored() {
        // Same alpha (0), wildly different RGB — premultiplied, both are zero.
        let a = vec![255u8, 0, 0, 0];
        let b = vec![0u8, 255, 0, 0];
        assert_eq!(image_diff_rms(&a, &b), 0.0);
    }

    #[test]
    fn silhouette_presence_dominates() {
        // A draws an opaque white pixel; B leaves it transparent. This is the
        // "A has a feature B lost" case, and it must score near the maximum.
        let a = vec![255u8, 255, 255, 255];
        let b = vec![0u8, 0, 0, 0];
        let d = image_diff_rms(&a, &b);
        assert!(
            d > 0.99,
            "a full silhouette mismatch should score ~1.0, got {d}"
        );
    }

    #[test]
    fn coverage_difference_alone_is_seen() {
        // Same RGB, different alpha: premultiplied colour differs AND the alpha
        // channel differs, so a partial-opacity change is not invisible.
        let a = vec![200u8, 200, 200, 255];
        let b = vec![200u8, 200, 200, 0];
        assert!(image_diff_rms(&a, &b) > 0.0);
    }

    #[test]
    fn mismatched_or_empty_inputs_are_zero() {
        assert_eq!(image_diff_rms(&[], &[]), 0.0);
        assert_eq!(image_diff_rms(&[1, 2, 3, 4], &[1, 2, 3]), 0.0);
    }

    #[test]
    fn knee_needs_three_points() {
        assert_eq!(find_knee(&[0.0, 1.0], &[1.0, 0.0]), None);
    }

    #[test]
    fn straight_line_has_no_knee() {
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0];
        let ys = [4.0, 3.0, 2.0, 1.0, 0.0];
        assert_eq!(find_knee(&xs, &ys), None);
    }

    #[test]
    fn finds_the_elbow_of_a_diminishing_returns_curve() {
        // A sharp early drop then a long flat tail: the knee is at the bend,
        // not at the ends and not in the flat tail.
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let ys = [1.0, 0.55, 0.25, 0.1, 0.06, 0.03, 0.0];
        let knee = find_knee(&xs, &ys).expect("a bent curve has a knee");
        assert!(
            (2..=3).contains(&knee),
            "knee should sit at the bend (index 2–3), got {knee}"
        );
    }

    #[test]
    fn knee_is_scale_invariant_in_x() {
        // The same curve shape sampled on log-spaced x still finds the bend.
        let xs: Vec<f64> = [2.0, 5.0, 12.0, 30.0, 80.0, 200.0, 500.0]
            .iter()
            .map(|d: &f64| d.ln())
            .collect();
        let ys = [1.0, 0.6, 0.3, 0.12, 0.06, 0.02, 0.0];
        let knee = find_knee(&xs, &ys).expect("knee exists");
        assert!(knee >= 1 && knee < xs.len() - 1);
    }

    // ── find_knee_increasing: the decimation cost curve ─────────────────────

    #[test]
    fn increasing_knee_needs_three_points() {
        assert_eq!(find_knee_increasing(&[0.0, 1.0], &[0.0, 1.0]), None);
    }

    #[test]
    fn increasing_straight_line_has_no_knee() {
        // A linear rise is all cost, no elbow — keep the authored parameters.
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0];
        let ys = [0.0, 1.0, 2.0, 3.0, 4.0];
        assert_eq!(find_knee_increasing(&xs, &ys), None);
    }

    #[test]
    fn increasing_concave_rise_has_no_knee() {
        // A curve that rises fast then flattens bulges ABOVE the chord — that is
        // the diminishing-returns shape `find_knee` owns, not a cost elbow, so
        // the increasing rule declines it rather than picking the wrong side.
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let ys = [0.0, 0.6, 0.85, 0.94, 0.97, 0.99, 1.0];
        assert_eq!(find_knee_increasing(&xs, &ys), None);
    }

    #[test]
    fn finds_the_elbow_of_an_accelerating_cost_curve() {
        // Decimation-shaped: the diff stays flat and low while the mesh can be
        // cut for free, then climbs sharply once detail starts to go. The knee
        // is the last aggressive candidate before the climb, not in the flat run
        // and not at the runaway end.
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let ys = [0.0, 0.01, 0.03, 0.08, 0.25, 0.55, 1.0];
        let knee = find_knee_increasing(&xs, &ys).expect("a convex rise has an elbow");
        assert!(
            (3..=4).contains(&knee),
            "knee should sit at the bend (index 3–4), got {knee}"
        );
    }

    #[test]
    fn increasing_knee_ignores_a_spurious_dip_below_the_base() {
        // A single candidate that renders momentarily CLOSER to the base than the
        // one before it (an upward blip toward the chord) must not be read as the
        // knee: the elbow is still the point that drops farthest below the chord.
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let ys = [0.0, 0.02, 0.05, 0.10, 0.40, 0.90];
        let knee = find_knee_increasing(&xs, &ys).expect("elbow exists");
        assert!(
            (2..=3).contains(&knee),
            "expected the convex bend, got {knee}"
        );
    }

    #[test]
    fn increasing_knee_is_scale_invariant_in_x() {
        // Candidates spaced by log-ratio rather than by index still find the bend.
        let xs: Vec<f64> = [0.95f64, 0.6, 0.3, 0.15, 0.06, 0.02]
            .iter()
            .map(|r| -r.ln())
            .collect();
        let ys = [0.0, 0.02, 0.05, 0.09, 0.3, 0.8];
        let knee = find_knee_increasing(&xs, &ys).expect("elbow exists");
        assert!(knee >= 1 && knee < xs.len() - 1);
    }
}
