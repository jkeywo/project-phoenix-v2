# Increment 19 — Ordinary deployables and carried craft

Contract preparation, 15 October 2026, under the adopted
[parallel-work contract](parallel-work.md). Static deployment can follow 15's
initial integration/targeted checks after the shared 19/20 custody contract is
adopted. Carried/remote craft join actual 18 movement and 16 links; deception joins
17 and passengers join 20's single personnel custody. Full completion requires
verified 16/17/18 exits. No runtime, commitment or acceptance status changes here.

## Outcome and scope

Launch useful equipment/craft, sustain their real work and recover the actual
survivors. Deployed tools use the same entity/System/resource/damage/AI rules as
other ships, with authored capabilities and different ordinary controllers. They
are not free ability tokens, new per-family simulations or the 15 emitted-ordnance
flight carrier. The catalogue and
[PASM contract](../../../pasm/spec/design/deployables-craft.yaml) govern.

| Selected entries | Required delivery and limits |
|---|---|
| S017 | Optional fighters/attack drones execute bounded sorties with ordinary per-System AI and full 3D motion. Operators choose mission/priorities, launch/recall/recovery and risk; direct human piloting is outside this initial scope. |
| S019 | A decoy is an actual short-lived engineless ship with finite battery and spoofing System. A turret is an engineless ship with weapon/battery/simple ordinary AI. Apparent spoofed hull geometry remains evidence, not a collider; 17 supplies deception. |
| S061/S101 | Physical probes use actual independent sensors, ordinary history and viable command/telemetry/relay equipment. Gap findings reappear in normal history with original dates after restored links. No hidden live update, special catch-up System or message-capacity quota. |
| S013 | Persistent mines use actual payload/delivery, finite deployment and authored local trigger/sensing/identification. Indiscriminate variants remain possible; recognition uses held evidence, not hidden faction truth or friendly blast immunity. |
| S086; G061 | AI pilots physical utility craft; team and craft commitments are distinct. After delivery, a shuttle can take another job only when the current task does not need its tools, shelter or immediate extraction. Later pickup needs actual craft, travel, access and capacity; no automatic recovery/double booking. |
| S092/S093 | Supply drones repeat viable approved physical deliveries under goals/reserve limits. Work drones perform supported repair or deployment of carried equipment with real parts/access. No unrestricted gifts, remote System control or general construction of new entities from materials. |
| S094 | Optional actual energy/shield assistance uses source resources, supported recipient compatibility, geometry and acceptance. It is not a physical repair beam; actual teams/work craft restore components. |
| S083/S084/S100 | Existing Tractor/Umbilical physical constraints and source/receiver stocks remain. Escape-pod signalling is equipment-backed open/directed/secure/silent intent with finite endurance and real pickup; 20 owns occupants/Medical. Silence is not guaranteed stealth. |

The reference Cruiser's Tactical seat retains deployment/sortie/physical transport
commitments, Sensors interprets supported observations, Comms handles links/requests,
and Engineering supplies resources/restoration. These are authored assignments,
not hard-coded roles, a new mandatory pilot seat or Captain private map. Preserve
2/4/6/8-seat scope and useful ordinary delegation without per-craft steering chores.

## Existing owners and first actual consumer

Use the current accepted EntityConfig/spawner/ship-spawn path, equipment/energy/
compartment damage and ordinary System registry/tree/admission. Existing 13 portable
equipment identity/condition and fitting validation, physical Repair jobs, local
routes and finite lockers supply relevant handling patterns. 12 Stock/logistics/
ports retain cargo; 14 sensors/history/sharing and 16 links retain information;
18 owns motion/docking. None is evidence that a deployed craft lifecycle already
exists. 15 flight carriers preserve only emitted shots and cannot masquerade as
deployable inventory, a bay or an operable drone.

First consumer: one actual battery-only engineless sensor probe, physically deployed
from a finite compatible compartment/bay using real work/access, acquiring a real
dated observation and recovered without resetting its battery/condition/identity.
Then add one real finite-round stationary turret using the same lifecycle and
existing weapon controller. These tracers establish actual producers rather than
an unused deployment registry or a special effect that simply appears remotely.

Model owns closed craft/bay/lifecycle/order DTOs; Gameplay owns physical lifecycle,
finite handling/capacity/claim rules and supported job activities. World authors
accepted entity recipes, missions/context and actual outcomes; Session retains
participant/authority. Simulation joins actual actors, resources and jobs without
sibling imports. Every deployed actual System uses its existing individual AI,
admitted commands and permitted Console knowledge; no swarm/wing/central controller.

## Adopted first physical probe slice

