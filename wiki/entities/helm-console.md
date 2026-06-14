---
title: Helm Console
type: entity
tags: [console, helm, input, ship, physics, radar, impulse]
sources: [gui/helm-console.html, gui/console-state.js, gui/radar-widget.js, src/ship_plugin.rs, src/ship/physics.rs, src/ship/impulse.rs, src/modifiers/coordination.rs, PRD-022]
updated: 2026-06-14
---

# Helm Console

The pilot's seat. The **only** console that can move the ship.

## Controls

- **Joystick:** up/down → thrust (0.0 to 1.0); left/right → steering (−1.0 to +1.0).
- **Steering snaps to centre on release** so the ship stops turning when the operator lets go.
- Sends `HelmInput { thrust, steering }` at **10 Hz** while controls are active.
- **Impulse button:** issues `StartImpulseCharge` when phase is `Idle`.
- **Impulse overlay (Charging + Active):** the joystick is hidden and replaced with a charging progress bar, a `CANCEL IMPULSE` button, and a status readout (`x.x / y.y s` while charging, `ENGAGED` while active). See [Impulse drive](#impulse-drive).

## Impulse drive

When the impulse drive enters `ImpulsePhase::Charging` (`ShipView.impulse_charge_progress > 0.0`):

- Client side: `refresh_helm_impulse_state` in `src/console/helm/client.rs` flips `HelmJoystickPad`/`HelmImpulseOverlay` visibility (compare-then-write), updates the progress fill width, writes the status text via the pure `format_impulse_status` helper, and pauses the joystick resend timer (`JoystickResendTimer.paused`). On the rising edge Idle→Charging it calls `reset_joystick_drag` so stale `last_dx`/`last_dy` cannot leak past the gate.
- Server side: `process_helm_inputs` in `src/ship_plugin.rs` detects the same Idle→Charging edge with a `Local<Option<ImpulsePhase>>` and zeroes `LastHelmInput`. While `ImpulseState::is_active()` it overrides input to `thrust = 1.0, steering = 0.0` and builds a per-tick `ShipPhysicsConfig` copy whose `acceleration` is multiplied by `ImpulseConfigResource.acceleration_multiplier` (falling back to the `IMPULSE_ACCELERATION_MULTIPLIER` const if the configured value is `≤ 0.0`).
- The speed cap during Active is driven by `ImpulseConfigResource.speed_multiplier` flowing through `translate_impulse_modifiers` → `ModifierSlot::MaxSpeed` (under `ModifierSource::ImpulseDrive`).

Cancel buttons live on Helm, Sensors (`ScienceCancelImpulseButton`) and Navigation (`NavCancelImpulseButton`); all three emit `ClientMessage::CancelImpulse`.

## Server reception

Each simulation tick:
1. Look up `helm_token()` from `SessionManager`.
2. Drain `HelmInput` messages tagged with that token; keep the latest.
3. Pass `(state, input, dt, config)` into `compute_physics()` from `src/server/ship_physics.rs` — a pure Rust function, no Bevy.
4. Apply the resulting velocity directly to the [Ship](./ship.md)'s Rapier rigid body.

If no one is at Helm, no `HelmInput` is read and the ship coasts/decelerates.

## Helm radar

The helm console renders an overhead radar showing nearby contacts from the HTML `RadarWidget`. `gui/console-state.js` projects contacts into the ship-relative frame for `gui/helm-console.html`.

Navigation can set one shared custom waypoint. The server owns it as `SimSnapshot.navigation_waypoint`; the JS client mirrors it into `state.navigationWaypoint`; `buildHelmConsoleState()` appends a `kind: "waypoint"` blip. If the waypoint is outside Helm radar range, the blip is clamped to the radar edge and marked with `edge: true` so Helm still sees the bearing.

`gui/helm-console.html` caches the last radar payload before calling `RadarWidget.update()`, so impulse-only state pushes (`impulse_charge_progress` changing every tick while charging) don't push redundant data into the widget. For that caching to actually stop the canvas redrawing, `RadarWidget` is **render-on-demand**: its `requestAnimationFrame` loop only repaints when a dirty flag is set (by `update()`, resize, pan/zoom gestures, or an async icon load) instead of unconditionally every frame. Together these keep the helm radar still during impulse charge — the previous unconditional 60 fps repaint was the source of the visible flicker.

## Tuning constants

From PRD #22, loaded from `[helm_console]` in `assets/entities/player_ship.toml`:

- Max forward speed: **50 units/s** (1 unit ≈ 1 m).
- Acceleration: **16.7 units/s²** (~3 s to max).
- Deceleration on zero thrust: **50 units/s²** (~1 s to stop).
- Max yaw rate at full steering: **π/2 rad/s** (90 °/s).
- Movement plane: XZ; Y-up. Forward = −Z when yaw = 0.
- `impulse_charge_duration` (default `3.0 s`) — total charge time; broadcast to clients on `Welcome` via `ShipClientConfig.impulse_charge_duration`.
- `impulse_speed_multiplier` (default `10.0`) — `MaxSpeed` factor applied during Active impulse.
- `impulse_acceleration_multiplier` (default `5.0`) — acceleration factor applied during Active impulse; `≤ 0.0` falls back to the const.

These live as constants in `compute_physics`'s `ShipPhysicsConfig` so they can be tuned without touching simulation/Bevy code.

## Related

- [Ship](./ship.md) · [Ship Physics](../concepts/ship-physics.md)
- [Radar Projection](../concepts/radar-projection.md)
- [PRD #22 — Helm and Game World](../sources/prd-022-helm-and-game-world.md)
