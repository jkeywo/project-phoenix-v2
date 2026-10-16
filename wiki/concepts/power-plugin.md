---
title: Power Runtime
type: concept
tags: [power, energy, battery, coolant, heat, ai, continuation]
sources: [crates/phoenix-sim-gameplay/src/resources/energy.rs, crates/phoenix-sim-gameplay/src/resources/energy_network.rs, crates/phoenix-sim-gameplay/src/resources/energy_topology.rs, crates/phoenix-sim-gameplay/src/modifiers/power_system.rs, crates/phoenix-sim-gameplay/src/ship/power.rs, crates/phoenix-simulation/src/ship/power.rs, crates/phoenix-simulation/src/ship/energy_runtime.rs, crates/phoenix-simulation/src/ship/energy_publication.rs, crates/phoenix-simulation/src/console_ai/server.rs, crates/phoenix-simulation/src/snapshot/preflight.rs, crates/phoenix-simulation/src/sim_digest.rs, editor/workshop-energy.js, editor/workshop-energy-ai.js, gui/components/ph-energy-controls.js, gui/stations/energy-actions.js, pasm/spec/design/local-energy-and-thermal.yaml]
updated: 2026-10-16
---

# Power Runtime

Each equipped entity owns one `ShipPowerSystem`. Its `PowerSystem` contains
immutable authored hardware and one `EnergyNetworkState`. The reactor capacitor
and every consumer reserve reuse Gameplay's continuous energy/thermal processor.
Incoming flow fills reserves; actual machinery spends them. Fullness-dependent
loss, gross charging/depletion heat and thermal trip/recovery live in that owner.
There is no allocation-to-speed/damage multiplier or ship-wide exhaustion lock.

Power stages one candidate for each energy/stock transaction. Its internal network
processor borrows that candidate and retains network pre/post validation; public
network calls still stage their own candidate. Runtime observation validation
borrows the current readings rather than materialising a save snapshot. Failure
discards the candidate before actual energy, coolant credit or readings commit.

Power retains a compiled immutable definition with indexed group ancestry,
children and ordered consumers. Construction and refitting validate and replace
that definition together. The live tick reuses it while retaining pre/post checks
on all mutable reserves, allocations, cuts, cooling and resource use. Raw Network
APIs and continuation preflight retain full definition validation; the indices
are derived data and do not change saved state or deterministic digests.

Battery-only equipment uses the same network with explicitly zero sustainable
generation. Positive intake capacity cannot supply energy without actual incoming
flow. [battery_probe.toml](../../assets/entities/battery_probe.toml) exercises finite ordinary sensing and
retained history through exhaustion, without a separate deployable power owner.

Gameplay's `resources/energy_network.rs` resolves nested flow/cooling budgets,
manual whole-flow cuts, restoration holds and Minimal priority allocation. Rank
zero is first; shared resources respect ancestor rank before fine sibling rank.
Rejected intake stays at its source. Saved requested values survive a cut.

Simulation's `ship/energy_runtime.rs` joins that processor with the existing
World-owned coolant stocks through explicit service bindings. It accounts for
preceding queued changes and queues only actual integer withdrawals. Already
withdrawn fractional coolant remains in canonical continuation, not another stock.
The independent `PowerFlowCuts` RNG stream keeps cuts reproducible without
changing damage or Comms draws.

`ShipPowerPlugin` clears the existing derived work-emission receipts in
`FixedFirst`, after the tick-scoped mint sync and before all Input work.
`tick_power_system` preserves those receipts through Physics, so later passive
signature consumers include actual Input-paid equipment work in the same
interval. The next `FixedFirst`, continuation restore and stored-energy
processing clear receipts. Direct Gameplay drivers explicitly call
`PowerSystem::begin_work_interval` before their first machinery payment;
energy integration alone does not start a new interval. These receipts are
neither saved state nor additional resource credit.

`handle_power_messages` applies admitted reactor, group, priority, management-mode
and fine-flow settings transactionally. Human and ordinary reactor AI use the
same System-addressed commands. `ai_power_allocation` executes the authored
numeric state tree against permitted Console Power observations; no legacy
ranked-rung controller is installed. Private tree state survives handoff.
The optional Boolean `priority-allocation` output selects the existing Minimal
or Manual mode through normal admission. An absent output preserves the current
mode. Shipped trees choose priorities so low reserves cannot permanently strand
a randomly cut branch under unchanged reactor output; no hardware capacity or
operating cost is increased.

`ship/energy_publication.rs` projects actual reserves, heat, desired/effective
flows, cuts and coolant. Radar/navigation work is funded once at the physical
tick boundary; publishers consume its recorded fraction without spending energy.
The common equipment status distinguishes stored-energy loss, thermal stops and
intake cuts, and never treats zero intake alone as an unavailable charged System.

Engineering, dedicated Power and Courier use the same `ph-energy-controls`.
Min/mid/detail depth is Console working-view state. Workshop edits whole hardware
profiles, supported numeric trees and physical consumer composition through its
existing source/history/validation/Preview/Test workflow. Old packs require
explicit energy hardware and cannot install both old and new Power controllers.

Snapshots carry local reserves, thermal/ramp state, allocation intent,
cut/restoration state, fine overrides, funded-work readings and coolant credit.
Preflight validates identities and physical stock references before restoration;
all energy owners participate in deterministic digests. Existing transport and
Console ownership remain in force.

Explicit Tactical weapons restraint is independent of intake. Scenario
`hold_fire`/`release_fire` orders and Tactical controls update the same canonical
intent; existing weapons-cold flags mirror it. Dynasty strike boost spends the
same reactor capacitor, with paid attack bonuses retained on in-flight attacks.

See [resource stores](resource-stores.md), [weapons intent](weapons-intent.md) and
[ship physics](ship-physics.md) for the actual work and replenishment consumers.
