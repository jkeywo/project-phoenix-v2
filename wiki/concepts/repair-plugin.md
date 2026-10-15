---
title: Repair Runtime
type: concept
tags: [repair, damage, teams, blackboard, ai, external-repair, semantic-actions, feedback]
sources: [crates/phoenix-sim-gameplay/src/jobs/security.rs, crates/phoenix-simulation/src/security/server.rs, crates/phoenix-simulation/src/snapshot/security.rs, gui/job-work-list.js, crates/phoenix-model/src/personnel.rs, crates/phoenix-sim-gameplay/src/personnel.rs, crates/phoenix-simulation/src/console/repair/personnel.rs, crates/phoenix-simulation/src/snapshot/personnel.rs, editor/workshop-personnel.js, gui/workshop-personnel-panel.js, crates/phoenix-sim-gameplay/src/jobs/repair_external_physical.rs, crates/phoenix-sim-gameplay/src/jobs/repair_physical.rs, crates/phoenix-model/src/job.rs, crates/phoenix-sim-gameplay/src/jobs/repair.rs, crates/phoenix-sim-gameplay/src/jobs/repair_workflow.rs, gui/job-work-list.js, crates/phoenix-simulation/src/console/repair/reports.rs, crates/phoenix-simulation/src/console/repair/server.rs, crates/phoenix-simulation/src/console/repair/dispatch.rs, crates/phoenix-simulation/src/console/repair/external_server.rs, crates/phoenix-simulation/src/console/repair/visibility.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-simulation/src/ship/damage_sync.rs, crates/phoenix-sim-gameplay/src/ship/components.rs, crates/phoenix-sim-gameplay/src/modifiers/repair_teams.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-simulation/src/command_admission/mod.rs, gui/components/ph-repair-teams.js, gui/stations/engineering-actions.js, gui/action-map.js]
updated: 2026-10-15
---

# Repair Runtime

`RepairPlugin` owns the server adapter for internal repair teams, Repair Backfill, blackboard publication, and external-repair registration. The Bevy-free `jobs/repair.rs` owns canonical assignments. `modifiers/repair_teams.rs` retains the physical repair phase executor and reexports the public RepairTeams facade.

## Internal repairs

The Repair console and `operate_repair_ai` emit the same admitted payloads. The dispatch adapters in `crates/phoenix-simulation/src/console/repair/dispatch.rs` apply team dispatch and priority changes. `tick_repair_teams` advances each ship's own teams and advances actual structural/component remedies against its own `ShipDamage`.
Legacy unequipped fixtures retain the independent `EntitySystemHull` path.

An on-site team diagnoses its compartment and performs one bounded structural or component task. Distinct teams can repair different components concurrently; each exact task is exclusive. `SetRepairTargetPriority` chooses inspected work in a compartment with an on-site team. Team slots carry `SystemId` and display text so the client never reconstructs target identity from an obsolete console enum.

`repair.dispatch-team` preserves the exact authored Repair owner, selected integer
team slot and Station/Console/Core target, while `repair.prioritise-system` preserves that
owner plus the named damaged `SystemId`. Their
parameter-free bindings choose only an idle visible team/target or a damaged
row included in the owner's exact `priority_targets` projection. That list is
derived from the same sweep-group candidate predicate that applies a named
priority, then visibility-filtered, so neither the adapter nor the component
guesses reachability from partial hull rows. The owning dispatch router
returns `Applied` only for an existing, non-committed slot with a resolvable
target; named priority returns it only when an on-site sweep accepts the pin.
All other correlated outcomes are `Refused`, with no optimistic team movement.

`RepairRequestQueue` records outstanding requests for human and AI Repair
operators. It deduplicates by typed `RepairGroup` (home Station, detached Console,
or Core), reducing current readings to the worst known tier and largest deficit. `reports.rs`
preserves separate contributor observations, accepts improving readings from the
same source and rejects older samples.
Recovery watermarks prevent strictly older reports from reopening a resolved group;
equal-tick damage remains admissible because diagnosis precedes damage sampling.

