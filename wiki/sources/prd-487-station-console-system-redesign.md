---
title: PRD #487 - Station / Console / System architecture redesign
type: source
tags: [prd, stations, systems, consoles, ai, damage, power, wire]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/487
status: open
updated: 2026-06-22
---

# PRD #487 - Station / Console / System architecture redesign

## Status

Open. Issue #488 is the first foundational contract slice.

## Problem

The current game treats a monolithic `Console` as lobby assignment, GUI,
authority boundary, AI delegation unit, and damage slot. The player ship TOML
also carries one station layout per player count plus `next`/`previous`
promotion links, making crew composition brittle and hard to tune.

## Solution

Split the ship model into:

- **Station** - fixed roster seat a player can claim.
- **Console** - the cohesive GUI owned by one station.
- **System** - the fine-grained capability instance: helm movement, phaser bank,
  torpedo tube, magazine, radar, repair, power, viewscreen, comms, and so on.

Systems can be human-controlled or AI-controlled. Per-station ratings define
which systems the AI operates. Unclaimed or disconnected stations automate all
their systems, so an AI ship is the same model with no human-held stations.

## Key decisions

- Replace per-player-count station layouts with one fixed roster.
- Use stable ship-wide `SystemId`s and station-scoped `StationId`s instead of
  `Console` as the future addressing model.
- Ratings are explicit per-station data tables listing automated systems.
- Ownerless systems are valid only when explicitly `ai_only`; they live at
  Core for repair/control purposes.
- Power is allocated by operator-facing group, then resolved to member systems.
- System control uses typed payloads sent to a target `SystemId`.
- Damage is per system and orthogonal to human/AI control.
- Cross-system interaction is limited to state reads, authoritative sim-level
  messages, and a lagged AI coordination bus.

## Open user stories

The PRD covers fixed station claiming, mid-game rating changes, AI backfill,
disconnect takeover, reconnect handoff, granular system damage, station/Core
repair dispatch, viewscreen arbitration, power groups, and system-fragment GUI
composition.

## Cross-references

- [Issue #488 - Station/System ADR](./issue-488-station-system-adr.md)
- [ADR-0002](../../docs/adr/0002-station-system-ship-config-contract.md)
- [Console](../entities/console.md)
- [player_ship.toml](./player_ship_toml.md)
