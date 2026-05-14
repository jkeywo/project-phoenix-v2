// Bevy plugin: reads the loaded ScenarioConfig and dispatches spawn actions
// through the existing entity-spawn pipeline.
//
// This plugin is server-only and runs during the startup phase. It calls the
// config_cache accessors directly (same pattern as simulation::setup_world)
// and for each resolved spawn invokes `spawn_entity`, mirroring the map
// entity pipeline.

use bevy::prelude::*;

use crate::entity_spawner::spawn_entity;

/// Bevy plugin that spawns entities declared in the active scenario at startup.
pub struct ScenarioPlugin;

impl Plugin for ScenarioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_scenario_entities);
    }
}

/// Startup system: resolves all scenario spawn positions and spawns each entity
/// through the shared `spawn_entity` helper.
fn spawn_scenario_entities(mut commands: Commands) {
    let scenario_config = match crate::config_cache::get_scenario_config() {
        Some(s) => s,
        None => return, // No scenario loaded — nothing to do.
    };

    let map_config = crate::config_cache::get_map_config();
    let anchors = map_config
        .as_ref()
        .map(|mc| mc.anchors.clone())
        .unwrap_or_default();

    let config_cache = crate::config_cache::get_config_cache();

    let resolved = match crate::scenario::resolve_positions(&scenario_config, &anchors) {
        Ok(r) => r,
        Err(e) => {
            bevy::log::error!("ScenarioPlugin: failed to resolve spawn positions: {e}");
            return;
        }
    };

    for spawn in &resolved {
        let config = config_cache.get(&spawn.entity_path);

        let Some(config) = config else {
            bevy::log::warn!(
                "ScenarioPlugin: no config found for entity path '{}' (spawn '{}') — skipping",
                spawn.entity_path,
                spawn.name
            );
            continue;
        };

        let position = Vec3::new(
            spawn.position[0],
            spawn.position[1],
            spawn.position[2],
        );

        spawn_entity(
            &mut commands,
            config,
            position,
            spawn.uuid.clone(),
            Some(spawn.name.clone()),
        );

        bevy::log::info!(
            "ScenarioPlugin: spawned '{}' at {:?} uuid={}",
            spawn.name,
            spawn.position,
            spawn.uuid
        );
    }
}
