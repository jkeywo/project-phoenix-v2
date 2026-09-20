---
title: Workshop Authoring
type: entity
tags: [workshop, editor, tooling, scenario, entity, models, mod]
sources: [workshop.html, editor.html, viewer.html, editor/workshop-launch.js, editor/workshop-document.js, editor/workshop-runtime.js, editor/workshop-recovery.js, editor/workshop-provider.js, editor/workshop-source-provider.js, editor/workshop-preview.js, editor/workshop-preview-runtime.js, editor/workshop-composition.js, editor/workshop-entity-composition.js, editor/workshop-ship-authoring.js, editor/workshop-role-presets.js, editor/workshop-spatial.js, editor/workshop-scripts.js, editor/workshop-models.js, editor/workshop-model-structure.js, editor/workshop-test.js, editor/mod-pack-workspace.js, editor/mod-pack-export.js, gui/workshop-boot.js, gui/workshop-redirect.js, gui/native-workshop.js, gui/workshop-authoring.js, gui/workshop-models-panel.js, gui/workshop-model-preview-panel.js, gui/workshop-layout-model.js, gui/workshop-layout-renderer.js, gui/workshop-test-panel.js, scripts/build-workshop.mjs, scripts/dev-workshop.mjs, src/workshop/mod.rs, src/workshop/document.rs, src/workshop/provider.rs, src/workshop/archive.rs, src/native_host/workshop/mod.rs, src/native_host/workshop/bridge.rs, src/native_host/workshop/document.rs, src/native_host/workshop/preview.rs, src/delivery/args.rs, src/viewer/preview.rs, pasm/spec/architecture/workshop-model-authoring.yaml, pasm/spec/architecture/workshop-live-inspector.yaml]
updated: 2026-09-20
---

# Workshop Authoring

Workshop is the single authoring shell for worlds, entities, roles, scripts,
spatial composition, models and mod archives. Native Workshop opens an explicit
project or mod root through the host provider. Browser Workshop imports and
exports archives without gaining filesystem authority. Both use the same exact
source document, grouped history, validation, preview and disposable-Test
components; neither is part of the authoritative running simulation.

## Shell and persistence

`WorkshopDocument` owns exact imported or native sources, dirty state and one
chronological `UndoStack`. Native persistence passes through the bounded
Workshop provider; browser persistence is recovery plus validated archive
export. Every mutation validates a candidate before it enters shared history.
Unknown fields, comments, ordering, BOMs and untouched line endings remain exact.

## Modes

- Files and Source select and edit exact documents.
- Composition, Entity, Ship and Spatial forms produce exact-source grouped changes.
- Scripts edits inline and sibling Rhai using the runtime host-function registry.
- Models edits rig/LOD structure and previews immutable captured candidates.
- Test runs a disposable simulation from the exact unsaved draft.
- Validate, Save and Export share the ordinary runtime candidate boundary.

MOD ZIP import, Validate and Export remain the `editor.mod` semantic actions.
They use the shared feedback and remapping lifecycle, retain each ordered
archive member's exact bytes, and replace a workspace only after readable input
has been captured. ZIP paths and text use the same bounded validation and fatal
UTF-8 rules as the Rust upload reader. Validation findings stay attached to the
candidate source; refusal leaves the previous workspace intact.

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

## Live handoff

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
model/rig field form, the captured model preview, sound audition and Rhai editor can split, tab,
float in-surface, close and reset while lifecycle commands remain in the fixed
menu/toolbar. Each registered panel carries a class: `source` and the model preview
are *documents* — the surfaces the context is arranged around, stamped
`data-panel-kind="document"`; Rhai is also a document, and every other panel is a *tool*. The class is
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
profile sanitizer mirrors the browser's version-7 panel registry and migration.
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

The Composition tool patches a selected world's top-level `extra_worlds` array
and the selected manifest's root scenario blocks without reserializing either
file. It inventories Rhai and TOML load/unload references and labels each world
as editable draft, immutable base content or immutable retained-pack content.
Each add or remove is first applied to a private candidate copy, checked for
missing, cyclic, duplicate and disallowed references, and passed through the
same runtime validator used by Test, browser export and native save. Only an
accepted, still-current candidate becomes one undo entry; refusal or a stale
asynchronous result leaves the exact draft bytes and history unchanged.

