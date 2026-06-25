---
title: Issue #552 — B NPC ships receive Ship marker + Components on spawn
type: source
tags: [ai, npc, ship, spawner, prd-520, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/552
status: shipped
updated: 2026-06-25
---

# Issue #552 — B NPC ships receive Ship marker + Components on spawn

PRD #520 slice B. NPC entities with a `[behaviour]` TOML section now receive the `Ship` marker and all four Ship Components at spawn time, making them first-class Ships in the ECS.

## Changes

- `spawn_entity` in `src/entities/spawner.rs`: when `behaviour.is_some()`, inserts `Ship`, `ShipConfigComponent::default()`, `ShipSystemControlSources` (all-Ai resolver), `ActiveStationRatings::default()`, `CoordinationQueue::default()`
- `operate_tactical_ai` does not yet guard against NPC ships at this slice; that happens in #553

## Smoke test added

`npc_channel3_coordination_is_consumed` — verifies `route_coordination(Ai, Ai) == Consume`.

## Files

- `src/entities/spawner.rs`

## Cross-references

- [PRD #520](./prd-520-ai-ship-unification.md) — parent
- [AI Ship Unification](../concepts/ai-ship-unification.md)
