use bevy::prelude::*;

use crate::console_bridge::AiChatterEvent;

pub(crate) use crate::ship::components::{load_ship_config_from_disk, HelmInputTimer};
pub use crate::ship::components::{
    ActiveStationRatings, BankConfigResource, BoostConfigResource, CoordinationEnqueue,
    CoordinationQueue, HelmWaypointClearance, ImpulseConfigResource, LastHelmInput,
    LastSystemTiers, PendingArcBearingRequest, PendingShipConfig, RepairHumanAlerted,
    ShipConfigComponent, ShipPhysicsConfigResource, ShipSystemControlSources, BANK_LERP_RATE,
};
pub use crate::ship::coordination_systems::{
    handle_coordination_enqueue, handle_coordination_messages, process_coordination_lag,
};
pub use crate::ship::damage_sync::{detect_damage_tier_crossings, sync_console_damage_tiers};
pub(crate) use crate::ship::helm_admission::{
    operate_helm_engine_ai, process_helm_inputs, publish_joystick_to_engines,
};
pub(crate) use crate::ship::helm_ai::{
    ai_helm_impulse, ai_helm_lateral_thrust, ai_helm_steering, ai_helm_thrust, ai_helm_tick_ready,
    detect_reached_objective_completion, helm_axes_operate_ai, tick_ai_helm_timer, AiHelmTickReady,
    AiHelmTickTimer,
};
pub use crate::ship::impulse_boost_systems::{handle_boost_messages, handle_impulse_messages};
pub(crate) use crate::ship::impulse_boost_systems::{tick_boost, tick_impulse};
pub(crate) use crate::ship::physics_systems::{
    apply_helm_commands, integrate_ship_physics, sync_ship_position,
};
pub use crate::ship::rating_systems::handle_station_rating_change;

// Ã¢â€â‚¬Ã¢â€â‚¬ Plugin Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

