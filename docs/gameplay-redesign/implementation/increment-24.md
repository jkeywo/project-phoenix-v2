# Increment 24 — Opportunity discovery, adaptive direction and final integration

Approved future plan, 14 October 2026. Start after [increment 23](increment-23.md)'s
verified playable exit. Reuse [increment 22](increment-22.md)'s activity contracts,
HTN and scheduler; do not introduce another execution or physical simulation model.
See [architecture decisions](../architecture/README.md#mission-planning-increments-22-24)
and [PASM design](../../../pasm/spec/design/gameplay-redesign.yaml).

## Playable exit

Bounded GOAP-like search discovers useful combinations from the existing world,
offers selected opportunities through the fiction and integrates them into HTN
missions. Demonstrate appropriate experiences with 2/4/6/8 occupied Stations.

## 24A — Opportunity discovery

Search declared activity operators around authored goals and values such as rescue,
delivery, recovery and protection. Bind candidates to actual entities, capabilities
and needs. Use the same precondition/outcome vocabulary as authored activities.

Example: captured vessel → restore transport capability → embark refugees → deliver
them to an accepting port. Returning the vessel, salvage or escort may be competing
opportunities when their requirements hold. Neither an arbitrary new goal nor an
unsupported physical action is invented by search.

Bound depth, candidate count and deterministic expansions. Reject cycles,
duplicates, unsupported effects and mission-constraint violations. Exhaustion
produces no new offer while committed execution continues. Separate director-owned
setup actions from uncertain player/NPC choices; a hypothetical capture is never a
guaranteed future asset. Hypothetical search states never mutate the real world.

**Exit:** discover return-to-port and evacuation chains from reusable operators,
without a bespoke captured-ship evacuation opportunity template.

## 24B — Admission into the HTN

Candidates expose bound subjects, prerequisites, expected outcomes, costs, workload
and explanations. Revalidate current facts before admission. Assess mission value,
contribution to underused roles, deadline feasibility, resource contention,
repetition, ongoing obligations and a plausible discovery/introduction path.

Admit selected opportunities as optional HTN tasks with proposed dependencies.
Reveal them through normal observations, Comms and objectives; an offer is not
acceptance. Acceptance, refusal and world changes determine continuation or
withdrawal. Keep hidden facts out of player and individual AI knowledge.

Prefer useful existing entities before spawning new actors. Any new encounter uses
authored setup rules and accounts for continuing obligations after completion.

**Exit:** a discovered opportunity joins an existing schedule, survives revalidation
and can be refused without forcing execution or invalidating the main mission.

## 24C — Bounded live pacing

Adapt unannounced activities using crew capacity and observed workload within
authored difficulty/pacing bounds. Low input rate alone is not proof of boredom:
allow deliberate waiting, conversation, preparation and effective delegation.

Preserve announced deadlines, established facts, recovery periods and earned
advantages. Automation reducing personal burden does not automatically earn
replacement chores. Larger crews receive meaningful concurrent responsibilities;
combat opposition remains independently balanced. Include lingering escort,
repair, custody and transport work before admitting another encounter.

**Exit:** changes in participation alter future unannounced scheduling without
rewriting committed events, punishing successful preparation or overloading a
different Station merely to occupy Tactical.

## 24D — Diagnostics, continuation and final integration

Workshop previews search candidates, explanations, rejection reasons and schedules
across crew configurations. Explain discovery, selection, delay and rejection;
retain separate inspectors for plans, situation execution and physical jobs.
Support validation, history, export/reload and disposable Test.

Persist committed plans, bindings, deadlines, director history and required search
continuation through existing snapshots/digests. Stable ordering and expansion
budgets prevent machine speed from deciding outcomes. No separate networking or
System controller is introduced; World owns mission progression and Simulation
connects Gameplay and Session facts through shared contracts.

Complete coarse planetary activities, catalogue coverage, Workshop composition,
shipped-content compatibility and 2/4/6/8-seat audits from the original increment.
Generated and authored opportunities must use the same physical mechanisms.

## Acceptance and completion

- Discover and integrate both vessel-return and evacuation opportunities.
- Refuse an offer; change ownership; destroy its subject; remove reception capacity.
- Reject incompatible commitments and insufficient crew/equipment/cargo capacity.
- Bound search under branching/cyclic libraries; existing missions keep executing.
- Save/resume during discovery, offer, acceptance and physical execution.
- Verify replay and seeded runs agree, without duplicate effects or hidden knowledge.
- Verify automation does not trigger compulsory replacement work or retract rewards.
- Inspect crew-specific schedules, then measure meaningful participation in human
  playtests; assess combat balance separately.

Each sub-increment includes authoring, runtime, player feedback, ordinary AI
integration and continuation. Apply targeted tests during development and the
existing integration gates at the programme boundary. Report measured duration and
workload evidence separately from provisional tuning. Completing the planner does
not waive the original planetary, catalogue, Workshop or manual acceptance gates.
