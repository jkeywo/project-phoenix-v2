---
title: Console
type: entity
tags: [console, role, lobby]
sources: [src/shared/messages.rs, CONTEXT.md]
updated: 2026-05-08
---

# Console

A role a player occupies on the bridge. **Each console has exactly one seat.** Vacancy is immediate on disconnect.

## Currently shipped

| Console | Page | Authority |
|---|---|---|
| `CaptainChair` | [Captain Console](./captain-console.md) | Start game, toggle Red Alert, change View Mode |
| `Helm` | [Helm Console](./helm-console.md) | Drive the ship via thrust + steering |

Defined in `src/shared/messages.rs:39`:

```rust
pub enum Console {
    CaptainChair,
    Helm,
}
```

`display_name()` returns the human-readable label (`"Captain's Chair"`, `"Helm"`).

## Planned

PRD #66 adds `Weapons` and `Engineering`. Future drafts add `Science` (Draft 3) and `Comms` (Draft 8 — stub). See [Bridge Crew Stations (planned)](./bridge-crew-stations-planned.md) and [Console Expansion roadmap](../roadmap/console-expansion.md).

## Console invariants

1. **One seat per console.** Server enforces in `SessionManager`.
2. **Captain authority is checked server-side.** `StartGame`, `ToggleRedAlert`, and `SetView` are no-ops unless the sender's token equals `captain_token()`.
3. **Helm is the only console that can move the ship.** `HelmInput` from any other token is silently dropped.
4. **A `Player.consoles` field is a `Vec`** — the data model anticipates a player holding multiple consoles, even though current PRDs use one-per-player. See [Console Plugin Pattern](../concepts/console-plugin-pattern.md).

## How a new console is added

Per PRD #66 + the architectural deepening in commits `f3ef92c` and `3ad236d`:

1. Add the variant to the `Console` enum and `display_name()` arm.
2. Add the consumer messages to `ClientMessage` / `ServerMessage`.
3. Round-trip tests in `src/shared/codec.rs`.
4. Server-side handler in `simulation.rs` (or a new pure handler module beside `lobby_handler.rs`).
5. Client-side **Console Plugin** (`src/client/<name>_plugin.rs`) — owns the UI, marker components, and event handlers.
6. Update `available_consoles()` in `SessionManager`.

## Related

- [Player](./player.md) · [Session](./session.md)
- [Console Plugin Pattern](../concepts/console-plugin-pattern.md)
- [PRD #66](../sources/prd-066-weapons-and-engineering.md) — adds two consoles
