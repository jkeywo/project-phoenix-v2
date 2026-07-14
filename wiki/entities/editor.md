---
title: Editor
type: entity
tags: [editor, tooling, scenario, entity, definitions, fsa, vitest]
sources: [PRD-350, editor/app-v2.js, editor/scenario-mode.js, editor/mode-shell.js, editor/project-root.js, editor/save-flow.js, editor/invalidation-bus.js, editor/entity-cache.js, editor/validation.js, editor/action-schema.js, editor/world-toml.js, editor/entity-toml.js]
updated: 2026-05-22
---

# Editor

The Scenario Editor is an in-browser TOML authoring tool for editing scenario worlds, entity templates, factions, and complexity presets directly on disk via the File System Access API. It is structured as three top-level modes — **World**, **Entity**, **Definitions** — built as part of the PRD #350 rewrite. The codebase is heavily decomposed into pure ESM modules so every non-UI behaviour is unit-testable under Vitest without a browser.

The editor is **not part of the game runtime**. It does not link into the WASM binary, does not run in `server.html` or `client.html`, and does not speak the wire codec. It is a separate static page that targets the same TOML schemas the runtime parses in `src/world/config.rs`, `src/entities/config.rs`, `src/ai/faction.rs`, and `src/console_ai/complexity.rs`.

## Single Entry Point, Three Modes

`editor/app-v2.js` is the sole boot entry. It wires up the `ModeShell`, `SaveFlow`, `InvalidationBus`, project-root picker, and mounts each of the three modes uniformly:

- `mountScenarioMode(...)` (`editor/scenario-mode.js:66`) — World Mode. Owns the Konva spawn canvas, layers panel, properties panel, world-content sidebar, triggerable-worlds panel, and the "+ New World" dialog.
- `mountEntityMode(...)` (`editor/entity-mode-view.js`) — Entity Mode.
- `mountDefinitionsMode(...)` (`editor/definitions-mode-view.js`) — Definitions Mode.

`app-v2.js` sets `stringifyDefinitionsPayload` (`editor/app-v2.js:20`) to route Definitions saves through `{ kind: 'faction'|'complexity', data }`. All three modes share the same `ModeShell`, `SaveFlow`, `InvalidationBus`, and project-root I/O.

The legacy `editor/app.js` entry point has been deleted; everything it did is now owned by `mountScenarioMode`.

## Three Modes (PRD #350)

`ModeShell` (`editor/mode-shell.js:5`) manages per-mode open files, dirty state, active file, and undo history. Default modes: `['World', 'Entity', 'Definitions']` (`editor/mode-shell.js:1`).

- **World Mode** — edit unified world TOML files (`assets/worlds/*.toml`). Canvas renders entities by their `[radar_appearance]` colour and a tag-derived shape (Ship → triangle, Station → diamond, Asteroid → dot, Planet → ring, X fallback for missing appearance). Regions render as outlined sphere/box/torus shapes with a 15% alpha fill and a centre cluster of effect icons. Anchors are draggable canvas objects with rename-safety (in-layer ref rewrite + cross-layer warning) and delete-safety (blocked by references). A World Content sidebar lists anchors, named entities, triggers, comms templates, and derived objectives. Dedicated structured editors for triggers (stacked action cards) and comms templates (indented tree). Spawn `[entity.overrides]` are edited per-field against a resolved-template view.

- **Entity Mode** — edit entity template TOML files (`assets/entities/*.toml`). Three-pane: file list, component cards (one card per present TOML section + raw-TOML toggle + "+ Add component" picker with common-combo templates and a raw-section submenu), and a static top-down preview pane (collider, radar appearance, region shape, asteroid-field donut, forward arrow, overlay text). NPC behaviour editor: structured states-and-transitions two-list. Player-ship `[stations]` editor: tab-strip per player count with next/previous dropdowns populated from adjacent counts; dangling-chain and duplicate-name validation. Coordinator: `editor/entity-mode-view.js`.

