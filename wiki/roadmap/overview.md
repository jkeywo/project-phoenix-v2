---
title: Roadmap Overview
type: roadmap
tags: [roadmap, planning, status]
sources: [PRD-001, PRD-017, PRD-022, PRD-036, PRD-041, PRD-051, PRD-066, PRD-077, PRD-086, PRD-115, PRD-116, PRD-117, PRD-118, PRD-119, PRD-120, PRD-142, PRD-153, PRD-154, PRD-180, PRD-218, PRD-266, PRD-271, PRD-272, PRD-302, PRD-317, PRD-337, PRD-344, PRD-350, PRD-397, PRD-398, PRD-419, docs/]
updated: 2026-06-04
---

# Roadmap Overview

A synthesis of where Project Phoenix is, what's open, and what's still drafted-but-not-PRD. Sourced from the GitHub `PRD`-labelled issues and the `docs/` design drafts.

> Reconciled 2026-06-04 against the code and the live GitHub issue states. The previous version (2026-05-11) was a full wave behind: it listed #118/#119/#120 as "in flight" and the game as having "5 consoles". All of those have since shipped — the game now carries **nine** consoles, stations, a scenario/world engine, NPC AI, regions, and per-console complexity. The open set is now #116, #398, #419 (plus #115's deployment slices, on hold).

## Shipped

Closed PRDs whose features are live in `main` (number order):

