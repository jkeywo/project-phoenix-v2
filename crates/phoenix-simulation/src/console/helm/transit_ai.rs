//! The actual drive's one ordinary System controller; it cannot author positions.
use super::*;

pub(super) fn operate(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Option<&EntityUuid>,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipSystemBlackboards,
        Option<&ShipPhysics>,
        Option<&crate::console::navigation::NavigationWaypoint>,
        &ShipTransitDrives,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (
        uuid,
        config,
        sources,
        boards,
        physics,
        waypoint,
        drives,
        programs,
        mut runtimes,
        mut admitted,
    ) in &mut ships
    {
        for (owner, state) in &drives.0 {
            if !crate::ai::host::ai_operates(&sources.0, owner) {
                continue;
            }
            let Some(program) = programs.0.get(owner) else {
                continue;
            };
            let Some(runtime) = runtimes.0.get_mut(owner) else {
                continue;
            };
            let view = config.0.observations_for_system(owner, &boards.0);
            let mut requests = match view.get(&crate::ship::system_registry::viewscreen_system_id())
            {
                Some(SystemBlackboard::Viewscreen(board)) => board
                    .scored_objectives
                    .iter()
                    .filter(|row| {
                        row.score > 0.0
                            && !row.snapshot.unassigned
                            && row
                                .relevance
                                .contains(&crate::core::messages::SystemAffinity::Helm)
                    })
                    .filter_map(|row| match row.directive {
                        AiDirective::Transit { revision } => Some((
                            row.score,
                            TransitRequestKey {
                                objective_id: row.id.clone(),
                                revision,
                            },
                        )),
                        _ => None,
                    })
                    .filter(|(_, key)| state.request_is_fresh(key))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            requests.sort_by(|a, b| {
                b.0.total_cmp(&a.0)
                    .then_with(|| a.1.objective_id.cmp(&b.1.objective_id))
            });
            let request = requests.first().map(|(_, key)| key).filter(|_| {
                crate::science::reading::mission_text_ready(
                    runtime, &config.0, boards, owner, tick.0,
                )
            });
            let course_available = physics.zip(waypoint).is_some_and(|(physics, waypoint)| {
                course(&config.0, boards, owner, waypoint, physics.y).is_some()
            });
            let facts = BTreeMap::from([
                (
                    "can_operate".into(),
                    online(sources, owner)
                        && physics.is_some()
                        && !drives
                            .0
                            .iter()
                            .any(|(other, state)| other != owner && state.phase.active()),
                ),
                ("request_present".into(), request.is_some()),
                ("course_available".into(), course_available),
                (
                    "preparing".into(),
                    matches!(state.phase, TransitPhase::Preparing { .. }),
                ),
                ("active".into(), state.phase.active()),
            ]);
            let pending = BTreeMap::from([("begin".into(), false), ("cancel".into(), false)]);
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
            if result.intents.get("cancel") == Some(&true)
                && matches!(state.phase, TransitPhase::Preparing { .. })
            {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
                    owner.clone(),
                    Control::CancelTransit,
                    sources,
                    &sessions,
                    Some(config),
                    &mut admitted,
                );
            } else if result.intents.get("begin") == Some(&true)
                && !state.phase.active()
                && course_available
            {
                if let Some(request) = request {
                    crate::command_admission::ai_emit::emit_ai_command(
                        uuid,
                        owner.clone(),
                        Control::BeginTransitToWaypoint {
                            request: Some(request.clone()),
                        },
                        sources,
                        &sessions,
                        Some(config),
                        &mut admitted,
                    );
                }
            }
        }
    }
}
