//! Actual transponder intent and current paid identity. No physical truth changes.
use crate::{
    core::messages::SystemId,
    entities::spawner::EntityUuid,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_sim_gameplay::ship::composition::{
    identity_composition, CommandChannelValue, SystemCommandChannels,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipIdentityEmitters(pub BTreeMap<SystemId, SystemCommandChannels>);
#[derive(Clone, Debug)]
pub struct IdentityClaim {
    pub callsign: String,
    pub faction: Option<String>,
}
/// Current physical emission only; never saved as prepaid work or published as a
/// false-versus-real bit. Broad uses this only after its normal acquisition gate.
#[derive(Component, Clone, Debug, Default)]
pub struct IdentityEmissions {
    processed_tick: Option<u64>,
    entries: BTreeMap<SystemId, IdentityClaim>,
}
impl IdentityEmissions {
    pub fn claim(&self, tick: u64) -> Option<&IdentityClaim> {
        (self.processed_tick == Some(tick))
            .then(|| self.entries.values().next())
            .flatten()
    }
}
impl ShipIdentityEmitters {
    pub fn from_ship(ship: &crate::ship::config::ShipConfig) -> Option<Self> {
        let rows: BTreeMap<_, _> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::IDENTITY_EMITTER_KIND)
            .map(|system| {
                (
                    system.id.clone(),
                    SystemCommandChannels::from_definitions(&system.channels),
                )
            })
            .collect();
        (!rows.is_empty()).then_some(Self(rows))
    }
    pub fn validate(&self, ship: &crate::ship::config::ShipConfig) -> Result<(), String> {
        let owners: BTreeSet<_> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::IDENTITY_EMITTER_KIND)
            .map(|system| &system.id)
            .collect();
        if owners != self.0.keys().collect() {
            return Err("identity emitter owner roster differs from accepted hardware".into());
        }
        for (owner, channels) in &self.0 {
            let system = ship
                .system(owner)
                .ok_or("unknown identity emitter System")?;
            phoenix_sim_gameplay::ship::composition::validate_composition(system)?;
            let binding = identity_composition(&system.components)
                .ok_or("missing identity emitter composition")?;
            if channels.0.len() != system.channels.len()
                || !matches!(
                    channels.0.get(binding.enabled_channel),
                    Some(CommandChannelValue::Boolean(_))
                )
            {
                return Err(
                    "identity emitter intent differs from its exact Boolean channel".into(),
                );
            }
        }
        Ok(())
    }
    pub fn enabled(&self, ship: &crate::ship::config::ShipConfig, owner: &SystemId) -> bool {
        ship.system(owner)
            .and_then(|system| identity_composition(&system.components))
            .zip(self.0.get(owner))
            .is_some_and(|(binding, channels)| channels.boolean(binding.enabled_channel))
    }
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(state) = config
        .ship_config
        .as_ref()
        .and_then(ShipIdentityEmitters::from_ship)
    {
        commands.insert((state, IdentityEmissions::default()));
    }
}
/// Every enabled actual System pays once, even if no observer receives it.
/// Only fully paid work produces this first binary identity claim.
#[allow(clippy::type_complexity)]
pub(crate) fn tick(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    mut owners: Query<(
        Entity,
        &EntityUuid,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipIdentityEmitters,
        &mut IdentityEmissions,
        Option<&mut ShipPowerSystem>,
        Option<&ShipDamage>,
        Option<&crate::resources::fitting_server::CompiledShipFitting>,
        Option<&crate::resources::fitting_server::ShipFitting>,
    )>,
) {
    let mut order: Vec<_> = owners.iter().map(|row| (row.1 .0.clone(), row.0)).collect();
    order.sort();
    for (_, entity) in order {
        let Ok((
            _,
            _,
            config,
            sources,
            state,
            mut emissions,
            power,
            damage,
            fitting_profile,
            fitting,
        )) = owners.get_mut(entity)
        else {
            continue;
        };
        if emissions.processed_tick == Some(tick.0) {
            continue;
        }
        emissions.processed_tick = Some(tick.0);
        emissions.entries.clear();
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
                || match (fitting_profile, fitting) {
                    (Some(profile), Some(state)) => {
                        crate::resources::fitting_server::unavailable(profile, state, owner)
                    }
                    (None, None) => false,
                    _ => true,
                }
            {
                continue;
            }
            let Some(binding) = config
                .0
                .system(owner)
                .and_then(|system| identity_composition(&system.components))
            else {
                continue;
            };
            let operation = damage.factor(owner, phoenix_model::damage::DamageBinding::Operation);
            if !operation.is_finite() || operation <= 0.0 {
                continue;
            }
            let Ok(grant) = power.0.work_continuous(owner, dt, 1.0 / operation) else {
                continue;
            };
            if grant.fraction != 1.0 || grant.granted <= 0.0 {
                continue;
            }
            emissions.entries.insert(
                owner.clone(),
                IdentityClaim {
                    callsign: binding.config.callsign.clone(),
                    faction: binding.config.faction(),
                },
            );
        }
    }
}
#[path = "identity_control.rs"]
mod control;
pub(crate) use control::project;

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    use crate::sim_sets::{FixedStep, SimSet};
    app.declare_state::<ShipIdentityEmitters>(StateClass::Folded, "ship-identity-emitter-intent")
        .declare_state::<IdentityEmissions>(
            StateClass::Derived,
            "identity-emitter-current-paid-claims",
        );
    app.register_admitted_consumer(
        ConsumerMatcher::kind_payload(
            crate::ship::system_registry::IDENTITY_EMITTER_KIND,
            Payload::SetIdentityEmitterEnabled,
        )
        .with_feedback(
            FeedbackAddress::DeclaredKindOnly,
            &[Payload::SetIdentityEmitterEnabled],
        ),
    );
    crate::ai::cadence::register_ai_cadence(app);
    app.add_systems(
        FixedUpdate,
        (
            control::operate_ai
                .in_set(SimSet::Input)
                .in_set(FixedStep::OperateIdentityEmitterAi)
                .before(control::handle_commands)
                .run_if(crate::ai::cadence::ai_tick_ready),
            control::handle_commands
                .in_set(SimSet::Input)
                .in_set(FixedStep::HandleIdentityEmitterCommands)
                .before(tick),
            tick.in_set(SimSet::Input)
                .in_set(FixedStep::TickIdentityEmitters),
        ),
    );
}

#[cfg(test)]
#[path = "identity_tests.rs"]
pub(crate) mod tests;
