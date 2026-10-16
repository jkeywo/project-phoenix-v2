# Increment 19 — First moving utility shuttle

Adopted design preparation, 16 October 2026. Root approved the bounded mechanism
review in [the proposal](increment-19-moving-craft-proposal.md); the
[PASM decision](../../../pasm/spec/design/deployables-craft.yaml) records the
selected design. This specification introduces no runtime or completion claim.
**Implementation waits for the fixed turret/decoy native batch to pass.** Root
owns integration, compatibility versions, canonical census and Cargo execution.

## Playable outcome

An ordinary unarmed AI utility shuttle launches through the existing paid Bay,
receives a genuinely transmitted known XYZ waypoint, flies above the capital
ship plane using its own paid equipment, then receives Recall and approaches its
actual home for matched, paid same-body recovery. No direct Human piloting,
passenger transport, weapons or inter-system travel is included in this tracer.

The ordinary hull has accepted physical layout, damage, finite resources,
individual System AI and explicit Console feeds. Existing Full3D is XYZ
translation plus yaw. Preserve Planar/Bounded capital behaviour. Extend the real
Navigation/Helm destination consumer; do not introduce pitch/roll, a projectile
pilot, remote steering or a separate integrator.

## One owner for each fact

| Fact | Canonical owner and boundary |
|---|---|
| Actual body and XYZ velocity | Existing ShipPhysics and sole paid integrator; ordinary spawn must retain that installed owner rather than overwrite it with fixed-probe defaults. |
| Location and presence | Existing craft custody; Carried derives inactive simulation/backend/presentation through current owners. Deployed does not grant visibility. |
| Home association | An immutable actual launch receipt retained with craft lifecycle history: craft, parent, Bay ActivityJob and commit tick. It establishes identity only; no live parent pose or fabricated radar track. |
| Received instructions | Existing recipient Comms inbox, reading and decision history; no extra mailbox or receiving controller. |
| Accepted trip | One actual onboard Navigation-owned shared Job, minted only by its own admitted acceptance. Its activity binds launch provenance, accepted request and current leg. Recall redirects the same active Job; new unrelated trip after settlement has a new JobId. |
| Current navigation goal | Existing NavigationWaypoint and generation. The Job binds that generation and original accepted leg; it does not retain a competing target, transform, actuator plan or resource account. |
| Motion requests | Existing individual Helm axes under normal command admission, permitted Nav feed, priorities, machinery health and paid actuator limits. |
| Physical capture | Existing parent Bay Job and end-Physics commit, with new authored relative-speed feasibility. Actual custody changes once. |
| Remote progress | Only genuinely received returned Comms facts. Home's own physical Bay completion is direct local evidence. No host-private trip progress projection. |

The historical launch receipt is not a second location, actor registry or home
telemetry service. Launch alone creates no Navigation Job. Only a real successful
new launch replaces this provenance; an initially deployed body with no actual
launch receipt cannot fabricate a home or accept this voyage family. A recovered
craft does not refill, heal or lose its ordinary System AI memory. Retained old
request/receipt history remains historical and cannot start work while Carried,
reopen a settled voyage or bind a later launch. First accepted Travel, not Recall
without an existing accepted voyage, mints the Navigation Job.

Physical recovery by a later different owner, capture or a new operating faction
never rewrites the old launch/receipt history into present authority. New remote
acceptance rechecks ordinary actual authority and allowed relationship. Save
validation preserves genuine historical evidence without requiring the home or
original operator to remain alive/eligible forever. Actual removal and later
launches use their existing owners; no missing remote actor becomes a current
private destruction report. This tracer proves same-home recovery, not automatic
transfer of an active trip to a foreign facility or new commander.

## Launch report knowledge — approved owner refinement

The current `ship/craft.rs::BayState` retains one handling Job only.
`begin_order` replaces a terminal Job when the next real assignment begins;
`complete_receiving` retains a receiving handoff receipt on that one Job.
`entities/craft_view.rs::publish` projects that current Job and actual own stored
craft. There is no retained launch-result journal to reuse. A second launch or
recovery would therefore erase the first launch evidence needed for a later
Travel request. Polling the deployed child's current custody/association instead
would silently reveal foreign recovery/relaunch or loss.

Extend this same Bay owner with a narrow dated **own launch report**. Only its
real successful paid end-Physics launch transaction creates the immutable cause:
craft identity, home identity, actual Bay ActivityJob and physical commit tick.
That one event establishes the child's launch association and the parent's own
report copy together. The parent copy is retained knowledge of its own action,
not a second location, current-child registry, controller or pose account. Failed,
paused, cancelled, blocked or merely ready handling produces neither copy;
retry/restore cannot manufacture another launch event.

