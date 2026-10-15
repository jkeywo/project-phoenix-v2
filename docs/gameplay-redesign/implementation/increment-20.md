# Increment 20 — Personnel, Medical and physical custody

Contract preparation, 15 October 2026, under the main `bc5cfbc6`
[parallel-work contract](parallel-work.md). Local personnel work may follow 15's
initial integration and targeted checks; full completion depends on 14 and 19's
verified exits. External operations join 19's actual craft/deployment contract.
The initial local slice is integrated as `7c45dd28` (source checkpoint
`320e8f0e`). It adds finite officer custody and home-only posting to the existing
Repair teams, an optional provisional expertise hull, existing-screen controls
and accepted Workshop roster editing. It keeps the actual team navigation and
work owners. Independent source reviews and 159 targeted client tests passed
in the worker. Later joined-host execution also passes actual posted/unposted
expertise, finite parts and partial-work custody through 360 complete JSON
capture/digest comparisons; see the acceptance evidence for source pins and limits.
Compatibility advances to protocol 16, snapshot 57 and simulation rules 0.29.
Medical, security, boarding, away/transport/capture joins and the full playable
exit remain open. See [acceptance evidence](../../acceptance/1562-personnel.md).

## Outcome and scope

Make finite teams and named expertise useful without mandatory leaders, manual
walking or a second unit-control game. Give Medical, internal defence, away work
and rescue physical consequences while humans retain useful participation after
simulated loss. The catalogue, AD56–58/75–76 and
[PASM contract](../../../pasm/spec/design/personnel-custody.yaml) govern.

| Selected entries | Delivery and boundaries |
|---|---|
| G068/G076; S079/S080 | Finite Duty Teams with stable authored competencies and useful overlap. Named Duty Officers add compatible team/System/away bonuses, not basic eligibility or a new AI skill tree. Assignment travel is automatic on the actual ship map; bonuses begin only on arrival. |
| G069; S088 | Prepare, equip, dispatch, support and recover real away teams. Routine execution follows ordinary approved work without compulsory repeated prompts. Display exact check percentages and contributing factors for supported checks; no covert modifiers hidden behind a supposedly exact number. |
| G070; S082 | Located boarding/internal-defence teams use actual compartments/inter-deck routes and objectives. Whole-ship capture transfers operational authority; a local team objective cannot grant partial System control. Captured Console records retain original evidence age/uncertainty and normal access limits. |
| G071/G012; S081 | Minor injury permits reduced duty; severe injury prevents duty. Treatment uses actual finite services/work. Explicitly lethal circumstances can permanently kill simulated officers; no per-damage-tick death rolls, automatic deterioration loop, mandatory clone system or human elimination. |
| G061/G064; S085/S087/S100 | Actual external repair, transport, rescue/extraction, abandonment and recovery. Suitable established destination evidence is required, not a universal receiving pad. Range, capacity, shields, interference, damage and access remain real. Pods offer supported open/directed/secure/silent signalling; silence is not guaranteed stealth. |
| S028/S029/S030/S040/S102 | Fitted isolation/breakers/bypass work, suppression/seals/venting, self-destruct interlocks and coarse compartment viability share actual equipment/routes/boundaries. No second door state or detailed numerical atmosphere/pressure solver. Deliberate isolation cannot be reopened by a pathfinder. |
| G110 | Only specifically authored mission-critical work, such as an exceptional surgery, adds manoeuvre constraints. Routine healing/maintenance remains forgiving; Helm retains authority. Existing propulsion stress/external geometry is separate. |

The reference Cruiser may assign Medical to Science while Engineering retains
life-support machinery and repair/stores; Tactical retains physical transport/
boarding commitments and Comms establishes needs/access. These remain data-driven
layouts, not runtime role rules. Preserve 2/4/6/8-seat progression, visiting controls,
ordinary delegation and Captain's lack of a private interior/space truth map.

## Current owners and first local consumer

Gameplay `jobs/repair*.rs` already owns real Repair JobId/team navigation, physical
work, recall and return. `security/teams.rs` supplies the existing closed Security
activities and finite team state; Simulation `security/server.rs` applies admitted
dispatch and supported target outcomes. Transporter, external repair, compartment
damage/interior routes and `crew_spectator` are existing integration boundaries.
Extend these actual consumers; do not mistake Security's older crossing timer for
already implemented common compartment routing or a complete personnel model.

