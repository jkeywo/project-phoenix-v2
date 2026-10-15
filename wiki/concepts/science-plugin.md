---
title: Science / Sensors target
type: concept
tags: [science, sensors, scans, objectives, ai, blackboard]
sources: [crates/phoenix-sim-gameplay/src/core/messages.rs, crates/phoenix-model/src/observation.rs, crates/phoenix-simulation/src/science/mod.rs, crates/phoenix-simulation/src/science/server.rs, crates/phoenix-sim-gameplay/src/science/scan.rs, crates/phoenix-simulation/src/ship/sensors.rs, crates/phoenix-simulation/src/world/server/mod.rs, crates/phoenix-model/src/messages.rs, pasm/spec/architecture/radar-sensors.yaml]
updated: 2026-10-15
---

# Science / Sensors target

## Ownership

`science::SciencePlugin` is the authoritative scan plugin, implemented in
`crates/phoenix-simulation/src/science/server.rs` and registered by `WorldPlugin` in
`crates/phoenix-simulation/src/world/server/mod.rs`. It applies admitted scan requests in `tick_scans` and
publishes each ship's scan blackboard. The independently selected
Science/Sensors **target** is first-class per-ship state owned by the Sensors
system in `crates/phoenix-simulation/src/ship/sensors.rs`:

- The selection lives in the per-entity **`SensorRadarSelection`** component
  (`crates/phoenix-simulation/src/ship/sensors.rs`; also reachable via the
  `crate::sensors_plugin::SensorRadarSelection` alias). Every ship — player and
  NPC — carries its own.
- `handle_sensors_messages` (`crates/phoenix-simulation/src/ship/sensors.rs`) consumes admitted
  `SetScienceTarget` / `ClearScienceTarget` command payloads and writes the
  ship's own `SensorRadarSelection`.
- `operate_sensors_ai` (`crates/phoenix-simulation/src/ship/sensors.rs`) is the AI decide-and-emit
  system (issue #828): rather than writing `SensorRadarSelection` directly, it
  emits an admitted `SetScienceTarget` / `ClearScienceTarget` through the same
  command-admission seam the human path uses, so AI and human converge on one
  applier.

## Survey Scan objectives (#1139)

The same `operate_sensors_ai` host now consumes mission/doctrine
`Scan { target }` directives with Sensors affinity. After its normal AI-control
and authored `SensorsTargetSelector` gates it takes a positive Scan request.
Fitted sensing resolves that request through the actual System's permitted current
observations and observed identities/references, then lets its closed state tree
emit the same admitted `ScanTarget` command as the console. It retains an existing
work handle that still matches the request instead of replacing a human's paid
question or intrusive mode with the default. Other explicitly fed radar handles
do not implicitly replace that work. Unmigrated equipment retains its legacy
selection path.

The emitter does not decide whether the scan is legal and does not write or
latch scan state. `science::server::tick_scans`
(`crates/phoenix-simulation/src/science/server.rs`) remains the sole applier: it evaluates the
hull-authored suite, actual paid local resources, condition, interference and
range. It records the completed answer; only General completion writes the
scenario's `scan.<entity-id>.taken` flag. Optional typed question/permission and
observer-aware event owners are indexed in [Sensing and Spatial Fields](./sensing-and-fields.md). A refusal leaves
the objective active, so Backfill retries on the next deterministic Sensors
snapshot. Ordinary target designation is independent and may emit in the same
tick.

## Publishing to the blackboard (#829)

The configured Science owner retains its private machinery selection; permitted
Console publications expose the corresponding measured contact handle.
`SensorRadarBlackboard.selected_target` projects that selection through its
observing instrument. The viewscreen aggregator lifts that value into
`ViewscreenBlackboard::science_target` (`crates/phoenix-model/src/messages.rs`), and every
cross-system consumer reads the frozen `science_target` viewscreen fact rather
than the live per-ship selection. The Sensors-panel target-info snapshot also
carries `science_target_uuid` per ship (`crates/phoenix-simulation/src/ship/sensors.rs`).

## Recorded reading metadata

`scan_reading_snapshot_from_reading` projects common metadata from the existing
`ScanReading`: its subject, original `taken_at_tick`, and `condition_step` as the
quantization of `condition_fraction`. Publishing or reconnecting does not renew
that sample. Typed quantities retain their own recorded precision; a non-General
answer has no structural condition/mass field. The recorded value and precision
are unchanged. Metadata follows the
same explicit Scan feed as the reading; see [Recorded Observations](./recorded-observations.md).

## Sources

- `crates/phoenix-simulation/src/ship/sensors.rs` (`SensorRadarSelection`, `handle_sensors_messages`,
  `operate_sensors_ai`, publish systems)
- `crates/phoenix-simulation/src/science/server.rs` (`tick_scans`, the sole scan applier)
- `crates/phoenix-sim-gameplay/src/science/scan.rs` (pure scan range/fidelity derivation)
- `crates/phoenix-model/src/messages.rs` (`SensorRadarBlackboard`, `ViewscreenBlackboard.science_target`)
- Issues #828 (per-entity Sensors migration), #829 (raw-blackboard split), and
  #1139 (Scan directive and Backfill emitter)
- [Radar Projection](./radar-projection.md)
- [WeaponsPlugin](./weapons-plugin.md) (the Combat Lock counterpart, `TacticalRadarSelection`)
