//! Normal per-System toggle admission and sole ordinary state-tree host.
use super::*;
use crate::core::messages::{AdmittedCommands, SystemControlPayload as Control};

pub(crate) fn handle_commands(
    mut owners: Query<(
        &AdmittedCommands,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &mut ShipConcealmentEmitters,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::server::OutboundMessage>>>,
) {
    for (commands, config, sources, mut state) in &mut owners {
        for command in &commands.0 {
            let Some(system) = config
                .0
                .system(&command.target)
                .filter(|system| system.kind == crate::ship::system_registry::CONCEALMENT_KIND)
            else {
                continue;
            };
            let Some(binding) = concealment_composition(&system.components) else {
                continue;
            };
            let Control::SetConcealmentEnabled { enabled } = &command.payload else {
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
        &ShipConcealmentEmitters,
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
                .and_then(|system| concealment_composition(&system.components))
            else {
                continue;
            };
            let Some(runtime) = runtimes.0.get_mut(owner) else {
                continue;
            };
            let enabled = state.enabled(&config.0, owner);
            let facts = BTreeMap::from([("enabled".into(), enabled)]);
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
                    Control::SetConcealmentEnabled { enabled: *next },
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
) -> Option<phoenix_model::concealment_ui::ConcealmentControlsView> {
    let config = world.get::<ShipConfigComponent>(entity)?;
    let state = world.get::<ShipConcealmentEmitters>(entity)?;
    concealment_composition(&config.0.system(owner)?.components)?;
    if !state.0.contains_key(owner) {
        return None;
    }
    let tick = world.get_resource::<crate::sim_tick::SimTick>()?.0;
    let dt = world
        .get_resource::<Time<Fixed>>()?
        .timestep()
        .as_secs_f32();
    let signature = operating_signature(world.get::<ShipPowerSystem>(entity), Some(config), dt);
    let field = world
        .get::<ConcealmentFields>(entity)
        .filter(|fields| fields.processed_tick == Some(tick))
        .and_then(|fields| fields.entries.get(owner));
    let emission = signature.map_or(0.0, |signature| signature.0);
    let perceptibility = field.map_or(1.0, |field| {
        field.config.perceptibility(field.paid_fraction, emission)
    });
    Some(phoenix_model::concealment_ui::ConcealmentControlsView {
        enabled: state.enabled(&config.0, owner),
        active: field.is_some(),
        paid_per_mille: field.map_or(0, |field| (field.paid_fraction * 1000.0).round() as u16),
        perceptibility_per_mille: (perceptibility * 1000.0).round() as u16,
        operating_emission: emission.to_string(),
        signature: signature.map_or(Default::default(), |signature| signature.1),
        controllable: world
            .get::<ShipSystemControlSources>(entity)
            .is_some_and(|sources| {
                sources.0.entries().any(|(actual, _)| actual == owner)
                    && sources.0.policy_for(owner).accept_human_input
            }),
    })
}
