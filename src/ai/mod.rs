pub mod core;
pub mod faction;
pub mod lod;
pub mod patrol_cursor;
pub mod policy;
pub mod selector;
pub mod server;

pub use core::{
    assess_hazards, decide_impulse, decode_steering_from_facing, decode_thrust_from_velocity,
    docking_close_manoeuvre, encode_local_facing, encode_local_velocity, find_nearest_hostile,
    plan_fly_through_pass, plan_helm_travel, resolve_objective_target, score_doctrine_pool,
    steer_toward, target_relative_motion, visible_entities, AiWorldEntity, FlyThroughLeg,
    FlyThroughPassInput, HazardAssessmentRaw, HazardContribution, ImpulseDecision,
    ImpulseDecisionInput, TargetRelativeMotion, WorldView, AVOIDANCE_BUFFER,
    AVOIDANCE_LOOK_AHEAD_SECS, DOCKING_APPROACH_SPEED, DOCKING_ENGAGE_DISTANCE,
    HAZARD_IGNORE_SIZE_RATIO, IMMINENT_COLLISION_FACING_THRESHOLD, IMPULSE_ANGLE_TOLERANCE_RAD,
    LATERAL_HAZARD_SENSITIVITY, MAX_VERTICAL_OFFSET, NAV_HANDOFF_SPEED, PATROL_DEADBAND_RAD,
    PATROL_FULL_STEER_RAD, VERTICAL_HAZARD_SENSITIVITY, VERTICAL_RETURN_RATE,
    WAYPOINT_ARRIVAL_RADIUS,
};
