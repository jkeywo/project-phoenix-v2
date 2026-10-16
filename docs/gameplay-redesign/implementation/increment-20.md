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
nonfuture execution ticks and valid completion/return joins before writes.
Deployment/return duration must match accepted operator terms; a still-present
candidate target must offer the saved action with exactly the accepted risk and
working duration. Removed targets remain valid interruption/return continuations,
whose ordinary executor withdraws before any completion or consequence. The
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

The structural Security source checkpoint adds permitted shared Job work-list rows
to the existing Security blackboard and existing screen, using the established
read-only work-list renderer. Its six targeted client files pass 459 tests
(15 October 2026; worker tree), including return/terminal display and unchanged
Security dispatch/recall. New owner and snapshot tests cover identity, recall,
JSON application, strict old-shape refusal and malformed no-write candidates;
the integrator still needs to execute those native tests, repin intentionally
changed digest goldens, run actual hosted continuation and advance compatibility.
No native, human, route, Medical or increment exit is claimed from this checkpoint.

## Adopted physical local Security tracer

[ai] Optional `security.local` selects an explicitly local-only Security profile:
one home compartment per existing finite Security team and a closed list of
compartment targets/actions. Initial supported local verbs are secure/contain and
assist evacuation, with the existing authored work terms and consequences. The
ordinary dispatch command, Console, priority selection and admitted Backfill path
remain in use; local target addresses resolve only within the commanding ship.
The optional reference changes no shipped hull or scenario. External-only profiles
retain existing crossing behavior. A local profile offers and admits local work
only until actual craft/transfer custody supports mixed external operations.

[ai] The same canonical Security Jobs retain one RouteCursor per finite team. It
uses the existing InteriorGraph and actual InteriorAccess boundary state, including
its crossing safety interlock; no local deployment or return timer runs beside the
route. Work begins only at the authored destination, consumes existing actual
Security energy and Operation grants, and retains the normal work duration. A
blocked route waits or finds another actual route without reopening a requested
seal. Recall and machinery loss select return on the same job/cursor; they cannot
teleport the team home. Safe return continues even if the control machinery is
unavailable. The Job releases only on actual home arrival.

[ai] Accepted compilation validates homes, compartment actions, topology and
actual Security work-energy machinery. Capture/digest retain the same cursor with
mint/work/result state. Preflight validates accepted local mode, team/home count,
route/crossing, action terms and working/home location joins before writes.
Workshop edits these same profile values through accepted document history,
validation and Test; local authoring is not complete until that workflow exists.
This tracer does not establish hazard simulation, medical care, boarding or a full
increment20 exit. Native hosted continuation and human acceptance remain separate.

The physical local source checkpoint installs this consumer on optional
`security_cruiser.toml`: two existing finite teams, three authored one-shot
compartment tasks, inherited real energy and topology, one actual weapons access
boundary. Existing layouts and external-only profiles remain unchanged. The
structured Workshop Security panel supports accepted inherited materialisation,
home/action/risk/duration editing, explicit `local = false`, history/reload,
Preview/Test and cross-editor reference refusal. The player panel shows actual
compartment/crossing/blocked state, without a fictitious travel-progress timer.

Worker source validation: 10 client files pass 486 tests (14.14s); after the final
access-gated reference addition, the two affected files pass 18 tests (3.62s).
The actual Workshop build, layer/import check, strict 6,220-entry String Table
check and redesign ledger check pass. Independent source review passes the local
runtime/persistence/authority and authoring/GUI diff. Native owner/adapter/whole
snapshot tests and the new joined-host tracer remain unexecuted in this worker:
only the root integrator holds Cargo. The tracer requests ordinary joined Human
controls, actual seals, blocked/funded work, malformed no-write candidates,
fresh blocked/working JSON continuations of 360 steps and normal AFK Backfill
handoff. Authored assertions are pending evidence, not passing acceptance.
Medical, real external custody and full increment20 remain open.


## Adopted next local consumer: treatment

