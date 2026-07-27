---
title: Helm Runtime
type: concept
tags: [helm, ai, impulse, boost, steering]
sources: [src/ship/helm_admission.rs, src/ship/helm.rs, src/ai/core.rs, src/ai/server.rs, src/console/helm/server.rs, gui/action-map.js, gui/console-state.js]
updated: 2026-07-16
---

# Helm Runtime

Human helm sends admitted per-axis `ControlSystem` commands: `SetThrust` → `helm-thrust`, `SetSteering` → `helm-steering` (the client joystick's action fans out to both), plus lateral thrust, impulse, and boost payloads targeting their own systems. Since #824, `process_helm_inputs` in `src/ship/helm_admission.rs` is the sole writer of the shared intent components (`ThrustInput`, `SteeringInput`, `LateralThrustInput`, `ImpulseCommand`, `BoostCommand` in `src/ship/helm.rs`) for every ship — it applies whatever was admitted, human- or AI-sourced, with authority checked once at admission.

Helm AI executes on the host as four per-axis systems (`ai_helm_thrust`, `ai_helm_steering`, `ai_helm_lateral_thrust`, `ai_helm_impulse`), each deciding its own axis from the shared `HelmAiSurfacesFrame` (built once per AI tick) and emitting admitted commands through `command_admission::ai_emit::emit_ai_command` (the shared AI-emit helper over `validate_and_admit`), gated on its own axis's `ControlSource`, all sharing one fixed-rate sim tick (`[global] ai_tick_hz`, issues #803/#889). Each calls the pure `operate_helm` / `operate_lateral_thrust` over console-owned surfaces (Tactical's target, Navigation's waypoint, the objective cursors). Weapons can issue an arc-bearing request that `ai_helm_steering` consumes while the target remains valid. A single integrator, `integrate_ship_physics`, applies the intent components to `ShipPhysics` for human and AI ships alike. See [AI Helm Decomposition](./ai-helm-decomposition.md).

The console publishes authoritative position, yaw, speeds, impulse, boost, radar, and engine state through Helm blackboards. The client only renders that state and sends controls.

The ship-scoped capability model, shared 3D planning and hazard surface, vertical movement modes, and player/AI actuator convergence are design decisions recorded in [PASM's Helm slice](../../pasm/spec/design/helm-controls.yaml).
