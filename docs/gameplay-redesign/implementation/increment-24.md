# Increment 24 — Opportunity discovery, adaptive direction and final integration

Approved future plan, 14 October 2026. Full completion follows [increment 23](increment-23.md)'s
verified playable exit. The 15 October [parallel-work contract](parallel-work.md) permits bounded search alongside 23 after 22's operator contracts and actual consumer settle. Reuse [increment 22](increment-22.md)'s activity contracts,
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


## First 24A consumer: finite maintenance opportunities

Adopted 15 October 2026 under the parallel-work contract. The actual Cruiser
already has a finite `hold-structural-parts` to `repair-structural` delivery route;
its structural remedy consumes one `structural_parts` through the existing service
binding and restores up to ten HP. First search discovers useful structural Repair,
or Delivery followed by that Repair, from real positive compartment damage, local
shortage and a funded compatible main store. This is an initial real operator
composition, not the required later captured-vessel return/evacuation exit.

Search declarations identify existing imported primitive tasks and the supported
closed Repair/Delivery precondition and outcome families. Keep their exact typed
actor/System/team/target/store/commodity/quantity bindings and immutable recipe
validation. The source may restrict eligible compartments, goals and budgets; it
cannot invent new physical effects. Useful structural improvement differs from
full restoration. Search estimates a supported outcome; only actual committed
owner facts can establish it later.

Project current finite Damage, Stock/claims/incoming/held capacity, ResourceServices,
Repair/Logistics jobs and access routes through existing owners. Existing automatic
replenishment, an unbound Repair assignment or an incoming batch already serving
the need makes the proposed extra work ineligible. Include ordinary reserve floors,
receiving room and actual route/equipment capability. Known discovery evidence must
come from the issuing ship's permitted current maintenance reports/tools; no private
foreign observation becomes crew information through an offer. Successful ordinary
automation can leave no candidate, without spawning replacement chores.

Use deterministic sorted bindings, at most eight search steps, 256 expansions and
16 candidates. Canonical visited states reject cycles and duplicate resulting
bindings. Scratch hypothetical stock transfers and expected repair results never
call physical commit, reserve real goods, mint a JobId or alter canonical Damage.
Retain bounded discovery inputs, original tick, source/operator bindings, expansion
order/result and refusal explanations needed to derive accepted history. Exhaustion
produces no offer while existing execution continues.

### Same-HTN optional execution join

Keep the primary PlanChoice unchanged. A bounded optional record in the same World
owner refers to frozen source operators and has a closed source-derived execution
identity with a monotonic ordinal. Scope every leaf, definition, Scenario callback
and publication from that identity; arbitrary caller prefixes are unsupported.
Reuse the existing typed primitive expander, Decomposition release/history/request
logic, SituationInstances and ordinary physical/Comms consumers. Static authored
root argument equality remains strict. A discovered graph is derived from its
retained source/bindings, not a pretend authored file or a separately implemented
physical executor.

The first admission is deliberately conservative: the primary and earlier optional
work must have released all their nodes, reached actual terminal physical settlement
and completed required Scenario callbacks. An unreleased/refused future node does
not count as finished. Optional failure/abandonment needs an explicit retained
cause and closed future-node decision; it neither teleports returning teams nor
undoes paid work. This is not concurrent 24B admission or workload-reactive 24C pacing.

Allow a later activity to reuse a historical team/target/route only when its arming
tick is strictly later than the earlier actual terminal settlement/home/release
frontier. RepairAchieved alone cannot free a returning team. Apply the same causal
rule at arming and full historical pairwise validation. Retain the original scoped
JobId/radio origin, evidence and Scenario results permanently within this bounded
run; old facts cannot bind or satisfy the new instance. The global 32-situation
limit still applies. At most eight optional records are retained; capacity exhaustion
refuses new work rather than silently evicting committed history.

### Normal offered communication and atomic acceptance

At most one outstanding offer per actor/source enters the normal existing Comms
conversation path. Its original immutable proposal identity travels with the open,
active dialogue and response callback. A stale response cannot accept a replacement
offer. Offer entry is distinct from an admitted response, acceptance and execution;
no trigger/timer may masquerade as a response on that bound conversation.

An admitted normal Human/ordinary Comms response requests acceptance or refusal of
that exact record. Immediately before admission, revalidate current own permitted
evidence, physical needs, jobs, routes, finite stocks and capacity. Stage the existing
Promise ledger record, optional choice and Situation release as one World commit.
A failed child release cannot leave a newly recorded promise. Duplicate, stale or
withdrawn offers refuse explicitly. A changed need can withdraw an unaccepted offer;
accepted source/choice/deadline/history is never replaced by a new search result.
Only ordinary admitted System owners execute any released instructions.

