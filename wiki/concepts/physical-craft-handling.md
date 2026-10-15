---
title: Physical Craft Handling
type: concept
tags: [craft, jobs, resources, ownership, persistence]
sources: [crates/phoenix-model/src/craft.rs, crates/phoenix-sim-gameplay/src/ship/craft.rs, crates/phoenix-sim-gameplay/src/ship/craft_profile.rs, crates/phoenix-simulation/src/entities/craft.rs, crates/phoenix-simulation/src/entities/craft_runtime.rs, crates/phoenix-simulation/src/entities/craft_presence.rs, crates/phoenix-simulation/src/entities/craft_ai.rs, crates/phoenix-simulation/src/snapshot/craft.rs, crates/phoenix-presentation/src/actor_visibility.rs, gui/components/bay-controls.js, gui/workshop-system-components.js]
updated: 2026-10-15
---

# Physical Craft Handling

The first physical Bay consumer handles a fixed battery-backed sensor probe.
`assets/entities/probe_carrier.toml` adds an optional actual Craft Bay;
`assets/entities/physical_probe.toml` is ordinary finite sensing equipment with
a same-entity procedural mesh. These profiles use existing Console ownership,
energy, physical damage and observation history.

## Owners and work

`ProbeCustody(CraftLocation)` is the one location owner: Deployed, or Carried by
an actual parent UUID and Bay System. `PhysicalProbe` holds accepted immutable
space requirements. The probe retains its entity identity, equipment and reserves.
It is not represented by a second inventory item or emitted-ordnance carrier.

`ShipBayJobs` holds one existing shared Job per actual Bay. Gameplay's `BayState`
validates and advances finite work; Simulation joins paid Power work, actual
parent motion, installed compartment/access, available Operation, physical
capacity and recovery observations. Launch accepts an own stored identity;
recovery accepts a current permitted contact alias. Sensor loss pauses the same
assignment. Actual entity removal fails bound work once through the removal hook.
Voluntary cancellation settles that Job without refund or physical movement.

Paid handling advances after Dock movement in Modifiers. A ready Job hands off
custody at the next end-Physics boundary, before ordinary Rapier backend
synchronisation. The handoff rechecks physical constraints and clear release
space. No separate pending phase or deployment identity store exists.

The Bay uses ordinary per-System state trees and command admission. Its supported
facts describe its own readiness, stored roster, observed contacts and active
work. Human controls use the same admitted commands through common System status
details. Delegated summaries retain observations without granting control.

## Stored physical presence

Carried probes keep their actual Collider and rigid-body definitions, with backend
handles removed and derived Disabled membership. Operational queries omit them;
canonical snapshot, digest and authored World action target maps include them.
World destruction/group edges still require explicit destruction events; storage
does not fabricate death. An authored removal can address the actual stored actor. The existing Power owner
ages reserves and thermal state without generation or active sensing work. EMP
expiry keeps the ordinary absolute logical clock. Acquired history remains dated;
current physical sensor associations are withdrawn when the body is stored.

Presentation owns visual visibility. The shared actor visibility gate combines
observer privacy exclusions and Carried custody roots. Deployed never forces an
actor visible; clearing all hide reasons restores the gate's prior visibility.
Simulation has no renderer dependency or saved Visibility field.

## Continuation and authoring

Snapshot rows carry actual custody and Bay Jobs. Whole-roster preflight validates
accepted profiles, fixed probe pose, owners, work clocks, parent/Bay references,
unique active claims and aggregate occupied/reserved space before writes. Bulk
restore suppresses physical-loss cascades and rebuilds derived presence after
canonical rows. A single-row GM inverse cannot reconstruct a custody graph and
refuses before creating a body.

Workshop's existing System component editor exposes supported Bay configuration,
Power input and the ordinary tree vocabulary. Source edits retain the existing
accepted transaction/history boundary. Entity fields expose probe occupied space;
World rows bind initial `carried_by` to an explicit existing parent and Bay.
Initial carried GameStart rows follow explicit unconditional crew slots, preserving
ordinary ship classification and avoiding extra claimable seats.