First technical retention is at most 128 reports per actual Bay, in deterministic
(commit tick, Bay JobId, craft identity) order, keeping the newest local report
for each craft. Only a newer successful launch by this same Bay can replace that
craft's retained report. Ordinary bounded oldest-report retirement may forget a
report; it does not cancel already emitted requests or accepted work, whose
existing original request/Job references retain their own continuation. This is
a technical knowledge-history bound, not ship message capacity or a launch cost:
a full journal never prevents physical launch or creates a resource reservation.
No distant recovery, foreign launch, missing contact, remote loss or controller
change edits or withdraws the parent's retained historical report.

The existing typed Bay publication exposes only this parent's retained report
under the actual Bay System key. A Comms/Navigation System sees it only through
an explicit Console Bay feed; sharing a seat or display grants no access. The
parent report can establish a known receiver identity and VoyageRef for this new
best-effort request family. It supplies no current contact, alias, position,
velocity, route, receiving power, delivery viability or future remote authority.
It never becomes a generic public contact or a hidden remote progress roster.

A source semantic action binds the exact currently permitted own historical
report plus the separately offered known XYZ Navigation point. The actual Radio
owner still validates/pays ordinary emission and physically delivers through its
normal path. Source success means real emission, never proof of reception.
Silence, receiver absence, link loss and later foreign custody cannot be used as
an implicit source-side delivery/eligibility oracle. A returned fact must still
be genuinely transmitted and heard. The receiver independently compares the
request to its own actual launch association/current allowed relationship and
separately admits its own Navigation; an old sender report is no actuator token.

Snapshot/digest includes the child's association and this existing Bay owner's
retained report history. Validate identities, original scope actor/owner/JobId,
finite ordered clocks, uniqueness and retention bound before either restore door
writes. Compare exact cause against the actual completed launch Job and matching
child association where those original owners are still available for that same
launch. Their later replacement/removal is legitimate history, not evidence of a
forgery or permission to authenticate an erased past. In particular a parent
report may outlive the child, while the child's genuine association may outlive
the home; no invented live lookup, reconstructed launch or global history service
repairs missing provenance. Existing accepted request/Job lineage remains checked
against its retained original cause when the local report has retired.

Required owner evidence includes two successive real launches replacing the
single current Bay Job while the first own report remains usable; exact no-write
failure/retry and same-transaction association/report creation; explicit-feed
isolation; parent report unchanged across actual remote removal/foreign launch;
newer same-Bay launch and deterministic bounded retirement; and fresh whole-save
continuation with a retained historical report whose original actor/Job no longer
exists. This contract refinement adds no runtime or Comms enum by itself.

## Closed Comms contract

These are intended new closed definitions, **not existing APIs**. Comms has
reviewed the vocabulary/ownership direction; exact Rust leaf integration remains
coordinated with its owner before enum edits. Keep existing Formation sequence
and facility ReceiveCraft semantics intact.

`VoyageRef` identifies `{ craft, home, launch_scope: ActivityJob, launch_tick }`.
The scope names the actual parent Bay Job whose successful physical launch
established this voyage. No entity can assert a voyage merely by choosing IDs.
The craft binds this reference to its own actual launch history, and the sender
must be the genuine launch-home identity. Those identities are provenance, never
an observation association or permission to inspect a private actor.

| Request | Immutable transmitted parameters | Receiving consumer |
|---|---|---|
| `CraftTravel` | VoyageRef; positive sequence; expires_tick; accepted waypoint identity and bounded `[i32;3]` World XYZ. | Own Comms reads/accepts the offer; own Navigation separately admits the actual leg and its shared Job. |
| `CraftRecall` | Same VoyageRef; positive sequence; expires_tick. No target position or current actor UUID selection. | Own Comms reads/accepts; own Navigation admits return intent using separately earned current home observation. |