This first consumer is one actual battery-only, engineless sensor probe handled by
an installed compatible Bay System. Its accepted profile has no passengers, child
craft, visual descendants or weapon capability. These are limits on this prototype,
not restrictions on unrelated ordinary entities. Stationary turrets, moving release,
propelled craft, passengers and emitted-ordnance joins remain later slices. The
probe retains its ordinary sensors, finite reserves, component condition, dated
history and existing viable Comms/sharing; the parent's bay view cannot read remote
live state through private custody references.

The first closed composition is `CraftBay { id, power_input, config }` on the
actual Bay System. `BayConfig` supplies compartment, finite capacity space,
launch/recovery work seconds, recovery range, maximum parent speed and local
release offset. The probe's `ProbeProfile` supplies its occupied space; its
ordinary accepted energy/damage/sensor profiles supply actual capability.
`LaunchCraft { craft }`, `RecoverCraft { contact }` and `CancelCraftHandling` use
the same admitted owner: launch addresses an own stored identity, recovery resolves
an exact current permitted contact, and physical placement checks reveal no hidden
obstacle facts. Each per-bay job wraps the existing actual Job vocabulary.

Initial `carried_by = { parent, bay }` belongs to the ordinary authored World row,
referencing a declared base actor and actual Bay System. Resolve and commit it
inside the existing spawn chain before any gameplay, physics or publication;
there is no bootstrap tick where an authored Carried probe operates in space.
Conditional, layered/additive and runtime Carried authoring are explicitly
unsupported in this first adapter and refuse before materialization. Later
lifecycle extensions must reuse the same custody owner.

The same actual probe has exactly one canonical location: `Carried { parent, bay }`
or `Deployed`. One actual BayJob owns accepted launch/recovery work and progress;
Launching/Recovering are projections of that job, not another saved phase/progress
copy on the probe. Bay occupancy derives from actual carried locations and the
job's receiving claim. Each accepted bay reserves a finite compatible physical
allocation disjoint from existing cargo and portable-equipment locker allocations.
It cannot count the same compartment space twice. Refusal stages no payment, new
identity or extra receiving claim; completed handling changes that same craft's
location once and preserves its actual physical state.

Handling requires the actual Bay System's installed/source/Operation/access and
paid work gates, plus a finite authored nonnegative maximum parent speed. Measure
speed from the existing physical motion owner; a resolved fixed parent with no
motion owner is stationary, while missing motion on a movable parent refuses.
No fake Helm, propulsion or second integrator is installed to satisfy handling.
Parent movement beyond that threshold, damage, disablement, missing Power or lost
access pauses/refuses supported work without resetting performed progress. Release
and recovery use accepted local bay geometry and the same actual body; moving
launch momentum and propelled rendezvous are not claimed by this stopped-parent
prototype. Human and the Bay System's ordinary individual AI issue the same
admitted commands under their actual Console authority.

While carried, derive nonspatial inactive presence from the canonical location.
The initial adapter may use Bevy Disabled, but must explicitly detach actual Rapier
backend handles at stow and allow ordinary backend initialization at release;
Disabled or RigidBodyDisabled alone is insufficient proof of removal. Preserve the
authored collider/body and every causal component. Actual physics-query/backend
and presentation tests must prove hit before stow, no hit/visible body while stored,
and hit again after release with the same identity and state. No standalone unused
presence framework is an exit. The childless profile keeps recursive visual and
weapon-flight storage outside this first supported adapter.

Stored actors still participate in canonical capture/digest/preflight and passive
reserve drain, thermal/condition aging and absolute EMP/resistance expiry. Explicit
query participation must include stored actors where that owner requires it;
ordinary active generation, sensor acquisitions, emissions and controller work are
suspended. At stow, use the existing no-work observation boundary to withdraw
current Broad support/private routes while retaining original dated history. A
storage gap cannot yield fabricated continuous velocity or refreshed observations.
Relaunch neither refills the battery nor heals, resets, remints or clones equipment.

Actual removal of a parent loses its actual Carried craft in the same existing
removal cleanup/event transaction before the next public/checkpoint boundary. Use
the ordinary World/GM/physical-loss lifecycle owners; retain deterministic ordering
and normal terminal cleanup without minting replacement craft or emitting an
unpaid ejection. Deployed craft survive parent removal independently. A disabled
but present parent only blocks handling. The actual removal consumer must cover
ordinary scripted/GM, physical loss and supported layer teardown paths, rather
than leave hidden stored orphans through an unhandled deletion path. Bulk restore
suppresses gameplay loss cascades and validates the entire candidate parent/bay/
craft roster before writes; unsupported single-row inverse resurrection of contained
craft refuses before creating any body.

