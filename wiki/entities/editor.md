---
title: Editor
type: entity
tags: [editor, tooling, scenario, entity, definitions, models, mod]
sources: [editor/app-v2.js, editor/scenario-mode.js, editor/mode-shell.js, editor/project-root.js, editor/save-flow.js, editor/invalidation-bus.js, editor/entity-cache.js, editor/validation.js, editor/world-toml.js, editor/entity-toml.js, editor/models-mode-view.js, editor/mod-mode-view.js, editor/mod-actions.js, editor/mod-pack-workspace.js, editor/mod-pack-export.js, gui/editor-mod-actions.js, gui/client-semantic-actions.js, gui/operator-profile.js, gui/semantic-controls-remapper.js, workshop.html, editor/workshop-document.js, editor/workshop-runtime.js, editor/workshop-recovery.js, src/workshop/mod.rs, src/workshop/document.rs, src/workshop/provider.rs, src/workshop/archive.rs, src/workshop/provider/assets.rs, src/workshop/provider/test_snapshot.rs, src/workshop/test_protocol.rs, src/native_host/workshop/test_clock.rs, src/native_host/workshop/test_process.rs, editor/workshop-test.js, gui/workshop-test-panel.js, src/native_host/workshop/mod.rs, src/native_host/workshop/bridge.rs, src/native_host/workshop/document.rs, src/native_host/workshop/keyboard.rs, src/boot/mod.rs, src/delivery/args.rs, editor/workshop-provider.js, gui/native-workshop.js, gui/workshop-authoring.js, gui/workshop-layout-model.js, gui/workshop-layout-renderer.js, scripts/build-workshop.mjs, run-workshop.bat, scripts/serve-workshop.mjs, assets/audio/sound-cues.toml, src/sound_cues.rs, gui/sound-audition-panel.js, editor/workshop-sound-cues.js, src/world/pack_asset_validation.rs, src/audio_decode.rs, src/entities/pack_assets.rs, src/entities/pack_assets/versioned.rs, editor/asset-dependencies.js, editor/workshop-assets.js, pasm/spec/architecture/workshop-runtime-assets.yaml, editor/workshop-handoff.js, editor/workshop-source-provider.js, gui/workshop-source-link.js, pasm/spec/architecture/workshop-source-handoff.yaml, src/workshop/test_clock.rs, src/workshop/test_source.rs, src/workshop/test_browser.rs, workshop-test.html, editor/workshop-test-frame.js, editor/workshop-test-child.js, editor/workshop-test-runtime.js, editor/workshop-test-snapshot.js, gui/workshop-test-boot.js, tests/smoke/workshop-test-runtime.render.spec.js, src/entities/pack_assets/snapshot.rs, editor/workshop-models.js, gui/workshop-models-panel.js, gui/workshop-model-preview-panel.js, pasm/spec/architecture/workshop-model-authoring.yaml, src/workshop/model_fields.rs, src/inspector.rs, gui/inspector-field.js, pasm/spec/architecture/workshop-live-inspector.yaml]
updated: 2026-09-15
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

## Runtime assets

Workshop retains supported model, texture and sound members as exact bytes.
The ordinary host upload and offline Workshop validator both use
`src/world/pack_asset_validation.rs`: actual glTF buffer/accessor/primitive
checks, image decoding and the shared MP3/OGG/WAV decoder. Browser dependency
capture uses the build's length/CRC manifest and dependency names; native
validation uses an explicit byte snapshot. Replacing an external buffer also
validates its existing model consumers. A missing or changed immutable
dependency refuses the operation. These checks do not install the candidate.

Accepted packs retain their source archive alongside source and asset members.
`src/entities/pack_assets.rs` registers the common Bevy reader, giving every
accepted stack revision new asset identities. Model, planet, LOD, viewer and
dust consumers retire their previous visual state before render extraction;
late completion of an old load cannot overwrite a current handle. Canonical
simulation entities remain in place.

## Standalone Workshop Authoring

The browser GM's source control uses `gui/workshop-source-link.js` to retain an
exact loaded source pack, close the GM connection and navigate to Workshop.
`editor/workshop-handoff.js` carries only authored archives and read-only base
declarations through a bounded, expiring, single-use IndexedDB record. The URL
fragment is consumed on arrival. `editor/workshop-source-provider.js` creates
a fresh document/history for the selected archive and immutable dependencies
from the other packs. Storage refusal preserves Live. It does not export live
simulation state or grant project filesystem access; native GM surfaces do not
advertise this browser capability.

