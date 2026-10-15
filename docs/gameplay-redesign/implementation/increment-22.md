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

### 22A first consumers: actual Repair and local delivery

Contract preparation, 15 October 2026. This adopts the early existing-capability
slice permitted by [parallel work](parallel-work.md); it starts no runtime and
does not waive 22's full dependency on 21. Keep the existing HTN/Scenario-tree/World
ownership above and 24's hypothetical opportunity search unchanged.

The first opt-in authored demonstration binds two concurrent situation instances
to disjoint actual work: one named component/structure repair and one finite
main-to-local delivery on accepted existing equipment. Use ordinary Human/AI
commands and actual claims, routes, parts, power and receiving capacity. Do not
add a generic activity executor, physical job manager or another team/inventory.

The existing evidence boundaries are concrete:

| Existing owner | Usable evidence and limitation |
|---|---|
| Gameplay `jobs::repair::{RepairTeams,RepairJobState}` and `jobs/repair_physical.rs` | Ship/System-scoped `JobId` and the actual physical work stage bind work. A successful remedy stages the target change, consumes the recipe claim and advances its stage. `JobOutcome` becomes terminal separately after actual return; a completed team row is replaced on reassignment. A patch or useful repair stage is not proof that all defects are gone. |
| Gameplay `resources::logistics::{LogisticsState,DeliveryBatch}` | `LogisticsTickResult.completed` contains every batch outcome only after the exact receiving transaction accepts the whole actual batch. Return-to-source completion is Cancelled/returned, not delivery success. Saved `last_outcome` retains one row and cannot preserve several completions for a delayed consumer. |
| Simulation `console/repair/server.rs` and `resources/logistics_server.rs` | These join real damage, Stock, navigation and power. Publish supported outcomes only after their physical transaction commits; never from a proposed stage, reservation, command feedback or failed staged candidate. Delivery currently checks the result for failure then commits staged owners; 22A must retain its supported outcomes at that commit boundary. |
| World `world::content::WorldEvent` and Simulation `world/server/{state,triggers,registration}.rs` | `pending_world_events` already crosses the next collection boundary and is saved. `WorldEventBuffer` is replaced by the current pass; it is not a durable journal. Existing Objective instances own objective credit/last-known views, not repair/delivery execution or a reusable situation lifecycle. |

These are extension sites, not claims that an activity adapter or durable physical
completion journal already exists. Gameplay and World must not import each other.
Model supplies the narrow shared outcome vocabulary; Simulation connects the
ordinary physical owner to existing World event collection. Use that existing
pending-event lifecycle and its snapshot codec, not a parallel universal bus.

#### Binding and supported physical outcomes

Before issuing work, bind the actual actor, owning System, supported work family,
physical target or source/destination endpoint and requested quantity/outcome.
When normal admission accepts an actual job, retain its scoped JobId in that
activity's binding. Job numbers alone are not globally unique. Recipe stages need
the actual stage identity as well. The adapter must obtain this binding from the
accepted owner result, not infer it from a later team slot or matching display text.
A submitted request and `ActionFeedback::Applied` establish at most acceptance;
neither establishes work achieved or goods received.

Closed owner-produced facts distinguish at least:

- **Repair work achieved:** the actual target, remedy/stage and supported committed
  result. A structural increment, patch and full component remedy remain different
  outcomes. Use the actual applied result, not total ship HP, nominal recipe effects
  or current private state polled later. Subsequent damage does not rewrite this
  historical result; an activity requiring a current condition states that separate
  predicate explicitly.
- **Repair assignment settled:** the same job's actual Completed, Cancelled or
  Failed outcome after its supported return/lifecycle boundary. Work-achieved and
  team-available are separate dependencies. Recall does not undo a paid remedy,
  teleport a returning team or release another job's resources.
- **Delivery received or returned:** the exact committed batch, destination,
  commodity and quantity under its accepted endpoint bindings. Issuing, loading,
  arrival, reserved incoming space and unrelated stock growth are not receipt.
  A return retains its actual quantity/location and cannot satisfy the destination.
- **Supported refusal/interruption:** admission refusal, paused/access/parts/power/
  receiving blockage, cancellation and failure retain their real distinctions.
  Paused work is not a terminal failure; do not invent death, lost cargo or a
  successful cancellation where the current physical owner supplies no such fact.

