---
title: Electronic Equipment
type: concept
tags: [comms, observations, equipment, simulation]
sources:
  - crates/phoenix-sim-gameplay/src/electronic/identity.rs
  - crates/phoenix-simulation/src/science/identity.rs
  - crates/phoenix-simulation/src/science/broad.rs
  - gui/components/identity-controls.js
  - editor/workshop-identity.js
  - assets/entities/identity_cruiser.toml
  - crates/phoenix-sim-gameplay/src/electronic/jamming.rs
  - crates/phoenix-sim-gameplay/src/ship/composition.rs
  - crates/phoenix-simulation/src/comms/jamming.rs
  - crates/phoenix-simulation/src/regions/signal_path.rs
  - crates/phoenix-simulation/src/comms/server.rs
  - crates/phoenix-simulation/src/weapons/mounts.rs
  - crates/phoenix-simulation/src/snapshot/jamming.rs
  - gui/components/jamming-controls.js
  - gui/stations/jamming-actions.js
  - editor/workshop-system-composition.js
  - assets/entities/jamming_cruiser.toml
  - pasm/spec/design/electronic-operations.yaml
updated: 2026-10-15
---

# Electronic Equipment

The first electronic equipment consumer is the composed Comms `JammingEmitter`.
`ShipJamming` stores only its actual System's generic enabled and target channels.
Shared mount state owns traversal. Commands, ordinary System AI and equipment
controls use that same intent; there is no separate Tactical selection or target
controller. An explicit `Jam` objective is relevant to Comms and uses the existing
reading frontier before an empty target can be selected.

Aiming uses the exact owning Console's permitted, dated full-XYZ observation.
The configured maximum sample age applies to that original sample. Hidden live
positions cannot refresh retained intent. Operation, source policy, equipment
availability, actual mount coverage and finite continuous Power work gate each
emission. The Input work boundary samples the current actual emitter pose and
mount bearing; later Physics movement/traversal affects the next boundary.

`JammingEmissions` is a derived current-step view of actual paid emissions.
`RegionSignalQuery` sums each source's strength, shared falloff, coverage, natural
emitter-to-receiver Comms transmission and granted work exactly once. The factor
`1/(1+exposure)` scales each receiver endpoint's own reach. The existing natural
conversation path stays separate. Self and friendly receivers use the same
physical rule. Environmental `CommsJammed` retains its existing zero-factor denial.

Radio, sustained returned outcomes, fictional Hail and Comms-based sharing call
the same physical link owner. A distinct Input wrapper refreshes the existing
range owner before Hail; Broadcast retains its normal refresh. Unprocessed sources add no physical exposure and cannot cancel another source’s
current paid effect. Receiver notices may remain unknown; incomplete knowledge
cannot deny links or reuse prior/bootstrap credit. Every installed emitter
settles an empty result when its physical prerequisites are absent.

Snapshots retain generic channels, actual mount, Power, conditions and sensing
through their existing owners. Emissions are cleared on restore and require the
next ordinary paid boundary. Whole-candidate and single-row validation bind the
exact accepted source recipe and closed channel roster before writes, including
actors that need materialization.

The equipment detail view shows only own enabled/emitting intent and permitted
aim choices. The Comms composer can show a Boolean own-receiver interference
notice, without source identity, victim counts or success reporting. Workshop
uses the actual component schema, shared Boolean host vocabulary and accepted
composition/history transaction. `jamming_cruiser.toml` is the finite reference.
This code path does not establish the remaining electronic protection, spoof,
access, sensor interference, cloak or decoy consumers.

The composed `identity-emitter` has one Boolean enabled channel, one actual
PowerInput, real Operation/placement and one ordinary state-tree controller.
`ShipIdentityEmitters` is its canonical intent; `IdentityEmissions` contains only
fully paid current-tick claims. Missing or partly funded equipment makes no
claim, and restore clears derived claims. Stable System order selects the first
active configured claim; no observer-specific controller exists.

Broad applies that literal callsign and optional faction claim only after its
normal physical acquisition succeeds. The actual body, source association,
observer-local aim alias, geometry, classification and real actor name/faction
are unchanged. False identity samples gain no real dossier references. Explicit
observer reports retain their priority, and held history and independently
received Comms evidence are not rewritten. Own shared equipment details expose
intent and paid activity without a deception-success or observer-attribution bit.
The optional `identity_cruiser.toml` uses finite existing reactor output and
ordinary allocation; it adds no crew seat.
