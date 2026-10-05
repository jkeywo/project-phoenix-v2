use super::*;
/// Per-axis helm AI: vertical thrust (issue #744). Decides the up/down axis for
/// ships whose `helm-vertical-thrust` system is AI-operated and emits it as an
/// admitted `VerticalThrustInput` into the ship's own `AdmittedCommands`,
/// through the same `emit_ai_command` arbiter as the other per-axis operators.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<VerticalAxis>`](run_helm_axis) driver's; this body
/// assembles the per-ship context and emits the payload it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_vertical_thrust(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    _sessions: Res<crate::lobby::Sessions>,
    ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&crate::entities::spawner::HelmCapabilitySection>,
            Option<&FineSystemAiPolicies>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    phoenix_sim_gameplay::ship::helm_ai::vertical::ai_helm_vertical_thrust(
        &ai_env, frame, plan, ships,
    );
}
