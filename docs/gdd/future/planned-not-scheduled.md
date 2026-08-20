# Project Phoenix — Planned but Not Scheduled

| Field | Value |
|---|---|
| Status | Accepted direction without a release band |
| Scope | Dependency-grouped features that do not currently fit the C1–C9 content or T1–T6 technical tracks |
| Audience | Design, roadmap, content and future planning |

These features are accepted parts of Phoenix's long-term design but have no promised release band. Their order below expresses dependencies, not priority or delivery sequence. Work should enter a band only when its supporting systems, scenario proof and release capacity are clear.

Related documents: [Future Modes](./future-modes.md), [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md), [Command and Crew Control](../mechanics/command-and-crew-control.md), [Native and Network Foundation](../systems/native-network-foundation.md), [Campaign Continuity](../foundation/campaign-continuity.md), and [World and Environmental Systems](../systems/world-environmental-systems.md).

## Human ship-to-ship media

Human-controlled ships may establish one-to-one voice, video and typed-text calls through their ordinary simulated Comms relationship. A call requires a known reachable contact, an explicit hail and acceptance by the human currently controlling each ship's Comms station. Range, interference, damage and jamming govern availability. A short interrupted-link grace period retries silently with media muted; expiry requires another hail. The first version treats simulated reachability as connected or interrupted rather than deliberately degrading packets.

Each ship controls its end independently. While private, the call sends from and receives through the device presenting that ship's Comms station, producing Comms-to-Comms, bridge-to-Comms or bridge-to-bridge combinations. Putting a call on screen moves audio and video together as one live endpoint handoff to the viewscreen device. It is not possible to promote only one medium. Comms retains call authority while the shared viewscreen follows the existing latest-valid-request-wins policy; when another valid request displaces Comms, the call returns to the private endpoint without ending. Switching away from the Comms station tab does not end or promote a private call, and the Hero Bar retains active-call, mute and attention state.

The zero-setup source is the device presenting the endpoint. Native bridge profiles may instead assign separate cameras, microphones and outputs to the private Comms surface and viewscreen. Promotion shows a clear transition and transmission state. Hardware or permission failure leaves the established private call intact. Neither side transmits microphone or camera data before acceptance, and no media content is recorded. Phoenix may retain call participants, times, routing state and interruption reason. Live captions remain transient; deliberate typed messages enter the ordinary Comms record with ship and speaker identity.

This feature ships with its Discord bot rather than treating Discord as a later add-on, because Discord is the first planned test environment. A host-configured bot acts as that ship's bridge endpoint, streams the viewscreen and bridge audio, and returns remote media while obeying the same reachability, consent, routing and mute rules. Browser-to-browser calls remain a complete supported path. Fleet conferences and open voice rooms are out of scope for the first delivery.

## Duty Teams, Away Missions and Medical — scheduled for Band C7

The shared personnel foundation comprises typed anonymous Duty Teams, named Duty Officer leaders, system assignments, personnel consequences, multiple Operations systems, shuttle/transporter deployment, branching off-screen Away Missions, boarding, internal defence and Medical. Its minimum complete vertical is now scheduled for Band C7 and proven by the authored War run.

This cluster is detailed in [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md). It should ship as a vertical mission slice rather than as a roster screen without playable assignments.

## Tactical and ship-operation extensions

### Mines

Mines are persistent deployable ordnance with arming delay, trigger rules, detectability, ownership, expiry or recovery, AI use, save behavior and civilian-risk consequences. They build on entity lifecycle, Sensors, faction relationships and signature rules.

### Cloaking

Cloaking extends Band C5 signature management. A cloak suppresses detection rather than granting invisibility, trades power and heat against concealment, restricts high-emission actions and can be defeated by proximity, focused observation, fields or prior tracking. Player and NPC ships use the same rules.

### Expanded EWar

Band C5 supplies the Jam, Spoof and Harden MVP. Later EWar may add remote disruption and access operations, but every effect must remain authored, inspectable and integrated with Sensors, Comms, signatures and modifiers.

### Surrender — scheduled for Band C7

Comms may offer or demand surrender. The receiving actor evaluates authored conditions and may accept, refuse, counteroffer or feign compliance. Acceptance changes objectives, hostility and vessel orders without automatically despawning or transferring ownership. Player surrender is likewise a scenario-authored outcome.

### Self-destruct

