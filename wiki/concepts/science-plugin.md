---
title: Science / Sensors target
type: concept
tags: [science, sensors, scans, objectives, ai, blackboard]
sources: [src/science/mod.rs, src/science/server.rs, src/science/scan.rs, src/ship/sensors.rs, src/world/server.rs, src/core/messages.rs, pasm/spec/architecture/radar-sensors.yaml]
updated: 2026-08-27
---

# Science / Sensors target

## Ownership

`science::SciencePlugin` is the authoritative scan plugin, implemented in
`src/science/server.rs` and registered by `WorldPlugin` in
`src/world/server.rs`. It applies admitted scan requests in `tick_scans` and
publishes each ship's scan blackboard. The independently selected
Science/Sensors **target** is first-class per-ship state owned by the Sensors
system in `src/ship/sensors.rs`:

- The selection lives in the per-entity **`SensorRadarSelection`** component
  (`src/ship/sensors.rs:20`; also reachable via the
  `crate::sensors_plugin::SensorRadarSelection` alias). Every ship — player and
  NPC — carries its own.
- `handle_sensors_messages` (`src/ship/sensors.rs:156`) consumes admitted
  `SetScienceTarget` / `ClearScienceTarget` command payloads and writes the
  ship's own `SensorRadarSelection`.
- `operate_sensors_ai` (`src/ship/sensors.rs:768`) is the AI decide-and-emit
  system (issue #828): rather than writing `SensorRadarSelection` directly, it
  emits an admitted `SetScienceTarget` / `ClearScienceTarget` through the same
  command-admission seam the human path uses, so AI and human converge on one
  applier.

## Survey Scan objectives (#1139)

The same `operate_sensors_ai` host now consumes mission/doctrine
`Scan { target }` directives with Sensors affinity. After its normal AI-control
and authored `SensorsTargetSelector` gates it takes the first positive Scan,
resolves the name through the active world's name table (with UUID/`EntityName`
fallback), and emits the same admitted `ScanTarget` command as the console.

The emitter does not decide whether the scan is legal and does not write or
latch scan state. `science::server::tick_scans`
(`src/science/server.rs:203`) remains the sole applier: it evaluates the
hull-authored suite, power, interference, and range, then writes the reading and
the scenario's `scan.<entity-id>.taken` flag only on success. A refusal leaves
the objective active, so Backfill retries on the next deterministic Sensors
snapshot. Ordinary target designation is independent and may emit in the same
tick.

## Publishing to the blackboard (#829)

The Sensors system publishes its raw truth into the ship's `sensor-radar`
blackboard: `SensorRadarBlackboard.selected_target` mirrors this ship's
`SensorRadarSelection`. The viewscreen aggregator lifts that value into
`ViewscreenBlackboard::science_target` (`src/core/messages.rs`), and every
cross-system consumer reads the frozen `science_target` viewscreen fact rather
than the live per-ship selection. The Sensors-panel target-info snapshot also
carries `science_target_uuid` per ship (`src/ship/sensors.rs`).

## Sources

- `src/ship/sensors.rs` (`SensorRadarSelection`, `handle_sensors_messages`,
  `operate_sensors_ai`, publish systems)
- `src/science/server.rs` (`tick_scans`, the sole scan applier)
- `src/science/scan.rs` (pure scan range/fidelity derivation)
- `src/core/messages.rs` (`SensorRadarBlackboard`, `ViewscreenBlackboard.science_target`)
- Issues #828 (per-entity Sensors migration), #829 (raw-blackboard split), and
  #1139 (Scan directive and Backfill emitter)
- [Radar Projection](./radar-projection.md)
- [WeaponsPlugin](./weapons-plugin.md) (the Combat Lock counterpart, `TacticalRadarSelection`)
