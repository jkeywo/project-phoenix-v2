---
type: AFK
priority: infrastructure
---

# 002 — Message Type Definitions

Define the canonical wire types shared across all modules.

## What to build

`src/messages.rs` — pure data module with:

- `Console` enum: `CaptainChair` (expand later)
- `GamePhase` enum: `Lobby`, `InProgress`
- `Player` struct: `token: String, name: String, console: Option<Console>, connected: bool`
- `GameState` struct: `phase: GamePhase, players: Vec<Player>`
- `SimSnapshot` struct: `red_alert: bool`
- `ClientMessage` enum: `Identify { token, name }`, `SetName { name }`, `SelectConsole { console }`, `ClearConsole`, `StartGame`, `ToggleRedAlert`
- `ServerMessage` enum: `Welcome { state: GameState }`, `PlayerJoined { player }`, `PlayerLeft { token }`, `ConsoleSelected { token, console }`, `ConsoleCleared { token }`, `NameChanged { token, name }`, `GameStarted`, `SimState { snapshot: SimSnapshot }`

All types: `Clone, Debug, serde::Serialize, serde::Deserialize`.

## Acceptance Criteria

- [ ] Module compiles with `cargo check`
- [ ] No logic — pure data structs/enums only
