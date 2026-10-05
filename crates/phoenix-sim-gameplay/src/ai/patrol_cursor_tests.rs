use super::*;

fn make_anchors(pairs: &[(&str, [f32; 3])]) -> HashMap<String, [f32; 3]> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn route(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// A cursor parked at `index` on the test objective.
fn cursor_at(index: usize) -> PatrolCursor {
    PatrolCursor {
        objective_id: "obj".to_string(),
        index,
        settled: false,
    }
}

// ── Test 1: Empty waypoints list ───────────────────────────────────────

#[test]
fn empty_waypoints_returns_stalled() {
    let anchors = make_anchors(&[]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(&mut cursor, &[], false, [0.0, 0.0, 0.0], &anchors, 20.0);
    assert_eq!(cursor.index(), 0);
    assert!(reached.is_empty());
}

// ── Test 2: Non-looping, index past end ────────────────────────────────

#[test]
fn non_looping_index_past_end_returns_terminal() {
    let waypoints = route(&["a", "b", "c"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(5);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 5);
    assert!(reached.is_empty());
}

// ── Test 3: Looping, index past end wraps to 0 ────────────────────────

#[test]
fn looping_index_past_end_wraps_to_first() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0]), ("b", [200.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(5);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0);
    assert!(reached.is_empty(), "wrapping is not an arrival");
}

// ── Test 4: Not at waypoint (far away) ────────────────────────────────

#[test]
fn far_from_waypoint_holds_cursor_and_reports_no_arrival() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0);
    assert!(reached.is_empty());
}

// ── Test 5: Arrived at waypoint, advance to next ──────────────────────

#[test]
fn arrived_at_waypoint_advances_to_next() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 1);
    assert_eq!(reached, route(&["a"]));
}

// ── Test 6: Arrived at final waypoint, non-looping ────────────────────

#[test]
fn arrived_at_final_waypoint_non_looping_stops() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 1);
    assert_eq!(reached, route(&["a"]));
    // The terminal cursor stays terminal and stops announcing.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 1);
    assert!(reached.is_empty());
}

// ── Test 7: Arrived at final waypoint, looping ────────────────────────

#[test]
fn arrived_at_only_waypoint_of_looping_route_announces_once_then_settles() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
    // Sitting on the single waypoint of a looping route: the lap closes
    // with nowhere to steer, so `a` is announced once and the cursor
    // settles instead of re-announcing `a` on every subsequent call.
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [100.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0, "the cursor keeps a real waypoint index");
    assert!(cursor.settled());
    assert_eq!(reached, route(&["a"]));

    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [100.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert!(cursor.settled(), "still nowhere to steer → still settled");
    assert!(reached.is_empty(), "a settled cursor announces nothing");
}

/// A looping route only closes its lap when there is genuinely nowhere to
/// steer: arriving at the last waypoint of a normal loop wraps the cursor
/// back to the first and keeps patrolling.
#[test]
fn arrived_at_final_waypoint_of_normal_looping_route_wraps_to_first() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    // Sitting on `b` (the last waypoint); `a` is 100 units away.
    let mut cursor = cursor_at(1);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [100.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0, "cursor wraps back to the first waypoint");
    assert!(!cursor.settled());
    assert_eq!(reached, route(&["b"]));
}

// ── Test 8: Arrived at each of 3 waypoints in sequence ────────────────

#[test]
fn arrived_at_three_waypoints_sequential_non_looping() {
    let waypoints = route(&["a", "b", "c"]);
    let anchors = make_anchors(&[
        ("a", [0.0, 0.0, 0.0]),
        ("b", [100.0, 0.0, 0.0]),
        ("c", [200.0, 0.0, 0.0]),
    ]);
    let mut cursor = cursor_at(0);

    // At a → advance to b
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 1, "should advance past a");
    assert_eq!(reached, route(&["a"]));

    // At b → advance to c
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [100.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 2, "should advance past b");
    assert_eq!(reached, route(&["b"]));

    // At c → terminal stop
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [200.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 3, "should advance past c");
    assert_eq!(reached, route(&["c"]));
}

