# Increment 18 — Movement, docking and transit

Contract preparation, 15 October 2026. The approved
[parallel-work contract](parallel-work.md), pinned on main `bc5cfbc6`, permits
movement work after 15's initial integrated implementation and targeted checks.
Full completion depends on 14/15's verified exits; remote formation request delivery
joins 16's integrated channel consumer. Preparation starts no runtime or acceptance.

Initial foundation integrated on main at `45a17f28` and `940a973c`: closed
owner-bound Helm requests, finite intent validation and the shared physics
calculation's assisted/direct interpretation. The existing physics entry point
retains its assisted behaviour, and Cruiser opts into explicit axis bindings.
Actual admission/application, paid integration, retained views, GUI/AI and
Workshop/continuation joins are still being implemented in the movement lane.
This foundation has independent source review; native execution and the playable
exit are pending. An authored binding alone does not establish working controls.

## Outcome and selected scope

Give Helm useful control of speed, heading, relative motion and travel while actual
equipment, energy, conditions, mass and geometry remain decisive. Assistance uses
the same actuators and physics as direct input and ordinary individual System AI.
It does not hold ideal combat range/aspect or make another officer's decisions.
The selected catalogue and [PASM contract](../../../pasm/spec/design/movement-transit.yaml)
govern; researched portal/instant-kill mechanics are not automatic requirements.

| Selected entries | Required behaviour |
|---|---|
| G027/G032; S044–46 | Desired forward/reverse speed and absolute heading assistance, plus a visible HUD direct-control toggle for thrust, steering and fitted lateral control. Capital ships remain planar except limited collision avoidance; actual small craft use full 3D. No new parallel physics or controller. |
| S047 | Operator-controlled boost duration under actual energy/thermal/equipment limits; stopping thrust preserves momentum. Retire a universal fixed burst without silently converting every equipment profile. |
| S048 | Persistent Match Speed uses the same selected contact's last observed speed. Throttle or explicit toggle cancels it; heading remains Helm's. Loss holds the last observed value with stale feedback; reacquire only the same measured contact, never unseen motion or an arbitrary replacement. |
| S049; S083/S084 | Existing physical docking, towing and umbilical constraints stay actual. Add short visible equipment/state-dependent separation and supported emergency release consequences. Delegated approach remains one existing Helm controller; manual takeover is explicit. Services/stock transfer remain their own owners. |
| S050/S051; S041 | Configured discrete inter-system transit and optional tactical jump, finite long-distance fuel, scatter-then-gravity-push arrival. No universal receiving anchor. Local jumps reset velocity in the local navigation frame; ordinary propulsion remains electrical, not propellant consumption. |
| S052/S053 | One destination remains sufficient; optional multi-leg routes and relative formation slots use ordinary Navigation/Helm intent. Publishing a route is not engaging autopilot. Formation jump/readiness and breakaway respect each ship's actual authority/capability and received 16 requests. |
| G045 | Integrate supported anomaly/gravity/route consumers when authored and evidenced. Static orbital bodies remain static; this increment does not adopt every researched wormhole, orbital-motion or lethal-portal idea. |

## Current owners and first real consumers

`ship/physics_systems.rs` integrates actual `ShipPhysics`; Gameplay `ship/helm.rs`
and the existing per-axis `ship/helm_ai/` owners carry/execute admitted intent.
`console/helm/server.rs` and `ship/helm_admission.rs` own normal controls;
`entities/config/helm.rs` holds accepted equipment data. Extend these boundaries.
The actual Cruiser engines, steering, lateral/boost equipment and Helm surface
are the first equivalent consumers. Preserve promoted NPC and player machinery
parity; do not let LOD create a second drive intent or unlimited free motion.

Existing `dock/server.rs` and `dock/mating.rs` own physical mating. Navigation
course/waypoint/sharing, Tractor and Umbilical retain their owners. Derive assistance
from each actual controlling System's permitted Console observations and current
contact routes; internal actor positions are only physical collision/coupling
inputs. The 14 observed Helm surface and 15 mount/weapon geometry remain read views,
not a shared omniscient navigation scene or an automatic human-control override.

Transit is a new supported capability, not an existing completed drive. Its pure
profile/state belongs Gameplay; Model defines closed authored data and commands;
Simulation joins actual accepted equipment, resource and world placement owners.
World selects authored destinations/sector transitions and consequences; Session
keeps participation/authority. No package imports a sibling domain. Reuse current
world load/spawn/content binding and deterministic minting at the join rather than
creating another universe, transport or always-running sector simulation.

