use bevy::prelude::*;

use crate::server_app::Ship;
use crate::server_app::{ShipBoost, ShipImpulse};
use crate::ship::components::{
    BoostConfigResource, ImpulseConfigResource, ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::helm::{ImpulseCommand, SteeringInput, ThrustInput};

/// Hull-damage impulse auto-cancel (issue #695, reshaped by #824): writes an
/// `Idle` `ImpulseCommand` intent when a ship's hull took damage this
/// tick, rather than mutating `ShipImpulse` directly. The shared
/// `apply_helm_commands` system applies the actual `cancel_charge`
/// transition.
///
/// The admitted `StartImpulseCharge`/`CancelImpulse` loop this system used to
/// carry moved to `ship::helm_admission::process_helm_inputs` (issue #824),
/// which applies impulse commands for every ship — human-admitted and
/// AI-emitted alike — with the same `BlocksImpulse` region gate. This system
/// runs in `SimSet::Input`, before `process_helm_inputs` (`SimSet::Physics`),
/// so an admitted command can still override the hull-damage cancel within
/// the same tick — matching the old sequential direct-mutation order exactly.
pub fn handle_impulse_messages(
    mut ships: Query<
        (
            &mut crate::server_app::ImpulseHullHistory,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&mut ImpulseCommand>,
            Option<&mut crate::ship::helm::DriveCommandWrites>,
        ),
        With<ShipImpulse>,
    >,
) {
    for (mut history, hull, command, writes) in &mut ships {
        let current_hp = hull.map_or(100.0, |hull| hull.0.total_current());
        let damaged = history.0.is_some_and(|previous| current_hp < previous);
        history.0 = Some(current_hp);
        if damaged {
            if let Some(mut command) = command {
                command.0 = crate::ship::impulse::ImpulsePhase::Idle;
                if let Some(mut writes) = writes {
                    writes.impulse = true;
                }
            }
        }
    }
}

/// Availability for the authored drive instance, with the ordinary legacy ID fallback.
pub fn drive_available(
    sources: Option<&ShipSystemControlSources>,
    config: Option<&ShipConfigComponent>,
    kind: &str,
    fallback: crate::core::messages::SystemId,
) -> bool {
    let id =
        crate::ship::helm_admission::authored_system_id_for_kind(config, kind).unwrap_or(&fallback);
    sources.is_none_or(|sources| sources.0.policy_for(id).coordinate)
}

pub fn tick_impulse(
    time: Res<Time>,
    mut ships_q: Query<
        (
            &mut ShipImpulse,
            Option<&ImpulseConfigResource>,
            Option<&ShipSystemControlSources>,
            Option<&ShipConfigComponent>,
        ),
        With<Ship>,
    >,
) {
    let dt = time.delta_secs();
    for (mut impulse, entity_cfg, sources, config) in ships_q.iter_mut() {
        if !drive_available(
            sources,
            config,
            crate::ship::system_registry::HELM_IMPULSE_KIND,
            crate::ship::system_registry::helm_impulse_system_id(),
        ) {
            impulse.0.cancel_charge();
            continue;
        }
        let charge_duration = entity_cfg.cloned().unwrap_or_default().charge_duration;
        impulse.0.tick(dt, charge_duration);
    }
}

// `handle_boost_messages` was retired by issue #881 into
// `ship::helm_admission::process_helm_inputs`, the shared per-entity applier
// that already carried the impulse payloads (issue #824 did the same
// retirement for `handle_impulse_messages`' admission loop). It was
// `With<LocalShip>` and read a single entity, so the admitted `SetBoost` that
// `ai_helm_boost` (#780) emits for a non-local `AiHighFidelity` NPC never
// reached `BoostCommand`. `BoostCommand` now has exactly one writer.

fn normalized_boost_drain_factor(thrust: f32, steering: f32) -> f32 {
    thrust.clamp(-1.0, 1.0).abs() + steering.clamp(-1.0, 1.0).abs()
}

/// Boost depletion is authoritative for every ship replica. Local crew sessions
/// and the HUD's LastHelmInput cache differ by host and cannot drive this fold.
/// The admitted per-entity actuator latches are the same inputs physics consumes.
pub fn tick_boost(
    time: Res<Time>,
    mut ships: Query<
        (
            Option<&BoostConfigResource>,
            &mut ShipBoost,
            Option<&ThrustInput>,
            Option<&SteeringInput>,
            Option<&ShipImpulse>,
        ),
        With<Ship>,
    >,
) {
    for (entity_cfg, mut boost, thrust, steering, impulse) in &mut ships {
        let config = entity_cfg.cloned().unwrap_or_default();
        if !config.enabled {
            continue;
        }
        let drain_factor = if impulse.is_some_and(|state| state.0.is_active()) {
            normalized_boost_drain_factor(1.0, 0.0)
        } else {
            normalized_boost_drain_factor(
                thrust.map_or(0.0, |input| input.0),
                steering.map_or(0.0, |input| input.0),
            )
        };
        boost.0.tick_with_drain_factor(
            time.delta_secs(),
            config.active_duration,
            config.recharge_duration,
            drain_factor,
        );
    }
}
