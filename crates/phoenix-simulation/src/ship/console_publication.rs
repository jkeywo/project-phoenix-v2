//! Recipient projections over existing publications. No simulation facts live here.
use crate::core::messages::{ServerMessage, StationId, SystemBlackboard, SystemId};
use crate::ship::components::{ConsoleHosts, ShipConfigComponent};
use bevy::prelude::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Resource, Default)]
pub struct LastConsolePublications(
    pub BTreeMap<String, (Option<u64>, BTreeMap<SystemId, SystemBlackboard>)>,
);

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
        .filter(|(id, board)| {
            hosts.0.iter().any(|host| {
                host.host.as_ref() == Some(station)
                    && config.console(&host.console).is_some_and(|console| {
                        config
                            .system(id)
                            .is_some_and(|system| system.console.as_ref() == Some(&console.id))
                            || console.feeds.iter().any(|feed| feed.accepts(id, board))
                    })
            })
        })
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
    let generation = Some(generation.unwrap_or(0));
    let tick = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    let vis = crate::console::repair::visibility::hull_visibility(world);
    let mut query = world.query_filtered::<(
        &ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        Option<&ConsoleHosts>,
    ), With<crate::server_app::LocalShip>>();
    let Some((config, boards, hosts)) = query.iter(world).next() else {
        return;
    };
    let empty_hosts = ConsoleHosts::default();
    let hosts = hosts.unwrap_or(&empty_hosts);
    let sessions = world.resource::<crate::lobby::Sessions>();
    let mut next: BTreeMap<_, _> = sessions
        .0
        .players()
        .iter()
        .filter(|p| p.connected)
        .map(|player| {
            let mut projection =
                permitted_publications(&config.0, hosts, player.station.as_ref(), &boards.0);
            for (source, board) in &mut projection {
                *board = project_permitted_board(&config.0, source, board, vis.as_ref());
            }
            (player.token.clone(), (generation, projection))
        })
        .collect();
    next.insert(
        crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
        (
            generation,
            boards
                .0
                .iter()
                .map(|(id, board)| (id.clone(), board.clone()))
                .collect(),
        ),
    );
    world.init_resource::<LastConsolePublications>();
    let previous = world.resource::<LastConsolePublications>();
    let mut pending = Vec::new();
    for (token, (stamp, boards)) in &next {
        let old = previous.0.get(token);
        if old.is_some_and(|(old_stamp, old_boards)| old_stamp == stamp && old_boards == boards) {
            continue;
        }
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
    world.resource_mut::<LastConsolePublications>().0 = next;
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
