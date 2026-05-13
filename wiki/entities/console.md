---
title: Console
type: entity
tags: [console, role, lobby, station]
sources: [src/messages.rs, src/stations.rs, CONTEXT.md]
updated: 2026-05-13
---

# Console

A role on the bridge. Players no longer pick consoles directly — they pick **stations**, and a station bundles one or more consoles (see PRD #120 / [`stations.rs`](../sources/prd-120-station-based-lobby.md)). At small player counts a single station may bundle several consoles; at full crew each station typically bundles one.

`Player.consoles` is a `Vec<Console>` derived from the player's currently-assigned station. The JS tab bar + `ActiveConsole` resource control which panel is rendered when a player holds more than one console.

## Currently shipped

| Console | Page | Authority / Role |
|---|---|---|
| `CaptainChair` | [Captain Console](./captain-console.md) | Start game, toggle Red Alert, change View Mode |
| `Helm` | [Helm Console](./helm-console.md) | Drive the ship; radar overlay; push radar to viewscreen; trigger impulse charge |
| `Tactical` | — | Lock targets (`SetTarget`), fire phasers (`FirePhaser`), set phaser mode (`Auto`/`Manual`), fire torpedoes (`FireTorpedo`) |
| `Repair` | — | Shape-matching repair (`Repair { shape }`); three teams; wrong shape or wrong console = penalty cooldown |
| `Power` | — | Distribute 6 base + up to 2 battery points across `Helm` / `Tactical` / `Science` via `IncreasePower` / `DecreasePower` |
| `Science` | — | Advisory target hand-off (`SetScienceTarget`), cancel impulse (`CancelImpulse`), push `ScienceRadar` / `SystemChart` view modes |

Defined in `src/messages.rs:142`:

```rust
pub enum Console {
    CaptainChair,
    Helm,
    Tactical,
    Repair,
    Science,
    Power,
}
```

`display_name()` returns the human-readable label. `ALL_CONSOLES` in `client_lobby.rs` lists all six in display order.

## Planned

`Console::Comms` is drafted in PRD #119 (Space Stations, Scenario Engine & Comms Console). It is the only console variant not yet on the wire.

## Console invariants

1. **One seat per console** within a station; one station per player. Server enforces in `SessionManager` + `stations.rs`.
2. **Captain authority is checked server-side.** `StartGame`, `ToggleRedAlert`, and `SetView` are no-ops unless the sender's station bundles `CaptainChair`.
3. **Helm is the only console that can move the ship.** `HelmInput` from any other token is silently dropped.
4. **A `Player.consoles` field is a `Vec`** — derived from the assigned station. `ALL_CONSOLES` in `client_lobby.rs` defines the canonical display order.
5. **Tactical authorization is checked server-side.** `FirePhaser` requires a locked target plus `is_fire_ready()` (range + arc); `FireTorpedo` requires a loaded tube and a target.
6. **Repair authorization is checked server-side.** `Repair { shape }` must match the current head of `BreakdownQueue` and come from `Console::Repair`. Wrong shape, wrong console, or no free team incurs a penalty cooldown via `repair_teams.rs`.
7. **Power authorization is checked server-side.** `IncreasePower` / `DecreasePower` only accepted from `Console::Power`; bounded by 6 base + up to 2 battery; battery exhaustion locks all consoles to level 1 until recharged past `emergency_threshold`.
8. **Console complexity is per-console, per-player.** `SetComplexity { console, preset_name }` switches between `Low` and `Full` presets (PRD #154); hidden controls at `Low` are operated server-side by `console_ai`.

## How a new console is added

1. Add the variant to the `Console` enum and `display_name()` arm in `messages.rs`.
2. Add the consumer messages to `ClientMessage` / `ServerMessage` and round-trip tests in `codec.rs`.
3. Server-side handler in `simulation.rs` (or a new pure handler module beside `lobby_handler.rs`).
4. Add the console to `ALL_CONSOLES` in `client_lobby.rs`.
5. Add a panel setup function and a visibility toggle system in `client_app.rs`; add button handlers that emit `OutboundClientMessage` events.
6. Add `ClientSimState` fields for any console-specific state the client needs.
7. Add the console to one or more station bundles in `assets/entities/player_ship.toml` `[stations]` blocks; add a complexity preset under `assets/complexity/<console>.toml` (PRD #154).

## Related

- [Player](./player.md) · [Session](./session.md)
- [Console Plugin Pattern](../concepts/console-plugin-pattern.md)
- [PRD #66](../sources/prd-066-weapons-and-engineering.md) — adds Tactical + (then-named) Engineering
- [PRD #118](../sources/prd-118-repair-and-power-consoles.md) — splits Engineering into Repair + Power
- [PRD #120](../sources/prd-120-station-based-lobby.md) — replaces console-picking with station-picking
- [PRD #154](../sources/prd-154-console-complexity.md) — per-console Low/Full complexity presets + AI
