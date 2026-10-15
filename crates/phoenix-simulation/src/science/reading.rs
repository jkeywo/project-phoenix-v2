//! Tick-based private reading progress inside the existing System controller.
//! No task planner, computer-capacity owner or command producer.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use bevy::prelude::*;
use phoenix_sim_gameplay::ai::text_reading::TextReading;

pub fn request_key(request: &str) -> String {
    format!("request/{}:{request}", request.len())
}
pub fn request_ready(
    runtime: &phoenix_sim_gameplay::ai::state_tree::TreeRuntime,
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    request: &str,
    tick: u64,
    reading_ticks: u32,
) -> bool {
    let Some(key) = semantic_key(config, boards, owner, request) else {
        return false;
    };
    let reading_ticks = required_ticks(config, boards, owner, request, reading_ticks);
    reading_ticks == 0
        || runtime.reading.as_ref().is_some_and(|reading| {
            reading.required_ticks == reading_ticks && reading.ready(&key, tick)
        })
}
fn semantic_key(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    request: &str,
) -> Option<String> {
    let permitted = config.observations_for_system(owner, &boards.0);
    let SystemBlackboard::Viewscreen(board) =
        permitted.get(&crate::ship::system_registry::viewscreen_system_id())?
    else {
        return None;
    };
    let chosen = crate::objectives::top_operate_directive(
        &board.scored_objectives,
        crate::core::messages::SystemAffinity::Sensors,
        |directive| crate::objectives::scan_directive_target(directive).is_some(),
    )?;
    if crate::objectives::scan_directive_target(chosen) != Some(request) {
        return None;
    }
    let row = board
        .scored_objectives
        .iter()
        .find(|row| std::ptr::eq(&row.directive, chosen))?;
    // Scores/progress/receipt time are not text revisions. Semantic text IDs,
    // authored substitution parameters and typed request content are.
    let held = held_request_reference(config, boards, owner, request);
    serde_json::to_string(&(
        &row.id,
        &row.snapshot.text,
        &row.snapshot.text_params,
        chosen,
        held,
    ))
    .ok()
    .map(|revision| format!("{}:{revision}", request_key(request)))
}

fn held_request_reference(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    request: &str,
) -> Option<phoenix_model::sensing_references::AttachedSensorReference> {
    let view = config.observations_for_system(owner, &boards.0);
    let SystemBlackboard::Dossiers(dossiers) =
        view.get(&crate::dossier::dossier_blackboard_key())?
    else {
        return None;
    };
    super::references::held_references(dossiers).remove(request)
}
fn required_ticks(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    request: &str,
    base: u32,
) -> u32 {
    held_request_reference(config, boards, owner, request).map_or(base, |reference| {
        base.max(reference.reference.reading_ticks)
    })
}

fn mission_key(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
) -> Option<String> {
    let permitted = config.observations_for_system(owner, &boards.0);
    let SystemBlackboard::Viewscreen(board) =
        permitted.get(&crate::ship::system_registry::viewscreen_system_id())?
    else {
        return None;
    };
    let mut rows: Vec<_> = board
        .scored_objectives
        .iter()
        .filter(|row| row.score > 0.0 && !row.snapshot.unassigned)
        .map(|row| {
            (
                &row.id,
                &row.snapshot.text,
                &row.snapshot.text_params,
                &row.directive,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    if rows.is_empty() {
        return None;
    }
    serde_json::to_string(&rows)
        .ok()
        .map(|key| format!("mission/{key}"))
}
pub fn mission_text_ready(
    runtime: &phoenix_sim_gameplay::ai::state_tree::TreeRuntime,
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    tick: u64,
) -> bool {
    let Some(key) = mission_key(config, boards, owner) else {
        return true;
    };
    runtime
        .reading
        .as_ref()
        .is_some_and(|reading| reading.ready(&key, tick))
}

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        advance
            .in_set(crate::sim_sets::SimSet::Input)
            .in_set(crate::sim_sets::FixedStep::ReadSystemText),
    );
}

#[allow(clippy::type_complexity)]
fn advance(
    tick: Res<crate::sim_tick::SimTick>,
    mut ships: Query<(
        &ShipConfigComponent,
        &ShipSystemControlSources,
        &ShipSystemBlackboards,
        Option<&super::server::ShipScanRecord>,
        Option<&super::broad::BroadProfile>,
        &mut crate::ai::state_tree_runtime::ShipAiTreeRuntime,
    )>,
) {
    for (config, sources, boards, scans, broad, mut runtimes) in &mut ships {
        for (owner, runtime) in &mut runtimes.0 {
            let required = scans
                .and_then(|scans| scans.0.get(owner))
                .map(|scan| scan.policy.reading_ticks)
                .or_else(|| {
                    broad
                        .and_then(|broad| broad.0.get(owner))
                        .map(|profile| profile.profile.reading_ticks)
                })
                .or_else(|| {
                    config
                        .0
                        .system(owner)
                        .and_then(|system| {
                            phoenix_sim_gameplay::ship::composition::illumination_composition(
                                &system.components,
                            )
                        })
                        .map(|binding| binding.config.reading_ticks)
                })
                .or_else(|| {
                    config
                        .0
                        .system(owner)
                        .and_then(|system| {
                            phoenix_sim_gameplay::ship::composition::jamming_composition(
                                &system.components,
                            )
                        })
                        .map(|binding| binding.config.reading_ticks)
                });
            let Some(required) = required else {
                continue;
            };
            let tactical = config.0.system(owner).is_some_and(|system| {
                matches!(
                    system.kind.as_str(),
                    crate::ship::system_registry::TACTICAL_RADAR_KIND
                        | crate::ship::system_registry::ILLUMINATION_KIND
                        | crate::ship::system_registry::JAMMING_KIND
                )
            });
            let selected = if tactical {
                mission_key(&config.0, boards, owner).map(|key| (key, required))
            } else {
                requested_text(&config.0, boards, owner).and_then(|request| {
                    semantic_key(&config.0, boards, owner, request).map(|key| {
                        (
                            key,
                            required_ticks(&config.0, boards, owner, request, required),
                        )
                    })
                })
            };
            let eligible = crate::ai::host::ai_operates(&sources.0, owner) && selected.is_some();
            if let Some((key, required)) = selected {
                if let Err(error) =
                    TextReading::observe(&mut runtime.reading, &key, required, tick.0, eligible)
                {
                    warn!("AI reading refused invalid continuation: {error}");
                }
            } else if let Some(reading) = &mut runtime.reading {
                // Preserve work on information loss but do not count the gap.
                if reading.last_tick.is_none_or(|last| tick.0 >= last) {
                    reading.last_tick = Some(tick.0);
                }
            }
        }
    }
}

pub fn requested_text<'a>(
    config: &'a crate::ship::config::ShipConfig,
    boards: &'a ShipSystemBlackboards,
    owner: &SystemId,
) -> Option<&'a str> {
    let permitted = config.observations_for_system(owner, &boards.0);
    let SystemBlackboard::Viewscreen(board) =
        permitted.get(&crate::ship::system_registry::viewscreen_system_id())?
    else {
        return None;
    };
    crate::objectives::top_operate_directive(
        &board.scored_objectives,
        crate::core::messages::SystemAffinity::Sensors,
        |directive| crate::objectives::scan_directive_target(directive).is_some(),
    )
    .and_then(crate::objectives::scan_directive_target)
}

#[cfg(test)]
#[path = "reading_tests.rs"]
mod tests;
