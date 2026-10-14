use bevy::prelude::*;

use crate::server_app::Ship;
use crate::server_app::{ShipBoost, ShipImpulse};
use crate::ship::components::{
    BoostConfigResource, ImpulseConfigResource, ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::helm::ImpulseCommand;

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
            Option<&crate::ship::physical_damage::ShipDamage>,
            Option<&mut ImpulseCommand>,
            Option<&mut crate::ship::helm::DriveCommandWrites>,
        ),
        With<ShipImpulse>,
    >,
) {
    for (mut history, hull, damage, command, writes) in &mut ships {
        let current_hp = crate::ship::physical_damage::structural_totals(damage, hull)
            .map_or(100.0, |(current, _)| current);
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
            Option<&mut crate::ship::power::ShipPowerSystem>,
        ),
        With<Ship>,
    >,
) {
    let dt = time.delta_secs();
    for (mut impulse, entity_cfg, sources, config, mut power) in ships_q.iter_mut() {
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
        if impulse.0.phase == crate::ship::impulse::ImpulsePhase::Charging {
            let fraction = crate::ship::energy_work::continuous_for_kind(
                power.as_deref_mut().map(|p| &mut p.0),
                crate::ship::system_registry::HELM_IMPULSE_KIND,
                dt,
                1.0,
            );
            impulse.0.tick(dt * fraction, charge_duration);
        }
    }
}

// `handle_boost_messages` was retired by issue #881 into
// `ship::helm_admission::process_helm_inputs`, the shared per-entity applier
// that already carried the impulse payloads (issue #824 did the same
// retirement for `handle_impulse_messages`' admission loop). It was
// `With<LocalShip>` and read a single entity, so the admitted `SetBoost` that
// `ai_helm_boost` (#780) emits for a non-local `AiHighFidelity` NPC never
// reached `BoostCommand`. `BoostCommand` now has exactly one writer.

/// The old boost battery field is a compatibility readout of the owning local
/// reserve. Work is paid at physics integration; this observer neither recharges
/// another battery nor clears the saved engagement intent on exhaustion.
pub fn tick_boost(
    mut ships: Query<
        (
            Option<&BoostConfigResource>,
            &mut ShipBoost,
            Option<&crate::ship::power::ShipPowerSystem>,
        ),
        With<Ship>,
    >,
) {
    for (entity_cfg, mut boost, power) in &mut ships {
        let config = entity_cfg.cloned().unwrap_or_default();
        if !config.enabled {
            continue;
        }
        boost.0.battery = power
            .and_then(|p| {
                let id = &p
                    .0
                    .ship_config()
                    .systems
                    .iter()
                    .find(|system| system.kind == crate::ship::system_registry::HELM_BOOST_KIND)?
                    .id;
                let state = p.0.network().consumers.get(id)?;
                let profile =
                    p.0.config()
                        .consumers
                        .iter()
                        .find(|profile| &profile.system == id)?;
                Some(state.energy / profile.reserve.capacity)
            })
            .unwrap_or(0.0);
    }
}