Facts carry the original logical tick and scoped physical identity, with a closed
family/stage discriminator sufficient to reject duplicate credit. Actual bodies,
conditions, parts, payloads and job progress remain solely at their current owners.
World stores only the situation binding, received evidence and progression. An
adapter is shared across situations; no mission-specific callback mutates machinery.

#### Retention, ordering and exactly-once credit

These retention guarantees apply to admitted, explicitly bound activities. Ordinary
ship repair/delivery without a planner keeps operating and does not create an
unacknowledgeable planner backlog or become unavailable merely because no planner
is installed. Register a binding before its physical work can produce credited
outcomes; unbound work remains with its ordinary owner and existing scenario rules.

The physical commit and enqueue into the existing saved pending World event owner
must form one observable fixed-step boundary. Capture cannot expose changed stock/
condition while omitting its unconsumed completion, or queue an outcome for an
uncommitted transaction. Collection must fold matching evidence into the World
instance before the transient buffer is replaced. If scheduling, consumer absence
or a snapshot barrier can postpone that fold, retain the same unconsumed typed
outcome in the existing pending owner until acknowledgement; do not rely on Bevy
reader lifetime, `last_outcome`, terminal team rows or a one-tick polling window.

World's situation record retains the consumed evidence keys and terminal
transition needed to suppress repeat credit and buffered effects after restore.
No unbounded global event archive is required: retain pending facts until their
bound consumer has acknowledged them, and keep the bounded per-instance evidence/
terminal history required by declared outcomes and persistent consequences. Closed
limits apply before new activity work is admitted; exhaustion pauses/refuses new
work without dropping an already committed physical fact. Do not silently evict
unconsumed completions. Save pending facts and consumption frontiers together.

A physical outcome is credited only to its explicit instance/job binding. The first
demonstration uses disjoint bindings; any later intentional shared satisfaction
must be declared, rather than allowing two similar targets or stock polls to award
cross-instance credit. Script-call success/failure is a separate result of its
buffered effects; script and tree cannot both own the same progression transition.
Existing authored scenarios remain opt-in and retain their original trigger credit.

#### First implementation sequence and proof

The first implementation uses a closed shared physical-fact vocabulary and
explicitly armed disjoint request bindings. Repair acceptance returns its actual
JobId from the dispatch operation; delivery retains the JobId already returned
by its dispatch. Matching display strings, later team slots and action feedback
are not binding sources. Arming reserves two named evidence slots for a Repair
activity (one declared achieved result and one settlement), or one terminal
Receive/Return slot for a delivery. Unrelated later sweep stages do not acquire
planner obligations. Ordinary unbound work has no planner retention requirement.

The existing pending World event owner retains bound committed facts until
acknowledgement. Collection sends these only to the bound situation consumer,
without repeatedly replaying them through legacy scripts or replacing existing
event drain semantics. Situation definitions lower their supported conditions
and explicit script-call effects through the existing World evaluator and staged
script-effect application boundary. Each live instance retains the activation
identity/result required for exactly-once application and refusal/re-entry.
The definition and live instance identities remain separate. This lowering
implements the first Scenario lifecycle; it does not deliver 22B's actual HTN.

Repair bindings distinguish the exact declared admitted request (including an
ordinary group request) from the named required achievement. The named target
must belong to that request's actual supported work scope. The ordinary AI
keeps choosing its group through the existing controller; assignment or useful
work on another component does not satisfy the named goal. Actor references
resolve authored names or stable `slot:<authored ship-slot id>` identities,
never a host's LocalShip view. The first base-world definitions support bounded
acyclic Scenario state chains; layer-owned activation/unload refuses explicitly
until its lifecycle is implemented. Multiple independent base-world instances
may bind only named actors declared in the base World or actual authored launch
slots. Declared but unresolved actors arm only when physically present. Dynamic
script/GM and layer-only actor bindings refuse with an actionable source error;
their immutable recipe/content provenance join remains open22 work. The existing
base template/overrides and launch bootstrap validate retained historical evidence
after actor removal without copying a second spawn owner into World. Instances
can reuse a definition.

