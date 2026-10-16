# Increment 15 — bounded physical marking payload

Status: adopted implementation contract, 16 October 2026; no implementation or
playable-exit claim. Prepared against `efcddb08` and reviewed through `b5d4faba`.
Root adopts the bounded contact, attachment and paid-report contracts after
independent Craft and Comms source/API review, before runtime implementation.

## Selected scope and open choices

S012 option A in `system-review.md` and the accepted physical-payload decision in
`pasm/spec/design/layered-combat.yaml` select physical observation-producing
beacons. Increment15 requires finite vulnerable equipment, actual subject-relative
placement, paid operation and a working report link. It does not select a generic
accuracy bonus, hidden interior access or permanent perfect tracking.

Exact measurements, endurance, link, removal and countermeasures were left open.
The bounded choices below are selected implementation contracts, not quotations
of settled tuning. Equipment-specific penetration is separate. General mines,
terrain manipulation, Bay handling, recovery and cleaning are outside this slice.
No new authored entity enters the global catalogue before an actual accepted
`resolve_from_disk` → `EntityConfig::from_toml` regression accompanies it.

## Existing owners and missing joins

| Responsibility | Existing owner | Proposed extension |
|---|---|---|
| Finite round and loading | `weapons/torpedo.rs`, actual magazine Stock and tube/burst | Optional mark-only delivery frozen with the round; no beacon inventory |
| Interception and impact | Existing PDC/torpedo lifecycle, stable collision order | Surviving actual contact can create one body; interception cannot also deliver |
| Equipment installation | `entities/spawner.rs` and `ship_spawn.rs` | Use the same accepted configured entity installer |
| Runtime reconstruction | `EntitySpawnOrigin`, template/content ledger, snapshot preflight | Share accepted runtime equipment provenance, rather than pretending a script spawned it |
| Pose | `ship/physics_systems.rs` physical synchronization and existing corrections | One explicit rigid-constraint consumer for attached bodies; no independent motion integrator |
| Paid measurement/history | `science/broad.rs`, actual BroadTracks | Restrict this instrument to its attached subject; retain existing error, cadence and provenance |
| Report and receipt | Actual Radio payment/route; existing radar sample export/receipt | Optional paid radio-backed export of original samples into existing recipient BroadTracks |
| Damage and endurance | Actual ShipPowerSystem and ShipDamage | Ordinary fitted local batteries, Operation and compartment damage |

`physical_probe.toml` demonstrates ordinary finite equipment. It is not a marking
consumer: its accepted profile is fixed, it has no attached location state, and
`battery_probe.toml` has neither a Radio nor a sharing configuration. Carried
custody deliberately disables active work and must not be used for attachment.

Existing `science/sharing.rs` preserves original sample identity and history and
checks range/jamming/equipment availability. It does not prove actual paid radio
work. Only the new opted-in report path gains the stricter physical Radio gate;
legacy sharing is not migrated by this change.

## Delivery, accepted recipe and real contact

An optional mark-only payload on the generic Guided delivery references an
accepted battery-equipment recipe. It is per actual tube, not a new global
TorpedoSystem mode. Ordinary controls, target selection, ammunition claims,
loading, barrel pattern, launch work, guidance and PDC remain their existing owners.
The initial optional reference fit changes one tube; shipped hulls remain intact.

Freeze the accepted payload recipe identity and actual launch faction with each
committed burst/round. Source removal cannot erase the payload or silently change
its equipment/faction. A frozen profile containing a recipe reference may require
Clone instead of Copy; do not replace it with a source-only lookup that fails
after launcher loss. Follow the actual accepted dependency through native/browser
preload, pack validation and frozen content closure. A runtime file lookup or a
private JSON merge is not accepted equipment construction.

First eligible subjects are ordinary planar physical ship hulls with supported
collider geometry and ordinary shield arcs. Asteroids, invisible markers, missiles,
Carried/Disabled bodies and another attached beacon are not attachment subjects.
Actual collision eligibility, not the launch target UUID or allegiance, chooses
the struck subject. Friendly/civilian impact has the same physical consequence.