- **[PRD #1](../sources/prd-001-bridge-simulator.md)** — Lobby, Captain Console, Red Alert, rotating-cube viewscreen.
- **[PRD #17](../sources/prd-017-mobile-ux-and-status.md)** — Fullscreen canvas, mobile-friendly UI, connection status bar.
- **[PRD #22](../sources/prd-022-helm-and-game-world.md)** — Helm Console, ship physics, asteroid field, collisions.
- **[PRD #36](../sources/prd-036-captain-view-selector.md)** — Captain View Modes (Fore/Aft/Port/Starboard + Radar).
- **[PRD #41](https://github.com/jkeywo/project-phoenix-v2/issues/41)** — Helm Radar & client Bevy/WASM migration (client became a full Bevy app).
- **[PRD #51](../sources/prd-051-smoke-test-harness.md)** — Playwright smoke tests with the BroadcastChannel PeerJS shim.
- **[PRD #66](../sources/prd-066-weapons-and-engineering.md)** — Tactical (phasers, target lock) and Engineering (repair loop, breakdown queue), hull integrity.
- **[PRD #77](https://github.com/jkeywo/project-phoenix-v2/issues/77)** — Entity Config Files & Game Map (`entity_config.rs`, `assets/`, data-driven entities/maps).
- **[PRD #86](https://github.com/jkeywo/project-phoenix-v2/issues/86)** — Science Console & unified radar (long-range radar, system chart, impulse).
- **[PRD #117](../sources/prd-117-modifier-system.md)** — Modifier system for cross-console multipliers (pure modifier cache; foundation for #118+).
- **[PRD #118](../sources/prd-118-repair-and-power-consoles.md)** — Split `Engineering` into **Repair** (shape-matching) + **Power** (6+2 allocation).
- **[PRD #119](../sources/prd-119-stations-scenarios-comms.md)** — Space stations, TOML scenario engine, **Comms** console.
- **[PRD #120](../sources/prd-120-station-based-lobby.md)** — Station-based lobby with cascade reassignment + spectator FIFO.
- **[PRD #142](../sources/prd-142-ai-and-behaviour.md)** — NPC AI & behaviour system (pure state machine, faction configs, patrol loop).
- **[PRD #153](../sources/prd-153-region-entities.md)** — Region entities, component-driven spawning, region modifier flags.
- **[PRD #154](../sources/prd-154-console-complexity.md)** — Per-console complexity (Low/Full) with server-side AI for Low consoles.
- **[PRD #180](../sources/prd-180-viewscreen-frame.md)** — Viewscreen frame: Bevy UI border, alert vignette shader, HUD readouts.
- **[PRD #218](https://github.com/jkeywo/project-phoenix-v2/issues/218)** — Architecture: merge Scenario into World.
- **[PRD #266](https://github.com/jkeywo/project-phoenix-v2/issues/266)** — Architecture: adopt Bevy States, Observers, and SystemSets for sim control flow.
- **[PRD #271](https://github.com/jkeywo/project-phoenix-v2/issues/271)** — Integer modifiers, scenario-applied effects & debug overlay.
- **[PRD #272](https://github.com/jkeywo/project-phoenix-v2/issues/272)** — Per-console hull points & direct repair-team dispatch.
- **[PRD #302](https://github.com/jkeywo/project-phoenix-v2/issues/302)** — Codebase reconciliation: bugs, missing features, spec alignment.
- **[PRD #317](https://github.com/jkeywo/project-phoenix-v2/issues/317)** — Generic GUI library (`src/gui/`) unifying console UI widgets.
- **[PRD #337](https://github.com/jkeywo/project-phoenix-v2/issues/337)** — Complete map/scenario merger: unify `WorldConfig`, `[[entity]]` block, spawn pipeline.
- **[PRD #344](https://github.com/jkeywo/project-phoenix-v2/issues/344)** — Helm & Captain GUI refresh: `ConsoleShell` framework, image assets, orientation-aware layout.
- **[PRD #350](../sources/prd-350-scenario-editor-rewrite.md)** — Editor v2: Scenario/Entity/Definitions modes with `RadarAppearance` rendering and full TOML authoring.
- **[PRD #397](https://github.com/jkeywo/project-phoenix-v2/issues/397)** — Engine additions for branching scenarios.

Net result today: **nine consoles** in the wire types — `CaptainChair, Helm, Tactical, Repair, Sensors, Shields, Navigation, Power, Comms` (`src/core/messages.rs`). Players join **stations** (bundles of consoles per player count). Full simulation: TOML-driven ship physics, grid-based streaming asteroid field, phaser banks + torpedoes, four-quadrant shields, impulse drive, per-console hull damage, three-team dispatch repair, 6+2 power allocation, region effects, per-console complexity presets with server-side AI, TOML world engine with objectives + NPC AI patrols, viewscreen frame/HUD, a generic GUI library, and a scenario/entity editor. Fully tested in CI (Rust unit + Playwright smoke).

## Open PRDs (planned / in flight)

| PRD | State | Theme | Notes |
|---|---|---|---|
| [#115 Native PC Server](../sources/prd-115-native-pc-server.md) | PRD closed, slices on hold | Distribution | Native binary + cloudflared tunnel + WebSocket transport. Deployment slices [#135–#141](https://github.com/jkeywo/project-phoenix-v2/issues/115) are on hold; WASM/GitHub Pages remains the only shipped target. |
| [#116 Save/Load](../sources/prd-116-save-load-sessions.md) | Open | Persistence | `localStorage` save slots; `save.rs` is the second sanctioned `serde_json` surface. |
| [#398 'Before the Fire' Scenario](https://github.com/jkeywo/project-phoenix-v2/issues/398) | Open | Content | A full authored scenario built on the branching-scenario engine (#397). |
| [#419 HTML-Based Console UI](https://github.com/jkeywo/project-phoenix-v2/issues/419) | Open | Client | Move console UI off the Bevy/WASM client toward an HTML-based rendering path. |

## Drafted (no PRD yet)

`docs/` retains design drafts. Almost all have now been promoted and shipped:

| Draft | State | Theme |
|---|---|---|
| [Draft 1 — Entity Config Files](../sources/design-01-entity-config-files.md) | shipped (#77) | `entity_config.rs` + `assets/entities/`. |
| [Draft 2 — Game Map](../sources/design-02-game-map.md) | shipped (#77, #337) | World TOML + streaming asteroid lifecycle; map/scenario merged. |
| [Draft 3 — Science Console](../sources/design-03-science-console.md) | shipped (#86), since split | Science later split into Sensors / Shields / Navigation. |
| [Draft 4 — Combat Update](../sources/design-04-combat-update.md) | shipped | Phaser banks + torpedoes + four-quadrant shields. |
| [Draft 5 — Ship's Power](../sources/design-05-ships-power.md) | shipped (#118) | Engineering split + battery / 6+2 model. |
| [Draft 6 — Space Stations](../sources/design-06-space-stations.md) | shipped (#119) | |
| [Draft 7 — Scenario File](../sources/design-07-scenario-file.md) | shipped (#119, #218, #337) | |
| [Draft 8 — Comms Console](../sources/design-08-comms-console.md) | shipped (#119) | |
| [Draft 9 — AI and Behaviour](../sources/design-09-ai-and-behaviour.md) | shipped (#142) | NPC state machine + faction configs. |
| [Draft 10 — Region Entities](../sources/design-10-region-entities.md) | shipped (#153) | Invisible volumes → region modifier flags. |
| [Draft 11 — Console Complexity](../sources/design-11-console-complexity.md) | shipped (#154) | Per-console Low/Full toggle + server-side AI. |
| [Architecture Improvement Notes](../sources/notes-architecture-improvements.md) | partly addressed | Per-console message subscriptions; some settled by #266 SystemSets work. |

## Themes

The work clusters into:

1. **[Console Expansion](./console-expansion.md)** — From 4 consoles to the full bridge crew of **nine**. Repair/Power split (#118), Comms (#119), and the Science split into Sensors/Shields/Navigation are all live.
2. **[Combat & Damage](./combat-and-damage.md)** — Hull, shields, phasers, torpedoes, repair loop, breakdown queue, per-console hull points (#272). Draft 4 shipped in full.
3. **[Data-Driven Content](./data-driven-content.md)** — Entity files (#77), world/map merger (#337), scenarios + stations (#119), branching scenarios (#397), NPC AI (#142), regions (#153), editor v2 (#350).
4. **[Open Architectural Questions](./open-architectural-questions.md)** — Largely settled: the modifier system (#117), Bevy States/Observers/SystemSets (#266), and the generic GUI library (#317) closed most of them. The remaining live question is the console-UI rendering path (#419, HTML vs Bevy/WASM).

## Tensions across the open work

- **Save format vs shipped scenario/world state.** #116's save snapshot was scoped to ship/hull/asteroids/weapons. The game has since gained station assignments, scenario/objective state, regions, and NPC AI — save fidelity now needs to cover (or explicitly version-exclude) all of it. Save/load has not yet landed, so this is design-time, not migration.
- **HTML console UI (#419) vs the Bevy/WASM client.** The client is currently a full Bevy/WASM app with a generic GUI library (#317) and `ConsoleShell` framework (#344). Moving consoles to HTML rendering is a large client-architecture shift that must keep the wire protocol and session model unchanged.
- **Native server slices (#115) on hold vs ongoing client changes.** The native bridge mirrors `bridge.rs`/`server.html`; while #419 reworks the client and #135–141 sit on hold, the two must stay protocol-compatible (`Identify`-first handshake, `Target`-based routing) when the native work resumes.

## Cross-references

- All [sources](../index.md#sources)
- [Project Overview](../concepts/project-overview.md)
- [Open Architectural Questions](./open-architectural-questions.md)