[ai] Optional treatment extends the existing finite named officer roster with one
coarse fit/minor/severe condition in PersonnelState. Authored initial injury is a
setup fact, not evidence of automatic injury, deterioration or casualties. An unfit
officer provides no expertise and cannot be newly posted; actual completed care
restores that same condition once.

[ai] A typed Treatment activity borrows the existing Repair TeamJobs slot and its
one physical navigation cursor. MedicalCare competency is authored on an existing
finite team; it cannot repair, refit, work abroad or treat another patient at the
same time. One active job claims one patient. The first supported patient remains
at its actual home, with configured service equipment in that same compartment.
The team travels to the patient, starts only on actual arrival with medication
and paid machinery, and releases only after real return. Recall preserves position
and already consumed medication. Power/access loss pauses or returns actual work,
never moves the patient to a convenient location.

[ai] Accepted care work terms, equipment, finite medication stock and configured
port replenishment must arrive together with Workshop editing and ordinary
Human/AI controls. Knowledge stays on the existing Repair Console. Explicit
human-seeking plot surgery remains an authored exception. Physical injury inputs,
remote casualties and carried-craft custody remain later joins; this local slice
cannot close increment20 or claim those capabilities.


### Local treatment source checkpoint

The optional `medical_cruiser.toml` binds MedicalCare to existing Repair team 1,
starting at Bridge, and an authored minor injury to the existing named officer at
Engineering. The patient stays at that actual home; accepted canonical Repair
machinery supplies care in the same compartment. The finite team walks the real
route, consumes one actual medical-supplies unit only when funded work begins,
changes the one PersonnelState condition to fit on completion and returns before
release. Minor/severe work times (6/12 seconds) and initial stocks are provisional
reference authoring, without a general injury or balance claim.

Treatment is a typed activity in the existing Repair TeamJobs. It excludes
repair/refit/external use of the same team and duplicate patient claims. Actual
service refits also exclude active care; existing fitting unavailability pauses
paid work. A later workflow failure during return preserves an already completed
patient outcome and still allows physical return. Required condition state,
accepted work terms, route/site, competency, actual patient and existing job joins
are validated before restore writes; no old conditionless continuation adapter
is introduced.

The reference has actual managed main/local medication stores, source goals and
capacity space. `medical_service_port.toml` offers replenishment from finite
stock through the existing paid Dock/umbilical/PortService path. Care consumes the
same capacity owner using the existing atomic requirement transaction, without
creating a medication inventory or persisted reservation frontier beside it.

The existing admitted Personnel applier handles posting and TreatDutyOfficer.
Its integration dependency is explicit: root must move the existing
HandleDutyOfficerPosting fixed step after HandleSetRepairTargetPriority and before
TickRepairTeams. Ordinary Repair AI reads only the permitted own Personnel view,
selects eligible patients when its damage queue is empty and emits that same
control. The applier then precedes funded work, so valid idle postings affect
that tick's work. If a same-tick ordinary dispatch already commits a team, a later
new posting/treatment is refused rather than moving an officer to an away team.
There is no duplicate applier, controller or central step.

The existing Personnel widget adds eligible care controls and actual team/phase,
progress and medication-waiting readouts. Workshop edits supported competencies,
initial conditions, service/work terms and finite medication requirements through
accepted history and Preview/Test, including explicit inherited `care = false`.
Worker validation: six selected client files pass 631 tests (5.10s); Workshop build,
layers and strict String Table checks pass. Independent core source review passes
conditional on the stated root schedule join. Model/Gameplay owner tests, actual
adapter, eight malformed whole-candidate cases, supported failing-return-tree and
finite paid-port replenishment tests are authored but unrun in this worker.
Root owns Cargo, compatibility versions and integration evidence. Normal joined
Human/AI care, fresh 360-step hosted continuation, physical browser/native and
workload acceptance remain pending. Injury propagation, away work, real craft
custody and capture remain open; this source checkpoint does not close increment20.


