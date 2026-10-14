//! The equipped entity's one physical damage owner. Props retain their separate
//! legacy hull owner; no equipped entity spends or restores legacy System HP.
pub use super::compartment_damage::{DamageDefinition, DamageState};
use crate::core::messages::SystemId;
use bevy::prelude::*;
use phoenix_model::damage::{DamageBinding, DamageHit, DamageOutcome};

#[derive(Component, Clone, Debug)]
pub struct ShipDamage {
    pub definition: DamageDefinition,
    pub state: DamageState,
}

impl ShipDamage {
    pub fn from_config(
        config: &crate::entities::config::EntityConfig,
    ) -> Result<Option<Self>, String> {
        let Some(damage) = &config.damage else {
            return Ok(None);
        };
        let definition = DamageDefinition::compile(
            config
                .interior
                .as_ref()
                .ok_or("damage requires a physical interior")?,
            damage,
            config
                .ship_config
                .as_ref()
                .ok_or("damage requires installed Systems")?,
        )?;
        let state = DamageState::new(&definition);
        Ok(Some(Self { definition, state }))
    }

    pub fn structural_fraction(&self) -> f32 {
        self.state.structural_fraction(&self.definition)
    }
    pub fn factor(&self, system: &SystemId, binding: DamageBinding) -> f32 {
        self.state.effect_factor(&self.definition, system, binding)
    }
    pub fn operational(&self, system: &SystemId) -> bool {
        !self.state.catastrophic && self.factor(system, DamageBinding::Operation) > 0.0
    }
    pub fn catastrophic(&self) -> bool {
        self.state.catastrophic
    }

    /// A recoverable ship failure. Usable charged buffers disprove power loss;
    /// neither commanded output nor allocation/throttle settings establish it.
    /// Intentionally engineless equipment does not fail this propulsion test.
    pub fn disabled(
        &self,
        config: &crate::ship::config::ShipConfig,
        power: Option<&crate::ship::power::ShipPowerSystem>,
    ) -> bool {
        use crate::ship::system_registry::*;
        let propulsion: Vec<_> = config
            .systems
            .iter()
            .filter(|system| {
                matches!(
                    system.kind.as_str(),
                    HELM_ENGINE_KIND
                        | HELM_THRUST_KIND
                        | HELM_IMPULSE_KIND
                        | LATERAL_THRUST_KIND
                        | VERTICAL_THRUST_KIND
                )
            })
            .collect();
        let generation: Vec<_> = config
            .systems
            .iter()
            .filter(|system| system.kind == POWER_REACTOR_KIND)
            .collect();
        if propulsion.is_empty()
            || generation.is_empty()
            || propulsion.iter().any(|system| self.operational(&system.id))
        {
            return false;
        }
        let Some(power) = power else {
            return false;
        };
        let generation_lost = power.0.network().reactor.thermal_stopped
            || generation.iter().all(|system| {
                !self.operational(&system.id)
                    || self.factor(&system.id, DamageBinding::Intake) == 0.0
            });
        if !generation_lost {
            return false;
        }
        let reactor_usable = !power.0.network().reactor.thermal_stopped
            && power.0.network().reactor.energy > 0.0
            && generation.iter().any(|system| self.operational(&system.id));
        let local_usable = power.0.network().consumers.iter().any(|(system, state)| {
            state.energy > 0.0 && !state.thermal_stopped && self.operational(system)
        });
        !reactor_usable && !local_usable
    }

    pub fn structural_totals(&self) -> (f64, f64) {
        (
            self.state.compartments.values().map(|room| room.hp).sum(),
            self.definition
                .interior
                .compartments
                .iter()
                .map(|room| room.max_hp as f64)
                .sum(),
        )
    }

