---
title: Editor
type: entity
tags: [editor, tooling, scenario, entity, definitions, models, mod]
sources: [editor/app-v2.js, editor/scenario-mode.js, editor/mode-shell.js, editor/project-root.js, editor/save-flow.js, editor/invalidation-bus.js, editor/entity-cache.js, editor/validation.js, editor/world-toml.js, editor/entity-toml.js, editor/models-mode-view.js, editor/mod-mode-view.js, editor/mod-actions.js, editor/mod-pack-workspace.js, editor/mod-pack-export.js, gui/editor-mod-actions.js, gui/client-semantic-actions.js, gui/operator-profile.js, gui/semantic-controls-remapper.js, workshop.html, editor/workshop-document.js, editor/workshop-runtime.js, editor/workshop-recovery.js, src/workshop/mod.rs, src/workshop/document.rs, src/workshop/provider.rs, src/workshop/archive.rs, src/workshop/provider/assets.rs, src/workshop/provider/test_snapshot.rs, src/workshop/provider/preview_snapshot.rs, src/workshop/test_protocol.rs, src/native_host/workshop/test_clock.rs, src/native_host/workshop/test_process.rs, src/native_host/workshop/preview.rs, editor/workshop-test.js, gui/workshop-test-panel.js, src/native_host/workshop/mod.rs, src/native_host/workshop/bridge.rs, src/native_host/workshop/document.rs, src/native_host/workshop/keyboard.rs, src/boot/mod.rs, src/delivery/args.rs, src/delivery/serve.rs, editor/workshop-provider.js, editor/workshop-preview.js, editor/workshop-preview-runtime.js, editor/workshop-scripts.js, editor/script-editor.js, editor/script-editor-view.js, src/world/script/authoring.rs, gui/native-workshop.js, gui/workshop-authoring.js, gui/workshop-scripts-panel.js, gui/workshop-layout-model.js, gui/workshop-test-layout-model.js, gui/workshop-layout-renderer.js, scripts/build-workshop.mjs, run-workshop.bat, scripts/serve-workshop.mjs, assets/audio/sound-cues.toml, src/sound_cues.rs, gui/sound-audition-panel.js, editor/workshop-sound-cues.js, src/world/pack_asset_validation.rs, src/audio_decode.rs, src/entities/pack_assets.rs, src/entities/pack_assets/versioned.rs, editor/asset-dependencies.js, editor/workshop-assets.js, pasm/spec/architecture/workshop-runtime-assets.yaml, editor/workshop-handoff.js, editor/workshop-source-provider.js, gui/workshop-source-link.js, pasm/spec/architecture/workshop-source-handoff.yaml, src/workshop/test_clock.rs, src/workshop/test_source.rs, src/workshop/test_browser.rs, workshop-test.html, editor/workshop-test-frame.js, editor/workshop-test-child.js, editor/workshop-test-runtime.js, editor/workshop-test-snapshot.js, gui/workshop-test-boot.js, tests/smoke/workshop-test-runtime.render.spec.js, tests/smoke/workshop-preview.render.spec.js, tests/native_workshop_ultralight.rs, src/entities/pack_assets/snapshot.rs, editor/workshop-models.js, gui/workshop-models-panel.js, gui/workshop-model-preview-panel.js, pasm/spec/architecture/workshop-model-authoring.yaml, src/workshop/model_fields.rs, src/inspector.rs, gui/inspector-field.js, pasm/spec/architecture/workshop-live-inspector.yaml, src/workshop/definitions.rs, editor/workshop-definitions.js, gui/workshop-definitions-panel.js, src/entities/config_cache.rs, pasm/spec/architecture/workshop-definition-authoring.yaml, src/headless/app.rs, src/workshop/composition.rs, src/workshop/source_spans.rs, editor/workshop-composition.js, gui/workshop-composition-panel.js, src/workshop/entity.rs, editor/workshop-entity.js, gui/workshop-entity-panel.js, src/entities/include_resolve.rs, src/entities/entity_override.rs, src/entities/config.rs, tests/smoke/workshop-entity.render.spec.js, src/workshop/presets.rs, editor/workshop-presets.js, gui/workshop-presets-panel.js, gui/gm-role-presets.js, tests/gm_role_preset_digest_neutrality.rs]
updated: 2026-09-20
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
profile sanitizer mirrors the browser's version-10 panel registry and migration.
Each stored version is sanitized against the vocabulary that version had, so a panel
registered later can only enter through migration, never out of an older tree;
its private Workshop bridge publishes only the already-captured textual dependency
snapshot, never binary dependency assets or filesystem authority.

Test is a second dock context with its own versioned `testLayout` in that same
private profile. Its launch and clock controls are a tool panel and its disposable
Viewscreen is a document panel. Switching contexts hides one retained layout and
restores the other; only panel placement persists. World, hull, seed, current run,
tick and stale state remain owned by the disposable Test controller and never enter
the profile. Browser and native profile sanitizers share a case fixture, while the
common dock renderer supplies narrow projection, keyboard movement and reset.

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

Traces, breakpoints, role preview and specialised entity/definition/model panels
remain M6 continuation work. The existing editor/viewer remain until
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

## Workshop exact-source member lifecycle (issue #1471)

A draft can create, rename and delete text and binary members. Renaming carries
the member's VALUE across rather than re-encoding it, so a move cannot normalise
line endings, a BOM or a native asset reference — renaming a file is not an edit
of it. Deleting is confirmed, and undone by the ordinary history if it was not
what the operator meant.

History entries are now GROUPS of changes rather than one path each. A rename is
two changes (the old path goes, the new arrives) and an accepted migration
rewrites a member wholesale; both must undo as ONE press, because half a rename
is a member with no name and half an accepted migration is source nobody
reviewed. Recovery snapshots are version 4 (browser) and 5 (native); versions 2
and 3 are still read, as one-change groups, since a draft recovered from an older
crash is still a draft.

