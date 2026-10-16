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