Both are Contact-only sustained structured requests on the existing paid radio.
The issuing semantic action resolves an actual current permitted Navigation
waypoint choice, ignores injected caller coordinates and rechecks exact current
Human/AI System authority. The waypoint publication comes from an accepted known
authored finite XYZ point, not a hidden actor position or arbitrary content path.
The actual XYZ instruction is genuinely transmitted. The first wire uses
whole-unit `[i32;3]` coordinates bounded to plus/minus 1,000,000, exactly representable by the existing f32 motion inputs and compatible
with the closed request's Eq semantics. The new authored waypoint consumer
refuses fractional/off-grid/overflow points without rounding; existing other
Navigation routes retain their current precision. No infinite/NaN coordinates,
unknown IDs, private route paths or executable payload are accepted. Coordinate
and lifetime limits belong the supported accepted contract and Workshop validator;
operators choose the offered destination/Recall, not timeout numbers.

Sequence mints **at actual paid emission**, from existing retained source history,
scoped to home/craft/launch. Refusal or merely displaying a choice consumes no
instruction. Require original emitted_tick <= expires_tick, bounded positive
lifetime in the same fixed-tick domain, and no future emission. Receipt and
reading alone do not supersede Navigation. At actual local Nav acceptance, the
command must still be unexpired, match this launch and exceed the Job's accepted
sequence; repeated or delayed older messages have no machinery effect. Comms
Accept and own Nav acceptance remain distinct facts. A rejected/stale request
cannot clear the current course or reset work.

The current accepted command reference/frontier stays with the actual Job after
an inbox view is dismissed or retired. Whole-candidate validation must retain
sufficient genuine lineage to bind it; pruning cannot turn the reference into
fresh authorization. Accepted work continues after expiry or link loss under its
ordinary local constraints. An unseen Recall cannot alter it. Physical removal
has its existing actual event semantics; silence never proves destruction.

## Chart-first own XYZ consumer — approved reuse

Owner audit finds no missing authored waypoint registry. World Transit
`StaticTransitScopes::views` already publishes id/label/float-XYZ arrival points,
but their accepted destinations can bind other Worlds. Civilian `World.routes`
is a traffic anchor-chain owner, not a granted Navigation catalogue. Neither is
silently converted into local flight. Existing Navigation `knowledge::Picture`
already supplies current-World `charted` records with identity/name and full
public XYZ, after actual chart coverage, shows and concealment. Those records
are the first known spatial points. Existing planar shared observations do not
supply an unobserved altitude.

Extract/reuse the neutral id/label/float-XYZ point DTO shape while preserving the
existing TransitRouteView Rust API and serialized semantics. A separate typed
charted-points Navigation publication is derived only from those actual public
Picture records; it is not a second authored list, World registry, route or
history. Both capital ships and utility craft may hold this explicitly public
information through their own Navigation owner. Comms reads it only through the
exact explicit Navigation feed/source; no transit/civilian union, private anchor
lookup or cached World path is a known local point.

Use the existing SetNavigationWaypoint source-reference admission and
chart-contacts selector. Re-resolve the exact current public charted reference,
ignore injected caller coordinates and pay the normal actual Navigation
selection. An actually Full3D hull can select the real offered XYZ through a
charted spatial mode/cache on the existing NavigationWaypoint. Planar/Bounded
hulls keep their existing planar anchoring and can still offer public points for
later remote orders. Free XZ courses remain their existing separate planar
intent; they are never described as an earned spatial target. No new command,
selector source, authored point table, Job or pilot is created for this course.

The point's current public identity is the goal identity. Preserve the same
Navigation generation and ordinary clearance across legitimate refresh of that
point; only a new selection or clear changes intent. Retain original selection
and permitted publication frontier with the spatial cache. Losing a public view
must not chase private actor pose, substitute own/zero height or renew an old
sample. The accepted last legitimate fixed point remains known; future return
to a moving home remains the separate dated-measurement consumer, with missing
height/velocity pausing that work.

Only the ordinary shared Helm planner and its existing individual vertical axis
gain the cleared XYZ demand. Existing authority, AI priorities, hazard response,
paid actuator scaling, inertia and the sole physics integrator remain in force.
Actual own pose/tuning supplies braking; an unavailable actuator is not teleported
or assigned free velocity. This first course neither creates a voyage nor
fabricates launch provenance and can be proved before Bay admission exists.

Capture/fold the accepted mode, complete XYZ and original source/frontiers through
the existing waypoint owner. Both restore doors validate finite data, actual
Navigation ownership/Full3D capability, matching generation, exact original
accepted/public point evidence where available and consistent candidate permitted
publications before writes. Whole restore uses candidate knowledge rather than
the fresh host's convenient live chart. Restore rebuilds views without issuing
selection, work, clearance or a new observation; it does not refresh the retained
course frontier. Source replacement/removal cannot invent a fresh grant or turn
an erased past into authenticated knowledge.

