pub mod cadence;
pub mod core;
pub mod faction;
pub mod host;
pub mod lod;
pub mod patrol_cursor;
pub mod policy;
pub mod selector;
pub mod server;

// `core::hazard_threat_fraction` is deliberately NOT re-exported (issue #968
// review). It is `pub` so the two hazard entry points below can document it by
// link and so a future fidelity can share the one severity curve, but it has no
// consumer outside `core.rs` today and a `crate::ai::`-level name would imply
// one. Reach it as `crate::ai::core::hazard_threat_fraction` if that changes;
// it is total for any distance, but both in-tree callers still range-gate
// first because WHICH hazards register is a separate decision from how hard
// each one pushes.
pub use core::{
    assess_hazards, avoidance_steering, decide_impulse, decode_steering_from_facing,
    decode_thrust_from_velocity, docking_close_manoeuvre, encode_local_facing,
    encode_local_velocity, find_nearest_hostile, hostile_arc_exposure, plan_artillery_position,
    plan_fly_through_pass, plan_helm_travel, plan_recovery_orbit, resolve_objective_target,
    score_doctrine_pool, steer_toward, target_relative_motion, visible_entities, AiWorldEntity,
    ArtilleryPositionInput, FlyThroughLeg, FlyThroughPassInput, HazardAssessmentRaw,
    HazardContribution, ImpulseDecision, ImpulseDecisionInput, RecoveryOrbitInput,
    TargetRelativeMotion, WorldView, AVOIDANCE_BUFFER, AVOIDANCE_LOOK_AHEAD_SECS,
    DOCKING_APPROACH_SPEED, DOCKING_ENGAGE_DISTANCE, HAZARD_IGNORE_SIZE_RATIO,
    HAZARD_THREAT_EXPONENT, IMMINENT_COLLISION_FACING_THRESHOLD, IMPULSE_ANGLE_TOLERANCE_RAD,
    LATERAL_HAZARD_SENSITIVITY, LOW_LOD_AVOIDANCE_DEVIATION_RAD, MAX_VERTICAL_OFFSET,
    NAV_HANDOFF_SPEED, PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD, VERTICAL_HAZARD_SENSITIVITY,
    VERTICAL_RETURN_RATE, WAYPOINT_ARRIVAL_RADIUS,
};
