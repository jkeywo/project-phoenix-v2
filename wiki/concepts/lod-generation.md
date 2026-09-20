---
title: LOD Generation
type: concept
tags: [tooling, assets, models, rendering, ci]
sources: [scripts/generate-lods.mjs, scripts/capture-billboards.mjs, scripts/viewer-lods.mjs, scripts/dev-viewer.mjs, scripts/blender-voxel-remesh.py, scripts/lod-manifest.toml, scripts/lod-capture-manifest.toml, src/entities/config.rs, src/entities/model_rig.rs, src/native_host/workshop/billboard_capture.rs, src/native_host/workshop/lod_generation.rs, editor/workshop-billboard-capture.js, editor/workshop-lod-generation.js, src/perf/assets.rs, src/perf/mesh.rs, tests/client/generate-lods.test.js, tests/client/capture-billboards.test.js, tests/client/viewer-lods.test.js]
updated: 2026-09-20
---

# LOD Generation

How a model's decimated LOD levels are produced, and how CI knows the ones in
the tree still match what the sidecars ask for (issue #919).

Generated GLB tiers that declare `tier_rig = "identity"` have no sibling
sidecar to fetch, but still inherit the primary model rig's offset and rotation
at runtime. Their extra scale lives on a presentation-only LOD root below the
authoritative entity, so it is applied exactly once without changing marker,
weapon, collision, or digest geometry. This keeps 180°-corrected hulls
nose-forward across every LOD transition while rendererless GM peers remain
simulation-identical to rendered peers.

## The sidecar declares the whole ladder

Since #914 a model's `[[lod]]` chain lives in its rig sidecar. Since #919 a
level that was *decimated out of another file* also carries the parameters that
produced it, so nothing about the ladder lives in a script:

```toml
[[lod]]
max_distance = 100.0
model = "assets/models/asteroid_common_1_lod1.glb"

[lod.generate]                                   # build-time only
source = "assets/models/asteroid_common_1.glb"
ratio = 0.25                                     # meshoptimizer vertex ratio
error = 0.01                                     # meshoptimizer error limit
texture_size = 512                               # max texture dimension (px)
```

`LodGeneration` (`src/entities/config.rs`) parses these under the sidecar's
strict schema and the renderer then ignores them entirely — by load time the
decimation has already happened. A level with no `[lod.generate]` is authored
by hand and the generator never touches it.

Authoring a ladder does not require a text editor: `scripts/viewer-lods.mjs`
reads and rewrites exactly these blocks (preserving the rest of the sidecar
byte for byte — every shipped sidecar round-trips unchanged), and validates a
proposed edit by running this script's own `collectTargets` over it before
writing anything.

## The command

```bash
npm run lods                                     # every declared output
node scripts/generate-lods.mjs asteroid_common_1 # one model
node scripts/generate-lods.mjs --plan            # print the work, run nothing
```

The model viewer runs the same command over the model it is showing, from the
same sidecars, with the `--remesh` and `--force` flags as checkboxes — see
[Model Viewer](./model-viewer.md). There is no second code path: the panel edits
the sidecar and shells out to this script, because a ladder that only the viewer
could produce would be a ladder CI's drift check could not verify.

Native Workshop runs that same script for the exact selected sidecar against a
private stage of the captured draft. Its pinned Node tools and optional
configured Blender stay in the selected project checkout. Progress is bounded,
and cancellation kills the child. Successful GLBs, an optional remesh
intermediate, the exact sidecar and the manifest are served from nonce-bearing
loopback routes. The exact candidate must pass ordinary validation and can be
rendered in the captured model preview before one grouped adoption. A changed
draft or selection, tool failure, Test start, surface loss or exit removes the
stage without changing the draft. Browser Workshop has no process capability.

It reads every sidecar under `assets/models`, de-duplicates by output path (the
small/large/huge/cosmetic variants of one rock share one generated `.glb`, and are
required to agree about it), and runs `simplify` → `resize` through the pinned
`@gltf-transform/cli`. The planning half is pure and unit-tested in
`tests/client/generate-lods.test.js`; nothing there reads a file or spawns a
process.

## Drift, not rebuild

`scripts/lod-manifest.toml` records, per generated file, the hash of its source,
the parameters it was made with, and the hash and byte size of the output.
`npm run lods:check` (CI, `editor-test` job) re-hashes those three and fails on
any mismatch — a replaced source, a retuned ratio, or a hand-edited binary.

CI does **not** re-run gltf-transform: meshoptimizer output is not guaranteed
byte-identical across CLI versions, so a regenerate-and-diff gate would go red
for reasons that are not drift. The recorded `output_bytes` is asserted against
`src/perf/assets.rs`'s own inventory (issue #868) by a Rust test, so file size
has one measurement and two readers. Triangle and texture budgets belong to
issue #905. Both performance inventories exclude checked-in `.remesh.glb`
intermediates: they are generator inputs, not files the runtime can select.

Billboard PNGs have a parallel but separate contract. Every `[[lod]]` level
with `[lod.capture]` is recorded in `scripts/lod-capture-manifest.toml` with its
source/output hashes and byte size, authored yaw-view/resolution/pitch recipe,
capture-recipe version, every declaring sidecar, and the canonical `[base]`
transform the native renderer applies. That base is taken from the capture
source's `.model.toml`, or is explicit identity when no such sidecar exists;
variant billboard scales are not part of the shared atlas recipe.

`npm run lod-captures:check` re-hashes and checks PNG dimensions without a GPU
or capture binary. `node scripts/capture-billboards.mjs --adopt <model>` records
an already-reviewed committed PNG, while the default command performs a real
recapture and forwards all three authored parameters to the renderer. A
successful capture from the model viewer refreshes the same manifest record,
so browser and batch authoring cannot leave different provenance behind.

Native Workshop runs the production selected-target capture workflow for one
selected draft sidecar. It stages the exact draft in private temporary storage
and serves a nonce-bearing loopback review containing the PNG, every declaring
sidecar's exact dimension patch, and the refreshed capture manifest. The image
and its source revision, sidecar, LOD index and authored recipe are shown before
adoption. Adoption validates that complete candidate and lands every member as
one undoable draft transaction, so `lod-captures:check` reads the same metadata
as batch capture. Cancellation, a changed selection or draft, a replaced
surface, Test start, process failure or exit kills the process tree and removes
the temporary files and routes. Browser Workshop has no capture capability; its
ordinary model preview still renders packaged billboard levels.

The mesh-interior pass follows the runtime graph from top-level entity
templates into the selected model+variant sidecar. It loads every GLB named by
that sidecar so an oversized lower level still affects the per-level maximum,
but its aggregate triangle population sums only the deduplicated first/near
GLB level. Lower levels are mutually exclusive at runtime, and their sidecars
do not recursively introduce another ladder. A missing, malformed, or empty
ladder falls back to the entity's flat `[mesh] model`, matching rendering.

## Per-level runs

`generate-lods.mjs` filters targets on any substring of their output, source or
sidecar paths, so a level's own output path selects exactly that level. That is
how the viewer's per-LOD **Generate** button works — one decimation at a time
while it is being tuned, rather than a whole-model run:

```bash
node scripts/generate-lods.mjs assets/models/asteroid_common_4_lod2.glb
```

## Stubborn meshes

`asteroid_common_4` is the worked example of a mesh meshoptimizer cannot reduce
— mostly split vertices, so simplification stalls around 67% however loose the
error bound. Its shipped levels predate the script and are recorded as-is. The
optional pre-pass for this case is `scripts/blender-voxel-remesh.py`
(`remesh_voxel_size` in `[lod.generate]`, then `--remesh`), which rebuilds a
watertight surface that decimates predictably; it writes a checked-in
intermediate, so only whoever re-runs the pre-pass needs Blender. Any level that
comes out larger than the file it replaced is reported at the end of a run.