Physical recovery arrives through ordinary System reports and on-site team
diagnoses, including rooms inspected before completing work. No remote truth
poll prunes physical requests. `prune_repair_request_queue` retains the legacy
operational-group diagnostic only for the legacy HP owner.
That observation does not resolve other outstanding damage in the group.
`RepairHumanAlerted` remains the separate popup latch. Queue observations and
recovery watermarks participate in capture, restore and deterministic digest.

## Jobs and workflows

The optional `[personnel]` roster binds named officers to existing finite Repair
teams. `phoenix-model/src/personnel.rs` declares closed competencies and custody;
Gameplay `personnel.rs` validates postings against the same Repair jobs and team
navigation. The admitted `SetDutyOfficerPosting` consumer requires the actual
Repair owner and an idle team at the officer's authored home. Posting and release
cannot move an officer to or from an away team. Attached custody follows that
team's existing cursor; there is no second movement timer.

Mechanical expertise scales only on-site internal remedy progress. Travel,
parts reservation/consumption and paid tool intervals retain their existing
owners. External repair and refitting retain custody without this bonus. The
optional `personnel_cruiser.toml` demonstrates a provisional 1.25 multiplier.
Repair's permitted publication carries the roster and available handoffs; its
existing widget uses exact owned Human authority for controls. Workshop's
`workshop-personnel.js` and structured panel use accepted-source history,
explicit inherited roster materialisation, and existing Preview/Test. Snapshot
`personnel.rs` validates custody against the candidate's own saved Repair jobs
before writes and the digest folds custody once. This local Repair consumer
does not establish Medical, Security or external personnel acceptance.

`TeamJobs<RepairActivity>` is the single per-ship owner of assignments and team
commitments. Each row retains a ship-scoped JobId, issuing System, activity,
private TreeRuntime, executor clock and bounded last outcome. Redirected local
work keeps its committed team's identity. External retargeting creates a fresh
activation; a repeated identical dispatch leaves the job intact. Internal recall
keeps its team until the existing return trip finishes. External release remains
immediate. TeamSlot readouts are derived from these jobs and are not saved.

`repair.workflow` reuses the state-tree compiler, evaluator and editor with
closed repair facts and stage-effect channels. Final writes gate one physical
phase per executor step; arrival, sweep handoff and queued return keep their old
boundaries. Workflows execute admitted human and AI assignments alike. The ordinary
repair selector remains the System controller. A failed/premature terminal workflow
surfaces interruption without releasing its team; ordinary recall/release provides
cleanup. Workshop edits the same typed tree format through accepted-source history.
The shipped repair workflow fragment reproduces current behavior.

Canonical jobs and private continuation are captured once in RepairState.jobs,
validated against the authored owner/crew/workflow on restore, and folded in the
internal-repair namespace. Snapshot 46/rules 0.18 explicitly reject older state.
Permitted Repair blackboards expose assignment/outcome rows to the shared read-only
work list. It does not expose private memory, concealed damage or extra controls.

Physical stages reserve real Infrastructure parts, consume atomically on
completion and release unused claims on recall or retarget. Recipe-defined manual
work remains possible when powered tools fail; the actual Repair operating factor
scales paid acceleration. Full restoration is preferred, with indefinite reduced
patches only when full restoration cannot be funded.

## Physical layout and access

`ship/interior.rs` validates authored decks, compartment geometry, System homes
and explicit connections, then compiles stable minimum-cost routes. System
placement is independent of Console ownership. `jobs/repair_navigation.rs`
adapts the retained phase executor: teams must physically arrive before spending
repair work or exposing on-site diagnosis. Cross-room sweeps travel, redirects
finish committed crossings and blocked routes wait or replan. Canonical positions
live once in `RepairJobState.navigation`, including external commitments.

