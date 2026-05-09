---
title: Roadmap Overview
type: roadmap
tags: [roadmap, planning, status]
sources: [PRD-001, PRD-017, PRD-022, PRD-036, PRD-051, PRD-066, docs/]
updated: 2026-05-09
---

# Roadmap Overview

A synthesis of where Project Phoenix is, what's in flight, and what's drafted but not yet a PRD. Sourced from the GitHub `PRD`-labeled issues and the `docs/` design drafts.

## Shipped

Closed PRDs whose features are live in `main`:

- **[PRD #1](../sources/prd-001-bridge-simulator.md)** — Lobby, Captain Console, Red Alert, rotating cube viewscreen.
- **[PRD #17](../sources/prd-017-mobile-ux-and-status.md)** — Fullscreen canvas, mobile-friendly UI, connection status bar.
- **[PRD #22](../sources/prd-022-helm-and-game-world.md)** — Helm Console, ship physics, asteroid field, collisions.
- **[PRD #36](../sources/prd-036-captain-view-selector.md)** — Captain View Modes (Fore/Aft/Port/Starboard + Radar).
- **[PRD #51](../sources/prd-051-smoke-test-harness.md)** — Playwright smoke tests with the BroadcastChannel PeerJS shim.
- **[PRD #66 — Weapons & Engineering](../sources/prd-066-weapons-and-engineering.md)** — Tactical (phasers, target lock) and Engineering (repair loop, breakdown queue). Hull integrity, phaser beam events, `AsteroidDestroyed`. Client now a full Bevy/WASM app.

Net result today: **4 consoles** (Captain, Helm, Tactical, Engineering), one ship, destroyable asteroid field, phaser weapons, hull damage, breakdown/repair system, client WASM Bevy UI, fully tested in CI.

## In flight

No open PRDs currently. Next features are in the `docs/` drafts.

## Drafted (no PRD yet)

`docs/` contains design drafts in various states. Each is a candidate for a future PRD.

| Draft | State | Theme |
|---|---|---|
| [Draft 1 — Entity Config Files](../sources/design-01-entity-config-files.md) | full | Asteroids and ships described in data files. |
| [Draft 2 — Game Map](../sources/design-02-game-map.md) | full | Solar systems, planets, streamed asteroid fields. |
| [Draft 3 — Science Console](../sources/design-03-science-console.md) | full | Long-range radar, impulse, system chart on viewscreen. |
| [Draft 4 — Combat Update](../sources/design-04-combat-update.md) | full | Phaser banks, torpedoes, four-quadrant shields. |
| [Draft 5 — Ship's Power](../sources/design-05-ships-power.md) | full | Engineering 6-point power distribution + aux battery. |
| [Draft 6 — Space Stations](../sources/design-06-space-stations.md) | stub | One-line TODO. |
| [Draft 7 — Scenario File](../sources/design-07-scenario-file.md) | stub | One-line TODO. |
| [Draft 8 — Comms Console](../sources/design-08-comms-console.md) | stub | One-line TODO. |
| [Architecture Improvement Notes](../sources/notes-architecture-improvements.md) | note | Per-console message subscriptions. |

## Themes

The drafts cluster into four themes, each with its own roadmap page:

1. **[Console Expansion](./console-expansion.md)** — From 2 consoles to the full bridge crew (Captain, Helm, Weapons, Engineering, Science, Comms).
2. **[Combat & Damage](./combat-and-damage.md)** — Hull, shields, phasers, torpedoes, repair loop, breakdown queue.
3. **[Data-Driven Content](./data-driven-content.md)** — Entity files, scenarios, system maps. Ship the engine, not the content.
4. **[Open Architectural Questions](./open-architectural-questions.md)** — Per-console messaging, scenario lifecycle, viewscreen authority.

## Tensions across drafts

The drafts were written independently and contradict each other in places:

- **Combat scope.** PRD #66 is explicit: *no shields, single hull pool*. Draft 4 adds four-quadrant shields and torpedoes. Resolution: PRD #66 ships first; Draft 4 supersedes for v2.
- **Viewscreen authority.** Today the Captain alone selects view via `SetView`. Draft 3 has the Science console drive system-chart content onto the viewscreen. Either Captain remains the sole authority and Science *requests* a view change, or the model needs to broaden.
- **World streaming.** Today `WorldData` is sent once at game start. Draft 2 streams nearby asteroids continuously. The `radar_dots` pure iterator already filters by range — extending the same iterator to gate spawning is a small step.
- **Power feeds everything.** Draft 5 modulates many tunables (max speed, repair rate, weapon cooldown) by Engineering's power slider. Any PRD landing before Draft 5 should keep its tunables in one place so Engineering can multiply them later.

## Cross-references

- All [sources](../index.md#sources)
- [Project Overview](../concepts/project-overview.md)