First attach one finite named Duty Officer to an actual existing Repair team or
compatible fitted System. Reuse actual navigation/work to move that officer and
activate/deactivate its one supported bonus. Prove basic work without the officer.
Then migrate a real Security task onto shared team/job/navigation rules and add
one actual local treatment consumer. No unused general personnel scheduler or
additional crew AI is a prerequisite to these tracers.

Model owns closed IDs/competencies/conditions/assignment and DTO contracts;
Gameplay owns shared team/job/custody transitions and check rules. World owns
authored work, risk/context and actual outcome effects; Session owns participation,
station tenure and operational control. Simulation stages cross-domain inputs and
commits real transitions. Human and ordinary individual System AI issue the same
admitted commands; no sibling-domain dependency or global personnel controller.

## Shared 19/20 physical custody contract

Adopt this join before either lane implements passenger craft. 19 owns actual
craft identity, deployment/motion, finite cabin/berth capacity and recovery; 20 owns
actual personnel identity, condition, assignment and location/custody. A craft's
passenger view is derived from this same custody owner. Do not add a second
passenger stock, cloned officer/team roster or freight count that can independently
create people. Anonymous membership can remain aggregate but must conserve
quantities and team identity through every split/merge supported by an actual job.

One stable person/officer/team identity has exactly one physical custody/location:
an actual compartment, actual carried craft/berth, or actual in-progress transfer/
crossing job with committed endpoints. Assignment is intent, not another location.
Existing team navigation owns actual cursor/crossing; custody references it instead
of ticking a competing movement copy. A transfer record is an actual location and
one existing job commitment, never a free-floating duplicate at both endpoints.

The carrier declares capacity/compatible transfer access through its own accepted
profile; personnel supplies actual occupied space/eligibility. Stage source presence,
physical access/link, compatible destination capacity and exclusive team/job claim
together before starting. Only commit the location handoff at the actual completion
boundary; reserve supported receiving space for dispatched work without creating
another passenger inventory. Normal rejection leaves no new partial claims/payment.
Mid-crossing cancellation/departure/loss preserves the real cursor/transfer state
and supported return/recovery outcome; it cannot teleport everyone home or refund
completed travel. Destination loss never mints a replacement team.

19 exposes actual carrier/location/transfer availability, not private personnel
truth. 20 exposes permitted counts/readiness/transfer outcomes, not hidden interiors.
Shared Simulation adapters join the actual owners and their candidate restore state.
Whole-ship rescue/capture can change custody only through supported real consumers.
Comms 16 conveys requests/receipts, not instantaneous physical handoffs; 18 supplies
actual movement. Neither lane waits for the other's full completion merely to
adopt this shared contract; prove the joined consumer before passenger acceptance.

## Team work, expertise and safety

Retain one actual finite team commitment and real JobId from dispatch through work,
recall and return. Competencies are stable authored capabilities with overlap, not
routine conversion of a spare team into a new speciality. Basic teams function
without officers. A named officer's compatible assignment is exclusive; leaving
the old post ends its active bonus, travel grants no destination bonus, and arrival
begins the new one. Cancellation/reassignment respects actual location and route.
No instant swaps, shift/watch chore or per-member first-person simulation.

Automatic routing reads the same actual boundary state used by containment.
Requested closure, actual closed/sealed, damaged and unavailable states are distinct.
Routing can choose a viable alternative or wait; it cannot reopen a deliberate
seal to satisfy a job. Fitted remote switches may isolate existing paths, while
building a physical bypass needs real teams, access, materials and work. No magical
new resource edge. Hazard/viability changes can interrupt or require relocation
under explicit supported work rules; they do not erase committed people/resources.

Coarse life-support/atmosphere conditions retain reserves and intervention margin.
Use shared damage/energy/Stock and actual compartment supply, not generic hull
health damage or a new pressure-flow simulator. Suppression/venting/sealing follows
actual capability and access. Self-destruct uses supported interlocks/authority and
explicit consequences; the name is not permission to bypass protected decisions.

Medical checks minor/severe condition and actual treatment eligibility. Only a
declared lethal circumstance can remove a simulated officer permanently; disclose
the lethal risk before commitment where the action is offered. First supported
checks use displayed numeric factors for the exact advertised probability and
existing deterministic outcome/RNG ownership. Unknown conditions may introduce
explicit new circumstances, not secretly alter that advertised check. Save actual
check/outcome frontiers so restore/retry cannot reroll. Broader special risk policies
require their own supported definition rather than implicit hidden modifiers.