The separate actual-host Medical proof is authored and independently source
reviewed: `tests/medical_gameplay_continuation.rs` covers normal joined Human
admission, an incompatible-team refusal, physical arrival/funded work, one actual
medication consumption, seven malformed no-write candidates, fresh complete JSON
360 and ordinary AFK/Backfill/Human continuation. A second case starts no Human
care command and requires ordinary Repair AI to select and complete the setup
patient. Exact LocalShip UUID matching governs private inspection/forgeries.
The finite roster guard includes a forged terminal extra-team no-write regression
before accepted-home indexing. Native execution remains solely the integrator's;
authored assertions and source review are not passing gameplay evidence.

## Adopted structural prerequisite: retained personnel owner

[ai] Move the existing canonical per-entity PersonnelState maps into one Gameplay
PersonnelRoster resource. Stable identity is the original actor UUID plus the
accepted local officer ID. Each origin retains its finite accepted definition and
source provenance. CompiledPersonnel remains a derived actor binding; actual
posting, repair expertise, care and local read-model consumers join the one roster.
No old canonical map, dual-write adapter, extra crew pool or global crew planner
survives the migration. Authored profiles and Workshop editing remain unchanged.

[ai] Actual actor removal keeps ordinary destruction events, objective results,
GM semantics and body removal. On-board officers receive explicit terminal loss
records in the roster. A later passenger consumer can retain living off-ship
custody without retaining an operational origin ship or inventing another actor.
Restore suppression prevents bootstrap replacement from generating new casualties.
No passenger location, transfer scheduler or berth counter is added in this slice.

[ai] Snapshot and digest the roster once globally. Validate complete finite
populations, immutable accepted source definitions, active actor and Repair
job/cursor references, removed-origin terminal closure, unique origin identities
and nonfuture clocks before writes. Resolve retained runtime source recipes against
accepted content; compare authored/live definitions with their accepted bindings.
Missing required roster state and old per-entity payloads are refused explicitly.
Single-row machinery restore may validate against the live roster but cannot rewind
global personnel custody. Console and individual System AI publication remains the
explicit permitted local subset; the roster resource confers no knowledge access.

Structural owner, loss, malformed no-write and fresh hosted continuation evidence
are required before the owned-carrier passenger loop starts. Root owns compatibility
versions and native integration. This prerequisite does not close increment20.

The structural source checkpoint removes canonical PersonnelState components.
The one required global snapshot field captures immutable accepted definitions,
per-officer custody/condition, origin lifecycle and source provenance. Existing
posting, bonus, local treatment and permitted publication consume the same roster.
Actual profile despawn records terminal Lost custody and leaves ordinary entity
removal/events untouched. Whole-restore suppression prevents false bootstrap loss;
single-row machinery application cannot rewind personnel.

Runtime provenance includes the actual Entity WorldId birth tuple, derived from
the accepted actor identity rather than process insertion time. Preflight checks
canonical UUID rendering, exact receipt/key and birth-clock joins, the saved mint
frontier, accepted source definition and finite population closure. Authored
origins bind their fresh accepted UUID/definition. This validates consistency of
claimed supported history, matching ordinary snapshot semantics; it does not add
cryptographic historical authentication or a second population ledger.

Worker client regression evidence: three existing Personnel/Repair/Workshop client
files pass 72 tests (4.90s). Owner, actual adapter, source-receipt/terminal no-write
and joined Human/Backfill 360-step Medical continuation assertions are migrated
or authored for the single owner, but native execution remains with root.

## Adopted next consumer: owned carried personnel

[ai] Runtime work starts only after native verification of the retained roster.
The first reference adds finite accepted passenger berths to an actual battery-only
engineless carried pod. It reuses the existing ordinary entity, Bay Job, Carried/
Deployed custody, body presence, energy, damage and launch/recovery path. Existing
probes retain zero berths. This does not deliver a flying shuttle or special pilot.

[ai] Closed destination requests extend the established Repair work/request owner.
A request selects the actual officer and destination, never invents another people
pool or reserves all future work. Human control may explicitly choose an eligible
team; ordinary delegated Repair AI selects one through the same admitted boundary.
No unconditional boarding policy or second System controller runs beside it.
Later World/planner commands can issue that same normal intent without becoming
custody or completion evidence.

