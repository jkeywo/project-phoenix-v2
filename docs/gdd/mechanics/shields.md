# Project Phoenix — Shields

| Field | Value |
|---|---|
| Status | Current mechanic |
| Scope | Directional shield arcs, focus, damage routing, collapse and recovery |
| Audience | Design, content, UI, simulation and playtest |

Shields turn the direction of danger into a crew decision. The grid is divided into authored arcs, incoming damage strikes the matching available facing, and the Shields operator can focus protection at a cost elsewhere.

Related documents: [Targeting and Weapons](./targeting-weapons.md), [Movement and Helm](./movement.md), [Power and Resource Network](./power-resource-network.md), [Damage, Diagnosis and Repair](./damage-repair.md), and [Station Experiences](../systems/station-experiences.md).

## Experience goals

- Make facing and incoming-fire patterns immediately relevant.
- Give Shields a proactive decision without requiring constant rapid input.
- Create coordination with Helm, Power and Tactical through visible consequences.
- Make collapse and recovery long enough to generate manoeuvre decisions.
- Apply identical defensive rules to player and NPC ships.

## Arc model

Each ship authors its own set of shield arcs, including geometry, priority, maximum health, regeneration, offline duration and fine-system availability. A conventional hull may use forward, aft, port and starboard facings, but the mechanic does not require that exact layout.

Incoming collision, weapon and regional damage is matched against online arcs by geometry and priority. The selected arc absorbs effective damage until depleted; shield pierce and any remaining amount continue to the system-damage route. A missing, offline or unmatched facing protects nothing.

The Shields console receives aggregate grid state and per-arc geometry, health, availability and focus. Presentation should preserve the ship-relative spatial relationship rather than reducing the grid to an unordered list.

## Focus

The operator may focus one authored arc or clear focus. Focus applies the hull's configured protection and trade-off multipliers. It is not a universal bonus: strengthening the expected impact direction should expose a cost elsewhere or consume another authored constraint.

A human Shields operator owns focus while the system is human-controlled. AI cannot override that choice. Sensors may provide information through ordinary crew communication, but there is no privileged Sensors-to-Shields threat-bearing command.

## Collapse and recovery

When a facing reaches zero it collapses, goes offline for its authored duration and absorbs nothing. After the delay it returns online at zero health and regenerates upward at its authored rate. It does not snap back to full.

A recovering facing struck at zero by effective damage collapses again and receives a fresh offline period. Sustained fire can therefore keep a breach open. This creates a meaningful interval where Helm can turn another facing toward the enemy, Power can favour shield regeneration, or the crew can break contact.

The console must distinguish offline delay, online-at-zero recovery and ordinary partial depletion. These states have different tactical answers even though each may display very little current health.

## AI and backfill

AI Shields evaluates recent incoming damage over an authored window. If one arc receives a sufficient share, it focuses that arc. When damage is not concentrated, it may use current health imbalance as a fallback; otherwise it clears focus.

The policy runs on the shared fixed AI cadence and emits the same admitted focus command as a human. It acts only while Shields is AI-controlled and cannot consult a hidden predictive threat feed unavailable to the crew.

## Coordination

Helm can change which arc faces danger. Power can change shield regeneration. Tactical can create windows by disabling an attacker or forcing its orientation. Shields communicates the grid's immediate need: hold this facing, turn the damaged side away, or buy time for recovery.

No one of those stations should solve the entire defence alone. Scenarios should combine incoming direction, multiple threats, movement constraints and resource pressure so the grid becomes shared tactical language.

## Authoring and tuning

Arc geometry and priority, maximum health, regeneration, offline duration, focus multipliers, damage-history window, concentration threshold and AI fallback belong in hull TOML. Weapon pierce belongs with the weapon. Scenario effects use the common damage path rather than directly editing a console arc.

## Presentation and accessibility

Arc state must be readable without relying on red, amber and green alone. Use shape, labels, fill, outline and explicit state text. Direction labels should remain understandable on small screens and under ship rotation. Focus needs a distinct marker from merely having the highest health.

## Playtest questions

- Can Shields identify the threatened facing before impact and the struck facing afterward?
- Does focus create a real trade-off rather than an obvious permanent setting?
- Do collapse and zero-to-full recovery create useful break-contact decisions?
- Can Helm and Shields coordinate using the same directional language?
- Can players distinguish an offline arc from an online arc that has just returned at zero?
- Does AI focus plausibly from recent damage without appearing prescient?

## Canonical sources

- [Shields design](../../../pasm/spec/design/shields.yaml)
- [Shields architecture](../../../pasm/spec/architecture/shields.yaml)
- [Shields intent wiki](../../../wiki/concepts/shields-intent.md)
- [Weapons architecture](../../../pasm/spec/architecture/weapons.yaml)