The Changes panel reports what the draft has done to the source it was imported
as: added, removed, renamed and modified members. It compares BYTES and paths and
never meaning — nothing is normalised before comparing, which is the same promise
exact-source authoring makes everywhere else. A rename is recognised from
identical content rather than guessed from similarity, and a member that moved
AND changed is reported honestly as a removal plus an addition, because the bytes
that arrived are not the bytes that left.

Older supported content is offered a migration rather than a refusal. The only
one that exists is the content pin: the runtime refuses a pack pinned to a
superseded `content_epoch` with no remediation, so an author's only recourse was
a blind hand-edit. The proposal rewrites ONE scalar and leaves every comment, key
order and line ending around it untouched; it never moves a pin backwards, and
never re-pins a pack built for different content, because that pack is not out of
date — it is for something else. Proposing is not applying: acceptance
re-proposes first and refuses if the draft moved, since the bytes reviewed would
otherwise not be the bytes written.

## Workshop faction and complexity definitions (issue #1474)

The `definitions` dock panel (Workshop layout v6, beside the inspector) edits
faction relationships and the console-complexity ladder through forms whose
fields, choices and defaults come from the runtime rather than a JS schema.
"Complexity definitions" are the `[[station.rating]]` rungs (`name`,
`automated_systems`, optional `ai_tuning`) and each station's `visiting_rating`
on hull templates under `assets/entities/`; `assets/complexity/*.toml` no longer
exists. Faction definitions are `assets/factions/*.toml` (`FactionConfig`: uuid,
name, display_name, enemies, compliance), whose unknown keys are legal and are
preserved byte-for-byte and listed read-only.

`src/workshop/definitions.rs:470` `catalog` reads the draft's text members plus
the immutable dependency bundle into a `DefinitionCatalog`: every faction and
hull with 1-based lines from the `toml_edit` spans, enemies resolved to names
across the effective set, the stations' owned systems (the only legal
`automated_systems` choices), order responses spelled by `OrderResponse`'s serde
contract, AI rules from `console_ai::server`'s constants and compliance defaults
from `ComplianceDisposition::default()`. Base and pack members are listed with
their origin and carry no controls; only draft members are editable. Browser:
`wasm_workshop_definitions`; native: `Operation::Definitions` → `Response::Definitions`.

Structural edits are a Rust `toml_edit` transaction — `src/workshop/document.rs:316`
`edit` applies `set`/`put`/`insert`/`remove`/`append_table` in order, all-or-nothing,
behind an exact `expected_source` guard, each value parsed as exactly one TOML
value. The emitter drops carriage returns, so surviving lines are given their
original endings back and new lines follow the document's convention. The pure
`editor/workshop-definitions.js` plans the edit list from a form
(`planFactionEdits`, `planRatingEdits`); `gui/workshop-definitions-panel.js`
sends ONE `runtime.edit` per member per Apply and lands the answer as one
`draft.edit`, never touching the draft before the runtime answers and refusing
when the draft moved. A new faction is `toml::to_string(FactionConfig)` through
`wasm_workshop_new_faction` / `Operation::NewFaction`, put at
`assets/factions/<slug>.toml`; deletion is confirmed and undone by the ordinary
history, with the dangling references becoming findings.

Cross-file validity is a finding, not an edit-time refusal, so an author can add
the enemy first and the faction second. `definitions.rs:735` `findings` runs from
both `validate_pack` (candidate = the pack, beneath = base + other packs) and
`validate_project`, so an error refuses save and export: duplicate faction
uuid/name, self-enemy, unknown or invalid enemy, an entity's unknown `faction`,
a world trigger or scripted `add_faction_enemy`/`remove_faction_enemy` naming no
faction, and the rating rules mirrored from `src/ship/config.rs` (duplicate rung
name, unknown or unowned automated system, unknown or missing visiting rating, a
visiting rating or host order on a station that seats no human) with the
runtime's own message text plus the line the runtime lacks. Two READ-ONLY files
beneath the draft sharing a uuid are a `dependency-duplicate-uuid` warning on
the last path, never a refusal. The one edit-time refusal beyond type checks is
a rung whose name a station already has. A rung's `ai_tuning` rules are put and
removed one key at a time: a rung whose last rule was removed keeps an empty
`[station.rating.ai_tuning]` standard table, which a table-valued put would be
refused over, while a per-key put enters it and materialises a missing table
alike. The catalog resolves a uuid declared twice the way the registry does —
base, packs in stack order, then the draft, latest winning — so a choice label
and a resolved enemy name never disagree; a Project workspace's catalog resolves
against nothing beneath, as its Check does.

The faction registry is built from EFFECTIVE content on both targets
(`src/entities/config_cache.rs:1742` native, `:1281` wasm): native reads
`assets/factions/*.toml` under the cwd — the pinned content root, or a disposable
Test child's stage — then overlays every active pack's faction files, newest
winning a shared uuid; wasm takes the set a browser Test captured through
`replace_faction_registry` (`src/workshop/test_browser.rs`), so a faction the draft
deleted is gone in the Test. The compiled-in four are only the fallback for an
absent directory or an empty thread-local set. Packs' factions therefore reach the
registry when the App builds; live pack UPLOAD still does not validate faction
references (follow-up), and a pack uploaded after boot does not refresh the
resource until the next App. `phoenix-headless` pins no content root, so
`src/headless/app.rs` replaces the registry with the factions BESIDE its
templates (`faction_directory_beside`, the sibling of `--ship`'s directory):
a run launched from another directory flies the hulls and the factions of one
tree. Known residue: two DEPENDENCY packs sharing a uuid under different file
names resolve by stack order on the Live host but by file name in a Test's
staged directory (the warning above says so); a Test whose draft deletes a
faction has no end-to-end browser run yet, only the traced capture path.

## Workshop world composition and scenario entry points (issue #1475)

The `composition` dock panel (Workshop layout v7, in the `files` column beside
`changes` on both the browser and the native operator profile) authors what a
mod or project composes and which roots it offers. Composition is authored in
three places; the panel edits two — the manifest's `[[scenario]]` roots (`id`,
`world`, `label`, curated `ships`) in `scenarios.toml` (pack, `[pack]` header)
or `assets/scenarios.toml` (project, `[content]` header), and each world's
`extra_worlds` — and LISTS the third read-only: script-driven `load_world` /
`unload_world` references, whether TOML trigger actions, calls in an inline
`[script]` body or in the sibling `.rhai` a world declares, each with its line,
its source and the origin of the path it names. Editing Rhai is #1478's.

