---
title: Helm Control Intent
type: concept
tags: [helm, ai, impulse, boost, steering, pasm]
sources: [src/ship_plugin.rs, src/ai/core.rs, src/ai/server.rs, src/console/helm/server.rs, gui/action-map.js, pasm/spec/architecture/helm-controls.yaml]
updated: 2026-07-14
---

Summary

The shipped helm path splits cleanly into two modes. Human helm sends admitted `ControlSystem` commands for thrust, steering, lateral thrust, impulse, and boost, while helm AI reads scored doctrine objectives plus a world snapshot and directly updates authoritative motion, cached helm intent, and impulse state on the host. The intended next-step design keeps that code truth visible, but moves toward a shared 3D desired-motion plus hazard-assessment surface consumed by fine helm systems.

The Phase 7 PASM design slice now records Helm player agency, tactical arc-request coordination, shared collision-hazard information, actuator capability failure, and the agreed impulse/hazard-response tuning intent in `pasm/spec/design/helm-controls.yaml`.

## Human helm path

- The phone client sends coarse helm commands through `gui/action-map.js:143`, `gui/action-map.js:165`, `gui/action-map.js:173`, `gui/action-map.js:181`, and `gui/action-map.js:344`.
- `process_helm_inputs` in `src/ship_plugin.rs:336` is the authoritative motion step. Every 1/30s it reads admitted `HelmInput` and `LateralThrustInput`, updates `LastHelmInput`, gates out if helm is AI-controlled, and then runs `compute_physics`.
- If impulse is active, human thrust and steering are ignored and the motion input is forced to full forward with zero steering and zero lateral thrust (`src/ship_plugin.rs:423`).
- Fine helm-engine damage scales thrust before physics: two dead engines means zero thrust, one dead engine halves thrust (`src/ship_plugin.rs:444`).
- `handle_impulse_messages` in `src/ship_plugin.rs:1181` starts or cancels charge on admitted helm commands and also auto-cancels on hull damage. It refuses to start impulse inside an impulse-blocking region.
- `handle_boost_messages` in `src/ship_plugin.rs:1239` toggles or explicitly sets boost only when the ship's TOML enables boost.
- `tick_boost` in `src/ship_plugin.rs:1277` drains boost battery from live helm demand, with impulse-active travel treated as full-forward demand.

## Published helm state

- `publish_helm_blackboard` in `src/console/helm/server.rs:28` derives the replicated helm snapshot from authoritative motion, impulse, boost, modifiers, and cached input.
- The main replicated surface is `HelmBlackboard` in `src/core/messages.rs:1672`, which carries position, yaw, forward speed, lateral speed, impulse progress, boost state, and damage-scaled radar range.
- Lateral-thrust status is published separately as `HelmLateralThrustBlackboard` in `src/core/messages.rs:1698`.
- Per-engine thrust/online telemetry is also derived server-side in `src/console/helm/server.rs:91`.
- The client assembles these into helm UI state through `buildHelmConsoleState` in `gui/console-state.js:617`, and the ship-specific helm pages bind that state into joystick, impulse, boost, and radar components such as `gui/battleship/helm.html:56`.

## Helm AI path

- Helm AI is not implemented as synthetic helm button presses or joystick messages.
- `aggregate_doctrine_blackboards` in `src/ai/server.rs:286` writes scored doctrine objectives into each ship's viewscreen blackboard, and `build_world_snapshot` in `src/ai/server.rs:230` provides the entity snapshot helm AI reads.
- `operate_helm_ai` in `src/ship_plugin.rs:547` is the authoritative executor. It reads scored helm objectives, filters visible entities by helm radar range, calls `operate_helm` from `src/ai/core.rs:394`, computes lateral avoidance with `operate_lateral_thrust` from `src/ai/core.rs:855`, and writes the resulting motion directly into `ShipPhysics`.
- For the local player ship under Backfill, `operate_helm_ai` also writes `LastHelmInput` so blackboards and UI reflect AI intent (`src/ship_plugin.rs:868`).
- Helm AI makes impulse decisions directly with `decide_impulse` from `src/ai/core.rs:77` and mutates `ShipImpulse` in place (`src/ship_plugin.rs:830`).
- Partial automation is separate: `operate_lateral_thrust_ai` in `src/ship_plugin.rs:990` runs only when lateral thrust is AI-controlled but main helm remains human-controlled.
- Weapons can bias helm AI through `PendingArcBearingRequest`, which `operate_helm_ai` consumes to steer into firing arc until the request is satisfied or the target disappears (`src/ship_plugin.rs:735`).

## Intended evolution

- Helm capabilities are ship-authored, not universal. Engines, steering, lateral thrust, vertical thrust, impulse, and boost should all be treated as per-ship capabilities; a ship without engines is effectively a starbase rather than a special-case code path.
- Collision avoidance should move out of separate helm and lateral AI helpers into one ship-level planning pass built from shared world perception.
- The intended planning surface is 3D and local-space even though most current ships still move in-plane. That avoids locking more code into X/Z-only assumptions before bounded or fully 3D craft arrive.
- Shared helm intent should split desired travel from desired orientation. `desired_velocity_local` and `desired_facing_local` should both be published so arc-bearing and docking requests can influence facing without forcing the same movement vector.
- Arc-bearing requests affect facing only; they must not cause slow lateral or reverse drift to improve firing arcs. Docking is a separate future intent that may legitimately use controlled lateral and reverse movement.
- AI should select the same typed actuator inputs a player supplies. Controller identity ends at input arbitration, before actuator and physics processing.
- Fine helm systems should read shared intent rather than invent their own avoidance outputs:
  - `helm-engines` owns forward/reverse thrust and may slow for arrival or collisions.
  - `helm-steering` owns rotation, currently constrained to yaw.
  - `helm-lateral-thrust` owns sideways translation and currently serves horizontal collision avoidance for AI.
  - `helm-vertical-thrust` is a separate non-player-facing fine system for AI vertical avoidance.
- Shared hazard assessment should publish capability-style facts rather than rigid categories, at least including `movable`, `dangerous`, and `size_rating`.
- Hazard assessment should add boids-like 3D force contributions to objective travel intent. A ship ignores hazards smaller than itself, while each actuator's authored distance and force sensitivity implicitly determines response priority.
- Shared hazard assessment may be filtered differently by each fine system. In particular, intended `helm-vertical-thrust` should react only to moving hazards, while engines and lateral thrust may still respect static hazards.
- Planned vertical support is ship-scoped through `vertical_movement_mode = none | bounded | full_3d`:
  - `none` keeps the ship planar.
  - `bounded` allows AI-only vertical offset inside authored limits so ships can slip above or below traffic without turning the game into full player-facing 3D flight, then return gradually toward the cruise plane.
  - `full_3d` implies six-degree-of-freedom craft.
- Once ship Y separation becomes real simulation truth, torpedoes should also move in full 3D rather than staying effectively planar.
- Impulse retains harsh steering rather than disabling it outright: the standard authored steering multiplier is `0.1`. Boost is unavailable during impulse.

## PASM takeaway

The PASM helm slice should model two different authority shapes:

- Human helm crosses the client-to-host trust boundary as admitted commands.
- Helm AI stays entirely on the authoritative host side and mutates motion/drive state directly.
- The intended next PASM step is a shared 3D desired-motion and hazard-assessment contract that fine helm systems consume according to their capabilities.
