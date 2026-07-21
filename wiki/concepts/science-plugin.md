---
title: Science / Sensors target
---

# Science / Sensors target

> **This page was rewritten (2026-07).** The old `SciencePlugin` /
> `handle_set_science_target` / `ScienceTargetSuggestion` /
> `src/science_plugin.rs` model described here no longer exists — it was a
> stale advisory-message design superseded by the per-entity Sensors migration
> (#828) and the raw-blackboard split (#829). The current reality is below.

## Ownership

There is no standalone `SciencePlugin`. The Science/Sensors **target** — what
used to be called the "science target" — is now first-class per-ship sensor
state owned by the Sensors system in `src/ship/sensors.rs`:

- The selection lives in the per-entity **`SensorRadarSelection`** component
  (`src/ship/sensors.rs:18`; also reachable via the
  `crate::sensors_plugin::SensorRadarSelection` alias). Every ship — player and
  NPC — carries its own.
- `handle_sensors_messages` (`src/ship/sensors.rs:122`) consumes admitted
  `SetScienceTarget` / `ClearScienceTarget` command payloads and writes the
  ship's own `SensorRadarSelection`.
- `operate_sensors_ai` (`src/ship/sensors.rs:627`) is the AI decide-and-emit
  system (issue #828): rather than writing `SensorRadarSelection` directly, it
  emits an admitted `SetScienceTarget` / `ClearScienceTarget` through the same
  command-admission seam the human path uses, so AI and human converge on one
  applier.

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
- `src/core/messages.rs` (`SensorRadarBlackboard`, `ViewscreenBlackboard.science_target`)
- Issues #828 (per-entity Sensors migration) and #829 (raw-blackboard split)
- [Radar Projection](./radar-projection.md)
- [WeaponsPlugin](./weapons-plugin.md) (the Combat Lock counterpart, `TacticalRadarSelection`)
