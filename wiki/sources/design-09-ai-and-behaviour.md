---
title: Draft 9 — AI and Behaviour
type: source
tags: [draft, ai, behaviour, state-machine, npc]
source_path: docs/9. Draft Design - AI and Behaviour.md
status: draft
updated: 2026-05-11
---

# Draft 9 — AI and Behaviour

A predefined state-machine model for enemy ships and other mobile entities. State machine is defined in the entity config TOML and overridable per-spawn in scenario files.

## Predefined states

| State | Behaviour |
|---|---|
| `idle` | Stationary. |
| `patrolling` | Loops between waypoints (anchor names or absolute positions). |
| `pursuing` | Moves toward a target at max speed. |
| `attacking` | In weapons range — fires phasers and/or torpedoes. |
| `fleeing` | Moves away from a threat at max speed. |
| `warping_out` | Graceful exit: accelerates and despawns. Used for scenario-unload graceful exits. |

## Transition conditions

Reuses scenario trigger vocabulary plus in-simulation conditions:

`on_attacked`, `on_destroyed`, `on_scenario_unloaded`, `on_timer`, `in_weapons_range`, `hull_below { threshold }`, `target_destroyed`.

## Graceful exit

Entities in `warping_out` are orphaned when their owning scenario unloads — they self-manage their despawn. The scenario unloads immediately and does not wait.

## Overrides

A scenario can override any behaviour parameter per-spawn via a flat override block (`behaviour.initial_state`, `on_attacked.scenario`, etc.).

## Cross-references

- [Draft 7 — Scenario File](./design-07-scenario-file.md) — shares the trigger vocabulary.
- [PRD #119 — Stations, Scenarios, Comms](./prd-119-stations-scenarios-comms.md) — the trigger system that this draft would plug into; AI-driven NPCs are explicitly out of scope for #119.
- [Roadmap Overview](../roadmap/overview.md)