Keep existing fractional World routes and public chart coordinates unchanged.
Only the later closed voyage wire converts its actually selected offered XYZ to
whole units within plus/minus 1,000,000, rejecting off-grid requests rather than
rounding or narrowing the underlying Navigation/Transit authoring grammar.
Required proof includes actual admitted own chart selection/ordinary AI, finite
paid nonzero-altitude movement, stable generation, missing/private reference
refusal, retained-source/no-write restoration and fresh whole JSON continuation.
No Bay or remote-voyage claim follows from an own-chart flight proof.

## Navigation, elevation and normal AI

Own admitted Navigation acceptance selects the real XYZ course and creates or
redirects its actual Job. Preserve ordinary mission/command priorities and Human
control: a Comms response cannot seize Nav or any Helm axis. New leg or explicit
clear changes course generation; ordinary measured refresh of that same return
intent does not repeatedly change generation or re-incur clearance lag.

Extend the current waypoint mode/published permitted view and shared Helm plan
with the actual XYZ destination consumer. Use existing own-body pose/velocity for
local control and normal current-tick paid input resolution. Return requires an
unambiguous current permitted public home contact, with elevation from the same
dated observation as planar position. Human selection uses an independently
earned current alias. Ordinary AI matches unique matured publicly observed
identity/name and legitimately held relationship/reference evidence from its
launch history, following the Formation correlation discipline. Launch UUID
alone never selects a private Broad source or turns its source map into identity
evidence. Ambiguous identity, a changed public claim or absent supported explicit
reference leaves return paused. Measured velocity is required for matching;
missing velocity is not zero. No private geometry supplies current motion.

Apply the instrument's configured age/uncertainty policy to the original sample.
Missing, stale or unsupported height/motion pauses and surfaces the local reason;
refresh does not renew unseen samples. Retain actual inertia: neutral/braking
requests consume ordinary available machinery and never erase velocity. The
ordinary vertical System AI receives the valid altitude/braking demand in its
existing decision path; hazard avoidance and supported higher priorities retain
their current owners. No extra pilot/state tree runs beside those Systems.

## Recovery and physical receipts

Add optional `maximum_relative_speed` to the actual Bay profile, alongside
existing recovery_range and maximum_parent_speed. Absence preserves legacy fixed
handling: the child must still have zero motion, and the parent may move within
its existing authored speed limit. Never default this new relative constraint to
zero. Existing probe_carrier and reception_outpost allow parent speed up to 0.1;
a stationary child and a parent creeping at 0.05 legitimately have relative speed
0.05. A new zero relative limit would reject that previously supported recovery.

Every supported moving profile requires an explicit finite positive relative
bound in its compatible Bay; omission cannot admit unlimited moving recovery.
With an authored bound, compare actual child/parent World-space XYZ velocity at
recovery admission and again at end-Physics commit. Use actual yaw-transformed forward/lateral velocity and
vertical velocity, not scalar speed difference or private observed guesses.
Physical handling may validate its actual bound body; that check is not published
remote telemetry or AI navigation knowledge.

Range is actual same-body XYZ distance within the accepted Bay range. Preserve
current source/Operation/fitting/access/space/resource and stopped-parent guards.
The same ordinary shuttle Helm approaches and brakes; no Dock pose writer is
installed for this consumer. Before commit its existing integrator owns motion.
Only the existing successful custody/capture transition settles stored motion
and derives inactive backend presence. Fast/far/unobserved craft cannot be
teleported into storage. Interruption pauses the same paid Bay Job; ready work is
not charged twice and cancellation/refusal follows current rules.

Closed voyage receipt facts identify `{ voyage, request: CommsRequestReference,
nav_scope: ActivityJob, course_generation, stage, event_tick }`. First supported
owner stages are NavigationAccepted, AtWaypoint, Returning and Paused.
NavigationAccepted proves actual local admission, not motion readiness.
AtWaypoint requires actual configured XYZ arrival/speed tolerances and the same
active permitted course, not stale evidence or generic Job completion. Returning
means the return leg was actually admitted; Paused is a supported local state
transition with a closed reason. Repeated transition identity comes from this
actual owner frontier; no parallel mission clock/store is introduced.

Use a distinct closed Comms request-facts extension. The existing coarse
CommsRequestStage::Completed dedupe is insufficient for later Paused/Returning
changes: emission and returned evidence must bind the exact actual owner event.
Normal paid outcome delivery may wait across outages. No receipt changes the
trip again or grants control, and no remote view shows it before actual delivery.
Captured/intercepted request projections initially expose the closed family only,
redacting craft/voyage/Bay/Nav identity fields just as other private references.