The marker has zero damaging/EMP payload and zero proximity detonation radius.
The first consumer explicitly models a point marker's contact at the current
Damage boundary. It does not claim a swept first-impact or continuous collision
solution. The actual lifecycle owner enumerates live supported hull colliders;
TorpedoTargetSnapshot's UUID-sorted World radii are neither physical shape proof
nor a required prefilter for this path. Missing/undersized coarse World radius
must not prevent a real contained point reaching the exact query. A path crossing the hull between
sampled endpoints can miss under this bounded model. Zero radius alone proves
nothing, and no explosive near-miss can become a delivered marker.

Require the actual surviving round's current point to satisfy live
`Collider::contains_point(translation, rotation, point)`. Supported shape is an
actual same-body, parentless Ball, Y Capsule or Y Cylinder with
`ColliderScale::Absolute(ONE)`, no substituted shape/config/bounding radius.
The translation is current ShipPhysics x/y/z and rotation is exactly
`Quat::from_euler(YXZ, -yaw, 0, roll)`, matching physical synchronization.
Visual/model/LOD Transform scale is not collider scale and never inflates this
query. Missing/unsupported collider, nonfinite input or outside endpoint gives
no attachment. This is a real point/solid intersection at the sampled boundary,
not an inference from target intent, a cached World pose or an unearned report.

For an intersecting endpoint, `Collider::project_point` with the same pose and
`solid=false` supplies its nearest real boundary projection. Require finite
output on that accepted shape; this is the attachment contact point, not a
precise component position. If several actual hulls contain the endpoint, pick
least endpoint-to-boundary distance then UUID in the existing lifecycle's
stable round order. Retain that world contact as subject-local offset. Current
shield arc uses the round's actual incoming bearing through existing shield
routing. PDC/interception has already run; ordinary explosive rounds keep their
existing proximity behavior. No new projectile owner or general CCD is added.

### Sole lifecycle frontier and layer boundary

The existing Simulation `console/weapons/torpedo.rs::tick_torpedo_lifecycle`
remains the only terminal flight/impact/expiry owner. Within its existing stable
owner/round walk, the frozen mark-only profile selects the exact live-shape
predicate instead of Gameplay `TorpedoSystem::find_detonation_hits`' coarse
proximity predicate. The pure proximity finder must explicitly skip mark-only
rounds: a coarse match outside the actual shape does not consume, destroy,
detonate, spawn or stop that marker. It continues through ordinary guided
advancement and eventual exact impact or expiry. Unmarked flight retains its
existing proximity path and outcome ordering.

Only after the private Simulation shape query supplies a real contact does the
same existing `TorpedoSystem::handle_collision_full` removal frontier consume
that exact surviving round and return its frozen payload. A value-only private
contact result carries target identity, world surface point and incoming bearing
to the existing lifecycle settlement/spawn branch. It is not stored as a second
flight state or published as a controller observation. Stable UUID tie order
applies inside physical contact choice; ordinary source ordering is preserved.

Gameplay owns frozen payload data, motion, ammunition and single-round removal;
it imports no Rapier/Simulation scene API. Simulation owns the actual Collider
query and accepted runtime equipment installation. Neither private physical
geometry nor raw subject route enters AI inputs, Console controls or permitted
reports. Preserve the existing launch-origin/source exclusion and actual
surviving collision interval, PDC-before-impact and contact-before-final-expiry
ordering. No new arming timer, early source lookup or projectile controller is
introduced. Unsupported geometry contributes no marker hit; it is not replaced
with a radius, free beacon, refunded round or implicit World action.

### Local dependency/API proof

The locked dependency is bevy_rapier3d 0.33.0 (Rapier 0.31.0, Parry 0.25.3),
already used by Simulation. Its local `geometry/collider_impl.rs` exposes
`contains_point` (line754) and `project_point` (line718), both querying the
actual `Collider.raw` with a supplied translation/rotation; neither requires a
new backend broad phase, renderer or package. These APIs are the exact selected
point-contact and boundary-projection consumers. The cache source pin is
`C:/Users/jkeyw/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` with the
locked package/version. Line references describe that locked source, not a new
remote dependency.

Repository `phoenix-model/src/entity/hull.rs` defines the three accepted shapes.
Simulation `entities/spawner.rs` constructs the actual Ball, Capsule(length/2)
and Cylinder(half_height) and pins Absolute(ONE). Gameplay
`ship/physics_systems.rs::sync_ship_position` supplies the same physical pose;
Physics SyncBackend/Writeback precede the torpedo Damage boundary. Therefore
this query uses the current physical body's actual shape/pose, including planar
banking rotation, while ignoring presentation-only scaling. Tests must pin the
actual Capsule/Cylinder altitude/cap failures that a broad bounding radius
would falsely accept, visual-scale independence, missing/undersized coarse radius,
continued marker flight after coarse outside-shape matches, current-pose movement
and full-yaw/roll attachment/saved-projection consistency.

