---
title: Activity Contracts
type: concept
tags: [world, activities, repair, logistics, continuation]
sources: [crates/phoenix-sim-world/src/world/planning_dispositions.rs, crates/phoenix-simulation/src/world/planning_monitor.rs, crates/phoenix-sim-world/src/world/planning_opportunities.rs, crates/phoenix-content-preparation/src/world/opportunities.rs, crates/phoenix-simulation/src/world/opportunities.rs, crates/phoenix-simulation/src/world/opportunity_inputs.rs, crates/phoenix-model/src/activity.rs, crates/phoenix-sim-world/src/world/activities.rs, crates/phoenix-sim-world/src/world/planning.rs, crates/phoenix-sim-world/src/world/planning_compile.rs, crates/phoenix-sim-world/src/world/planning_runtime.rs, crates/phoenix-sim-world/src/world/planning_temporal.rs, crates/phoenix-simulation/src/world/planning_inputs.rs, crates/phoenix-simulation/src/lobby/crew_replication.rs, crates/phoenix-simulation/src/snapshot/crew_presence.rs, crates/phoenix-content-preparation/src/world/planning.rs, crates/phoenix-simulation/src/world/planning.rs, gui/planned-requests.js, editor/workshop-planning.js, gui/workshop-planning-panel.js, crates/phoenix-simulation/src/debug/scenario.rs, crates/phoenix-sim-world/src/world/config.rs, crates/phoenix-simulation/src/world/activities.rs, crates/phoenix-simulation/src/world/server/triggers.rs, crates/phoenix-simulation/src/console/repair/dispatch.rs, crates/phoenix-simulation/src/resources/logistics_server.rs, crates/phoenix-simulation/src/snapshot/activities.rs, crates/phoenix-simulation/src/sim_digest_activities.rs, crates/phoenix-sim-gameplay/src/jobs/activity_bindings.rs, crates/phoenix-content-preparation/src/world/activity_bindings.rs, editor/workshop-activities.js, gui/workshop-activities-panel.js, crates/phoenix-simulation/src/debug/scenario.rs]
updated: 2026-10-16
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

Optional `planning.temporal` profiles and primitive forecasts add bounded tick
scheduling to that same choice. The pure World owner computes conservative
max-duration/recovery intervals, exact phase-overlap attention and physical
resource exclusion; those forecasts claim neither resources nor completed work.
The saved decision retains its original input, absolute deadline and either
committed intervals or a recomputed refusal. Actual leaf release additionally
requires its scheduled start and the same acknowledged physical evidence.

Simulation projects actual source, eligible hosting Station, rating and canonical
`ShipCrewPresence` into the original forecast. Presence joins existing admitted
crew rating transitions and mesh capture/restore; default Human controls and
rating-only GM commands cannot create people. Ordinary in-play reconnect and
unready retain their established operational authority. Historical validation
checks immutable Console hosting candidates and Station/rating vocabulary, not
today's Sessions or a guessed historical Console depth. GM/visiting overrides
mean the host Station rating alone does not prove a past source.

The same author inspection shows retained temporal decisions, intervals, original
participation and exact minute workload; its current tick only marks overdue.
The existing backend-validated source transaction edits nested temporal profiles
and primitive phases without a second authoring acceptance owner. Estimates in
the opt-in maintenance fixture remain provisional, not measured human workload.

This first imported maintenance consumer supports one base actor or authored
launch slot and disjoint finite leaves. Multi-actor
planning and automatic plan repair remain separate work; source checks do not
establish whole-host or full increment completion.


A refused temporal decision with no released leaves can append at most eight
explicit actual crew/control-change decisions on the same PlanChoice. The new
schedule begins at its dated cause while preserving the original method graph,
difficulty profile and absolute deadline. Unchanged inputs do not retry; a
committed decision freezes this first recovery path. Preflight recomputes every
record, including exact changed owners and clocks; release uses the effective
commitment. Original and revised decisions remain separately visible. This is
refusal recovery, not unfinished-job replanning or a capability-loss solution.


Communication primitives reuse the same situation/fact history with a closed
ActivityExecution reference: actual physical JobId or actual Comms origin and
recipient/emission identity. Only already-known current own Comms contacts enter
planned communication publications. The normal paid SendCommsRequest consumes an
exclusive prepared World binding; a payment refusal drops it without acceptance.
Normal Comms AI uses its existing policy and reading owner to send that command.