## Intent, observation and physical execution

Separate desired intent from actual velocity/heading/readiness. Human direct input,
assistance and delegated AI are mutually exclusive writers under existing control
policy. Switching mode cannot reset motion, refill reserves, clear conditions,
teleport or make a second actuator pass. Use one existing energy/Operation grant
per actual owner, with explicit fixed-step ordering and idempotent repeated input.
Ship heading control is absolute; supported assistance reaches it only within
actual turn limits. Finite output/acceleration and inertia determine braking.

Match Speed stores its public contact handle and last legitimately observed speed/
sample tick as local intent, not another target truth store. Unknown velocity is
unavailable, not zero. A newly delivered sample may update matching; stale/predicted
information never silently reads current actor velocity. Physical executors still
apply collision/hold constraints. Local cancellation and observation loss have
different semantics and truthful controls/status.

Formation jumps use the best participating drive stats, as selected in S053.
Every member still needs its own working drive capable of that jump type and
its actual authority, readiness and resource commitment. Resolve scatter and
gravity displacement once for the formation reference point, then apply the same
translation to all participating ships, preserving their actual relative offsets.
Do not snap ships to slots, guarantee clearance or carry a ship without an
operating drive. The first supported profile must state which participating stats
combine and how membership/readiness loss affects commitment; best stats must not
silently become the weakest member's stats or independent per-ship scatter.

Formation slots are optional authored relative geometry, not a central fleet AI.
Each recipient accepts permitted intent and executes with its own ordinary Helm
Systems; other ships' movement, readiness and agreement are observed/reported.
No guaranteed perfect station-keeping or new remote actuator permission. Link loss
preserves accepted onboard work under its declared policy; a sent/cancelled 16
request is not already received. A human can break away through normal authority.

Dock approach respects actual visible target knowledge and physical mating rules.
Define one assistance handoff/cancel state on the existing controller; resume manual
steering before coupling without two active writers. Once coupled, undock/emergency
release uses the existing physical connection state, with actual separation work,
availability and authored consequences. Loss of contact does not prevent legitimate
release of one's own hold; it also does not invent new mate facts or services.

## Transit and arrival contract

Authored drive profiles declare finite preparation, paid operation/fuel, supported
destination domain, accuracy/gravity response and interruption policy. Tactical
and inter-system drives can differ without a global transit/combat lock. Validate
all finite geometry/resource bindings before starting; actual stock is the sole
fuel owner, with normal reservation/commit semantics and no parallel travel counter.
No unsupported phase or failed start may consume half a required indivisible set.

Prepare, committed travel and arrival are distinct canonical phases. Authored
effects/reporting explain what interruption can stop, retain or consume. Restore
must not repeat fuel payment, arrival effects or entity minting. For sector travel,
capture the actual transition/content binding rather than inferring arrival from
wall time or whichever client has a world loaded. Keep active physical entities,
launched ordnance, carried equipment and accepted obligations in their real owners.

Arrival first scatters around the selected point, then applies the supported
outward gravity displacement. Use deterministic existing RNG/mint ownership and
stable body order; gravity bodies remain actual authored static bodies. Local
jumps end at rest in the local frame. Define inter-system velocity/heading explicitly
with its first real profile, rather than silently applying a different drive's rule.
Arrival accuracy is evidence/forecast, not a guarantee of clearance or a hidden
map. Physical placement/collision may inspect actual bounds solely to resolve the
arrival; disclose only supported outcomes. Reject nonfinite/unrepresentable geometry,
not with a new arbitrary player-input puzzle or universal receiving beacon.

## Adopted first LocalJump consumer

The first supported transit slice is an optional `TransitDrive` component on an
actual individual System, with an explicit finite LocalJump profile. It does not
convert Impulse, host teleport or every existing hull into a jump drive. The
existing Navigation waypoint remains the sole course source. The drive's Console
must explicitly receive that same typed Navigation publication; a normal Human or
AI Begin reads only that permitted own course and freezes its accepted point and
original course generation/date. No live foreign target lookup, global waypoint
permission or privileged AI route is introduced. Changing Navigation later does
not redirect preparation already accepted by the drive.