[ai] A typed activity borrows one existing finite Repair TeamJobs slot and physical
cursor, excluding repair, fitting, treatment and other personnel work. The team
first reaches the officer's actual source compartment. Only then does custody move
from that compartment onto the same actual committed Job/cursor. It escorts the
officer along ordinary access-controlled routes to the real carried pod's Bay
compartment. Actual handoff then installs the one AboardCraft/berth custody record.
The same team returns physically before its Job releases. After ordinary recovery,
a later job can collect at the Bay and physically return the officer to their home.
No patient, officer, team or craft is teleported by destination selection.

[ai] Berth occupancy and committed claims derive from the one roster and actual
active jobs. Launch/recovery and escort handoffs cannot concurrently claim the
same craft/Bay. Recheck actual carried parent, equipment/access readiness and finite
compatible receiving space before handoff; pending desired intent is not a claim.
Blocked routes, recall, changed destination or missing craft retain actual custody
and supported physical return. An already completed handoff is never retroactively
undone. Off-ship officers provide no original-ship expertise or manning bonus.

[ai] Keep the same origin/officer identity through departure, Bay launch, deployed
operation, recovery and unloading. Origin removal retains living off-ship custody;
actual host/craft loss records the supported terminal loss without protecting the
body, suppressing destruction events or replacing people. The full candidate must
close origin/source, actual craft/berth, exclusive team/Job and cursor references
before any restore writes. Whole-restore suppression must not manufacture loss.
Foreign disembarkation, rescue/capture completion and participant recovery remain
later consumers; recovering an occupied ordinary craft cannot duplicate its roster.

[ai] Extend accepted Workshop berth/profile/request references and the existing
Personnel/Bay controls through history, validation, Preview and disposable Test.
Explicit local feeds expose only permitted real boarding choices and work; a
globally stored roster creates no shared AI knowledge. Actual Human/ordinary AI,
physical collect/handoff/return, interruption, origin/craft loss, finite capacity,
malformed no-write and fresh 360-step complete capture/digest evidence are required.
Root owns compatibility versions and full gates. This bounded consumer does not
close increments19/20.

First-loop knowledge is deliberately local. Exact officer condition/location and
escort controls are published only for actual local compartments/teams/escorts,
or a pod physically stored on the origin carrier through its explicit permitted
Bay feed. Deployed, foreign-carried and remotely Lost officers produce identical
live controls and ordinary AI inputs. Existing actual departure Job history may
remain as earned history; it cannot be refreshed into privileged current remote
condition or loss. Bay manifests/counts derive only for locally stored craft.
Assert paired hidden-state variants produce identical permitted projections.
Later dated Comms telemetry can extend this view through its real delivery owner;
global physical custody alone is never a knowledge grant.


## Owned carrier source slice - awaiting native integration evidence

The first implementation uses `PersonnelTransfer` on the existing Repair
TeamJobs owner. `SetDutyOfficerDestination` persists a closed request on
RepairRequestQueue; `DispatchOfficerTransfer` borrows a compatible actual finite
team. The same ordinary Repair AI responds to explicit intent through these
admitted commands. A request grants neither custody nor a berth reservation.
Actual collection, escort, handoff and team return share one JobId and physical
cursor. Retarget/recall after collection escorts the officer home before releasing
the team. Completed handoffs remain completed during return cleanup.

The optional `personnel_carrier.toml` inherits the existing six-seat probe carrier,
adds finite Transport competency and two named officers, and explicitly grants
Engineering a Bay observation feed. Tactical retains Bay authority.
`personnel_pod.toml` inherits ordinary fixed battery-backed sensor equipment with
one accepted berth; it creates no personnel population, pilot or new controller.
Existing probes keep empty berth lists.

The first consumer serializes one active crew handoff per Bay. It excludes actual
Bay handling, refit and physical equipment stripping, while ordinary component
repair can coexist and actual availability can pause handoff. Canonical occupants
and current jobs determine free berths for every command and physical handoff.
The existing fixed-step owner remains a singleton: its exclusive wrapper rebuilds
an explicitly Derived tick-stamped context using the existing actual Bay readiness
function, then executes the ordinary admitted consumer. Restore clears this context;
it has no snapshot or digest payload. Missing/stale context refuses new work.

