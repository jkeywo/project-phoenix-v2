pub mod core;
pub mod faction;
pub mod lod;
pub mod patrol_cursor;
pub mod server;

pub use core::{
    assess_hazards, decide_impulse, decode_steering_from_facing, decode_thrust_from_velocity,
    encode_local_facing, encode_local_velocity, find_nearest_hostile, operate_helm,
    operate_lateral_thrust, resolve_objective_target, score_doctrine_pool, steer_toward,
    visible_entities, AiWorldEntity, CaptainAi, HazardAssessmentRaw, ImpulseDecision,
    ImpulseDecisionInput, WorldView, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS,
    IMPULSE_ANGLE_TOLERANCE_RAD, NAV_HANDOFF_SPEED, PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD,
    WAYPOINT_ARRIVAL_RADIUS,
};
