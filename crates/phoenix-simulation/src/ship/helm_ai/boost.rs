use super::*;
pub use phoenix_sim_gameplay::ship::helm_ai::boost::{BoostAxis, HelmBoostAiPolicyState};
/// Per-axis helm AI: boost drive (issue #780). Decides engage/release for ships
/// whose `helm-boost` system is AI-operated and emits it as an admitted
/// `SetBoost { active }` into the ship's own `AdmittedCommands` — the SAME seam
/// a human `SetBoost`/`ToggleBoost` passes through (`process_helm_inputs`),
/// preserving human/AI symmetry (AGENTS.md #6).
///
/// Availability (AC6) is the presence of an *enabled* [`BoostConfigResource`].
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<BoostAxis>`](run_helm_axis) driver's; this body checks the
/// boost capability, assembles the per-ship context, and emits the payload it
/// returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_boost(
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
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&ShipBoost>,
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&FineSystemAiPolicies>,
            Option<&HelmBoostAiPolicyState>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::components::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
    clock: Res<AiPolicyTickClock>,
) {
    phoenix_sim_gameplay::ship::helm_ai::boost::ai_helm_boost(&ai_env, frame, plan, ships, clock);
}
