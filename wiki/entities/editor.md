---
title: Editor
type: entity
tags: [editor, tooling, scenario, entity, definitions, models, mod]
sources: [editor/app-v2.js, editor/scenario-mode.js, editor/mode-shell.js, editor/project-root.js, editor/save-flow.js, editor/invalidation-bus.js, editor/entity-cache.js, editor/validation.js, editor/world-toml.js, editor/entity-toml.js, editor/models-mode-view.js, editor/mod-mode-view.js, editor/mod-actions.js, editor/mod-pack-workspace.js, editor/mod-pack-export.js, gui/editor-mod-actions.js, gui/client-semantic-actions.js, gui/operator-profile.js, gui/semantic-controls-remapper.js, workshop.html, editor/workshop-document.js, editor/workshop-runtime.js, editor/workshop-recovery.js, src/workshop/mod.rs, src/workshop/document.rs, src/workshop/provider.rs, src/workshop/archive.rs, src/workshop/provider/assets.rs, src/workshop/provider/test_snapshot.rs, src/workshop/test_protocol.rs, src/native_host/workshop/test_clock.rs, src/native_host/workshop/test_process.rs, editor/workshop-test.js, gui/workshop-test-panel.js, src/native_host/workshop/mod.rs, src/native_host/workshop/bridge.rs, src/native_host/workshop/document.rs, src/native_host/workshop/keyboard.rs, src/boot/mod.rs, src/delivery/args.rs, editor/workshop-provider.js, gui/native-workshop.js, gui/workshop-authoring.js, gui/workshop-layout-model.js, gui/workshop-layout-renderer.js, scripts/build-workshop.mjs, run-workshop.bat, scripts/serve-workshop.mjs, assets/audio/sound-cues.toml, src/sound_cues.rs, gui/sound-audition-panel.js, editor/workshop-sound-cues.js, src/world/pack_asset_validation.rs, src/audio_decode.rs, src/entities/pack_assets.rs, src/entities/pack_assets/versioned.rs, editor/asset-dependencies.js, editor/workshop-assets.js, pasm/spec/architecture/workshop-runtime-assets.yaml, editor/workshop-handoff.js, editor/workshop-source-provider.js, gui/workshop-source-link.js, pasm/spec/architecture/workshop-source-handoff.yaml, src/workshop/test_clock.rs, src/workshop/test_source.rs, src/workshop/test_browser.rs, workshop-test.html, editor/workshop-test-frame.js, editor/workshop-test-child.js, editor/workshop-test-runtime.js, editor/workshop-test-snapshot.js, gui/workshop-test-boot.js, tests/smoke/workshop-test-runtime.render.spec.js, src/entities/pack_assets/snapshot.rs, editor/workshop-models.js, gui/workshop-models-panel.js, gui/workshop-model-preview-panel.js, pasm/spec/architecture/workshop-model-authoring.yaml, src/workshop/model_fields.rs, src/inspector.rs, gui/inspector-field.js, pasm/spec/architecture/workshop-live-inspector.yaml, src/workshop/definitions.rs, editor/workshop-definitions.js, gui/workshop-definitions-panel.js, src/entities/config_cache.rs, pasm/spec/architecture/workshop-definition-authoring.yaml, src/headless/app.rs, src/workshop/composition.rs, src/workshop/source_spans.rs, editor/workshop-composition.js, gui/workshop-composition-panel.js]
updated: 2026-09-18
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

`src/workshop/definitions.rs:509` `catalog` reads the draft's text members plus
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
the enemy first and the faction second. `definitions.rs:774` `findings` runs from
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

`src/workshop/composition.rs:828` `catalog` reads the draft's text members plus
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
source untouched: `composition.rs:1250` `compose` applies `document::edit` to a
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
`composition.rs:972` `new_world_source` (a `[global]` with the title, built
through `toml_edit` and asserted through `parse_world`) put at
`assets/worlds/<slug>.toml`. Browser: `wasm_workshop_compose`,
`wasm_workshop_new_world`; native: `Operation::Compose`, `Operation::NewWorld`
→ `Response::Patched`.

The same rules are findings with lines: `composition.rs:1327` `findings` runs
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
"The same validated catalogue" is `composition.rs:934` `scenario_catalogue`:
`world::manifest::build_catalog` over the candidate manifest resolving worlds
through candidate ∪ beneath, which `src/workshop/tests.rs` proves equal entry
for entry between the pack path (store zip plus dependency bundle) and the
project path (the same members as files, nothing beneath); the panel's
Catalogue section is that list. `src/workshop/test_source.rs:38`
`validate_selection` accepts a root whose draft-declared child exists only in
the candidate and refuses when the child is missing, so the exact unsaved
composition is what a Test runs. It also applies
`composition.rs:1417` `selection_findings` — the composition rules in the
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
