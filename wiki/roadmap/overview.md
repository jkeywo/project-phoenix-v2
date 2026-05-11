---
title: Roadmap Overview
type: roadmap
tags: [roadmap, planning, status]
sources: [PRD-001, PRD-017, PRD-022, PRD-036, PRD-051, PRD-066, PRD-115, PRD-116, PRD-117, PRD-118, PRD-119, PRD-120, docs/]
updated: 2026-05-11
---

# Roadmap Overview

A synthesis of where Project Phoenix is, what's open, and what's still drafted-but-not-PRD. Sourced from the GitHub `PRD`-labeled issues and the `docs/` design drafts.

## Shipped

Closed PRDs whose features are live in `main`:

- **[PRD #1](../sources/prd-001-bridge-simulator.md)** — Lobby, Captain Console, Red Alert, rotating cube viewscreen.
- **[PRD #17](../sources/prd-017-mobile-ux-and-status.md)** — Fullscreen canvas, mobile-friendly UI, connection status bar.
- **[PRD #22](../sources/prd-022-helm-and-game-world.md)** — Helm Console, ship physics, asteroid field, collisions.
- **[PRD #36](../sources/prd-036-captain-view-selector.md)** — Captain View Modes (Fore/Aft/Port/Starboard + Radar).
- **[PRD #51](../sources/prd-051-smoke-test-harness.md)** — Playwright smoke tests with the BroadcastChannel PeerJS shim.
- **[PRD #66 — Weapons & Engineering](../sources/prd-066-weapons-and-engineering.md)** — Tactical (phasers, target lock) and Engineering (repair loop, breakdown queue). Hull integrity, phaser beam events, `AsteroidDestroyed`. Client now a full Bevy/WASM app.

Plus a chunk of work landed without a labelled PRD (visible from git history): **Phaser banks** (port/starboard, `phaser.rs`), **Torpedoes** (`torpedo.rs`, `WeaponsUpdate.torpedo_*` fields, `TorpedoLaunched`/`TorpedoDestroyed`), **Four-quadrant shields** (`shield.rs`, `ShieldStatus`), **Impulse drive** (`impulse.rs`, `StartImpulseCharge`/`CancelImpulse`), **Science console** (`Console::Science`, `SetScienceTarget`, `ScienceTargetSuggestion`, `ViewMode::ScienceRadar`/`SystemChart`), **Data-driven entities** (`entity_config.rs`, `map_config.rs`, `config_cache.rs`, `assets/`), **Range-gated asteroid lifecycle** (`asteroid_lifecycle.rs`).

Net result today: **5 consoles in the wire types** (Captain, Helm, Tactical, Engineering, Science), one ship, destroyable asteroid field, phaser banks, torpedoes, four-quadrant shields, impulse drive, hull damage, breakdown/repair, data-driven entities/maps, client WASM Bevy UI, fully tested in CI.

## Open PRDs (in flight)

Six open `PRD`-labelled issues form the next wave of planned work:

| PRD | Theme | Notes |
|---|---|---|
| [#115 Native PC Server](../sources/prd-115-native-pc-server.md) | Distribution | Native binary + cloudflared tunnel + WebSocket transport. New `native` Cargo feature alongside `server`/`client`. |
| [#116 Save/Load](../sources/prd-116-save-load-sessions.md) | Persistence | `localStorage` slots; `save.rs` is the second sanctioned `serde_json` surface. |
| [#117 Modifier System](../sources/prd-117-modifier-system.md) | Infrastructure | Pure `modifiers.rs`. Lands first; consumed by #118+. |
| [#118 Repair + Power](../sources/prd-118-repair-and-power-consoles.md) | Console | Splits `Engineering` into `Repair` (shape-matching) + `Power` (6+2 allocation). Depends on #117. |
| [#119 Stations / Scenarios / Comms](../sources/prd-119-stations-scenarios-comms.md) | Content | TOML scenario engine, station entities, `Console::Comms`. Depends on Science + Power. |
| [#120 Station-Based Lobby](../sources/prd-120-station-based-lobby.md) | Lobby | Per-station picking with cascade reassignment + spectator FIFO. |

Suggested ordering (from PRD dependencies):

1. #117 (modifier infrastructure)
2. #118 (consumes #117)
3. #119 (assumes Science + Power)
4. #120 (lobby model)
5. #115 + #116 are independent — schedule alongside.

## Drafted (no PRD yet)

`docs/` retains design drafts not yet promoted:

| Draft | State | Theme |
|---|---|---|
| [Draft 1 — Entity Config Files](../sources/design-01-entity-config-files.md) | shipped (in main) | Now realised as `entity_config.rs` + `assets/entities/`. |
| [Draft 2 — Game Map](../sources/design-02-game-map.md) | partly shipped | `map_config.rs` + `assets/maps/`; streaming partly via `asteroid_lifecycle.rs`. |
| [Draft 3 — Science Console](../sources/design-03-science-console.md) | shipped | `Console::Science` live; long-range radar, impulse, system chart in wire types. |
| [Draft 4 — Combat Update](../sources/design-04-combat-update.md) | shipped | Phaser banks + torpedoes + four-quadrant shields all in main. |
| [Draft 5 — Ship's Power](../sources/design-05-ships-power.md) | superseded by #118 | Engineering split + battery model formalised. |
| [Draft 6 — Space Stations](../sources/design-06-space-stations.md) | absorbed by #119 | |
| [Draft 7 — Scenario File](../sources/design-07-scenario-file.md) | absorbed by #119 | |
| [Draft 8 — Comms Console](../sources/design-08-comms-console.md) | absorbed by #119 | |
| [Draft 9 — AI and Behaviour](../sources/design-09-ai-and-behaviour.md) | candidate | NPC state machine; not in any PRD yet. |
| [Draft 10 — Region Entities](../sources/design-10-region-entities.md) | candidate | Invisible volumes; modifier slot already exists for `RegionEffect`. |
| [Draft 11 — Console Complexity](../sources/design-11-console-complexity.md) | candidate | Per-console Low / Full toggle on Tactical / Science / Engineering. Builds on shield-frequency mechanic and the Power model. |
| [Architecture Improvement Notes](../sources/notes-architecture-improvements.md) | candidate | Per-console message subscriptions. |

## Themes

The work clusters into:

1. **[Console Expansion](./console-expansion.md)** — From 4 consoles to the full bridge crew. Science is in. Repair/Power split (#118) and Comms (#119) extend further.
2. **[Combat & Damage](./combat-and-damage.md)** — Hull, shields, phasers, torpedoes, repair loop, breakdown queue. Most of Draft 4 has shipped.
3. **[Data-Driven Content](./data-driven-content.md)** — Entity files (shipped), map files (shipped), scenarios + stations (#119), AI (Draft 9), regions (Draft 10).
4. **[Open Architectural Questions](./open-architectural-questions.md)** — Per-console messaging, scenario lifecycle, viewscreen authority. The modifier system (#117) and station lobby (#120) settle several of them.

## Tensions across the open work

- **Wire breakage cadence.** #118 renames `Engineering` → `Repair`; #120 removes `Console`-based lobby messages. #116 (save/load) versions the save format from 1; an `Engineering` save made today would be invalid the moment #118 lands. Save/load probably lands *after* the rename or includes an explicit migration note.
- **Captain authority vs Comms / Science viewscreen pushes.** Today the captain alone calls `SetView`. Science already pushes `ScienceRadar`/`SystemChart`; #119 adds `Comms` push. The captain-as-final-authority model needs an explicit override path (PRD #119 user story 13 calls this out).
- **Default-scenario-always-loaded vs save format.** #116's save snapshot covers ship/hull/asteroids/weapons; it does not yet cover scenario state. Once #119 lands, save fidelity needs a follow-up.
- **Spectator persistence vs save.** #120 keeps the spectator FIFO across Lobby → InProgress; #116 doesn't currently mention restoring it.

## Cross-references

- All [sources](../index.md#sources)
- [Project Overview](../concepts/project-overview.md)
- [Open Architectural Questions](./open-architectural-questions.md)
