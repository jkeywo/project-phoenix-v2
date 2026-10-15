# Increment 15: layered combat

Adopted implementation contract prepared against main `8d53ada5`, before
increment15 runtime work. Increment14 supplies the sensing/environment contract;
its initial checkpoint and remaining validation must be recorded separately.
This specification is a dependency-ordered implementation plan, not evidence of
a playable exit. The user's authorised pipeline permits later validation on a
pinned worktree; it does not turn untested behaviour into a passed result.

## Outcome and selected scope

Give the bridge several useful ways to fight, support, survive and disengage.
Longer-range missiles create time to observe and respond; finite interception
creates protection choices; broadside equipment creates useful Helm decisions.
None establishes a compulsory missile→PDC→broadside sequence. Preserve viable
beam/projectile, close-pass and stand-off configurations and a crew's option to
avoid or leave an engagement when its actual situation permits.

The [catalogues](../ship-systems.md), [selected review](../system-review.md) and
[PASM](../../../pasm/spec/design/layered-combat.yaml) govern the work. Research
provides examples, not competitor constants or additional mandatory mechanics.

| Selected entries | Required delivery and retained dependencies |
|---|---|
| G048; S015/S055 | Primary Combat Lock plus explicit weapon/System or prepared group assignments. Authored/saved group membership; an individual override survives later group commands until explicitly released. Unassigned delegated offence prefers the primary; authored doctrine may permit alternatives or require concentration. |
| G049/G052/G056; S001–2/S006–9 | Sustained beam start/stop and conservation within real geometry/reserves/heat; energy bolts and finite-round kinetic equipment; distinct broadside arcs and finite traverse for suitable turrets; supported heavy recoil. Existing local batteries/cooling provide endurance, without another shot capacitor or flux meter. |
| G051/G053; S003/S008 | Longer-range, slower reference missiles with finite life and physical flight; ordinary PDC interception and dual-purpose short-range fire compete for the same mount, stock and machinery. Protect other assets only within real observation/geometry/reach. No universal transit/combat lock. |
| G054; S004/S014 | Actual finite rounds, loading commitments and optional shared loader groups/priorities. Extend existing stores, timed logistics, fabrication recipes and permitted port replenishment; no separate combat inventory or compulsory batch-click loop. |
| G105/G107; S003/S054/S096 | Suitable rounds accept supported post-launch intent over a working equipment link; fixed-intent/independent variants remain. Separate Science illumination provides assistance under finite capacity/geometry/resources. Tactical retains targets and launch authority. Helm sees only conditional predictions from observed ordnance. |
| G050; S005/S010–12/S016 | Optional equipment-supported shield enquiry/tuning, temporary EMP with stacking resistance, physical collateral/area effects, penetration through existing compartment damage, and actual limited marking-beacon observations. Scan-derived compartment/approach advice uses acquired findings, not private component coordinates. |
| S013 and later consumers | Reuse the payload/trigger/sensing contracts for mines; persistent field placement, selective mine equipment and general deploy/recover lifecycles remain increment19. Electronic attack/countermeasures remain17; channels/encryption/relay orders remain16. These dependencies are not claims that those catalogue entries are already implemented. |

The reference six-seat Cruiser keeps Tactical weapon/defensive priorities,
Science investigation/illumination/shields, Engineering resources/restoration,
Helm motion and Comms coordination. Assignments are authored data, not Rust role
rules. Keep Courier2, Destroyer4, Cruiser6 and Battleship8 rosters; no new
claimable charging or fire-control seat, Captain map or Captain shot approvals.

## Extend the current owners

Increment7 already compiles `WeaponDefinition` into beam/bolt/guided strategies,
with shared `MountConfig`, effect delivery and barrel scheduling. The current
mount provides static facing/arc/range; finite traversal and target-domain
support are extensions, not existing completed turret behaviour. Existing beam
cycle/fractional damage, bolt lead and guided ammunition/burst state remain their
owners while each strategy gains its selected behaviour. Existing physical
damage, local work/resource services, fitting and increment14 findings remain
authoritative. No parallel weapon simulation, universal combat manager or AI
planner may bypass those owners.

First migrate an existing consumer onto each extended contract with equivalent
behaviour; then author the new equipment and intentional tuning. Static mounts,
fixed-duration beams and independent loaders remain valid supported equipment
configurations. A capability's richer mode must not silently reinterpret every
shipped weapon. Unsupported combinations fail accepted-source validation.
Temporary adapters only translate data/payloads and are removed before15 closes.