Normal admitted Begin arms preparation; preparation advances only through the
installed drive's current source, Operation and actual paid continuous work.
Unavailable equipment holds its accepted work and goal. The same ordinary
individual System AI uses a closed authored tree over an explicit permitted
transit request/directive, its permitted course, local capability and own phase,
and emits the same admitted Begin/Cancel actions. Mere course availability never
orders a jump. Each request uses the existing objective id plus an explicit
revision. A bounded handled-request frontier stays inside the same drive state;
reserve its capacity before admitting a new distinct request, never evict a handled
key, and refuse reuse/change of an accepted id/revision. Two standing objectives
must not alternate completed jumps. Preserve old revisions through restore without
re-execution; this is own order receipt evidence, not World mission progression.
The ordinary Begin payload may carry this optional exact permitted request key but
no position; without a key it remains a deliberate manual jump from the permitted
own course. AI emits only a fresh visible key after the existing per-System text
reading delay; authored `reading_ticks` reuses the same TreeRuntime continuation,
not another timer or planner. Frontier capacity is a technical validation bound,
not a message-capacity gameplay task or an extra operator job. It has no second planner or
motion authority. Repeated Begin while active is refused without payment, and
Cancel is allowed only before commitment. Human takeover/delegation does not itself
rewind, cancel or complete accepted onboard preparation.

The canonical phase/destination owner distinguishes Idle, Preparing, Committed
and the completed arrival frontier. A ready preparation automatically attempts
commitment. Freeze the accepted drive profile and fuel requirement for this order;
later equipment/profile changes cannot reduce its required work or alter an
already selected arrival. A failed commitment retains an allowed waiting phase
with a truthful reason, without double spending or repeating scatter.
Fuel is a real finite commodity in the existing actual Local store
bound to this System through Logistics/Capacity. Stage the existing Stock claim
and actual discrete Power debit atomically; on shortage or refused work neither
resource commits. Successful commitment consumes both once and fixes the arrival
result once. Cancellation discards preparation without refunding already performed
work; committed travel cannot be cancelled through the pre-commit Cancel action.
No second fuel counter, generic inventory, reservation registry or duration
controller is added. The optional reference hull must include finite fuel and a
real supported Stock delivery/replenishment journey, not just an inexhaustible test
balance or a hidden refill on Begin.

One arrival owner writes the existing `ShipPhysics` pose and resets local velocity
once. The accepted origin and destination, original acceptance tick, paid work,
commit/arrival frontier and selected deterministic result continue exactly through
restore and LOD; no bootstrap may repeat fuel debit, scatter or arrival effects.
Preparation/commitment status and controls must state what can still be cancelled.
The arrival placement is the existing physical owner, not another continuously
running motion integrator or a host-only teleport exception.

Arrival accuracy uses a stable drive/order-keyed scatter, then stable physical
outward gravity displacement. Gravity requires an explicit finite authored
arrival policy on actual World/body content; a star/planet visual, displayed size
or mass alone grants no gravity behaviour. The physical placement solver may read
those actual bodies solely for arrival resolution and reports only the supported
own result. Validate finite representable geometry and stable body ordering;
ordinary propulsion and static orbital bodies remain unchanged.

The existing per-ship drive map permits only one active Preparing/Committed order.
A competing Begin refuses before work, and whole-candidate preflight rejects
multiple active orders. Alternate drives retain their own histories without a
second mutex, controller or physical arrival writer.

First supported sources use typed optional `arrival_gravity` on the normally
resolved EntityConfig, explicitly authored through the static World row's
ordinary overrides. Require an unambiguous finite absolute position and an
ordinary non-movable resolved actor. The initial supported source set is the base
World only; additive/layer gravity requires a later explicit composition extension
and refuses rather than silently omitting a configured body. Existing static spawn-reference
validation rejects gravity-bearing action/script templates; the existing runtime
SpawnEntity dispatcher also refuses a resolved gravity policy before minting,
covering computed overrides without another registry or lifecycle. Relative, random, conditional and movable
sources refuse with an actionable error. At commitment, the actual physical
source must still agree with that accepted fixed policy and pose; a moved source
refuses instead of applying its obsolete authored location. Keep the original
sorted body inputs and keyed scatter only in the same order's private arrival
receipt; after commitment that receipt survives later source loss. Validate its
accepted immutable provenance and deterministic reconstruction before restoring
any state. These private inputs are not operator observations.

Publish the
supported own arrival forecast and actual displacement through normal views;
refuse nonrepresentable/no-valid-placement results without guaranteeing collision
clearance. A formation later
uses one reference-point scatter/gravity translation for all participating offsets
and the selected best participating drive stats, while every ship still commits
its own working equipment, authority and resources.

