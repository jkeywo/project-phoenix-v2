---
title: PRD #350 — Scenario Editor Rewrite (Three-Mode Authoring)
type: source
tags: [prd, editor, tooling, scenario, entity, definitions, fsa, vitest, in-progress]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/350
status: in-progress
updated: 2026-05-22
---

# PRD #350 — Scenario Editor Rewrite (Three-Mode Authoring)

A rewrite of the in-browser `editor/` tool into a three-mode authoring environment (Scenario / Entity / Definitions) covering every TOML schema in `assets/`. Adds File System Access API project-root persistence, structured editors for triggers and comms templates, a per-field override editor, an entity-mode component-card UI with a static top-down preview, faction and complexity-preset editors, lightweight JS validation surfaced as inline badges, per-file undo/redo (100 ops, cleared on save), and a Vitest test surface for the deep modules.

## Status

In progress. Slices 1–6 form the v1 release; slice 7 is post-launch polish. The migration is complete on the entry-point side: `editor/app-v2.js` is the sole boot entry, mounting Scenario / Entity / Definitions uniformly via `mountScenarioMode` / `mountEntityMode` / `mountDefinitionsMode`. The legacy `editor/app.js` has been deleted; its World-mode responsibilities now live in `editor/scenario-mode.js`. Vitest suite under `editor/tests/` is large and covers the deep modules (parsers, validators, schemas, undo, save-flow, mode-shell, cross-references, behaviour, stations, comms, faction, complexity, project-root) — see `package.json` `test:editor` script.

## Problem

Designers and developers authoring scenarios, entity templates, factions, and complexity presets had to hand-edit TOML in a plain text editor across four `assets/` subtrees. The pre-rewrite `editor/` only covered 2D spawn placement and a "new entity" modal, still spoke the pre-PRD-#341 schema (separate maps + scenarios with `[[spawn]]`), and could not author triggers, comms templates, scenario-level entity overrides, factions, complexity presets, or any structured view of entity components. This forced a constant translation tax between author intent and what the runtime parsers accept, and produced silent breakage from mistyped entity-name references, dangling station chains, deep-merged overrides, and malformed comms response trees.

## Solution

Three top-level modes chosen via a mode switcher:

- **Scenario Mode** — edit unified world TOML files. Canvas draws entities using their `[radar_appearance]` colour and a shape derived from `tags` (Ship → triangle, Station → diamond, Asteroid → dot, Planet → ring, etc.), with an X fallback for entities lacking `[radar_appearance]`. Regions render as outlined shapes with a 15% alpha fill and a centre cluster of effect icons. Anchors are first-class draggable canvas objects with rename-safety (in-layer ref rewrite + cross-layer warning) and delete-safety (blocked by in-use references). A "World Content" sidebar lists anchors, named entities, triggers, comms templates, and derived objectives. Triggers and comms templates have dedicated structured editors. Spawn `[entity.overrides]` are edited per-field against a resolved-template view with an overrides-summary card.

- **Entity Mode** — edit entity template TOML files. Three-pane layout: file list, component cards (one card per present TOML section with structured fields + per-card raw-TOML toggle, plus an "+ Add component" picker offering common-combo templates and a raw-section submenu), and a static top-down preview pane showing collider, radar appearance, region shape, asteroid-field donut, forward arrow, and overlay text (tags, faction name, consoles, hull total). Behaviour editor (v1) is a structured states-and-transitions two-list editor. `[stations]` editor uses a tab-strip per player count with next/previous dropdowns populated from adjacent counts.

- **Definitions Mode** — edit factions and complexity presets. Faction editor: UUID, name, enemy multi-select resolving other faction names. Complexity preset editor: preset list, `hidden_elements` multi-select, delegated controls table, AI tuning blocks.

The editor uses the File System Access API to pick a project root once per session, after which it reads and writes any TOML file under `assets/`. The entity cache is rewritten to read on-demand from the root handle, with invalidation hooks fired on save so cross-mode live updates work (editing an entity in Entity Mode invalidates the cache so an open Scenario Mode canvas re-renders).

## Runtime Status (extra_worlds + LoadWorld / UnloadWorld)

The runtime half of the schema additions **shipped via issue #352**. Concrete landing points:

- `extra_worlds: Vec<String>` on `WorldConfig` (`src/world/config.rs:240`); auto-loaded at startup by `load_extra_worlds` (`src/world/server.rs:478`).
- `TriggerAction::LoadWorld { path }` (`src/world/config.rs:278`) and `TriggerAction::UnloadWorld { path }` (`src/world/config.rs:280`); parsed at `src/world/config.rs:422` / `:425`; dispatched at `src/world/server.rs:1017` / `:1022`.
- Additive-loading state: `WorldLayerMap` + `PendingWorldLayerChanges` (`src/world/server.rs:83`–`:95`); drained each frame by `apply_world_layer_changes` (doc comment `src/world/server.rs:1146`, fn at `:1156`). Test coverage at `src/world/server.rs:2308`–`:2710`.