**Recovered is the home carrier's genuine local completed Bay event**, bound to
the same craft, launch, Nav Job and actual recovery Bay Job. It settles the
onboard voyage through that physical boundary. A Carried/Disabled shuttle cannot
transmit a fictional final radio message. Third-party recovery reports are later
supported consumers; ReportStatus, FormUp and facility receiving consent never
stand in for this handoff.

## Ordered implementation slices after native clearance

1. Pin leaf field bounds/accepted recipes and Comms/Navigation command vocabulary
   with the actual owners. Before any moving host tracer, execute a targeted
   native accepted-loader check through real include resolution and
   EntityConfig::from_toml for the complete actual authored recipe. Literal TOML
   parsing, a hand-merged JS model and source review do not establish acceptance.
   Atomic energy/interior/sensing/damage replacements must retain all required
   hardware; verify exact authored Console/System ownership after real synthesis.
   Add the strict unarmed ordinary Full3D consumer, its real accepted waypoint
   publication, existing Workshop validation and source editing. Do not broadly
   relax fixed-probe validation or permit unsupported propulsion by a Boolean
   marker.
2. Add real paid travel/recall send/read/decision lineage and own admitted Nav Job
   acceptance. Wire the same course/generation to existing paid XYZ Helm; prove
   finite above-plane travel before adding return handling.
3. Join genuine launch-home history, own earned return elevation/motion and
   ordinary approach/braking. Add relative-speed capture to the same Bay owner,
   with runtime and preflight mode-aware motion guards. Coordinate occupied-pod
   reception/personnel files with their owner; this first recipe has no berths.
4. Join exact owner-event facts/returned dedupe, working human controls and
   ordinary AI choices, current projections and actual Workshop Preview/Test.
   Integrate snapshots/digests with each state-bearing slice; no alternate save
   owner, backend state serialization or restore-time work.

## Evidence before a playable exit

Prove normal Human/ordinary AI launch and requests, actual paid nonzero-Y travel,
current earned home/elevation, matched same-body recovery and finite resource
conservation. Exercise real radio path loss, absent/stale height or velocity,
duplicate/delayed old Recall, real parent removal and interrupted handling.
Parent uncertainty and craft-local progress must remain independent.

Use independently booted whole-JSON360 complete captures/digests at partial
launch, moving flight, a real link gap, return and partial recovery. Validate
mode, installed profiles, voyage/request/sequence/expiry, Job/course generation,
same-body capture, presence and custody before any writes. Include a legacy
fixed-child recovery with parent motion below its existing 0.1 limit, and reject
a moving profile with an absent/nonpositive/nonfinite relative limit before work
or writes. Explicitly refuse fabricated launch history on an initially deployed body, old-voyage messages after
recovery/relaunch, Recall without an admitted trip, current generation/request
mismatch and invalid later-owner authorization. Preserve genuine historical
receipts after actual owner/home loss rather than retesting old eligibility.
Malformed candidates
must leave complete state/digest unchanged; topology/fixture mismatches cannot
serve as substitute refusal witnesses. Include another LocalShip observer,
actual authoring/history/Preview/Test and relevant browser/native journeys.

Root records actual native results separately. A source review, metadata compile,
scripted position change, bare API call or fake remote contact is not a flight
playtest or full increment19 exit. Armed craft, passengers, cargo sorties,
inter-system trips and wider 2/4/6/8-seat workload acceptance remain explicit
later work.


## First ordinary-equipment checkpoint — source only

The optional `assets/entities/utility_shuttle.toml` defines an ordinary unarmed
Full3D hull with nine AI-only Systems and five explicit no-screen Consoles. It
uses the existing shared ship installer and individual Helm/Navigation/Comms
policies, real finite local reserves, two ordinary Broad instruments and complete
energy/interior/Operation bindings. It adds no seat, pilot, propulsion owner or
projectile controller. Its reactor remains the inherited zero-generation source.

This checkpoint intentionally has no `[probe]`: fixed carried-equipment validation
and its existing zero-motion continuation remain unchanged until moving custody
and relative-speed capture are joined together. It does not demonstrate Bay
launch, a selected XYZ course, remote Travel/Recall or return. The shared ship
installer preserves actual authored height for Full3D while retaining previous
Planar/Bounded initialization; supplied yaw keeps the existing seed owner.