The canonical retained Request retries through ordinary paid radio operation.
World observes only the issuer's own sent lineage and actually heard returned
Outcomes. Original reading/report clocks and actual receipt clocks remain distinct;
ReportStatus does not grant permission. Candidate validation reconciles retained
requests, accepted scopes and receipt evidence before writes. The imported
communication-maintenance domain and existing Workshop source/activity editors
use these same closed operators and read/reported/declined conditions. Actual
whole-host, peer and human workload acceptance remains separate from source tests.


Scenario activation results retain their actual script-effect application clocks
on the same SituationInstance. The bounded result and tick maps have identical
state keys. A `scenario_applied` dependency names an explicit callback and waits
for its actual Applied result; a report receipt or refused callback cannot satisfy
it. Scenario-local references point only to an earlier callback in the acyclic
chain. Preflight checks causal, nonfuture and nondecreasing clocks; inspection
shows the application clock separately from the underlying physical/radio fact.
This records a historical authored consequence and does not create another
permission inventory or prove arbitrary past script effects from current state.


An opt-in root can name `planning.required_commitment`. The existing unique
World promise must be Open before Simulation first chooses that root; offers and
declines do not release mission instructions. PlanChoice retains that exact ID,
and preflight validates its original made/resolved interval from the same saved
CommitmentLedger. Later Kept/Broken resolution preserves accepted work and clocks.
A same-tick script acceptance can await the next collection pass; an older open
interval cannot explain a missing accepted root. The existing Workshop source
transaction edits this binding and privileged Scenario inspection displays it.
The committed-assistance probe exercises ordinary Comms acceptance and actual
machinery evidence; the integration owner's native accept/refuse host checks
pass. Physical custody and full Patrol acceptance remain separate.


An opt-in base World `opportunity_search` imports real typed maintenance operators
through the same accepted content closure. Its bounded scratch search reads actual
own reported damage, finite local/main stores, routes and outstanding work. It
forecasts a useful structural repair, with delivery first only when parts are
missing. Search does not dispatch jobs or move resources.

The existing collector retains bounded immutable proposals, original observations
and search inputs. An exact internal own-Comms dialogue records Accept or Decline
through ordinary Human/AI response admission. Acceptance revalidates actual need
and resources, then stages one generated promise and a distinct optional
Decomposition in the existing Situation owner. Primary choice and history remain
unchanged. Released requests use the same Console publications and physical
Repair/Logistics bindings; later leaves wait for actual prerequisite evidence.

Snapshot preflight reconstructs original proposals and optional namespaces from
accepted operator sources before writes. Actual settled jobs and finished
callbacks gate reuse; achievement alone does not free a team. Privileged Scenario
inspection shows the retained input, steps, decisions and release clocks. Workshop
uses its existing accepted source/document history and Preview/Test path.

The first host probe uses a Human to issue the released delivery and repair,
then delegates actual partial work before complete JSON continuation. Autonomous
coordination of future repair intent, Human overrides of frozen pending work,
pacing and full continuing-mode acceptance remain open. This is a bounded
maintenance consumer, not full generation or whole increment completion.

Offered records require a unique retained pending/active internal Comms envelope.
Final script-root failures record a dated closed source-unavailable cause without a
player decision or Promise; budget deferral retains the pending envelope. A bound
reply missing its decision refuses before buffered effects or response selection.
Retained originating response metadata cannot contradict a recorded decision.


Accepted future Repair orders now have a distinct readonly own-Console intent
view. Their original prerequisite evidence does not dispatch work. Ordinary Repair
AI holds only competing generic visit footprints for unaccepted intent, leaving
other reported groups available. Disposition monitoring for the generated final
Structure leaf reads its own permitted HP, local stores, route and work list.
It retains bounded blocked/cleared or externally restored observations without
creating or rebinding physical execution. A source-declared changed-work callback
uses the existing script sandbox/budget and actual effect result/tick; it alone
chooses any Promise consequence. Prior real assignments still settle normally.
The new native autonomous/override probes are authored and awaiting integration
execution; source review of projection and client tests are separate evidence.