The existing `GameStartEntityUuid` bootstrap record adds its actual
`FleetPlacement.authored_slot_id`; the record still owns the same authored row
and minted UUID. Validate unique slots/UUIDs and compatible authored rows before
application. Removed-slot history resolves through this existing bootstrap
relation and `FrozenShipSlots` hull choice, reusing the normal player-hull
override merge. It does not infer identity from LocalShip or copy a second recipe
owner. Accepted bindings/evidence can outlive a removed physical actor without
inventing a returned/Failed result that the actual owner never emitted.

1. Add the narrow shared fact schema and pure matching/deduplication rules, then
   wire the actual repair work/settlement and exact delivery Receive boundaries.
   Preserve ordinary dispatch, operator authority and one controller per System.
2. Execute the two authored instances through Scenario lifecycles with explicit
   result bindings, cancellation/failure handling and no HTN resource claims.
   Only then decompose these real operators in 22B and schedule them in 22C.
3. Expose accepted source edits, evidence/instance/job inspection and refused
   bindings through existing Workshop history/includes/Preview/Test. Forecast
   duration and attention remain labelled estimates until actual measurements.

Prove simultaneous completions, multiple delivery results in one tick, team
reassignment before the next World read, useful work followed by recall, receiving
blockage/return, and failed staged payment with no completion. Exercise consumer
ordering/delay and fresh JSON restore before commit, with a pending fact, after
consumption and after terminal effects. Compare at least 360 complete captures/
digests and different-LocalShip peers; no replayed reward, missed result, duplicate
resource/crew owner or physical rollback. Full-candidate preflight validates actual
actor/System/job/stage/endpoint bindings, finite result quantities, nonfuture ticks,
unique evidence keys and coherent pending/consumed/terminal frontiers before writes.
The actual adapters, authoring and ordinary AI/human evidence are required for 22A;
a schema or synthetic event-only test cannot establish the consumer exit.

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

### 22B first consumer: imported maintenance domain

Implementation contract, 15 October 2026. This is the early existing-capability
22B slice, not a claim that 22A, 22C/D, broader personnel/facilities or full 22 have
completed. The first actual imported domain decomposes the supported internal
Repair and main-to-local delivery operators above. Existing worlds without an
explicit planning declaration keep their present execution.

World source declares `planning.import` rows with an asset `path`, namespace `as`
and exact exported `names`, plus a named root task and typed arguments. A domain
library is closed versioned TOML: format 1, stable library identity, explicit
exports, required supported operator families, typed task parameters, alternative
methods and primitive activity operators. Nested imports use the same explicit
shape. Imported definitions are identified by canonical asset path and definition
id; a namespace is an authored reference, not a new identity. Resolve through the
existing overlay-aware WorldReader and LoadedWorld/LedgerPlan boundary. The ledger
records every actually read library's source before content freeze. Missing source,
absolute/traversing paths, unsupported format/capability requirements, duplicate
exports/namespaces, ambiguous references, import/definition cycles, unreachable
exports and incompatible parameter bindings refuse activation and Preview/Test.
An exported root may use its library's validated private definition closure;
private definitions do not become selectable imports in a caller.

The first closed parameter kinds are actor, System, team, Repair request, physical
Repair target, Repair goal, store, commodity and quantity. A call argument is an
explicit typed literal or a named parameter reference; no string interpolation,
implicit name lookup or arbitrary Rhai expression supplies a physical binding.
Team/quantity obey the existing command frontiers. Actor scope remains the actual
base-world names/authored launch slots accepted by 22A. The first maintenance plan
binds one actual actor; its Repair and delivery leaves share that binding. Existing
independent 22A situations remain independent. Multi-actor method composition is
still open22B work rather than an implicit Cartesian search.

For each offered hull, content preparation proves at least one compatible complete
decomposition through the shared immutable support checks; alternatives need not
all fit every offered hull. Runtime waits for that actor's actual accepted
equipment, then chooses the first supported authored alternative deterministically.
Explanation retains why earlier methods refused. The accepted actor relation and
choice tick bind that decision, never the host's LocalShip. A missing future actor
waits; later refit/capability loss does not silently replace the saved method.
Capability support is not a resource claim,
a successful command or proof of a current/finished physical result.

