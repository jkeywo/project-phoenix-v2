---
title: Objectives
type: concept
tags: [world, objectives, ai, captain, gui]
sources: [src/objectives.rs, src/world/server.rs, src/world/dispatch.rs, src/console/comms/server.rs, src/console/captain/server.rs, src/ai/core.rs, assets/worlds/combat_test.toml]
updated: 2026-07-16
---

# Objectives

World triggers and comms responses create mission objectives. Each carries player text, status, targets, an optional AI directive, utility configuration, and a `Mission` or `Doctrine` source.

## Flow

1. `add_objective`, `complete_objective`, and `fail_objective` actions mutate the session-lifetime `ObjectiveManager`.
2. Active objectives are utility-scored from base priority, mandatory bonus, world conditions, modifiers, zero gates, and an optional Captain boost.
3. Backfill Helm and Tactical consume the shared scored pool through system-affinity directives. This is a temporary global bridge, not a per-ship objective blackboard.
4. Comms receives all sorted snapshots. Captain receives mission objectives even at zero utility, while zero-score doctrine objectives are hidden. Ship-specific GUI objective lists render these blackboards.

## Current Risks

- Captain's single boost is global, so it can affect every current AI consumer rather than a selected ship or system.
- Player visibility differs by console: Comms has the complete list while Captain filters doctrine objectives by score.
- Objective IDs are unique for the entire session and completed/failed objectives remain; worlds do not yet provide authoring-time collision validation.
- `Hail` exists as a directive kind but current AI consumption is primarily Helm/Tactical; responsibility and UI treatment are incomplete.

## Desired Changes

- Captain priority must be local to its intended ship or system AI consumer, rather than a session-global boost.
- Comms should use the same doctrine visibility rule as Captain: hide zero-score doctrine objectives while retaining mission objectives.
- World composition validation must include objective IDs and references, rejecting duplicates before activation.
- A world layer owns the objectives it authors; unloading that layer removes those objectives.
- Backfill Comms must consume `Hail` directives and issue the same actions as a player.