Snapshots/digests retain proposal/decision/child history on that same World owner.
Preflight frozen source and exact typed bindings, actor provenance, promise open
interval, execution namespaces, complete callback/evidence clocks, global history
capacity and settlement-before-repeat before writes. Do not rerun today's usefulness
test against an accepted historical child. Reject orphan leaves and erased or
rebound decisions. Privileged Workshop/Scenario inspection explains discovery,
rejection, offer, acceptance and actual jobs separately; ordinary Comms exposes
only the offered information. Extend existing accepted-source/history/Preview/Test.

Prove a genuinely funded Delivery→Repair chain, direct Repair alternative, incoming
or already-owned work exclusion, stale/duplicate/currently-unfunded acceptance,
refusal without commitment, interrupted physical work and at least 360 complete
fresh-JSON updates across discovery/offer/acceptance/physical checkpoints. Measure
search cost separately from human workload. Full 23, captured-vessel search, concurrent
admission/pacing, planetary scope, catalogue coverage and human 2/4/6/8 acceptance
remain open until their actual consumers and evidence land.


Generated Promise IDs use the reserved `opportunity.<source>.<ordinal>` namespace.
Authoring validation rejects literal script attempts to record it and runtime
script admission rejects computed attempts before buffering ledger changes;
raising discards the whole call. Only the World collector records such a Promise
from an exact validated proposal tuple in its atomic admission. Existing unrelated
script Promise IDs and authored keep/break consequences retain their semantics.

Ordinary AI must receive accepted future work and its unsatisfied prerequisites
through its own Console before the autonomous Delivery-to-Repair consumer closes.
That informational intent must prevent other ordinary teams from independently
consuming the same planned work, without reserving a Job or resources early.
Human override remains normal admission; inapplicable frozen work needs an explicit
blocked/cancelled/externally-changed result, never retroactive credit. The first
implementation/proof may use Human-issued released work and ordinary AI continuation;
full autonomous execution and override settlement remain an open follow-on slice.

## Initial maintenance consumer checkpoint

The first implementation uses the accepted `opportunity_search` source and real
imported Repair/Delivery primitives, finite own maintenance projection, frozen
optional records, ordinary offered Comms and atomic existing Promise/Situation
admission. The existing Scenario inspection explains the retained observations,
search steps, decisions and release clocks. Workshop edits the accepted World and
operator-library documents through its existing history, Preview and Test path;
this slice does not add a separate visual opportunity form.

`tests/optional_maintenance_continuation.rs` authors an owner-level physical
incident and consumes existing local parts to create a genuine missing-resource
case. It waits for an ordinary System report, answers the exact own-Comms offer,
issues only released delivery/repair through normal Human admission, then delegates
actual partial paid work and compares complete fresh-JSON state/digests for 1200
updates. A separate decline creates no promise or jobs. Frozen source, report clock,
namespace and missing-leaf mutations must refuse without writes. These native tests
are authored and pending the integration owner's run; source review is separate.
The incident is not a weapon/GM or authored-encounter acceptance result.

Targeted executed JavaScript evidence: Scenario activity inspection 5 tests pass;
strict string validation reports 6271 IDs, zero errors/warnings. Native compilation,
actual host execution, compatibility epochs and integrated gates remain with the
integration owner. Future-AI prerequisite intent and Human-override outcomes remain
open before autonomous delivery and repair can be claimed. Continuing modes, pacing,
capture chains, planetary/catalogue coverage and human crew acceptance remain open.


### Offer presentation failure closure

An Offered record must have exactly one immutable pending-open or active dialogue
binding at a capture frontier. Deferral for the existing script budget retains the
same pending open. A validated internal root that finally fails to load/resolve,
refuses, returns invalid shape or produces no dialogue records a closed
source-unavailable outcome at its actual tick; it fabricates no Human decision and
creates no Promise. Its buffered effects are discarded. This retained outcome
unblocks later bounded discovery and explains why this exact offer is unavailable.
A bound on_pick must return the closed Accept/Decline decision: a missing decision
or absent node refuses before effects or response selection, leaving the actual
offer answerable. Historical preflight checks exact source/ref/actor/clocks and
reserved promise absence, including an empty optional-record history.
