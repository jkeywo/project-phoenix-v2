---
title: Helm Console
type: entity
tags: [console, helm, input, ship, physics, radar, impulse, boost]
sources: [gui/battleship/helm.html, gui/cruiser/helm.html, gui/destroyer/helm.html, gui/console-state.js, gui/components/ph-helm-radar.js, src/weapons/arc_geometry.rs, src/console/helm/server.rs, src/ship/helm_admission.rs, src/ship/physics_systems.rs, src/ship/physics.rs, src/ship/impulse.rs, src/ship/boost.rs, src/modifiers/coordination.rs]
updated: 2026-08-02
---

# Helm Console

The pilot's seat. The **only** console that can move the ship.

## Controls

- **Joystick:** up/down → thrust (0.0 to 1.0); left/right → steering (−1.0 to +1.0).
- **Steering snaps to centre on release** so the ship stops turning when the operator lets go.
- Sends `SetThrust { value }` -> `helm-thrust` and `SetSteering { value }` -> `helm-steering` at **10 Hz** while controls are active (per-axis wire split, issue #801).
- **Impulse button:** issues `StartImpulseCharge` when phase is `Idle`.
- **Impulse overlay (Charging + Active):** the joystick is hidden and replaced with a charging progress bar, a `CANCEL IMPULSE` button, and a status readout (`x.x / y.y s` while charging, `ENGAGED` while active). See [Impulse drive](#impulse-drive).
- **Boost button:** toggles the boost drive when `[helm_console.boost]` is present. While active, boost multiplies speed/acceleration by `multiplier` and yaw rate by `steering_multiplier`.
- **Arc-bearing popup:** when Tactical's weapons target is in weapons range but outside all phaser bank firing arcs, `tick_weapons_arc_request` emits a `CoordinationPayload::ArcBearingRequest` through the channel-3 coordination bus. Human Helm receives a "Tactical: come about — bring phasers to bear" popup; AI Helm consumes it silently and biases steering toward the target via `PendingArcBearingRequest` + `steer_toward`.

## Impulse drive

When the impulse drive enters `ImpulsePhase::Charging` (`ShipView.impulse_charge_progress > 0.0`):

- Client side: `refresh_helm_impulse_state` in `src/console/helm/client.rs` flips `HelmJoystickPad`/`HelmImpulseOverlay` visibility (compare-then-write), updates the progress fill width, writes the status text via the pure `format_impulse_status` helper, and pauses the joystick resend timer (`JoystickResendTimer.paused`). On the rising edge Idle→Charging it calls `reset_joystick_drag` so stale `last_dx`/`last_dy` cannot leak past the gate.
- Server side: `process_helm_inputs` in `src/ship/helm_admission.rs` zeroes the stale helm input at the `StartImpulseCharge` command application site (same-tick, replacing the pre-#824 `Local<Option<ImpulsePhase>>` phase-edge latch). While `ImpulseState::is_active()` it overrides input to `thrust = 1.0, steering = 0.0` and builds a per-tick `ShipPhysicsConfig` copy whose `acceleration` is multiplied by `ImpulseConfigResource.acceleration_multiplier` (falling back to the `IMPULSE_ACCELERATION_MULTIPLIER` const if the configured value is `≤ 0.0`).
- The speed cap during Active is driven by `ImpulseConfigResource.speed_multiplier` flowing through `translate_impulse_modifiers` → `ModifierSlot::MaxSpeed` (under `ModifierSource::ImpulseDrive`).

Cancel buttons live on Helm, Sensors (`ScienceCancelImpulseButton`) and Navigation (`NavCancelImpulseButton`); all three emit `ClientMessage::CancelImpulse`.

## Low-speed turn boost

`[helm_console] low_speed_turn_boost = X` makes a hull turn harder the slower it flies. `effective_yaw_rate` in `src/ship/physics.rs` scales `max_yaw_rate` by `1 + X * (1 - speed_fraction)` — x`1+X` at a dead stop, lerping linearly to x1 at the speed cap — where `speed_fraction` is the post-thrust speed over the cap for the direction of travel (`max_reverse_speed` when astern). Absent or `0.0` restores the flat speed-independent rate.

It sits inside `compute_physics`, so it applies identically to human and AI helms, and it stacks multiplicatively with the `MaxYawRate` power/damage modifiers and boost's `steering_multiplier` that `integrate_ship_physics` folds into `max_yaw_rate` first.

Authored per class, lightest hulls first: Alliance courier/destroyer `0.5`, cruiser `0.2`, battleship `0.0`; Harrow escort/destroyer `0.3`, cruiser/patrol `0.1`, warhawk `0.0`.

## Boost drive

Boost is enabled by `[helm_console.boost]` in `assets/entities/player_ship.toml`.

- `multiplier` applies to max forward speed, max reverse speed, and acceleration while engaged.
- `steering_multiplier` applies separately to `ShipPhysicsConfig.max_yaw_rate`; the player ship sets this to `2.0`.
- Battery drain is scaled by `abs(thrust) + abs(steering)` after clamping each input to `[-1, 1]`. Idle boost spends no battery; full thrust or full steering spends at the base rate; full thrust plus full steering spends at double rate.

## Server reception

Each simulation tick:
1. Look up `helm_token()` from `SessionManager`.
2. Drain admitted `SetThrust`/`SetSteering` commands per axis; keep the latest of each.
3. Pass `(state, input, dt, config)` into `compute_physics()` from `src/server/ship_physics.rs` — a pure Rust function, no Bevy.
4. Apply the resulting velocity directly to the [Ship](./ship.md)'s Rapier rigid body.

If no one is at Helm, no `SetThrust`/`SetSteering` is read and the ship coasts/decelerates.

## Helm radar

The helm console renders an overhead radar showing nearby contacts from the HTML `RadarWidget`. `gui/console-state.js` projects contacts into the ship-relative frame for `gui/helm-console.html`.

Navigation can set one shared custom waypoint. The server owns it as `SimSnapshot.navigation_waypoint`; the JS client mirrors it into `state.navigationWaypoint`; `buildHelmConsoleState()` appends a `kind: "waypoint"` blip. If the waypoint is outside Helm radar range, the blip is clamped to the radar edge and marked with `edge: true` so Helm still sees the bearing.

`gui/helm-console.html` caches the last radar payload before calling `RadarWidget.update()`, so impulse-only state pushes (`impulse_charge_progress` changing every tick while charging) don't push redundant data into the widget. For that caching to actually stop the canvas redrawing, `RadarWidget` is **render-on-demand**: its `requestAnimationFrame` loop only repaints when a dirty flag is set (by `update()`, resize, pan/zoom gestures, or an async icon load) instead of unconditionally every frame.

The HTML Helm panel also treats the charge countdown (`0 < impulse_charge_progress < 1`) as a stabilized radar phase: it pauses the decorative radar scan, disables the charge bar transition, and skips radar widget updates even if blips drift due to unrelated entity snapshots. Once progress reaches `1.0` (`ENGAGED`), radar updates resume.

### Hostile weapon-arc overlay (issue #874)

At **red alert only**, the helm radar paints a faint wedge for each online
direct-fire bank of every hostile contact on the scope, anchored at that
contact's blip.

The geometry has exactly one producer:
`weapons::arc_geometry::weapon_arc_sectors` turns a hull's authored
`facing_deg`/`fire_arc_deg` banks plus its world yaw into **world-bearing**
sectors (`bearing_deg`, `half_angle_deg`, `range`). `ai::server` calls it once
per world-snapshot rebuild and publishes the result on
`AiWorldEntity::weapon_arcs`. Two consumers read that one list:

- the helm AI, which reduces it to three scalar facts — `hostile_arc_exposure`
  (how many hostile arcs bear on this ship), `hostile_arc_escape_deg` (signed
  degrees to the shorter way out) and `hostile_arc_inescapable` (`1.0` when a
  bearing arc spans a full turn, so no amount of turning leaves it) — see
  `ai::hostile_arc_exposure`. All three are seeded by
  `helm_ai::seed_hostile_arc_facts`, which every helm policy host reaches:
  `ai_policy_state_tick` and the thrust/steering/boost hosts via
  `seed_helm_travel_facts`, and the impulse/lateral/vertical hosts directly. So
  a guard on any of the three is readable from every `[helm_console.*_ai]`
  block, including the lateral and vertical dodge axes;
- `publish_helm_blackboard`, which copies the sectors verbatim into
  `HelmBlackboard::hostile_weapon_arcs` for the **local ship only**, gated on
  `ShipRedAlert` and on the helm radar range.

`ph-helm-radar` renders those sectors without recomputing them — its only
maths is world bearing → screen angle and world position/range → scope space.
A near- or fully-all-round bank is drawn as a full disc built from two
half-circles. A single SVG elliptical arc whose endpoints coincide is omitted by
the spec, which would leave the player's scope blank against exactly the hull the
AI reads as permanently exposed — so the wedge builder switches to the disc
whenever the endpoints it is about to EMIT (after rounding to one decimal place)
are equal, not merely when `fire_arc_deg` reaches 360. `ship_harrow_lancer.toml`
authors a literal `fire_arc_deg = 360.0` twice, but a sweep just under a full
turn collapses the same way once the screen radius is small enough, which a
short-ranged bank on a wide scope reaches routinely.
Arcs are never scan-gated: they are authored hull configuration, so a helm
knows them for any hostile it can see at all. The overlay colour is authored
per hull in `[helm_console] hostile_arc_color` and reaches the client through
`ShipClientConfig::hostile_arc_color`; the parse default there, the
`ClientSimState` placeholder and every shipped hull's authored value are all the
same RGBA, so a hull that omits the key looks identical to one that sets it.

Two accuracy notes the code carries in comments and this page should not
contradict. The overlay paints **over** the radar's blips, not under them —
`.svg-overlay` is a sibling after `<ph-radar>`, so the authored 0.07 alpha, not
the stacking order, is what keeps contacts legible through it. And
`publish_helm_blackboard` runs every frame while `build_world_snapshot` runs on
the derived ~10 Hz snapshot cadence, so the sectors and anchors it publishes are
the most recent snapshot tick's and can lag the live blips by up to ~100 ms. The
AI fact reads the same snapshot, so the two never disagree.

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
- `[helm_console.boost].steering_multiplier` (player ship: `2.0`) — yaw-rate factor while boost is engaged.
- Boost battery drain factor: `abs(thrust) + abs(steering)`.

These live as constants in `compute_physics`'s `ShipPhysicsConfig` so they can be tuned without touching simulation/Bevy code.

## Related

- [Ship](./ship.md) · [Ship Physics](../concepts/ship-physics.md)
- [Radar Projection](../concepts/radar-projection.md)
- PRD #22 — Helm and Game World