Self-destruct uses several dedicated interlocks distributed through human-seeking station surfaces. Every interlock must be activated. If station consolidation gives one player multiple interlocks, they remain separately labelled actions and all still require deliberate activation. A visible cancellable countdown precedes scenario-authored physical and campaign consequences.

## Accessibility assistance and advanced spectators — distributed across T1 and C2–C9

Personal accessibility profiles may delegate named subfunctions to the same limited-AI machinery used by the complexity ladder. Settings describe effects rather than diagnoses and remain private. Station selection shows suitability, explains genuine incompatibility and causes a complete human-seeking station to skip an incompatible player at its required rating. This work now begins with T1's settings, Hero Bar and eligibility seams and advances through every later content band.

Every base hull at full supported player count must retain one accessible station/rating for a simple scenario, without guaranteeing solo operation of complex content. Rich spectator mode adds authorised system-monitor selection in Band C9 beyond T1's summary-screen MVP. The complete distribution is in [Release Bands C7–C9](./release-bands-g-i.md).

## Pursuit — scheduled for Band C7

Pursuit is an authoritative relationship between a pursued actor and a named pursuing force. Between systems the force is abstract and advances against a lead or interception clock; on contact it materialises as ordinary world entities. Warp choices, repairs, detours, signatures, misinformation and local actions change the lead.

The simulation keeps exact state while the crew receives an Intelligence estimate: last confirmed position, likely route, evidence-derived countdown, uncertainty interval and graded interception risk. Stronger evidence narrows the interval. The same system may support a single scenario, Patrol arc or War run.

## War Mode — scheduled for Bands C7–C8

Band C7 ships one authored War run after the authored Patrol MVP. Each run creates one decisive destination mission. The crew jumps through intervening systems, gathering allies and supplies, weakening opponents and changing the conditions of that final mission through ordinary bridge play. Band C8 adds procedural generation for War and Patrol together.

The player remains aboard one ship. Sector control, fronts and fleet disposition are coarse campaign state. Allied ships remain autonomous; the crew issues high-level assignments through Comms, and allies may accept, refuse, negotiate or fail to comply according to authority, standing and circumstances.

## Sandbox Mode — scheduled for Band C9

Sandbox follows War Mode and deepens the shared actor-and-economy layer. Routes, production, consumption, shortages, cargo and faction needs change through discrete events and player action rather than continuous galaxy simulation. Players choose their own assignments in a persistent world.

The shared Director has greater freedom in Sandbox because there is no fixed Patrol assignment chain or War destination objective. It may offer missions from central command, generate crises and advance actors. The mode also provides the foundation for GM-controlled sessions, with the GM able to shape or override Director activity.

## Shared generated-world foundation — scheduled for Bands C6–C9

Band C6 establishes the authored Patrol run shell and mode-profile seam. Band C7 adds authored War and strategic state. Band C8 supplies the shared generator and Director for Patrol and War. Band C9 consumes them for Sandbox under a wider profile. Bulk resources begin in Band C4; Band C9 trade adds discrete cargo lots with type, quantity, owner, destination and scenario tags. Reputation remains layered by actor, faction and institution rather than collapsing into one score.

Warp is discrete system-to-system transit. Warp fuel is distinct from local main-drive propellant but normally abundant; it becomes consequential through aid, loss, sabotage, unusual route costs or authored hazards. Transit may trigger events without simulating continuous interstellar flight.

## Planetary Orbital Mode — scheduled for Band C8

A ship enters Orbital Mode by approaching a planet's transition envelope. Impulse is disabled and normal planar controls move the ship across a coarse wrapping global surface map. Authored or generated points of interest support scans, Comms, Away Missions and external operations.

Leaving Orbital Mode requires maintaining maximum outward thrust for an authored duration before returning to the normal local-space plane. The mode does not add terrain collision, altitude control or a separate atmospheric flight model.

## Moving work into a band

A planned feature enters a release band only when it has a scenario or mode that proves its player value, a bounded authoritative state model, human/AI symmetry where applicable, accessibility and information-boundary behavior, persistence classification and a vertical acceptance test. Moving one cluster does not implicitly schedule the others.

## Canonical sources

- [Phoenix delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml)
- [Future Modes](./future-modes.md)
- [Campaign Continuity](../foundation/campaign-continuity.md)
- [Fields and epistemics design](../../../pasm/spec/design/fields-epistemics.yaml)
