//! Sole retained protection intent and current paid capability grants. Allocation
//! has no exposure input; no receiver history or reusable work credit is saved.
use crate::{
    core::messages::SystemId,
    entities::spawner::EntityUuid,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_sim_gameplay::ship::composition::{
    protection_composition, CommandChannelValue, SystemCommandChannels,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipSignalProtection(pub BTreeMap<SystemId, SystemCommandChannels>);
#[derive(Clone, Debug)]
struct PaidProtection {
    receivers: Vec<SystemId>,
    suppression: f64,
}
/// Ephemeral physical grant, never copied into snapshots or Console facts.
#[derive(Component, Clone, Debug, Default)]
pub struct ProtectionGrants {
    processed_tick: Option<u64>,
    entries: BTreeMap<SystemId, PaidProtection>,
}
impl ProtectionGrants {
    /// Only an exact actual receiver owner can consume its own current grant.
    /// Missing work is neutral, including bootstrap/restore and late spawns.
    pub fn suppression_for(&self, receiver: &SystemId, tick: u64) -> f64 {
        if self.processed_tick != Some(tick) {
            return 0.0;
        }
        self.entries
            .values()
            .filter(|grant| grant.receivers.contains(receiver))
            .map(|grant| grant.suppression)
            .sum()
    }
    #[cfg(test)]
    pub(crate) fn is_unprocessed(&self) -> bool {
        self.processed_tick.is_none() && self.entries.is_empty()
    }
}
impl ShipSignalProtection {
    pub fn from_ship(ship: &crate::ship::config::ShipConfig) -> Option<Self> {
        let owners: BTreeMap<_, _> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::SIGNAL_PROTECTION_KIND)
            .map(|system| {
                (
                    system.id.clone(),
                    SystemCommandChannels::from_definitions(&system.channels),
                )
            })
            .collect();
        (!owners.is_empty()).then_some(Self(owners))
    }
    pub fn validate(&self, ship: &crate::ship::config::ShipConfig) -> Result<(), String> {
        let owners: BTreeSet<_> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::SIGNAL_PROTECTION_KIND)
            .map(|system| &system.id)
            .collect();
        if owners != self.0.keys().collect() {
            return Err("protection owner roster differs from accepted equipment".into());
        }
        for (owner, channels) in &self.0 {
            let system = ship.system(owner).ok_or("missing protection System")?;
            phoenix_sim_gameplay::ship::composition::validate_composition(system)?;
            let binding = protection_composition(&system.components)
                .ok_or("missing protection composition")?;
            if channels.0.len() != system.channels.len() {
                return Err("protection channels differ from actual equipment".into());
            }
            if !matches!(
                channels.0.get(binding.enabled_channel),
                Some(CommandChannelValue::Boolean(_))
            ) {
                return Err("protection enabled channel has invalid type".into());
            }
            let Some(CommandChannelValue::SystemPriority(priorities)) =
                channels.0.get(binding.priorities_channel)
            else {
                return Err("protection priorities channel has invalid type".into());
            };
            binding.config.validate_priorities(priorities)?;
        }
        Ok(())
    }
    pub fn enabled(&self, ship: &crate::ship::config::ShipConfig, owner: &SystemId) -> bool {
        ship.system(owner)
            .and_then(|system| protection_composition(&system.components))
            .zip(self.0.get(owner))
            .is_some_and(|(binding, channels)| channels.boolean(binding.enabled_channel))
    }
    pub fn priorities<'a>(
        &'a self,
        ship: &crate::ship::config::ShipConfig,
        owner: &SystemId,
    ) -> &'a [SystemId] {
        let Some(binding) = ship
            .system(owner)
            .and_then(|system| protection_composition(&system.components))
        else {
            return &[];
        };
        self.0.get(owner).map_or(&[], |channels| {
            channels.priorities(binding.priorities_channel)
        })
    }
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(state) = config
        .ship_config
        .as_ref()
        .and_then(ShipSignalProtection::from_ship)
    {
        commands.insert((state, ProtectionGrants::default()));
    }
}
/// Input work is paid once even if no interference or no receivers are selected.
/// No spatial component is required for this onboard equipment consumer.
#[allow(clippy::type_complexity)]
pub(crate) fn tick(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    mut owners: Query<(
        Entity,
        &EntityUuid,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipSignalProtection,
        &mut ProtectionGrants,
        Option<&mut ShipPowerSystem>,
        Option<&ShipDamage>,
        Option<&crate::resources::fitting_server::CompiledShipFitting>,
        Option<&crate::resources::fitting_server::ShipFitting>,
    )>,
) {
    let mut order: Vec<_> = owners.iter().map(|row| (row.1 .0.clone(), row.0)).collect();
    order.sort();
    for (_, entity) in order {
        let Ok((_, _, config, sources, state, mut grants, power, damage, fitting_profile, fitting)) =
            owners.get_mut(entity)
        else {
            continue;
        };
        if grants.processed_tick == Some(tick.0) {
            continue;
        }
        grants.processed_tick = Some(tick.0);
        grants.entries.clear();
        let dt = fixed.timestep().as_secs_f32();
        if !dt.is_finite() || dt <= 0.0 || state.validate(&config.0).is_err() {
            continue;
        }
        let (Some(mut power), Some(damage)) = (power, damage) else {
            continue;
        };
        for owner in state.0.keys() {
            if !state.enabled(&config.0, owner)
                || sources.0.is_offline(owner)
                || sources.0.source_for(owner)
                    == crate::ship::control_source::ControlSource::Offline
                || !sources.0.entries().any(|(id, _)| id == owner)
                || fitting_profile
                    .zip(fitting)
                    .is_some_and(|(profile, state)| {
                        crate::resources::fitting_server::unavailable(profile, state, owner)
                    })
            {
                continue;
            }
            let Some(binding) = config
                .0
                .system(owner)
                .and_then(|system| protection_composition(&system.components))
            else {
                continue;
            };
            let Ok(receivers) = binding.config.allocate(state.priorities(&config.0, owner)) else {
                continue;
            };
            let operation = damage.factor(owner, phoenix_model::damage::DamageBinding::Operation);
            if !operation.is_finite() || operation <= 0.0 {
                continue;
            }
            let Ok(grant) = power.0.work_continuous(owner, dt, 1.0 / operation) else {
                continue;
            };
            if grant.granted <= 0.0 {
                continue;
            }
            let Ok(suppression) = binding.config.paid_suppression(grant.fraction) else {
                continue;
            };
            if suppression > 0.0 {
                grants.entries.insert(
                    owner.clone(),
                    PaidProtection {
                        receivers,
                        suppression,
                    },
                );
            }
        }
    }
}
pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::sim_sets::{FixedStep, SimSet};
    app.declare_state::<ShipSignalProtection>(StateClass::Folded, "ship-signal-protection-intent")
        .declare_state::<ProtectionGrants>(StateClass::Derived, "signal-protection-paid-grants");
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    crate::ai::cadence::register_ai_cadence(app);
    for (payload, feedback) in [
        (
            Payload::SetSignalProtectionEnabled,
            &[Payload::SetSignalProtectionEnabled] as &'static [_],
        ),
        (
            Payload::SetSignalProtectionPriorities,
            &[Payload::SetSignalProtectionPriorities] as &'static [_],
        ),
    ] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(
                crate::ship::system_registry::SIGNAL_PROTECTION_KIND,
                payload,
            )
            .with_feedback(FeedbackAddress::DeclaredKindOnly, feedback)
            .with_capability_ownership(),
        );
    }
    app.add_systems(
        FixedUpdate,
        (
            control::operate_ai
                .in_set(SimSet::Input)
                .in_set(FixedStep::OperateSignalProtectionAi)
                .before(control::handle_commands)
                .run_if(crate::ai::cadence::ai_tick_ready),
            control::handle_commands
                .in_set(SimSet::Input)
                .in_set(FixedStep::HandleSignalProtectionCommands)
                .before(tick),
            tick.in_set(SimSet::Input)
                .in_set(FixedStep::TickSignalProtection)
                .before(FixedStep::TickJamming),
        ),
    );
}
#[cfg(test)]
#[path = "protection_tests.rs"]
pub(crate) mod tests;

#[path = "protection_control.rs"]
mod control;
pub(crate) use control::project;