A method contains named task calls and explicit predecessor/evidence edges. The
expanded graph must be acyclic. Dependencies use the existing Accepted, Achieved,
Settled(outcome), Received or Returned facts of the addressed leaf instance.
Repair achievement never implies a returned/free team, and cancelled return never
satisfies delivery receipt. Finite technical limits apply before selecting a new
plan: at most 32 imported libraries, import depth 8, 256 validated definitions/expanded
nodes and 32 physical leaves, sharing the existing situation capacity with explicit
22A instances. Exhaustion refuses the new plan without evicting existing work,
evidence or history. No 22C attention/deadline feasibility is implied by these bounds.

WorldContentRuntime owns the selected root arguments, selected method identities
and original choice tick, accepted actor UUID and bounded released-leaf id to
original tick frontier. Release and SituationInstances.arm commit together at the
existing World boundary. Released history requires its exact corresponding
instance even after actor loss; a genuinely unreleased future leaf remains absent.
This is mission release identity, never a copy of physical job progress. Existing
bootstrap/slot provenance validates the accepted actor relation. Immutable domain
definitions compile from frozen source;
the selected graph is derived from those identities and bindings. Stable plan/node
paths generate leaf situation ids. Existing SituationInstances remain the sole
owner of accepted jobs, evidence, Scenario effects and execution frontiers; a plan
is not another physical job, stock/team map or progression controller. A ready leaf
arms through the existing actor/accepted-request bridge only when predecessor
evidence is acknowledged. Earlier unrelated ordinary work is neither rolled back
nor retrospectively credited to a newly unlocked leaf.

Expose only ready requests for the actual owning ship/System through its ordinary
permitted Repair or Logistics publication. These are derived typed request views,
not a new message bus or authority grant. A request is a mission instruction,
not evidence of hidden damage, Stock, a remote actor or current scene geometry.
Repair still chooses/diagnoses work through its normal permitted reports and
actual on-site tools; Logistics retains its normal permitted endpoint/demand
knowledge. Receiving the request never expands either knowledge boundary.
A Human may use the existing ordinary commands voluntarily. The same existing per-System AI reads its exact permitted
view and emits the same admitted Dispatch command when its real machinery permits;
the HTN itself never dispatches a job, writes resources or overrides delegation.
Other unbound Human/AI work remains independent. Current stock/access/power/team
availability is still revalidated by the ordinary physical transaction. A blocked
request waits/refuses honestly; hypothetical method effects do not complete it.

Snapshot/digest include the saved choice and existing situation/pending frontiers.
Candidate-only validation recompiles the exact imported closure, checks chosen
method/argument/leaf identities and causal choice/arming/dependency ticks before
any writes. Historical completed leaves keep their accepted immutable provenance
after actor removal or team reuse. An unresolved future actor is not silently
re-armed, and restore does not choose a new method from today's circumstances.

The opt-in source demonstration must use an imported compound task with alternative
methods and real unordered/ordered Repair and delivery leaves, accepted backend
validation and existing Workshop source/history/Test. Inspect chosen methods,
unsupported reasons, dependencies and actual job/evidence separately. Prove actual
normal Human/AI commands, partial-order release, refusal/no fabricated completion,
no duplicate effects and 360 complete JSON capture/digest continuation. Scheduling,
attention calibration, dynamic opportunity search and automatic plan repair remain
22C/D/24 work; no complete22B exit follows from a schema-only decomposition test.

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

### 22C first consumer: dated maintenance forecasts

Adopted finite encoding, 15 October 2026. This opt-in extension schedules the
actual imported internal Repair/local-delivery leaves above. It does not finish
22C's hazard/permission demonstration, calibrate human attention, or implement
22D automatic repair. Existing plans without `planning.temporal` are unchanged.

An optional primitive `forecast` declares integer `min_secs`, `max_secs` and
`recovery_secs`, plus ordered nonoverlapping `phase` rows. Each row has
`kind = active | monitor`, `offset_secs`, positive `duration_secs` and positive
`attention_permille` at most 1000. Undeclared time is unattended. Phase intervals
must fit inside `max_secs`; minimum is positive and no greater than maximum.
Recovery follows the maximum interval, occupies the same forecast physical
resource, and has no invented human work. Estimates are authored hypotheses,
never physical progress, operator skill multipliers or measured playtest results.

