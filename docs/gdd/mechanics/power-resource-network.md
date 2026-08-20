# Project Phoenix — Power and Resource Network

| Field | Value |
|---|---|
| Status | Current allocation mechanic with accepted network extension |
| Scope | Reactor output, battery, power groups, exhaustion, heat and future supply networks |
| Audience | Design, content, UI, simulation and playtest |

Power makes the crew choose which kind of ship they need right now. The current game models a compact allocation problem; the accepted direction grows that into a legible supply-and-demand network without losing the hard consequences of exhausting the ship.

Related documents: [Damage, Diagnosis and Repair](./damage-repair.md), [Movement and Helm](./movement.md), [Targeting and Weapons](./targeting-weapons.md), [Shields](./shields.md), and [Ships and Ship Systems](../systems/ships-and-systems.md).

## Experience goals

- Make power a changing tactical priority rather than a set-and-forget optimisation.
- Let the crew deliberately overdraw when the situation warrants the risk.
- Ensure every allocation change has an understandable effect on another station.
- Keep current and future models authored, host-authoritative and shared by human and AI operators.
- Add depth through connected consequences, not through opaque simulation detail.

## Current allocation model

The current ship exposes three power groups: Helm, Weapons and Shields. Each has an absolute commanded level. Reactor availability supplies the groups and the battery covers excess demand or charges from surplus according to authored limits.

Allocation effects are deliberately concrete. Helm power changes movement performance, Weapons power changes phaser damage, and Shields power changes regeneration. Radar acquisition and authored weapon reach are not hidden consumers of these groups. If another system later gains a power effect, it should be explicit in the authored contract and in the relevant console feedback.

The host owns commanded levels, current battery, charging and draining state, availability and control lock. Human and AI Power operators submit the same group commands.

## Exhaustion and recovery

The crew may command an overdraw. This is a real option, not an invalid input. When demand exceeds generation, the battery drains. When available supply exceeds demand, it may charge. Charging and draining are independently derived states; an exact balance can be neither.

When the battery reaches zero, the current emergency rule forces all groups to level one and locks allocation controls. Control returns only after the battery recovers past an authored threshold. This is intentionally blunt: exhaustion is a ship-wide loss of flexibility rather than a graceful per-group reduction.

The console must forecast the consequence before the crew commits. It should show commanded total, available reactor output, battery direction and the point at which emergency recovery would occur. After exhaustion, it must explain why controls are locked and what will unlock them.

## Crew coordination

Power decisions are meaningful because their benefits live on other stations. Helm asks for acceleration or impulse margin, Tactical asks for damage output, and Shields asks for regeneration. The Power operator weighs those needs against the battery and the scenario clock.

Requests should remain advisory unless a scenario explicitly creates a command relationship. The intended play is a negotiation around priorities, not three stations directly moving their own allocation sliders.

## AI and backfill

AI Power uses authored rules to bid for group levels. Each rule can require a minimum battery reserve, and the planner evaluates the combined plan against an authored maximum commanded total. It does not receive a safer battery model than a human; it simply expresses its risk policy in data.

Backfill should keep the ship viable without flattening every interesting trade-off. Conservative defaults are appropriate for routine play, while scenario or hull policies may accept overdraw under named conditions.

## Accepted supply-and-demand extension

The accepted future model represents generation, storage and consumption as a connected resource network. A reactor supplies components and a battery buffers demand. Systems state their demand and priority; the resolver produces delivered power and an inspectable reason when demand is unmet.

This extension retains hard battery exhaustion while adding graceful local degradation. A system can receive less than requested without forcing every group to minimum. It also introduces per-system heat and over-power behavior: pushing a component can improve output while accumulating heat, and an overload takes that component offline until it cools rather than locking the whole ship.

Coolant and thermal detail should be tiered. The default console presents actionable capacity, heat and recovery. More elaborate simulator installations may expose richer controls, but the underlying model and outcomes stay the same.

Reactor profiles are sidegrades, not a linear upgrade ladder. Authored profiles trade generation, battery capacity, recovery and heat dissipation. Campaign rewards may unlock a sanctioned starbase refit, but ordinary mission progression should not turn early hulls into strictly obsolete versions of themselves.

Band C4 includes life support as a damageable resource-network consumer. Its failure creates authored ship-wide or compartment survival clocks, degraded crewed-system availability, evacuation pressure or emergency-power choices. It does not introduce hunger, sleep, individual oxygen consumption or continuous crew location.

Band C4 presets include hull-authored defaults and a small number of run-local player-adjusted configurations. Applying a preset requests a network state and reports deviations caused by damage or unavailable components. Personal campaign-wide preset persistence remains uncommitted.

Band C4 resources are named bulk amounts with capacity, transfer and consumption. Main-drive propellant and repair material enter here; coolant remains detail-tier and batteries remain part of the energy model. Later generated-mode work adds discrete cargo lots without turning inventory into a slot grid.

## Resource-network boundary

The generic node vocabulary can also describe external scenario infrastructure: producers, conduits, buffers and consumers with capacity, dependency and priority. Ship power and external infrastructure should converge on compatible concepts without sharing presentation or allowing a scenario script to bypass ship-system authority.

The design does not require continuous electrical flow simulation. Resolution should be deterministic, static-topology and as simple as needed to answer which consumers receive how much capacity and why.

## Authoring and tuning

Current group limits, reactor output, battery behavior, emergency threshold, modifiers and AI rules belong in hull TOML. Future nodes, demand curves, heat limits, cooling rates and reactor profiles must also be data-authored. Display text remains in the string catalogue.

## Playtest questions

- Can the Power operator predict whether an allocation drains, balances or charges the battery?
- Do Helm, Tactical and Shields notice and discuss the effects of allocation changes?
- Is overdraw tempting in the right moments, and is exhaustion severe without feeling arbitrary?
- Does AI Power preserve interesting trade-offs rather than pinning one universal optimum?
- In the future network model, can a player explain every underpowered or overheated system from the console state?
- Do reactor profiles create different plans without producing a dominant upgrade path?

## Canonical sources

- [Power design](../../../pasm/spec/design/power.yaml)
- [Power, modifiers and regions architecture](../../../pasm/spec/architecture/power-modifiers-regions.yaml)
- [Ship customisation design](../../../pasm/spec/design/ship-customisation.yaml)
- [Power plugin wiki](../../../wiki/concepts/power-plugin.md)
