# Project Phoenix — Planned but Not Scheduled

| Field | Value |
|---|---|
| Status | Accepted direction without a release band |
| Scope | Dependency-grouped features that do not currently fit Bands B–F |
| Audience | Design, roadmap, content and future planning |

These features are accepted parts of Phoenix's long-term design but have no promised release band. Their order below expresses dependencies, not priority or delivery sequence. Work should enter a band only when its supporting systems, scenario proof and release capacity are clear.

Related documents: [Future Modes](./future-modes.md), [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md), [Command and Crew Control](../mechanics/command-and-crew-control.md), [Campaign Continuity](../foundation/campaign-continuity.md), and [World and Environmental Systems](../systems/world-environmental-systems.md).

## Duty Teams, Away Missions and Medical

The shared personnel foundation comprises typed anonymous Duty Teams, named Duty Officer leaders, system assignments, personnel consequences, multiple Operations systems, shuttle/transporter deployment, branching off-screen Away Missions, boarding, internal defence and Medical. It depends on campaign persistence, the station database framework and normal authoritative scenario effects.

This cluster is detailed in [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md). It should ship as a vertical mission slice rather than as a roster screen without playable assignments.

## Tactical and ship-operation extensions

### Mines

Mines are persistent deployable ordnance with arming delay, trigger rules, detectability, ownership, expiry or recovery, AI use, save behavior and civilian-risk consequences. They build on entity lifecycle, Sensors, faction relationships and signature rules.

### Cloaking

Cloaking extends Band E signature management. A cloak suppresses detection rather than granting invisibility, trades power and heat against concealment, restricts high-emission actions and can be defeated by proximity, focused observation, fields or prior tracking. Player and NPC ships use the same rules.

### Expanded EWar

Band E supplies the Jam, Spoof and Harden MVP. Later EWar may add remote disruption and access operations, but every effect must remain authored, inspectable and integrated with Sensors, Comms, signatures and modifiers.

### Surrender

Comms may offer or demand surrender. The receiving actor evaluates authored conditions and may accept, refuse, counteroffer or feign compliance. Acceptance changes objectives, hostility and vessel orders without automatically despawning or transferring ownership. Player surrender is likewise a scenario-authored outcome.

### Self-destruct

Self-destruct uses several dedicated human-seeking interlock systems distributed across stations. Every interlock must be activated. If station consolidation gives one player multiple interlocks, they remain separately labelled actions and all still require deliberate activation. A visible cancellable countdown precedes scenario-authored physical and campaign consequences.

## Accessibility assistance and advanced spectators

Personal accessibility profiles may delegate named subfunctions to the same limited-AI machinery used by the complexity ladder. Settings describe effects rather than diagnoses and remain private. Station selection shows suitability, explains genuine incompatibility and causes human-seeking systems to skip an incompatible player.

Every base hull at full supported player count must retain one accessible station/rating for a simple scenario, without guaranteeing solo operation of complex content. Rich spectator mode adds authorised system-monitor selection beyond Band A2's summary-screen MVP.

## Pursuit

Pursuit is an authoritative relationship between a pursued actor and a named pursuing force. Between systems the force is abstract and advances against a lead or interception clock; on contact it materialises as ordinary world entities. Warp choices, repairs, detours, signatures, misinformation and local actions change the lead.

The simulation keeps exact state while the crew receives an Intelligence estimate: last confirmed position, likely route, evidence-derived countdown, uncertainty interval and graded interception risk. Stronger evidence narrows the interval. The same system may support a single scenario, Patrol arc or War run.

## War Mode

War Mode follows the Patrol MVP and reuses its seeded generator and Director. Each run creates one decisive destination mission. The crew jumps through intervening systems, gathering allies and supplies, weakening opponents and changing the conditions of that final mission through ordinary bridge play.

The player remains aboard one ship. Sector control, fronts and fleet disposition are coarse campaign state. Allied ships remain autonomous; the crew issues high-level assignments through Comms, and allies may accept, refuse, negotiate or fail to comply according to authority, standing and circumstances.

## Sandbox Mode

Sandbox follows War Mode and deepens the shared actor-and-economy layer. Routes, production, consumption, shortages, cargo and faction needs change through discrete events and player action rather than continuous galaxy simulation. Players choose their own assignments in a persistent world.

The shared Director has greater freedom in Sandbox because there is no fixed Patrol assignment chain or War destination objective. It may offer missions from central command, generate crises and advance actors. The mode also provides the foundation for GM-controlled sessions, with the GM able to shape or override Director activity.

## Shared generated-world foundation

Patrol, War and Sandbox use one generator and Director with different mode profiles. Bulk resources begin in Band D; later trade adds discrete cargo lots with type, quantity, owner, destination and scenario tags. Reputation remains layered by actor, faction and institution rather than collapsing into one score.

Warp is discrete system-to-system transit. Warp fuel is distinct from local main-drive propellant but normally abundant; it becomes consequential through aid, loss, sabotage, unusual route costs or authored hazards. Transit may trigger events without simulating continuous interstellar flight.

## Planetary Orbital Mode

A ship enters Orbital Mode by approaching a planet's transition envelope. Impulse is disabled and normal planar controls move the ship across a coarse wrapping global surface map. Authored or generated points of interest support scans, Comms, Away Missions and external operations.

Leaving Orbital Mode requires maintaining maximum outward thrust for an authored duration before returning to the normal local-space plane. The mode does not add terrain collision, altitude control or a separate atmospheric flight model.

## Moving work into a band

A planned feature enters a release band only when it has a scenario or mode that proves its player value, a bounded authoritative state model, human/AI symmetry where applicable, accessibility and information-boundary behavior, persistence classification and a vertical acceptance test. Moving one cluster does not implicitly schedule the others.

## Canonical sources

- [Phoenix delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml)
- [Future Modes](./future-modes.md)
- [Campaign Continuity](../foundation/campaign-continuity.md)
- [Fields and epistemics design](../../../pasm/spec/design/fields-epistemics.yaml)