Capture the canonical location and the sole BayJob/claims alongside the existing
ordinary probe state. Rebuild presence/occupancy/backend views from accepted state,
without saving another mutable stored blueprint, bay count or physics owner.
Before writes, reject missing/duplicate parent or craft, incompatible/overfull or
overlapping allocation, impossible job/location/clock and unsupported profile,
including candidates whose carried actors are absent from ordinary active queries.
Prove actual paid launch/acquisition/recovery, passive aging and unchanged battery/
condition/identity, interrupted/blocked work, actual parent removal versus deployed
survival, exact fresh JSON 360-step continuation and complete-candidate no-write
negatives. Supported accepted-source Workshop data/forms and real backend/render
journeys belong to this same probe consumer; this slice does not close full 19.

## One actual craft and equipment owner

The battery-only consumer uses the existing energy network with explicitly zero
sustainable generation. Its finite shared source capacitor may export stored
energy, and its ordinary local reserves pay work and losses. Zero generation
must be supported by initial validation and accepted profile reconfiguration;
negative/nonfinite generation, invalid output/ramp and inconsistent saved
reserves still refuse. No new battery controller or per-deployable energy store
is introduced. A zero-generation authored probe is the first consumer; unchanged
powered hulls retain their existing coefficients. Exhaustion stops actual sensing
and transmission through the usual paid work owners, without clearing acquired
history or refilling on restore. This prerequisite alone does not implement
physical deploy/recover or close the first lifecycle slice.

The first source checkpoint adds `assets/entities/battery_probe.toml`, an ordinary
AI-only broad instrument with finite local energy and zero generation. The shared
authoring, saved-network validation and accepted refit path allow nonnegative
generation while retaining positive ramp and all other finite bounds. The existing
Workshop energy controls and fitting validator use the same supported coefficients.
Three Gameplay owner cases pass (zero-profile bounds, transactional refit refusal
and finite work through JSON); 45 affected Workshop checks pass. The actual-host
probe identifies a real contact, exhausts its actual reserve and preserves dated
history across 360 complete JSON capture/digest comparisons (5.98 seconds;
`.phoenix/battery-real-host-positive-intake-retry.log`). Its accepted hardware has
positive intake capacity, zero allocation and zero generation; intake capacity
does not create energy. The initial source's omitted collider length and invalid
zero intake capacities were corrected before this passing data run. These checks
do not establish a launch bay,
deployment/recovery job, command link or completed increment 19.

Preserve a stable craft/deployable identity and actual equipment/resource/condition
state through carried, launching, deployed, recovering and lost/destroyed phases.
For the first supported adapter, retain the actual ordinary entity/continuation
owner while carried and explicitly suspend its spatial/active operating capability;
the bay's occupied roster derives from that location state. Do not keep a mutable
stored blueprint/counter alongside a separately live copy. If a later storage
representation materialises an entity, it must transfer that same full causal state
transactionally, not instantiate a fresh refill/healed copy with another identity.

Authored bay capacity/compatibility is a physical allocation, not a universal fit
currency. Actual cargo, equipment lockers and bays must not each claim the same
space or identity independently. Named portable assemblies retain global identity;
their physical condition and EMP/resistance travel with them under the existing
single-owner rules. Reserve/temperature/condition do not reset on store/deploy;
absolute causal expiry continues even while carried. Active stored operation needs
explicit actual support/supply and permissions, never a hidden free generator.

Deploy/recover work uses existing physical jobs/team navigation or a supported
ordinary handling mechanism with real rate/resources/conditions. Stage source
presence, equipment readiness, access, exclusive job and compatible destination/
launch space together for the next executable step. Only actual completion changes
location/physical operation; a request or animation is not a materialised payload.
No free ID, stock decrement or paid work on refusal. Real already-performed work
and transported equipment persist through interruption. Cancellation/recall keeps
actual cursor/flight/location and supported return/wait-for-space behaviour,
without teleporting to bay, replacing losses or erasing paid work.

Supported deployment position/aim comes from permitted local geometry/observations
and actual launcher/bay pose. Physical placement can inspect actual colliders to
resolve a refusal/contact, not reveal hidden contents or a perfect safe point.
Moving or failing parent equipment changes actual release/recovery conditions.
Emitted ordnance continues through its existing owner/carrier even if the launcher
is stowed, replaced or destroyed; storage cannot freeze/hide already emitted shots.

## Shared 19/20 custody and mission commitment