// ── Test 9: Missing anchor — skip to next valid ───────────────────────

#[test]
fn missing_anchor_skips_to_next_valid() {
    let waypoints = route(&["missing", "valid"]);
    let anchors = make_anchors(&[("valid", [100.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 1);
    assert!(
        reached.is_empty(),
        "a waypoint skipped for an unknown anchor was never reached"
    );
}

// ── Test 10: Missing anchor on only waypoint ──────────────────────────

#[test]
fn missing_anchor_on_only_waypoint_terminates() {
    let waypoints = route(&["missing"]);
    let anchors = make_anchors(&[]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 1);
    assert!(reached.is_empty());
}

// ── Test 11: Position outside arrival radius ──────────────────────────

#[test]
fn outside_arrival_radius_does_not_advance() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
    // Entity at (50, 0, 0) → 50 units away, arrival_radius = 20 → not arrived
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [50.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0);
    assert!(reached.is_empty());
}

// ── Test 12: Zero arrival radius (must be exact) ──────────────────────

#[test]
fn zero_arrival_radius_requires_exact_position() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
    // 1 unit away → not arrived with radius 0
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [99.0, 0.0, 0.0],
        &anchors,
        0.0,
    );
    assert_eq!(cursor.index(), 0);
    assert!(reached.is_empty());

    // Exactly at waypoint → arrived
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [100.0, 0.0, 0.0],
        &anchors,
        0.0,
    );
    assert_eq!(cursor.index(), 1);
    assert_eq!(reached, route(&["a"]));
}

// ── Test 13: Multiple advances in one call (consecutive close waypoints) ─

#[test]
fn multiple_advances_in_one_call_report_every_waypoint_consumed() {
    // Three waypoints all within arrival radius of entity position
    let waypoints = route(&["a", "b", "c"]);
    let anchors = make_anchors(&[
        ("a", [0.0, 0.0, 0.0]),
        ("b", [1.0, 0.0, 0.0]),
        ("c", [5.0, 0.0, 0.0]),
    ]);
    // Entity at (0,0,0), radius 20 → all three waypoints within radius
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 3);
    assert_eq!(
        reached,
        route(&["a", "b", "c"]),
        "every waypoint the cursor stepped past must be reported, not just the first"
    );
}

/// Waypoints spaced closer than the arrival radius: the cursor jumps from
/// `a` straight to `c`, and the intermediate `b` must still be reported —
/// a trigger keyed to `b` would otherwise silently never fire.
#[test]
fn intermediate_waypoint_inside_radius_is_still_reported() {
    let waypoints = route(&["a", "b", "c"]);
    let anchors = make_anchors(&[
        ("a", [0.0, 0.0, 0.0]),
        ("b", [5.0, 0.0, 0.0]),
        ("c", [200.0, 0.0, 0.0]),
    ]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(
        cursor.index(),
        2,
        "cursor skips a and b, landing on the distant c"
    );
    assert_eq!(reached, route(&["a", "b"]));
}

#[test]
fn multiple_advances_in_one_call_looping_closes_the_lap_and_settles() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [1.0, 0.0, 0.0])]);
    // Both within radius → a full lap is consumed with nowhere left to
    // steer, so both are announced once and the cursor settles.
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0);
    assert!(cursor.settled());
    assert_eq!(reached, route(&["a", "b"]));

    // Crucially it does not re-announce the lap on every later call.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert!(cursor.settled());
    assert!(reached.is_empty());
}