Global custody alone is never telemetry. Publication hides deployed,
foreign-carried and remotely Lost officers identically; only current owned stored
pods with an explicit Bay feed expose their local occupants and finite capacity.
Actual departure work remains factual Job history. Whole-candidate preflight
joins accepted craft/berths, unique occupants, exclusive Bay jobs, exact team/cursor
and retained origin receipts before writes; single-row rewinds refuse this graph.

Source review is independent of native acceptance. Targeted JavaScript checks
cover correlated destination/escort commands, stale held controls, delegation,
finite berth authoring, explicit feeds and unchanged seats/probes. Gameplay owner,
Simulation adapter and hidden-state tests are authored but unrun by this worker;
root owns compilation, schema/version joins and all native checks. The full-host
Human/ordinary AI, malformed no-write, loss and fresh JSON360 tracer follows in a
separate donor. This source slice is not an increment19/20 playable exit.


The separate `tests/personnel_craft_continuation.rs` proof now covers four
actual-host journeys: joined Human pickup/Bay exclusion/finite occupancy/launch/
recovery/unload, explicit intent completed by ordinary AFK Backfill, trusted World
body removal with normal destruction events and correct off-ship survival, and
retargeted same-Job physical cancellation/return. Eight malformed whole candidates
must leave complete capture and digest unchanged. Fresh JSON continuations compare
every step and final state across 360 updates while boarding, deployed, unloading,
returning and after loss. The AI case reproduces the real AFK holder on the fresh
host before restore and checks ordinary AI policy on both hosts throughout.
Independent source review passed; native execution is pending at the integrator.
This quiet one-host proof does not claim network/device or full20 acceptance.


## Next dependency: stable references and explicit actual holders

The first carrier loop deliberately supports own-origin officers. Its on-ship
compartment and posted-team locations implicitly mean the origin ship, and its
request keys are local officer IDs. A recovered pod can preserve foreign
survivors, but those survivors cannot honestly disembark into a different ship
until that ambiguity is removed.

First make an equivalent structural migration: a typed `PersonRef` contains the
existing origin UUID and officer ID; actual on-ship compartment/team custody
explicitly contains its holder actor. Transfer work, orders, normal controls and
permitted rows retain the same stable reference. The one global roster keeps its
existing origin/officer maps and immutable accepted definitions. No extra person,
manifest authority, current-position copy, scheduler or controller is introduced.
Existing own posting, local care, expertise, escort, physical return, loss and
strict continuation journeys must remain equivalent. Unsupported earlier payloads
must fail explicitly. This phase alone does not admit foreign jobs.

Knowledge and loss use different joins. Actual holder determines physical
casualties and local permitted facts. An origin row is retained identity, never
implicit remote telemetry or a continuing manning bonus. The receiving Console
may learn actual people aboard or a locally recovered pod manifest through its
explicit current-custody feed; the original ship's individual AI and controls
must still produce identical views for hidden remote survivor variants.

Only after that migration is verified should a bounded foreign rescue handoff
move a named survivor from an actual recovered stored pod to an accepted finite
receiving compartment. It must borrow the same real finite team and interior
cursor, commit real receiving capacity through actual jobs, and change sole
custody only at physical collection/delivery. Ordinary recovery or a destination
request is not rescue completion. This consumer will need normal Human/AI,
authoring, capacity/no-write/loss and full fresh continuation proof. Broader
boarding, capture, duty-officer away work and full20 acceptance remain open.


The dispatch boundary also latches the exact observed destination request. Custody
`changed_tick` alone is insufficient because changing intent need not move an
officer. The admitted owner compares expected/current requests before planning,
team claim or Job mint; ordinary AI uses its own permitted request. Held controls
retain request and selected-destination identity, so a replacement order or a
stored pod disappearing cannot become an unseen dispatch or fallback Home order.
Targeted GUI regressions cover both cases. The consumer-level stale rejection
asserts unchanged complete capture/digest, with a matching-request positive
counter proving that the same prepared actual hardware can mint exactly one Job.
That adapter fixture is not a substitute for normal Human admission/AI choice in
the separate host proof; native execution remains integrator-owned.


