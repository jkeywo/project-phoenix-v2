use super::*;
pub use phoenix_sim_gameplay::ship::helm_ai::engines::{EnginesAxis, HelmEnginesAiPolicyState};
/// Per-axis helm AI: throttle. Decides the throttle for ships whose
/// helm-thrust system is AI-operated and emits it as an admitted `SetThrust`
/// into the ship's own `AdmittedCommands` (issues #800, #704, #824) —
/// `process_helm_inputs` applies it to `ThrustInput` later this tick.
///
/// `AiHighFidelity`-scoped: the frame is only built for ships carrying that
/// marker, and the intent components the admitted command lands on only
/// exist there (`lod_ai_ships` inserts/removes them with the marker).
///
/// Since #1208 the gate/declare/resolve preamble is the shared
/// [`run_helm_axis::<EnginesAxis>`](run_helm_axis) driver's; this body only
/// assembles the per-ship context and emits the payload it returns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_helm_thrust(
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
            // Availability of the two optional drives, seeded honestly into the
            // fact snapshot (see `EnginesAxis::seed`).
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&FineSystemAiPolicies>,
            Option<&HelmEnginesAiPolicyState>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    phoenix_sim_gameplay::ship::helm_ai::engines::ai_helm_thrust(
        &ai_env, frame, plan, clock, ships,
    );
}
