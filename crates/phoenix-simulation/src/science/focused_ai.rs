//! Acquisition for additional fitted focused Systems. The bound designation
//! System has one combined controller in ship::sensors and is always skipped.
use super::{
    broad::{BroadTracks, SensingControls},
    server::ShipScanRecord,
};
use crate::{
    core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload, SystemId},
    entities::spawner::EntityUuid,
    server_app::ShipSystemBlackboards,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use std::collections::BTreeMap;

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        operate
            .in_set(crate::sim_sets::SimSet::Input)
            .in_set(crate::sim_sets::FixedStep::OperateFocusedSensorsAi)
            .run_if(crate::ai::cadence::ai_tick_ready),
    );
}

fn contact_for_request(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    tracks: &BroadTracks,
    owner: &SystemId,
    requested: &str,
) -> Option<String> {
    let permitted = config.observations_for_system(owner, &boards.0);
    for source in tracks.0.keys() {
        let publication = super::contact_routing::publication_source(config, source);
        let Some(records) = permitted
            .get(&publication)
            .and_then(super::contact_routing::observations)
        else {
            continue;
        };
        for record in records
            .iter()
            .filter(|record| record.selectable && !record.predicted)
        {
            if record.contact_id == requested
                || record.identity.as_ref().is_some_and(|identity| {
                    identity.name == requested
                        || identity
                            .references
                            .iter()
                            .any(|reference| reference == requested)
                })
            {
                if super::contact_routing::resolve(
                    config,
                    boards,
                    Some(tracks),
                    owner,
                    &record.contact_id,
                )
                .is_some()
                {
                    return Some(record.contact_id.clone());
                }
            }
        }
    }
    None
}

#[allow(clippy::type_complexity)]
pub(crate) fn operate(
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Option<&EntityUuid>,
        &ShipScanRecord,
        &SensingControls,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipSystemBlackboards,
        Option<&BroadTracks>,
        &crate::ai::state_tree_runtime::ShipAiTreePrograms,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
        &mut AdmittedCommands,
    )>,
) {
    use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
    for (
        uuid,
        records,
        controls,
        config,
        sources,
        boards,
        tracks,
        programs,
        mut runtimes,
        mut admitted,
    ) in &mut ships
    {
        for owner in records.0.keys() {
            if controls.0.science_target.as_ref() == Some(owner)
                || !crate::ai::host::ai_operates(&sources.0, owner)
            {
                continue;
            }
            let permitted = config.0.observations_for_system(owner, &boards.0);
            let Some(SystemBlackboard::Scan(scan)) =
                permitted.get(&super::scan_blackboard_key_for(owner))
            else {
                continue;
            };
            let requested = match permitted
                .get(&crate::ship::system_registry::viewscreen_system_id())
            {
                Some(SystemBlackboard::Viewscreen(board)) => {
                    crate::objectives::top_operate_directive(
                        &board.scored_objectives,
                        crate::core::messages::SystemAffinity::Sensors,
                        |directive| crate::objectives::scan_directive_target(directive).is_some(),
                    )
                    .and_then(crate::objectives::scan_directive_target)
                }
                _ => None,
            };
            let requested = requested.filter(|request| {
                runtimes.0.get(owner).is_some_and(|runtime| {
                    super::reading::request_ready(
                        runtime,
                        &config.0,
                        boards,
                        owner,
                        request,
                        tick.0,
                        records.0[owner].policy.reading_ticks,
                    )
                })
            });
            // Only an explicitly permitted currently measured return can route
            // a request. Missing observations never become a world search.
            let target = tracks.and_then(|tracks| {
                if let Some(requested) = requested {
                    return contact_for_request(&config.0, boards, tracks, owner, requested);
                }
                let designation = controls.0.science_target.as_ref()?;
                let SystemBlackboard::Sensors(designation) = permitted.get(designation)? else {
                    return None;
                };
                super::contact_routing::alias_for_source(
                    &config.0,
                    boards,
                    Some(tracks),
                    owner,
                    designation.science_target_uuid.as_deref()?,
                )
            });
            let acquiring = scan.focused.is_some();
            let available = scan.capable
                && target.as_ref().is_some_and(|target| {
                    scan.focused
                        .as_ref()
                        .is_none_or(|work| &work.subject != target)
                        && (requested.is_some()
                            || !scan
                                .history
                                .iter()
                                .any(|reading| &reading.subject_uuid == target))
                });
            let Some(runtime) = runtimes.0.get_mut(owner) else {
                continue;
            };
            let facts = BTreeMap::from([
                ("can_operate".into(), scan.capable),
                ("request_available".into(), available),
                ("acquiring".into(), acquiring),
                ("has_target".into(), target.is_some()),
            ]);
            let channels = ["designate", "enquire", "cancel"]
                .into_iter()
                .map(|channel| (channel.to_owned(), false))
                .collect();
            let Ok(TreeHostOutcome::Evaluated(result)) = decide_tree(
                &sources.0,
                owner,
                programs.0.get(owner),
                &facts,
                &channels,
                runtime,
                tick.0,
            ) else {
                continue;
            };
            let payload = if acquiring && result.intents.get("cancel") == Some(&true) {
                Some(SystemControlPayload::CancelFocusedScan)
            } else if result.intents.get("enquire") == Some(&true) {
                target.map(|uuid| SystemControlPayload::ScanTarget { uuid })
            } else {
                None
            };
            if let Some(payload) = payload {
                crate::command_admission::ai_emit::emit_ai_command(
                    uuid,
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

#[cfg(test)]
#[path = "focused_ai_tests.rs"]
mod tests;
