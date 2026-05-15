# WorldPlugin

`WorldPlugin` is a Bevy plugin that owns world bootstrap logic for the simulation.

## Current scope

- Registers `setup_world_hardcoded` as a `Startup` system.
- `setup_world_hardcoded` spawns the procedural starfield and the player ship when no `MapConfig` is preloaded (development/testing fallback path).
- When a `MapConfig` is present, the hardcoded setup skips itself and `SimulationPlugin`'s config-based path runs instead.

## Location

`src/world/server.rs` — registered in `src/bridge.rs` alongside `SimulationPlugin` and `ScenarioPlugin`.

## Relationship to #218

This plugin is the landing zone for the World/Scenario merger described in [#218](https://github.com/jkeywo/project-phoenix-v2/issues/218). Subsequent slices will migrate scenario types and systems into this module, ultimately making `WorldPlugin` the single owner of all world and scenario state.

See also: [World Data](../entities/world-data.md)