Inter-system destination/content-binding transitions and formation request/readiness
joins extend this same phase/destination contract later. They remain open after a
LocalJump checkpoint. This first slice still requires accepted Workshop fields,
normal AI/Human controls, truthful status/HUD, actual paid/fuel conservation,
interruption/repeated-order/no-write cases, and active fresh-JSON 360-step complete
capture/digest continuation. Source or LocalJump owner tests do not close those
later capabilities or the full increment 18 acceptance requirements.

## Ordered slices and authoring

1. Migrate actual Cruiser speed/heading/direct controls and ordinary AI onto one
   shared assisted/direct intent contract with equivalent baseline motion first.
2. Add observed Match Speed, hold/stale/cancel/reacquisition and a useful boost
   configuration; exercise finite energy/conditions and meaningful braking.
3. Extend actual Dock separation/emergency release and delegated approach/manual
   takeover; keep Tractor/Umbilical/service joins physically consistent.
4. Add one authored local jump and one discrete transit consumer with actual fuel,
   preparation/interruption, deterministic arrival and continuation. Record tuning
   as provisional and compare old content intentionally, without changing unrelated
   scenario pacing/deadlines to make a test pass.
5. Add optional route legs/formation slots and actual 16 requests/readiness outcomes;
   demonstrate a real 3D small craft through 19's shared lifecycle when available.

Workshop uses accepted definitions/history/includes/Preview/Test for drive limits,
boost/jump/transit profiles, route/formation geometry and emergency-release policy.
Expose supported finite controls, not a programmable physics processor. Existing
static mounts/equipment and optional no-transit hulls stay valid. The reference
Cruiser keeps Helm motion authority; no new required seat, Captain map or per-bank
Helm approval. Formation data may not write another ship's canonical motion.

## Continuation and verified exit

Save canonical mode/intent, Match Speed's dated held value, route/slot state, actual
Dock separation and transit phase/payment/frontiers at their existing owners.
Do not save duplicate actual physics, inferred target motion or derived previews.
Whole-candidate preflight checks actual System/profile/owner, fixed clock, finite
limits/phase/route/fuel claims, exact physical relationship and missing-actor
materialisation before any write. Coordinate schema/digest/census/version/schedule
joins once with the integrator; incompatibility is refused or explicitly converted.

Prove actual normal Human/AI commands, current observation scope and no guessed
private target path; direct/assisted handoff; boost stop momentum; stale matching;
limited capital versus full 3D craft; separation/failed emergency release; formation
loss/breakaway; exact fuel and arrival conservation; interruption/duplicate order;
and invalid/future/tampered candidate no-write boundaries. Actual active manoeuvre,
coupling and transit checkpoints require fresh JSON restore and complete capture/
digest equality for at least 360 fixed steps plus different-LocalShip peer proof.

Shipped hull/world/Workshop round trips, browser/native control journeys, movement/
combat comparisons, deterministic/headless/layer checks and 2/4/6/8-human workload
remain required. These prove motion and choices rather than a new minigame. Source
or targeted tests do not establish physical controls/performance/human acceptance,
or close 18/19 merely because a preparation or initial implementation commits.

## Adopted configured gravity response

Extend the actual existing LocalJump consumer with an authored finite nonnegative
`gravity_response` multiplier. Omission means 1, preserving the original drive's
arrival policy and original keyed scatter. A smaller value reduces that drive's
gravity displacement; zero removes that displacement, while actual collision and
representable-placement checks still apply. A larger value increases the effective
exclusion radius. Multiply each accepted static body's original exclusion radius
by the accepted drive response using wider finite arithmetic; do not mutate body
policy, historical evidence, the World roster or another drive's state.

Freeze the response in the same accepted drive profile as preparation, scatter
and fuel. Recompute the saved physical arrival from that immutable profile and
original static body recipes during whole-candidate preflight. Failed placement
retains the existing waiting phase and cannot partially consume fuel or payment.
The original default response is omitted from JSON, retaining the old stateless
scatter key for unchanged equipment. No additional arrival cache or RNG stream is
introduced. Workshop edits the same closed drive profile through its existing
accepted source/history transaction.

The concrete consumer is an optional gravity-hardened LocalJump Cruiser derived
from the existing LocalJump Cruiser, retaining its actual drive System, finite
fuel, energy, preparation, Console feeds and AI. Its provisional response is 0.25;
this is an equipment demonstration, not a balance claim or inter-system transit.
It must pass actual profile/placement/continuation and authoring checks before the
capability is considered verified. Formation and sector travel remain the
following consumers; their best-stat and actual World/content transitions cannot
be claimed from this local extension.


## Adopted first World transit consumer

