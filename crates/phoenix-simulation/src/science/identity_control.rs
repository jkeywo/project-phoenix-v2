//! Normal per-System toggle admission and sole ordinary state-tree host.
use super::*;
use crate::core::messages::{AdmittedCommands, SystemControlPayload as Control};

pub(crate) fn handle_commands(
    mut owners: Query<(
        &AdmittedCommands,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &mut ShipIdentityEmitters,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::server::OutboundMessage>>>,
) {
    for (commands, config, sources, mut state) in &mut owners {
        for command in &commands.0 {
            let Some(system) = config.0.system(&command.target).filter(|system| {
                system.kind == crate::ship::system_registry::IDENTITY_EMITTER_KIND
            }) else {
                continue;
            };
            let Some(binding) = identity_composition(&system.components) else {
                continue;
            };
            let Control::SetIdentityEmitterEnabled { enabled } = &command.payload else {
                continue;
            };
            let value = Some((
                binding.enabled_channel,
                CommandChannelValue::Boolean(*enabled),
            ));
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
        &ShipIdentityEmitters,
        Option<&crate::server_app::ShipSystemBlackboards>,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, config, sources, state, boards, programs, mut runtimes, mut admitted) in &mut owners
    {
        for owner in state.0.keys() {
            if !crate::ai::host::ai_operates(&sources.0, owner) {
                continue;
            }
            let Some(binding) = config
                .0
                .system(owner)
                .and_then(|system| identity_composition(&system.components))
            else {
                continue;
            };
            let Some(runtime) = runtimes.0.get_mut(owner) else {
                continue;
            };
            let enabled = state.enabled(&config.0, owner);
            let mut facts = BTreeMap::from([("enabled".into(), enabled)]);
            facts.extend(crate::console::command::emission::policy_facts(
                &config.0, boards, owner,
            ));
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
                    Control::SetIdentityEmitterEnabled { enabled: *next },
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
) -> Option<phoenix_model::identity_ui::IdentityControlsView> {
    let config = world.get::<ShipConfigComponent>(entity)?;
    let state = world.get::<ShipIdentityEmitters>(entity)?;
    let binding = identity_composition(&config.0.system(owner)?.components)?;
    if !state.0.contains_key(owner) {
        return None;
    }
    let active = world
        .get::<IdentityEmissions>(entity)
        .zip(world.get_resource::<crate::sim_tick::SimTick>())
        .is_some_and(|(emissions, tick)| {
            emissions.processed_tick == Some(tick.0) && emissions.entries.contains_key(owner)
        });
    Some(phoenix_model::identity_ui::IdentityControlsView {
        enabled: state.enabled(&config.0, owner),
        active,
        callsign: binding.config.callsign.clone(),
        faction: binding.config.faction(),
        controllable: world
            .get::<ShipSystemControlSources>(entity)
            .is_some_and(|sources| {
                sources.0.entries().any(|(actual, _)| actual == owner)
                    && sources.0.policy_for(owner).accept_human_input
            }),
    })
}
