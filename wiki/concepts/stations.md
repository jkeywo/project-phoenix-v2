# Stations

## Overview

The station system maps player count → ordered station definitions, drives the lobby assignment cascade, and gates which consoles a player may occupy.

## Module split

> **Staleness note:** this page predates the B3/B4 (#533/#534) station-layout
> rewrite and the #601/#605 admission-gate changes below. `stations_policy.rs`
> — the sibling module this page originally documented alongside
> `stations_config` — was a tombstone alias with no live members by the time
> it was deleted outright; the `reassign_on_join`/`reassign_on_leave`/
> `advance_on_join` functions and the `complexity_presets` section further
> down describe that pre-B3/B4 shape and may no longer be accurate. This page
> needs a pass against current `src/lobby/stations_config.rs` and whatever
> replaced the deleted policy functions before it can be trusted.

The station config code lives in `src/lobby/stations_config.rs`, re-exported
at crate root via `src/lib.rs`:

```rust
pub use lobby::stations_config;
```

The old `src/stations.rs` shim has been deleted (issue #242). All call sites now import directly from `crate::stations_config`.

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

### Spectator queue removed (#605)

`SessionManager`'s write-only spectator queue was deleted — the FIFO that used to back empty-station assignment when all slots were taken no longer exists. At-capacity join behaviour is unchanged: players still receive `StationAssigned { station: None }`, but there is no backing queue to promote on departure.

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

## Config-derived admission gate (#601)

The `station_for_system()` lookup (`src/server_app.rs`) was replaced from a hardcoded system-id match to a `ShipConfig`-derived lookup via `config.system() -> station`. Unknown system IDs are now denied with a `warn!` log (previously conservative allow). Shield-arc prefix routing is preserved as a necessary exception (arc IDs derive from the variable-count shield-arc config, not from `ShipConfig.stations`). Admission-gate tests cover: config-defined system controllable only by its owning station's holder; unknown system ID is denied.

## Complexity presets

`ShipStations.complexity_presets` carries the per-console available preset names. Defaults (via `default_complexity_presets()`):
- `Low` + `Full` for: `CaptainChair`, `Helm`, `Tactical`, `Repair`, `Power`, `Comms`
- `Full` only for: `Sensors`, `Shields`, `Navigation`

## Asset

`assets/player_ship.toml` — the canonical ship/station TOML. The `[stations]` section is parsed by `parse_and_validate`. The ship config is currently embedded at build time via `include_str!`.