`ship/access.rs` applies ordinary admitted access-gate controls and scoped
per-System trees. Protected seal requests block entry while existing crossings
clear; actuator intent and actual boundary state remain distinct. Offline
actuators preserve their transition. Immutable `InteriorLayout` geometry is
installed independently of team count and projected through permitted Repair
publications. The shared interior map retains deck selection through existing
Console working-view admission. Co-hosting and read-only feeds grant no controls.

## External repairs

`crates/phoenix-simulation/src/console/repair/external_server.rs` owns dispatch to a nearby ally or structure. It shares the ship's team pool, consumes ordinary admitted commands, and applies the visited target's physical structural/component remedies using the source ship's actual parts and tools. Unequipped targets retain the existing condition-track adjustment. Backfill uses the same command seam.

`repair.external-dispatch` is the shared dispatch/recall semantic action on
Repair and Engineering variants. Dispatch feedback completes after the live
lock, range and free-team verdict; explicit recall is an idempotent applied
assignment. The shared job owner holds the exact named team until release. External recall retains its existing immediate release; visited physical work uses the target remedy's work time and manual fraction. Restoring a target retains the ordinary aboard commitment until recall or loss of range.
ExternalRepairDispatch carries authored terms and transient refusal, with no second crew claim. External physical stages remain in the existing Job, cache bounded recipe requirements for deletion-frontier cleanup, and reserve only actual source stock. Active task claims are reconstructed from internal/external jobs; pending local requests wait without taking an existing foreign claim. A correlated external command on a
hull without the optional dispatch capability terminates once as `Refused` rather
than entering a queue with no owner.

## Publication and visibility

`publish_repair_blackboard` writes the per-ship Repair blackboard. `crates/phoenix-simulation/src/console/repair/visibility.rs` projects recipient-visible damage before the state reaches a player. `repair_state_broadcaster` sends LocalShip state at 10 Hz to the holder of the authored `repair` system.

## Coordination receive path

`process_coordination_lag` retains the generic concerns: delay, live Station
control, Popup/Consume/Suppress selection, session-token resolution, and
per-recipient Repair visibility. It emits `DeliveredCoordination` with an `Ai`
or `HumanRouted`/`HumanPopup` delivery outcome while preserving the producer-owned
presentation envelope.

`receive_repair_coordination` validates the ship's authored Repair address and
owns behavior after that seam. Both AI and `HumanRouted` deliveries merge into
`RepairRequestQueue`; the peer-identical routing outcome owns this update rather
than the presenting peer's popup. Human popups retain their existing severity/latch
rules and ordered flush. Human-supplied observation metadata is stripped at enqueue,
so a client cannot impersonate an equipment report or assert a future sample.

Repair publication filters contributor subject and source-System metadata through
the same detail permissions as the queue row. An explicit Repair feed receives the
producer Console's projection, without granting repair-team control.

## Related

- [Recorded Observations](./recorded-observations.md)

- [Damage and Repair Intent](./damage-and-repair-intent.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Broadcaster Seam](./broadcaster-seam.md)

## Shared Security commitments

Security now uses the same `TeamJobs` commitment and JobId mint through Gameplay
`jobs/security.rs`. Its existing finite muster, action vocabulary, dispatch/recall,
paid work and authored consequences remain in `security/teams.rs` and Simulation
`security/server.rs`. A completed work result stays on the returning activity;
only actual arrival releases the team and records the terminal Job outcome. The
existing Tactical panel renders these permitted Jobs with the shared work-list
renderer. That projection grants no control.

Snapshots retain complete Security Jobs and used mint history, even once every
team returns. `snapshot/security.rs` validates accepted owner/count, IDs, finite
phases and result/return joins before global or per-entity writes. The Security
digest includes these same Jobs and canonical phase floats. Local compartment
routing and Medical remain subsequent consumers; external Security retains its
existing crossing model pending the actual craft/custody join.