`src/workshop/composition.rs:850` `catalog` reads the draft's text members plus
the immutable dependency bundle into a `CompositionCatalog`: the manifest view
(kind, header, every root with `id_line`/`world_line`, the world's origin, the
ships it curates and whether the world offers each, the unknown keys preserved
read-only), every world of the effective set (draft first, then base, then
packs) with its `extra_worlds`, script references and `[[available_ships]]`,
the members with origin, whether a mod pack may carry them
(`is_allowed_content_path`) and which manifest or world references them, the
runtime's choices (worlds; hull templates that compose to a class with a ship
config, the Test catalog's rule), the catalogue and the findings. Lines are
1-based `toml_edit` spans through the `src/workshop/source_spans.rs` helpers
lifted out of `definitions.rs`. Browser: `wasm_workshop_composition(files,
textDependencies)`; native: `Operation::Composition` → `Response::Composition`,
a Project workspace resolving against nothing beneath through the provider's
shared `reference_dependencies` (Definitions uses it too).

Unlike the definition forms, a composition edit is REFUSED at edit time with the
source untouched: `composition.rs:1272` `compose` applies `document::edit` to a
copy and re-checks the edited member over candidate ∪ dependencies (the
candidate winning a path) — a duplicate or empty scenario id, an empty, non-
`assets/worlds/*.toml` or missing world, a ship the world does not offer, a
dropped `[pack]`/`[content]` header; an `extra_worlds` entry that is missing,
duplicate, the world itself, outside `assets/worlds/`, or one that closes a
CYCLE over the `extra_worlds` graph (DFS, the message lists `a -> b -> a`). The
refusal is `<rule>: <detail>`, the rule being the finding category the same
violation reports as, and only rules the edit INTRODUCES are refused, so a
hand-broken draft is repaired one edit at a time. The pure
`editor/workshop-composition.js` plans the edits (`planScenarioEdits`,
`planExtraWorldEdits`; a reorder is `set`s on the swapped slots so comments stay
with their positions) and maps a refusal's rule prefix to a
`workshop.composition.refused.*` string with the runtime's sentence as the
detail; `gui/workshop-composition-panel.js` sends ONE `runtime.compose` per
member per Apply and lands the answer as one `draft.edit`. A new world is
`composition.rs:994` `new_world_source` (a `[global]` with the title, built
through `toml_edit` and asserted through `parse_world`) put at
`assets/worlds/<slug>.toml`. Browser: `wasm_workshop_compose`,
`wasm_workshop_new_world`; native: `Operation::Compose`, `Operation::NewWorld`
→ `Response::Patched`.

The same rules are findings with lines: `composition.rs:1349` `findings` runs
beside `definitions::findings` in both `validate_pack` and `validate_project`
and reports `extra-worlds-missing` (before this a load error with no line),
`extra-worlds-duplicate`, `extra-worlds-self`, `extra-worlds-disallowed`,
`extra-worlds-cycle` (at the entry that closes it), `world-missing-load-reference`
(a trigger or scripted load/unload path in the candidate's worlds and
`assets/worlds/*.rhai` that is in neither the draft nor its dependencies),
`scenario-world-disallowed`, and `member-disallowed` as a WARNING for a project
workspace only (a pack already gets `disallowed-path` from the archive gate) and
only for a member the manifest or a world NAMES, so the audio catalogues, string
tables and join codes a project legitimately carries draw no warning. `catalog`
additionally reports `runtime-source-invalid` for a draft manifest or world the
parser refuses, so the panel is not silently missing a member it cannot read.
"The same validated catalogue" is `composition.rs:956` `scenario_catalogue`:
`world::manifest::build_catalog` over the candidate manifest resolving worlds
through candidate ∪ beneath, which `src/workshop/tests.rs` proves equal entry
for entry between the pack path (store zip plus dependency bundle) and the
project path (the same members as files, nothing beneath); the panel's
Catalogue section is that list. `src/workshop/test_source.rs:38`
`validate_selection` accepts a root whose draft-declared child exists only in
the candidate and refuses when the child is missing, so the exact unsaved
composition is what a Test runs. It also applies
`composition.rs:1439` `selection_findings` — the composition rules in the
selected root's own scope — so a Test no longer starts on a cyclic or
dangling-reference composition that save and export would refuse. A rule another
world breaks still only reaches Check, because a Test runs one root.

Known residue: the world loader reads `extra_worlds` ONE level deep, so a
child's own `extra_worlds` are silently ignored at runtime (a cycle is refused
here because the authored graph and the runtime's flat list have parted, not
because it would loop); script references are listed, never edited; a dangling
`load_world` in a read-only base or pack script is listed with a null origin but
is not a finding, since nothing the author can edit carries it; the literal scan
cannot judge a path built from a variable, and it reads a `load_world("...")`
spelled INSIDE another string literal as a reference. A refusal names the slot of
the violation that has no counterpart in what the member already carried, which
for a repeated value is the LATER slot — so adding a second copy of an entry is
refused at the copy that was already there, not at the one just written. A reorder of roots moves
the scalars between two slots rather than the entries themselves, because the
edit vocabulary cannot insert mid-array: a comment authored beside one root
therefore stays at its slot and ends up beside the root that moved into it. The
alternative, remove-and-append, would drop the moved root to the end of a list
whose order IS the lobby's.