Use the existing shared anchored World space and tracked World layer owner for
first inter-system travel. An authored closed destination ID binds one accepted
supporting-world path and one named finite arrival anchor from that accepted
composition. Paths, templates and script units pass the existing content closure,
composition, load and ledger gates; a player's command names the destination ID,
never an arbitrary content path. The untracked PendingScenarioLoad merge is not
a transit loader. Do not add a parallel universe, sector-coordinate registry,
transport, scene owner or always-running sector simulation.

Extend the same installed Transit System/profile/order machinery with an explicit
World destination domain. A local-only drive cannot accept that domain. The
concrete optional long-distance drive declares real fuel/preparation/energy and
its own accuracy/gravity response; it does not grant ordinary engines free travel.
The ship's permitted Navigation route view publishes the authorised destination
ID, label and arrival-point intent. That view does not reveal destination actors,
private gravity recipes or undiscovered mission facts. Human and ordinary drive
AI consume the same route and existing System-addressed admission.

World resolves the authored binding and materialises the supporting world through
its existing tracked layer load, compiled AST, origin/lifecycle and deterministic
minting path. The existing order retains its accepted binding while preparation
and loading proceed; it waits truthfully if required content or physical placement
is unavailable. The ordinary World layer queue is the only pending load owner.
Repeated input/restore cannot mint a second layer or charge fuel twice. Load/parse
refusal leaves the actual ship at its source with no indivisible fuel commitment.
Extend original static gravity provenance from accepted base rows to accepted
tracked-layer rows: preserve the owning path, immutable authored recipe and actual
fixed pose through the existing content/snapshot binding. Do not infer gravity
from visual scale or introduce a mutable body registry.
At the fixed handoff boundary, require the accepted layer, original static gravity
inputs and representable physical arrival to agree; commit actual finite fuel and
energy and place the same ship once. World transit arrives at rest in its selected
navigation frame with heading retained, matching the explicit first LocalJump rule.
No Scenario/HTN action directly writes the ship's physics or simulates a journey.

Keep existing actors, launched ordnance, carried craft, crew, stocks, damage and
accepted obligations in their actual owners. Loading a destination does not unload
the source or imply all fleet players departed. In the first consumer, leave the
source layer resident; subsequent unloading remains an explicit normal World
operation that must respect actual remaining actors and obligations. Return travel
reuses a resident layer and its real state rather than refreshing its stores or
restarting its scripts. This consumer does not introduce coarse offscreen state.

Publish own accepted route, loading/preparation/commit/arrival status and a typed
actual arrival receipt through existing observations/reports. Receipts identify the
accepted destination and actual completed order, not a predicted arrival or foreign
success oracle. This is the join available to the authored continuing modes and
mission planner. Preserve existing World layer/script/content and Transit snapshot
owners; preflight the complete candidate binding and roster before any write.
Actual two-destination departure/return, finite conservation, no-write refusals,
fresh JSON 360-step continuation, multiplayer/local-ship symmetry and Workshop
accepted history/Preview/Test evidence are required before claiming this consumer.
Formation remains a later consumer of genuinely received and acknowledged Comms
orders, preserving every participant's own authority, readiness and paid equipment.


## Adopted first received formation request

The first closed `FormUp` request carries a two-integer slot offset in the existing
World x/z plane. Its leader is the genuine sender of the physically received
Comms request, not an arbitrary UUID supplied in its payload. Existing reading,
Human choice/ordinary AI decision and real outcome delivery apply. Comms accepting
the request records coordination intent; that receipt does not claim Navigation
accepted a leader, Helm engaged, the ship reached its slot or a jump is ready.

Navigation accepts only under its own ordinary System/source/rating authority.
A Human explicitly selects an earned own Broad contact handle as the leader.
An ordinary Navigation AI can correlate the received sender callsign/identity to
independently matured observed identity/name and genuinely held faction/reference
evidence only when the result is unambiguous. The private Broad source-route map
is never association evidence: matching a hidden actual UUID to the Comms sender
cannot select a contact or prove identity. Missing or ambiguous correlation leaves
the request pending with an honest reason. An explicit supported information feed
carries only genuinely received formation suggestions into Navigation; co-hosting
Consoles grants no additional knowledge.

Navigation owns the accepted finite slot intent and its held request/leader
reference. Helm owns whether its ordinary assistance executes that intent.
Human Helm retains control until it engages assistance; AI Helm makes its ordinary
own-System decision using permitted onboard intent. Every machinery command still
passes the existing owning System admission. Shared screens may combine controls
only when their operator actually holds each required authority. No Comms action,
remote sender, plot callback or planner writes another System's physics directly.

