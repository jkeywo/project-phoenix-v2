//! Normal per-System admitted protection controls and ordinary tree host.
use super::*;
use crate::core::messages::{AdmittedCommands, SystemControlPayload as Control};

pub(crate) fn handle_commands(
    mut owners: Query<(
        &AdmittedCommands,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &mut ShipSignalProtection,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::server::OutboundMessage>>>,
) {
    for (commands, config, sources, mut state) in &mut owners {
        for command in &commands.0 {
            let Some(system) = config.0.system(&command.target).filter(|system| {
                system.kind == crate::ship::system_registry::SIGNAL_PROTECTION_KIND
            }) else {
                continue;
            };
            let Some(binding) = protection_composition(&system.components) else {
                continue;
            };
            let value = match &command.payload {
                Control::SetSignalProtectionEnabled { enabled } => Some((
                    binding.enabled_channel,
                    CommandChannelValue::Boolean(*enabled),
                )),
                Control::SetSignalProtectionPriorities { systems } => {
                    binding.config.validate_priorities(systems).ok().map(|()| {
                        (
                            binding.priorities_channel,
                            CommandChannelValue::SystemPriority(systems.clone()),
                        )
                    })
                }
                _ => continue,
            };
            let policy = sources.0.policy_for(&command.target);
            let applied = sources.0.entries().any(|(id, _)| id == &command.target)
                && (policy.accept_human_input || policy.operate_ai)
                && value.is_some_and(|(channel, value)| {
                    state
                        .0
                        .get_mut(&command.target)
                        .is_some_and(|channels| channels.set(channel, value).is_ok())
                });
            crate::command_admission::finish_admitted_action_feedback(
                &mut outbound,
                command,
                if applied {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            );
        }
    }
}
#[allow(clippy::type_complexity)]
pub(crate) fn operate_ai(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut owners: Query<(
        Option<&EntityUuid>,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipSignalProtection,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, config, sources, state, programs, mut runtimes, mut admitted) in &mut owners {
        for owner in state.0.keys() {
            if !crate::ai::host::ai_operates(&sources.0, owner) {
                continue;
            }
            let Some(binding) = config
                .0
                .system(owner)
                .and_then(|system| protection_composition(&system.components))
            else {
                continue;
            };
            let Some(runtime) = runtimes.0.get_mut(owner) else {
                continue;
            };
            let enabled = state.enabled(&config.0, owner);
            let facts = BTreeMap::from([
                ("enabled".into(), enabled),
                (
                    "has_priorities".into(),
                    !state.priorities(&config.0, owner).is_empty(),
                ),
            ]);
            let pending = BTreeMap::from([(binding.enabled_channel.into(), enabled)]);
            let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
                &sources.0,
                owner,
                programs.0.get(owner),
                &facts,
                &pending,
                runtime,
                tick.0,
            ) else {
                continue;
            };
            if let Some(next) = result
                .intents
                .get(binding.enabled_channel)
                .filter(|next| **next != enabled)
            {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    owner.clone(),
                    Control::SetSignalProtectionEnabled { enabled: *next },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}
pub(crate) fn project(
    world: &World,
    entity: Entity,
    owner: &SystemId,
) -> Option<phoenix_model::protection_ui::ProtectionControlsView> {
    let config = world.get::<ShipConfigComponent>(entity)?;
    let state = world.get::<ShipSignalProtection>(entity)?;
    let binding = protection_composition(&config.0.system(owner)?.components)?;
    if !state.0.contains_key(owner) {
        return None;
    }
    let paid = world
        .get::<ProtectionGrants>(entity)
        .zip(world.get_resource::<crate::sim_tick::SimTick>())
        .filter(|(grants, tick)| grants.processed_tick == Some(tick.0))
        .and_then(|(grants, _)| grants.entries.get(owner));
    Some(phoenix_model::protection_ui::ProtectionControlsView {
        enabled: state.enabled(&config.0, owner),
        priorities: state.priorities(&config.0, owner).to_vec(),
        capacity: binding.config.capacity,
        systems: binding
            .config
            .systems
            .iter()
            .filter_map(|(id, cost)| {
                config.0.system(id).map(|system| {
                    phoenix_model::protection_ui::ProtectionSystemOption {
                        system: id.clone(),
                        display_name: world
                            .get::<crate::entities::spawner::EntitySystemHull>(entity)
                            .and_then(|hull| hull.0.get(id))
                            .map_or_else(|| id.0.clone(), |entry| entry.display_name.clone()),
                        cost: *cost,
                        home_station: config.0.system_home_station(system).cloned(),
                    }
                })
            })
            .collect(),
        active: paid.is_some(),
        protected_systems: paid.map_or_else(Vec::new, |grant| grant.receivers.clone()),
        controllable: world
            .get::<ShipSystemControlSources>(entity)
            .is_some_and(|sources| {
                sources.0.entries().any(|(actual, _)| actual == owner)
                    && sources.0.policy_for(owner).accept_human_input
            }),
    })
}
#[cfg(test)]
#[path = "protection_control_tests.rs"]
mod tests;