## Workshop entity template and fragment composition (issue #1476)

The `entity` dock panel (Workshop layout v8, in the `inspector` column after
`definitions` on both the browser and the native operator profile —
`gui/workshop-layout-model.js:44` `ADDED_IN_V8` and
`src/native_host/panes/operator.rs:343` `WORKSHOP_ADDED_IN_V8`, both
`[['entity','inspector']]`) composes ONE entity template from its ordered
`includes`. It adds, removes and reorders included fragments, adds and removes
supported components, shows every effective field with the member that authored
it and the merge chain it came through, and writes an inherited value into the
local document as an exact-source override on request. #1481 (ship stations,
systems and AI) will extend this panel rather than add another, because it
authors the same document.

Nothing here re-implements the merge. `src/workshop/entity.rs:736` `catalog`
READS `src/entities/include_resolve.rs:629` `resolve_template`'s `Provenance`
— the composed value plus a `BTreeMap` from field address
(`hull.hull_integrity`, `system[id=helm-thrust].ai_only`,
`station[id=bridge].rating[name=Std].automated_systems`) to the member that
authored it and the chain it came through — and turns it into an
`EntityComposition` (`entity.rs:76`): the template's origin (`draft`, `base`,
`pack:<id>`), whether the closure resolves and the resolver's own sentence when
it does not, the `includes` with each entry's authored text, the member it
canonicalises to, its 1-based line and that member's origin, the merge order
(`provenance.sources()`), one `ComponentView` per supported key (local with its
line, inherited from a named member, or absent — and `local` and `inherited_from`
are INDEPENDENT, see below), one `FieldView` per effective field (the exact span
text and line when local, the resolved value serialised to TOML when inherited,
the value's TOML type, and whether materialising it could write anything at all),
the fragments it could still include, and the findings. `entity.rs:357`
`parse_address` reads a keyed address
back into steps and takes each identity key from the merge's own
`MergePolicy::array_rule`, never a copy of `entity_override::COMPOSE_KEYED_ARRAYS`.
Browser: `src/workshop/wasm.rs:143` `wasm_workshop_entity(files,
textDependencies, path)`; native: `Operation::Entity` → `Response::Entity`, a
Project workspace resolving against nothing beneath through the provider's
shared `reference_dependencies`.

The component vocabulary is the runtime's. `EntityConfig` is
`deny_unknown_fields`, so `entity.rs:668` `supported_components` parses a probe
document carrying one impossible key and reads serde's own "unknown field `x`,
expected one of `a`, `b`, …" list out of the error (53 keys, `OnceLock`-cached,
ratchet-tested). `entity.rs:709` `component_skeleton` asks the runtime what a
component with nothing authored IS — deserialise `{}` into it through
`EntityConfig::from_toml`, serialise back, take that subtree (18 of the 53
answer) — and the catalog carries both `skeleton` (the flag) and
`skeleton_source` (that default as ONE inline TOML value), because a component
Add is an exact-source `Edit::Put` and `Put` needs a `value_source` string. A
component with no skeleton is LISTED with `skeleton: false` and the panel says
why it offers no Add. Serde's list deliberately omits `includes` (the resolver
strips it) and the keys `EntityConfig::from_toml` consumes before serde sees them
(`station`, `system`, `power_groups`, `shield_arc`), which is why
`component-unsupported` is asked of the RUNTIME at edit time rather than
compared against that list.