A real include-resolver → EntityConfig::from_toml regression pins the complete
accepted recipe, both scanner profiles, scope ownership, finite consumers and
unarmed layout. A second owner regression exercises the ordinary shared installer
at nonzero altitude/yaw, actual AI/profile installation and zero emitted work at
spawn. They are authored and independently source-reviewed; native execution is
pending root. Literal TOML parsing, rustfmt and diff checks are preparation only.
No host flight tracer is authored before the actual native accepted-loader gate.

The return dependency is a Navigation consumer of existing dated
`SensorTrackSnapshot.elevation`, not another sensor. Main already has actual paid
Broad altitude/error and sample-derived vertical velocity (81c649b2). Navigation's
legacy planar Picture currently substitutes observer height; that value must never
be used as target elevation. The later return join must preserve the original
sample tick/error and refuse absent/stale height or measured velocity without a
private home-pose lookup.


### Chart-first runtime checkpoint (source; native execution pending)

The existing admitted `SetNavigationWaypoint` now resolves an offered current-World
charted contact into a fixed `ChartedXYZ` course only on actual Full3D equipment.
The course retains its original Navigation owner, complete fractional XYZ point,
selection tick and existing generation. The normal selector uses the same mode;
withdrawal of the offer preserves that legitimate fixed point, without following
an actor or inventing a new observation. Planar/Bounded and free XZ orders retain
their existing behavior.

`NavigationPointView` extracts the existing `TransitRouteView` value shape; the old
Rust name, fields, validation bounds and JSON are preserved. Actual chart choices
are published separately as `NavigationBlackboard.charted_points`; selected fixed
intent is `charted_course`. `navigation::known::charted` reads only the explicitly
permitted actual Navigation source and never unions foreign Transit arrivals or
civilian lanes. Simple own course selection creates no Job or launch provenance.

The normal clearance, explicit Navigation feed and shared Helm planner supply the
existing admitted per-axis controls. Full3D vertical travel uses actual vertical
speed/acceleration tuning and the ordinary paid vertical operator/integrator;
hazard, docking and authored maneuver priorities remain in the existing planner.
No body pose writer, special pilot or player-facing vertical control is added.

The same canonical waypoint captures/restores the original selected point and
clock, with a Some-only digest contribution. Candidate validation checks actual
Navigation/movement equipment, finite point and clocks, generation, exclusive
mode, exact retained own publication and available original candidate chart point
before writes. A current chart withdrawal
does not reconstruct erased history or refresh knowledge.

Source tests cover actual accepted utility chart admission/payment, denied private
chart selection, explicit-feed isolation, fractional height and withdrawal; the
ordinary vertical axis and finite physical integrator; original JSON/application
continuation and ten malformed whole-candidate no-write cases (including missing
canonical/published course mismatches). These are owner/source
checks, not a full-host flight/360-step or browser acceptance result. Actual
native execution is queued through the integration owner. Mode-aware Bay custody,
launch association/history, transmitted voyage orders, shared Job lifecycle,
measured return and moving recovery remain separate required joins.


### Actual local chart flight witness (authored; native execution pending)

The integration lane passed the real chart-selector utility recipe and pure
movement owner gates at `1062d545`. The new
`tests/charted_utility_continuation.rs` therefore boots that accepted ordinary
utility hull in a real local World with an actual charted navigation beacon.
The two-seat Courier stays outside the utility's instruments but inside the
unchanged fleet LOD bubble, so the craft uses normal high-fidelity per-System
AI and the existing paid XYZ/yaw physics owner.

A test-only read observer records actual Navigation/actuator emitted work. The
witness waits for a genuinely selected fractional XYZ point, normal clearance,
paid planar/vertical work and actual upward motion before taking its JSON
checkpoint. Fresh-host continuation compares complete captures and digests
through all 360 updates, with a final comparison after update 360. Post-checkpoint
motion and newly paid vertical work are required; boot travel or inherited
inertia cannot satisfy the continuation claim. Three malformed whole candidates
must refuse without changing state or digest. The same ordinary motion then has
to settle near its XYZ goal under authored acceleration/braking.

This is a local-course proof, not a Bay launch, voyage Job, remote-order,
link-loss, return or recovery journey. It does not mutate live knowledge, power,
controls or body poses. Chart withdrawal and explicit-feed isolation already
have separate owner tests; no artificial live binding change is used to claim
an intermittent-feed gameplay test. This paragraph records authored evidence;
source review and native execution results remain separate integration records.