pub struct ShipPlugin;

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmInputTimer(Timer::from_seconds(
            1.0 / 30.0,
            TimerMode::Repeating,
        )))
        .init_resource::<BankConfigResource>()
        .add_message::<CoordinationEnqueue>()
        .add_message::<AiChatterEvent>()
        // Shared AI-helm sim tick (issue #803): the timer/latch pair that
        // gates all four per-axis AI helm systems, plus the dedicated system
        // that advances it. The tick system runs `.after` every gated system
        // so the latch is consumed before it is re-armed — the same
        // consume-then-arm shape as `ai::server::tick_ai_snapshot_timer`.
        .init_resource::<AiHelmTickTimer>()
        .insert_resource(AiHelmTickReady(true))
        .add_systems(
            Update,
            tick_ai_helm_timer
                .after(ai_helm_thrust)
                .after(ai_helm_steering)
                .after(ai_helm_lateral_thrust)
                .after(ai_helm_impulse),
        )
        .add_systems(
            Update,
            (
                // `.after(AiTickLabel)`: this system reads the viewscreen
                // blackboard's scored objectives, which the AI tick writes.
                ai_helm_lateral_thrust
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel)
                    .before(process_helm_inputs)
                    // Shared AI-helm sim tick (issue #803) — one fixed-rate
                    // cadence for all four per-axis systems.
                    .run_if(ai_helm_tick_ready),
                // Applies commanded impulse/boost phase transitions (issue
                // #695). Must run before `process_helm_inputs` — whose
                // stale-input edge-detection reads `ShipImpulse.phase` and
                // needs to see this tick's transition, not last tick's —
                // and before `tick_impulse`/`tick_boost` so a
                // freshly-started charge/engagement begins progressing the
                // same tick it was commanded, mirroring the old fused
                // `process_helm_inputs`/`handle_impulse_messages` ordering.
                apply_helm_commands
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(handle_impulse_messages)
                    .after(handle_boost_messages)
                    .before(process_helm_inputs)
                    .before(tick_impulse)
                    .before(tick_boost),
                process_helm_inputs
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(tick_impulse),
                publish_joystick_to_engines
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(process_helm_inputs),
                // A `LastHelmInput` thrust/steering pair reader.
                // `ai_helm_thrust`/`ai_helm_steering` declare
                // `.before(operate_helm_engine_ai)` themselves, and
                // `ai_helm_lateral_thrust` (the `.lateral` field's writer) is
                // transitively before it via `process_helm_inputs`.
                operate_helm_engine_ai.in_set(crate::sim_sets::SimSet::Physics),
                detect_reached_objective_completion.in_set(crate::sim_sets::SimSet::Broadcast),
                tick_impulse.in_set(crate::sim_sets::SimSet::Physics),
                tick_boost.in_set(crate::sim_sets::SimSet::Physics),
                handle_impulse_messages.in_set(crate::sim_sets::SimSet::Input),
                handle_boost_messages.in_set(crate::sim_sets::SimSet::Input),
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

        // Per-axis helm AI (issue #701). Registered separately from the tuple
        // above purely because that tuple is at Bevy's 20-element limit.
        //
        // Ordering contract:
        // - `.after(AiTickLabel)`: both systems read the viewscreen
        //   blackboard's scored objectives, which the AI tick writes.
        // - Each system gates on its own axis alone (#800) and is the sole
        //   writer of the axis it owns, so no edge between them (or against a
        //   second writer) is needed for write exclusion. They are
        //   deliberately *unordered* against each other: each writes only its
        //   own axis and neither reads the other's output.
        // - Ordered AFTER `process_helm_inputs`, unlike
        //   `ai_helm_lateral_thrust` (lateral's fine system being AI-operated
        //   means no lateral command is ever admitted, so there is nothing
        //   for `process_helm_inputs` to clobber it with). Thrust and
        //   steering are per-axis admitted (`SetThrust` → `helm-thrust`,
        //   `SetSteering` → `helm-steering`), and `process_helm_inputs`
        //   additionally skips writing an AI-held axis's intent. Running last
        //   makes the per-axis AI the authoritative writer of its axis
        //   deterministically rather than by set-order luck.
        // - Both systems write `LastHelmInput.{thrust,steering}` for the
        //   player ship, one field each, and are the only writers of those
        //   fields. Every reader of that *pair* in `SimSet::Physics` must be
        //   ordered after BOTH of them, or it can observe a torn pair — this
        //   tick's AI throttle next to last tick's stale human steering. The
        //   pair readers are `publish_joystick_to_engines`,
        //   `operate_helm_engine_ai` and `tick_boost`;
        //   `helm_ai_last_input_pair_is_not_torn` pins the result.
        //   (`ai_helm_lateral_thrust` writes only the disjoint `.lateral`
        //   field and is already `.before(process_helm_inputs)`;
        //   `ai_power_allocation` reads `.thrust` alone, so it cannot see a
        //   torn pair and needs no edge.)
        app.add_systems(
            Update,
            (ai_helm_thrust, ai_helm_steering)
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .after(process_helm_inputs)
                .before(publish_joystick_to_engines)
                .before(operate_helm_engine_ai)
                .before(tick_boost)
                .before(integrate_ship_physics)
                // Shared AI-helm sim tick (issue #803) — one fixed-rate
                // cadence for all four per-axis systems.
                .run_if(ai_helm_tick_ready),
        );

        // Per-axis helm AI: impulse (issue #703). Registered on its own because
        // its ordering is the mirror image of the other two per-axis systems.
        //
        // `.before(apply_helm_commands)` is the hard requirement — that is what
        // consumes `ImpulseCommand` into a `ShipImpulse` phase transition, and
        // it already runs before `process_helm_inputs`, so this system cannot
        // join `ai_helm_thrust`/`ai_helm_steering` in running last.
        // `.after(AiTickLabel)` covers the scored objectives this system reads.
        // The `apply_helm_commands` edge also transitively keeps this system
        // before `ai_helm_thrust` (`apply_helm_commands` runs before
        // `process_helm_inputs`, which `ai_helm_thrust` runs after), so no
        // explicit edge between them is needed.
        app.add_systems(
            Update,
            ai_helm_impulse
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(crate::lobby::LobbySystemSet)
                .after(crate::sim_sets::AiTickLabel)
                .before(apply_helm_commands)
                // Shared AI-helm sim tick (issue #803) — one fixed-rate
                // cadence for all four per-axis systems.
                .run_if(ai_helm_tick_ready),
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