The [20 custody contract](increment-20.md#shared-1920-physical-custody-contract)
is authoritative for personnel: 19 owns actual carrier identity/location/motion/
capacity; 20 owns actual people/team identity/condition/custody. Passenger lists
derive from that same custody, not a second stock or mirrored crew roster. Existing
team navigation owns actual crossings. Stage source, physical transfer/link and
compatible receiving claims before handoff; commit once at the actual boundary.
Cancelled/interrupted transfers retain actual location and recovery consequences.

The craft's active mission and transported team's actual job are separate exclusive
commitments. A shuttle delivering a team may leave only if that task does not need
its tools/shelter/immediate extraction; clearing a craft order does not recall or
complete the team's job. Pickup/abandonment/risk decisions retain normal authority
and explicit consequences. Transporter joins the same custody boundary through
suitable established destination evidence, range/shields/interference/capacity,
not a universal receiving pad or invisible passenger teleport.

16 orders retain recipient, original issuing sequence, validity/persistence and
known supersession. Each actual craft receives and checks instructions under its
own normal authority/knowledge/capability; no direct foreign actuator access.
Link loss withholds new intent/telemetry, while accepted onboard work and ordinary
System AI continue under supported local policy. Unseen cancellation cannot stop
it. Delivery/read/accepted/refused/completion receipts travel back only over viable
links; missing receipt leaves the parent uncertain, even where private host state
knows the craft worked or died. Actual own bay/launch commitment is still a local fact.

Probes/craft continue only the observations their real operating instruments acquire.
Store dated history in those same sensor owners; later delivery retains origin,
sample time, uncertainty and current-versus-predicted distinctions. Copies/relays do
not improve confidence or grant raw physical routes. Parent operators see only
received/permitted data, never live remote reserves/position/host-private mission
progress after link loss. Routine history viewing covers restored gaps without
manual transcription or a second catch-up interface.

## Special consumers on the same lifecycle

Mines bind a supported local detector/trigger policy to an actual physical payload.
Recognition/discrimination uses that detector's legitimate observations and declared
rules, while indiscriminate contact/proximity variants may exist. Physical blast/
penetration/collateral follows 15's actual delivery owner independently of selection
filters; clearing/disarming needs supported actual work or physical damage. No separate mine
health pool, secret affiliation check or arbitrary damage immunity is introduced.

Standing supply goals bind approved recipient/resource/reserve thresholds to actual
cargo and viable delivery/transfer jobs. Do not pre-reserve all future repeats,
ignore a full destination or consume both a mirrored drone inventory and Stock.
Work craft can carry supported tools/parts and perform real repair/deployment;
specific construction missions need separate adopted consumers, not unrestricted
new entity fabrication. Energy/shield emitters use actual compatible source and
receiver accounting with finite reach/efficiency; they cannot repair components.

Battery-only decoys/turrets and short-lived emitters use actual energy/conditions,
17 spoofing, 14 sensing and ordinary controllers. Expiry/exhaustion stops supported
operation without inventing a fully charged replacement. Actual physical decoy
geometry is distinct from an apparent spoofed vessel. Destroyed/lost assets retain
supported wreck/recovery consequences and 20 participant/custody handling.

## Ordered slices, authoring and verified exit

1. Finite physical probe deploy/paid observation/recover and stationary turret on
   common entity/job/storage rules, with actual state conservation and interruption.
2. 18 full-3D ordinary-AI carried fighter/shuttle sortie/return and actual 16 command/
   telemetry/relay loss; no direct-human-pilot surface or perfect fleet mind.
3. 20 personnel transfer/recovery join, independent team/craft commitment and pod
   signalling/pickup. Prove capacity and route/availability loss without duplication.
4. 17 physical decoy, actual mines/clearance and supply/work/support variants with
   their real resource/repair/receipt consumers and meaningful shared constraints.

Workshop authors accepted ordinary entity profiles, compatible bays/handling,
finite reserves/tools/payloads, closed mission/trigger/standing-goal policies and
actual Console feeds through includes/history/Preview/Test. Preserve underlying
equipment editors and one controller per System. No per-family arbitrary runtime
code or generic construction/programming interface. First tuning is explicit and
provisional; source fixtures are not flight, combat or crew-workload acceptance.

Capture actual craft location/lifecycle/job/claims, real resources/equipment,
onboard mission/order frontiers, dated history and pending receipts once. Shared
20 custody remains its owner. Validate the full candidate parent/bay/craft/personnel
roster, unique identities, exclusive jobs, actual capacity/location/phase, immutable
profiles and nonfuture clocks before writes, including materialised/missing actors.
Derived bay/readiness/link/proxy views rebuild before use. Integrator owns schema,
digest/census/schedule/version changes; old incompatible state is refused or converted.

Prove normal Human/AI same-command paths, finite actual work/energy/ammunition/cargo,
source/parent loss, interrupted deploy/pickup, space blockage, no identity/refill/
condition reset, current-only remote evidence and continued onboard work during
link loss. Restore fresh JSON hosts at actual launch/flight/return/transfer/link-gap
checkpoints and compare complete captures/digests for at least 360 steps plus
different-LocalShip peers. Forged roster/custody/capacity/claim/order/clock candidates
must refuse without writes. Shipped hull/scenario/Workshop and physical browser/
native journeys, performance/sortie/combat/activity comparisons and 2/4/6/8-human
workload remain required; static foundations do not close full 19 or passenger 20.
