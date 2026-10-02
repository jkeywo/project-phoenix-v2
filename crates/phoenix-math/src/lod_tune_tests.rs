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
