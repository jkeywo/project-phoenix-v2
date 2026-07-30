use bevy::prelude::*;

use crate::console_bridge::AiChatterEvent;

pub(crate) use crate::ai::cadence::ai_tick_ready;
pub(crate) use crate::ship::components::load_ship_config_from_disk;
pub use crate::ship::components::{
    ActiveStationRatings, BankConfigResource, BoostConfigResource, CoordinationEnqueue,
    CoordinationQueue, DockingMotionIntent, HelmWaypointClearance, ImpulseConfigResource,
    LastHelmInput, LastSystemTiers, PendingArcBearingRequest, PendingShipConfig,
    PendingTacticalFrequencyHint, RepairHumanAlerted, ShipConfigComponent,
    ShipPhysicsConfigResource, ShipSystemControlSources, BANK_LERP_RATE,
};
pub use crate::ship::coordination_systems::{
    handle_coordination_enqueue, handle_coordination_messages, process_coordination_lag,
};
pub use crate::ship::damage_sync::{detect_damage_tier_crossings, sync_console_damage_tiers};
pub(crate) use crate::ship::helm_admission::{
    operate_helm_engine_ai, process_helm_inputs, publish_joystick_to_engines,
};
pub(crate) use crate::ship::helm_ai::{
    ai_helm_boost, ai_helm_impulse, ai_helm_lateral_thrust, ai_helm_steering, ai_helm_thrust,
    ai_helm_vertical_thrust, ai_policy_state_tick, build_helm_ai_surfaces_frame,
    detect_reached_objective_completion, helm_axes_operate_ai, AiPolicyTickClock,
    HelmAiSurfacesFrame,
};
pub(crate) use crate::ship::helm_planner::{helm_motion_planner, HelmMotionPlan};
pub use crate::ship::impulse_boost_systems::handle_impulse_messages;
pub(crate) use crate::ship::impulse_boost_systems::{tick_boost, tick_impulse};
pub(crate) use crate::ship::physics_systems::{
    apply_helm_commands, integrate_ship_physics, sync_ship_position,
};
pub use crate::ship::rating_systems::handle_station_rating_change;

// Ã¢â€â‚¬Ã¢â€â‚¬ Plugin Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

