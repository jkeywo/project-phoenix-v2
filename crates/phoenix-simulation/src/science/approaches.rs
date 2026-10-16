//! Helm consumes its Console's permitted measured feeds after publication.
use crate::{
    core::messages::SystemBlackboard, server_app::ShipSystemBlackboards, ship::state::ShipPhysics,
    ship_plugin::ShipConfigComponent, simmath,
};
use bevy::prelude::*;

pub(crate) fn register(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        publish
            .in_set(crate::sim_sets::SimSet::Publish)
            .in_set(crate::sim_sets::FixedStep::PublishObservedApproaches),
    );
}

pub(crate) fn publish(
    mut ships: Query<(
        &ShipConfigComponent,
        &ShipPhysics,
        &crate::entities::spawner::HelmConsoleSection,
        &mut ShipSystemBlackboards,
    )>,
) {
    for (config, physics, helm, mut boards) in &mut ships {
        let horizon = helm
            .0
            .projection
            .as_ref()
            .and_then(|profile| profile.horizon());
        let sin = simmath::sin(physics.yaw);
        let cos = simmath::cos(physics.yaw);
        let velocity = [
            sin * physics.forward_speed + cos * physics.lateral_speed,
            -cos * physics.forward_speed + sin * physics.lateral_speed,
        ];
        let mut projected = Vec::new();
        for system in &config.0.systems {
            if system.kind != crate::ship::system_registry::HELM_RADAR_KIND {
                continue;
            }
            let approaches = horizon.map_or_else(Vec::new, |horizon| {
                let permitted = config.0.observations_for_system(&system.id, &boards.0);
                let records = config
                    .0
                    .systems
                    .iter()
                    .filter_map(|source| permitted.get(&source.id))
                    .flat_map(|board| match board {
                        SystemBlackboard::Helm(board) => {
                            board.tracks.as_deref().unwrap_or_default()
                        }
                        SystemBlackboard::TacticalRadar(board) => {
                            board.tracks.as_deref().unwrap_or_default()
                        }
                        SystemBlackboard::SensorRadar(board) => {
                            board.tracks.as_deref().unwrap_or_default()
                        }
                        _ => &[],
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                phoenix_sim_gameplay::science::approaches::derive(
                    &records,
                    [physics.x, physics.z],
                    velocity,
                    horizon,
                )
            });
            projected.push((system.id.clone(), approaches));
        }
        for (owner, approaches) in projected {
            if let Some(SystemBlackboard::Helm(board)) = boards.0.get_mut(&owner) {
                board.approaches = approaches;
            }
        }
    }
}

#[cfg(test)]
#[path = "approaches_tests.rs"]
mod tests;
