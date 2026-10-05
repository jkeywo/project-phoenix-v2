//! Pure comms range math.
//!
//! Two entities can communicate when the distance between them is at most
//! the smaller of their two comms ranges. This module is Bevy-free so it can
//! be unit-tested directly on native and reused by both server and client.

/// Returns true when `distance` is within the effective comms range between
/// two entities whose individual ranges are `a` and `b`.
///
/// The effective range is `a.min(b)` (a transmission only goes through when
/// both ends can hear each other). Negative ranges are treated as zero
/// (entities with no comms capability are never in range).
pub fn in_range(distance: f32, a: f32, b: f32) -> bool {
    if distance.is_nan() || a.is_nan() || b.is_nan() {
        return false;
    }
    let effective = a.min(b).max(0.0);
    distance <= effective
}

#[cfg(test)]
#[path = "range_tests.rs"]
mod tests;