Shared mounts own actual aim/traverse once. Components in a System share its
target/commands and a shared turret, including a fitted weapon/targeting array.
Separate capabilities do not gain independent invisible barrels or instantaneous
retargeting. Reuse the geometry with fitted sensing/illumination. Ordinary
gun aiming and target predictions use permitted observations; a physical hit
processor may inspect actual colliders/compartments solely to resolve impact.

## Ordered playable slices

1. **Assignments and mount execution.** Preserve primary-target play, add
   prepared group/System targets and explicit override/rejoin controls, then
   finite shared mount tracking. Demonstrate a human and an ordinary delegated
   bank using the same target/geometry and respecting a stale-host rejection.
2. **Useful weapon configurations.** Add supported sustained beams, finite-round
   kinetic bolts/heavy recoil and a distinct side-facing battery configuration.
   Extend actual round/loader services and existing firing cycles rather than
   adding stock counters. Existing best-effort cycles fire ready listed barrels
   and skip unavailable ones without substitution or free ammunition. Prove a
   useful broadside pass and an alternative effective engagement style.
3. **Observed missiles and interception.** Extend the existing guided lifecycle
   to full3D flight while capital ships remain planar. Tune a reference missile
   substantially slower and longer-ranged than its old baseline, with finite
   turn rate/endurance and one actual launch snapshot. Add detectable/damageable
   ordnance and PDC targeting using the common mount/delivery mechanism. Routine
   tracking/firing is ordinary per-System AI; operators choose protection and
   offence/reserve priorities. Saturation produces an understandable shortfall.
4. **Guidance and illumination.** Add closed intent changes/cancellation only
   for rounds whose authored receiver and real link support them. Model link
   loss and seeker limits explicitly; preserve last accepted intent or the
   configured independent behaviour, without a global perfect target feed.
   Fit independent Science illumination with persistent assignments; ordinary
   AI maintains actual tracking and assistance. Losing illumination removes
   assistance, never changes Tactical targets or creates a firing-permission
   veto. Equipment-specific guidance requirements remain separate.
5. **Payloads and investigated weaknesses.** Route area/EMP/penetrating effects
   through common impact/condition owners and add limited physical marking
   observations. Expose optional focused shield enquiries and scan-derived
   compartment/angle advice, then configured weapon tuning. Exercise an ordinary
   foe where probing is a small benefit and suitable special equipment where it
   matters more. Default ordinary combat remains viable without this enquiry.

Each slice includes authored data, operation/delegation, status/feedback,
Workshop editing/validation/Preview/Test and immediate continuation integration.
An initial commit is allowed before its later validation; record the exact
pending work instead of claiming completion or omitting an included capability.

## Physical effects and information

### Optional missile reference and producer evidence

The actual PDC producer probe finds its first loaded firing opportunity at tick
183, but its first permitted shield assessment only at tick 361, after the
inherited broadside approach has turned its fore tube away. The failed probe is
retained as evidence of that mismatch, not proof of interception. Do not weaken
the assessment gate or expose hidden shields to make it fire.

Add an optional Missile Cruiser using the existing authored artillery doctrine,
finite stocks, loading, power and guided lifecycle. Provisional reference tuning
is speed 5, lifetime 120 seconds and range 600 (old Cruiser 15/20/300), with
matching finite seeker/receiver reach and a 600-unit Tactical radar. Its doctrine
holds at 80 units within a 100-unit artillery envelope rather than making the
short-range broadside pass. Both existing axes receive the same envelope.
These are explicit equipment/doctrine changes on the optional reference, not
balance changes to the shipped Cruiser or faster/free sensor assessment. The
PDC single/salvo probes may use this ordinary configured opponent; keep their
actual launch, acquired contact, interception, saturation, conservation and
continuation assertions. Verify accepted loading and observed outcomes before
claiming a playable missile/PDC engagement or balance result.

The configured opponent produces real paid launches, but the next host probe
exposes a missing observation join: the ordinary Broad ordnance adapter still
publishes every projectile as unselectable, irrespective of an explicit Missile
selection grant. Correct that actual producer to honour the radar's declared
Missile selection capability and publish the canonical `missile` classification
through its existing timed acquisition. Default instruments without that grant
retain unselectable ordnance. Ordinary defensive AI still requires current acquired
classification, measured height/velocity and real range/geometry; no launcher,
target intent or private projectile coordinates are published or substituted.