- **Definitions Mode** — edit factions (`assets/factions/*.toml`) and complexity presets (`assets/complexity/*.toml`). Faction editor: UUID, name, enemy multi-select resolving other faction names. Complexity preset editor: preset list, `hidden_elements` multi-select, delegated controls table, AI tuning blocks. Coordinator: `editor/definitions-mode-view.js`.

## File System Access + Project Root

`editor/project-root.js` wraps the File System Access API:

- `pickProjectRoot()` (`editor/project-root.js:37`) prompts `showDirectoryPicker({ mode: 'readwrite' })` and persists the granted handle in IndexedDB under DB `phoenix-editor` / store `project-root` (DB v1, see `editor/project-root.js:3`).
- `getProjectRoot()` (`editor/project-root.js:44`) restores the handle on next visit, no re-prompt.
- `readFile(path)` / `writeFile(path, content)` (`editor/project-root.js:50`, `:62`) operate against the persisted handle.
- `onRootChanged(cb)` (`editor/project-root.js:14`) is a listener bus so dependent modules (entity cache, mode panes) can rebuild when the root switches.

Browsers without FSA (Firefox, Safari at time of writing) show a "browser not supported" message; the editor refuses to start. Chromium only.

## Save Flow + Invalidation

`SaveFlow` (`editor/save-flow.js:3`) is constructed in `editor/app-v2.js:32` with:

- A per-mode stringifier map (`World`, `Entity`, `Definitions` payload → TOML text).
- An optional `writeFile` (defaults to `project-root.writeFile`).
- An optional `InvalidationBus`.
- An optional `commentConfirm` gate (the one-time-per-session "comments will be lost" warning from `editor/comment-warning.js`).
- A `setSessionOnlyChecker` hook so session-only files (slice 4b triggerable-world previews) are excluded from disk writes.

`InvalidationBus` (`editor/invalidation-bus.js:1`) emits `EntitySaved`, `WorldSaved`, and `FactionSaved` events. Subscribers (entity cache, open canvases, file lists) re-fetch and re-render. There is no `ComplexitySaved` event yet — complexity presets only re-read on explicit mode re-open.

Save model: explicit save only. Cmd/Ctrl+S saves the active file; a "Save All" button saves every dirty file. No auto-save. Each layer in the tree shows a dirty indicator.

## Entity Cache

`editor/entity-cache.js` is a `path → parsed-TOML` Map (`entity-cache.js:3`) seeded by `preloadEntityCache()` (`entity-cache.js:33`), which walks `assets/entities/` via the JS-global `window.tomlParse`. `invalidateEntity(path)` (`entity-cache.js:71`) and `invalidateAll()` (`entity-cache.js:79`) are wired to `InvalidationBus.EntitySaved` so cross-mode edits propagate immediately. `onInvalidate(callback)` (`entity-cache.js:87`) lets canvas/sidebar code subscribe to a single signal.

## Validation Surface

`validation.js` (`editor/validation.js:103`) composes the file-level validator:

- `validateWorldToml(obj)` (`editor/world-toml.js:11`) — minimal `[global]` + `[anchors]` presence checks.
- `validateEntityToml(obj)` (`editor/entity-toml.js:28`) — required `tags` non-empty + section sanity.
- `validateEntitySections(obj)` (`editor/entity-toml.js:80`) — per-section schema checks against `component-schema.js`.
- `validateStations(stationsConfig)` (`editor/stations-validate.js`) — dangling next/previous chains, duplicate names per player count, invalid console enum values.
- `validateTriggerActions(actions)` (`editor/action-schema.js:205`) — every action validated against `ACTION_SCHEMA` (`editor/action-schema.js:32`), which mirrors the Rust `TriggerAction` enum from `src/world/config.rs`. Also exports `MODIFIER_SLOTS`, `INT_MODIFIER_SLOTS`, `FLAG_KINDS`.
- `validateWorldReferences` / `validateWorldReferencesIndexed` (`editor/world-references.js`, `editor/world-references-indexed.js`) — cross-reference checks (trigger entity-names, objective ids, AI states, anchor refs).
- `validateBehaviourBlock(behaviour)` (`editor/validation.js:53`) — exactly one `initial_state`, no orphan transitions.

