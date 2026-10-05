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
//! (issue #908, simmath.rs; same opt-out as crates/phoenix-presentation/src/viewer/camera.rs).
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
#[path = "lod_tune_tests.rs"]
mod tests;