Execution uses the receiver's current earned leader position/velocity measurement
and supported own motion limits. The first slot is in World axes; no unobserved
leader heading or guaranteed hull-relative orientation is invented. Missing/stale
measurement pauses the same accepted intent and reports its actual reason; it
does not query the actor's private pose. Manual Helm intervention breaks away
through the existing control handoff. Physical holds and collision/acceleration
limits can make the slot infeasible. No snapping or perfect station keeping.

Local status distinguishes received, coordination accepted, leader selected,
awaiting Helm, executing, paused and broken away. Source knowledge of those stages
requires actual ordinary returned Comms reports; an accepted request is not a
foreign readiness oracle. Formation jump membership/readiness subsequently binds
these real acknowledgements and each member's own paid drive, with best admitted
range/accuracy/gravity stats and one shared reference displacement as already
specified. This contract alone implements neither movement nor formation travel.


## Adopted current World policy binding

Keep one small canonical `TransitWorldBinding` per actual drive-equipped ship:
Base, or an accepted authored destination binding. It records the current arrival
policy/route context only. The same successful physical World arrival updates it;
preparing, refused, cancelled and merely accepted requests cannot overwrite it.
A local drive uses this current accepted World and its relevant static supporting
recipes rather than silently reverting to base-world gravity after sector travel.
Dormant prefetched masses do not act as physical global obstacles. Original body
positions, visibility, removal, physics and ordinary World layer lifetime remain
with their existing owners.

Seed from actual authored placement: base actors start Base; a layer-authored
actor must map unambiguously to its accepted destination context or refuse the
unsupported drive binding. Preserve the existing EntityOriginLayer lifetime and
script ownership; travelling does not reparent an actor into an unload list or
rewrite its AI flag chain. The new binding is not a physical-space identity,
interaction filter, second sector simulation or a substitute for World membership.

Capture/fold and whole-candidate preflight validate the binding against accepted
content, actual supported source and the same Transit owner. Cross-check completed
World arrival records where present. Restore cannot infer it from current render
content or whichever file loaded most recently. Departure/return and subsequent
local-jump witnesses must exercise the actual selected/current World recipe scope,
including relevant static supporting worlds, before claiming that joined exit.


## Dormant World content checkpoint

Closed root `transit_destination` rows name an ID, strings key and Base/Layer
arrival-anchor binding. Activate ingestion, browser dependency discovery and
Workshop acceptance share the existing supporting-content path: route targets
and their declared static support are read and validated before the content
freeze. This prefetch does not enqueue startup activation. Ordinary legacy
`extra_worlds` keep their previous one-level boot discovery.

Transit support discovery deduplicates paths, preserves the deterministic first
declaration's loader ancestry and terminates cycles; existing Workshop cycle,
duplicate and dangling-reference rules still refuse invalid authored graphs.
Disposable Test checks the selected support and sibling-script closure too.
Technical bounds are 128 route rows and 128 destination/support Worlds, with
canonical bounded paths. Supporting files cannot redefine the root catalogue;
selected anchors must exist in their own World, and simultaneously resident
route actors need distinct authored names under the existing name mapping.

This is a structural source checkpoint. It adds no playable sector travel,
physical placement, activation policy, new authoritative location, current-World
state, route controls or simulation evidence. Those remain the next actual
Transit consumer; native owner tests are integrator-owned and pending.

## Adopted first Navigation route selection

Use the existing paid Navigation waypoint command to select an exact published,
permitted authored route anchor. A World-capable drive freezes the unique closed
route ID, point and Navigation generation from that reading. Matching a guessed
private anchor against a private recipe is insufficient: the route must also be
in the same permitted Navigation publication. Ambiguous/shared coordinates are
refused with an actionable reason. This form adds no second waypoint owner or
route-selection command; ordinary Navigation can use its existing authored
Reach-to-anchor policy to select the same course. Free courses remain local.
Derive immutable accepted route/body recipes at the existing world materialisation
boundary; the same actual Transit state retains current World context and owns
paid work/arrival. No private destination actor or file path enters the route UI.


Every accepted order freezes its original World context so retained completed
orders on other drives keep their original local gravity scope. The existing
ship Transit owner retains one latest successful World-arrival stamp (owning
System, closed route ID and actual arrival tick). This is a causal receipt,
not another phase, payment ledger or physical-space owner. Only that latest
frontier determines current World context; with no such arrival the context
must match actual authored placement. Inactive historical arrivals do not all
claim to be the ship's current World.