/// Regression (issue #696 review): settling must never be permanent. A
/// looping route whose legs are shorter than the authored arrival radius
/// closes its lap immediately — but the moment the entity is outside the
/// radius (knockback, tow, scenario teleport, drift) the route must resume
/// and be flown again, announcing arrivals as before.
#[test]
fn settled_route_resumes_once_the_entity_leaves_the_arrival_radius() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [5.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);

    // Legs (5 units) are far shorter than the radius (150): the lap closes.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        150.0,
    );
    assert_eq!(reached, route(&["a", "b"]));
    assert!(cursor.settled());
    assert_eq!(cursor.index(), 0, "the cursor keeps a real waypoint index");

    // Shoved 2000 units out: there is somewhere to steer again, so the
    // cursor un-settles and holds `a` to fly back to. Nothing is
    // announced — it has not arrived anywhere.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [2000.0, 0.0, 0.0],
        &anchors,
        150.0,
    );
    assert!(!cursor.settled(), "the route must resume, not stay dead");
    assert_eq!(cursor.index(), 0, "steering back to `a`");
    assert!(reached.is_empty());

    // It is steering at a real waypoint again, which is what the low-LOD
    // path needs to stop it flying off forever.
    assert_eq!(
        cursor_target(cursor.index(), &waypoints, true, &anchors),
        Some([0.0, 0.0, 0.0])
    );

    // Back in the cluster: the lap is flown and announced afresh.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        150.0,
    );
    assert_eq!(
        reached,
        route(&["a", "b"]),
        "a resumed route announces its arrivals again"
    );
    assert!(cursor.settled());
}

/// A settled cursor holds station on its route rather than losing it: it
/// still names a waypoint to steer at, so the low-LOD path never falls
/// through to the dumb forward-move that flies the ship out of the cluster.
#[test]
fn settled_cursor_still_names_a_target_to_steer_at() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [5.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);
    advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        150.0,
    );
    assert!(cursor.settled());
    assert_eq!(
        cursor_target(cursor.index(), &waypoints, true, &anchors),
        Some([0.0, 0.0, 0.0]),
        "a settled cursor still steers at its waypoint"
    );
    assert_eq!(
        arrived_waypoint(
            cursor.index(),
            &waypoints,
            true,
            [0.0, 0.0, 0.0],
            &anchors,
            150.0
        ),
        Some("a".to_string()),
        "and its waypoint is still a real, reachable one"
    );
}

// ── Test 14: Independence for multiple objectives ─────────────────────

#[test]
fn advancement_independence_for_multiple_objectives() {
    // Two objective states: one at index 0 (far from waypoint), one at index 0
    let waypoints_a = route(&["wp_a"]);
    let waypoints_b = route(&["wp_b"]);
    let anchors = make_anchors(&[("wp_a", [100.0, 0.0, 0.0]), ("wp_b", [0.0, 0.0, 0.0])]);

    // Advance cursor A (far away)
    let mut cursor_a = cursor_at(0);
    let reached_a = advance_cursor(
        &mut cursor_a,
        &waypoints_a,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor_a.index(), 0, "A should not advance");
    assert!(reached_a.is_empty());

    // Advance cursor B (arrived)
    let mut cursor_b = cursor_at(0);
    let reached_b = advance_cursor(
        &mut cursor_b,
        &waypoints_b,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor_b.index(), 1, "B should advance to terminal");
    assert_eq!(reached_b, route(&["wp_b"]));

    // A's state is unchanged
    let reached_a2 = advance_cursor(
        &mut cursor_a,
        &waypoints_a,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor_a.index(), 0, "A should still be at index 0");
    assert!(reached_a2.is_empty());
}

// ── Additional edge cases ─────────────────────────────────────────────

#[test]
fn looping_with_skip_past_end_wraps_and_finds_valid() {
    let waypoints = route(&["missing", "a"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
    // Start at index past end (2), loop wraps to 0, skip missing, land on a
    let mut cursor = cursor_at(2);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 1);
    assert!(reached.is_empty());
}

#[test]
fn all_anchors_missing_non_looping_terminates() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 2);
    assert!(reached.is_empty());
}

