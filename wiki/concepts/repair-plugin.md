---
title: Repair Console — Server Plugin
---

# Repair Console — Server Plugin

Server-side logic for the Repair console lives in `src/console/repair/server.rs`. It is registered as part of `SimulationPlugin` via `crate::console::repair::server::RepairServerPlugin`.

## Overview

The current repair model is **direct team dispatch**: the Repair console operator selects an Idle team and a `RepairTarget` (a station or Core), and the server dispatches that team to the station. There is no shape-matching minigame — that was removed in an earlier refactor (PRD #272-era). The human UX is `gui/repair-console.html` (and the shared `ph-repair-teams` component) sending `dispatch_repair_team` actions via `gui/action-map.js`, which encodes them as `ControlSystem { target: SystemId("repair"), payload: DispatchRepairTeam { .. } }` after #619 deleted the legacy `ClientMessage::DispatchRepairTeam { console }` wire path.

Since issue #1013, an on-site team no longer fixes one system and walks home: it **sweeps** every non-Operational system at its station (or the ownerless `core` group) worst-first — tier, then damage fraction, then id — rewriting its `Repairing` slot in place, and only goes `Returning` once nothing repairable remains. Destroyed (0 HP) systems are swept too, not skipped. Since issue #1015, the Repair console no longer offers the old per-team 1/2/3 ordinal buttons for a busy team; instead it shows a worst-first list of the ship's damaged/destroyed systems (`ph-repair-teams`'s `.damaged-list`), and tapping a row sends `SystemControlPayload::SetRepairTargetPriority { system_id }`. The host resolves which on-site team's sweep the named system belongs to and pins that SYSTEM (never a client-computed ordinal — see `handle_set_repair_target_priority` in `src/console/repair/dispatch.rs`) as its next job; the pin wins over the #1013 standing ordinal while it remains a candidate, and clears once the team moves off it. The old `SetRepairPriority { team_idx, priority }` ordinal payload stays on the wire and handled, but nothing in the UI sends it anymore.

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
| `handle_dispatch_repair_team`, `handle_set_repair_priority`, `handle_set_repair_target_priority` | `SimSet::Physics`, `.after(operate_repair_ai)` | Apply admitted `DispatchRepairTeam` / `SetRepairPriority` / `SetRepairTargetPriority` (issue #1015) commands, in that fixed order relative to the AI decide/emit and the tick below (issue #785 AC4 determinism). `handle_dispatch_repair_team` resolves `RepairTarget::Station(id)` to the matching `SystemId` on the ship's `EntitySystemHull` (`RepairTarget::Core` to `SystemId("core")`) and calls `teams.dispatch(team_idx, system_id, display_name)`. Post-#619 the legacy `ClientMessage::DispatchRepairTeam { console: Console }` wire path is gone — only the admission-gated `ControlSystem` envelope survives. |
| `tick_repair_teams` | `SimSet::Physics`, `.after(...)` the appliers above | Advances team progress each frame; sweeps the on-site station's remaining damaged systems worst-first (issue #1013) and restores hull HP on the per-entity `EntitySystemHull` |
| `operate_repair_ai` | `SimSet::Physics`, gated on the shared AI cadence | Runs AI-controlled repair dispatch when the repair station is `Backfill` or `Ai`; ranks damaged stations with the authored `[repair.selector]` `TargetSelector` (issue #785) rather than a hardcoded largest-deficit comparator, then emits `DispatchRepairTeam` for each free team through the admission seam |
| `publish_repair_blackboard` | `SimSet::Publish` | Writes a `RepairBlackboard` into `ShipSystemBlackboards` for the repair system key |
| `repair_state_broadcaster` | `PostUpdate` | Reads the blackboard and broadcasts `SystemBlackboard::Repair` to the station holder at 10 Hz |

## RepairBlackboard

```rust
pub struct RepairBlackboard {
    pub teams: Vec<TeamSlot>,
    pub system_hull: Vec<SystemHullStatus>,
    pub travel_duration_secs: f32,
    pub damageable_systems: Vec<SystemId>,
    pub queue_depth: Vec<QueueEntryPreview>,
    pub aggregate_hull_fraction: Option<f32>,
    pub destroyed_hull_fraction: Option<f32>,
}
```

`queue_depth` (issue #682) is the pending-request severity preview; `aggregate_hull_fraction` and `destroyed_hull_fraction` (issue #1014) are ship-wide scalars everyone may have even where the per-system rows are withheld. Issue #737's `src/console/repair/visibility.rs` projects `system_hull` and `queue_depth` per-recipient before this struct reaches the wire — see [Damage And Repair Intent](./damage-and-repair-intent.md).

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

Issue #830 dropped `ShipRepairTeams`'s legacy global `Resource` derive: it is now `#[derive(Component, Clone)]` only, and every ship — player and NPC alike — reads and writes its own component, with no ship-wide singleton to fall back to. The player ship carries a per-entity `ShipRepairTeams` component seeded from its TOML `[repair]` block. NPC ships also get one when their entity TOML declares a `[repair]` block (skipped otherwise).

`tick_repair_teams` (`src/console/repair/server.rs:219`) iterates every ship (`With<Ship>`), reading each ship's own `ShipModifiers` component directly (`&ShipModifiers`, not `Option<&ShipModifiers>` — the #606 cleanup removed the `Option`/Resource-fallback branch here too) to scale `RepairRate`. `handle_dispatch_repair_team` and `publish_repair_blackboard` are also per-entity and iterate every `Ship`, not just `LocalShip` — NPC teams tick and publish to their own `ShipSystemBlackboards` so their AI can read them. Only `repair_state_broadcaster` (the outbound wire) stays `LocalShip`-scoped, since NPC team state never reaches a client.

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

Tests live in `src/console/repair/server.rs` and `src/console/repair/dispatch.rs` under `#[cfg(test)] mod tests`, plus the sweep/priority state machine in `src/modifiers/repair_teams.rs`.

Notable tests:

| Test | What it checks |
|---|---|
| `dispatch_sends_team_to_travelling` (`server.rs`) | A dispatch to a station puts the team in `Travelling` |
| `station_dispatch_repairs_damaged_owned_fine_system` (`server.rs`) | A completed dispatch resolves to and repairs the correct owned fine system |
| `publish_repair_blackboard_contains_damageable_systems` (`server.rs`) | `damageable_systems` contains both `SystemId("helm")` and `SystemId("core")` |
| `station_name_colliding_with_a_hull_row_resolves_to_no_dispatch` (`dispatch.rs`) | A station whose own name is also an ownerless hull row must not fall back to sweeping that row |
| `a_damaged_owned_system_still_wins_over_the_fallback` (`dispatch.rs`) | The most-damaged owned system is picked over the station-name fallback |

See the source files for the full list, including the issue #1013 sweep-ordering and issue #1015 target-priority tests in `src/modifiers/repair_teams.rs`.

## Sources

- `src/console/repair/server.rs`
- `src/console/repair/dispatch.rs`
- `src/console/repair/visibility.rs` (issue #737 per-recipient projection)
- `src/modifiers/repair_teams.rs`
- `src/core/messages.rs` (RepairBlackboard, RepairTarget, SystemHullStatus)
- `assets/entities/player_ship.toml` ([repair] block, [[hull.system_hull]] entries)
- `gui/repair-console.html`, `gui/components/ph-repair-teams.js`, `gui/repair-dispatch.js`, `gui/action-map.js`
- Issue [#508](https://github.com/jkeywo/project-phoenix-v2/issues/508)
- Issue [#526](https://github.com/jkeywo/project-phoenix-v2/issues/526)
- Issue [#619](https://github.com/jkeywo/project-phoenix-v2/issues/619) — Console enum + legacy `console_hull` / `damageable_consoles` fields deleted; `system_hull` / `damageable_systems` are the survivors
- Issue [#1013](https://github.com/jkeywo/project-phoenix-v2/issues/1013) — on-site teams sweep every damaged system at a station worst-first
- Issue [#1015](https://github.com/jkeywo/project-phoenix-v2/issues/1015) — repair console damaged-systems list, tap-to-prioritise `SetRepairTargetPriority`
- [Console UI Authoring Library](./console-ui-library.md)
- [Broadcaster Seam](./broadcaster-seam.md)
