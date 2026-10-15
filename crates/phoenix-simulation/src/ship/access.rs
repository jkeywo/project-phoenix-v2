//! Admission, scoped ordinary AI and physical access processing.
use crate::core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload as Control};
use bevy::prelude::*;
pub use phoenix_sim_gameplay::ship::access::InteriorAccess;
use std::collections::{BTreeMap, BTreeSet};

/// Immutable accepted geometry, independent of repair-team availability.
#[derive(Component, Clone)]
pub struct InteriorLayout(pub phoenix_model::interior::InteriorConfig);

pub(crate) fn register(app: &mut App) {
    use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    app.register_admitted_consumer(
        ConsumerMatcher::kind(crate::ship::system_registry::ACCESS_GATE_KIND).with_feedback(
            crate::command_admission::FeedbackAddress::DeclaredKindOrCanonical("access-gate"),
            &[Payload::SetAccessSeal, Payload::SetAccessActuator],
        ),
    );
    app.add_systems(
        FixedUpdate,
        (ai_access, apply_access, tick_access)
            .chain()
            .in_set(crate::sim_sets::SimSet::Physics)
            .after(crate::sim_sets::FixedStep::AiShieldFocus)
            .before(crate::sim_sets::FixedStep::HandlePowerMessages)
            .before(crate::console::repair::server::tick_repair_teams),
    );
}

fn apply_access(
    mut owners: Query<(&AdmittedCommands, &mut InteriorAccess)>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    for (admitted, mut access) in &mut owners {
        let ids: Vec<_> = access.controls.keys().cloned().collect();
        for id in ids {
            for command in admitted.for_target(&id.0) {
                let applied = match &command.payload {
                    Control::SetAccessSeal { sealed } => access.set(&id, *sealed, true),
                    Control::SetAccessActuator { sealed } => access.set(&id, *sealed, false),
                    _ => continue,
                };
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
}
fn tick_access(
    time: Res<Time>,
    mut owners: Query<(
        &mut InteriorAccess,
        Option<&crate::console::repair::server::ShipRepairTeams>,
        Option<&crate::resources::logistics_server::CompiledShipLogistics>,
        Option<&crate::resources::logistics_server::ShipLogistics>,
        Option<&crate::security::ShipSecurityTeams>,
        &crate::ship_plugin::ShipSystemControlSources,
    )>,
) {
    for (mut access, teams, logistics_profile, logistics, security, sources) in &mut owners {
        let mut crossing = BTreeSet::new();
        if let Some(teams) = teams {
            if let (Some(layout), Some(navigation)) = (teams.0.layout(), teams.0.navigation()) {
                for team in navigation {
                    if let phoenix_model::interior::TeamLocation::Crossing { connection, .. } =
                        &team.location
                    {
                        if let Some(id) = layout
                            .connections
                            .iter()
                            .find(|link| &link.id == connection)
                            .and_then(|link| link.boundary.as_ref())
                        {
                            crossing.insert(id.clone());
                        }
                    }
                }
            }
        }
        if let (Some(profile), Some(logistics)) = (logistics_profile, logistics) {
            for batch in logistics.0.batches.values() {
                if let phoenix_model::interior::RouteLocation::Crossing { connection, .. } =
                    &batch.navigation.location
                {
                    if let Some(id) = profile
                        .0
                        .graph()
                        .config
                        .connections
                        .iter()
                        .find(|link| &link.id == connection)
                        .and_then(|link| link.boundary.as_ref())
                    {
                        crossing.insert(id.clone());
                    }
                }
            }
        }
        if let Some((profile, navigation)) = security.and_then(|security| {
            security
                .local
                .as_ref()
                .zip(security.teams.navigation.as_ref())
        }) {
            for cursor in navigation {
                if let phoenix_model::interior::RouteLocation::Crossing { connection, .. } =
                    &cursor.location
                {
                    if let Some(id) = profile
                        .graph
                        .config
                        .connections
                        .iter()
                        .find(|link| &link.id == connection)
                        .and_then(|link| link.boundary.as_ref())
                    {
                        crossing.insert(id.clone());
                    }
                }
            }
        }
        let unavailable = access
            .controls
            .keys()
            .filter(|id| {
                let policy = sources.0.policy_for(id);
                !policy.accept_human_input && !policy.operate_ai
            })
            .cloned()
            .collect();
        access.tick_available(time.delta_secs(), &crossing, &unavailable);
    }
}
fn ai_access(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut owners: Query<(
        Option<&crate::entities::spawner::EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, sources, config, boards, programs, mut runtimes, mut admitted) in &mut owners {
        for system in config
            .0
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::ACCESS_GATE_KIND)
        {
            if !crate::ai::host::ai_operates(&sources.0, &system.id) {
                continue;
            }
            let permitted = config.0.observations_for_system(&system.id, &boards.0);
            let Some(SystemBlackboard::Repair(board)) =
                permitted.get(&crate::core::messages::SystemId("repair".into()))
            else {
                continue;
            };
            let Some(status) = board
                .boundaries
                .iter()
                .find(|status| status.system == system.id)
            else {
                continue;
            };
            let [phoenix_sim_gameplay::ship::composition::SystemComponentConfig::AccessBoundary {
                sealed_channel,
                ..
            }] = system.components.as_slice()
            else {
                continue;
            };
            let facts = BTreeMap::from([
                ("seal_requested".into(), status.seal_requested),
                (
                    "sealed".into(),
                    status.phase == phoenix_model::interior::BoundaryPhase::Sealed,
                ),
            ]);
            let channels = BTreeMap::from([(sealed_channel.clone(), status.actuator_sealed)]);
            let Some(runtime) = runtimes.0.get_mut(&system.id) else {
                continue;
            };
            let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
                &sources.0,
                &system.id,
                programs.0.get(&system.id),
                &facts,
                &channels,
                runtime,
                tick.0,
            ) else {
                continue;
            };
            if let Some(sealed) = result
                .intents
                .get(sealed_channel)
                .filter(|value| **value != status.actuator_sealed)
            {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    system.id.clone(),
                    Control::SetAccessActuator { sealed: *sealed },
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "access_tests.rs"]
mod tests;
