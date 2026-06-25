---
title: PRD #520 — AI Ship Unification (minus damage)
type: source
tags: [prd, ai, npc, ship, ecs, components, per-kind-plugin, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/520
status: shipped
updated: 2026-06-25
---

# PRD #520 — AI Ship Unification (minus damage)

Unified the player ship and NPC ships under a single ECS Ship entity model. NPC ships now carry the same per-entity Components as the player ship and drive their helms through the same per-kind plugin path.

## Status

Shipped (issues #547–#554, landed 2026-06-25).

## Problem

Before this PRD, the player ship and NPC ships were architecturally separate:

- **Player ship**: four singleton `Resource`s (`ShipConfigResource`, `ShipSystemControlSources`, `ActiveStationRatings`, `CoordinationQueue`) and a `Ship` ECS marker.
- **NPC ships**: `AiControllerComponent` on each entity; helm physics applied directly in `tick_ai_controllers`; no `Ship` marker; no `ControlSourceResolver`.

This made it impossible to:
1. Support multiple player ships without a rewrite.
2. Apply the same `ControlSourceResolver` gating to NPC systems.
3. Route NPC helm through the same `operate_helm_ai` per-kind plugin as the player ship on Backfill.

## Solution

Eight issues landed in order:

| Slice | Issue | Description |
|-------|-------|-------------|
| A1 | #547 | `ControlSourceResolver` and `ControlTickPolicy` utilities |
| A2 | #548 | Promote 4 singleton Resources → per-entity Ship Components |
| A3 | #549 | Per-kind helm AI plugin (`operate_helm_ai`) |
| A4 | #550 | Rename AI tick functions to `operate_*` naming convention |
| A5 | #551 | Add 8 AI controller stubs/schedules (`operate_shields_ai`, `operate_comms_ai`, etc.) |
| B  | #552 | NPC ships receive `Ship` marker + Components on spawn |
| E5 | #553 | Cutover: `server.rs` writes intent only; `operate_helm_ai` applies NPC physics |
| D  | #554 | Docs (this page + concept pages + per-slice source pages) |

## Key decisions

- **No `PlayerShip` marker.** Apart from the viewscreen, every system is treated identically whether it is human- or AI-controlled. Queries use `With<Ship>` + `.iter()` throughout — the player ship is not special.
- **All NPC systems default to `ControlSource::Ai`.** The `ControlSourceResolver` for NPC ships is seeded from the entity's `ShipConfig` with every system set to `Ai`.
- **`operate_helm_ai` handles both paths.** Player-ship Backfill (no `AiControllerComponent`): writes zero `LastHelmInput`. NPC ships (have `AiControllerComponent`): applies physics directly to `Transform`.
- **`operate_tactical_ai` guards on `Without<AiControllerComponent>`.** NPC tactical still fires through synthetic `InboundMessage` tokens via `server.rs`; `operate_tactical_ai` is restricted to the player ship.
- **`server.rs` is intent-only for helm.** After E5, `tick_ai_controllers` only sets `last_helm_intent`; the per-kind plugin (`operate_helm_ai`) applies the Transform physics.

## Key files changed

- `src/ship/control_source.rs` — `ControlSourceResolver`, `ControlTickPolicy`, `ControlSource`
- `src/ship_plugin.rs` — `operate_helm_ai`, all four Components, `ShipPlugin::build`
- `src/server_app.rs` — player ship spawn attaches all four Components
- `src/lobby/server.rs` — `iter_mut().next()` replaces `single_mut()`
- `src/entities/spawner.rs` — NPC spawn inserts Ship + Components
- `src/ai/server.rs` — `AiControllerComponent.last_helm_intent`; physics removed
- `src/console/weapons/server.rs` — `operate_tactical_ai` guarded with `Without<AiControllerComponent>`

## Out of scope

- Per-system damage integration (tracked separately).
- Multiple simultaneous player ships (the architecture is ready; the lobby is not).
- `operate_tactical_ai` NPC path through the per-kind plugin (NPC tactical still uses synthetic tokens).

## Cross-references

- [AI Ship Unification](../concepts/ai-ship-unification.md) — concept page
- [PRD #142 — AI and Behaviour System](./prd-142-ai-and-behaviour.md) — parent AI system
- [Coarse-system migration](../concepts/coarse-system-migration.md) — `ControlSourceResolver` context
- [Issue #547 — ControlSourceResolver utilities](./issue-547-ai-e1-control-source-resolver.md)
- [Issue #548 — Ship Components](./issue-548-ai-e2-ship-components.md)
- [Issue #549 — Helm AI plugin](./issue-549-ai-e3-helm-ai-plugin.md)
- [Issue #552 — NPC ships receive Ship + Components](./issue-552-ai-b-npc-ship-components.md)
- [Issue #553 — E5 cutover](./issue-553-ai-e5-cutover.md)