`planning.temporal.profile` supplies distinct ascending `min_operators` thresholds
and positive `deadline_secs`. The first threshold is zero; selection uses the
highest applicable threshold. Horizon/deadline and each complete duration are
at most 86400 seconds, with at most 16 phases per primitive and 16 profiles.
All selected leaves require a forecast when temporal scheduling is enabled.
The actual positive fixed timestep converts seconds to integer ticks by rounding
up, with checked arithmetic. The one accepted absolute deadline is original choice
tick plus that profile's deadline. It never moves after choice. Deadline pressure
changes only in this opt-in policy; combat effectiveness remains independent.

At original choice, Simulation projects only the bound actor's actual System
control entries, ordinary authority host and replicated Station Rating. A Human
operator additionally requires that exact Station in the actual replicated
participation set below and a current non-Backfill rating. A vacant/default-Human seat is not
capacity; cohosted Consoles share their one Station operator budget. AI-operated
owners have no invented human attention demand. Missing/Offline controls or a
Human source without supported occupied provenance withhold the new schedule.
Initially vacant seats joined later enter that set only through the same admitted
crew transition below; local Sessions are never a cross-peer World fallback.
No participant token or new physical crew owner is introduced.

World computes a deterministic bounded schedule in dependency/topological and
stable node-id order. It uses conservative maximum duration plus recovery,
requires every forecast predecessor to finish before its dependent begins, and
finds the earliest placement whose exact phase overlaps stay within 1000 per
actual operator. Minute buckets report attention area; they do not replace the
continuous overlap test. The actual actor/Repair owner/team is an exclusive
forecast resource; first delivery scheduling conservatively treats its actual
Logistics owner as exclusive. These are forecasts, never reservations or claims.
The first accepted schedule must fit wholly within its authored deadline; an
infeasible graph explains refusal without releasing or dispatching physical work.

The same `PlanChoice` retains original timestep, bounded owner/source/Station/
rating inputs, selected profile, absolute deadline and exact per-node start/end
ticks. No new timer, job map, scheduler resource or callback queue is added.
Release still requires real acknowledged predecessor evidence and additionally
waits for the forecast earliest start. Early ordinary unbound work stays ordinary;
late evidence does not become success merely because its forecast elapsed. Late
actual work retains its valid history and pending requests; inspection reports
overdue from current tick versus the committed deadline. No automatic cancellation,
failure, reward or deadline extension is inferred from the forecast.

Full-candidate validation recomputes timing and minute summaries using the saved
original inputs and immutable source. Validate exact actor/owner identity,
authored Station/rating vocabulary, bounded control mode and consistent historical
time/frontier shape. It cannot prove a past disconnection or rating by looking at
today's Sessions/control source, and does not pretend to reconstruct that history.
Accepted historical inputs are persisted facts like the original method choice;
later disconnect, rating, delegation or capability changes never cause fresh
method/profile selection or erase legitimate history. Current ordinary admission
and equipment still decide whether physical work can run. Save and fold these
inputs and resulting commitment once on the existing World owner.

Proof includes independent versus cohosted operators, vacant/AI/Offline controls,
continuous overload inside one minute, uncertainty/recovery/deadline refusal,
exact real-evidence release and delayed completion, forged schedule no-write and
JSON continuation under changed current ratings. Existing accepted-source editing,
privileged Scenario inspection and ordinary Repair/Logistics requests expose the
same forecast without gameplay knowledge or authority grants. The quiet imported
maintenance demonstration supplies the first real consumer; its duration and
attention values remain provisional until actual playtest measurements.

The later communication primitive requires an explicit adopted join to actual
ReportStatus read and returned lineage before any authored Scenario permission
effect may follow. A status report is not permission. Do not create hypothetical
permission/hazard facts or an invented communication JobId to complete this exit.

#### Accepted refusal and actual participation prerequisite

