use std::collections::HashMap;

/// One objective's place on its patrol route.
///
/// Owns the whole of a cursor's mutable state, so "which waypoint am I flying
/// to" and "have I already announced this lap" are separate fields rather than
/// magic values overloaded into a single index. [`advance_cursor`] is the only
/// thing that writes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatrolCursor {
    /// Id of the objective whose route this cursor walks.
    pub objective_id: String,
    /// Index into the objective's waypoint list. Always a plain index: either
    /// in range, or `>= waypoints.len()` for a non-looping route that has run
    /// past its final waypoint (terminal stop). Never a sentinel — a looping
    /// route's cursor always names a real waypoint, so it can always be
    /// resumed.
    index: usize,
    /// Whether this cursor has already walked a lap from where the entity is
    /// now and found nowhere to steer — see [`advance_cursor`].
    settled: bool,
}

impl PatrolCursor {
    /// A fresh cursor at the start of `objective_id`'s route.
    pub fn new(objective_id: impl Into<String>) -> Self {
        Self {
            objective_id: objective_id.into(),
            index: 0,
            settled: false,
        }
    }

    /// A cursor put back exactly where a snapshot found it (issue #862).
    ///
    /// Separate from [`Self::new`] because `new` is a *fresh* cursor by
    /// definition, and a restore that had to go through it would put every
    /// patrolling ship back at waypoint 0 — steering for the start of a route
    /// it was halfway around.
    pub fn restored(objective_id: impl Into<String>, index: usize, settled: bool) -> Self {
        Self {
            objective_id: objective_id.into(),
            index,
            settled,
        }
    }

    /// The waypoint index this cursor is steering toward.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Whether the cursor is holding station: it has announced a lap from the
    /// entity's current place on the route and is waiting for the entity to
    /// move somewhere that gives it something to fly to again.
    pub fn settled(&self) -> bool {
        self.settled
    }
}

/// Resolve `current_index` against `waypoints`, applying wraparound for
/// looping routes. Returns `None` when the route is empty, or when a
/// non-looping route has run past its final waypoint (terminal stop).
fn resolve_index(current_index: usize, waypoints: &[String], loop_path: bool) -> Option<usize> {
    if waypoints.is_empty() {
        return None;
    }
    if current_index < waypoints.len() {
        return Some(current_index);
    }
    loop_path.then_some(0)
}

fn distance_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

/// The position the cursor is currently steering toward, *without* advancing it.
///
/// Read-only counterpart to [`advance_cursor`]: consumers that only need to
/// know "where am I heading right now" (e.g. the cheap low-LOD steering path)
/// use this, leaving arrival detection and cursor advancement to the single
/// evaluator that owns them.
///
/// Returns `None` when the route is empty, when a non-looping route has
/// finished, or when the current waypoint's anchor is unknown. A settled
/// cursor (see [`advance_cursor`]) still names its waypoint: it holds station
/// on the route rather than losing it.
pub fn cursor_target(
    current_index: usize,
    waypoints: &[String],
    loop_path: bool,
    anchors: &HashMap<String, [f32; 3]>,
) -> Option<[f32; 3]> {
    let idx = resolve_index(current_index, waypoints, loop_path)?;
    anchors.get(waypoints[idx].as_str()).copied()
}

/// Has this non-looping route been flown to its end?
///
/// Separates the two very different reasons [`cursor_target`] returns `None`:
/// the route is **finished** — the cursor has run past the last waypoint and the
/// entity belongs where it is — versus the route is merely unflyable (empty, or
/// the current anchor is unknown). A caller that treats "finished" as "no route"
/// keeps flying: that is how the Requiem Courier reached its destination and
/// then cruised straight on past it forever.
///
/// A looping route is never finished — it wraps. One whose anchors are *all*
/// unknown settles on a valid index rather than running off the end (see
/// "Settling" on [`advance_cursor`]), so it stays unfinished indefinitely.
///
/// # What "finished" does and does not mean
///
/// This is a question about the **cursor index**, not about whether the entity
/// arrived anywhere. The unknown-anchor case reads as unfinished only for the
/// single tick before [`advance_cursor`] skips past it: on a one-waypoint
/// non-looping `Reach` whose anchor the world never defines, the next tick
/// leaves `index == 1 == waypoints.len()` and this returns `true` from then on —
/// the entity is classified as having *arrived* at a place that does not exist,
/// and its caller parks it where it stands. Parking is a better end state than
/// the endless drift it replaced, but it is not evidence the route was flown.
/// An anchor no world defines is a content error, and nothing in this module can
/// see it; `route_with_an_unknown_anchor_is_not_completed_only_until_the_skip`
/// pins both halves.
pub fn route_completed(current_index: usize, waypoints: &[String], loop_path: bool) -> bool {
    !waypoints.is_empty() && !loop_path && current_index >= waypoints.len()
}