## First physical World-route consumer checkpoint

The optional sector Cruiser extends the existing actual Transit drive. Known
routes are published on its permitted Navigation board and selected through the
existing paid waypoint command. World-capable drives freeze the closed route,
full accepted point and Navigation generation; no private anchor lookup grants
knowledge. Free courses stay Local. Existing paid preparation, local fuel,
commitment and next-tick physical arrival are retained.

The ordinary tracked World layer queue loads destination/support parent-first.
Immutable accepted recipes select relevant static gravity evidence, while the
same Transit owner retains current World policy, original order context and the
latest successful World-arrival stamp. Source actors and scripts remain resident;
travelling never changes EntityOriginLayer. Pre-mint validation refuses ambiguous
or unsupported layer drive origins. Snapshot preflight retains exact installed
config/cost checks and validates source/current/frontier/route before writes.

Independent source review covered knowledge, paid ownership, historical drive
contexts, loader ancestry, pre-mint refusal and GUI/Workshop controls. Targeted
JavaScript: 35 tests in six files passed; strict strings: 6311 entries, no errors
or warnings. Rust formatting and source review are not native execution. The
new actual-host proof exercises paid departure, selected supporting gravity,
subsequent local travel, return/revisit without restarting scripts, valid layer
preparation before JSON restore, full 360-update capture/digest comparisons and
five malformed no-write candidates. Its native result is integrator-owned and
pending. Renderer/native human acceptance is also pending.

This checkpoint does not close increment 18. Remaining work includes the actual FormUp Navigation/Helm consumer, formation
transit and final multi-seat/native acceptance. The basic World-route form is
recorded in the authoring checkpoint below. No new network or save
mechanism, campaign progression or background sector simulation is introduced.

## Basic World-route authoring checkpoint

The existing Workshop spatial panel now includes a structured route catalogue
form. It edits actual selectable draft roots, chooses actual accepted target
Worlds/finite anchors and writes closed Base/Layer bindings. Dependency targets
remain read-only. Addition, leaf updates and removal use the existing runtime
TOML editor inside the same accepted-source validation/history transaction.
Undo/redo, export/reload and disposable Test therefore consume the same source.
Duplicate IDs or x/z arrival choices, unsupported paths and stale/disposed edits
refuse without changing accepted history.

Independent source review passed. Focused route/spatial JavaScript: 23 tests
in four files passed; strings 6322 clean and layer checks passed. A real Rust
source-editor regression covers existing array-of-table rows, leaf comment
preservation, insertion, removal and first-row creation; native execution remains
integrator-owned. This source checkpoint closes the missing basic form, while
actual browser/native Workshop Test and the remaining increment-18 movement
acceptance are still open.

## Adopted formation course and axis owners

Use a Formation variant in the existing NavigationWaypoint, retaining the actual
heard-and-accepted request reference, earned public leader handle and dated
measured slot cache. Regular observation refresh keeps its semantic generation;
a new/cleared course changes it. Stale or absent motion retains the request but
suppresses execution. Private actor pose and hidden source identity never fill
the gap. A received-formation candidate joins the ordinary authored Navigation
selector without a mission-objective source score; receipt alone cannot override
its mission priorities. Human Navigation uses its own paid admitted selection.

Each existing Helm axis may retain a follow demand bound to that exact course
generation. Human Helm explicitly enables it through the HUD; individual Helm
AI decides after the ordinary clearance handoff. Manual axis commands replace
that demand normally. The current mechanical axis consumer and paid integrator
remain the actuator path. Existing snapshot/digest/preflight definitions extend
these owners, rather than adding a parallel goal, fleet controller or physics
store. Contract adoption alone is no execution evidence.

The first native source-editor run exposed a mismatch concealed by the initial
JavaScript adapter: TOML value-array Insert cannot append [[array-of-table]]
rows. Route addition now uses the actual existing AppendTable operation. That
consumer preserves both array-table and inline-table-array representations,
including its existing duplicate-rating refusal. Target updates use exact leaf
edits so nested and inline target tables both remain supported. Native source
regressions retain their source/comment assertions and additionally cover these
representations; the corrected native run remains integrator-owned.


## Formation Navigation source checkpoint

The existing Navigation course now carries a genuine accepted received request,
an earned observer-local leader handle and a dated measured World-plane slot.
Own Navigation admission and actual discrete reserve work select it. The
ordinary selector can rank the same candidate without mission-objective score;
AI identity correlation refuses missing or ambiguous matured public names.
Current paid observations refresh its cache without changing course generation.
Information or funding loss pauses execution and retains the dated intent.

