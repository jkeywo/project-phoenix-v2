---
title: Helm Runtime
type: concept
tags: [helm, ai, impulse, boost, steering]
sources: [src/ship_plugin.rs, src/ai/core.rs, src/ai/server.rs, src/console/helm/server.rs, gui/action-map.js, gui/console-state.js]
updated: 2026-07-14
---

# Helm Runtime

Human helm sends admitted `ControlSystem` commands for thrust, steering, lateral thrust, impulse, and boost. The authoritative helm step in `src/ship_plugin.rs` applies those inputs to `ShipPhysics`; it gates human inputs when the relevant system is AI-controlled.

Helm AI currently executes on the host. It reads doctrine objectives and a world snapshot, then writes the resulting motion, lateral avoidance, impulse state, and, for Backfill, displayed helm input state. Weapons can issue an arc-bearing request that this AI consumes while the target remains valid.

The console publishes authoritative position, yaw, speeds, impulse, boost, radar, and engine state through Helm blackboards. The client only renders that state and sends controls.

The ship-scoped capability model, shared 3D planning and hazard surface, vertical movement modes, and player/AI actuator convergence are design decisions recorded in [PASM's Helm slice](../../pasm/spec/design/helm-controls.yaml).
