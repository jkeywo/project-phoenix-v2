---
title: Activity Contracts
type: concept
tags: [world, activities, repair, logistics, continuation]
sources: [crates/phoenix-model/src/activity.rs, crates/phoenix-sim-world/src/world/activities.rs, crates/phoenix-sim-world/src/world/config.rs, crates/phoenix-simulation/src/world/activities.rs, crates/phoenix-simulation/src/world/server/triggers.rs, crates/phoenix-simulation/src/console/repair/dispatch.rs, crates/phoenix-simulation/src/resources/logistics_server.rs, crates/phoenix-simulation/src/snapshot/activities.rs, crates/phoenix-simulation/src/sim_digest_activities.rs]
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
This is the first activity consumer foundation. HTN decomposition, dynamic
opportunities, workload scheduling and broader activity authoring are separate
implementation work, not implied by these types or source tests.
