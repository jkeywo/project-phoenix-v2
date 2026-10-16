//! Canonical enabled intent and current paid physical concealment. No observer knowledge.
use crate::{
    core::messages::SystemId,
    entities::spawner::EntityUuid,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_sim_gameplay::ship::composition::{
    concealment_composition, CommandChannelValue, SystemCommandChannels,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipConcealmentEmitters(pub BTreeMap<SystemId, SystemCommandChannels>);
#[derive(Clone, Debug)]
pub struct ConcealmentField {
    pub config: phoenix_sim_gameplay::electronic::concealment::ConcealmentConfig,
    pub paid_fraction: f32,
}
/// Paid field only. Current operating work is measured separately at each
/// existing physical sensing boundary, after the performed machinery work.
#[derive(Component, Clone, Debug, Default)]
pub struct ConcealmentFields {
    processed_tick: Option<u64>,
    entries: BTreeMap<SystemId, ConcealmentField>,
}
impl ConcealmentFields {
    pub fn perceptibility(&self, tick: u64, actual_emission: f32) -> f32 {
        if self.processed_tick != Some(tick) {
            return 1.0;
        }
        self.entries.values().next().map_or(1.0, |field| {
            field
                .config
                .perceptibility(field.paid_fraction, actual_emission)
        })
    }
}
/// Measure the actual actor's operating signature now, without storing a
/// previous-tick signature or granting any controller access to physical truth.
pub(crate) fn operating_signature(
    power: Option<&ShipPowerSystem>,
    config: Option<&ShipConfigComponent>,
    dt: f32,
) -> Option<(f32, phoenix_model::sensing::PassiveSignature)> {
    let power = power?;
    if !dt.is_finite() || dt <= 0.0 {
        return None;
    }
    let output = if power.0.network().generation_enabled {
        f64::from(power.0.network().output_actual_per_sec)
    } else {
        0.0
    };
    let machinery = config.into_iter().flat_map(|config| {
        config
            .0
            .systems
            .iter()
            .map(|system| (system.kind.as_str(), power.0.emitted_work(&system.id)))
    });
    phoenix_sim_gameplay::science::signatures::observed_signature(output, machinery, dt)
}
pub(crate) fn perceptibility(
    fields: Option<&ConcealmentFields>,
    power: Option<&ShipPowerSystem>,
    config: Option<&ShipConfigComponent>,
    tick: u64,
    dt: f32,
) -> f32 {
    if power.is_none() || config.is_none() {
        return 1.0;
    }
    fields.map_or(1.0, |field| {
        field.perceptibility(
            tick,
            operating_signature(power, config, dt).map_or(0.0, |signature| signature.0),
        )
    })
}
impl ShipConcealmentEmitters {
    pub fn from_ship(ship: &crate::ship::config::ShipConfig) -> Option<Self> {
        let rows: BTreeMap<_, _> = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::CONCEALMENT_KIND)
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
            .filter(|system| system.kind == crate::ship::system_registry::CONCEALMENT_KIND)
            .map(|system| &system.id)
            .collect();
        if owners != self.0.keys().collect() {
            return Err("concealment emitter owner roster differs from accepted hardware".into());
        }
        for (owner, channels) in &self.0 {
            let system = ship
                .system(owner)
                .ok_or("unknown concealment emitter System")?;
            phoenix_sim_gameplay::ship::composition::validate_composition(system)?;
            let binding = concealment_composition(&system.components)
                .ok_or("missing concealment emitter composition")?;
            if channels.0.len() != system.channels.len()
                || !matches!(
                    channels.0.get(binding.enabled_channel),
                    Some(CommandChannelValue::Boolean(_))
                )
            {
                return Err(
                    "concealment emitter intent differs from its exact Boolean channel".into(),
                );
            }
        }
        Ok(())
    }
    pub fn enabled(&self, ship: &crate::ship::config::ShipConfig, owner: &SystemId) -> bool {
        ship.system(owner)
            .and_then(|system| concealment_composition(&system.components))
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
        .and_then(ShipConcealmentEmitters::from_ship)
    {
        commands.insert((state, ConcealmentFields::default()));
    }
}
/// Every enabled actual System pays once, even if no observer receives it.
/// A partial grant produces only its paid fraction of the configured field.
#[allow(clippy::type_complexity)]
pub(crate) fn tick(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    mut owners: Query<(
        Entity,
        &EntityUuid,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipConcealmentEmitters,
        &mut ConcealmentFields,
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
                .and_then(|system| concealment_composition(&system.components))
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
            if !grant.fraction.is_finite()
                || grant.fraction <= 0.0
                || grant.fraction > 1.0
                || grant.granted <= 0.0
            {
                continue;
            }
            emissions.entries.insert(
                owner.clone(),
                ConcealmentField {
                    config: binding.config.clone(),
                    paid_fraction: grant.fraction,
                },
            );
        }
    }
}
#[path = "concealment_control.rs"]
mod control;
pub(crate) use control::project;

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    use crate::sim_sets::{FixedStep, SimSet};
    app.declare_state::<ShipConcealmentEmitters>(
        StateClass::Folded,
        "ship-concealment-emitter-intent",
    )
    .declare_state::<ConcealmentFields>(StateClass::Derived, "concealment-current-paid-fields");
    app.register_admitted_consumer(
        ConsumerMatcher::kind_payload(
            crate::ship::system_registry::CONCEALMENT_KIND,
            Payload::SetConcealmentEnabled,
        )
        .with_feedback(
            FeedbackAddress::DeclaredKindOnly,
            &[Payload::SetConcealmentEnabled],
        )
        .with_capability_ownership(),
    );
    crate::ai::cadence::register_ai_cadence(app);
    app.add_systems(
        FixedUpdate,
        (
            control::operate_ai
                .in_set(FixedStep::OperateConcealmentAi)
                .in_set(SimSet::Input)
                .before(control::handle_commands)
                .before(FixedStep::OperateSignalProtectionAi)
                .run_if(crate::ai::cadence::ai_tick_ready),
            control::handle_commands
                .in_set(FixedStep::HandleConcealmentCommands)
                .in_set(SimSet::Input)
                .before(tick),
            tick.in_set(FixedStep::TickConcealment)
                .in_set(SimSet::Physics)
                .after(FixedStep::TickPowerSystem)
                .before(FixedStep::TickCommsRadio),
        ),
    );
}

#[cfg(test)]
#[path = "concealment_tests.rs"]
pub(crate) mod tests;
