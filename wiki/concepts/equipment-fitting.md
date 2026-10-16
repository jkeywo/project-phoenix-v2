---
title: Equipment Fitting
type: concept
tags: [equipment, repair, resources, workshop, continuation]
sources: [crates/phoenix-model/src/fitting.rs, crates/phoenix-sim-gameplay/src/resources/fitting.rs, crates/phoenix-sim-gameplay/src/resources/fitting_state.rs, crates/phoenix-sim-gameplay/src/jobs/repair_refit.rs, crates/phoenix-simulation/src/resources/fitting_server.rs, crates/phoenix-simulation/src/resources/equipment_recovery_server.rs, crates/phoenix-simulation/src/snapshot/fitting.rs, crates/phoenix-sim-contracts/src/logical_clock.rs, crates/phoenix-simulation/src/snapshot/historical_clock.rs, editor/workshop-fitting.js, gui/components/fitting-controls.js, gui/fitting-plans.js, crates/phoenix-simulation/src/workshop/rehearsal.rs, crates/phoenix-simulation/src/workshop/rehearsal_report.rs, editor/fitting-rehearsal.js, gui/workshop-test-panel.js, pasm/spec/design/equipment-fitting-and-refitting.yaml]
updated: 2026-10-16
---

# Equipment Fitting

`CompiledFitting` validates optional hull slots, supported beam/reactor hardware,
actual assemblies and finite equipment lockers. Slots retain their System,
Console, compartment, operation component and energy topology. An empty fitting
profile leaves the hull's existing equipment unchanged. The Alliance Cruiser
provides the first authored choices; Dynasty explicitly clears the inherited
profile.

`ShipFitting` owns desired selections, actual equipment locations and active refit
stages. Assemblies have one identity across installation, storage and work.
Installed condition belongs to `ShipDamage`; removal transfers that condition
into the portable assembly and installation transfers it back. Fixed buffers,
links, thermal state and unrelated damage remain with their existing owners.

The target System's admitted `RequestEquipment` changes intent. Repair's own
admitted dispatch/cancel commands assign existing teams and routes. Its ordinary
AI reads permitted fitting reports and emits the same dispatch command. Refits
reserve and consume actual parts and use paid Repair tools with supported manual
work. A reactor change additionally requires an eligible, coupled, operational
facility capability. Hardware replacement uses the current capability compiler
and preserves energy continuation through `reconfigure_energy`.

`ShipEquipmentRecovery` extends the fitted Recovery tool's existing enable/AI
path. Supported field assemblies can be removed from known eligible disabled
sources into finite local equipment storage, retaining identity and condition.
Major assemblies report their facility restriction. Repair/refit/recovery claims
refer to actual physical tasks; JobId is scoped by ship and owning System.

Snapshot fitting preflight resolves candidate fitted definitions before restoring
energy and other machinery. It checks complete-roster equipment identity, work,
parts, receiving-space and physical claims before applying state. Fitting,
recovery and sensing use the existing versioned save and transport mechanisms.
Historical combat timestamps use the shared
logical tick clock and retain their canonical bits rather than lossy host ages.

Workshop edits the closed slot/library/locker/assembly declarations through
accepted-source history, reference validation, materialisation, Preview and
disposable Test. Common System details expose fitting and recovery controls.
Personal plans store hull-definition-scoped preferences separately from actual
equipment and queue ordinary requests; a favourite does not grant stock or
installation authority.

The private Workshop Test panel also supplies the rehearsal callback to that
same favourite widget. Ordinary phone/lobby widgets do not receive it. Shared
captured-source admission and a Test-only constructor stage hypothetical initial
definitions through the original fitting profile and Power capability adapter,
preserving actual assemblies and reserves before first machinery work. Two
fresh serial children compare real reference workloads; a read-only FixedLast
TestInfra report measures settled work/Stocks and validates result identity.
This source path has independent source review and targeted JavaScript coverage;
native accepted-loader and actual paid-trial evidence remain pending.
