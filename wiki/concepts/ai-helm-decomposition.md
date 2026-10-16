---
title: AI Helm Decomposition
type: concept
tags: [ai, helm, per-axis, commands, control-source, tick, lod]
sources: [assets/entities/alliance_cruiser.toml, crates/phoenix-sim-gameplay/src/ship/helm_ai/impulse.rs, assets/entities/fragments/ai/movement_broadside_orbit.toml, crates/phoenix-simulation/src/console/helm/server.rs, crates/phoenix-simulation/src/ship/helm_ai/mod.rs, crates/phoenix-simulation/src/ship/helm_ai/surfaces.rs, crates/phoenix-sim-gameplay/src/ship/helm_ai/facts.rs, crates/phoenix-simulation/src/ship/helm_ai/steering.rs, crates/phoenix-simulation/src/ship/helm_ai/lateral.rs, crates/phoenix-simulation/src/ship/helm_ai/vertical.rs, crates/phoenix-simulation/src/ship/helm_ai/impulse.rs, crates/phoenix-simulation/src/ship/helm_ai/boost.rs, crates/phoenix-simulation/src/ship/helm_ai/engines.rs, crates/phoenix-simulation/src/ship_plugin.rs, crates/phoenix-sim-gameplay/src/ship/helm_planner.rs, crates/phoenix-sim-gameplay/src/ai/core.rs, crates/phoenix-simulation/src/ai/cadence.rs, crates/phoenix-simulation/src/ai/lod.rs, crates/phoenix-simulation/src/ai/server.rs, crates/phoenix-presentation/src/server_app_render.rs]
updated: 2026-10-16
---

# AI Helm Decomposition

AI Helm is split by fine system. Each host decides one authored axis or drive capability, emits the same admitted command a human console would send, and is gated by that system's own `ControlSourceResolver` policy.

## Module layout

| File | Responsibility |
|---|---|
| `crates/phoenix-simulation/src/ship/helm_ai/mod.rs` | Shared policy-machine state and the public module surface. |
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

Helm's `receive_helm_coordination` owns post-lag `ArcBearingRequest`,
`ArcBearingWithdraw`, and `NavigateTo` effects. It accepts only the AI outcome,
resolves the authored Helm Station from `helm-steering`, and rechecks that fine
system's live policy before preserving arc geometry or the exact waypoint
generation. Withdrawal clears the standing request regardless of weapon family;
the steering host then reads the resulting state on the following logical tick.

## Command symmetry and ordering

The hosts call `command_admission::ai_emit::emit_ai_command`. Their payloads enter that ship's `AdmittedCommands` before `process_helm_inputs`, alongside admitted human payloads. `process_helm_inputs` is the sole normal writer of helm intent components; `apply_helm_commands` applies drive transitions; `integrate_ship_physics` consumes the complete intent set.

Authority is checked at admission. Nothing downstream can distinguish a human-issued command from an AI-issued command, and no host falls back to a coarse `helm` system id.

## Pursuit drive and torpedo holds

The Impulse axis releases a charging or active drive when a live Helm objective no longer resolves a permitted destination. The shared impulse geometry also cancels if that destination leaves the forward alignment cone, returning steering and throttle to normal control. Human-operated drives remain outside this AI path; a lull with no Helm objective retains the existing continuity behavior.

The broadside movement fragment gives both axes an authored upper-range exit from `torpedo_run`. It resumes acquisition when the target escapes `torpedo_run_exit_range` (authored beyond the entry range), provided no torpedoes remain in flight. This preserves guidance for committed rounds while preventing an uncommitted stationary hold from lasting throughout a fleeing target’s escape.

The Alliance Cruiser authors a 30-second Captain combat window to retain alert through attack-pass retreats. Its inherited 150-unit first-contact proximity trigger remains unchanged; this tuning does not add sensor knowledge or alter human control.

## Shared cadence

All policy hosts run on the deterministic logical-tick cadence in `crates/phoenix-simulation/src/ai/cadence.rs`. The world authors `sim_tick_hz`, `ai_tick_hz`, and `ai_snapshot_hz`; validation requires whole cadence ratios. Decisions therefore depend on `SimTick`, not rendered frames or wall-clock timers.

## Simulation and render LOD

AI simulation LOD in `crates/phoenix-simulation/src/ai/lod.rs` promotes nearby NPCs to `AiHighFidelity` and demotes distant ones with authored thresholds, hysteresis, and dwell. Every frozen-roster fleet ship is a permanent high-fidelity anchor on every peer; `LocalShip` is not an LOD input, so stationless GM and ship-host simulations agree. Authored world `LodBubble` entities also work without a local or fleet ship. High-fidelity ships run the full helm command/integration path. Low-fidelity ships use the cheaper deterministic path in `crates/phoenix-simulation/src/ai/server.rs` while retaining objective cursors and combat intent.

Mesh LOD is unrelated. It is selected by the renderer in `crates/phoenix-presentation/src/server_app_render.rs` from the model rig's authored levels and only changes visuals.

## Unknown target capabilities

Measured Helm contacts do not expose enemy weapon reach or facing shields. Recovery
and shadow manoeuvres use `unknown_target_safe_range` only when the doctrine authors
that fallback radius; a known reach still uses the existing reach-plus-margin ring.
The fallback does not create a threat-range fact. The broadside torpedo manoeuvre's
zero shield-gap setting explicitly permits an unknown shield finding; positive
requirements still need an observed opening. Per-axis requirement facts are seeded
in `tick_policy_machine` from that System's own parameters.

## Related

- [Helm Runtime](./helm-control-intent.md)
- [AI Ship Unification](./ai-ship-unification.md)
- [ShipPlugin](./ship-plugin.md)
- [Information-Parity Audit](./information-parity-audit.md)
