---
title: Console
type: entity
tags: [console, role, lobby, station]
sources: [src/core/messages.rs, src/lobby/stations_config.rs, gui/console-registry.js]
updated: 2026-07-03
---

# Console

The GUI panel owned by a [Station](./station.md). Each of the 9 bridge stations
has exactly one console. Access derives from `Player.station: Option<StationId>`
— the station a player claimed in the lobby.

**The old `Console` enum was deleted in issue #619** (part of PRD #516). The
canonical identity for lobby/session/authority is now the lowercase-kebab
`StationId` string (`"captain"`, `"helm"`, `"tactical"`, `"repair"`,
`"sensors"`, `"shields"`, `"navigation"`, `"power"`, `"comms"`). Client tab
switching and per-panel routing key on the same lowercase strings via
`gui/console-registry.js`.

Fine-grained authority routing keys on `SystemId` (see [System](./system.md)).
A single station may own multiple systems (e.g. `tactical` owns `phaser-fore`,
`phaser-aft`, `torpedo-tube-*`, `torpedo-magazine` — see the
[coarse-system migration](../concepts/coarse-system-migration.md) fine-system
table).

## Current consoles (9)

| StationId | Console panel | Role |
|---|---|---|
| `"captain"` | Captain | Start game, toggle Red Alert, change View Mode |
| `"helm"` | Helm | Drive the ship; radar overlay; impulse charge |
| `"tactical"` | Tactical | Lock targets, fire phasers/torpedoes, set phaser mode |
| `"repair"` | Repair | Direct team dispatch (per SystemId); three teams |
| `"sensors"` | Sensors | Advisory target hand-off; long-range radar |
| `"shields"` | Shields | Manage shield facings (per-arc SystemIds) |
| `"navigation"` | Navigation | Plot waypoints; view navigation chart |
| `"power"` | Power | Distribute power across groups |
| `"comms"` | Comms | Hail stations; relay intelligence |

Defined by the `[[station]]` blocks in `assets/entities/player_ship.toml` and
the `gui/console-registry.js` mapping (`stationId → sectionId + iframeId`).
There is no runtime enum for these — the strings are the authority.

## Authority model

Handler systems gate input on:

1. **Station ownership** — the sender's `Player.station` must equal the
   required `StationId` (checked via `Sessions::holder_for_station`).
2. **SystemId routing** — all console actions arrive as
   `ClientMessage::ControlSystem { target: SystemId, payload }`. The handler
   checks `ShipSystemControlSources::accept_human_input(target)` before
   processing.

`StationId` is the "who owns the console" address; `SystemId` is the "what
capability is being addressed" address. Both are just lowercase-kebab strings.

## Console invariants

1. **One player per station.** Server enforces in `SessionManager`.
2. **Captain authority is checked server-side.** `ToggleRedAlert` and
   `SetViewMode` are no-ops unless the sender holds `"captain"`.
3. **Helm is the only station that can move the ship.** `HelmInput` from any
   other token is silently dropped (also gated by `SystemId::helm`).
4. **Tactical authorization is checked server-side.** `FirePhaser` requires a
   locked target plus `is_fire_ready()` (range + arc) on the addressed phaser
   bank SystemId; `FireTorpedo` requires a loaded tube SystemId and a target.
5. **Repair authorization is checked server-side.** Only the `"repair"`
   station holder may issue `DispatchRepairTeam`; the target is a
   `RepairTarget::Station(StationId)` or `RepairTarget::Core`.
6. **Power authorization is checked server-side.**
   `SetPowerGroupAllocation` is only accepted from `"power"`; bounded by
   capacity and emergency threshold.

## How a new console is added

1. Add a `[[station]]` block to `assets/entities/player_ship.toml` with a new
   `id`, `name`, `description`, `rank`, `short_code`, and at least one
   `[[station.rating]]`.
2. Add a `[[system]]` block for each system the station owns.
3. Add the station id to `ALL_STATIONS` in `gui/lobby-state.js`.
4. Register the panel in `gui/console-registry.js` (lowercase station id →
   section + iframe).
5. Add the client HTML panel (`gui/<name>-console.html`) and wire its actions
   through `gui/action-map.js` as `ControlSystem` envelopes.
6. Implement the server-side handler plugin under `src/console/<name>/server.rs`.

## Related

- [Station](./station.md) — the lobby seat that owns a Console
- [System](./system.md) — fine-grained capability addressed by SystemId
- [Player](./player.md) · [Session](./session.md)
- [Console UI Authoring Library](../concepts/console-ui-library.md)
- PRD #487 — station/system redesign
- Issue #518 — B1–B6 config migration
- Issue [#619](https://github.com/jkeywo/project-phoenix-v2/issues/619) — Console enum deletion (final slice of PRD #516)
