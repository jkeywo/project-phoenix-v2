---
type: AFK
priority: feature-tracer
---

# 006 — Game Simulation (Bevy)

Minimal in-game simulation: Red Alert toggle.

## What to build

`src/simulation.rs`:

- `SimulationPlugin` Bevy plugin
- `RedAlertState` resource: `active: bool`
- Handle `ClientMessage::ToggleRedAlert` — captain only
- Broadcast `ServerMessage::SimState { snapshot }` at 10 Hz to all clients

## Acceptance Criteria

- [ ] `cargo check` passes
- [ ] Red Alert toggled only by captain (test with Bevy App harness)