/// Every anchor unknown on a looping route: there is nowhere to steer from
/// *any* position, so the lap closes and the cursor settles for good. It
/// keeps a valid index, announces nothing (an unreachable waypoint was
/// never reached), and does not re-walk the route on later calls.
#[test]
fn all_anchors_missing_looping_terminates_by_settling() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0, "the index stays a real index");
    assert!(cursor.settled());
    assert!(
        reached.is_empty(),
        "unreachable waypoints are never reached"
    );

    // Moving does not help — no position makes an unknown anchor known.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [9000.0, 0.0, 9000.0],
        &anchors,
        20.0,
    );
    assert!(cursor.settled());
    assert!(reached.is_empty());
}

/// One known anchor among unknown ones is enough to resume a settled
/// route: the unknown waypoints are skipped and the cursor lands on the
/// waypoint it can actually fly to.
#[test]
fn settled_route_with_one_known_anchor_resumes_when_the_entity_moves_away() {
    let waypoints = route(&["missing", "b"]);
    let anchors = make_anchors(&[("b", [0.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);

    // Sitting on `b`: `missing` is skipped, `b` is reached, the lap closes.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(reached, route(&["b"]));
    assert!(cursor.settled());

    // Shoved away from `b` → resumes, skips `missing`, steers at `b`.
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [500.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert!(!cursor.settled());
    assert_eq!(cursor.index(), 1, "cursor lands on the flyable waypoint");
    assert!(reached.is_empty());
}

#[test]
fn arrived_missing_anchor_sequence_skips_and_advances() {
    let waypoints = route(&["a", "missing", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    // Arrived at a (idx→1), skip missing (idx→2), found b at idx 2
    assert_eq!(cursor.index(), 2);
    assert_eq!(reached, route(&["a"]), "only `a` was actually reached");
}

#[test]
fn y_component_affects_arrival() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [0.0, 10.0, 0.0])]);
    // Entity at (0,0,0) → distance = 10, arrival_radius = 5 → not arrived
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        5.0,
    );
    assert_eq!(cursor.index(), 0);
    assert!(reached.is_empty());

    // Entity at (0,9,0) → distance = 1, arrival_radius = 5 → arrived
    let mut cursor = cursor_at(0);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 9.0, 0.0],
        &anchors,
        5.0,
    );
    assert_eq!(cursor.index(), 1);
    assert_eq!(reached, route(&["a"]));
}

#[test]
fn arrived_at_final_waypoint_non_looping_reaches_terminal_index() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    // Entity at b (100,0,0), arrived at b, non-looping → terminal
    let mut cursor = cursor_at(1);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [100.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 2);
    assert_eq!(reached, route(&["b"]));
}

#[test]
fn already_at_index_past_end_looping_wraps_without_announcing() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
    // Index is 1, waypoints.len() = 1, looping → wrap to 0, far from waypoint
    let mut cursor = cursor_at(1);
    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        true,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );
    assert_eq!(cursor.index(), 0);
    assert!(reached.is_empty());
}

// ── route_completed ───────────────────────────────────────────────────

#[test]
fn route_completed_only_for_a_non_looping_route_past_its_end() {
    let waypoints = route(&["a", "b"]);
    assert!(
        !route_completed(0, &waypoints, false),
        "still flying to `a`"
    );
    assert!(
        !route_completed(1, &waypoints, false),
        "still flying to `b`"
    );
    assert!(route_completed(2, &waypoints, false), "flown to the end");
}

#[test]
fn route_completed_is_false_for_a_looping_or_empty_route() {
    let waypoints = route(&["a", "b"]);
    assert!(
        !route_completed(2, &waypoints, true),
        "a looping route wraps rather than finishing"
    );
    assert!(
        !route_completed(0, &[], false),
        "an empty route was never flown, so it is not finished"
    );
}

