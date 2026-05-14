---
title: PRD #142 — AI and Behaviour System
type: source
tags: [prd, ai, npc, state-machine, in-flight]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/142
status: in-flight
updated: 2026-05-14
---

# PRD #142 — AI and Behaviour System

A data-driven state-machine framework for NPC ships and stations. Behaviour trees are loaded from TOML; NPCs emit the same `ClientMessage` inputs as players, so they exercise exactly the same simulation paths.

## Status

In flight. The pure AI engine and Bevy plugin are landed and wired into `SimulationPlugin`:

- `src/ai.rs` (1222 LoC) — pure (no Bevy) `AiController`, fixed five-slot `Blackboard`, `AiState` (`Idle` / `Patrolling` / `Pursuing`), `AiInput`, `AiTickOutput`, the pure `tick` function, and the `should_emit` edge-emission filter.
- `src/ai_plugin.rs` (428 LoC) — Bevy orchestrator. `AiTokenRegistry` resource; per-entity controllers ticked each frame; emits synthetic `InboundMessage`s on the same channel as remote players.
- `src/faction.rs` — faction enum + targeting filter, used by `Pursuing` state.
- `src/entity_spawner.rs` — `[behaviour]` section in TOML entity files signals `ai_plugin` to attach an `AiController`.

Console-AI work (PRD #154) shares the same action vocabulary — see issues #175, #176, #177, #179 (all landed):

- `console_ai.rs` / `console_ai_plugin.rs` — `auto_fire_torpedo`, `tick_frequency_hint`, `tick_auto_match_frequency`. These synthesise `InboundMessage`s from the appropriate console holder when a console is at Low complexity.

Still open:

- TOML schema for arbitrary state-machine definitions (current states are hard-coded enum variants).
- Squad behaviours, retreat conditions, and richer blackboard slots.
- Integration with PRD #119 scenario triggers for spawn/direct/despawn.

## Problem

The simulation today only supports a single player ship and inert asteroids. There is no way to populate scenarios with hostile, friendly, or civilian craft, no way to express "patrol this area," "flee under threat," or "escort that ship." Without NPCs, scenarios cannot have any narrative or combat depth beyond physics hazards.

## Solution

A pure `ai.rs` module owns NPC state machines defined in TOML (`assets/ai/*.toml`). Each NPC has an `EntityConfig` extension declaring its behaviour file. State transitions read the same world snapshot as the player ship; actions emit the same `ClientMessage` types (`HelmInput`, `FirePhaser`, `FireTorpedo`, etc.) which are then processed by `simulation.rs` as if they came from a remote player.

This means: any console hidden by complexity (PRD #154) and operated by `console_ai` shares its action vocabulary with NPC ships. The two systems are the same machinery.

## Key decisions (drafted)

- **Pure state-machine engine** in `src/ai.rs`. No Bevy.
- **States and transitions are TOML.** Conditions reference world facts (`distance_to(target)`, `hull_below(0.5)`, `flag(SensorBlind)`); actions emit messages or set blackboard variables.
- **Blackboard per NPC.** Carries target uuid, last-known position, retreat anchor.
- **Reuses `ClientMessage`.** No new "AI control" wire — the same input types players send.
- **Spawned by scenarios.** PRD #119 triggers create NPC entities; `ai.rs` ticks them.

## Out of scope

- GOAP / utility AI / behaviour trees beyond simple state machines.
- Squad-level coordination.
- Dialogue / comms scripting (lives in PRD #119).
- Player-vs-player.

## Cross-references

- [Draft 9 — AI and Behaviour](./design-09-ai-and-behaviour.md) (informs this PRD)
- Depends on [PRD #119 — Stations + Scenarios + Comms](./prd-119-stations-scenarios-comms.md)
- Shares action vocabulary with [PRD #154 — Console Complexity](./prd-154-console-complexity.md) (`console_ai` operates hidden controls)
- [Roadmap Overview](../roadmap/overview.md)