`workshop.html` is M6's offline browser Authoring surface. `trunk build` ships
the page and its runtime WASM artifact; `npm run build:workshop` updates the
HTML/JS and dependency snapshot alongside an existing runtime. On Windows,
`run-workshop.bat` runs Trunk and uses
`scripts/serve-workshop.mjs` to serve it on loopback port 8083, opening the browser
only after the server is listening. Its JavaScript and runtime are local build artifacts. It opens
one new or user-selected mod ZIP without a project-directory grant or GM session.
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
the prior draft and history. Export validates the exact candidate archive through
`src/workshop`: the ordinary mod-pack gate, runtime source parsers, include and
composition checks, and budgeted Rhai compiler. Static child worlds and scripts
resolve against an explicit immutable base/other-pack source snapshot. Validation
does not install content, start an App, or apply a content ledger. Missing runtime
artifacts or dependencies refuse export. Findings link back to authored files and
reported lines. `WorkshopDocument.check()` remains a structural helper for older
consumers; the Workshop UI never treats it as runtime acceptance.

`workshop-layout-model.js` and `workshop-layout-renderer.js` own the registered
Authoring panel arrangement independently of `WorkshopDocument`: file browser,
source document, generic inspector, file addition, recovery, validation findings,
correlated action feedback, immutable dependencies, private operator settings, the
model/rig field form, the captured model preview and the sound audition can split, tab,
float in-surface, close and reset while lifecycle commands remain in the fixed
menu/toolbar. Each registered panel carries a class: `source` and the model preview
are *documents* — the surfaces the context is arranged around, stamped
`data-panel-kind="document"` — and every other panel is a *tool*. The class is
presentation only and confers no authority. The renderer moves each registered panel's original node rather
than cloning controls. The versioned layout is a
presentation-only field in the private operator profile, repaired to defaults on
obsolete or invalid data. Narrow surfaces project one selected panel through a
switcher without replacing the retained desktop tree. Browser and native Workshop
mount this same renderer through `mountWorkshopAuthoring`. Presenter-driven reveal
reopens and activates findings or source in desktop and narrow projection without
changing validation authority or entering dependency reads into document history.
Settings replace only the relevant Accessibility or binding field in the existing
private operator profile and apply Accessibility effects immediately. The native
profile sanitizer mirrors the browser's version-4 panel registry and migration.
Each stored version is sanitized against the vocabulary that version had, so a panel
registered later can only enter through migration, never out of an older tree;
its private Workshop bridge publishes only the already-captured textual dependency
snapshot, never binary dependency assets or filesystem authority.

The field inspector uses `toml_edit` source spans in the same Rust module. It
patches exactly one scalar and refuses stale documents or incompatible types;
comments, unknown fields, ordering and unchanged line endings remain byte-for-byte.
World `[global]` descriptors derive expected types and defaults from the actual
`GlobalConfig` fields. Model sidecars use `src/workshop/model_fields.rs` to derive
scalar types from the runtime rig and LOD fields, including array elements and
generation/capture metadata. Integer-spelled float values remain editable as
floats. Base transforms expose their runtime defaults; required geometry and
optional entity-dependent LOD values do not invent defaults. Enum spellings and
unsigned integer limits use the field's runtime deserializer. Other scalar paths,
including unknown fields, remain source fallbacks and pass the same final runtime gate.
These source fields are Authoring-only; none provides an arbitrary live write.
`src/inspector.rs` and `gui/inspector-field.js` share their type, default, source,
validation and Live mutability metadata with the connected GM's constrained
[NPC doctrine inspector](../concepts/npc-doctrine-controls.md). Exact source
locations come from the document spans; a Live reading without such a span
reports it unavailable. Named Live actions keep their own canonical owner.

`workshop-recovery.js` stores one versioned data-only draft in its own IndexedDB
database. The record carries immutable imported bytes, current documents, last
exported documents, selected path and chronological history. Reload requires an
explicit Restore or Discard before another import. Corrupt records and storage
failure never silently replace the open source. This storage grants no filesystem
authority and carries no private operator profile or live runtime state.

