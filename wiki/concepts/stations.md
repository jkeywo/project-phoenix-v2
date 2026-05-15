# Stations

## Overview

The station system maps player count → ordered station definitions, drives the lobby assignment cascade, and gates which consoles a player may occupy.

## Module split

The station code lives in two modules under `src/lobby/`:

| Module | Path | Responsibility |
|--------|------|---------------|
| `stations_config` | `src/lobby/stations_config.rs` | TOML deserialization, `ShipStations`, `StationDef`, `parse_and_validate`, `get_station`, `all_stations_filled` |
| `stations_policy` | `src/lobby/stations_policy.rs` | Pure assignment policy: `reassign_on_join`, `reassign_on_leave`, `advance_on_join`, spectator FIFO interactions |

Both are re-exported at crate root via `src/lib.rs`:

```rust
pub use lobby::stations_config;
pub use lobby::stations_policy;
```

And a thin shim at `src/stations.rs` re-exports the public surface for callers that import `crate::stations::*`.

## Config (`stations_config`)

### Key types

- **`StationDef`** — a single station within a player-count bucket: `name`, `description`, `consoles: Vec<Console>`, `rank`, `short_code`, `next: Option<String>`, `previous: Option<String>`.
- **`ShipStations`** — the validated configuration resource. `configs: HashMap<u32, Vec<StationDef>>`, `min_players`, `max_players`, `complexity_presets`.
- **`StationAssignments`** — `HashMap<String, String>` mapping session token → station name. Absent = spectator.
- **`StationConfigError`** — enum of all validation errors.

### Public functions

- `parse_and_validate(toml_str) -> Result<ShipStations, StationConfigError>` — parses the `[stations]` TOML section; validates console names, empty-consoles, duplicate names, explicit `next`/`previous` references, and implicit `MissingNext`.
- `get_station(stations, player_count, name) -> Option<&StationDef>` — lookup by count and name.
- `all_stations_filled(stations, player_count, current: &[Console]) -> bool` — true when every station at `player_count` has at least one console represented in `current`.

### TOML schema

```toml
[stations]
min_players = 1
max_players = 4

[[stations.1]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]
next = "Bridge"        # optional — implicit if same name exists at count+1
```

Loaded from `assets/player_ship.toml` at runtime.

## Policy (`stations_policy`)

Pure functions with no Bevy dependencies, imported via `crate::stations_policy::`:

- `reassign_on_join(stations: &ShipStations, current: &StationAssignments, new_player: &str) -> StationAssignments` — cascades the N+1 station layout when a player joins; the new player is placed at the station with no predecessor.
- `reassign_on_leave(stations: &ShipStations, current: &StationAssignments, leaving_player: &str, spectators: &VecDeque<String>) -> (StationAssignments, VecDeque<String>)` — cascades the N-1 station layout when a player leaves; promotes a spectator if a slot stays empty.
- `advance_on_join(stations: &ShipStations, current: &StationAssignments) -> StationAssignments` — lobby-safe variant: advances existing assigned players to the N+1 layout without assigning the new joiner (they must select a station explicitly).

Call sites import directly from `crate::stations_policy` rather than via the legacy `crate::stations` shim.

## Complexity presets

`ShipStations.complexity_presets` carries the per-console available preset names. Defaults (via `default_complexity_presets()`):
- `Low` + `Full` for: `CaptainChair`, `Helm`, `Tactical`, `Repair`, `Power`, `Comms`
- `Full` only for: `Sensors`, `Shields`, `Navigation`

## Asset

`assets/player_ship.toml` — the canonical ship/station TOML. The `[stations]` section is parsed by `parse_and_validate`. The ship config is currently embedded at build time via `include_str!`.