The same Composition panel resolves editable entity templates through
`entity-includes.js` and presents their effective components and field source
owners. Base and retained-pack fragments remain read-only. Adding or removing a
local component or include, and deliberately materialising an inherited field
as a local override, changes only the selected template's exact source. Each
candidate resolves its complete include closure and passes ordinary runtime
validation before one history entry is recorded. Preview, Test, browser export
and native save therefore consume the same unsaved composed entity the author
reviewed; recovery retains that authored source and grouped undo entry.

Playable-ship authoring builds on that exact composed view. It lists effective
Stations and Systems with local or included source ownership, while edits stay
limited to local array-table blocks. Station fields, System membership and
runtime kind, each rating's automated-System references, and typed doctrine
kinds and target fields use ordinary form controls backed by the Rust
System/Directive registries. A candidate topology
must pass source-linked ownership and reference checks plus the ordinary runtime
validator before one undo entry is written. Test captures that same unsaved
selected hull; its participant-free start leaves every authored Station on
Backfill and therefore exercises the authored consoles and AI policies rather
than a separate Workshop mock.

The Composition panel also authors the runtime's existing presentation-only GM
role-preset schema. Native form controls create, edit, order and remove presets,
their panel, quick-action and contact assignments, and typed attention, workload,
actions and note widgets. The form draws its choices from the ordinary GM panel,
action, attention-band and attention-category vocabularies and world entity names;
unknown kinds, invalid references and options that do not belong to the selected
widget type are source-linked refusals. Mutations patch or move only the selected
TOML block, preserving comments, unknown fields, order and unrelated tables. The
ordinary candidate validator runs before the shared history write, so recovery,
save, export and Test all consume the same unsaved accepted source. These controls
compose no new action and do not change GM authority or admission.

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

Test may also launch with one typed root-world or loaded-layer state breakpoint:
a boolean Flag value or integer counter comparison using the runtime's existing
Flag vocabulary. `workshop/test_breakpoint.rs` observes that store in `FixedLast`
after the completed `SimTick`, then holds the same shared Test clock and discards
remaining fixed overstep. Browser and native therefore stop on the same logical
boundary. The status carries the condition, current value, source when a matching
Flag trace supplied one, and nearby bounded trace rows. Step advances one ordinary
fixed tick, Resume continues without immediately retriggering a still-true
condition, and Restart creates a fresh runtime. Breakpoint state is Test-local and
does not enter snapshots, digests, save/export data or the Rhai interface.

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

Role preview and remaining specialised entity/definition panels
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
- [Workshop Model Preview](../concepts/model-viewer.md)
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
statistics. Browser capture transfers those exact bytes to a private preview
frame. Native capture materialises them in the source worker and serves each
member from a nonce-scoped, immutable, loopback-only URL; the JSON bridge carries
only authored references and a path manifest. Retired capture paths return Not
Found before static bundle resolution, so they cannot fall through to project or
delivery assets. A late old capture id cannot retire the current generation.
Test, draft replacement, pane failure/replacement and exit stop the preview and
withdraw its native routes. External
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

## Workshop Rhai authoring (issue #1478)

The Rhai panel discovers both a world's inline `[script]` string blocks and its
sibling `.rhai` member. It mounts the existing script editor and reads completion
entries from `world::script::authoring::host_fns`, the descriptors collected by
the same registrations that build the runtime engine. There is no Workshop copy
of the host-call vocabulary.

Live diagnostics run the ordinary bounded loading engine's compile and top-level
execution pass. Each result carries the selected source file and its line offset;
the panel also captures the current draft revision and discards a response after
the draft, selected unit or editor source changes. Final acceptance remains the
ordinary Workshop validation pass because that also checks composition,
references, dependencies and the complete candidate pack.

Saving an inline unit replaces only that TOML string payload. Saving a sibling
unit replaces only that member. The candidate is cloned and validated before one
grouped history operation reaches the draft, so a refusal changes no source and
an accepted edit participates in the existing undo, crash recovery, native save
and browser export flows. The panel adds no simulation authority, source-level
stepping, local-variable inspection or stack inspection.

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

The same captured run supplies its exact unsaved `[[gm_role_preset]]` and typed
widget descriptors through `wasm_get_gm_role_presets`, the export used by the
live GM page. Test feeds that payload into the ordinary role selector and
widget panel only after startup succeeds. The choice is local to the disposable
iframe: a removed preset uses the ordinary All fallback, reappearing source can
resolve the still-local choice, and restart begins at All. Retiring a boot before
startup prevents its descriptor payload from reaching a replacement page.
Role selection and cards remain presentation only; every Test GM write binding
continues to be an explicit refusal.