A currently raised shield arc stops the marker; it consumes the actual round
without spawning equipment or applying damage. A successful unshielded contact
consumes the surviving round once and places one real body at that actual contact
point with a finite subject-local offset. Detonation, interception, expiry and
restore cannot deliver it again. Unsupported/missing recipe or subject refuses
attachment without a substitute body or resource refund. Marker identity is a
deterministic child of the consumed round, not a second mutable ID ledger.

Install the real body with ordinary Systems, Consoles, accepted energy, damage,
interior, faction and runtime spawn provenance. No station is added. The accepted
recipe is engineless and crewless, has no Dock/Tractor/Bay/fitting actuator, and
contains only its supported SensorRadar and Radio equipment. No second stock,
controller or player/AI simulation variant is introduced.

## One physical constraint owner

An explicit accepted attachment profile permits one canonical location state:
Attached(subject UUID, subject-local offset, attached tick), or Detached. Only
physical contact creates Attached. It is mechanical state, never a public target
handle, observation or machinery command. The local three-dimensional offset is
computed with the inverse of the subject's full existing physical rotation
`Quat::from_euler(YXZ, -yaw, 0, roll)`, and projected with that same rotation on
every update. Thus banking/roll cannot slide the beacon onto a different hull
surface or disagree with the actual collider. It adds no capital-ship pitch,
vertical Helm control or independently chosen child pose. Parent planar motion
and its existing physical banking remain unchanged.

The existing physical pose-synchronization owner gains a reusable rigid-constraint
projection. For an attached body that projection is its sole pose writer:
there is no Helm integrator, low-LOD motion, Dock/Tractor writer, independent
velocity integration or thrust. ShipPhysics/Transform are the physical projection
used by current collision, sensing and presentation consumers, not another
independent position intent. Captured attached ShipPhysics is a checked projection
of the candidate relation and subject pose, not independent canonical intent;
it becomes ordinary retained pose only on actual detach. Child motion cannot
move the subject or spend free propulsion on its behalf.

The same projection must run at the physical synchronization boundary before
Rapier SyncBackend, and after existing Modifiers pose corrections (including
TickDock) before measurements/reports use the body. This is one shared physical
constraint algorithm at the two required boundaries, not two body integrators.
The exact integration seam and sanctioned writer/census change need Root/Craft
review. Reordering the entire existing physics/damage/Dock graph is not implied.
If this cannot be joined coherently, record that dependency rather than adding a
second independently scheduled pose writer or measuring from a different pose.

At contact, the real beacon can touch or overlap its subject collider. Exclude
only this exact attached mechanical pair from physical contact/solver response
and from `server_app/collision.rs::handle_collisions` damage/de-overlap. The
existing Physics registration must gain a narrow relation-aware pair filter;
there is no existing attachment filter to assume. Join that filter and the
ordinary collision consumer before enabling contact spawning. The concrete seam is the existing `register_physics` installation of
`RapierPhysicsPlugin<()>`, changed to a read-only relation-aware
`BevyPhysicsHooks` SystemParam. Add `ActiveHooks::FILTER_CONTACT_PAIRS` only on
the beacon. Local dependency `pipeline/physics_hooks.rs` exposes actual entities
through `PairFilterContextView::collider1/collider2`; return None only for its
exact current attached subject, and `Some(SolverFlags::COMPUTE_IMPULSES)` for
other pairs after existing groups/ActiveCollisionTypes. Non-beacon pairs never
invoke this optional hook. The ordinary collision consumer reuses that exact
pair predicate, including any existing retained contact pair. No shape-query,
weapon ray or global collision group is removed. The exclusion
ends on actual detach and grants no immunity against other bodies, weapons,
ray queries, compartment damage or detection. Prove prolonged attachment causes
neither recurring parent-pair damage nor solver forces, while an actual third
body/weapon can still damage the beacon through normal consumers.

Actual subject removal detaches the real body at its last physical pose and
withdraws new subject-only measurements. Missing observations or a broken radio
link never mean the subject was removed. Detached equipment stays an ordinary
vulnerable finite body; it does not acquire a new target, free drift or recovery
mode. Beacon destruction follows the normal actual removal path. No lifetime
counter duplicates energy: exhausted equipment remains inert physical hardware.

