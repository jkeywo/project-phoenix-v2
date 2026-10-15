# Increment 18 — Movement, docking and transit

Contract preparation, 15 October 2026. The approved
[parallel-work contract](parallel-work.md), pinned on main `bc5cfbc6`, permits
movement work after 15's initial integrated implementation and targeted checks.
Full completion depends on 14/15's verified exits; remote formation request delivery
joins 16's integrated channel consumer. Preparation starts no runtime or acceptance.

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
