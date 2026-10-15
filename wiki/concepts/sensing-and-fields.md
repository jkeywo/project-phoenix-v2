---
title: Sensing and Spatial Fields
type: concept
tags: [science, observations, regions, simulation]
sources:
  - crates/phoenix-model/src/sensing.rs
  - crates/phoenix-simulation/src/science/server.rs
  - crates/phoenix-simulation/src/science/focused.rs
  - crates/phoenix-simulation/src/science/passive.rs
  - crates/phoenix-sim-gameplay/src/regions/field.rs
  - crates/phoenix-sim-gameplay/src/regions/signal_path.rs
  - crates/phoenix-simulation/src/snapshot/spatial_fields.rs
---

# Sensing and Spatial Fields

`ShipScanRecord` owns active paid focused enquiries, bounded unfinished work,
retained readings and the latest-screen projection. `tick_scans` admits no free
completion: current geometry, shared Sensors path, operational condition and
actual local energy constrain progress. Completion samples published condition
and emits the existing scenario scan flag. Historical readings do not track
later live condition changes. The Console publication carries the actual owner
for controls; a typed feed grants information only.

`PassiveProfile` binds an independently powered System and ordinary closed AI
tree. `PassiveState` retains enabled intent and the last actual measured sample.
Physical producers use actual resource expenditure, mount coverage and signal
paths. Public signals carry bearing, error, signature, strength and dated local
provenance rather than the emitting actor's private identity. Existing screens
show enquiry progress/history and passive receiver controls/readings.

`RegionFieldState` owns bounded analytic parameters, origin and causal tick anchor.
`RegionSignalQuery` shares deterministic segment attenuation/occlusion between
actual sensing and fictional Comms reach. Explicit `SignalMaterialConfig` and
physical shape bounds describe substantial terrain; rendered mesh detail does
not define obstruction. `replace_region_field` replaces accepted field parameters
through the ordinary typed World effect boundary.

Region membership, exposure and sampled pose are derived. Restore reconstructs
these views, Region-owned modifiers and actual Comms reach before the next Input,
preserving captured hail intent and avoiding synthetic boundary events. Field and
recorded-observation continuations participate in the existing snapshot/digest;
protocol 13, snapshot 51 and rules 0.23 mark the current compatibility boundary.

The broader redesign contract and incomplete work belong in PASM and the
implementation/acceptance records, rather than this current-owner index.
