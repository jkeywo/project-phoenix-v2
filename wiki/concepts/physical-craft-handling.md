---
title: Physical Craft Handling
type: concept
tags: [craft, jobs, resources, ownership, persistence]
sources: [crates/phoenix-simulation/src/comms/reception.rs, assets/entities/reception_outpost.toml, crates/phoenix-model/src/craft.rs, crates/phoenix-sim-gameplay/src/ship/craft.rs, crates/phoenix-sim-gameplay/src/ship/craft_profile.rs, crates/phoenix-simulation/src/entities/craft.rs, crates/phoenix-simulation/src/entities/craft_runtime.rs, crates/phoenix-simulation/src/entities/craft_presence.rs, crates/phoenix-simulation/src/entities/craft_ai.rs, crates/phoenix-simulation/src/snapshot/craft.rs, crates/phoenix-presentation/src/actor_visibility.rs, gui/components/bay-controls.js, gui/workshop-system-components.js]
updated: 2026-10-16
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
space requirements and optional accepted berth names. The probe retains its entity identity, equipment and reserves.
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


## Fixed weapons and identity equipment

`assets/entities/battery_turret.toml` composes a fixed beam bank, actual local
Power and ordinary targeting/control on the same carried/deployed body. Its
explicit radar and Viewscreen feeds supply permitted aim evidence. The existing
static-defence consumer runs its ordinary per-System weapon AI; the Bay owns
handling and custody. Storage disables active work without replacing the entity,
weapon state or remaining reserve.

`assets/entities/battery_decoy.toml` instead fits one AI-only IdentityEmitter and
finite local energy. Its current paid claim reaches an observer only through
that observer's actual acquisition. Exhaustion stops the claim; it does not
destroy the recoverable body or rewrite its real identity. Both recipes pass
the bounded profile checks in Gameplay `ship/craft_profile.rs`, retaining the
fixed-pose constraints. Neither adds a moving-craft controller or physics writer.

Full-host witnesses live in `tests/battery_turret_continuation.rs` and
`tests/battery_decoy_continuation.rs`. Their current targets compile; native
execution and physical-device acceptance are recorded separately by integration.

## Finite passenger reference

`personnel_pod.toml` inherits the fixed ordinary sensor craft with one berth and
no own personnel definition. Occupants exist only in PersonnelRoster. Repair's
actual finite escort Job handles local pickup/handoff/return; Bay launch and
recovery keep their original lifecycle and physical entity identity. An active
crew handoff prevents concurrent Bay handling, refit and stripping. Current owned
stored pod facts require an explicit permitted Bay feed; deployment withdraws
live personnel telemetry. The optional six-seat Personnel Carrier grants that
feed to Engineering while Tactical retains Bay control. Broader craft and
rescue/capture acceptance remain separate consumers.


## Negotiated receiving work

`comms/reception.rs` reads retained accepted recipient-local ReceiveCraft facts
only through an explicitly permitted Comms feed. The claimed craft identity is
correlated with a separately earned current Broad alias before normal Bay
admission. Human Comms consent does not operate a Human Bay. The ordinary Bay
tree can use `has_accepted_reception`; the fixed optional reception outpost
uses that fact rather than selecting arbitrary contacts.

The same BayActivity retains the immutable accepted order and paid work. Its
physical commit stages the exact receiving-copy update and receipt, changes
actual ProbeCustody, then completes that same Job and original request facts.
The receipt carries actual receiver/Bay/Job, original order and decision clock,
physical handoff clock and sorted stable people references. The ordinary paid
reverse Comms outcome reports that evidence later. Personnel remain AboardCraft
in their sole roster; no foreign compartment transfer or second manifest owner
is introduced. A stored Bay view derives the locally aboard manifest, including
people whose original actor was removed. No remote personnel telemetry follows.

Whole-candidate checks join Bay work and Comms lineage, the Job mint frontier,
finite personnel origins and identical historical copies. Conflicting receipts
for one physical Job refuse. Fixed equipped actors use the existing authored
Comms policies and own sensor publication; no propulsion or special receiving
controller is installed. Browser-module tests and source review are separate
from actual native/radio/session and full reception acceptance.
