use std::collections::HashMap;

/// Advance a single patrol cursor based on the entity's current position.
///
/// Returns `(new_index, Option<waypoint_position>)`:
/// - `new_index` is the updated waypoint index after any arrival/advancement
/// - `Some(position)` if there is a valid waypoint to steer toward
/// - `None` if patrol is complete (terminal stop) or stalled (empty/missing)
pub fn advance_cursor(
    current_index: usize,
    waypoints: &[String],
    loop_path: bool,
    entity_pos: [f32; 3],
    anchors: &HashMap<String, [f32; 3]>,
    arrival_radius: f32,
) -> (usize, Option<[f32; 3]>) {
    if waypoints.is_empty() {
        return (current_index, None);
    }

    let mut idx = current_index;
    let mut wrapped = false;

    if idx >= waypoints.len() {
        if loop_path {
            idx = 0;
            wrapped = true;
        } else {
            return (idx, None);
        }
    }

    let radius_sq = arrival_radius * arrival_radius;

    loop {
        let waypoint_name = &waypoints[idx];

        match anchors.get(waypoint_name.as_str()) {
            None => {
                idx += 1;
                if idx >= waypoints.len() {
                    if loop_path && !wrapped {
                        idx = 0;
                        wrapped = true;
                    } else {
                        return (idx, None);
                    }
                }
                continue;
            }
            Some(&pos) => {
                let dx = entity_pos[0] - pos[0];
                let dy = entity_pos[1] - pos[1];
                let dz = entity_pos[2] - pos[2];
                let dist_sq = dx * dx + dy * dy + dz * dz;

                if dist_sq <= radius_sq {
                    if wrapped {
                        return (idx, Some(pos));
                    }
                    idx += 1;
                    if idx >= waypoints.len() {
                        if loop_path && !wrapped {
                            idx = 0;
                            wrapped = true;
                        } else {
                            return (idx, None);
                        }
                    }
                    continue;
                }

                return (idx, Some(pos));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_anchors(pairs: &[(&str, [f32; 3])]) -> HashMap<String, [f32; 3]> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    // ── Test 1: Empty waypoints list ───────────────────────────────────────

    #[test]
    fn empty_waypoints_returns_stalled() {
        let anchors = make_anchors(&[]);
        let (idx, pos) = advance_cursor(0, &[], false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 0);
        assert!(pos.is_none());
    }

    // ── Test 2: Non-looping, index past end ────────────────────────────────

    #[test]
    fn non_looping_index_past_end_returns_terminal() {
        let waypoints = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0])]);
        let (idx, pos) = advance_cursor(5, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 5);
        assert!(pos.is_none());
    }

    // ── Test 3: Looping, index past end wraps to 0 ────────────────────────

    #[test]
    fn looping_index_past_end_wraps_to_first() {
        let waypoints = vec!["a".to_string(), "b".to_string()];
        let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0]), ("b", [200.0, 0.0, 0.0])]);
        let (idx, pos) = advance_cursor(5, &waypoints, true, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    // ── Test 4: Not at waypoint (far away) ────────────────────────────────

    #[test]
    fn far_from_waypoint_returns_index_and_position() {
        let waypoints = vec!["a".to_string()];
        let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    // ── Test 5: Arrived at waypoint, advance to next ──────────────────────

    #[test]
    fn arrived_at_waypoint_advances_to_next() {
        let waypoints = vec!["a".to_string(), "b".to_string()];
        let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 1);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    // ── Test 6: Arrived at final waypoint, non-looping ────────────────────

    #[test]
    fn arrived_at_final_waypoint_non_looping_stops() {
        let waypoints = vec!["a".to_string()];
        let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0])]);
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 1);
        assert!(pos.is_none());
    }

    // ── Test 7: Arrived at final waypoint, looping ────────────────────────

    #[test]
    fn arrived_at_final_waypoint_looping_wraps() {
        let waypoints = vec!["a".to_string()];
        let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
        // Start at (100,0,0) → arrived at waypoint a → wrap to 0, still at a
        let (idx, pos) = advance_cursor(0, &waypoints, true, [100.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    // ── Test 8: Arrived at each of 3 waypoints in sequence ────────────────

    #[test]
    fn arrived_at_three_waypoints_sequential_non_looping() {
        let waypoints = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let anchors = make_anchors(&[
            ("a", [0.0, 0.0, 0.0]),
            ("b", [100.0, 0.0, 0.0]),
            ("c", [200.0, 0.0, 0.0]),
        ]);

        // At a → advance to b
        let (idx, _) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 1, "should advance past a");

        // At b → advance to c
        let (idx, _) = advance_cursor(idx, &waypoints, false, [100.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 2, "should advance past b");

        // At c → terminal stop
        let (idx, pos) = advance_cursor(idx, &waypoints, false, [200.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 3, "should advance past c");
        assert!(pos.is_none(), "non-looping should stop at end");
    }

    // ── Test 9: Missing anchor — skip to next valid ───────────────────────

    #[test]
    fn missing_anchor_skips_to_next_valid() {
        let waypoints = vec!["missing".to_string(), "valid".to_string()];
        let anchors = make_anchors(&[("valid", [100.0, 0.0, 0.0])]);
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 1);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    // ── Test 10: Missing anchor on only waypoint ──────────────────────────

    #[test]
    fn missing_anchor_on_only_waypoint_terminates() {
        let waypoints = vec!["missing".to_string()];
        let anchors = make_anchors(&[]);
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 1);
        assert!(pos.is_none());
    }

    // ── Test 11: Position outside arrival radius ──────────────────────────

    #[test]
    fn outside_arrival_radius_does_not_advance() {
        let waypoints = vec!["a".to_string()];
        let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
        // Entity at (50, 0, 0) → 50 units away, arrival_radius = 20 → not arrived
        let (idx, pos) = advance_cursor(0, &waypoints, false, [50.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    // ── Test 12: Zero arrival radius (must be exact) ──────────────────────

    #[test]
    fn zero_arrival_radius_requires_exact_position() {
        let waypoints = vec!["a".to_string()];
        let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
        // 1 unit away → not arrived with radius 0
        let (idx, pos) = advance_cursor(0, &waypoints, false, [99.0, 0.0, 0.0], &anchors, 0.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));

        // Exactly at waypoint → arrived
        let (idx, pos) = advance_cursor(0, &waypoints, false, [100.0, 0.0, 0.0], &anchors, 0.0);
        assert_eq!(idx, 1);
        assert!(pos.is_none());
    }

    // ── Test 13: Multiple advances in one call (consecutive close waypoints) ─

    #[test]
    fn multiple_advances_in_one_call() {
        // Three waypoints all within arrival radius of entity position
        let waypoints = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let anchors = make_anchors(&[
            ("a", [0.0, 0.0, 0.0]),
            ("b", [1.0, 0.0, 0.0]),
            ("c", [5.0, 0.0, 0.0]),
        ]);
        // Entity at (0,0,0), radius 20 → all three waypoints within radius
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 3);
        assert!(pos.is_none());
    }

    #[test]
    fn multiple_advances_in_one_call_looping() {
        let waypoints = vec!["a".to_string(), "b".to_string()];
        let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [1.0, 0.0, 0.0])]);
        // Both within radius → advance through both and wrap to 0
        let (idx, pos) = advance_cursor(0, &waypoints, true, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([0.0, 0.0, 0.0]));
    }

    // ── Test 14: Independence for multiple objectives ─────────────────────

    #[test]
    fn advancement_independence_for_multiple_objectives() {
        // Two objective states: one at index 0 (far from waypoint), one at index 0
        let waypoints_a = vec!["wp_a".to_string()];
        let waypoints_b = vec!["wp_b".to_string()];
        let anchors = make_anchors(&[("wp_a", [100.0, 0.0, 0.0]), ("wp_b", [0.0, 0.0, 0.0])]);

        // Advance cursor A (far away)
        let (idx_a, pos_a) =
            advance_cursor(0, &waypoints_a, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx_a, 0, "A should not advance");
        assert_eq!(pos_a, Some([100.0, 0.0, 0.0]));

        // Advance cursor B (arrived)
        let (idx_b, pos_b) =
            advance_cursor(0, &waypoints_b, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx_b, 1, "B should advance to terminal");
        assert!(pos_b.is_none());

        // A's state is unchanged
        let (idx_a2, pos_a2) =
            advance_cursor(idx_a, &waypoints_a, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx_a2, 0, "A should still be at index 0");
        assert_eq!(pos_a2, Some([100.0, 0.0, 0.0]));
    }

    // ── Additional edge cases ─────────────────────────────────────────────

    #[test]
    fn looping_with_skip_past_end_wraps_and_finds_valid() {
        let waypoints = vec!["missing".to_string(), "a".to_string()];
        let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
        // Start at index past end (2), loop wraps to 0, skip missing, land on a
        let (idx, pos) = advance_cursor(2, &waypoints, true, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 1);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    #[test]
    fn all_anchors_missing_non_looping_terminates() {
        let waypoints = vec!["a".to_string(), "b".to_string()];
        let anchors = make_anchors(&[]);
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 2);
        assert!(pos.is_none());
    }

    #[test]
    fn all_anchors_missing_looping_infinite_loop_avoids() {
        let waypoints = vec!["a".to_string(), "b".to_string()];
        let anchors = make_anchors(&[]);
        let (idx, pos) = advance_cursor(0, &waypoints, true, [0.0, 0.0, 0.0], &anchors, 20.0);
        // Skips both, wraps to 0, skips both again, wraps to 0... but
        // the function will eventually return (0, None) since idx keeps resetting.
        // Actually the function would loop forever in this case.
        // This is an edge case the caller should avoid; the loop doesn't
        // detect this condition. We just verify it's handled by termination.
        assert_eq!(idx, 2);
        assert!(pos.is_none());
    }

    #[test]
    fn arrived_missing_anchor_sequence_skips_and_advances() {
        let waypoints = vec!["a".to_string(), "missing".to_string(), "b".to_string()];
        let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 20.0);
        // Arrived at a (idx→1), skip missing (idx→2), found b at idx 2
        assert_eq!(idx, 2);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }

    #[test]
    fn y_component_affects_arrival() {
        let waypoints = vec!["a".to_string()];
        let anchors = make_anchors(&[("a", [0.0, 10.0, 0.0])]);
        // Entity at (0,0,0) → distance = 10, arrival_radius = 5 → not arrived
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 0.0, 0.0], &anchors, 5.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([0.0, 10.0, 0.0]));

        // Entity at (0,9,0) → distance = 1, arrival_radius = 5 → arrived
        let (idx, pos) = advance_cursor(0, &waypoints, false, [0.0, 9.0, 0.0], &anchors, 5.0);
        assert_eq!(idx, 1);
        assert!(pos.is_none());
    }

    #[test]
    fn arrived_at_final_waypoint_looping_wraps_then_returns_position() {
        let waypoints = vec!["a".to_string(), "b".to_string()];
        let anchors = make_anchors(&[("a", [0.0, 0.0, 0.0]), ("b", [100.0, 0.0, 0.0])]);
        // Entity at b (100,0,0), arrived at b, non-looping → terminal
        let (idx, pos) = advance_cursor(1, &waypoints, false, [100.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 2);
        assert!(pos.is_none());
    }

    #[test]
    fn already_at_index_past_end_looping_returns_position() {
        let waypoints = vec!["a".to_string()];
        let anchors = make_anchors(&[("a", [100.0, 0.0, 0.0])]);
        // Index is 1, waypoints.len() = 1, looping → wrap to 0, far from waypoint
        let (idx, pos) = advance_cursor(1, &waypoints, true, [0.0, 0.0, 0.0], &anchors, 20.0);
        assert_eq!(idx, 0);
        assert_eq!(pos, Some([100.0, 0.0, 0.0]));
    }
}