At launch, freeze actual payload properties, consumption, damage/effect profile,
flight equipment and firing origin. Later ship upgrades, target changes or
guidance messages cannot increase an in-flight payload or replenish its fuel.
Intersect physical paths/eligible targets; do not grant friendly/civilian blast
immunity. Area falloff, interception outcomes and supported cover are authored
and deterministic. AI risk decisions use held evidence, not unseen exact poses.
Avoid charging the same effect twice through endpoint and path attenuation.

EMP temporarily affects actual components and clears on expiry without repair.
Keep physical component conditions and the temporary effect/resistance lifetime
distinct under shared status composition. A reapplication retains the greater
of remaining EMP time and the new resistance-adjusted duration; the longer-lived
resistance stacks and reduces subsequent duration. Author duration, reduction,
stack/decay bounds and deterministic same-tick application order during the
implementation slice. The selected design does not supply universal constants,
permanent immunity or a proven maximum interruption interval.

Penetration keeps the increment11 weighted-deck/2D compartment path, material
resistance and component-condition model. Components have no precise internal
position/HP. Targeting advice names an observed compartment and useful approach;
it does not steer Helm, select a weapon target, expose an unscanned interior or
guarantee that intervening structure will be bypassed. Repeated hits can open
the existing physical path by reducing that structure.

A marking payload must leave a finite, vulnerable observation-producing piece
of equipment, with an actual subject-relative position, power/endurance and
working report link. It reports only its supported measurements; a flag on a
target is not a free permanent track/aim bonus. Use an ordinary configured entity
and the existing observation/power/condition contracts. General sortie,
deployment/recovery, minefield management and craft AI remain19. If a prerequisite
for this bounded impact-spawned consumer is missing, record the dependency
explicitly and complete it before claiming15's marking capability.

Shield weaknesses are acquired findings tied to the observed configuration.
They generally persist; equipment may rotate its shield/beam configuration,
making an older finding historical. Broad assessment never grants protected
frequency/weakness truth automatically. Ordinary delegated probing/tuning uses
the same equipment, permitted findings, elapsed work and opportunities as humans.
No compulsory recurring retune, exact enemy intention/timer overlay or hidden
future-hit knowledge is introduced. Actual battery charging/weapon preparation
can provide observable evidence for inference.

## Authority, complexity and authoring

Commands remain System-addressed through System→Console→hosting Station→player.
Displaying a group, radar or support report confers no machinery authority.
Cross-Console assistance/requests use explicit feeds and normal communication;
the receiver's individual controller retains its decision. Human-seeking and
ordinary temporary Backfill retain their existing semantics and hysteresis.

Use the existing closed, reusable/inheritable state trees and reference nodes.
Convert each migrated controller alongside its capability; never run a legacy
selector and a tree on one System. New facts expose only permitted findings,
actual own reserves/readiness and supported orders; actions emit the same typed
commands. Simpler authored levels delegate whole Systems. Beginner Tactical can
choose a primary target; middle levels add group/interception priorities; deep
levels expose supported overrides/timing. Beginner Science investigates while
ordinary AI handles protection/illumination; deeper levels can allocate persistent
support. Engineering listens/prepares reserves and rations under pressure, with
existing margin from stable preparation. Helm receives advice and owns steering.

Workshop extends its current accepted-source/history path: supported mount and
delivery combinations, groups, target rules, finite stocks/loaders, firing modes,
guidance/payload contracts, illumination and shield enquiry equipment. Show
incompatible references/ranges/resource requirements before Preview/Test. Reuse
ship/System/component templates, layout placement and personal fitting previews;
do not create an independent combat-only equipment definition or allow Workshop
scripts/extensions. Save supported working views by Console, not display type.

## Continuation, evidence and exit

Integrate authoritative assignment/override, mount aim, preparation/cycle/cadence,
loader/stock claims, projectile pose/flight/receiver intent, observation-producing
payloads, illumination assignments and timed effects into the existing snapshot,
digest, replay and delivery contracts in their own slice. Derived geometry,
projections and report caches remain derived. Preflight all IDs, references,
ownership, resources, timers, finite numeric bounds and cross-entity claims
before any restore writes. Advance compatibility explicitly; no silent old-state
reinterpretation or transport redesign. Use named fixed-phase ordering and prove
the complete host graph, rather than only isolated owner consumers.