An edit is REFUSED at edit time with the source untouched: `entity.rs:1193`
`compose` applies `document::edit` to a copy, re-resolves and re-parses over
candidate ∪ dependencies with the edited member winning, and refuses
`include-missing`, `include-cycle`, `include-self`, `include-disallowed` (not an
`assets/entities/**.toml` path), `component-unsupported`, `entity-invalid`
(carrying the runtime's own parse error), `component-inherited` and
`unknown-document`, as `<rule>: <detail>`. Only rules the edit INTRODUCES are
refused (#1475's before/after multiset over `entity.rs:1131` `member_issues`),
because a fragment is legitimately not a complete entity on its own —
`tests/fixtures/mod-packs/partial-entity-include.zip` pins that — and a
hand-broken draft must be repairable one edit at a time. The panel's own
include pre-checks follow the same rule (`editor/workshop-entity.js:89`
`includeViolations`): a draft whose `includes` was hand-edited to hold a bad entry
can still have its other entries reordered or removed.

**Who authors a component is a STRUCTURAL question, never a provenance reading.**
`entity.rs:945` `component_owners` asks each contributing template's own document
whether it declares that top-level key, because `Provenance` records only the
WINNER of each leaf and the two come apart in both directions. A template that
overrides PART of an inherited component wins those leaves while the inherited
table is still beneath it — 38 component-and-fragment pairs on the shipped
composed hulls, `helm_console` on `assets/entities/alliance_cruiser.toml` among
them — and a template that SHADOWS every leaf a fragment authors wins them all
while that fragment's whole component waits to be composed back (`power` on all
four Harrow hulls). So `local` and `inherited_from` are independent, and the row
says both: a component with LOCAL text offers Remove and names the fragment whose
copy composes once the override is dropped (`workshop.entity.component_override`),
while a component with NO local text of its own offers no Remove at all, because a
whole inherited table has no tombstone the merge understands — that is the one
case `component-inherited` refuses, judged from the request BEFORE the edit runs
so the message names the owning member rather than `toml_edit`'s "the key does not
exist". A keyed array ENTRY does have a tombstone (`{ id = "…", _remove = true }`)
and is an ordinary local edit inside the array.

Materialising is the ONLY place a runtime value becomes source, and it writes NEW
local text only (criterion 2): `entity.rs:1244` `materialise` reads the resolved
value at the provenance address, serialises exactly that subtree and `put`s it at
the address's own path, so no existing span is rewritten. It refuses
`materialise-local` (already this template's own), `materialise-unknown-address`
(not in the resolved document, or a gap deeper than the one missing plain-key
level a `put` creates) and `materialise-keyed-entry` (the merge reconciles that
array by key and this template authors no entry with it — including a provenance
POSITION such as `system[2]`, the fallback for an entry carrying no key, which
`entity.rs:532` `put_path_in` refuses wherever it appears in the address because
the merge APPENDS a keyless entry: the resolved index and the local one are not
the same array). `FieldView::materialisable` is that same answer computed per row,
so the panel draws Materialise exactly where it would land. Browser:
`wasm.rs:164` `wasm_workshop_entity_edit`, `wasm.rs:183`
`wasm_workshop_entity_materialise`; native: `Operation::EntityEdit`,
`Operation::EntityMaterialise` → `Response::Patched`.

The same include rules are findings with lines: `entity.rs:1333` `findings` runs
beside `definitions::findings` and `composition::findings` in both
`validate_pack` and `validate_project` and reports `include-missing`,
`include-cycle`, `include-self` and `include-disallowed` at the offending
ENTRY's line, plus `entity-unresolvable` carrying the resolver's sentence and the
include chain. That last one follows `include_resolve::composition_finding`'s own
asymmetry — it fires only for a COMPOSED template, because a template that
composes nothing and is not a valid entity is the ordinary source error the
source gate already owns. Shipped content yields zero. It does not double up with
the world-driven composition check either: a hull a manifest root reaches carries
ONE `include-missing`, pinned on both the project and the pack path in
`src/workshop/tests.rs`.

The pure `editor/workshop-entity.js` plans every edit and maps every refusal:
`:36` `authoredInclude` computes the text the resolver reads (relative to the
DECLARING template's directory, the form `canonical_include_path` joins) from the
two member paths rather than by a second copy of the resolver's rules; `:122`
`planIncludeEdits` expresses a reorder as `set`s on the swapped slots so an
entry's comments stay with their positions, removes by descending index, inserts
new entries at the end and creates an absent key with one whole-array `put`;
`:170` `planComponentAdd` / `:183` `planComponentRemove`; `:251`
`planFieldEdits` for local scalars only, a row being read-only for one of three
named reasons — inherited and not yet materialised, a keyed address `:208`
`fieldSegments` cannot express (a split that tracks quoting exactly as
`entity.rs:317` `split_address` does, because `join_field` quotes any key holding
a dot, a bracket, an equals or a space), or a list or sub-table `:232`
`fieldIsScalar` refuses because `document::edit`'s `Set` refuses an array and an
inline table outright — and `:240` `fieldIsMaterialisable` withholding the
Materialise button for an inherited row the runtime could not write; `:366`
`refusalStringId` maps the runtime's rule prefix to a
`workshop.entity.refused.*` string with word-pattern fallbacks, sends
`unknown-document` and a stale `expected_source` to the shared
`workshop.inspector_stale`, and keeps the runtime's own sentence in the
catch-all's `{detail}`. `gui/workshop-entity-panel.js:20` `mountWorkshopEntity`
sends ONE runtime call per press and lands the answer as one `draft.edit`, so one
undo reverts it. Focus moves AFTER the busy hold comes down (`:435` `guarded`
applies the landing spot `:462` `land` returns), because every control is disabled
while the hold is up: a landing spot chosen inside it could only ever be one the
rebuild happened to recreate, and after removing a component the runtime has no
default for — no Add takes the Remove's place — focus fell to the document body.
Criterion 5 reuses the surfaces that exist: Preview drives the
models panel's own `#workshop-preview-subject` select and reveals the
`model-preview` dock panel, Test drives the Test panel's own
`#workshop-test-ship` select (that panel is not a dock panel, so there is nothing
to reveal), and a surface that cannot take the template answers false, which the
panel reports rather than appearing to work.

Known residue: a second unsupported component added to a document that already
carries one is not refused, because the runtime issue is keyed by its category
and reads as carried — the document was already unparseable and the author learns
nothing new. A LOCAL field whose provenance address is keyed is read-only too,
because the local array index an edit would need is not in the address; those
arrays are #1481's. Dropping the local override off a component a fragment also
authors composes the fragment's copy back — that is what the row says it does, and
the next reading shows the component as purely inherited — so an author who wanted
the component GONE has to remove the include or tombstone the entries, which is the
merge's own vocabulary and not this panel's. `findings` resolves every entity
member on every Check and
`catalog` calls it as well (≈0.1 s over the whole shipped `assets/` tree), which
is the cost the panel pays per refresh, matching `composition::catalog`. A local
value the syntax tree cannot address falls back to the resolved serialisation
with a null line rather than showing nothing.

## Workshop GM role presets and typed widgets (issue #1477)

The `presets` dock panel (Workshop layout v9, in the `files` column after
`composition` on both the browser and the native operator profile —
`gui/workshop-layout-model.js:52` `ADDED_IN_V9` and
`src/native_host/panes/operator.rs:366` `WORKSHOP_ADDED_IN_V9`, both
`[['presets','files']]`, because both forms edit a WORLD member) creates, edits,
reorders and removes `[[gm_role_preset]]` blocks: a preset's id and label, the
panels and quick actions it shows, the contacts it narrows to, and its typed
`[[gm_role_preset.widget]]` mission cards. #1483 (preview draft GM roles and
widgets) will extend this panel rather than add another, because it previews the
same authored presets.

**The runtime owns every rule it has, and `src/workshop/presets.rs` invents none.**
`src/world/config.rs:1013` `GmRolePresetWidget::validate` already refuses an
empty id or label, an unknown `type` (`config.rs:920` `GM_WIDGET_TYPES` —
`attention`, `workload`, `actions`, `note`, closed on purpose), a key that
belongs to another type, an unknown band or category, an empty ship, an unknown
or repeated GM action id (`config.rs:939` `GM_WIDGET_ACTION_IDS`) and a note
whose text is not a String Table id — but its whole-file caller `parse_world`
never knew a LINE, and criterion 2 asks for exact source locations. So this
module supplies the location and asks the runtime for the judgement:
`presets.rs:397` `probe` builds a widget carrying a valid id, a valid label, the
authored type and ONE authored facet, `presets.rs:693` `runtime_issue` reports
the sentence `validate` returns word for word, and `presets.rs:442` `owns` asks
the same question with a value the runtime ACCEPTS, so even the type-owns-key
table is read by probing rather than copied. `presets.rs:450` `vocabulary` reads
the band and category lists out of `GmAttentionBand::authored_vocabulary()` and
re-checks each through `from_authored`, so a spelling the runtime would refuse
can never reach the panel as a choice. `validate` was private and is now
`pub(crate)` for exactly this: an authoring refusal and a load refusal cannot
drift apart.

Six sentences are written in the module, for want of a runtime function to ask,
and the header says which are whose. FOUR repeat `parse_world`'s own words —
`preset-empty-id`, `preset-reserved-id`, `preset-duplicate-id` and
`widget-duplicate-id`, which `parse_world` builds inline in its own loop, per
index, with no per-entry predicate to call, and whose duplicate messages name
BOTH indices where a finding points at one line and names one. Those four are the
only place a reword in `world::config` could drift unnoticed, so
`presets/tests.rs` `the_preset_sentences_are_pinned_to_parse_worlds_own_words`
compares each clause by clause against `parse_world`'s own error (the offender
before the first delimiter, the reason after the last `;`) — verified by
rewording the runtime and watching it fail. TWO are Workshop-owned:
`preset-empty-label`, because a preset with no heading is a row an operator cannot
read and the form authors one, and the REFERENCE rules, because `parse_world` sees
one world's text and cannot resolve them. A widget's `ship` and a preset's `contacts` name world
entities by their `[[entity]] name`, and `presets.rs:379` `entity_names` resolves
that set as the selected world's own names plus those of the worlds its
`extra_worlds` compose, over candidate ∪ dependencies — so `known: false` on a
`contacts` entry or a `ship` is an ERROR with a line, never a silent flag. Ground
truth was wrong here and was verified against the runtime: `validate` refuses
only an EMPTY ship, never one the world lacks, and there is no preset-label rule
at all.

Those two reference rules are the Workshop's own, because the runtime's
declarative cross-reference checks are vacuous: `src/world/validate.rs:307`
`collect_entity_references` was emptied when #985 deleted the `[[trigger]]`
front-end, and a scripted world's references are resolved by
`world::script::validate` instead. An authoring form that offers the world's own
entity names should say when an authored one resolves to nothing, and the finding
names the LINE.

The finding is an ERROR, so it refuses save and export — and refusing content the
GAME would load is worse than missing a typo. A name a script mints with
`spawn_entity` is authored correctly and appears in no `[[entity]]` block, so the
known set counts those too: in a script text that calls `spawn_entity` at all,
every `name:` literal is taken as a name that world may mint. The set errs toward
ACCEPTING a reference, which is the direction a gate that blocks an export has to
err in, while a typo in a world whose scripts never spawn is still caught; a name
assembled from a variable is the one case nothing structural can see, and
`a_ship_or_contact_a_script_spawns_is_known_and_one_nobody_mints_is_not` pins
both halves. `preset-empty-label` is stricter than the game in the same way
(`GmRolePresetEntry.label` is `#[serde(default)]`), which is why the form's
repair of a MISSING `label` key had to work: see the residue note below.

`presets.rs:564` `catalog(files, dependencies, path)` reads ONE world member into
a `PresetCatalog` (`presets.rs:138`): the path and its origin (`draft`, `base`,
`pack:<id>`), every preset with its index and the 1-based line of its id and
label, `panels` and `quick_actions` as open-vocabulary entries with lines,
`contacts` and widget `actions` as references carrying `known`, one `WidgetView`
per widget (index, id, kind, label and their lines; `band`, `category`, `ship`
and `text` as `{value,line}` or null, which is what tells a `put` from a `set`
for those — an id, `type` or `label` has no null to read, so an EMPTY reading is
written with `put`, which inserts or replaces, and a non-empty one with `set`,
which keeps the key's decor and its type. The LINE cannot make that choice: an
absent key reads as the entry's own header line, so choosing by line planned a
`set` for a key that is not there, and the exact-source owner refuses a `set` it
cannot locate — which left a preset or widget whose `label` key was MISSING
unrepairable from the form, exactly the hand-broken world the form exists for),
the `unknown_keys` every level preserves and shows read-only, the RUNTIME-owned
`choices`, the world members a preset may be authored in, and every preset
finding over the whole candidate so the panel's list and Check agree. A path in
neither the draft nor its dependencies yields an empty catalog, an empty origin
and an `unknown-document` finding rather than a silently blank panel. Browser:
`src/workshop/wasm.rs:203` `wasm_workshop_presets(files, textDependencies, path)`;
native: `Operation::Presets` → `Response::Presets` (status `presets`), a Project
workspace resolving against nothing beneath through the provider's shared
`reference_dependencies`; codec helpers at `src/core/codec.rs:281`
`encode_workshop_preset_catalog` and `:288` `decode_workshop_preset_request`.

**The vocabulary is SPLIT, and the catalog says which half is which.** From the
runtime: widget types, widget action ids, attention bands and categories, and the
world's own entity names. From the BROWSER: the panel ids this build DRAWS and
the quick-action ids, `gui/gm-role-presets.js:47` `GM_ROLE_PRESET_PANEL_IDS`,
`:58` `GM_ROLE_PRESET_PANEL_FOLLOWERS` and `:80`
`GM_ROLE_PRESET_QUICK_ACTION_IDS`, imported by `editor/workshop-presets.js`
rather than restated. `panels`, `quick_actions` and `contacts` are open string
vocabularies in Rust ON PURPOSE — a preset may already name a panel this build
does not draw yet — so an authored id nobody draws is NOT an error: Rust raises
no such warning and offers no such choice, and
`editor/workshop-presets.js:467` `presetFindings` raises
`preset-panel-not-drawn` and `preset-quick-action-not-drawn` as WARNINGS from the
browser's own list, in the same findings list as the runtime's own. A browser
finding carries a string id plus params instead of a sentence, so the panel
localises it exactly as it localises a severity word and the pure module stays
free of `gui/strings.js`.

An edit is REFUSED at edit time with the source untouched: `presets.rs:1061`
`compose` applies `document::edit` to a copy, re-reads the preset rules over it
(`presets.rs:925` `preset_issues`, `:713` `widget_issues`) and refuses only what
the edit INTRODUCES — `presets.rs:1040` `introduced` compares a multiset of
(category, offending VALUE), never of messages, because every message names an
index and a reorder shifts every index after the moved entry. The browser's own
pre-check (`workshop-presets.js:265` `presetViolations`) keys each rule EXACTLY as
`Issue::key` does — an empty id or label by the key NAME, an unknown type by the
type — so the two agree about what is pre-existing. Keying a label or a type by
the entry's id instead made the browser refuse renaming any preset or widget that
already carried one: a pre-existing violation read as new, which is the one
mistake the introduced-only rule exists to stop, and a refusal `compose` would not
have made. The message shape
is `<rule>: <detail>` with the runtime's own sentence as the detail, which
`editor/workshop-presets.js:572` `refusalStringId` maps by prefix to a
`workshop.presets.refused.*` string. Beyond contract B's fourteen categories the
runtime also refuses `widget-empty-actions`, `widget-empty-text`,
`widget-invalid-text` and `widget-duplicate-action`, because each is a LOAD
refusal an edit could otherwise introduce unchecked; those four are named in
`workshop-presets.js:544` `DETAIL_ONLY_RULES` and take the catch-all row with the
runtime's sentence, rather than borrowing `empty` (which would claim an empty ID
for an empty BUTTON ROW) or `duplicate` (which would claim a duplicate PRESET for
one button named twice). `presets.rs:1116` `findings` reports the same rules as
located ERROR findings beside the other three catalogs in `validate_pack` and
`validate_project`; shipped content yields zero. `presets.rs:637`
`new_preset_source(id, label)` builds the `[[gm_role_preset]]` block a new preset
is appended as through `toml_edit` and then ASSERTS it through `parse_world`, and
is exposed as `wasm.rs:241` `wasm_workshop_new_preset` / `Operation::NewPreset` /
`runtime.newPreset`. That operation spells a preset's own id `preset_id`: the
bridge envelope owns the key `id` (`codec::decode_workshop_request` removes it
before the typed operation is read, and `createWorkshopBridge` spreads the
operation OVER `{ id, ...operation }`), so a preset id travelling as `id` would
replace the correlation number the reply is matched by.