## Limited observations and actual Radio reports

The real SensorRadar pays its existing continuous work and respects Operation,
actual range/coverage, signal attenuation, uncertainty and sample cadence. Its
first supported question is attached-subject position and observed velocity.
Restrict its physical measurement input to that subject; no surrounding contact
sweep, identification, faction truth, assessment, component coordinates, interior,
shield frequency or live private UUID is granted to a controller or report.
Normal contact handles, maturation, age and confidence apply. Two real samples
are needed for observed motion. The attachment does not fabricate observations.

An ordinary same-faction compatible recipient SensorRadar can deliberately receive
the original measured samples through its existing sharing intent, BroadTracks
and explicit Console feeds. This is information only. It cannot select a target,
retarget a missile, illuminate, issue an order or grant control. Routine emission
needs no per-beacon polling; the ordinary sensor/radio controllers remain the
only controllers of those Systems.

Selected exact report frontier:

1. TickCommsRadio pays ordinary listening once. Expose a derived receipt keyed
   by exact actor/System and current tick containing the **raw paid fraction**,
   before Operation multiplication. It is cleared on restore and absent means
   zero. `Endpoint.listening` already includes Operation and its point is sampled
   before later motion; neither is a safe late-report receipt. Do not infer
   payment from unrelated work or a range-only legacy endpoint.
2. After final constrained pose and actual Broad measurement in Modifiers, stage
   one newest original sample for the beacon's configured channel. Each new
   observed tick can emit once. Failed source work retains eligibility; no
   unbounded backlog or same-tick chaining is created.
3. Rebuild source availability from its exact current-tick raw receipt, actual
   current equipment and current Operation **once**, with its configured channel
   and keys. A ready source pays the normal discrete transmission cost once and
   emits/stamps its frontier even when there are no viable receivers. No receiver
   existence, range, key or coverage fact can gate source emission/payment or
   create an implicit retry oracle.
4. For delivery only, rebuild each candidate's real current physical pose and
   current Operation/availability from its exact raw paid receipt once; recheck
   compatible channel/keys, receiver listening selection and current physical
   route, jamming/protection. Missing payment withholds receipt, never falls
   back to legacy sharing. Cached pre-Dock coordinates and already-multiplied
   listening factors are not current pose/raw payment. Ordinary continuous work
   is not paid twice and Operation is not multiplied twice.
5. Deliver only to actually eligible receivers. Copy original sample identity,
   time/error/origin into their existing BroadTracks. Receipt or retransmission
   cannot refresh measurement age or turn a private subject UUID into identity/
   correlation evidence. Losing link/power stops new arrivals; retained history
   ages and expires normally. Emission success still advances one retained
   source frontier regardless of delivery.

The emission frontier belongs to this equipment's existing observation/report
owner. No transport changes, player message-capacity accounting, AI voice,
separate radio controller or second sensor history is introduced. Exact first
recipe cadence, battery/Radio costs and ranges are authored provisional tuning;
finite positive work and bounds are mandatory. Root/Comms review must settle the
shared paid-endpoint and report-order API before runtime, including S066 receive
coverage if present in the integrated Radio.

## Continuation, knowledge and authoring

Capture/fold the attachment mode/offset/contact tick, immutable accepted runtime
spawn origin and last successful report frontier. Existing owners capture actual
energy, damage and Broad history. Rebuild derived physical projection, permitted
views and current paid Radio receipts; restoration never creates a fresh report,
new consent or a fresh observation. Attached rows must agree with the complete
candidate subject pose/accepted equipment at both restore doors. Reject cycles,
self/missing/Carried/unsupported subjects, foreign payload origins, nonfinite or
out-of-contact offsets, future ticks and duplicate round-child identity before
writes. Detached history can legitimately outlive a removed subject.

Workshop edits the supported payload reference through the existing generic
weapon document/history workflow and validates the actual dependent equipment.
Reference closure, finite reserves, matching System/Console/energy/layout,
restricted sensing and compatible paid radio are preview/Test prerequisites.
Status and existing radar history display real own/received equipment evidence;
no global list of secretly attached beacons or owner-free controls is added.

## Required proof before a delivered claim

