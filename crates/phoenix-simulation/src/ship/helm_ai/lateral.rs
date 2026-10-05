use super::*;
/// Per-axis helm AI: lateral thrust. Decides the dodge for ships whose
/// helm-lateral-thrust system is AI-operated and emits it as an admitted
/// `LateralThrustInput` into the ship's own `AdmittedCommands` (issues #703,
/// #704, #824). Docking translation still overrides it (issue #742), and the
/// emit → admit → apply arbiter path is unchanged.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<LateralAxis>`](run_helm_axis) driver's, with the docking
/// override expressed as [`LateralAxis::pre_override`](HelmAxisHost::pre_override);
/// this body assembles the per-ship context and emits the payload it returns.
pub(crate) fn ai_helm_lateral_thrust(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            // Optional, so it does not filter the iteration set: a ship without
            // a `[behaviour]` section still runs AI lateral thrust, on the
            // `crate::ai::*` fallbacks that match the serde defaults.
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&FineSystemAiPolicies>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    phoenix_sim_gameplay::ship::helm_ai::lateral::ai_helm_lateral_thrust(
        &ai_env, frame, plan, ships,
    );
}
