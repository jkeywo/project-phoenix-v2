# Project Phoenix — Targeting and Weapons

| Field | Value |
|---|---|
| Status | Current mechanic with accepted refinements |
| Scope | Combat lock, weapon families, readiness, firing and bearing coordination |
| Audience | Design, content, UI, simulation and playtest |

Weapons turn shared situational awareness into deliberate commitments. The crew acquires one combat lock, judges which authored weapon family can act, fires through the host-authoritative simulation, and coordinates with Helm when geometry is the obstacle.

Related documents: [Movement and Helm](./movement.md), [Shields](./shields.md), [Sensors and Epistemics](./sensors-epistemics.md), [Ships and Ship Systems](../systems/ships-and-systems.md), and [Combat Test](../content/scenarios/combat-test.md).

## Experience goals

- Give Tactical a legible cycle of acquire, assess, commit and observe.
- Make phasers, blasters and torpedoes tactically distinct rather than cosmetic variants.
- Use arcs, range, readiness and ammunition to create coordination with Helm and Power.
- Keep one combat lock per ship so human and AI crews operate under the same targeting constraint.
- Expose refusal and blocking reasons clearly enough that a missed shot never feels arbitrary.

## Combat-lock authority

The tactical radar owns one authoritative Combat Lock per ship. Human selection and AI target choice travel through the same admitted `SetTarget` command and the same applier. Destroyed, invalid or no-longer-acquirable targets clear through the same model.

The lock is an acquisition fact, not a guarantee that every weapon can reach or fire. Radar horizon may be affected by the ship's sensor state, while each weapon's authored reach remains its own property. Low sensor power can make acquiring a distant target harder without shortening a projectile or beam after the target is known.

Sensors has a separate Science Target. It can send a designation to Tactical, but it cannot overwrite the Combat Lock. This distinction allows Science to advise without giving one station two roles' authority.

## Weapon families

### Phasers

Phasers are sustained locked beams. A bank must be online, ready, in range and within its authored arc when fire begins. The beam applies damage over its active period and then enters a per-bank cooldown. Frequency and shield-pierce behavior are part of the authoritative firing state.

Phasers reward maintaining geometry and choosing when to open a sustained attack. The accepted refinement is to latch the target when the attack begins so a later lock change cannot redirect a beam already in progress.

### Blasters

Blasters fire predicted straight-line projectile volleys. They do not home. A bank authors projectile speed, lifetime or range, collision radius, damage, shield pierce, cooldown and optional hold-to-charge behavior.

Blasters reward lead prediction and volume of fire. Their presentation must make the predicted firing solution and charge state visible; otherwise their misses look like network or targeting failures rather than the consequence of firing a non-homing weapon.

### Torpedoes

Torpedoes are guided proximity projectiles. Individual tubes have firing arcs and reload state, while a shared magazine supplies ammunition. Damage and shield-pierce values are captured at launch so an in-flight torpedo does not change because the firing ship's later state changes.

Torpedoes reward ammunition judgment, tube readiness and attack geometry. The in-flight count is public authoritative state so consoles and AI policies can reason about salvos without inferring them from transient launch messages.

Band E adds authored torpedo payload sidegrades through Tactical calibration and ammunition fabrication. Payloads may trade shield interaction, armour penetration, disruption, marking or another explicit effect rather than forming a simple light-to-heavy damage ladder. A tube loads a selected available payload, and its behavior remains frozen at launch.

## Readiness grammar

Every emitter should present a common top-level answer: ready, offline, cooling or loading, out of range, out of arc, blocked by ammunition, or blocked by another authored condition. Family-specific detail can sit underneath that shared grammar.

Readiness is evaluated by the host. The client may preview geometry but cannot declare that a shot landed or that ammunition was spent. Commands rejected between display and execution should return a clear current reason rather than fail silently.

## Arc-bearing coordination

When a capable weapon family has at least one usable emitter that is in range but out of arc, and none in that family is ready, Tactical may request a bearing from Helm. The request carries the family and its usable arcs. It clears when the target changes, leaves reach, enters a usable arc or no longer exists.

Which family gets priority is an authored weapons-doctrine decision. A beam-oriented ship may ask to present phasers first; a torpedo doctrine may turn for loaded tubes when a shield facing is down. The request changes facing intent only and never commands thrust.

## Damage, shields and power

Incoming weapon damage routes through matching online shield arcs before leaked damage reaches repairable systems. Shield pierce is authored per weapon. Weapon-system damage can make an individual bank or tube unavailable without disabling every Tactical function.

Power currently modifies phaser damage but not authored weapon reach. Other weapon restrictions remain explicit properties of their bank, tube, magazine and control systems. Future power-network work may deepen demand and heat trade-offs, but it must keep readiness reasons inspectable.

## AI and backfill

AI Tactical chooses one target through an authored ranking policy, then operates eligible banks and tubes through ordinary admitted commands. Per-bank and per-tube policies decide whether to fire; an offline or intentionally idle emitter does not disarm its siblings. Low-detail NPCs must retain baseline combat capability rather than cease firing because a presentation-oriented simulation tier changed.

AI sees the same Combat Lock and weapon geometry as a human. It must not split fire across multiple independent hidden locks or fire through arcs that a player would have to manoeuvre to expose.

## Authoring and tuning

Weapon statistics, arcs, bank and tube topology, magazine capacity, cooldowns, projectile behavior, frequency behavior, shield pierce and AI policies belong in ship/entity TOML. Scenario scripts may create targets, relationships and objectives, but should not special-case how a named hull fires.

Persistent mines are accepted but unscheduled. They require arming, trigger, detection, ownership, expiry/recovery, AI, save and civilian-risk contracts and therefore sit outside Band E's torpedo-payload work. See [Planned but Not Scheduled](../future/planned-not-scheduled.md).

## Playtest questions

- Can Tactical tell why every unavailable weapon cannot currently fire?
- Do the three families produce recognisably different decisions and crew calls?
- Is the single Combat Lock a useful focus or an avoidable source of friction?
- Does the arc-bearing request help Helm without taking over Helm's job?
- Can players understand blaster prediction and torpedo ammunition without reading implementation detail?
- Does AI obey the same target and firing constraints as the crew?

## Canonical sources

- [Weapons architecture](../../../pasm/spec/architecture/weapons.yaml)
- [Shields architecture](../../../pasm/spec/architecture/shields.yaml)
- [Radar and sensors architecture](../../../pasm/spec/architecture/radar-sensors.yaml)
- [Weapons intent wiki](../../../wiki/concepts/weapons-intent.md)
- [Combat Test world](../../../assets/worlds/combat_test.toml)
