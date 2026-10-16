//! The installed Bay System's ordinary tree and normal admitted command path.
use super::*;
use crate::{
    core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload as Control},
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};

pub(crate) fn operate(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        &EntityUuid,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &ShipBayJobs,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (uuid, config, sources, boards, jobs, programs, mut runtimes, mut admitted) in &mut ships {
        for (owner, state) in &jobs.0 {
            if !crate::ai::host::ai_operates(&sources.0, owner) {
                continue;
            }
            let view = config.0.observations_for_system(owner, &boards.0);
            let Some(SystemBlackboard::Bay(board)) = view.get(owner) else {
                continue;
            };
            let Some(program) = programs.0.get(owner) else {
                continue;
            };
            let Some(runtime) = runtimes.0.get_mut(owner) else {
                continue;
            };
            let active = state.job.as_ref().is_some_and(|job| job.outcome.is_none());
            let facts = BTreeMap::from([
                ("can_operate".into(), board.can_launch || board.can_recover),
                ("has_stored".into(), !board.stored.is_empty()),
                ("has_contacts".into(), !board.contacts.is_empty()),
                ("active".into(), active),
                (
                    "has_accepted_reception".into(),
                    board.receptions.iter().any(|row| row.contact.is_some()),
                ),
            ]);
            let pending = BTreeMap::from([
                ("launch".into(), false),
                ("recover".into(), false),
                ("cancel".into(), false),
            ]);
            let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
                &sources.0,
                owner,
                Some(program),
                &facts,
                &pending,
                runtime,
                tick.0,
            ) else {
                continue;
            };
            let payload = if result.intents.get("cancel") == Some(&true) && active {
                Some(Control::CancelCraftHandling)
            } else if !active && result.intents.get("launch") == Some(&true) && board.can_launch {
                // Stable actual OWN stored roster; not a remote entity selector.
                board
                    .stored
                    .iter()
                    .min_by(|a, b| a.id.cmp(&b.id))
                    .map(|row| Control::LaunchCraft {
                        craft: row.id.clone(),
                    })
            } else if !active && result.intents.get("recover") == Some(&true) && board.can_recover {
                // Same exact public current contact choice offered to a Human.
                if let Some(row) = board.receptions.iter().find(|row| row.contact.is_some()) {
                    Some(Control::RecoverCraft {
                        contact: row.contact.clone().unwrap(),
                        reception: Some(row.order.clone()),
                    })
                } else {
                    board
                        .contacts
                        .iter()
                        .min_by(|a, b| a.contact.cmp(&b.contact))
                        .map(|row| Control::RecoverCraft {
                            contact: row.contact.clone(),
                            reception: None,
                        })
                }
            } else {
                None
            };
            if let Some(payload) = payload {
                crate::command_admission::ai_emit::emit_ai_command(
                    Some(uuid),
                    owner.clone(),
                    payload,
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            }
        }
    }
}