The optional formation_cruiser preserves Cruiser equipment and crew seats. It
adds an explicit Comms feed to Navigation and uses its existing TacticalRadar
motion feed; the ordinary authored selector weights mission goals above the
received coordination source. Existing Workshop Console/feed and AI-selector
editors author these same definitions. The shared Navigation controls expose
only current received requests and earned aliases, with exact own-Human
semantic rechecking. They never send a sender route UUID or supplied position.

Canonical capture/digest retain the course. Whole restore validates candidate
Comms provenance before writes; single-row rewind checks the live inbox instead.
A dated cache cannot contradict a retained canonical sample at the same sample
tick. Carried actors retain historical intent while their operational updates
are disabled. Ordinary Comms dismissal retains structured request lineage.

Targeted JavaScript passed 21 tests across four files; strings 6338 and layer
checks passed. Independent UI source review passed. The actual owner witness
uses paid radio, separately paid Broad acquisition, own paid selection,
ordinary Navigation AI, fresh JSON restore with an empty bootstrap inbox,
malformed no-write candidates and ordinary Comms dismissal. Its native execution
remains integrator-owned. Human Helm follow demand, completed physical formation
movement, full-host JSON continuation and device acceptance remain open; this
source checkpoint does not close increment 18.


### Explicit Helm following checkpoint — 2026-10-16

The existing actual thrust, steering and lateral intents can retain a selected
Navigation generation. The explicit HUD action follows only a permitted typed
Navigation course, with its original earned position/velocity sample and actual
configured age bound. It shares no private routing identity or leader heading.
Current-tick inputs are derived; missing, stale, withdrawn or unpaid Navigation
holds ordinary braking/neutral actuation under the existing paid limits and
inertia. Manual input replaces that axis's intent. Ordinary per-System AI can
request the same demand only through its normal travel verbs; combat/pass,
docking, hazard and Weapons-bearing decisions retain their ordinary precedence.
The existing cadence-held motion plan carries its generation in its existing
snapshot row and an empty-when-unused UUID-sorted digest contribution.

Targeted client tests cover current authority, stale HUD activation, closed wire
payloads and cancelling following. Pure and owner Rust witnesses are authored
for dated current inputs, manual breakaway, real paid movement, fresh restore,
future-generation no-write refusal and AI verb precedence. Native execution and
physical multiplayer/browser acceptance remain pending; this checkpoint does
not close increment 18 or claim a completed fleet formation scenario.


The restored Helm HUD rebuilds its availability from the same permitted shared
sample. The existing held-publication restore owner preserves a positive view
only when it equals that projection at the saved counter or the preceding
completed publication frontier. Empty held availability remains withheld. This
never restores the mechanical current-tick input cache. The exact configured
sample limit and first stale publication have a focused genuine-course source
witness; native execution remains pending.

`tests/formation_continuation.rs` adds a separate full-host source witness: a
joined Human leader selects an actual hailable contact and transmits paid FormUp;
ordinary backfill reading, acceptance, independently earned identity/motion,
Navigation clearance and Helm travel must produce real actuator work. It then
compares complete fresh-JSON captures/digests for 360 ordinary updates, including
an actual Human source turn, checks two malformed generation candidates without
writes, and requires continued displacement from the paid saved checkpoint.
This is authored test source until the integrator runs it, with no device or
multiplayer acceptance claim.

### Joined Helm checkpoint — 16 October 2026

The Formation consumer is integrated on the prepared media/capture baseline.
Canonical movement intent, existing held motion plans, current-tick derived
inputs and validated held HUD publication frontiers retain their original
owners. Follow commands use the actual three authored movement kinds at the
normal admission boundary. Empty/unbound plans preserve their prior digest;
the exact added empty stage is characterised alongside the existing namespaces.

The existing `HelmMotionPlan` resource is now declared `DeferredFold` rather
than left on the unclassified baseline: only `navigation_generation` is newly
folded. Its legacy held motion/hazard/docking fields remain the prior fold gap.
This checkpoint does not claim a whole-resource hash or introduce a second plan.

The replayed runtime/publication/snapshot joins passed independent source review.
Two targeted client files passed 23 tests (1.16s) after updating the exact action
catalogue expectations for four already integrated media/personnel entries.
Layer rules passed; strict strings checked 6,427 IDs with zero errors/warnings.
Native compilation, owner/JSON360 execution, compatibility versions and integrated
gates remain with the parent integration thread. No Cargo or device test ran here.
