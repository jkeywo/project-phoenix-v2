---
title: Console
type: entity
tags: [console, role, lobby]
sources: [src/messages.rs, CONTEXT.md]
updated: 2026-05-09
---

# Console

A role a player occupies on the bridge. **Each console has exactly one seat.** Vacancy is immediate on disconnect. A player may hold more than one console simultaneously (`Player.consoles` is a `Vec<Console>`); the JS tab bar and `ActiveConsole` resource control which panel is displayed.

## Currently shipped

| Console | Page | Authority |
|---|---|---|
| `CaptainChair` | [Captain Console](./captain-console.md) | Start game, toggle Red Alert, change View Mode |
| `Helm` | [Helm Console](./helm-console.md) | Drive the ship; radar overlay; push radar to viewscreen |
| `Tactical` | — | Lock targets (`SetTarget`), fire phasers (`FirePhaser`) |
| `Engineering` | — | Repair hull breakdowns (`Repair`); wrong-console = penalty |

Defined in `src/messages.rs:39`:

```rust
pub enum Console {
    CaptainChair,
    Helm,
    Tactical,
    Engineering,
}
```

`display_name()` returns the human-readable label. `ALL_CONSOLES` in `client_lobby.rs` lists all four in display order.

## Planned

Future drafts add `Science` (Draft 3) and `Comms` (Draft 8 — stub). See [Bridge Crew Stations (planned)](./bridge-crew-stations-planned.md) and [Console Expansion roadmap](../roadmap/console-expansion.md).

## Console invariants

1. **One seat per console.** Server enforces in `SessionManager`.
2. **Captain authority is checked server-side.** `StartGame`, `ToggleRedAlert`, and `SetView` are no-ops unless the sender's token equals `captain_token()`.
3. **Helm is the only console that can move the ship.** `HelmInput` from any other token is silently dropped.
4. **A `Player.consoles` field is a `Vec`** — a player may hold multiple consoles. `ALL_CONSOLES` in `client_lobby.rs` defines the canonical display order.
5. **Tactical authorization is checked server-side.** `FirePhaser` requires a locked target and the target must satisfy `is_fire_ready()` (range + arc).
6. **Repair authorization is checked server-side.** `Repair` from a console that doesn't match `authorized_repair_console` (the front of `BreakdownQueue`) is treated as an unauthorized repair and incurs a penalty cooldown.

## How a new console is added

1. Add the variant to the `Console` enum and `display_name()` arm in `messages.rs`.
2. Add the consumer messages to `ClientMessage` / `ServerMessage` and round-trip tests in `codec.rs`.
3. Server-side handler in `simulation.rs` (or a new pure handler module beside `lobby_handler.rs`).
4. Add the console to `ALL_CONSOLES` in `client_lobby.rs`.
5. Add a panel setup function (e.g. `setup_<name>_ui`) and a visibility toggle system in `client_app.rs`; add button handlers that emit `OutboundClientMessage` events.
6. Add `ClientSimState` fields for any console-specific state the client needs (analogous to repair fields).

## Related

- [Player](./player.md) · [Session](./session.md)
- [Console Plugin Pattern](../concepts/console-plugin-pattern.md)
- [PRD #66](../sources/prd-066-weapons-and-engineering.md) — adds two consoles
