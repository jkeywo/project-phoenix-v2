# Increment 15: frozen ordnance and interception contracts

This bounded contract implements the observed-missile/interception slice of the
[adopted increment15 plan](increment-15.md). It prepares the current owner; it is
not implementation or acceptance evidence.

`TorpedoSystem.in_flight` and `burst_states` remain the only authoritative round
and pending-emission collections. Full3D yaw/pitch flight already exists and is
retained. Capital-ship movement remains planar. No projectile proxy stores or
independent PDC controller are introduced.

A required Model `GuidedRoundProfile` contains the actual speed, maximum turn
rate, initial lifespan, detonation radius, hull/shield damage, shield pierce,
physical impact profile and maximum durability. Existing authored machinery
constructs it at burst commitment. The burst also freezes its cadence and
already-supported origin/barrel pattern. A launched round receives that same
profile plus its own remaining life and durability. Movement, contact and
impact read its profile, not the current launcher configuration.

Each actual emitted round still pays its ordinary work. Its existing
`strike_damage_bonus` records only the boost actually granted to that emission;
the first round's paid boost is never copied to an unfunded later round.
Changing launcher equipment during flight or between burst emissions cannot
increase their physical profile, reset endurance or replenish stock. A pending
emission can wait for ordinary machinery funding/availability.

Model validation and whole-candidate restore preflight reject nonfinite or
unsupported profile values, life/durability outside the frozen initial bounds,
invalid physical profiles and malformed identity/cadence. Capture, restore and
digest use the required profile directly, with no shadow values or old-state
fallback. The integrator advances the compatibility frontier with the coherent
increment15 state definition.

Later interception uses the existing bolt/mount/resource owner and its ordinary
bank AI. Optional authored target domains use the closed EntityTag vocabulary;
missing domains retain the ordinary legacy target set, while explicit Missile
permits supported ordnance selection. The controller consumes only its current
permitted measured picture and normal assignment/Hold/protection intent.
Physical impact can resolve actual projectile state after that permission seam;
AI cannot obtain hidden launcher allegiance, target orders or perfect trajectories.
Finite durability, actual ammunition/work/cooldown and shared mount tracking
make saturated defence a measurable shortfall. Post-launch guidance and separate
Science illumination follow only after physical interception works.

Meaningful verification includes a mid-burst equipment change, changing the
launcher after a full3D launch, shield/hull payload conservation, endurance and
physical interception without target truth, an unfunded later boost, exact
active-flight/pending-burst continuation, invalid no-write candidates and two
hosts projecting different LocalShips. Authoring and actual human/AI journeys
remain part of the complete slice, not proven by these pure contracts.

## Guidance observation and link checkpoint

The next implementation slice replaces the shared live-position guidance map
with per-round observations. The collision roster remains available only to
physical impact processing. Frozen seeker equipment defines finite range,
three-dimensional forward arc and minimum Sensors-path transmission. Optional
receiver equipment defines a finite Comms link and minimum transmission. Absence
of either item withholds that capability.

Each actual Torpedo retains its last accepted XYZ observation, original sample
tick and closed seeker/Console provenance. It may continue steering toward that
held point after observation loss; the point never follows hidden target motion.
Remote retarget/stop commands address the actual owning tube and actual round,
use existing authority and paid machinery, and preserve the observation's
original time. Stop cancels guidance while preserving the physical flight.
Science illumination remains a separate paid, mounted System and cannot retarget
Tactical or gate an otherwise viable shot. These are selected contracts; runtime,
Workshop and continuation evidence are recorded with their later checkpoints.


## Source orders and paid links

The source ship knows which IDs its actual tubes emitted, their initial orders
and frozen endurance. Its controls and ordinary tube AI use that source record,
not an enumeration of surviving physical flights. An issued order remains
unresolved without a viable return-link acknowledgement; a hidden interception
or lost contact does not remove a row or disclose a special refusal. Known
endurance permits retirement. This is command knowledge, not another round
simulation or a message-capacity system.

Current tube equipment's paired guidance-link profile and ordinary Operation
and work limits constrain transmission. The emitted receiver's frozen limits
independently constrain reception; both Comms path/range thresholds must pass.
Return acknowledgements need their own viable reverse path. Refitting away the
source equipment removes its ability to transmit, while already emitted physical
flights and source knowledge retain their existing owners.

Target updates require permitted measured XYZ, including observed elevation,
and retain original radar provenance/report times. Independent Science
illumination can supply a dated matching point beyond the round's seeker reach;
it cannot change Tactical's intent. Link loss stops new observations and leaves
the old measured point as memory. These declarations precede the corresponding
runtime/authoring/continuation implementation and make no exit claim.


### Receiver source checkpoint

The actual receiver handler now stages the local paid order before attempting
physical delivery. Its return receipt means **heard**, even when an independent
onboard permission prevents applying that intent. Missing remote equipment,
interception and loss of link provide no private refusal detail. The current
source tube and the emitted frozen receiver must each permit the outward and
return paths. Source-known records retain original sample/proof time and known
launch endurance without projecting remote survival.

Human and the existing tube AI deliver through one Physics handler **after** the
current flight advance and ordinary tube AI, **before** actual firing. An order
therefore steers on the next flight advance for either controller. Refresh follows
actual firing and advancement, retaining delayed launch provenance by its first
actual emitted ID rather than a second burst counter. Row-local source joins run
before single-row inverse/resource writes as well as whole restore; whole-roster
checks alone add cross-actor ownership joins. This is source implementation and
review evidence; native receiver, human UI, ordinary maintenance and independent
illumination acceptance remain pending.


## Independent illumination implementation contract

Science illumination uses a separate composed System: an `illumination_emitter`
component binds its declared Boolean enabled, EntityTarget target and PowerInput;
an optional Turret is the existing shared mount, not another tracker. Supported
finite config is its target mount plus link_range and minimum_transmission. The
existing paired receiver profile gains default-false
`accept_external_illumination`, explicitly authored true only on accepting rounds.
Receiver age remains the original finite maximum_sample_age_secs.

An enabled emitter must have a current permitted XYZ observation, actual mount
readiness, Operation and paid continuous work. Its airwave can update only an
already matching actual round through a viable finite source/receiver path.
There is no source success/receiver census and no Tactical target mutation.
Beyond the round's own seeker range, a newly observed point is a real steering
benefit. Loss withholds new readings; the old dated point remains memory. Human
controls and a single registered closed per-System state tree share ordinary
on/off and target commands. Tree selection comes from its permitted mission
request or retained independent target. Workshop composition, authoring data,
strict channels/owner continuation and exact loss/freshness tests belong to this
slice. This declaration precedes implementation and claims no playable exit.


The accepted `Illuminate { target }` Directive uses the existing closed
mission/doctrine/effect codec and Sensors relevance. A request is resolved only
through the emitter's permitted identity/reference picture after the existing
AI text-reading frontier (`reading_ticks`/TreeRuntime.reading). It is neither a
new mission planner nor a change to Destroy/Scan routing. The reference case
must issue this actual supported request so ordinary Backfill can select and
emit independently of Tactical selection.
