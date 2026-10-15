---
title: Sensing and Spatial Fields
type: concept
tags: [science, observations, regions, simulation]
sources:
  - crates/phoenix-model/src/sensing.rs
  - crates/phoenix-sim-gameplay/src/science/tracks.rs
  - crates/phoenix-sim-gameplay/src/science/sensing.rs
  - crates/phoenix-simulation/src/science/broad.rs
  - crates/phoenix-simulation/src/science/contact_routing.rs
  - crates/phoenix-simulation/src/science/focused_ai.rs
  - crates/phoenix-simulation/src/console_delivery.rs
  - crates/phoenix-simulation/src/console_delivery_references.rs
  - gui/components/ph-scan-readout.js
  - gui/workshop-sensing-panel.js
  - crates/phoenix-simulation/src/science/server.rs
  - crates/phoenix-simulation/src/science/focused.rs
  - crates/phoenix-simulation/src/science/passive.rs
  - crates/phoenix-sim-gameplay/src/regions/field.rs
  - crates/phoenix-sim-gameplay/src/regions/signal_path.rs
  - crates/phoenix-simulation/src/snapshot/spatial_fields.rs
---

# Sensing and Spatial Fields

`SensingConfig.focused`, `broad` and `passive` are arrays of fitted instruments.
`ShipScanRecord` maps actual System IDs to active paid focused enquiries, bounded unfinished work,
retained readings and the latest-screen projection. `tick_scans` admits no free
completion: current geometry, shared Sensors path, operational condition and
actual local energy constrain progress. Completion samples published condition
and emits the existing scenario scan flag. Historical readings do not track
later live condition changes. The Console publication carries the actual owner
for controls. Each qualified `scan/{UTF8-byte-length}:{owner}` channel follows its
actual System’s Console; an exact typed feed grants information only. Completed
readings retain the public contact alias captured by the enquiry.

`PassiveProfile` and `PassiveState` map actual System IDs to independent powered
receivers, ordinary closed AI frames, enabled intent and actual measured samples.
Physical producers use actual resource expenditure, mount coverage and signal
paths. Public signals carry bearing, error, signature, strength and dated local
provenance rather than the emitting actor's private identity. Existing screens
show enquiry progress/history and passive receiver controls/readings.

`SensingConfig.broad` is an authored array of independent instruments. Each entry
names a fitted Helm, Tactical or Sensor radar and derives range/filter coverage
from that capability's existing configuration. `BroadProfile` and `BroadTracks`
are keyed by actual System ID. Each `BroadTrackState` advances only from its own
paid grant and physical measurements, retains acquired findings with their sample
ticks and predicts lost positions using observed motion. Public contact IDs include
their instrument owner; current actuator routing stays private and expires with
measured support. Co-hosting instruments does not merge their knowledge.

An observer's substituted Sensors reports enter that same instrument-owned track
lifecycle. They retain the reported position, permitted label and original sample
metadata; repeated receipt does not renew their age or acquire current physical
facts. Public metadata names the instrument's contact handle, while the actual
actor remains only a private current actuator route. Other radar instruments do
not inherit the Sensors report. Explicit synthetic points use those same retained
tracks and generic reported provenance, with no physical actuator route. Their
private no-actor distinction is never copied into a public decoy-specific list or
flag; the measured GUI shows ordinary nonselectable reported evidence.

`contact_routing` resolves a current selectable public return through the consuming
System's permitted Console view before consulting its private actuator association.
Helm, Tactical and Science publish each actual instrument's measured records;
an empty measured picture does not fall back to world entities. Ordinary target
selectors use those permitted records and the configured System's closed tree.
Additional focused controllers skip the designated Science owner. Routine reference
matching withholds ambiguous display labels, and selecting an anonymous return
does not supply a name from the machinery's live entity lookup.

The existing browser and native outbound sinks use `ConsoleDeliveryProjection`
to preserve own-ship data and explicitly charted landmarks while withholding
foreign live snapshots and unseen lifecycle events. Typed target references are
projected through their producing Console's permitted instrument observations.
The accepted client capability roster keeps this boundary active after loss of
the local hull. The scan readout selects actual owners from `scans[]`; its controls
always address that selected System.

Workshop edits the whole sensing profile through accepted source/history, with
separate add/edit/remove rows for each role. Explicit `sensing.control` bindings
identify the singular Combat Lock and Science Designation machinery owners,
without pooling the independent instruments’ observations. Authored NPC Tactical radars
now have finite explicit reach; the old absent-table unbounded acquisition is not
an alternate equipment source.

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
the snapshot artifact and shared protocol constants declare the compatibility boundary.

The broader redesign contract and incomplete work belong in PASM and the
implementation/acceptance records, rather than this current-owner index.