## Structural stable-reference source slice

The local owner now carries `PersonRef { origin, officer }` through the four
normal personnel commands, permitted rows, Treatment/PersonnelTransfer work and
Repair destination keys. Authored IDs remain local definition keys; no person is
recreated or moved to another roster. Compartment and posted-team custody require
the actual holder UUID. Ordinary loss resolves actual occupants; an origin row
alone cannot kill or boost people elsewhere. Existing local work still refuses
foreign references before claiming a team, changing intent or minting a Job.

The compound-key request map is encoded as ordered reference/request rows for
JSON continuation, refusing duplicate or unsorted keys and the old string-key
shape. Whole candidate checks require actual local holder, own stable work/ref
identity and matching request; unsupported old holder fields cannot default.
Existing owner, adapter and host fixtures retain their physical outcomes, finite
payments, strict full capture/digest continuation and no-write assertions while
using exact live origin UUIDs. Additional tests cover wrong holder, same-named
foreign reference, bad request identity and closed wire shapes. The owner-level
loss regression directly supplies offship custody to test loss routing; it does
not claim an admitted foreign transfer.

Targeted browser-module tests: four files, **201 passed**, 4.82 s. Owned Rust
formatting and diff checks passed. Native compilation, actual host re-execution,
compatibility joins and integration gates remain root-owned and pending; source
review is being queued. The receiving-ship rescue consumer and full20 exit remain
open. This slice changes no station layout, authored finite roster or Workshop
capability.


## Next bounded consumer: negotiated occupied-craft reception

The adopted PASM decision `redesign-occupied-craft-negotiated-bay-reception`
uses the existing physical Bay recovery path. A receiving facility accepts one
proposed craft through ordinary generic Comms; acceptance does not operate a
Human Bay. The receiving Bay needs explicit Comms and sensor feeds, and resolves
its own currently earned contact before a Human or its ordinary System AI emits
the normal admitted recovery command with the exact accepted order. A claimed
UUID is neither an observed contact nor machinery authority.

The same Bay Job performs finite paid recovery and changes the actual deployed
craft to Carried only at its normal physical commit. People remain in the sole
roster as AboardCraft with the same accepted berths, including after origin loss.
The first reference facility recovers only explicit accepted targets through its
existing authored tree; no arbitrary-contact controller or passenger pool is
introduced. Foreign disembarkation and captured-vessel control stay separate.

A typed receipt is written only after actual custody handoff and completion of
that same Job. It retains the immutable original Comms reference/decision tick,
actual receiver/Bay/Job scope, craft, handoff tick and sorted stable PersonRefs.
The existing recipient request progress then records the completed reception;
its ordinary paid reverse outcome carries that dated evidence to the sender.
There is no new message queue or Activity variant: `CommunicationReturned` can
carry the genuine nested physical result. Existing ReportStatus-only planner
validation continues refusing unsupported receiving work until its real operator
is implemented. Job paid execution, physical handoff and later reported arrival
are distinct clocks; no team-return phase is invented for Bay equipment.

Implementation scope includes an actual equipped fixed receiving facility,
finite capacity/access/energy, the normal Human and ordinary AI controls,
Workshop editing and validation, strict whole-candidate graph continuation and
fresh JSON proofs for source/origin/receiver loss. No runtime reception or full
Patrol acceptance is claimed by this contract commit. Root owns compatibility
versions, central integration, Cargo and final gates.


## Negotiated reception source slice

ReceiveCraft now carries the sender's proposed craft identity internally. Its
normal radio consumer requires a current own stored craft through an explicit
Bay feed. The receiving Bay independently resolves its own earned current alias
and exact accepted local Comms order; Human Comms cannot operate a Human Bay.
The ordinary Bay state tree uses has_accepted_reception and normal admission.