Source and binary file additions/replacements use the same chronological history.
Binary members, including GLB and MP3, never pass through a text decoder. Undo of
an addition removes that member; undo back to the original source set restores
the complete original ZIP container. Pack validation still refuses binary members
until the runtime asset overlay is implemented. A loaded pack can enter through
`createBrowserWorkshopProvider` as an immutable source archive with separately
snapshotted dependencies; the selected pack is removed from the other-pack list.
The dependency viewer is read-only and never adds those files to the editable pack.

`NativeWorkshopProvider` is an offline selected-project or selected-mod-directory
capability constructed by native code. Its JSON request vocabulary cannot select
another root. It reads authored paths under `assets/` (plus a mod's `scenarios.toml`),
refuses traversal, path aliases and symlinks, and compares the complete loaded disk
source plus its revision before saving. Project saves use the real project manifest,
runtime source parsers, include/composition checks and the ordinary budgeted script
compiler. Mod saves use the same pack validator as browser export.

Native writes journal exact before/after bytes before replacing or deleting any
member. Reopening finishes an interrupted accepted save, but refuses an external
edit instead of overwriting it. A separate private per-root directory holds the
OS workspace claim, save journal and crash-draft record. Recovery retains the old
revision, so restoring a draft cannot silently overwrite newer files on disk.
`mountNativeWorkshop` mounts the same shared Authoring UI over a private request
callback and reply receiver; no browser HTTP/file provider is introduced. The
native shell opens through `phoenix-host --workshop-project DIR` or
`--workshop-mod DIR`, with `--client-dir` and an Ultralight build.

Native `load-sources` returns source text bytes and immutable asset-version
references from `src/workshop/provider/assets.rs`. The selected root owns the
private blob store; references cannot name authored or arbitrary filesystem paths.
Asset imports and preview reads use 64 KiB chunks. Save materializes and verifies
each referenced version before the existing runtime and external-edit gates.
Native recovery uses document version 3 to retain those references in the same
chronological history, including a saved replacement later undone after reopening.
Ordinary browser documents refuse reference-bearing recovery. The store has a
bounded 2 GiB capacity and refuses new versions when full; retained versions are
not automatically discarded while a recovery record or undo history may need them.

`src/native_host/workshop` composes the offline `BootProfile::NativeWorkshop`.
It retains the shared native render core and the existing pane input, resize and
texture-upload pipeline, while refusing world ingestion and live/crew launch
flags. The in-memory native document replaces only the shared browser boot;
its loopback delivery server receives no filesystem request endpoint or selected
source path. An ordered worker owns source IO and durable private operator
preferences, so validation and asset reads do not block the pane renderer.
Replies belong to one view epoch. Recreating a failed pane preserves the provider
and recovery draft, and retires an unfinished import after older accepted work
finishes. A pane becomes live only after its modules and source startup settle;
failed initialisation follows the bounded pane rebuild path.

Native Authoring forwards physical key identity, modifiers, repeat and release
through the shared semantic dispatcher. Unclaimed input reaches the embedded
engine for ordinary text and caret editing; Ctrl+Tab remains host-owned.

Native Test uses `provider/test_snapshot.rs` to materialize and validate one
immutable unsaved document plus its read-only dependencies. Its source-based
catalogue resolves complete composed hulls, including base hulls outside the
editable mod. `native_host/workshop/test_process.rs` stages exact bytes in a
private directory and launches the same host executable with explicit world,
hull and seed selection. The child has no delivery listener or crew transport.
The child pins both filesystem and Bevy asset roots to that stage. Runtime
shader support is captured read-only alongside the authored snapshot; there is
no fallback to current project files. The native Live layout store is disabled.
Inherited pipes carry only typed clock/visibility controls and current status.
The native host's ordinary solo launch assigns every Station Backfill.

`workshop/test_clock.rs` feeds the ordinary fixed schedules: Pause holds the virtual clock,
Step supplies exactly one fixed timestep after discarding a partial frame, and
1×/2×/4×/8× acceleration changes the virtual rate. The shared
`editor/workshop-test.js` controller and `gui/workshop-test-panel.js` keep Test
and source controls exclusive. Returning to Authoring pauses and hides the
retained run. Editing marks it stale; Restart captures new source and starts a
fresh process rather than patching the old simulation. Validation failures keep
the draft and previous run. Stop, child/output failure and view-epoch retirement
kill/wait the child and release staged files. Worker startup reclaims abandoned
UUID staging directories with valid typed markers under its existing root claim.

Browser Test uses the same source catalogue, selection gate and fixed clock.
`editor/workshop-test-snapshot.js` validates the exact unsaved pack and merges
captured base, ordered dependencies and candidate bytes. Text remains text;
binary members cross a private MessagePort as transferred defensive Uint8Array
copies, without detaching the Authoring document or expanding assets into JSON
number arrays. Each accepted replacement creates a fresh `workshop-test.html`
iframe and WASM App. A refused replacement retains the previous run. Stop and
disposal immediately retire even a pending frame; a late completion cannot
adopt a cancelled run. A pending frame stays in the viewport but transparent,
inert and excluded from accessibility until adoption: Winit's canvas
intersection tracking must not stall startup below the Authoring form.

`editor/workshop-test-runtime.js` supplies captured TOML, Rhai and model sidecars
through the ordinary preload callbacks before the shared browser boot. Its
asset sources are confined to the per-App snapshot. The Test edge accepts only
bounded clock controls and status requests. It installs no Live transport,
profile or save adapters; the disposable marker disables lifecycle capture and
the browser save APIs refuse access. Returning to a hidden held iframe shows
it before awaiting the next clock acknowledgement, because browsers may
suspend animation frames while it is hidden.

GM/other-player-ship views, traces, breakpoints, role
preview and specialised entity/definition/model panels remain M6 continuation work. The existing editor/viewer remain until
the parity workflow is delivered.

## Validation boundary

The Authoring sound panel reads `assets/audio/sound-cues.toml` from the current
draft. A catalog declares packaged MP3/OGG/WAV assets, categories, audiences and
informative equivalents; shared JavaScript checks and `src/sound_cues.rs` reject
invalid definitions. Known shipped sounds retain their category and minimum
information requirements. New nested packaged sounds use ordinary local asset
resolution and decoding. Source comments and byte history remain the document
owner's responsibility. Generated JSON is delivery data, not editable pack source.

Audition invokes the existing private audio provider locally for at most two
seconds and displays only authored equivalent text. Music, Ambience and Effects
join the same private profile mixer where audition is available. Draft replacement,
revision, a held Authoring surface, Stop and page disposal retire the preview.
Candidate bytes take precedence over the runtime's immutable dependency asset
resolver; unavailable bytes do not fall through to uncaptured delivery. Native
Workshop has no assigned audio endpoint and reports that limitation visibly;
the native GM surface uses its explicitly assigned private endpoint.

The editor performs fast client-side structural checks in `validation.js`, `world-toml.js`, and `entity-toml.js`. Runtime/PASM remain authoritative: saved scenario changes still need `uv run pasm validate`, `scan`, and `traceability`, while entity/config changes still pass Rust parsing and the repository's normal gates.

World composition uses `extra_worlds` and Rhai `load_world`/`unload_world` actions. Both flow into the same runtime `WorldLayerChange` path documented in [World Data](./world-data.md) and [WorldPlugin](../concepts/world-plugin.md).

## Related

- [World Data](./world-data.md)
- [Model Viewer](../concepts/model-viewer.md)
- [LOD Generation](../concepts/lod-generation.md)

## Workshop model source forms

The Workshop Models and rigs panel groups existing base-transform, marker,
target-point, LOD, bounds and build fields from the shared source inspector.
It reads exact TOML values and prepares changed scalar spans through the runtime
patch provider before committing one chronological draft edit. A stale source,
changed draft or failed field leaves the draft untouched. Variant creation copies
the selected rig source exactly and refuses to overwrite another variant.

These controls share Authoring's Test hold, validation, save/export and recovery
owners. The model source panel has no filesystem capability or Live action route.
The model preview panel consumes the explicit `modelPreview` provider. Refresh
captures immutable source; subsequent edits or selection changes label the
retained picture stale. Its camera, distance, lighting, marker and LOD controls
operate only on the disposable renderer and show its measured mesh/texture
statistics. Test, draft replacement and exit stop the preview. External
generation remains in the existing tooling pending its Workshop integration.
