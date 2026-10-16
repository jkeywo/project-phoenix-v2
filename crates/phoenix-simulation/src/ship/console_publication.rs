//! Recipient projections over existing publications. No simulation facts live here.
use crate::core::messages::{ServerMessage, StationId, SystemBlackboard, SystemId};
use crate::ship::components::{ConsoleHosts, ShipConfigComponent};
use bevy::prelude::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Resource, Default)]
pub struct LastConsolePublications(
    pub BTreeMap<String, (Option<u64>, BTreeMap<SystemId, SystemBlackboard>)>,
);

pub(crate) type ConsolePublicationQuery = QueryState<
    (
        &'static ShipConfigComponent,
        &'static crate::server_app::ShipSystemBlackboards,
        Option<&'static ConsoleHosts>,
    ),
    With<crate::server_app::LocalShip>,
>;

fn publication_is_permitted(
    config: &crate::ship::config::ShipConfig,
    hosts: &ConsoleHosts,
    station: &StationId,
    id: &SystemId,
    board: &SystemBlackboard,
) -> bool {
    hosts.0.iter().any(|host| {
        host.host.as_ref() == Some(station)
            && config.console(&host.console).is_some_and(|console| {
                config
                    .system(id)
                    .is_some_and(|system| system.console.as_ref() == Some(&console.id))
                    || console.feeds.iter().any(|feed| feed.accepts(id, board))
            })
    })
}

pub fn permitted_publications(
    config: &crate::ship::config::ShipConfig,
    hosts: &ConsoleHosts,
    station: Option<&StationId>,
    boards: &HashMap<SystemId, SystemBlackboard>,
) -> BTreeMap<SystemId, SystemBlackboard> {
    let Some(station) = station else {
        return BTreeMap::new();
    };
    boards
        .iter()
        .filter(|(id, board)| publication_is_permitted(config, hosts, station, id, board))
        .map(|(id, board)| (id.clone(), board.clone()))
        .collect()
}

/// Project a granted publication in its producing Console's knowledge context.
pub fn project_permitted_board(
    config: &crate::ship::config::ShipConfig,
    source: &SystemId,
    board: &SystemBlackboard,
    visibility: Option<&crate::console::repair::visibility::HullVisibility>,
) -> SystemBlackboard {
    let SystemBlackboard::Repair(raw) = board else {
        return board.clone();
    };
    let publisher = config.console_for_system(source).or_else(|| {
        config
            .systems
            .iter()
            .find(|system| system.kind == crate::ship::system_registry::REPAIR_KIND)
            .and_then(|system| config.console_for_system(&system.id))
    });
    match (visibility, publisher) {
        (Some(visibility), Some(console)) => SystemBlackboard::Repair(
            visibility.project_repair_for_console(config, &console.id, raw),
        ),
        _ => crate::console::repair::visibility::project_blackboard_for_token(None, None, board),
    }
}

/// Publish the current permitted Console observations to each recipient.
pub fn publish_console_blackboards(world: &mut World, generation: Option<u64>) {
    let mut query = world.query_filtered();
    publish_console_blackboards_with_query(world, generation, &mut query);
}

/// The scheduled publisher retains its QueryState; direct callers keep the
/// same entry point and packet semantics without introducing an ECS owner.
pub(crate) fn publish_console_blackboards_with_query(
    world: &mut World,
    generation: Option<u64>,
    query: &mut ConsolePublicationQuery,
) {
    let generation = Some(generation.unwrap_or(0));
    let tick = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    let Some((_, boards, _)) = query.iter(world).next() else {
        return;
    };
    // The reserved local route carries the raw publications and never uses
    // Repair visibility. Build that projection only when a phone may need it.
    let needs_visibility = world
        .resource::<crate::lobby::Sessions>()
        .0
        .players()
        .iter()
        .any(|player| player.connected)
        && boards
            .0
            .values()
            .any(|board| matches!(board, SystemBlackboard::Repair(_)));
    let vis = needs_visibility
        .then(|| crate::console::repair::visibility::hull_visibility(world))
        .flatten();
    world.init_resource::<LastConsolePublications>();
    let Some((config, boards, hosts)) = query.iter(world).next() else {
        return;
    };
    let empty_hosts = ConsoleHosts::default();
    let hosts = hosts.unwrap_or(&empty_hosts);
    let sessions = world.resource::<crate::lobby::Sessions>();
    let previous = world.resource::<LastConsolePublications>();
    // None retains the existing cache entry. In particular, unchanged local
    // boards can be compared by reference without cloning every publication.
    let mut next: BTreeMap<_, _> = sessions
        .0
        .players()
        .iter()
        .filter(|p| p.connected)
        .map(|player| {
            let projection: BTreeMap<_, _> = boards
                .0
                .iter()
                .filter(|(id, board)| {
                    player.station.as_ref().is_some_and(|station| {
                        publication_is_permitted(&config.0, hosts, station, id, board)
                    })
                })
                .map(|(source, board)| {
                    (
                        source.clone(),
                        project_permitted_board(&config.0, source, board, vis.as_ref()),
                    )
                })
                .collect();
            let unchanged = previous
                .0
                .get(&player.token)
                .is_some_and(|(stamp, old)| *stamp == generation && *old == projection);
            (
                player.token.clone(),
                (!unchanged).then_some((generation, projection)),
            )
        })
        .collect();
    let local = crate::console_bridge::LOCAL_CONSOLE_TOKEN;
    let local_unchanged = previous.0.get(local).is_some_and(|(stamp, old)| {
        *stamp == generation
            && old.len() == boards.0.len()
            && old
                .iter()
                .all(|(id, board)| boards.0.get(id) == Some(board))
    });
    next.insert(
        local.to_string(),
        (!local_unchanged).then(|| {
            (
                generation,
                boards
                    .0
                    .iter()
                    .map(|(id, board)| (id.clone(), board.clone()))
                    .collect(),
            )
        }),
    );
    let mut pending = Vec::new();
    for (token, projection) in &next {
        let Some((_, boards)) = projection else {
            continue;
        };
        // Scoped packets are complete projections: the unordered snapshot lane
        // may deliver a newer tick first. Rejecting an older packet must not
        // discard a board that the newer packet omitted as an unchanged delta.
        let updates = boards
            .iter()
            .map(|(id, board)| (id.clone(), board.clone()))
            .collect();
        pending.push((
            crate::lobby::Target::Token(token.clone()),
            ServerMessage::BlackboardUpdate {
                console_scope: Some(boards.keys().cloned().collect()),
                console_scope_tick: Some(tick),
                updates,
                presentation_generation: generation,
            },
        ));
    }
    let mut previous = world.resource_mut::<LastConsolePublications>();
    previous.0.retain(|token, _| next.contains_key(token));
    for (token, projection) in next {
        if let Some(projection) = projection {
            previous.0.insert(token, projection);
        }
    }
    drop(previous);
    world
        .resource_mut::<crate::server_app::SimOutbox>()
        .extend_snapshot(pending);
}

#[cfg(test)]
#[path = "console_publication_scope_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "console_publication_tests.rs"]
mod publication_regression_tests;

#[cfg(test)]
#[path = "console_observation_publication_tests.rs"]
mod observation_tests;
