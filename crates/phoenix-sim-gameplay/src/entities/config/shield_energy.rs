//! Resolve shield work against existing equipment; never manufacture a battery.
use super::{EnergyConfigSection, ShieldArcConfig};
use crate::core::messages::SystemId;
use crate::ship::config::ShipConfig;
use crate::ship::system_registry::{shield_arc_system_id, SHIELD_ARC_KIND};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_shield_arc_energy(
    arcs: &[ShieldArcConfig],
    ship: &ShipConfig,
    energy: &EnergyConfigSection,
) -> Result<(), String> {
    let arcs_by_system: BTreeMap<SystemId, &ShieldArcConfig> = arcs
        .iter()
        .filter_map(|arc| shield_arc_system_id(&arc.id).map(|id| (id, arc)))
        .collect();
    for arc in arcs {
        let owner = format!("shield_arc[{}].energy", arc.id);
        arc.energy
            .validate()
            .map_err(|error| format!("{owner}: {error}"))?;
        let mut seen = BTreeSet::new();
        for target in &arc.energy.recovery_targets {
            if !seen.insert(target) {
                return Err(format!("{owner}.recovery_targets repeats {}", target.0));
            }
            let recipient = arcs_by_system.get(target).ok_or_else(|| {
                format!(
                    "{owner}.recovery_targets names unknown shield arc {}",
                    target.0
                )
            })?;
            if !ship
                .systems
                .iter()
                .any(|system| system.id == *target && system.kind == SHIELD_ARC_KIND)
                || !energy
                    .consumers
                    .iter()
                    .any(|consumer| consumer.system == *target)
            {
                return Err(format!(
                    "{owner}.recovery_targets {} requires a shield arc System with a canonical energy reserve",
                    target.0
                ));
            }
            if arc.energy.recovery_energy_per_hp > 0.0
                && arc.energy.recovery_energy_per_hp >= recipient.energy.recharge_energy_per_hp
            {
                return Err(format!(
                    "{owner}.recovery_energy_per_hp must be strictly below recipient {} recharge_energy_per_hp",
                    target.0
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "shield_energy_tests.rs"]
mod tests;
