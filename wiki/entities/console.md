---
title: Console
type: entity
tags: [console, role, lobby, station]
sources: [src/messages.rs, src/lobby/stations_config.rs]
updated: 2026-06-23
---

# Console

The GUI panel owned by a [Station](./station.md). Each of the 9 bridge stations
maps 1-to-1 to a `Console` variant. The `Console` enum is the identifier used
for station assignment, tab switching, and authority checks in the existing
handlers; `SystemId` is the emerging replacement for authority routing (see
[System](./system.md)).

Players pick a **station** in the lobby; the station's `consoles` list is what
ends up in `Player.consoles`. The JS tab bar and `ActiveConsole` resource
control which panel is rendered.

## Current consoles (9)

| Console | Station | Role |
|---|---|---|
| `CaptainChair` | Captain | Start game, toggle Red Alert, change View Mode |
| `Helm` | Helm | Drive the ship; radar overlay; impulse charge |
| `Tactical` | Tactical | Lock targets, fire phasers/torpedoes, set phaser mode |
| `Repair` | Repair | Shape-matching repair; three teams |
| `Sensors` | Sensors | Advisory target hand-off; long-range radar |
| `Shields` | Shields | Manage shield facings |
| `Navigation` | Navigation | Plot waypoints; view navigation chart |
| `Power` | Power | Distribute power across groups |
| `Comms` | Comms | Hail stations; relay intelligence |

Defined in `src/messages.rs::Console`. `Console::from_console_id()` parses the
lowercase string used in `player_ship.toml`.

## Authority model

Handler systems gate input on:

1. **Console ownership** — the sender's `Player.consoles` must include the
   required `Console` variant.
2. **SystemId routing** — all console actions now arrive as
   `ClientMessage::ControlSystem { target: SystemId, payload }`. The handler
   checks `ShipSystemControlSources::accept_human_input(target)` before
   processing.

`Console` is the legacy layer; `SystemId` is the emerging canonical address.
Until decomposition is complete both layers coexist.

## Console invariants

1. **One seat per console.** Server enforces in `SessionManager`.
2. **Captain authority is checked server-side.** `ToggleRedAlert` and
   `SetViewMode` are no-ops unless the sender holds `CaptainChair`.
3. **Helm is the only console that can move the ship.** `HelmInput` from any
   other token is silently dropped (and is now also gated by `SystemId::helm`).
4. **Tactical authorization is checked server-side.** `FirePhaser` requires a
   locked target plus `is_fire_ready()` (range + arc); `FireTorpedo` requires a
   loaded tube and a target.
5. **Repair authorization is checked server-side.** Shape must match the current
   head of `BreakdownQueue`; wrong shape, wrong console, or no free team incurs
   a penalty cooldown.
6. **Power authorization is checked server-side.** `IncreasePower` /
   `DecreasePower` only accepted from `Console::Power`; bounded by capacity and
   emergency threshold.

## How a new console is added

1. Add the variant to `Console` in `messages.rs` and implement
   `Console::from_console_id()` for it.
2. Add a `[[station]]` block to `assets/entities/player_ship.toml` with
   `console = "<new_id>"` and at least one `[[station.rating]]`.
3. Add a `[[system]]` block for each system the console owns.
4. Add the console to `ALL_CONSOLES` in `gui/lobby-state.js`.
5. Implement the server-side handler plugin.
6. Add the client-side panel and wire up `ControlSystem` messages.

## Related

- [Station](./station.md) — the lobby seat that owns a Console
- [System](./system.md) — fine-grained capability addressed by SystemId
- [Player](./player.md) · [Session](./session.md)
- [Console Plugin Pattern](../concepts/console-plugin-pattern.md)
- [PRD #487](../sources/prd-487-station-console-system-redesign.md) — station/system redesign
- [Issue #518](../sources/issue-540-config-migration-docs.md) — B1–B6 config migration