The same choice's closed temporal decision distinguishes `Committed` from
`Refused`. Both retain their complete original bounded input; a refusal retains
its deterministic bounded explanation instead of releasing any leaf. The
immutable method choice still exists, so a legitimate infeasible mission capture
does not require weakening missing-history validation. Full-candidate validation
recomputes that result from original inputs/source. Missing/Offline/unoccupied
owners are explicit input alternatives, not missing keys or invented humans.
22D must add explicit cause-based reconsideration while preserving announced
commitments and earned history; no silent method retry is introduced here.

Actual participation extends the existing admitted crew transition. One
`ShipCrewPresence` Station set records supported occupied seats, alongside existing
ratings/sources; it is not a Session/player/token copy. Bootstrap seeds it only
from actual connected non-AFK/non-spectator launch participation, or the identical
frozen crew picture on every fleet peer, excluding Backfill defaults. Normal
claim/reconnect/ready/rating/AFK/release/disconnect handling projects that exact
post-handler holder availability at the existing local Session adapter. The same
`AssignStationRating` envelope carries optional `occupied`; its existing admitted
applier commits presence and rating together on all peers. A rating-only legacy,
GM or NPC command omits the field and cannot create participation. Voluntary
Backfill can retain actual presence, but contributes no Human budget until actual
source/rating supports Human operation. No new network message or admission route
is added. Solo uses the same accepted local result boundary, without a fake fleet.
Availability follows the established in-play operative holder, not the Lobby
`ready` flag. Ordinary reconnect restores the previous controls while that flag
is false, and in-play unready does not relinquish authority; both retain actual
participation. A disconnected, AFK, spectator or superseded token cannot create
presence by sending a rating change. Historical Human forecast bindings must
use that owner's actual controlling Console's authored home or human-seeking
host candidates, as well as a supported Station/rating; another valid Station
is not interchangeable. Empty presence is still a real bootstrap owner and
folds distinctly from an absent owner, because absence withholds plan choice.

`MeshCrewState` captures/restores the bounded sorted Station set with existing
ratings/sources. Validate actual authored Station membership, uniqueness and
required owner presence before candidate application; fold it once with crew
authority and declare the component at the normal state census. Standalone resume
keeps its existing policy of adopting newly connected local crew; mesh continuation
restores exact replicated authority. Historical plan inputs remain unchanged by
either policy. Plan choice runs only after this normal bootstrap/admitted crew
projection is installed, so transient default-Human components do not permanently
refuse an ordinary launch. A missing bootstrap owner waits before choosing.

## 22D — Monitoring, repair and authoring

The first accepted-source editor uses the existing World situation arrays and
chronological Workshop history. Its equipment choices come from the real resolved
actor templates and every offered launch hull, including normal World overrides.
The backend shares the ordinary pure Repair visit resolver and immutable binding
support check with runtime arming. Missing teams, goals/remedies outside the visit,
or a delivery outside the approved main-to-local route refuse before Preview/Test;
no resource levels or live damage are inspected to validate authored support.
The existing privileged ScenarioState collector projects exact instance/job,
pending versus acknowledged evidence, original causal ticks and Applied/Refused
activation frontiers. This grants no gameplay authority or foreign phone feed.

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

The first imported maintenance expansion retains 22A's disjoint binding rule
across all selected leaves, including historical bindings. An ordering edge does
not grant sequential reuse of the same bound Repair team/target or delivery
request. Such a domain is refused with the ordinary binding-conflict error;
reusable lifecycle/operator identities require a later explicitly designed
extension rather than deleting evidence to make room.

Body-only snapshot application and GM removal inverse leave the World owner in
place. They must refuse a physical Repair/Logistics job rewind beneath an
already accepted World binding. A removed bound actor requires a full-roster
restore with its matching World frontier; an ordinary unbound inverse is
unchanged. Whole-snapshot rollback validates its own candidate World rather than
requiring equality with today's accepted jobs. An explicit candidate spawn
origin must resolve exactly and retain the accepted base/slot equipment recipe;
a failed replacement cannot borrow bootstrap capability.


The same PlanChoice retains the original ordered immutable-capability method
refusal explanations, bounded by the existing 256-node search budget and the
existing 512-character diagnostic limit per reason. This is accepted choice
metadata, never a current damage/stock/resource claim. Full-candidate preflight
reconstructs deterministic selection against the accepted complete base/slot
recipe and checks both the chosen identities and original explanations. Privileged
Scenario inspection derives the graph and shows original choice/release/evidence
separately; it cannot choose a replacement or grant gameplay feeds.


