/// Distinct outstanding demands at which a Station becomes a candidate for
/// Overloaded, before the duration is considered.
pub const DEFAULT_OVERLOAD_COUNT: u32 = 3;

/// How long the count must stay at or above the threshold, in SIMULATION
/// seconds, before Overloaded is the answer.
pub const DEFAULT_OVERLOAD_SECS: f32 = 30.0;