Targeted evidence must include human/vacant/delegated operation, explicit target
and information isolation, group overrides, arcs/traverse/readiness, protected
asset interception/saturation, physical friendly collateral, finite ammo/loading/
production/port conservation, EMP refresh/resistance/expiry, real guidance link
loss and fixed launch payloads, optional probing/configuration changes, and a
limited marking source losing power or its link. Save/resume during active
aiming/salvos/flight/effects/support and compare full supported captures/digests;
forged continuation must refuse without writes. Use two fleet peers with
different LocalShips to expose knowledge/order dependence.

Compare representative combat performance/activity and non-combat utility with
increment14; keep existing balance gates and explicitly author intentional
tuning. Run scenario design digest/writeback before changing scenario pacing.
Exercise several viable equipment styles and 2/4/6/8-seat delegation, especially
simultaneous Science investigation/illumination/protection and Engineering work.
Record observed workload, waiting, overload and repetitive upkeep separately
from automated correctness. Browser/native and human acceptance remain distinct.
The integrator runs required repository gates once before a combined push.

### Illumination source checkpoint

The source consumer uses the canonical composed channel map, the actual mounted
System's continuous work and permitted dated XYZ. The optional Cruiser reference
adds finite equipment and a Sensors-only Illuminate mission request; it changes no
shipped hull's combat tuning. Receiver opt-in is frozen at launch. Independent
source feedback describes its own emission only. Workshop now edits component
bindings, both tree-output channels and the one shared guided profile, including
explicit inherited equipment disablement. Owner and continuation tests are
source-written; JavaScript controls/history have targeted evidence. Native,
whole-host and human acceptance remain pending until recorded by integration.

Targeted source-checkpoint evidence (2026-10-15): eight JavaScript suites passed
47 tests; the subsequent simultaneous-channel destination-collision fix passed
all 13 ship-authoring cases (overlapping the earlier batch). Offline Workshop
packaging passed; strict strings reported 5,931 entries with zero errors/warnings,
and layer imports plus owned Rust formatting/diff checks passed. The new runtime
owner and snapshot cases have not yet executed. This is source integration
readiness, not the increment's native, whole-host, human or balance exit.

### Runtime flight-carrier resume readiness

The existing delayed rebuild path must recognise its supported closed weapon-flight
carrier recipe, which has no authored SpawnOrigin. Keep `ready_to_restore` strict
about the roster already present; `ready_to_rebuild` may admit missing carrier rows
only after the same complete carrier-roster validation used by restore. Ordinary
missing actors still require their accepted SpawnOrigin. Do not create a launcher,
equipment or new flight vector to satisfy readiness. The actual PDC launcher-loss
journey uses this existing rebuild route and retains complete continuation checks.

### Optional missile/PDC resolution reference

The actual two-tube probe exposed coincident legacy ship-centre launches. Keep
that legacy behaviour for shipped hulls. The optional Missile Cruiser explicitly
authors each tube's existing barrel marker, using the established rig/origin
mechanism. Its paired fore rounds then have physical separation rather than a
private identity exception in sensing.

The optional PDC Cruiser uses a finer Tactical broad instrument: position error
0.05 instead of 2, with continuous work cost 1.5 instead of 1. All other sensing
and energy-profile fields, finite ammunition, generation, timed classification,
weapon cadence and damage remain equivalent. Materialise complete atomic profiles
through the existing include resolver; do not introduce partial profile merging.
These are provisional equipment choices, not a balance claim. Current measured
association, ambiguity, paid acquisition and ordinary AI remain mandatory, and
the real larger-salvo interception/conservation/continuation checks remain strict.

### Physical saturation probe

The precision instrument and separated tube origins can legitimately defend
against the original four-round single-launcher load over two paid cycles. Keep
that successful behaviour. The larger-load witness instead uses two independent
ordinary Missile Cruisers, at separate physical positions and with distinct
public identities. Each retains the same authored six finite rounds, paid
loading, real observations, per-System AI and normal fire admission. No new
observation, projectile, command or reserve is injected.

Require concurrent actual flight from both producers, actual qualified PDC
interception, and terminal payload damage to the protected ship. Attribute emitted
rounds by each live flight's actual source and check each producer's magazine,
tubes, hold and in-transit stock conservation separately. Keep the PDC bank's
hardware, cadence, finite stock and zero-intake energy accounting unchanged.
This quiet runtime probe changes no mission pacing, shipped balance threshold or
single-source launcher-loss/complete JSON continuation witness. Execution and
whole-increment acceptance remain pending until integration records them.