All validation surfaces as inline error/warning badges (`editor/validation-badge.js`).
The current editor permits saving despite errors. The accepted next design changes
this: definite errors will block save and mod-pack export, while warnings remain
non-blocking so authors can work across related files in any order. A
WASM-compiled full Rust pre-save pass remains deferred to a later phase.

## Planned Mod-Pack Export

The editor will export selected validated authored TOML files as a ZIP mod pack
with a required manifest naming its selectable root scenarios.
On the server page, before scenario selection, the host will upload one pack for
the current host session. Validated files overlay base content by exact supported
path: matching paths replace the base file and new supported paths add content.
Only root worlds named by the manifest join the selectable scenario list;
supporting world files remain private composition content. The whole pack is
rejected if its archive, manifest, paths, parse, or composed references are
invalid; it cannot modify an in-progress round.

Regular scenarios will use the same explicit contract in
`assets/scenarios.toml` at the asset root. The selection catalog will merge
that base manifest with a validated uploaded mod manifest.

## TOML Parsers (Pure)

`world-toml.js` and `entity-toml.js` wrap `smol-toml` (declared in the root `package.json` devDependencies) behind a tiny pure surface:

- `parseWorldToml(text)` / `stringifyWorldToml(obj)` (`editor/world-toml.js:3`, `:7`).
- `parseEntityToml(text)` / `stringifyEntityToml(obj)` (`editor/entity-toml.js:9`, `:18`).
- `buildFactionMap(factionFiles)` (`editor/entity-toml.js:46`) and `buildComplexityPaths(complexityFilenames)` (`editor/entity-toml.js:68`) feed the cross-resolution dropdowns (UUID → faction name, path → complexity preset).

Comments are lost on round-trip (`smol-toml` is lossy); the comment-warning gate is the chosen mitigation.

## Undo / Redo

Per-file op-log undo/redo (Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z) capped at 100 ops per file (`editor/undo-stack.js`, controller in `editor/undo-controller.js`, global keyboard wiring in `editor/app-v2.js:161` (`setupGlobalUndoShortcuts`)). Stacks are cleared on save so the undo history never drifts across persisted states.

## Schema Additions (runtime landed; editor UI in-flight)

The two PRD #350 runtime additions have **landed** in `src/world/config.rs` and `src/world/server.rs` (shipped via issue #352):

- World TOML carries a top-level `extra_worlds: Vec<String>` field (`src/world/config.rs:240`). Paths listed here are auto-loaded additively at startup by `load_extra_worlds` (`src/world/server.rs:478`), which pushes one `WorldLayerChange::Load` per path onto `PendingWorldLayerChanges` so the same code path handles startup and trigger-fired loads.
- Two new trigger actions: `TriggerAction::LoadWorld { path }` (`src/world/config.rs:278`) and `TriggerAction::UnloadWorld { path }` (`src/world/config.rs:280`). Parsed from TOML at `src/world/config.rs:422` / `:425`. Dispatched at `src/world/server.rs:1017` / `:1022`, where each fires queues a `WorldLayerChange` onto `PendingWorldLayerChanges`.
- Additive-loading runtime state: `WorldLayerMap(HashMap<String, WorldRuntime>)` and `PendingWorldLayerChanges(Vec<WorldLayerChange>)` (`src/world/server.rs:83`–`:95`). `apply_world_layer_changes` (`src/world/server.rs:1146` doc comment, fn at `:1156`) drains the queue each frame, mutating `WorldLayerMap` and `WorldContentRuntime`. Runtime test coverage lives at `src/world/server.rs:2308`–`:2710`.

