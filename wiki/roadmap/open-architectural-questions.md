---
title: Open Architectural Questions
type: roadmap
tags: [roadmap, architecture, open-questions, messaging, viewscreen, scenario]
sources: [docs/Architecture Improvement Notes.md, docs/2.md, docs/3.md, docs/7.md]
updated: 2026-05-08
---

# Open Architectural Questions

Cross-cutting decisions that aren't owned by any single PRD or draft. Each one will block or shape multiple future features.

## 1. Per-console message subscription

**Source:** [Architecture Improvement Notes](../sources/notes-architecture-improvements.md). Anticipated by [PRD #66](../sources/prd-066-weapons-and-engineering.md)'s `Target::One(token)` routing.

**Question:** Today every client receives every broadcast (`Target::All`). As consoles multiply, each phone receives messages it has no use for. Should the server filter outbound messages by *what kind of message* a client cares about, on top of *who* it's for?

**Sub-questions:**
- Subscription declared in code (per Console Plugin) or in data?
- Server keeps a `client → subscription set` map, or each client filters incoming itself?
- Granularity: per-message-type, per-category (ship-wide / nearby-entity / console-specific), or per-field?

**Decision needed before:** Science Console (heavy long-range radar payload) and any feature that pushes messages > 10 Hz.

## 2. Viewscreen authority model

**Source:** [Draft 3 — Science Console](../sources/design-03-science-console.md), interacting with [PRD #36 — Captain View Selector](../sources/prd-036-captain-view-selector.md).

**Question:** Today only the Captain selects what's on the viewscreen via `SetView`. Draft 3 has the Science Console drive system-chart content there. Two models:

- **Captain remains sole authority.** Science *requests* a view change; Captain accepts or rejects. Preserves the chain-of-command fiction; adds friction.
- **Viewscreen accepts content from any console.** Last writer wins, or a per-content-type slot. Easier to build, weakens Captain's role.

**Decision needed before:** Science Console.

## 3. Scenario lifecycle

**Source:** [Draft 7 — Scenario File](../sources/design-07-scenario-file.md) (stub).

**Question:** What's the lifecycle of a session? Today: Captain presses Engage → game runs forever → players close tabs. A scenario implies start conditions, objectives, win/fail terminals.

**Sub-questions:**
- Where does the scenario file live? Bundled with the build, fetched at runtime, uploaded by Captain?
- Is the lobby the place to pick a scenario, or does a single deploy = single scenario?
- What does "ship destroyed" mean? PRD #66 explicitly excludes game-over; Draft 7 implies it.

**Decision needed before:** any combat with consequence, any non-trivial deployment.

## 4. World streaming model

**Source:** [Draft 2 — Game Map](../sources/design-02-game-map.md), interacting with [World Data](../entities/world-data.md).

**Question:** Today `WorldData` is sent once at game start (full asteroid list). Draft 2 streams nearby asteroids only. Two upgrade paths:

- **Snapshot + delta.** Send full snapshot once, then `AsteroidSpawned` / `AsteroidDespawned` deltas as the ship moves.
- **Per-tick visibility list.** Send the visible-now list every tick; client diffs locally.

`radar_dots` already filters by range — same pattern works for spawn gating. Per-tick list is simpler but bandwidth-heavy as world grows.

**Decision needed before:** Draft 2 implementation, multi-system maps.

## 5. Entity ID lifecycle

**Source:** [PRD #66](../sources/prd-066-weapons-and-engineering.md) introduces UUIDs on asteroids.

**Question:** Once entities have IDs, what's the contract? Are IDs stable across reconnect? Across save/load (when scenarios introduce save points)? Server-authoritative or hashed from seed?

**Decision needed before:** anything that references an entity by ID across messages — target lock, damage reports, repair assignments.

## 6. Power as a global multiplier

**Source:** [Draft 5 — Ship's Power](../sources/design-05-ships-power.md).

**Question:** Power affects max speed, weapon cooldown, shield regen, repair rate, sensor range. Where does the multiplier live?

- **Distributed.** Each subsystem reads `PowerState` and scales itself. Hard to reason about overall ship performance.
- **Centralised tuning struct.** One `EffectiveTunables` resource the server recomputes when power changes; subsystems read static values. Easier to reason, requires invalidation discipline.

**Decision needed before:** Draft 5 implementation. Affects how much PRD #66's constants need refactoring.

## How to close one of these

When a question is answered:

1. Land the decision (PRD, ADR, or merged code with rationale in commit message).
2. Update this page: move the question to a `## Resolved` section with a one-line answer + link.
3. Update affected entity/concept pages.
4. Append a `log.md` entry.

## Cross-references

- All four [Roadmap](./overview.md) themes touch at least one question here.
- [Message Flow](../concepts/message-flow.md), [Console Plugin Pattern](../concepts/console-plugin-pattern.md), [View Modes](../concepts/view-modes.md)