/// Name of the waypoint the entity has just reached, if the cursor's current
/// waypoint resolves to a known anchor and `entity_pos` lies within
/// `arrival_radius` of it.
///
/// Deliberately does not advance the cursor — call [`advance_cursor`] for
/// that, which uses this as its per-step arrival test so that "has this
/// entity reached this waypoint?" is answered in exactly one place.
pub fn arrived_waypoint(
    current_index: usize,
    waypoints: &[String],
    loop_path: bool,
    entity_pos: [f32; 3],
    anchors: &HashMap<String, [f32; 3]>,
    arrival_radius: f32,
) -> Option<String> {
    let idx = resolve_index(current_index, waypoints, loop_path)?;
    let name = &waypoints[idx];
    let pos = anchors.get(name.as_str())?;
    (distance_sq(entity_pos, *pos) <= arrival_radius * arrival_radius).then(|| name.clone())
}

/// Is there anywhere on this route the entity could usefully fly to from
/// `entity_pos`? True when at least one waypoint has a known anchor that the
/// entity is *not* already inside `arrival_radius` of.
///
/// This is the exact negation of the condition under which [`advance_cursor`]
/// settles a looping route, and is therefore what un-settles it: the judgement
/// is re-made against the entity's position every call, so it can never latch.
fn has_somewhere_to_steer(
    waypoints: &[String],
    entity_pos: [f32; 3],
    anchors: &HashMap<String, [f32; 3]>,
    arrival_radius: f32,
) -> bool {
    waypoints.iter().any(|name| {
        anchors
            .get(name.as_str())
            .is_some_and(|pos| distance_sq(entity_pos, *pos) > arrival_radius * arrival_radius)
    })
}

/// Advance a single patrol cursor based on the entity's current position,
/// reporting every waypoint the cursor consumed on the way.
///
/// `cursor` is updated in place. The return value names, in order, each
/// waypoint the entity was inside `arrival_radius` of and which the cursor
/// therefore stepped past. One entry per waypoint actually consumed, so a tick
/// that skips over several tightly-spaced waypoints reports all of them and a
/// caller can announce each one. Waypoints skipped because their anchor is
/// unknown are *not* reported: they were never reached, only abandoned.
///
/// Advancement is single-step and bounded by `waypoints.len()`: each step
/// either consumes a reached waypoint or skips an unknown anchor, so at most
/// one lap is walked per call and the walk always terminates.
///
/// # Settling
///
/// A looping route has no end, so a lap can close with nowhere left to steer:
/// every waypoint on it is either already inside the arrival radius or has an
/// unknown anchor. That happens whenever a route's legs are shorter than the
/// authored `waypoint_arrival_radius` — a deliberate design for a
/// station-keeping patrol, not only a pathological one. Re-walking that lap
/// every tick would re-announce the same waypoints forever, so the cursor is
/// marked `settled`: it keeps its (valid) index and keeps steering, but stays
/// quiet.
///
/// Settling is *not* retirement. It is re-judged against position on every
/// call: the moment the entity is outside the arrival radius of any waypoint
/// with a known anchor — because it drifted, was knocked back, towed, or
/// teleported — the cursor un-settles and the route resumes normally,
/// re-announcing arrivals as it flies them.
///
/// A looping route whose anchors are *all* unknown can never be flown from any
/// position, so it stays settled: the cursor parks on a valid index, announces
/// nothing (an unreachable waypoint was never reached) and does no further
/// work per call beyond the position check.
pub fn advance_cursor(
    cursor: &mut PatrolCursor,
    waypoints: &[String],
    loop_path: bool,
    entity_pos: [f32; 3],
    anchors: &HashMap<String, [f32; 3]>,
    arrival_radius: f32,
) -> Vec<String> {
    let mut reached = Vec::new();

    let Some(mut idx) = resolve_index(cursor.index, waypoints, loop_path) else {
        // Empty or finished non-looping route: nothing to advance.
        return reached;
    };

    if cursor.settled {
        if !has_somewhere_to_steer(waypoints, entity_pos, anchors, arrival_radius) {
            // Still nowhere to go: hold station and stay quiet.
            return reached;
        }
        // The entity has moved somewhere the route can be flown from again.
        cursor.settled = false;
    }

    for _ in 0..waypoints.len() {
        match arrived_waypoint(
            idx,
            waypoints,
            loop_path,
            entity_pos,
            anchors,
            arrival_radius,
        ) {
            // Reached this waypoint — announce it and step past it.
            Some(name) => reached.push(name),
            // Not reached: either we still have to fly there (a known anchor
            // outside the radius — the cursor stays put and steers to it), or
            // the anchor is unknown and the waypoint is unreachable, in which
            // case step silently past it.
            None if anchors.contains_key(waypoints[idx].as_str()) => {
                cursor.index = idx;
                return reached;
            }
            None => {}
        }

        idx += 1;
        if idx >= waypoints.len() {
            if !loop_path {
                // Ran off the end of a one-shot route: terminal stop.
                cursor.index = idx;
                return reached;
            }
            idx = 0;
        }
    }

    // A full lap of a looping route without ever finding a waypoint to steer
    // toward. Keep the (valid) index and settle — see "Settling" above.
    cursor.index = idx;
    cursor.settled = true;
    reached
}

#[cfg(test)]
mod tests {
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
}
