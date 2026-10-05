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
#[path = "patrol_cursor_tests.rs"]
mod tests;
