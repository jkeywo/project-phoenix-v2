---
title: PRD #597 — Ship Parity: Eliminate All Player/NPC Divergences
type: source
tags: [ship, npc, parity, refactor, unification]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/597
updated: 2026-07-01
---

## Status

**In progress** — PR 1 underway.

## Problem

After PRD #581 unified the `Ship` marker and per-entity blackboards, a large number of divergences remain:

- **Critical bugs**: `With<Ship>` `.single()` calls that silently break when multiple ships exist; NPC-vs-NPC beams route damage to the player ship; torpedoes bypass player shields.
- **Data model mismatches**: `ShipShields` (player, n-facing) vs `EntityShield` (NPC, 1-facing); `ShipConfigComponent::default()` loads `player_ship.toml` for every NPC.
- **Feature gaps**: NPCs immune to collisions, region effects, modifiers, repair; all config resources are player-only singletons.

## Solution

Eliminate every divergence in 10 sequential PRs. After all 10: a ship is a ship. The only differences are `ShipSystemControlSources` (AI vs human control per station) and `LocalShip` (render/broadcast gate).

## PR Status

| PR | Title | Status |
|---|---|---|
| PR 1 | Fix critical `With<Ship>` regressions | **Done** (commit ebc0022) |
| PR 2 | Unified `ShipShields` (configurable `num_facings`) | **Done** |
| PR 3 | Per-entity `ShipConfigComponent` from each TOML | Pending |
| PR 4 | Physics/impulse/boost/bank config → per-entity | **Done** (2026-07-01) |
| PR 5 | Weapons/torpedo/phaser config → per-entity | Pending |
| PR 6 | Power/modifier/repair state → per-entity only | Pending |
| PR 7 | Weapons/sensors/navigation state → per-entity; unified beam system | Pending |
| PR 8 | Collision handling for all ships | Pending |
| PR 9 | Region effects for all ships | Pending |
| PR 10 | Combat activity per-entity; delete `ShipHullIntegrity`; cleanup | Pending |

## PR 1 Detailed Scope

Fix all bugs introduced when NPCs gained the `Ship` marker (PRD #581) but code was not updated:

- `tick_npc_beams` `ship_physics_q.single()` on `With<Ship>` → source-ship physics from own entity
- `tick_npc_beams` target identification: `player_ship_q.iter().any()` on `With<Ship>` → `Has<LocalShip>` check on resolved target
- `tick_npc_beams` `hull_query` filter excluded NPCs → allow NPC-vs-NPC beam damage
- `on_beam_started`/`on_beam_ended` `player_ship_q.single()` on `With<Ship>` → `With<LocalShip>`
- `handle_npc_beam_fire` `player_ship_q` `With<Ship>` misidentification → `Has<LocalShip>`
- `handle_slow_zone_speed_clamp` `ship_query.single_mut()` → `ship_query.get_mut(trigger.subject)`
- `src/server/pfx.rs` NPC engine trail filter `Without<Ship>` now always empty → use `With<AiControllerComponent>`
- `handle_coordination_enqueue` / `process_coordination_lag` → `With<LocalShip>`
- `handle_station_rating_change` → `With<LocalShip>`
- `process_lobby` / `handle_disconnect` ship query → `With<LocalShip>` + `single_mut()`

## Key Decisions

- Per-entity components are the sole source of truth. All `#[derive(Resource, Component)]` dual-derives eliminated by end of PR 7.
- Each ship reads its own config from its TOML. No global config Resources.
- `ShipShields` wraps the existing `ShieldSystem` (already supports configurable `num_facings`). `EntityShield` deleted in PR 2.
- Fog-of-war (NPC AI sensor range filtering) is out of scope for this PRD.

## Cross-references

- [PRD #581 — Unified Ship Entity Model](./prd-581-unified-ship-entity-model.md) — Phase 1 (completed)
- [AI Ship Unification concept](../concepts/ai-ship-unification.md)