- Actual accepted parent/child loader regression, including complete inherited
  reactor/Console/interior equipment and frozen dependency closure; malformed
  recipes refuse before spawning anything into a global catalogue.
- Real finite Human and ordinary delegated round preparation/launch, genuine
  hull contact and one equipped body. PDC interception, shield-up impact,
  expiry and unsupported contact create no beacon; all stock/paid work conserved.
- Single physical constraint owner through steering, transit-compatible local
  motion and Dock/tractor corrections; no subject force or Carried-disable trick.
  Subject loss detaches at last pose and ends subject-only measurements.
  The exact attached pair produces no recurring collision damage/forces, while
  an actual third body/weapon still damages the equipment normally.
- Actual paid limited samples and compatible radio reception beyond the original
  observing ship's sensor reach; no identity/interior/aim bonus. Blocked/keyed/
  unmonitored/unpowered links withhold new reports while history ages unchanged.
- Complete fresh JSON360 capture/digest continuation through attached, blocked
  report, exhausted and detached states; actual launcher loss; forged relation,
  payload provenance/report-clock and candidate-pose no-write cases.
- Workshop accepted edit/undo/save/reload/validate/Preview/disposable Test and
  human activity evidence. Worker source review and owner checks remain separate
  from root Cargo/native/integration gates and physical device acceptance.

This contract changes no runtime, asset, balance value or compatibility epoch.
The missing physical/report joins are implementation requirements, not evidence
that an attached body or a working report path already exists.

## Draft review receipt

Craft's read-only physical contract review and Comms' read-only report contract
review pass after the contact-pair, checked-pose, raw-payment and independent
source-emission clarifications. These reviews endorse the proposed contracts;
they do not establish that an attachment filter or paid-report join already
exists. The subsequent local dependency audit pins available existing Collider
point/projection APIs for the explicitly bounded endpoint contact model; that
API choice has Craft's additional read-only source/API review against the locked
local dependency. Root adopts the contract after those reviews; actual consumer
proof remains pending. No runtime, native execution,
asset acceptance or full increment exit is claimed.


## Adopted first-slice physical presence boundary

The first runtime source review found an interaction with carried craft: an
already attached beacon would remain an active world body if its deployed probe
subject were later recovered. A missing observation or Disabled component is
not evidence of destruction and must never detach it for that reason.

Root adopts the bounded first-slice resolution before its runtime change:
exclude any actual ProbeCustody owner or accepted ProbeProfile from eligible
marking contacts and from attached-subject continuation, even while deployed.
Keep ordinary independent hull and facility contacts. Absence of current
Carried state is not enough: the body must not be carry-capable. Both actual
contact and complete-candidate/current-parent restore guards enforce this same
boundary. A later attached-stored-body physical-presence design can deliberately
extend it; that remains open and this slice introduces no stored-child custody
or cleaning operation. No generic all-target attachment claim is made.


## Isolated runtime checkpoint (not an integrated delivery)

`codex/marking-beacon-runtime` contains reviewed commits `d1b0cb48` (finite
accepted recipe/closed values; Personnel source review) and `e1a2ba68` (actual
frozen launch faction/recipe, finite rounds/bursts and coarse-contact exclusion;
Craft source review). Neither commit has worker Cargo/native evidence.

Later source is intentionally held as work in progress: exact endpoint contact,
ordinary equipped spawn, one physical projection at the pre-Rapier/post-Dock
frontiers, exact mechanical-pair filtering, actual-removal detachment, limited
subject-only Broad inputs and canonical continuation joins. Comms reviewed the
narrow current-tick raw Radio receipt seam; its late report consumer is not yet
implemented. Unsupported report frontiers are refused rather than accepted as
paid history. Snapshot validation separates disposable row-shape checks from
complete candidate or current-parent checks, and checks typed causal WorldId
namespaces/frontiers without adding a birth ledger.

Owned formatting and diff checks pass. The recipe chunk previously passed strict
strings (6,510 strings, zero errors/warnings). Rust owner tests, real accepted
loader execution, actual host tracing, continuation forgeries, collision/native
acceptance and final gates remain integrator-owned and unexecuted for this packet.
Implementation of the adopted carry-capable target boundary above, S066
foundation synchronization, actual paid limited report/provenance, complete dependent recipe/Workshop support and
ordinary Human/AI host proof must close before a coherent vertical-slice claim.
### Explicit sensor-to-radio knowledge binding