`gui/workshop-presets-panel.js:29` `mountWorkshopPresets` renders the world
selector (draft members editable, base and pack read-only with their origin), the
Presets list with per-row id, label, Move up/down and Remove, an add-preset form
that refuses the reserved id, an empty id or label and a duplicate before the
runtime is asked — the duplicate judged against the READING, because the block is
appended to that source and form state hides a preset pending Remove and shows a
rename nobody applied, either of which would write one id twice; the same press is
refused outright while the forms hold unapplied edits, since appending to the
member they are editing forces a re-read that would drop them without a word —
and — for the selected preset — Panels and Quick actions as
checkbox lists over the BROWSER vocabulary (`:302` `renderFacet`, with every
authored-but-not-drawn value kept as an extra row that says so and can be
removed), Contacts added from the runtime's entity names, Widgets whose
per-widget controls FOLLOW the type (`:378` `renderWidgets`, gated by
`workshop-presets.js:124` `widgetOwns`, so a key is never OFFERED on a type that
does not own it) — including the TYPE itself, which is editable per row and not
only at Add, because the planner already rewrites the type and drops the old
type's keys as ONE group while the only route to it was otherwise
remove-then-add, two history entries and a new id — and Findings. The open-
vocabulary arrays (`panels`, `quick_actions`, `contacts`, a widget's `actions`)
are diffed by OCCURRENCE rather than membership, because Rust keeps them open with
no duplicate rule, so a hand-authored repeated entry is reachable and the Remove
beside one of two identical rows has to plan an edit; the surplus goes from the
tail so the kept rows keep their own lines. One Apply is ONE `runtime.editPresets` for ONE
member and ONE `draft.edit`, so one undo reverts it; focus moves after the busy
hold comes down, as the other three panels do. Which keys a type owns is itself
DERIVED in the browser too: `workshop-presets.js:101` `probeWidget` asks
`gui/gm-role-presets.js:185` `parseGmRolePresets` — the desk's own normaliser —
which keys a widget of that type keeps, so the rule has two readers and no third
copy.

**Criterion 4 is proved, not asserted.**
`tests/gm_role_preset_digest_neutrality.rs` runs the shipped probe world twice in
one process, seeded and `--deterministic`, comparing `world_digest` on every one
of 400 ticks, with exactly one difference between the arms: whether the world
carries presets. The authored arm is not hand-typed — it is built by
`presets::new_preset_source` plus one `presets::compose` `AppendTable` per widget
type, exactly as one Apply press is — so what is proved neutral is what the panel
actually writes, and the test asserts the run really simulated (the digest moves)
before concluding the two agree. Its own binary for the same reason
`tests/gm_presentation_neutrality.rs` is: Bevy's task pools are process-global,
so a determinism claim made beside other App-building tests is a claim about
whoever won that race. The browser half is the other direction:
`workshop-presets.js:134` `widgetActionChoices` intersects the runtime's action
list with `GM_WIDGET_ACTION_IDS` from the desk, because a widget button activates
a shipped control BY DOM ID and an id this build has no control for would be a
card with a dead button — and `tests/client/workshop-presets-panel.test.js`
asserts that every id the form OFFERS is in that vocabulary even when the runtime
lists a route AND the world authors one. The assertion is about what is offered,
not about what the page displays: an authored `__host*` id must be rendered, as a
removable row saying this build draws no button for it beside its warning, so
forbidding the string in the page would fail on exactly the row that lets an
author delete it while proving nothing about routes. The Rust binary also asserts
the two things that make its equality mean something: the LOADED `WorldConfig` of
the two arms is identical once `gm_role_presets` is set aside (so a preset block
changes no other authored input a GM's authority or the Admission gate could
read), and a third arm over the same world with one hull moved two metres must
produce a DIFFERENT digest series, so a `world_digest` blind to the world cannot
pass the comparison for the wrong reason. Its temp world fixtures remove
themselves through a `Drop`.

Known residue: a reorder is `set` edits on the swapped slots (the shape #1475's
roots use), and those edits can carry every scalar and list a preset holds but
NOT its `[[gm_role_preset.widget]]` tables nor an unknown key whose value the
catalog does not carry — so `workshop-presets.js:214` `presetMovable` refuses the
move on BOTH neighbours with a stated reason rather than handing one preset
another's widgets, and reordering such a preset is a source edit. That is a
CRITERION GAP, not a covered case: no edit in `document::edit`'s vocabulary moves
an array-of-tables entry, so criterion 1's preset reorder is delivered for
widget-less presets only — and both `[[gm_role_preset]]` blocks in the shipped
`assets/worlds/probe_gm_widgets.toml` carry widgets, so neither of them can be
reordered here. Each row now states which case it is in: the one that cannot be
carried says so, and one that CAN but whose neighbour cannot says that instead,
because a disabled control whose only explanation lives on the neighbour's row is
state without a word.
`widget-key-on-wrong-type` is not a browser pre-check at all: the planner writes a
key only when the type owns it and removes the old type's keys when the type
changes, so no edit it plans can introduce that violation, and a pre-existing one
is left byte-identical as the runtime's finding. An `actions` widget reports its
FIRST offending id only, exactly as the runtime refuses it, so fixing it reveals
the next. A ship or contact name minted only by a script is not structurally
knowable and reads as unknown; the finding names the line so an author can see
which reference is judged. `findings` re-reads every world member on every Check
and `catalog` calls it as well, which is the cost the panel pays per refresh,
matching `composition::catalog`.

## Workshop Rhai authoring (issue #1478)

The Rhai panel discovers both a world's inline `[script]` string blocks and its
sibling `.rhai` member. It mounts the existing script editor and reads completion
entries from `world::script::authoring::host_fns`, the descriptors collected by
the same registrations that build the runtime engine. Live diagnostics run the
ordinary bounded loading engine's compile and top-level execution pass and are
discarded when the draft revision, selected unit or editor source changes.

Saving an inline unit replaces only that TOML string payload. Saving a sibling
unit replaces only that member. The candidate is cloned and validated before one
grouped history operation reaches the draft, so refusal changes no source and an
accepted edit participates in undo, recovery, native save and browser export.

## Observing a disposable Test two ways (issue #1472)

One Test run can be watched through the authentic Viewscreen of a simulated
player ship or through the omniscient Game Master workspace. Switching is a
view change, not a restart: the same simulation keeps running at the same tick
while the surface changes.

Both levers are ones the game already classes as presentation.
`NativeGmPresentation` turns the ordinary GM projections on and off — a native
host already inserts and removes it beside a running ship — and `LocalShip` says
which hull is drawn. `tests/gm_presentation_neutrality.rs` proves the first one
is inert: it runs the same seeded world twice, switching the omniscient view on
mid-run and then on-and-off, and compares the digest on every tick. Before that
test, "the projection systems only read the world" was an inference.

The omniscient view is the REAL GM console. `scripts/build-workshop.mjs`
injects the `#gm-console` subtree from `server.html` into the Test page at build
time, the same single-source approach `native_gm/document.rs` takes at runtime.
A Workshop-only substitute would be a second surface to keep correct, and would
stop being evidence about the real one the moment it drifted.

It observes and nothing more. Every `window.__host*` GM write is bound to an
explicit refusal rather than left undefined: a missing global crashes a panel on
the first press and leaves "can this mutate the run?" answerable only by reading
every panel, whereas one list answers it and a Test that somehow acquired a real
route fails a test instead of quietly working.

It reads nothing its capture did not hand it, either. The ordinary GM workspace
fetches the private-feedback manifest, the cue catalogue and every sample they
name the moment it mounts — project assets outside the capture, which the Test
isolation smoke spec caught. The Test mounts the workspace `isolated`: feedback
audio gets a silent output with no manifest and the local audition is not
mounted, while every projection panel is unchanged.

The surface follows the runtime's reported view rather than the request that
asked for it, so a refused switch cannot leave the page claiming a view the run
is not drawing; the runtime refuses a ship the run does not have. A Test today
has one player ship, because the disposable boot keeps the default solo roster —
multi-ship Test authoring is #1153 — but the selector reads whatever player
ships the run actually has, so it needs no change when that arrives.
