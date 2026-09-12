---
title: Editor
type: entity
tags: [editor, tooling, scenario, entity, definitions, models, mod]
sources: [editor/app-v2.js, editor/scenario-mode.js, editor/mode-shell.js, editor/project-root.js, editor/save-flow.js, editor/invalidation-bus.js, editor/entity-cache.js, editor/validation.js, editor/world-toml.js, editor/entity-toml.js, editor/models-mode-view.js, editor/mod-mode-view.js, editor/mod-actions.js, editor/mod-pack-workspace.js, editor/mod-pack-export.js, gui/editor-mod-actions.js, gui/client-semantic-actions.js, gui/operator-profile.js, gui/semantic-controls-remapper.js, workshop.html, editor/workshop-document.js, gui/workshop-authoring.js, scripts/build-workshop.mjs]
updated: 2026-09-12
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
- MOD mode packages mod content against the existing validation boundary.

MOD ZIP import is the `editor.mod.import` semantic action in the `editor.mod`
context. It uses the shared local feedback lifecycle. A readable pack is loaded
into the existing workspace before validation, so definite findings are focused
and announced without discarding its source members. The workspace retains an
immutable copy of the imported archive and each ordered entry's exact bytes;
when the manifest fields and a member's text remain untouched, repair/export
reuses those bytes so comments, ordering, extension keys, and line endings do
not vanish. ZIP paths and contents use fatal UTF-8 decoding, matching the Rust
upload reader. An unreadable ZIP or manifest leaves the previous workspace
intact. T2 does not add the M6 live inspectors or project tooling to this editor.

The same bounded workspace now exposes a source textarea for one selected
member, preserving its imported base classification/digest and immutable source
entry while the edited text changes. `editor.mod.validate` runs the existing
export gate without downloading or clearing MOD dirty state;
`editor.mod.export` runs that gate and downloads only an accepted ZIP. Both use
Pressed/Pending/Applied-or-Refused feedback, focus an accessible refusal, and
return successful focus to Export. Import, Validate, and Export each have two
slots in the shared private operator profile and can be remapped through the
common controls component. The parent client catalogue retains those bindings
across surfaces, while ordinary play Settings filters out editor-only actions.
Reset All is profile-wide: Captain, Helm, and editor remaps return to their
authored defaults together. Feedback presentation is keyed per semantic action,
so a busy Export or Import refusal remains alongside an outstanding Validate
Pending state instead of erasing it. This tracer adds no unified inspector,
project-root editing, model tooling, or Workshop redesign.

## Standalone Workshop Authoring

`workshop.html` is M6's first offline browser Authoring slice. Build it with
`npm run build:workshop` and serve `dist/workshop.html`; ordinary Trunk builds
also ship it. Its JavaScript and TOML parser are local build artifacts. It opens
one user-selected text mod ZIP without a project-directory grant or GM session.
`WorkshopDocument` owns immutable imported bytes, the editable source documents,
and one chronological `UndoStack` across the whole pack. The source editor
preserves unknown keys, comments, source BOMs and unchanged mixed line endings;
it does not run the old editor's normalizing serializers. Undo selects the
document it changed. Dirty state compares against the imported or last exported
source, independently of history and validation.

The thin `workshop-authoring.js` adapter uses shared private Accessibility,
semantic Import/Validate/Export bindings, remapping and attributed local action
feedback. Accepted action bindings take precedence over conventional history
shortcuts outside editable fields. A failed replacement import or export keeps
the prior draft and history. Export derives a temporary `ModPackWorkspace` and
calls the existing `exportModPack` gate, preserving the exact edited manifest
instead of regenerating it. The UI explicitly calls these **structural** checks:
Rhai compilation and complete runtime admission remain the host's responsibility.
The existing gate still refuses BOM-prefixed TOML, while the source remains
available for repair. This slice has no Test simulation, project provider,
structured inspector or model-asset editing; the existing editor/viewer remain
until the M6 parity workflow is delivered.

## Validation boundary

The editor performs fast client-side structural checks in `validation.js`, `world-toml.js`, and `entity-toml.js`. Runtime/PASM remain authoritative: saved scenario changes still need `uv run pasm validate`, `scan`, and `traceability`, while entity/config changes still pass Rust parsing and the repository's normal gates.

World composition uses `extra_worlds` and Rhai `load_world`/`unload_world` actions. Both flow into the same runtime `WorldLayerChange` path documented in [World Data](./world-data.md) and [WorldPlugin](../concepts/world-plugin.md).

## Related

- [World Data](./world-data.md)
- [Model Viewer](../concepts/model-viewer.md)
- [LOD Generation](../concepts/lod-generation.md)
