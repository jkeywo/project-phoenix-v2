---
title: AI Helm Decomposition
type: concept
tags: [ai, helm, per-axis, commands, control-source, tick, lod]
sources: [src/ship/helm_ai/mod.rs, src/ship/helm_ai/surfaces.rs, src/ship/helm_ai/facts.rs, src/ship/helm_ai/steering.rs, src/ship/helm_ai/lateral.rs, src/ship/helm_ai/vertical.rs, src/ship/helm_ai/impulse.rs, src/ship/helm_ai/boost.rs, src/ship/helm_ai/engines.rs, src/ship_plugin.rs, src/ship/helm_planner.rs, src/ai/core.rs, src/ai/cadence.rs, src/ai/lod.rs, src/ai/server.rs, src/server_app_render.rs]
updated: 2026-08-27
---

# AI Helm Decomposition

AI Helm is split by fine system. Each host decides one authored axis or drive capability, emits the same admitted command a human console would send, and is gated by that system's own `ControlSourceResolver` policy.

## Module layout

| File | Responsibility |
|---|---|
| `src/ship/helm_ai/mod.rs` | Shared policy-machine state and the public module surface. |
| `surfaces.rs` | Builds one frozen, read-only decision frame per ship and projects policy output. |
| `facts.rs` | Seeds authored facts and parameters into each policy host. |
| `steering.rs` | Steering host and arc-bearing override. |
| `lateral.rs` / `vertical.rs` | Translation-axis hosts. |
| `impulse.rs` / `boost.rs` | Drive-transition hosts. |
| `engines.rs` | Engine-specific publishing and AI adapters. |

The forward-thrust host remains part of the shared module surface. Together the active hosts are thrust, steering, lateral thrust, vertical thrust, impulse, and boost.

## Frozen decision surface

`build_helm_ai_surfaces_frame` runs once before the hosts on each eligible AI tick. It folds authoritative, console-owned state into a per-ship frame: the viewscreen combat lock and scored objectives, Navigation waypoint and coordination clearance, current physics, world snapshot, shields, weapons reach, authored behaviour, and per-host policy memory.

Missing information remains missing; the Helm AI does not invent a goal. Named scenario targets resolve through the same world/runtime names available to the crew-facing surfaces. Private policy memory is snapshot-safe and scoped per fine system.

## Command symmetry and ordering

The hosts call `command_admission::ai_emit::emit_ai_command`. Their payloads enter that ship's `AdmittedCommands` before `process_helm_inputs`, alongside admitted human payloads. `process_helm_inputs` is the sole normal writer of helm intent components; `apply_helm_commands` applies drive transitions; `integrate_ship_physics` consumes the complete intent set.

Authority is checked at admission. Nothing downstream can distinguish a human-issued command from an AI-issued command, and no host falls back to a coarse `helm` system id.

## Shared cadence

All policy hosts run on the deterministic logical-tick cadence in `src/ai/cadence.rs`. The world authors `sim_tick_hz`, `ai_tick_hz`, and `ai_snapshot_hz`; validation requires whole cadence ratios. Decisions therefore depend on `SimTick`, not rendered frames or wall-clock timers.

## Simulation and render LOD

AI simulation LOD in `src/ai/lod.rs` promotes nearby NPCs to `AiHighFidelity` and demotes distant ones with authored thresholds, hysteresis, and dwell. High-fidelity ships run the full helm command/integration path. Low-fidelity ships use the cheaper deterministic path in `src/ai/server.rs` while retaining objective cursors and combat intent.

Mesh LOD is unrelated. It is selected by the renderer in `src/server_app_render.rs` from the model rig's authored levels and only changes visuals.

## Related

- [Helm Runtime](./helm-control-intent.md)
- [AI Ship Unification](./ai-ship-unification.md)
- [ShipPlugin](./ship-plugin.md)
- [Information-Parity Audit](./information-parity-audit.md)
