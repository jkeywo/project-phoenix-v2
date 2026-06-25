---
title: Issue #553 — E5 Cutover: server.rs intent-only, NPC helm through operate_helm_ai
type: source
tags: [ai, helm, npc, operate-helm-ai, cutover, prd-520, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/553
status: shipped
updated: 2026-06-25
---

# Issue #553 — E5 Cutover

PRD #520 slice E5. Removes direct Transform/physics application from `tick_ai_controllers`; `server.rs` becomes intent-only for helm. NPC ship physics now applied by `operate_helm_ai`.

## Changes

- **`src/ai/server.rs`**: Removed the `compute_physics` block from `tick_ai_controllers`. Now only sets `ctrl.last_helm_intent = Some((thrust, steering))`. Removed the `dt` variable and changed `&mut Transform` to `&Transform` in the query.
- **`src/ship_plugin.rs`**: `operate_helm_ai` expanded into two paths:
  - *Player ship* (`Without<AiControllerComponent>`): writes `LastHelmInput { thrust: 0, steering: 0 }` when `operate_ai`.
  - *NPC ship* (`With<AiControllerComponent>`): reads `last_helm_intent` and applies `compute_physics` to the entity's `Transform`.
- **`src/console/weapons/server.rs`**: Added `Without<AiControllerComponent>` filter to `operate_tactical_ai`'s `ship_query`. Prevents the player-ship tactical path from iterating NPC ships (which handle tactical via synthetic tokens).

## Smoke tests added

- `pirate_raider_ai_helm_policy_routes_through_npc_path` — NPC all-Ai resolver yields `operate_ai = true` for helm.
- `all_backfill_player_ship_helm_policy_gates_operate_ai` — Backfill player ship helm policy satisfies the operate-AI gate.

## Files

- `src/ship_plugin.rs`, `src/ai/server.rs`, `src/console/weapons/server.rs`

## Cross-references

- [PRD #520](./prd-520-ai-ship-unification.md) — parent
- [AI Ship Unification](../concepts/ai-ship-unification.md)