The **remaining editor-side work** for this PRD: the World Mode layer tree must show `load_world`-reachable worlds in a "triggerable worlds" section with a session-only load/unload toggle for preview, and the trigger action editor needs file-picker wiring for `load_world` / `unload_world` paths. These are still in flight. The runtime additive-loading state machine that [PRD #341](../sources/refactor-2026-05-entity-schema.md) collapsed is partially reintroduced for these two actions only, scoped narrowly to path-keyed load/unload.

## Test Infrastructure

Vitest (`npm run test:editor` → `vitest run`; root `package.json`). Tests live in `editor/tests/` and run on Node, importing deep modules directly — no browser DOM, no Konva.

Coverage spans the deep modules and slice integration paths:

- **Parsers/serializers:** `world-toml.test.js`, `entity-toml.test.js`, `toml-utils-transform.test.js`.
- **Validation:** `validation.test.js`, `validation-fixtures.test.js`, `validation-badge.test.js`, `stations-validate.test.js`, `action-schema.test.js`, `world-references-indexed.test.js`, `cross-references.test.js`, `triggerable-worlds.test.js`.
- **Schemas/templates:** `tag-shape-map.test.js`, `component-templates.test.js`.
- **Editors:** `override-editor.test.js`, `behaviour-editor.test.js`, `comms-editor.test.js`, `comms-adapter.test.js`, `complexity-editor.test.js`, `complexity-form-view.test.js`, `faction-editor.test.js`, `faction-form-view.test.js`, `trigger-pickers.test.js`.
- **Mode views:** `entity-mode.test.js`, `entity-component-card-view.test.js`, `entity-add-component-menu.test.js`, `entity-preview.test.js`, `entity-preview-view.test.js`, `definitions-file-list-view.test.js`.
- **Anchors:** `anchor-rename.test.js`, `anchor-rename-integration.test.js`, `anchor-delete.test.js`, `canvas-anchor.test.js`, `canvas-region.test.js`, `canvas-world.test.js`.
- **Shell / IO / lifecycle:** `mode-shell.test.js`, `save-flow.test.js`, `save-flow-comment-gate.test.js`, `save-confirm.test.js`, `comment-warning.test.js`, `invalidation-bus.test.js`, `entity-cache.test.js`, `project-root.test.js`, `project-root-listeners.test.js`, `undo-stack.test.js`, `undo-integration.test.js`, `new-world.test.js`, `world-file-picker.test.js`, `faction-complexity-discovery.test.js`, `layer-manager.test.js`, `world-content-panel.test.js`.
- **Slice integration:** `slice-3-override-cycle`, `slice-4a-trigger-edit`, `slice-4b-comms-edit`, `slice-4b-new-world`, `slice-4b-triggerable-worlds`, `slice-5-add-component`, `slice-5-behaviour-edit`, `slice-5-entity-mode-cycle`, `slice-5-stations-tab`, `slice-6-definitions-mode`, `slice-6-faction-invalidation`, `slice-7-canvas-entity-invalidation`, `slice-7-stations-warning-badge`.
- **Bootstrapping:** `smoke.test.js`.

## What is NOT tested (manual QA only)

- Canvas visuals (Konva output).
- File System Access API integration (browser-only).
- Mode-switcher and component-card DOM rendering.
- Trigger / comms editor DOM behaviour.
- Drag interactions, keyboard shortcuts beyond Cmd/Ctrl+S/Z.
- Cross-mode live cache invalidation end-to-end.
- The full save flow end-to-end (each step is unit-tested in isolation).

The editor's Vitest suite is **separate from** the runtime's Playwright smoke tests in `tests/smoke/` and does not run in the existing smoke-test CI job. A dedicated CI job for `npm run test:editor` is implied by PRD #350 slice 1.

## Where the Editor Lives

- `editor/` — all source, tests, and the two HTML entry points.
- Run locally via any static server pointed at `editor/`. No build step (ESM + browser-native imports).
- Targets only Chromium-based browsers (FSA dependency).
- Not deployed alongside `server.html` / `client.html`.

## Cross-Links

- Source: [PRD #350 — Scenario Editor Rewrite](../sources/prd-350-scenario-editor-rewrite.md)
- Runtime parser the editor mirrors: `src/world/config.rs`
- Related runtime sources: [PRD #341 — Entity Schema Refactor](../sources/refactor-2026-05-entity-schema.md), [PRD #153 — Region Entities + Entity Pipeline](../sources/prd-153-region-entities.md), [PRD #119 — Stations, Scenarios & Comms](../sources/prd-119-stations-scenarios-comms.md)
