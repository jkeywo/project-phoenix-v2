use super::*;
pub use phoenix_sim_gameplay::ship::helm_ai::steering::{HelmSteeringAiPolicyState, SteeringAxis};
/// Per-axis helm AI: steering. Decides the yaw for ships whose helm-steering
/// system is AI-operated and emits it as an admitted `SetSteering` into the
/// ship's own `AdmittedCommands` (issues #800, #704, #824); it owns the
/// arc-bearing step outright.
///
/// Steers toward the selected waypoint/target chosen by the pure
/// `crate::ai::operate_helm`, including the **Retreat consumer** (issue #688).
/// `ai_helm_steering_retreats_toward_anchor` pins that behaviour through this
/// system, and `ai_helm_steering_retreat_with_unknown_anchor_falls_through` the
/// other side of it.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<SteeringAxis>`](run_helm_axis) driver's; this body
/// assembles the per-ship context (including the mutable arc-bearing request)
/// and emits the payload it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_steering(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    clock: Res<AiPolicyTickClock>,
    ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            // Availability of the two optional drives — see `ai_helm_thrust`.
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&FineSystemAiPolicies>,
            Option<&HelmSteeringAiPolicyState>,
            Option<&mut PendingArcBearingRequest>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    phoenix_sim_gameplay::ship::helm_ai::steering::ai_helm_steering(
        &ai_env, frame, plan, clock, ships,
    );
}
