# Increment 22 — Authored mission planning, scheduling and execution

Approved future plan, 14 October 2026. This replaces increment 22's earlier
workflow-only scope; it does not claim implementation or change increments 0–21.
Full completion follows increment 21's verified playable exit. The approved 15 October [parallel-work contract](parallel-work.md) permits earlier existing-capability planner slices after contracts are adopted.
See [architecture decisions](../architecture/README.md#mission-planning-increments-22-24)
and [PASM design](../../../pasm/spec/design/gameplay-redesign.yaml).

## Playable exit

One reusable authored mission supports different crews through feasible sequential
or concurrent activity, suitable deadline pressure and persistent consequences.
Procedural opportunity discovery is deferred to [increment 24](increment-24.md).

## 22A — Activity contracts and situation execution

Define activities with typed parameters and entity bindings, preconditions, required
capabilities, supported outcomes, duration estimates/bounds, dependencies, deadlines,
interruption rules and phase-specific attention demand. Declare shared physical
requirements, observations, events, completion evidence, permitted setup effects,
cancellation behaviour and persistent consequences.

Keep definition identity separate from live instance identity. Several independent
situations can bind the same template to different subjects. One record owns each
progression state: scripts and trees must not advance it independently.

HTNs select/decompose work; Scenario state trees execute activity lifecycles and
react to observed outcomes. Explicit Rhai calls retain their sandbox and buffered
effects. Physical work remains ordinary admitted System commands and jobs. A
successful script call is not proof of physical completion. Predicted effects stay
hypothetical until authoritative observations confirm them.

Situation completion is not rollback: surviving vessels, passengers, damage, cargo
and obligations retain their ordinary owners and lifetimes. Mission activities
are not nested physical jobs or a new controller above individual System AI.

**Exit:** two simultaneous instances, explicit script-call results and physical
completion evidence operate without duplicate effects or progression ownership.

## 22B — HTN domains and reusable libraries

Support parameterised tasks, alternative methods and partial-order dependencies.
Libraries import explicitly named definitions with validated references and
compatibility requirements. Author required/optional goals, allowed methods, fixed
story commitments, forbidden outcomes, insertion points and difficulty profiles.

Bind methods to actual entities and capabilities. Introduce new entities only
through declared scenario setup. Preserve existing scripts and scenarios; first
adopt through an opt-in authored mission, not automatic catalogue conversion.

World owns planning and situation progression. Gameplay owns machinery; Session
provides participation/control facts. Simulation connects them through shared
contracts, without sibling dependencies or another authoritative world store.

**Exit:** alternative methods resolve against real capabilities, unsupported
bindings are explained, and existing scenarios continue without planner adoption.

## 22C — Crew-aware temporal scheduling

Keep ship capability/combat effectiveness, human crew capacity and shared physical
resources separate. Derive Station capability bundles from hosted Consoles,
Systems, Station Ratings and delegation. Multiple Consoles operated by one person
share that person's attention budget; vacant seats do not count as extra humans.

Forecast attention in minute buckets but retain exact simulation times for
dependencies/deadlines. Distinguish active decisions, monitoring and unattended
execution by activity phase. Honour shared equipment, location constraints,
existing obligations, duration uncertainty and recovery allowance. Human attention
modelling does not introduce Comms message quotas or simulated bandwidth budgets.

Choose feasible partial-order schedules within authored difficulty bounds. A larger
crew receives overlapping consequential responsibilities and appropriate deadline
pressure; combat opposition is balanced independently. Deadlines can expand or
contract at composition, but once revealed or committed change only through an
in-world cause. Planning reservations forecast contention; ordinary job/resource
mechanisms revalidate and make actual claims at execution.

**Exit:** one mission offers hazard-then-permissions to a small crew and
hazard-and-permissions under tighter scheduling to a larger crew. Document actual
playtest durations; do not treat estimated attention values as measurements.

## 22D — Monitoring, repair and authoring

Observe completion, failure, refusal, capability changes and outside-actor work.
Repair unfinished plans while preserving history, announced commitments and earned
advantages. If no continuation is feasible, report why and use authored failure or
partial-success handling; never fabricate resources or force human actions.

Workshop edits domains/activities and previews dependencies, timelines and Station
workload. Explain rejected methods and infeasible schedules. Keep plan, live
situation and physical-job inspection distinct. Support validation, history,
export/reload and disposable Test through existing authoring infrastructure.

Persist authoritative plans, bindings, execution state, deadlines and continuation
through existing snapshots/digests. Preserve command admission, networking and
knowledge-limited individual System AI. Player-facing reports reveal permitted
information, not the director's hidden world knowledge.

## Acceptance and handoff

- Compare 2/4/6/8 occupied Station schedules, including shared Consoles and Backfill.
- Reject incompatible simultaneous locations and conflicting equipment use.
- Exercise refusal, disconnect, capability loss, delayed work and partial failure.
- Save/resume during parallel activity; replay preserves ordering and outcomes.
- Verify independent instances and script effects cannot complete work twice.
- Verify announced deadlines, information boundaries and shipped scenarios remain valid.

Every sub-increment includes authoring, runtime, human feedback, ordinary AI
integration and continuation. Use targeted owner tests and scripted crew models;
human playtests judge pressure and meaningful participation separately from combat
balance. Apply the established integration gates before pushes. Hand off the
activity vocabulary and authored demonstration to [increment 23](increment-23.md).

Concrete schema encoding, solver implementation and calibrated tuning are resolved
in this increment's implementation specification before runtime changes. The
contracts above are approved; numerical attention/duration estimates remain tuning.