The same finite paid Bay Job retains the order and stages its physical receipt
with the exact original receiving copy. Only successful actual set_carried and
same-Job completion publish that receipt and mark the existing request completed.
Its later paid reverse outcome reports the original handoff clock. People remain
AboardCraft in the sole PersonnelRoster; local stored manifests are derived,
including retained original identities after origin loss. No foreign room,
second manifest owner, Activity kind, controller, queue or return phase is added.

Whole-candidate checks join accepted Comms order, actual Bay Job/mint, known
finite personnel and identical historical receipt copies. One physical Job
cannot claim conflicting receipts. Current custody is not inferred from an old
receipt. Ordinary source, receiver and pod destruction semantics remain intact.

Optional reception_carrier preserves the six-seat machinery and adds an
information-only Bay feed to Comms. reception_outpost is actual fixed equipment
with finite positive listening/handling energy costs, interior and normal Bay/
Comms policies. Existing own publication and authored policy installation cover
that static equipment without adding propulsion. Existing Workshop composition,
source/history and tree bindings edit the real capability.

Targeted JS/authoring passed five files/34 tests (1.60 s). Layer rules and strict
strings (6415 rows) passed. Independent source review passed the core plus narrow
static publication joins. UI/data review passed after a missing positive radio listening cost and retired
widget callbacks were corrected with assertions. Rust owner
fixtures are authored but unexecuted here. Native compile, actual Human/AI radio
and physical host proof, full JSON continuation/loss/forgery evidence and full
reception/Patrol acceptance remain pending root integration. This source slice
is not the full increment20/21/23 exit. Root owns versions, Cargo and final gates.

## Reception integration checkpoint

The reception donors were replayed onto the current media/capture integration
at `9d9e8746`, preserving both optional Comms capabilities, ordinary radio
settlement order and candidate-only captured-evidence validation. The request
enum's owned craft identity removes `Copy`; existing Media/FormUp and planned
communication consumers borrow or clone the same values. Planner validation
remains explicitly ReportStatus-only.

The actual Console builder now forwards the selected Comms publication's
`reception_craft` list. A regression exercises that builder with a nonstandard
Comms System ID through the normal request action, and verifies that a separate
unbound Bay publication does not supply craft choices.

Targeted browser tests passed **455 tests across six files** (1.46 s), including
the Console builder, Comms actions/requests/capture, reception and Workshop craft
authoring. Layer rules, strict strings (**6430 rows, zero errors**) and diff
checks passed. Independent source review passed the merged runtime/candidate
joins and the narrow planner ownership changes. Native compilation, protocol/
save/rules version joins, actual Human/AI host reception and JSON/loss proofs,
device/network acceptance and final integration gates remain root-owned and
pending. These checks establish a source integration checkpoint, not an
increment20/21/23 exit.

## Actual reception host proof source

A separate source-reviewed proof extends the existing personnel_craft_continuation
integration target through tests/personnel_craft/reception.rs, using an actual
quiet authored fixture. Normal Human source Station holders escort a real named
PersonRef into its finite pod, request reception through paid Comms, and launch
through the real Bay. A genuinely fixed non-Ship receiver reads and accepts with
ordinary Comms AI, acquires its own paid sensor alias, and completes the same
paid Bay Job before returning its factual receipt. No custody, observation,
energy, controller or completion state is injected. Trusted World body removals
are explicitly named inputs, not claims about GM session admission.

The proof compares complete capture and digest across fresh JSON restore for360
steps plus the final state at accepted, active/completed, historical and loss
boundaries, checking ordinary receiver AI on both hosts. Seven malformed receipt
candidates assert full no-write refusal. A second actual recovery retires the old
Job and fills the receiver's finite Bay; an otherwise Model-valid copied Comms
lineage claiming that old physical scope must then fail the exact conflicting
receipt guard. Actual origin loss preserves offship identities through reception;
later receiver loss removes the actual pod and resolves its occupants without
changing factual handoff history. All native execution is still pending root;
this source-reviewed test is not evidence of a completed gameplay or device exit.
