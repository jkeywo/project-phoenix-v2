---
title: Repair Console — Server Plugin
---

# Repair Console — Server Plugin

Server-side logic for the Repair console lives in `src/console/repair/server.rs`. It is registered as part of `SimulationPlugin` via `crate::console::repair::server::RepairServerPlugin`.

## Overview

The current repair model is **direct team dispatch**: the Repair console operator selects a team (0–N) and a `RepairTarget` (a station or Core), and the server dispatches that team to repair all damaged systems owned by the target station. There is no shape-matching minigame — that was removed in an earlier refactor (PRD #272-era). The human UX is `gui/repair-console.html` sending `dispatch_repair_team` actions via `gui/action-map.js`, which encodes them as `ControlSystem { target: SystemId("repair"), payload: DispatchRepairTeam { .. } }` after #619 deleted the legacy `ClientMessage::DispatchRepairTeam { console }` wire path.

## Key types

| Type | Location | Purpose |
|---|---|---|
| `RepairTeams` | `src/modifiers/repair_teams.rs` | Pure-Rust state machine: N slots of Idle / Travelling / Repairing / Cooldown, keyed on `SystemId` |
| `ShipRepairTeams` | `src/console/repair/server.rs` | Bevy `Resource` + `Component` wrapping `RepairTeams`; seeded from TOML `[repair]` block |
| `RepairBlackboard` | `src/core/messages.rs` | Snapshot broadcast to the Repair console holder |
| `RepairTarget` | `src/core/messages.rs` | `Station(StationId)` or `Core` |

## Systems

| System | SimSet | Responsibility |
|---|---|---|
| `handle_dispatch_repair_team` | `SimSet::Input` | Processes `ControlSystem { target: repair, payload: DispatchRepairTeam { team_idx, target } }`; resolves `RepairTarget::Station(id)` to the matching `SystemId` on the ship's `EntitySystemHull` and `RepairTarget::Core` to `SystemId("core")`, then calls `teams.dispatch(team_idx, system_id, display_name)`. Post-#619 the legacy `ClientMessage::DispatchRepairTeam { console: Console }` wire path is gone — only the admission-gated `ControlSystem` envelope survives. |
| `tick_repair_teams` | `SimSet::Modifiers` | Advances team progress each frame; restores hull HP for completed teams on the per-entity `EntitySystemHull` |
| `operate_repair_ai` | `SimSet::Input` | Runs AI-controlled repair dispatch when the repair station is in `Backfill` or `Ai` mode; iterates all entities with `ShipSystemControlSources` gated on `policy.operate_ai`, picks the system with the largest HP deficit on that ship's hull |
| `publish_repair_blackboard` | `SimSet::Broadcast` | Writes a `RepairBlackboard` into `ShipSystemBlackboards` for the repair system key |
| `repair_state_broadcaster` | `PostUpdate` | Reads the blackboard and broadcasts `SystemBlackboard::Repair` to the station holder at 10 Hz |

## RepairBlackboard

```rust
pub struct RepairBlackboard {
    pub teams: Vec<TeamSlot>,
    pub system_hull: Vec<SystemHullStatus>,
    pub travel_duration_secs: f32,
    pub damageable_systems: Vec<SystemId>,
}
```

- `damageable_systems` derives from `SystemHull.entries()` (`src/console/repair/server.rs`). Core appears in this list when `[[hull.system_hull]] system_id = "core"` is declared in `player_ship.toml`.
- `system_hull` is the per-`SystemId` HP/tier snapshot (`SystemHullStatus { system_id, display_name, current, max, tier }`) sent to the client so the Repair screen can show live damage bars with human-readable labels.
- `TeamSlot::{Travelling,Repairing,Returning}` carry `system_id: SystemId` + `display_name: String` in-slot, so the client renders the target label directly without a Console-enum lookup.

## TOML configuration

Repair team parameters come from the `[repair]` block in `assets/entities/player_ship.toml`:

```toml
[repair]
repair_team_count = 2
travel_duration_secs = 5
repair_rate_hp_per_sec = 0.5
```

Core hull is declared as a `[[hull.system_hull]]` entry (post-#619 TOML shape):

```toml
[[hull.system_hull]]
system_id = "core"
display_name = "Core"
max_hp = 20
damaged_threshold_pct = 0.75
disabled_threshold_pct = 0.25
debuff_magnitude = 0.10
```

`display_name` is optional — when omitted the wire falls back to the raw `system_id` string. It exists so designer-facing labels (`"Engine (Port)"`, `"Phaser Bank (Fore)"`, etc.) survive the removal of the `Console::display_name()` lookup.

## Per-entity migration (PRD #597 PR 6)

`ShipRepairTeams` derives both `Resource` and `Component`. The player ship carries a per-entity `ShipRepairTeams` component seeded from its TOML `[repair]` block. NPC ships also get a `ShipRepairTeams` component when their entity TOML declares a `[repair]` block (skipped otherwise). `tick_repair_teams`, `handle_dispatch_repair_team`, `publish_repair_blackboard`, and `repair_state_broadcaster` stay LocalShip-scoped (repair is a player mechanic today), preferring the per-entity component on LocalShip with a Resource fallback for tests. Both the Component and the Resource are dual-written to keep legacy Resource-based readers in sync.

## Dispatch path

```
gui/repair-console.html
  → action-map.js  dispatch_repair_team({ team_idx, target })
  → client.html JS: ControlSystem { target: SystemId("repair"),
                                     payload: DispatchRepairTeam { team_idx, target } } envelope
  → wasm_receive_message / handle_dispatch_repair_team (src/console/repair/server.rs)
  → RepairTeams.dispatch(team_idx, system_id, display_name)
  → tick_repair_teams advances progress, restores hull HP on completion
```

## Tests

Tests live in `src/console/repair/server.rs` under `#[cfg(test)] mod tests`.

Notable tests:

| Test | What it checks |
|---|---|
| `dispatch_repair_target_station_maps_helm` | `RepairTarget::Station("helm")` dispatches to `SystemId("helm")` |
| `dispatch_repair_target_core_dispatches_to_core` | `RepairTarget::Core` dispatches team 0 to `SystemId("core")` |
| `publish_repair_blackboard_contains_damageable_systems` | `damageable_systems` contains both `SystemId("helm")` and `SystemId("core")` |
| `player_ship_toml_repair_block_matches_runtime_default_values` | Drift guard: TOML repair values match `RepairTimings::default()` |
| `repair_teams_resource_reflects_player_ship_toml_repair_block` | Drift guard: TOML→runtime wiring for repair timings |

Five additional tests were added by issue #526. See the source file for the full list.

## Sources

- `src/console/repair/server.rs`
- `src/modifiers/repair_teams.rs`
- `src/core/messages.rs` (RepairBlackboard, RepairTarget, SystemHullStatus)
- `assets/entities/player_ship.toml` ([repair] block, [[hull.system_hull]] entries)
- `gui/repair-console.html`, `gui/action-map.js`
- Issue [#508](https://github.com/jkeywo/project-phoenix-v2/issues/508)
- Issue [#526](https://github.com/jkeywo/project-phoenix-v2/issues/526)
- Issue [#619](https://github.com/jkeywo/project-phoenix-v2/issues/619) — Console enum + legacy `console_hull` / `damageable_consoles` fields deleted; `system_hull` / `damageable_systems` are the survivors
- [Console Plugin Pattern](./console-plugin-pattern.md)
- [Broadcaster Seam](./broadcaster-seam.md)
