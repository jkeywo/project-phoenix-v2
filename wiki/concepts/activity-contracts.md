---
title: Activity Contracts
type: concept
tags: [world, activities, repair, logistics, continuation]
sources: [crates/phoenix-model/src/activity.rs, crates/phoenix-sim-world/src/world/activities.rs, crates/phoenix-sim-world/src/world/planning.rs, crates/phoenix-sim-world/src/world/planning_compile.rs, crates/phoenix-sim-world/src/world/planning_runtime.rs, crates/phoenix-content-preparation/src/world/planning.rs, crates/phoenix-simulation/src/world/planning.rs, gui/planned-requests.js, editor/workshop-planning.js, gui/workshop-planning-panel.js, crates/phoenix-simulation/src/debug/scenario.rs, crates/phoenix-sim-world/src/world/config.rs, crates/phoenix-simulation/src/world/activities.rs, crates/phoenix-simulation/src/world/server/triggers.rs, crates/phoenix-simulation/src/console/repair/dispatch.rs, crates/phoenix-simulation/src/resources/logistics_server.rs, crates/phoenix-simulation/src/snapshot/activities.rs, crates/phoenix-simulation/src/sim_digest_activities.rs, crates/phoenix-sim-gameplay/src/jobs/activity_bindings.rs, crates/phoenix-content-preparation/src/world/activity_bindings.rs, editor/workshop-activities.js, gui/workshop-activities-panel.js, crates/phoenix-simulation/src/debug/scenario.rs]
updated: 2026-10-15
---

# Activity Contracts

Optional World `situation_definition` and `situation` arrays bind bounded Scenario
state chains to existing internal Repair or main-to-local delivery. The first
consumer supports declared base-world names and authored launch slots; unresolved
actors arm when physically present. Dynamic/layer actor bindings refuse explicitly.

Gameplay owns actual jobs, damage, parts, power and cargo. Simulation binds the
scoped JobId returned by the ordinary committed assignment. A Repair request may
visit a group, while its activity still requires one exact named target/result.
Other useful sweep work does not credit that result. Work achieved and assignment
returned are separate facts. Only actual Receive/Return commits yield delivery
facts; command feedback and stock totals do not establish receipt.

The existing pending World event owner retains each reserved bound fact until
the situation acknowledges it. These facts do not enter the transient legacy
callback buffer. World retains consumed evidence and each state's Applied/Refused
activation frontier. Scenario calls use the existing sandbox, shared tick budget
and complete script-effect application path; unavailable budget postpones work,
and sandbox refusal discards buffered effects.

Snapshot and digest traverse this same World owner. Candidate validation checks
bindings and causal evidence before writes. Retained terminal history is checked
against immutable accepted equipment/remedies, including after a team slot is
reused or the actor is removed; it does not inspect today's former job or damage.
The Scripts workspace edits these same arrays through accepted source history.
Gameplay's immutable support helper is shared by dispatch/arming and content
preparation, which checks actual base actors, row overrides and every offered
launch hull before Preview or Test. The existing privileged Scenario inspection
shows exact instances, pending facts and activation results without advancing
work or acknowledging evidence.

Optional base-world `planning` imports explicitly exported tasks from closed,
versioned domain libraries through the existing World reader. Their exact bytes
join the content ledger and browser preload before activation. Typed parameters
and the private call closure are validated before a deterministic compatible
method is chosen against the actual accepted actor's equipment. The source graph
is derived; the same World owner retains only the chosen methods, actual actor
UUID, original ordered bounded capability refusal reasons and original released-leaf ticks. A primitive dependency opens only from its
predecessor's acknowledged Accepted/Achieved/Settled/Received/Returned evidence.
Release and ordinary situation arming commit together. Whole-candidate preflight
reconstructs the deterministic choice from the accepted complete recipe and
checks method identities plus original reasons before writes.

Own Repair and Logistics publications derive released `planned_requests`.
These are mission instructions, never assertions about current damage or stock.
The existing Repair AI still requires its ordinary damage report; the existing
Logistics tree still requires permitted local demand. Human operators may
undertake a current own-System request through the ordinary correlated dispatch
action. Received feeds confer knowledge without control. The existing physical
adapters alone bind accepted JobIds and commit progress/payment/results.

The Scripts workspace edits the World planning declaration and its actually
imported library source through one backend-validated chronological transaction.
Inherited library overrides require explicit materialisation. Privileged Scenario
inspection derives selected methods, source dependencies, original refusal and
release clocks beside the separate exact situation/job/evidence view; it does
not choose, release or acknowledge work.

This first imported maintenance consumer supports one base actor or authored
launch slot and disjoint finite leaves. Dynamic opportunities, deadline/workload
scheduling, multi-actor planning and automatic plan repair remain separate work;
source checks do not establish whole-host or full increment completion.