The first temporal inspection consumer keeps the existing privileged Scenario
surface read-only. It shows original profile/deadline/decision, exact leaf tick
intervals and per-Station minute attention area as lossless decimal nanoseconds.
A current presentation tick marks overdue; it does not edit deadlines or imply
physical failure. Nested World profiles and primitive forecast phases use the
existing validated imported-source history/export transaction. The opt-in
`maintenance-temporal-world.toml` consumes real maintenance primitives; its
estimates remain explicitly provisional. Historical validation proves authored
host/rating vocabulary and saved bounded inputs, not a past Console depth from
the host Station rating: visiting and GM overrides make that inference invalid.


### First 22D continuation: explicit crew/control refusal recovery

Only a retained temporal refusal with no released leaves may be reconsidered
when the actual bound actor's forecast participation/control input changes.
Keep the original method graph, actor, chosen difficulty profile and absolute
announced deadline. Schedule that same graph from the dated cause tick inside
that existing deadline. A larger crew does not rewrite a revealed commitment.

Append at most eight dated decisions to the same PlanChoice. Each records the
closed crew/control-change cause with its exact changed owner IDs and the new
bounded original input/decision. No old decision is evicted, no resources or
physical outcomes are forecast into existence, and exhaustion keeps the existing
refusal. Unchanged input never retries. Once any decision commits or any leaf is
released, this first continuation stops reconsidering; actual execution and
history remain with their normal owners. This is the first refusal-recovery
consumer, not the full capability-loss or outside-actor 22D exit.

Preflight recomputes every decision from immutable source and retained dated
inputs, checks cause differences, increasing clocks, the original deadline and
profile, and no released work before the effective commitment. Inspect original
and revised decisions separately. Simulation supplies only the same real
participation/control projection already used at first choice; no local Session
fallback, hypothetical job, method swap or fresh mission clock is introduced.


### Communication activity joins actual shared-channel execution

Adopted with increment16's existing ReportStatus consumer. Generalise the existing
activity execution reference to a closed identity: actual actor/System plus either
physical JobId or actual communication origin ID, recipient UUID and emission
tick. Keep ActivityJob as the physical helper. ActivityFact and SituationInstance
hold this one typed reference; no fake JobId, duplicate job, separate communication
mission journal or copied Comms controller state is introduced.

A communication primitive declares its actual issuing actor/System, a declared
base-world recipient reference, channel and ReportStatus request. First-recipient
scope is an existing base-world actor; launch-slot/dynamic recipient resolution
must refuse explicitly until its accepted bootstrap relation is supported. The
issuer keeps existing base/launch-slot support. Content validation checks both
references and actual issuing Comms equipment/channel across offered hulls.

Release arms the same bounded situation owner. Its ordinary Comms publication
includes a planned request only while the resolved recipient UUID is already in
that issuing Console's permitted current Comms contacts. It supplies no new contact
or hidden identity. Human and ordinary per-System AI use the same SendCommsRequest
with optional planned_request instance context, current known recipient and real
channel. Existing radio route, ownership, operation and finite energy gates remain
required. No World script sends on behalf of a human-owned System.

Before payment validate the exact released/unaccepted instance, issuing actor/
System and declared recipient/channel/request. After a successful ordinary paid
emission, bind its actual minted origin ID and emission tick at that same commit.
Duplicate or stale contexts refuse before another payment. An unrelated ordinary
request cannot be retrospectively attached; disappearance/route failure leaves
this instruction waiting without fabricating an accepted request.

The adapter observes only the issuing actor's actual sent Request and heard
returned Outcome lineage. A matching heard read receipt can acknowledge Read;
a heard Report or Decline acknowledges the distinct terminal outcome. Retain
original read/report clocks and the actual receipt time. Do not inspect a foreign
private inbox to tell the actor a request was read. A terminal receipt can supply
its original read evidence if the separate earlier receipt was not heard. Existing
saved pending World events retain these two bounded facts until acknowledgement;
the same SituationInstances own their historical evidence and Scenario effects.

