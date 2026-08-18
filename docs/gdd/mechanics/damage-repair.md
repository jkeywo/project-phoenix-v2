# Project Phoenix — Damage, Diagnosis and Repair

| Field | Value |
|---|---|
| Status | Current mechanic |
| Scope | Damage routing, system availability, diagnostic visibility, repair teams and defeat |
| Audience | Design, content, UI, simulation and playtest |

Damage is both material failure and an information problem. The whole crew can feel that the ship is hurt, station holders can inspect their own machinery, and Engineering must spend scarce team time to discover and repair exact failures elsewhere.

Related documents: [Power and Resource Network](./power-resource-network.md), [Shields](./shields.md), [Station Experiences](../systems/station-experiences.md), [AI and Backfill](../systems/ai-and-backfill.md), and [Difficulty, Balance and Playtesting](../foundation/difficulty-balance-playtesting.md).

## Experience goals

- Make damage change what the crew can do and what they need to discuss.
- Give Engineering an active diagnosis-and-allocation game rather than a universal health-bar panel.
- Preserve useful local expertise: each station knows its own machinery before Engineering does.
- Make travel time and team capacity meaningful without burying the crew in maintenance chores.
- Use the same damage and repair rules for player ships and NPCs wherever the entity supports them.

## Damage path

Collision, weapon and environmental damage enter one authoritative route. Applicable shield arcs absorb first, modified by geometry and pierce. Leaked damage is distributed across repairable system hull. Each fine system has current and maximum hull and crosses authored availability tiers as it deteriorates.

Damage tiers determine whether a system is degraded, disabled or destroyed. A damaged system cannot accept or apply controls that its availability forbids. This converts hull loss into a concrete station problem: reduced propulsion, a dead weapon bank, failed sensors or another loss grounded in the ship's authored topology.

The player ship is defeated when all repairable system hull is destroyed. NPC ships use the same material damage truth but normally despawn and emit destruction events rather than taking the local crew through the Game Over flow.

## Information model

Everyone may receive aggregate ship hull condition and destroyed fraction. Engineering always receives aggregate condition and exact Core detail. A station holder receives exact detail for systems owned by that station.

Engineering does not receive exact non-Core detail merely because damage exists. A repair team must arrive at the station before its internal systems and repair priorities are revealed. A team that is still travelling reveals nothing. When the team leaves, that privileged detail disappears again.

This asymmetry creates the intended conversation: a station can report symptoms and exact local readings; Engineering decides whether that report justifies diverting a team; arrival turns uncertainty into a repair plan.

## Repair-team cycle

Each team is authoritative ship state with travelling, on-site and returning phases. Engineering dispatches to a station-level target or Core, not to a hidden internal subsystem. Travel time delays both repair and diagnosis.

Once on site, the team repairs at an authored rate. Engineering can prioritise which revealed internal subsystem is repaired first; otherwise the host resolves the target according to the common repair policy. When the station no longer requires work, the team returns and becomes available again.

Team count, travel time and repair rate are tuning levers. More teams increase parallelism, while longer travel increases the cost of guessing. These values should produce consequential triage, not long periods where Engineering has made every available decision and can only wait.

## Requests and station ownership

Damage requests are advisory. A station can ask Engineering for attention, and AI-owned stations should issue the same level-three coordination request when their authored policy judges repair necessary. Engineering remains free to choose a different priority.

The request does not reveal hidden detail that Engineering is not otherwise entitled to see. It communicates need, not a remote diagnostic dump. A player at the damaged station should be able to give richer human context over voice.

## Repair limits and failure states

Repair cannot make a destroyed or otherwise non-repairable target operable unless the authored damage model explicitly allows it. Dispatch can also be refused when no team is free, the target is invalid, the station has no repairable damage or control authority is absent. Each refusal should be shown as a reason.

The Core is a dedicated Engineering target because ship-wide failure needs a repair path that does not depend on another station's ownership. Core detail remains Engineering-only.

## AI and backfill

AI Engineering sees the same permitted repair picture, dispatches the same teams and uses admitted repair commands. It may use authored priorities to handle routine failures, but it should not gain exact remote damage information that a human Engineering player would lack.

When a human reclaims Engineering, all team positions, revealed on-site detail and ongoing work remain authoritative continuity. The handover is a change of decision source, not a reset of repairs.

## Authoring and tuning

System hull, tier thresholds, repairability, station ownership, team count, travel times, rates and AI policies belong in entity TOML. Scenario and region content may apply damage through the common route, but should not mutate console-visible health independently of authoritative damage state.

## Presentation and accessibility

Damage state must not rely on colour alone. Use labels, icons, ordering and motion sparingly to distinguish degraded, disabled, destroyed, travelling, repairing and returning states. Aggregate condition, exact local detail and Engineering's revealed detail should look deliberately different so players understand the information boundary.

## Playtest questions

- Do station players report failures to Engineering without being prompted by the facilitator?
- Can Engineering explain what it knows, what it suspects and what requires a team to confirm?
- Does team travel create triage rather than idle frustration?
- Are degraded, disabled and destroyed systems meaningfully different in play?
- Can a returning player understand ongoing repair work immediately after reconnecting?
- Does AI backfill preserve the same diagnostic limits as a human?

## Canonical sources

- [Engineering and damage architecture](../../../pasm/spec/architecture/engineering-damage.yaml)
- [Repair diagnosis scenario model](../../../pasm/spec/scenarios/repair-diagnosis.yaml)
- [Damage and repair intent wiki](../../../wiki/concepts/damage-and-repair-intent.md)