pub struct ShipPlugin;

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        // Admitted-command consumers (issues #833, #745) for the per-axis Helm
        // wire targets are declared by the host-helm-control-router's own
        // dispatch module, so the router's dependency on the command-admission
        // seam is a real, observed code edge rather than an inline block shared
        // with dozens of other entities in this plugin file.
        crate::console::helm::dispatch::register_helm_dispatch(app);
        app.init_resource::<BankConfigResource>()
            .add_message::<CoordinationEnqueue>()
            .add_message::<AiChatterEvent>();
        // The ONE shared AI decision cadence (issues #803, #889). Installed by
        // every plugin that registers a gated system; the helper is idempotent
        // and registers its tick system in `Last`, so the latch is always
        // consumed by the `Update` systems it gates before it is re-armed.
        crate::ai::cadence::register_ai_cadence(app);
        app
            // The shared helm decision surface (issue #824): rebuilt once per
            // AI-helm sim tick by `build_helm_ai_surfaces_frame` and consumed
            // read-only by the four per-axis systems below.
            .init_resource::<HelmAiSurfacesFrame>()
            // The shared desired-motion + hazard surface (issue #741): rebuilt
            // once per AI-helm sim tick by `helm_motion_planner` from the
            // decision surface above, and consumed read-only by the per-axis
            // helm AI below.
            .init_resource::<HelmMotionPlan>()
            // Tick-derived clock for stateful policy `state_time` (issue #882).
            .init_resource::<AiPolicyTickClock>()
            .add_systems(
                Update,
                // ONE state tick for every stateful fine-system policy. Ordered
                // `.after(helm_motion_planner)` (its transition guards read the
                // hazard surface the planner publishes this tick) and `.before`
                // every per-axis actuator system, so the state it COMMITS is the
                // state those systems resolve their continuous outputs in —
                // AC2's same-tick guarantee. Under the shared
                // `run_if(ai_tick_ready)` latch so state time advances on
                // the fixed AI cadence, never per frame (AC4).
                ai_policy_state_tick
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::lobby::LobbySystemSet)
                    .after(crate::sim_sets::AiTickLabel)
                    .after(helm_motion_planner)
                    // Issue #791: this system seeds `torpedoes_in_flight` from
                    // the ship's LIVE `TorpedoSystemResource`, and both of the
                    // systems that write `in_flight` also run in
                    // `SimSet::Physics`. Without these two edges the reading
                    // would be run-order-dependent — the exact ambiguity that
                    // has bitten this codebase before — so the order is pinned
                    // rather than left to the scheduler:
                    //
                    // * `.after(handle_fire_torpedo)` so a salvo launched THIS
                    //   tick is already visible. Without it the doctrine could
                    //   read "no salvo in flight" on the very tick it launched
                    //   one and let go of the target a tick early.
                    // * `.after(tick_torpedo_lifecycle)` so a round that hit,
                    //   missed or expired this tick is already gone. The count
                    //   the transition guard sees is therefore the settled one
                    //   for this tick, in both directions.
                    //
                    // Both are absent from the ShipPlugin-only test fixtures,
                    // where an ordering edge against an unregistered system type
                    // is simply an empty constraint.
                    .after(crate::console::weapons::handle_fire_torpedo)
                    .after(crate::console::weapons::tick_torpedo_lifecycle)
                    .before(ai_helm_thrust)
                    .before(ai_helm_steering)
                    .before(ai_helm_lateral_thrust)
                    .before(ai_helm_vertical_thrust)
                    .before(ai_helm_impulse)
                    .before(ai_helm_boost)
                    .run_if(ai_tick_ready),
            )
            .add_systems(
                Update,
                (
                    // One assembly of the helm decision surface per AI tick
                    // (issue #824). `.after(AiTickLabel)`: it reads the viewscreen
                    // blackboard's scored objectives and the WorldSnapshot, which
                    // the AI tick writes. `.before` all four per-axis systems so
                    // whenever they run, the frame they read was built this tick
                    // (they share the same `run_if` latch).
                    build_helm_ai_surfaces_frame
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(crate::sim_sets::AiTickLabel)
                        .before(helm_motion_planner)
                        .before(ai_helm_lateral_thrust)
                        .before(ai_helm_impulse)
                        .run_if(ai_tick_ready),
                    // Shared desired-motion + hazard planner (issue #741). Runs
                    // between the decision-surface assembly it reads and the
                    // per-axis systems that consume its `HelmMotionPlan` output;
                    // it declares `.before` them here (they are registered in a
                    // separate tuple below). `ai_helm_lateral_thrust` reads the
                    // plan's docking lateral (issue #742), so it must run after
                    // the planner too — else it observes the previous tick's plan
                    // (empty on the first tick) and drops the docking translation.
                    helm_motion_planner
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(crate::sim_sets::AiTickLabel)
                        .before(ai_helm_thrust)
                        .before(ai_helm_steering)
                        .before(ai_helm_lateral_thrust)
                        .run_if(ai_tick_ready),
                    // `.after(AiTickLabel)`: this system reads the frame built
                    // from the viewscreen blackboard's scored objectives.
                    // `.before(process_helm_inputs)`: its emitted admitted
                    // command must be applied this tick (issue #824).
                    ai_helm_lateral_thrust
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(crate::sim_sets::AiTickLabel)
                        .before(process_helm_inputs)
                        // Shared AI-helm sim tick (issue #803) — one fixed-rate
                        // cadence for all four per-axis systems.
                        .run_if(ai_tick_ready),
                    // Applies commanded impulse/boost phase transitions (issue
                    // #695). Since #824 it runs AFTER `process_helm_inputs` —
                    // which is now the applier of admitted impulse commands into
                    // `ImpulseCommand` for every ship — and still before
                    // `tick_impulse`/`tick_boost` so a freshly-started
                    // charge/engagement begins progressing the same tick it was
                    // commanded. (`process_helm_inputs`' stale-input
                    // edge-detection therefore observes last tick's
                    // `ShipImpulse.phase`, one tick later than before #824 — the
                    // zeroing it performs is a handover cosmetic, not a physics
                    // input, and the impulse autopilot overrides helm intent
                    // while the drive is active anyway.)
                    apply_helm_commands
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(handle_impulse_messages)
                        .after(process_helm_inputs)
                        .before(tick_impulse)
                        .before(tick_boost),
                    // Per-entity admitted-command applier (issue #824): applies
                    // this tick's admitted helm payloads — human-admitted at the
                    // gate, AI-emitted by the four per-axis systems above — to
                    // every ship's intent components. Runs after every AI
                    // emitter (each declares `.before(process_helm_inputs)`) and
                    // before every intent/`LastHelmInput` reader below.
                    process_helm_inputs.in_set(crate::sim_sets::SimSet::Physics),
                    publish_joystick_to_engines
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(process_helm_inputs),
                    // A `LastHelmInput` thrust/steering pair reader. Since #824
                    // `process_helm_inputs` is the sole writer of that pair, so
                    // one `.after` edge is the whole torn-pair contract
                    // (`helm_ai_last_input_pair_is_not_torn` pins it).
                    operate_helm_engine_ai
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(process_helm_inputs),
                    detect_reached_objective_completion.in_set(crate::sim_sets::SimSet::Broadcast),
                    tick_impulse.in_set(crate::sim_sets::SimSet::Physics),
                    // `tick_boost` reads the `LastHelmInput` pair for drain
                    // scaling — `.after(process_helm_inputs)` is the torn-pair
                    // edge (see above); the boost-transition ordering edge is
                    // declared as `.before(tick_boost)` on `apply_helm_commands`.
                    tick_boost
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(process_helm_inputs),
                    handle_impulse_messages.in_set(crate::sim_sets::SimSet::Input),
                    // Sole helm-path writer of ShipPhysics (issues #695, #699):
                    // reads the intent components written by
                    // `process_helm_inputs` (human/admission) and the per-axis
                    // helm AI (`ai_helm_thrust`/`ai_helm_steering`/
                    // `ai_helm_lateral_thrust`), plus the post-transition
                    // `ShipImpulse`/`ShipBoost` state applied by
                    // `apply_helm_commands`, then performs the actual physics
                    // integration for whichever ship (LocalShip or promoted NPC)
                    // has fresh values this tick. Ordered after every writer of
                    // those intents and after `tick_impulse` so it reads this
                    // tick's freshly-ticked impulse phase, mirroring the old fused
                    // `process_helm_inputs` ordering. (`ai_helm_thrust` /
                    // `ai_helm_steering` declare `.before(integrate_ship_physics)`
                    // themselves; `ai_helm_lateral_thrust` and `ai_helm_impulse`
                    // are covered by `.after(process_helm_inputs)` /
                    // `.after(apply_helm_commands)` respectively.)
                    integrate_ship_physics
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(process_helm_inputs)
                        .after(apply_helm_commands)
                        .after(tick_impulse)
                        .after(tick_boost),
                    sync_ship_position
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(process_helm_inputs)
                        .after(integrate_ship_physics),
                    handle_station_rating_change.in_set(crate::sim_sets::SimSet::Input),
                    handle_coordination_enqueue.in_set(crate::sim_sets::SimSet::Input),
                    handle_coordination_messages.in_set(crate::sim_sets::SimSet::Input),
                    process_coordination_lag.in_set(crate::sim_sets::SimSet::Modifiers),
                    sync_console_damage_tiers.in_set(crate::sim_sets::SimSet::Damage),
                    detect_damage_tier_crossings.in_set(crate::sim_sets::SimSet::Damage),
                )
                    .after(crate::lobby::LobbySystemSet),
            );

        // Per-axis helm AI (issues #701, #824). Registered separately from the
        // tuple above purely because that tuple is at Bevy's 20-element limit.
        //
        // Ordering contract (issue #824 — emit-before-apply):
        // - `.after(AiTickLabel)`: the frame these systems consume is built
        //   from the viewscreen blackboard's scored objectives and the
        //   WorldSnapshot, which the AI tick writes.
        //   (`build_helm_ai_surfaces_frame` declares `.before` each of them.)
        // - Each system gates on its own axis alone (#800) and is the sole
        //   *decider* of the axis it owns. They are deliberately *unordered*
        //   against each other: each emits only its own axis's admitted
        //   command and none reads another's output.
        // - Ordered BEFORE `process_helm_inputs` — the reverse of the
        //   pre-#824 shape. The AI no longer writes intent components
        //   directly; it emits admitted commands into its own ship's
        //   `AdmittedCommands` (Option A of #824: a direct same-tick write
        //   via `validate_and_admit`, never a round-trip through the
        //   InboundMessage queue), and `process_helm_inputs` is the single
        //   applier for AI and human commands alike. Authority is checked
        //   once, at admission: a human command for an AI-held axis was
        //   already refused at the gate, so "who wins the axis" is decided
        //   by admission, not by system order.
        // - `LastHelmInput` now has ONE writer (`process_helm_inputs`
        //   mirrors applied payloads for the LocalShip), so the torn-pair
        //   contract is the single `.after(process_helm_inputs)` edge each
        //   pair reader (`publish_joystick_to_engines`,
        //   `operate_helm_engine_ai`, `tick_boost`) declares in the tuple
        //   above; `helm_ai_last_input_pair_is_not_torn` pins the result.
        //   (`ai_power_allocation` reads `.thrust` alone, so it cannot see a
        //   torn pair and needs no edge.)
        app.add_systems(
            Update,
            (ai_helm_thrust, ai_helm_steering)
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .before(process_helm_inputs)
                // Shared AI-helm sim tick (issue #803) — one fixed-rate
                // cadence for all four per-axis systems.
                .run_if(ai_tick_ready),
        );

        // Per-axis helm AI: impulse (issues #703, #824). Registered on its own
        // for symmetry with its pre-#824 shape; since #824 its ordering is the
        // same as its three siblings: `.before(process_helm_inputs)`, which
        // applies the emitted `StartImpulseCharge`/`CancelImpulse` into
        // `ImpulseCommand` and itself runs `.before(apply_helm_commands)` —
        // so the phase transition still lands the same tick it was decided.
        // `.after(AiTickLabel)` covers the frame this system reads.
        app.add_systems(
            Update,
            ai_helm_impulse
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .before(process_helm_inputs)
                // Shared AI-helm sim tick (issue #803) — one fixed-rate
                // cadence for all four per-axis systems.
                .run_if(ai_tick_ready),
        );

        // Per-axis helm AI: vertical thrust (issue #744). Same shape as its
        // siblings — gated on the shared AI-helm sim tick, `.after(AiTickLabel)`
        // for the decision frame, and `.before(process_helm_inputs)` so its
        // emitted `VerticalThrustInput` is applied this tick. It reads the
        // planner's `HelmMotionPlan` (the moving-hazard threat), so it also runs
        // `.after(helm_motion_planner)`. Registered separately because the main
        // tuple is at Bevy's 20-element limit.
        app.add_systems(
            Update,
            ai_helm_vertical_thrust
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .after(helm_motion_planner)
                .before(process_helm_inputs)
                .run_if(ai_tick_ready),
        );

        // Per-axis helm AI: boost (issues #780, #881). Same shape as its
        // siblings — gated on the shared AI-helm sim tick, `.after(AiTickLabel)`
        // for the decision frame, and `.before(process_helm_inputs)` so its
        // emitted admitted `SetBoost` is turned into a `BoostCommand` this tick.
        // Since #881 that applier is `process_helm_inputs` for EVERY ship rather
        // than the retired LocalShip-only `handle_boost_messages`, so a
        // non-local NPC's boost decision now actually lands; the same-tick
        // property is preserved because `apply_helm_commands` is
        // `.after(process_helm_inputs)` and `.before(tick_boost)`. It reads the
        // planner's `HelmMotionPlan` hazard facts, so it also runs
        // `.after(helm_motion_planner)`. Registered separately because the main
        // tuple is at Bevy's 20-element limit.
        app.add_systems(
            Update,
            ai_helm_boost
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .after(helm_motion_planner)
                .before(process_helm_inputs)
                .run_if(ai_tick_ready),
        );

        // Debug-only helm-path single-writer tripwire (issue #699). The frame
        // stamp is bumped once in `First` so every writer in a given frame
        // observes the same value; `integrate_ship_physics` stamps each ship
        // it integrates and panics if that ship was already stamped this
        // frame. Compiled out entirely in release builds.
        #[cfg(debug_assertions)]
        app.init_resource::<crate::ship::helm::HelmPhysicsFrame>()
            .add_systems(First, crate::ship::helm::tick_helm_physics_frame);
    }
}
