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

## Bootstrap precedence

On startup, `WorldPlugin` selects the world bootstrap via `choose_bootstrap()`:

1. **Scenario-driven** — if a `ScenarioConfig` was preloaded (via JS → `wasm_load_scenario`), `spawn_scenario_entities` handles entity spawn and `init_scenario_runtime` seeds comms/trigger state.
2. **Map-config-driven** — if a `MapConfig` is loaded but no scenario file was preloaded, a warning is logged and `SimulationPlugin`'s config-based path handles the world.
3. **Hardcoded fallback** — if neither config is present (dev/test, no map loaded), `setup_world_hardcoded` spawns a procedural starfield and player ship.

The `default_scenario` field in the map config file (`assets/maps/default.toml`) specifies which scenario file the JS host should preload before calling `wasm_init()`.

## Relationship to #218

This plugin is the landing zone for the World/Scenario merger described in [#218](https://github.com/jkeywo/project-phoenix-v2/issues/218). Subsequent slices will migrate scenario types and systems into this module, ultimately making `WorldPlugin` the single owner of all world and scenario state.

See also: [World Data](../entities/world-data.md)
