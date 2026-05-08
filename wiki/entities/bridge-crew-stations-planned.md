---
title: Bridge Crew Stations (planned)
type: entity
tags: [console, planned, weapons, engineering, science, comms, roadmap]
sources: [PRD-066, docs/3. Draft Design - Science Console.md, docs/8. Draft Design - Comms console.md]
updated: 2026-05-08
---

# Bridge Crew Stations (planned)

Consoles that have a PRD or design draft but are not yet shipped.

## Weapons (PRD #66 — open)

- Tap-to-lock targeting on a 60-unit ship-aligned radar.
- Lock works in 360°. Fire requires 40-unit range **and** 180° forward arc.
- Phaser beam: 6 s duration, 5 dmg/s = 30 total per beam, then a 6 s cooldown.
- Beam severs immediately on target destroyed, target out-of-arc, or target out-of-range.
- Active beam rendered on the server viewscreen as a line/glow.

See [PRD #66](../sources/prd-066-weapons-and-engineering.md).

## Engineering (PRD #66 — open)

- Hull integrity readout: **N/100** + 10-segment progress bar.
- Owns the breakdown queue. Each 10 HP lost queues one breakdown assigned to a random *other* console (never the same console twice in a row).
- Engineering sees one active breakdown at a time.
- All consoles get a Repair button — but the wrong console pressing it triggers a **30 s red-flash penalty cooldown**. The right console heals 1 HP / 3 s for 30 s (+10 HP). This is intentional: the crew has to *talk*.

Power management from Draft 5 also belongs here (6 distributable points, aux battery, Engineering chooses Helm/Weapons/Science allocations).

## Science (Draft 3 — design only)

- **Long-range radar** showing only stars and planets (sendable to viewscreen).
- **Target highlighting** for Weapons.
- **Impulse drive** authority — Helm requests 6 s charge for 10× speed; Science can cancel it.
- **System chart** as a second tab — first design that explicitly proposes multi-tab consoles.

See [Draft 3](../sources/design-03-science-console.md).

## Comms (Draft 8 — stub)

Source is a `-TODO-`. No mechanics drafted yet. Captured here so it doesn't get forgotten.

## Open questions

- All these consoles depend on per-console message routing (the
  [Architecture Improvement Note](../sources/notes-architecture-improvements.md))
  to avoid every client receiving every message. PRD #66 mentions
  `Target::One(token)` direct routing — the abstraction may already exist.
- Multi-tab consoles (Draft 3) are a new UI pattern not yet in the [Console Plugin Pattern](../concepts/console-plugin-pattern.md).

## Related

- [Console](./console.md) — current shipped enum
- [Console Expansion roadmap](../roadmap/console-expansion.md)
- [Combat & Damage roadmap](../roadmap/combat-and-damage.md)