The runtime additive-loading state machine that PRD #341 collapsed (multi-world layering) is partially reintroduced for these two actions only, scoped narrowly to path-keyed load/unload.

**Remaining editor-side scope (still in flight):** the World Mode layer tree must surface `extra_worlds`-loaded sub-worlds as auto-attached child layers and `load_world`-reachable worlds in a "triggerable worlds" section with a session-only load/unload toggle for preview; the trigger action editor needs file-picker wiring for `load_world` / `unload_world` paths.

**Naming drift:** this PRD refers to the world-authoring mode as "Scenario Mode", but the shipped mode label is `"World"` — see `DEFAULT_MODES = ['World', 'Entity', 'Definitions']` in `editor/mode-shell.js:1`. The entity page (`wiki/entities/editor.md`) uses the shipped name.

## Save / Validation / Undo Model

- Explicit save (active layer or all-dirty). Cmd/Ctrl+S = save active layer. No auto-save.
- One-time-per-session "comments will be lost" warning on first save of a comment-bearing file (`smol-toml` is lossy).
- Dirty indicator on every layer in the layer tree.
- Phase-1 validation: lightweight in-JS structural validation (required fields, type checks, enum values for modifier slots, flag kinds, condition names, console names, cross-reference checks). Surfaced as inline error/warning badges. Never blocks save.
- Phase-2 (separate PRD, post-launch): WASM-compiled actual Rust parsers for full pre-save validation.
- Per-file op-log undo/redo (Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z), cap 100 ops per file, cleared on save.

## Shipping Slices

1. **Foundation** — FSA project-root picker, entity cache rewrite, unified world TOML parser/writer in JS, entity TOML parser/writer in JS, mode-switcher shell, undo stack.
2. **Scenario Mode canvas** — Tag→shape mapping, radar_appearance rendering, X fallback, region rendering (sphere/box/torus + effect icons), anchor canvas objects.
3. **World Content panel** — Lists, cross-reference index, click-to-highlight, per-field override editor.
4. **Triggers + Comms editors** — Action schema, stacked action cards, comms tree editor, sub-world toggle in layer tree, `extra_worlds` + `load_world` / `unload_world` runtime + editor wiring.
5. **Entity Mode** — Three-pane shell, every component card, templates + raw-section picker, static preview, behaviour lists, stations tabs-per-count.
6. **Definitions Mode** — Faction editor, complexity preset editor.
7. **Polish** — Comment warning, full validation badges, behaviour state-machine diagram (v2), stations grid-with-arrows (v2).

v1 = slices 1–6. Slice 7 follows post-launch.

## Module Breakdown (Deep Modules)

Deep (pure, unit-testable): `project-root.js`, `world-toml.js`, `entity-toml.js`, `tag-shape-map.js`, `anchor-rename.js`, `cross-references.js`, `override-editor.js`, `action-schema.js`, `stations-validate.js`, `undo-stack.js`, `validation.js`, `component-schema.js`, `component-templates.js`.

Shallow UI shells: `mode-shell`, `canvas` (modify), `anchor-canvas`, `world-content-panel`, `trigger-editor`, `comms-tree-editor`, `subworld-loader`, `entity-mode`, `entity-preview`, `behaviour-editor`, `stations-editor`, `definitions-mode`, `faction-editor`, `complexity-editor`, `comment-warning`, `component-cards/`.

## Testing

Runner: Vitest (`npm run test:editor` → `vitest run`). Tests run on Node, import deep modules directly, no browser DOM.

Test philosophy mirrors the Rust side: take fixtures (shipped TOML), feed through pure functions, assert on returned values. Never assert on DOM, Konva layer counts, or event-handler registration.

**NOT tested (manual QA only):** canvas visuals (Konva), File System Access API integration, mode-switching UI, component-card rendering, trigger/comms editor DOM, drag/keyboard, cross-mode live cache invalidation, full save flow end-to-end.

## Out of Scope

WASM-compiled Rust pre-save validation (phase-2 PRD). Behaviour state-machine diagram and stations grid-with-arrows (slice 7 polish). Multi-select on canvas. Project-wide search. Live push to a running game session. Auto-save. Drag-reorder of comms responses. Comment preservation through round-trips. Editing fields the runtime parsers do not consume. Authoring new file types beyond the four documented in `docs/toml-authoring-guide.md`. Native binary or VS Code extension packaging. Browsers without FSA support (Firefox, Safari): editor shows "browser not supported" and refuses to start; users directed to Chromium.

## Cross-Links

- Issue: https://github.com/jkeywo/project-phoenix-v2/issues/350
- Entity page: [editor.md](../entities/editor.md)
- Runtime world parser this editor mirrors: `src/world/config.rs`
- Wiki sources for prior runtime work: [PRD #341 entity-schema refactor](./refactor-2026-05-entity-schema.md), [PRD #153 region entities + entity pipeline](./prd-153-region-entities.md), [PRD #119 stations, scenarios & comms](./prd-119-stations-scenarios-comms.md)
