# WorldPlugin

`WorldPlugin` is a Bevy plugin that owns world bootstrap logic for the simulation.

## Current scope

`WorldPlugin` is the single owner of world content lifecycle: parse, spawn, triggers, comms, and broadcast.

**Startup systems:**
- `setup_world_hardcoded` — starfield + player ship fallback when no `MapConfig` is present
- `spawn_scenario_entities` — resolves scenario spawn positions and calls the entity-spawn pipeline
- `init_scenario_runtime` — initialises `ScenarioRuntime`, `CommsInboxRes`, `ObjectiveManagerRes` from loaded `ScenarioConfig`

**Resources:** `ScenarioRuntime`, `CommsInboxRes`, `ObjectiveManagerRes`

**Update systems:** `handle_hail`, `handle_respond_to_message`, `handle_clear_comms`, `broadcast_comms_state`, `broadcast_objective_summary`, `handle_ai_events`

## Modules

| File | Contents |
|------|----------|
| `src/world/server.rs` | `WorldPlugin` and the `setup_world_hardcoded` startup system |
| `src/world/content.rs` | Pure (Bevy-free) scenario types: `ScenarioConfig`, triggers, comms templates, position resolution |

`WorldPlugin` is registered in `src/bridge.rs` alongside `SimulationPlugin` and `ScenarioPlugin`.

## Relationship to #218

This plugin is the landing zone for the World/Scenario merger described in [#218](https://github.com/jkeywo/project-phoenix-v2/issues/218). Subsequent slices will migrate scenario types and systems into this module, ultimately making `WorldPlugin` the single owner of all world and scenario state.

See also: [World Data](../entities/world-data.md)
