---
type: AFK
priority: feature-tracer
---

# 005 — Lobby System (Bevy)

Bevy systems managing the pre-game phase.

## What to build

`src/lobby.rs`:

- Bevy `Plugin` struct `LobbyPlugin`
- Handle inbound `ClientMessage` events: `Identify`, `SetName`, `SelectConsole`, `ClearConsole`, `StartGame`
- Validate authorship: only the captain (first player) can call `StartGame`
- Emit outbound `ServerMessage` events in response
- Transition `GamePhase` from `Lobby` → `InProgress` on `StartGame`

## Tests

Spin up a minimal Bevy `App` with only `LobbyPlugin`, inject `ClientMessage` events, assert on resulting `ServerMessage` events and `GamePhase` changes.

## Acceptance Criteria

- [ ] `cargo test lobby` passes
- [ ] Phase transition tested
