---
title: PRD #119 — Space Stations, Scenario Engine & Comms Console
type: source
tags: [prd, scenario, station, comms, console, planned]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/119
status: open
updated: 2026-05-11
---

# PRD #119 — Space Stations, Scenario Engine & Comms Console

Three interlocking systems that give the crew a mission to play through. Depends on the Science Console and Ship Power System being shipped first.

## Status

Open. Depends on Science + Power.

## Problem

The world is static — only asteroids to shoot. No mission, no objectives, no narrative thread. Each console operates in isolation.

## Solution

- **Space Stations** — persistent world entities with hull integrity, collision, and tags. Visible on viewscreen + radar. Can be hailed, attacked, destroyed.
- **Scenario Engine** — a TOML scripting layer loaded on top of the map at runtime. Spawns entities at named map anchors, registers triggers (`on_attacked`, `on_destroyed`, `on_hailed`, `on_timer`), fires actions (`load_scenario`, `add/complete/fail_objective`, push comms message), scripts comms exchanges. Scenarios fetched over HTTP by JS and passed to WASM via a new `wasm_load_scenario` call (mirroring entity configs).
- **Comms Console** — fifth crew station (`Console::Comms`). Two-panel UI: message list (sender + subject) + expanded chat with predefined responses. Manages mission objectives. Can push an active message to the viewscreen via `ViewMode::Comms`. Captain sees a read-only `ObjectiveSummary`.

A minimal default scenario ships with the PRD: **Starbase Alpha** — hailable, sends a distress call when attacked, generates a follow-on scenario with an objective.

## Key decisions

- **`scenario.rs`** — pure module owning loaded scenario state (entities + stable name + UUID, triggers, objectives, comms queue, contacts). Bevy-free.
- **`client_comms.rs`** — pure client state: `ClientCommsState` with `apply()` + outbound builders.
- **`Console::Comms`** new variant. `ViewMode::Comms` new variant.
- **New messages:** `Hail`, `SelectCommsMessage`, `RespondToMessage`, `ClearComms`, `CommsState`, `StationSpawned`, `StationDestroyed`, `ObjectiveSummary`.
- **Identity:** runtime UUID + optional stable `name` resolved by scenario engine. UUIDs never appear in TOML; scripts use `$param_name` substitution.
- **Scenario scoping:** owned entities despawn on unload; objectives removed; comms messages orphaned (marked "transmission ended", responses disabled) until cleared.
- **Default scenario always loaded** at startup, never unloaded. Holds fixed world furniture.
- **`CommsState` is event-driven,** not a 10 Hz poll. Only sent on change. `ObjectiveSummary` similarly to Captain only.
- **Entity config additions:** `tags = ["station", "comms_contact"]`, `shape = "sphere|cylinder|torus"` (renderer), `hull_integrity`, plus `on_attacked`/`on_destroyed` trigger blocks (overridable per spawn).
- **Scenario TOML schema:** `[[spawn]]` (entity path + position + name + overrides), `[[trigger]]` (condition + actions), `[[comms]]` (from / message / trigger / `[[comms.responses]]` with branching `follow_up`), `[[objective]]` (id + text + flag), `preload = [...]`.
- **`serde_json` constraint preserved.** TOML parser is separate; scenarios use `toml`.

## Implementation order

1. Scenario engine foundation + `wasm_load_scenario` + map `default_scenario` field
2. Station entities (TOML schema, shape rendering, identity, spawn/destroy messages)
3. Trigger system (all conditions + actions + parameter substitution)
4. Comms server-side (state, objectives, message queue, contact list, branching, scoping)
5. Comms client-side (two-panel UI, response buttons, `ViewMode::Comms`, Captain summary)
6. Default scenario content (Starbase Alpha + dialogue + distress chain)
7. Tests (unit, codec round-trips, `comms.spec.ts` smoke)

## Out of scope

- Docking / resupply / repair at stations.
- Free-text comms.
- AI-driven NPC ships (patrolling, counter-fire).
- Region entities and `on_entered_region` (see Draft 10).
- Ship destruction / game-over.
- Multi-scenario stress testing.

## Cross-references

- [Draft 6 — Space Stations](./design-06-space-stations.md) · [Draft 7 — Scenario File](./design-07-scenario-file.md) · [Draft 8 — Comms Console](./design-08-comms-console.md) — the three drafts this PRD consolidates.
- [Draft 9 — AI and Behaviour](./design-09-ai-and-behaviour.md) — adjacent; some triggers shared.
- [Draft 10 — Region Entities](./design-10-region-entities.md) — out of scope here, complementary.
- Depends conceptually on Science + Power consoles.
- [Roadmap: Data-Driven Content](../roadmap/data-driven-content.md)