Boarding/away objectives use supported actual work and route destinations. A secured
compartment is not partial operator authority. Successful capture changes the whole
ship's operational controller via Session's actual ownership/admission, retaining
physical equipment/crew/resources/records and individual System controllers.
New operators can read captured Console records within ordinary access; copies keep
source, age and uncertainty. Do not reveal all hidden enemy rooms through capture
progress, invent arbitrary betrayal, or remove meaningful human participation.

## Ordered slices and later joins

1. Local finite roster/expertise: one actual team/System posting with physical
   travel, no bonus while en route, normal cancellation and no-officer baseline.
2. Actual internal Security/Medical jobs: shared navigation/commitment and one real
   treatment, coarse hazard/seal/access consumer. Preserve existing Repair/refit
   physical work and recalls while removing any migrated duplicate team timers.
3. Joined 19 transport/extraction: actual craft/Transporter destination evidence,
   capacity and shared custody, interrupted movement, rescue and external repair.
4. Actual boarding/capture, away checks/support, abandonment/pod signalling and
   recoverable persistent casualties, preserving useful participant continuation.
5. Shipped activity/complexity layouts and 21 workforce using the same underlying
   team/custody model. Add no required seat or routine surgery/transport minigame.

Workshop supports accepted roster/competencies/officers, compatible postings,
closed work/check/risk, room/boundary, transport/capacity and treatment profiles,
with includes/history/Preview/Test. Keep actual hardware/configuration bindings
and normal Station complexity; no arbitrary runtime script editor or new AI
behaviour-unlock tree. Authored exceptional human choices remain protected.

## Continuation and completion

Capture each canonical team/officer/custody/job/condition/outcome once, with actual
travel/claims/check frontiers and operational ownership. Derived assignment/status
views do not own bonuses or people. Preflight the whole candidate across source
and destination, including materialised actors/carriers, exact capacity/JobId/
cursor/boundary joins, no duplicate identity, finite condition/work and nonfuture
ticks, before any live writes. Snapshot/digest/census/schedule/version changes go
through one integrator. Unsupported old state needs refusal or explicit conversion.

Use actual normal Human/AI commands to prove paid local posting/treatment, blocked
route/seal, injury/bonus changes, finite expertise/team availability, withdrawal,
successful/failed exact checks and no reroll, shared custody interruption, real
destination loss/capture, records and participant continuation. Restore fresh JSON
hosts at actual travelling/working/transferring checkpoints and compare complete
captures/digests for at least 360 steps; different-LocalShip peers must agree. Forged
IDs/location/claims/capacity/ticks/outcomes refuse without live writes.

Shipped scenarios/hulls and Workshop round trips, no-defaults owner tests, physical
browser/native journeys, actual risk/workload/conservation comparisons and 2/4/6/8
human/delegated layouts remain required. Local 20 source does not close external 20,
19 passenger operations, 21 workforce, human acceptance or the full increment.

## Adopted next local consumer: Security jobs

[ai] Structural migration first: the existing finite Security muster uses the
shared `TeamJobs<SecurityTeam>` commitment and JobId mint. The previous independently
saved assignment vector is removed. Existing target/action, paid work, deployment,
withdrawal, ordinary AI selection and authored World consequences remain equivalent.
Idle slots project an available team; unavailable idle status is derived from the
actual System availability rather than persisting a second personnel condition.
A Job retains its real assignment through return. A completed, recalled or failed
work result is held by that same activity until the team arrives, when its one
terminal Job outcome releases the commitment. A new dispatch never replaces a
returning team. No elapsed work, target or result is reconstructed from an AI claim.

[ai] Capture saves the complete bounded canonical jobs, including next mint,
workflow state, phase clock and terminal result. Restore validates actual accepted
owner/team count, unique prior JobIds, finite clock/risk, closed activity/phase,
nonfuture execution ticks and valid completion/return joins before writes. The
digest includes that same complete canonical state. Initial idle Security may omit
the save payload; a used mint or retained terminal outcome cannot be omitted.
Legacy Security assignment saves require the increment's explicit version refusal;
there is no runtime old-data adapter.

[ai] Subsequent local work binds an authored compartment to this same activity and
uses the existing InteriorGraph RouteCursor and actual access blockers. It replaces
local deployment/return clocks; external crossing remains the explicitly retained
legacy capability until the real increment19 transfer/custody consumer joins it.
This structural checkpoint does not claim local routing, treatment, external custody
or a complete increment20 exit. Root integration owns compatibility versions.
