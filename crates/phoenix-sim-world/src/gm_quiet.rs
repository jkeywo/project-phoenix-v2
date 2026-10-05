/// The quiet interval a world that says nothing gets: 120 simulation seconds
/// (PRD #1419). An author overrides it with `[gm_attention] quiet_time_secs`
/// and switches it off with the independent `quiet_time_disabled`, both on the
/// shared [`GmAttentionSettings`](crate::gm_attention::GmAttentionSettings)
/// table issue #1435 introduced for the queue's other advisory.
pub const DEFAULT_QUIET_SECONDS: f32 = 120.0;