/// The distinction the caller needs, and its limit. An unknown anchor also
/// makes `cursor_target` `None`, but on the tick it is first seen the route
/// is not finished — the cursor is about to skip past it.
///
/// One skip later the guarantee is gone: a one-waypoint non-looping route
/// whose anchor is unknown reads as *finished*, and the caller parks a ship
/// that never went anywhere. Pinned here so the doc on `route_completed`
/// cannot drift into claiming the split survives advancement.
#[test]
fn route_with_an_unknown_anchor_is_not_completed_only_until_the_skip() {
    let waypoints = route(&["missing"]);
    let anchors = make_anchors(&[]);
    let mut cursor = cursor_at(0);

    assert_eq!(cursor_target(0, &waypoints, false, &anchors), None);
    assert!(
        !route_completed(cursor.index(), &waypoints, false),
        "on the tick the unknown anchor is first seen, the route is unflyable, \
             not finished"
    );

    let reached = advance_cursor(
        &mut cursor,
        &waypoints,
        false,
        [0.0, 0.0, 0.0],
        &anchors,
        20.0,
    );

    assert!(
        reached.is_empty(),
        "an unreachable waypoint is never announced as reached"
    );
    assert_eq!(
        cursor.index(),
        waypoints.len(),
        "the cursor steps past the unknown anchor and off the end"
    );
    assert!(
        route_completed(cursor.index(), &waypoints, false),
        "one skip later the same unflyable route reads as finished — the entity is \
             classified as arrived at a place that does not exist"
    );
}

// ── cursor_target ─────────────────────────────────────────────────────

#[test]
fn cursor_target_returns_current_waypoint_without_advancing() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    // Sitting exactly on `a` — `advance_cursor` would move on to `b`, but
    // `cursor_target` reports the cursor's *current* waypoint unchanged.
    assert_eq!(
        cursor_target(0, &waypoints, false, &anchors),
        Some([0.0, 0.0, 0.0])
    );
}

#[test]
fn cursor_target_wraps_past_end_when_looping() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    assert_eq!(
        cursor_target(2, &waypoints, true, &anchors),
        Some([0.0, 0.0, 0.0])
    );
}

#[test]
fn cursor_target_none_past_end_when_not_looping() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0])]);
    assert_eq!(cursor_target(1, &waypoints, false, &anchors), None);
}

#[test]
fn cursor_target_none_for_empty_route_or_missing_anchor() {
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0])]);
    assert_eq!(cursor_target(0, &[], true, &anchors), None);
    assert_eq!(
        cursor_target(0, &["missing".to_string()], false, &anchors),
        None
    );
}

// ── arrived_waypoint ──────────────────────────────────────────────────

#[test]
fn arrived_waypoint_names_the_reached_waypoint() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    assert_eq!(
        arrived_waypoint(0, &waypoints, false, [5.0, 0.0, 0.0], &anchors, 20.0),
        Some("a".to_string())
    );
}

#[test]
fn arrived_waypoint_none_when_outside_radius() {
    let waypoints = route(&["a"]);
    let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
    assert_eq!(
        arrived_waypoint(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0),
        None
    );
}

#[test]
fn arrived_waypoint_reports_wrapped_first_waypoint_when_looping() {
    let waypoints = route(&["a", "b"]);
    let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
    // Index past the end on a looping route wraps to `a`, which we sit on.
    assert_eq!(
        arrived_waypoint(2, &waypoints, true, [0.0, 0.0, 0.0], &anchors, 20.0),
        Some("a".to_string())
    );
}

#[test]
fn arrived_waypoint_none_for_missing_anchor_or_terminal_route() {
    let anchors = make_anchors(&[]);
    assert_eq!(
        arrived_waypoint(
            0,
            &["missing".to_string()],
            false,
            [0.0, 0.0, 0.0],
            &anchors,
            20.0
        ),
        None,
        "a waypoint whose anchor is unknown is never 'reached'"
    );
    assert_eq!(
        arrived_waypoint(
            1,
            &["a".to_string()],
            false,
            [0.0, 0.0, 0.0],
            &make_anchors(&[("a", [0.0, 0.0, 0.0])]),
            20.0
        ),
        None,
        "a finished non-looping route has no current waypoint"
    );
}
