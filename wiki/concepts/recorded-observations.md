---
title: Recorded Observations
type: concept
tags: [observations, knowledge, console, repair, sensors]
sources: [crates/phoenix-model/src/observation.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-model/src/wire.rs, crates/phoenix-sim-gameplay/src/core/messages.rs, crates/phoenix-sim-gameplay/src/ship/console_ownership.rs, crates/phoenix-simulation/src/gm_information/reports.rs, crates/phoenix-simulation/src/dossier/projection.rs, crates/phoenix-simulation/src/console/repair/reports.rs, crates/phoenix-simulation/src/ship/console_publication.rs, crates/phoenix-simulation/src/snapshot.rs, crates/phoenix-simulation/src/sim_digest.rs, gui/observation-details.js, gui/console-state.js, gui/sim-state.js]
updated: 2026-10-14
---

# Recorded Observations

`ObservationMetadata` describes a permitted record's subject, typed provenance,
logical sample and receipt ticks, and uncertainty. Unknown time stays absent;
`Quantized` names an existing measured field and step, not a probability.
Receipt does not refresh the sample. There is no separate observation database.

## Existing owners and projections

- Scan metadata derives from the canonical `ScanReading` and its existing fidelity.
- Dossier evidence retains its original gathered tick and public provenance label.
- Delayed Sensors reports derive metadata only from `ReportState.presented`, never
  the pending sample or current hidden truth. Their sample-and-hold cadence remains
  in `gm_information/reports.rs`.
- Repair queue contributor observations and recovery watermarks are durable state
  owned by `RepairRequestQueue`; see [Repair Runtime](./repair-plugin.md).

`SystemBlackboard::recorded_observations` borrows these held records. Ordinary
live machinery gauges have no recorded metadata. `ConsoleKnowledgeView` applies
its existing owned-System/typed-feed predicate before returning those references.
Co-hosting or displaying a report grants neither extra knowledge nor control.

## Continuation and presentation

The existing frozen `ShipSystemBlackboards` surface remains the cross-System AI
read boundary. Snapshots preserve it alongside the canonical scan/evidence/report
owners. Repair report state also participates in capture, restore and digest.
`LastConsolePublications` is only a derived recipient publication cache. Handoff
and reconnect preserve sample ticks, and elapsed time alone does not republish an
unchanged held Scan reading.

The client receives a logical observation clock with a presentation generation.
`observationTickFor` exposes it only when the selected publication belongs to that
same generation. This avoids calculating an old record's age against a newly
restored world. `observation-details.js` formats only delivered metadata and
permits quantization detail only for fields explicitly visible in that surface.
Scan, dossier, Sensors and Repair components use the shared disclosure.

Workshop continues to author typed Console feeds through the existing validation,
preview and disposable Test workflow. Metadata is produced by the observation's
owner; Workshop does not author confidence or asserted truth.

Related: [Console](../entities/console.md), [Science](./science-plugin.md).
