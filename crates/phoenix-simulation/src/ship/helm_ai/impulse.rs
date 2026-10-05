use super::*;
/// Per-axis helm AI: impulse drive. Decides engage/cancel for ships whose
/// helm-impulse system is AI-operated and emits it as an admitted
/// `StartImpulseCharge`/`CancelImpulse` into the ship's own
/// `AdmittedCommands` (issues #703, #704, #824); `process_helm_inputs`
/// applies it to `ImpulseCommand` later this tick, before
/// `apply_helm_commands` consumes the transition.
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<ImpulseAxis>`](run_helm_axis) driver's; this body checks
/// the impulse capability, assembles the per-ship context, and emits the payload
/// it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_impulse(
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
            &ShipPhysics,
            Option<&ShipImpulse>,
            Option<&ImpulseConfigResource>,
            Option<&BoostConfigResource>,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&FineSystemAiPolicies>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            Option<&crate::ai::server::ObjectiveCursors>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    phoenix_sim_gameplay::ship::helm_ai::impulse::ai_helm_impulse(&ai_env, frame, plan, ships);
}