A status report is factual evidence, not permission. Only an explicit authored
Scenario transition may create a distinct in-world permission or consequence.
No hypothetical report, AI decision or elapsed reading estimate satisfies that
transition. Source libraries gain the closed communication operator, typed actor
binding and read/reported/declined conditions; existing Workshop validated source
history/Test and privileged inspection edit and explain those same definitions.

Candidate preflight validates exact accepted source/recipient/owner/channel and
causal emission/read/report/receipt clocks before writes, and reconciles ongoing
bindings with candidate Comms records. Terminal historical facts keep their scoped
identity after ordinary message retention, just as completed Repair jobs outlive
team-slot reuse. Save/digest this owner once. Prove actual Human/ordinary AI send,
reading/return, unpaid/unknown/disconnected refusal, no unrelated receipt credit,
Scenario permission separation and full JSON/digest continuation. This extends
22C's real activity vocabulary; it does not finish continuing modes or opportunity
search by itself.


A validated planned single-recipient Request may mark its same retained Comms
transmission `sustained`. Existing radio processing retries delivery through the
current paid live route, with the same origin and recipient-copy identity. It
charges the one original emission plus ordinary continuous equipment/relay work,
not repeated mission emission or free delivery. No retry queue, hidden receipt
oracle or second AI is added. Ordinary manual requests/text keep their existing
one-shot policy. Candidate preflight requires every sustained Request to match
its actual accepted activity transmission binding. The existing sustained return
Outcome remains distinct. An exclusive prepared binding token can be dropped on
payment failure without writes; consuming it after ordinary paid emission commits
the accepted reference without a fallible post-payment validation stage.


### 22C authored permission depends on the actual Scenario application

Adopted continuation, 15 October 2026. Extend the same SituationInstance's
bounded activation history with the actual application tick for every retained
Applied or Refused result. The result and tick have identical state keys and are
committed together by the existing script-effect boundary. No separate permission
store or synthetic physical outcome is introduced.

An HTN predecessor condition `scenario_applied { state = "..." }` requires that
exact predecessor's authored callback state to have completed with Applied. The
condition uses its actual application tick, not the preceding radio receipt or
forecast. Source validation requires the referenced state to exist and contain an
explicit callback. A refused callback cannot release dependent work. Conditions
inside a Scenario lifecycle may refer only to an earlier callback in that same
acyclic lifecycle. Complete candidate validation checks key parity, original
nonfuture and nondecreasing application ticks, and that each application follows
its own required evidence before any writes. It retains the established limitation
that a snapshot cannot independently prove arbitrary historical script effects.

The next opt-in composition uses ordinary Repair as the hazard and an actual heard
ReportStatus followed by an explicit authored permission callback. It distinguishes
report evidence from that Scenario consequence. Separate operators can address
hazard and permission concurrently; cohosted Consoles share their actual operator's
attention budget. Declared temporal profiles and phase estimates remain provisional
until native and human timing evidence is recorded. The original commitment and
deadline never reset after late evidence or a callback refusal.


The opt-in `hazard-clearance-world.toml` imports the same actual Repair and
Communication owners concurrently. Only its successful authored `grant_clearance`
callback creates the narrative clearance; the next state consumes that exact
application receipt. Its separate/cohosted operator test uses real authored domain
phases and distinct deadline profiles. It demonstrates the scheduler rule rather
than measured human timing or the full two-/six-seat exit: profile thresholds count
operators involved in those leaves, and two Cruiser crew can already divide
Engineering and Comms. Broader Helm/personnel/transit obligations and human trials
remain required before claiming the crew-size experience is complete.


The host probe `hazard_clearance_continuation` supplies one positive incident
through the existing physical Damage.apply_hit owner, then requires ordinary
Repair restoration, paid radio work, an actually heard report and the explicit
clearance callback. It captures partial real AI reading and compares complete
fresh-JSON continuation for 1200 updates plus malformed-clock no-write checks.
This source checkpoint has not yet run natively. The quiet fixture by itself does
not create a hazard; an authored encounter cause and measured human workload remain
open, rather than treating a clean ship or pure forecast as positive hazard work.