The finite marker's Radio Console explicitly receives its configured own
SensorRadar feed. Accepted-recipe validation requires that typed binding; a
missing binding refuses the recipe. This permits the selected reporting
consumer to read its own sensor evidence without pooling unrelated Console
knowledge. The accepted-loader positive and missing-feed tests are authored but
not executed in this worker. Independent source review passed; the actual paid
report consumer and the adopted carried-target guard remain separate unfinished work.

## Structural checkpoint for integrator foundation synchronization

The isolated branch now implements the adopted first-slice independent-hull
restriction at actual contact and both attached-parent continuation boundaries.
It also contains endpoint shape contact, ordinary finite body creation, exact
parent-pair filtering, the existing physical owner's bank-aware projection and
actual removal transaction, limited subject-only Broad acquisition, raw Radio
payment receipts, and canonical capture/fold/preflight/apply joins. `report`
remains `None`: unsupported historical report frontiers are refused until the
real paid provenance consumer exists.

Craft independently source-reviewed the physical/contact and removal tests;
Personnel reviewed the candidate/current-parent restore guards and typed causal
closure; Comms reviewed limited sensing and the once-paid raw Radio receipt.
These are bounded source verdicts, not execution evidence. The focused new owner
tests are authored but not run here. Explicit owned-file Rust formatting and
diff checks pass. Root separately ran the actual accepted-recipe tests against
the integrated recipe and knowledge-binding donors: two tests passed at
`1062d545`, recorded in
`.phoenix/resumed-charted-and-marking-accepted-owners-retry.log`.

This checkpoint permits the integrator to synchronize a clean tracked branch
with current S066 source without stashing work. Paid late report delivery,
historical report provenance, complete used-template dependency acceptance,
Workshop payload authoring and full actual-host continuation remain unfinished.
No complete marking consumer, generic carry-capable target support, native
physical acceptance or increment15 playable exit is claimed.

## Late paid-report source checkpoint

The next isolated source slice adds the adopted report consumer. Its private
current-endpoint adapter rebuilds physical poses, exact Radio control and current
Operation from the same-tick raw listening receipt. S066 receiver coverage and
the existing channel/key/range/path/jamming/protection route still apply; no
continuous payment is repeated. A fresh own bound sensor sample pays the normal
discrete transmission cost and advances its original sample frontier even when
no recipient can hear it. Missed samples are not queued or retried.

Delivery enters only willing same-faction SensorRadar track owners, through the
existing sample receipt mechanism after the legacy sharing batch. Private target
routes and identity/classification/assessment/observation claims are removed.
Original timing, uncertainty and measured motion remain the sample's existing
evidence. Neither delivery nor receipt grants a targeting association or control.
The retained frontier is checked against the candidate body's original sensor
sample token, allocated contact range and observation clock; aged-out samples
need no replacement history ledger and are never refreshed by restore.

Comms independently reviewed the current paid endpoints and report producer;
Craft reviewed the actual paid Radio/Broad/report owner tests; Personnel reviewed
the candidate frontier consistency guard. Owner tests are authored, not run in
this worker. The tests separate their initial physical attachment premise from
the still-pending whole-host flight/contact proof. Complete accepted dependency
validation, Workshop authoring, native execution and the full physical/report
continuation journey remain required before the vertical-slice exit.

## Accepted dependency source checkpoint

The optional `marking_cruiser.toml` reference changes only one inherited guided
tube's marker recipe and the hull name. Its actual resolved accepted-loader test
compares the complete canonical recipe with the base Cruiser after removing
those two intended differences, including real seats, Consoles, finite stocks,
energy, interiors, ordinary AI and the independent tubes.

Native pre-init discovery follows explicit marker recipes through its existing
parse/model-marker gate, including a child outside the initial hull directory.
Final native and browser acceptance validate each dependency against their
complete candidate cache. Missing or incompatible body equipment refuses before
play and cannot be satisfied from an older native global-cache entry. Runtime
launch/spawn remains cache-only. The existing native used-template identity walk
already follows `nested_template_paths`, as browser preload does; the added
ledger test checks the composed beacon digest without hashing unrelated assets.

Independent source review passed. These new accepted-launcher/dependency/ledger
tests remain unexecuted by the worker; native accepted-loader evidence must
precede the whole-host tracer. Workshop dependency-aware authoring and physical
native acceptance are still separate unfinished work.
