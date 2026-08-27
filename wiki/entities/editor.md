---
title: Editor
type: entity
tags: [editor, tooling, scenario, entity, definitions, models, mod]
sources: [editor/app-v2.js, editor/scenario-mode.js, editor/mode-shell.js, editor/project-root.js, editor/save-flow.js, editor/invalidation-bus.js, editor/entity-cache.js, editor/validation.js, editor/world-toml.js, editor/entity-toml.js, editor/models-mode-view.js, editor/mod-mode-view.js]
updated: 2026-08-27
---

# Editor

The browser editor is a project-root-aware authoring shell for World, Entity, Definitions, Models, and MOD modes. It edits repository-native TOML/assets through the File System Access API; it is not part of the simulation runtime.

## Shell and persistence

`ModeShell` owns the active mode, per-mode open files, dirty state, and undo history. `project-root.js` persists a granted directory handle in IndexedDB and exposes path-based reads/writes. `SaveFlow` centralises validation, serialisation, writes, dirty-state clearing, and invalidation events.

`InvalidationBus` tells dependent modes when an entity, world, faction, or other shared definition changes. `entity-cache.js` keeps parsed entity TOML keyed by repository path and invalidates affected entries after saves.

## Modes

- World mode (`scenario-mode.js`) edits scenario/world TOML, entity placement, layers, content, and Rhai script.
- Entity mode edits template sections against the shared component schema.
- Definitions mode edits faction and complexity data through typed save payloads.
- Models mode edits model-rig/LOD authoring surfaces.
- MOD mode packages and inspects mod content against the project layout.

## Validation boundary

The editor performs fast client-side structural checks in `validation.js`, `world-toml.js`, and `entity-toml.js`. Runtime/PASM remain authoritative: saved scenario changes still need `uv run pasm validate`, `scan`, and `traceability`, while entity/config changes still pass Rust parsing and the repository's normal gates.

World composition uses `extra_worlds` and Rhai `load_world`/`unload_world` actions. Both flow into the same runtime `WorldLayerChange` path documented in [World Data](./world-data.md) and [WorldPlugin](../concepts/world-plugin.md).

## Related

- [World Data](./world-data.md)
- [Model Viewer](../concepts/model-viewer.md)
- [LOD Generation](../concepts/lod-generation.md)
