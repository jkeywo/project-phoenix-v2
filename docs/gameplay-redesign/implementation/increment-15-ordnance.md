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
