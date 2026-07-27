---
title: AI Helm Decomposition
type: concept
tags: [ai, helm, per-axis, intent, lod, control-source, tick]
sources: [src/ship/helm_ai.rs, src/ship_plugin.rs, src/ai/core.rs, src/ai/lod.rs, src/ai/server.rs, src/ship/helm.rs, src/world/config.rs, src/server_app.rs, src/entities/config.rs]
updated: 2026-07-16
---

# AI Helm Decomposition

The `operate_helm_ai` monolith is gone (#704). AI helm control is four per-axis Bevy systems, each the sole writer of one intent component, gated on its own declared system's `ControlSource` — never a coarse fallback. Human and AI input converge on the same intent components, which one integrator consumes. This page covers the per-axis pattern, the intent-component surface, and the two LOD systems that scope it.

## Per-axis AI systems

All four live in `src/ship/helm_ai.rs`:

| System | Intent component written | Gate (own axis only) |
|---|---|---|
| `ai_helm_thrust` | `ThrustInput` | `helm-thrust`.operate_ai |
| `ai_helm_steering` | `SteeringInput` | `helm-steering`.operate_ai |
| `ai_helm_lateral_thrust` | `LateralThrustInput` | `helm-lateral-thrust`.operate_ai |
| `ai_helm_impulse` | `ImpulseCommand` | `helm-impulse`.operate_ai |

Rule-6 gating: each system checks `ControlSourceResolver::policy_for(<own system id>).operate_ai` and nothing else. The coarse `helm` id was deleted as a system by #801 (it survives as `HELM_STATION_ID`, the Helm console blackboard key), so there is no coarse policy for an axis to fall back on. An axis a hull does not declare is an axis no AI drives — `ControlSource::default()` is Human — which is why every shipped hull declares all four axes. `helm_axes_operate_ai` (used by display-mirror logic) means "both stick axes AI", derived from the per-axis declarations.

Decisions come from the pure `operate_helm` in `src/ai/core.rs` (thrust + steering; `ai_helm_lateral_thrust` uses the separate pure `operate_lateral_thrust`). Because `operate_helm` is a pure function of identical inputs, each per-axis system calls it independently and keeps only its own axis — there is deliberately no shared cached decision component. `ai_helm_steering` additionally folds in the Weapons→Helm `PendingArcBearingRequest` (channel-3 `ArcBearingRequest`, issue #677) via `apply_arc_bearing_request`, clearing it when the target is gone or a phaser arc already bears.

### Console-owned goal surfaces

The helm AI owns no goals; it reads shared surfaces a human could equally drive (`HelmAiSurfaces` in `src/ship/helm_ai.rs`):

| Surface | Owner | Answers |
|---|---|---|
| `TacticalRadarSelection` | Tactical (human `SetTarget` / `ai_target_selection`) | who to pursue |
| `NavigationWaypoint` + `HelmWaypointClearance` | Navigation (+ the channel-3 lag) | where to travel |
| `ObjectiveCursors` | `advance_objective_cursors` (`SimSet::Modifiers`, sole writer) | where on the route |

A missing surface means "no goal from that console", never a fabricated default.

### Shared combat targets

For an active `Destroy` directive, Helm also consumes explicit targets already
selected by Tactical, Sensors, or an entity-anchored Navigation waypoint.
Those selections add only the selected live entity to Helm's actionable view,
so Helm may pursue it beyond its own radar range without gaining general
long-range detection. Untargeted `Destroy` combat doctrine accepts only an
opposing-faction candidate (Tactical first, then Sensors, Navigation, then
Helm's own radar-limited hostile scan); named Destroy objectives retain their
authored target.

### Shared sim tick (issue #803)

All four systems share one `run_if(ai_helm_tick_ready)` gate. `AiHelmTickTimer` is a repeating timer; `tick_ai_helm_timer` advances it (after all four systems, so the flag is consumed before re-arming) and latches `AiHelmTickReady`. This decouples AI helm cadence from the rAF-driven frame rate — without it a 144 Hz host would steer on ~4x fresher data than a 60 Hz one, the nondeterminism PRD #620 exists to remove. The rate is TOML-authored: `[global] ai_helm_tick_hz` (`GlobalConfig::ai_helm_tick_hz` in `src/world/config.rs`, serde default 30 Hz), reconciled against the loaded world config each frame.

### Ordering

Since #824 the AI systems emit admitted commands rather than writing intents: `build_helm_ai_surfaces_frame` runs after `AiTickLabel` and before all four per-axis systems; the four emit `SetThrust`/`SetSteering`/lateral/impulse payloads through `command_admission::ai_emit::emit_ai_command` (the shared AI-emit helper over `validate_and_admit`, issue #738) into their own ship's `AdmittedCommands`, all **before** `process_helm_inputs`, which applies every admitted helm payload per-entity (human- and AI-sourced alike) and is the single writer of `LastHelmInput` (LocalShip only). `apply_helm_commands` runs after it, consuming `ImpulseCommand`. The registration comments in `src/ship_plugin.rs` state the full contract.

## Intent-component surface

`src/ship/helm.rs` declares the five components that decouple *admission* from *physics integration*:

| Component | Meaning |
|---|---|
| `ThrustInput(f32)` | Desired forward/reverse thrust, `[-1, 1]` (same range as `SetThrust`) |
| `SteeringInput(f32)` | Desired yaw input, `[-1, 1]` (same range as `SetSteering`) |
| `LateralThrustInput(f32)` | Desired strafe thrust |
| `ImpulseCommand(ImpulsePhase)` | Desired impulse phase transition (only `Idle`/`Charging` ever commanded; idempotent) |
| `BoostCommand(bool)` | Desired boost engagement |

Writer contract — one writer per axis per tick (#824):

- `process_helm_inputs` is the sole intent-component writer for every ship: it applies each ship's `AdmittedCommands` (per-axis wire: `SetThrust` → `helm-thrust`, `SetSteering` → `helm-steering`, since #801) regardless of whether the command was admitted from a human token or an `ai:` token — authority is checked once at admission.
- The four per-axis AI systems decide and **emit admitted commands**; they no longer write intent components directly.

Sole consumer: `integrate_ship_physics`, the single helm-path writer of `ShipPhysics` for the player ship and every AI-promoted NPC. A debug-only tripwire (`HelmPhysicsWriteGuard` + `HelmPhysicsFrame`, issue #699) panics if two helm-path writers stamp the same ship in one frame; the four sanctioned out-of-band `ShipPhysics` writers (collision, recoil, slow-zone clamp, low-LOD dead reckoning) are documented on `ShipPhysics` in `src/ship/state.rs` and do not opt in.

All five components are scoped to `AiHighFidelity`: they exist only on ships running full-fidelity helm (the player's `LocalShip`, always, and NPCs while promoted).

## LOD architecture

Two independent LOD systems share nothing but the word.

### AI simulation LOD

Pure evaluation in `src/ai/lod.rs`: `evaluate_lod(current, distance, sensor_range, …) -> LodState` (High/Low). Promotion (Low→High) is immediate when `distance <= sensor_range`; demotion requires `distance > sensor_range * 1.2` (`LOD_HYSTERESIS = 0.2`) **and** a 2 s dwell (`LOD_DWELL_SECS`) since the last transition — hysteresis plus dwell prevent oscillation at the range boundary.

`lod_ai_ships` (`src/ai/server.rs`) applies it per NPC against the player ship's position, inserting/removing the `AiHighFidelity` marker **and its scoped intent bundle** together: the five helm intent components plus `ShipFrequencyHintState`, `ShipPowerAiState`, `TorpedoIntents` (`ShieldArcIntents` retired into admitted emissions by issue #826; `PowerReactorIntents` likewise retired by issue #831 — NPC power now flows as admitted `SetPowerGroupAllocation`). `LocalShip` is never evaluated and always keeps its marker.

What each fidelity level runs:

- **High**: the full per-axis AI decision systems, physics via `integrate_ship_physics`.
- **Low**: `simulate_low_lod_ships` (`src/ai/server.rs`) dead-reckons `ShipPhysics.x/z/yaw` directly — a sanctioned out-of-band writer, filtered `Without<AiHighFidelity>` so it can never fight the integrator. A low-LOD ship with a Patrol/Reach objective still cheaply follows its route: it reads the same `ObjectiveCursors` the high-LOD path reads (one cursor surface since #702), so promotion/demotion resumes the route exactly where it left off. A ship that flies a **non-looping** route to its end (`route_completed`, `src/ai/patrol_cursor.rs`) has arrived: it coasts to a stop and holds station, matching the high-LOD path's zero decision inside the arrival radius. Only a genuinely routeless ship gets the dumb forward-drift.
- Low-LOD NPCs **keep firing phasers**: `ai_phaser_auto_fire` (`src/console/weapons/mod.rs`) is deliberately not filtered on `AiHighFidelity` — phaser fire is the main damage low-LOD NPCs contribute, and `phaser_auto_fire_runs_for_low_lod_npc_without_ai_high_fidelity` pins it. Torpedo auto-fire and shield-focus AI are high-LOD only (`ai_shield_focus` in `src/console_ai/server.rs` skips low-LOD ships, which retain whatever focus they had when demoted).

### Mesh (render) LOD

Entirely separate, client-visual only. `MeshConfig.lod: Vec<LodLevel>` (`src/entities/config.rs`) declares near→far distance bands; each `LodLevel` is a GLB (`model`) or procedural (`shape`) level with optional per-band visual overrides falling back to the flat `MeshConfig` fields. `select_lod` picks the band for the camera distance with a `LOD_HYSTERESIS_MARGIN = 5.0` world-unit band against flip-flopping. `update_mesh_lod` (`src/server_app.rs`) drives entities carrying the `MeshLods` component, swapping the active visual (GLB scene child vs procedural mesh on the parent) when the level changes.

## Cross-references

- [Helm Runtime](./helm-control-intent.md) — the human/console side of the same intent surface
- [Coarse-system migration](./coarse-system-migration.md) — the id namespaces and per-axis wire
- [AI Ship Unification](./ai-ship-unification.md) — the per-kind AI plugin pattern this decomposes