    /// Qualitative operating symptoms, not another System HP simulation or a
    /// disclosure of hidden component causes. Used by existing status/reports.
    pub fn operating_tier(&self, system: &SystemId) -> crate::ship::damage::DamageTier {
        use crate::ship::damage::DamageTier;
        use phoenix_model::damage::ComponentCondition;
        if self.catastrophic() {
            return DamageTier::Destroyed;
        }
        let Some(room) = self
            .definition
            .interior
            .placements
            .get(system)
            .and_then(|room| self.state.compartments.get(room))
        else {
            return DamageTier::Disabled;
        };
        if room.disabled {
            return DamageTier::Disabled;
        }
        if self.factor(system, DamageBinding::Operation) == 0.0 {
            return DamageTier::Destroyed;
        }
        if self.state.components.get(system).is_some_and(|components| {
            components
                .values()
                .any(|component| component.condition != ComponentCondition::Healthy)
        }) {
            DamageTier::Damaged
        } else {
            DamageTier::Operational
        }
    }

    /// Resolve an actual incident synchronously in its existing producer phase.
    /// The vector is world-plane travel direction; yaw transforms it locally.
    pub fn apply_hit(
        &mut self,
        yaw: f32,
        mut hit: DamageHit,
        rng: &mut vellum_rng::Pcg32,
    ) -> Result<DamageOutcome, String> {
        hit.incoming = local_hit_direction(yaw, hit.incoming);
        self.state.apply_hit(&self.definition, &hit, rng)
    }
}

/// Simulation yaw turns local forward -Z into [sin(yaw), -cos(yaw)].
/// Invert that orientation for physical planar travel; render Euler signs are
/// deliberately absent from this calculation.
fn local_hit_direction(yaw: f32, incoming: [f32; 2]) -> [f32; 2] {
    let (sin, cos) = (crate::simmath::sin(yaw), crate::simmath::cos(yaw));
    [
        incoming[0] * cos + incoming[1] * sin,
        incoming[1] * cos - incoming[0] * sin,
    ]
}

/// Existing wire hull summaries may use this derived view. It cannot determine
/// equipped-entity availability, repairs or destruction.
pub fn structural_fraction(
    damage: Option<&ShipDamage>,
    prop: Option<&crate::entities::spawner::EntitySystemHull>,
) -> Option<f32> {
    damage.map(ShipDamage::structural_fraction).or_else(|| {
        prop.map(|hull| {
            let (current, maximum) = (hull.0.total_current(), hull.0.total_max());
            if maximum > 0.0 {
                (current / maximum).clamp(0.0, 1.0)
            } else {
                1.0
            }
        })
    })
}

pub fn structural_totals(
    damage: Option<&ShipDamage>,
    prop: Option<&crate::entities::spawner::EntitySystemHull>,
) -> Option<(f32, f32)> {
    damage
        .map(|damage| {
            let (current, maximum) = damage.structural_totals();
            (current as f32, maximum as f32)
        })
        .or_else(|| prop.map(|hull| (hull.0.total_current(), hull.0.total_max())))
}

/// Explicit branching between the physical equipment owner and legacy props.
/// A recoverable structural zero is never physical destruction.
pub fn is_catastrophic(
    damage: Option<&ShipDamage>,
    prop: Option<&crate::entities::spawner::EntitySystemHull>,
) -> bool {
    damage.map_or_else(
        || prop.is_some_and(|hull| hull.0.is_destroyed()),
        ShipDamage::catastrophic,
    )
}

pub fn operating_tier(
    damage: Option<&ShipDamage>,
    prop: Option<&crate::entities::spawner::EntitySystemHull>,
    system: &SystemId,
) -> crate::ship::damage::DamageTier {
    damage.map_or_else(
        || {
            prop.map_or(crate::ship::damage::DamageTier::Operational, |hull| {
                hull.0.tier_for(system)
            })
        },
        |damage| damage.operating_tier(system),
    )
}

/// Intake/cooling faults do not turn off already charged working equipment.
pub fn operation_disabled(
    damage: Option<&ShipDamage>,
    prop: Option<&crate::entities::spawner::EntitySystemHull>,
    system: &SystemId,
) -> bool {
    if let Some(damage) = damage {
        !damage.operational(system)
    } else {
        prop.is_some_and(|hull| {
            matches!(
                hull.0.tier_for(system),
                crate::ship::damage::DamageTier::Disabled
                    | crate::ship::damage::DamageTier::Destroyed
            )
        })
    }
}

#[cfg(test)]
#[path = "physical_damage_tests.rs"]
mod tests;
