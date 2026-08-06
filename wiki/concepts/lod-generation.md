---
title: LOD Generation
type: concept
tags: [tooling, assets, models, rendering, ci]
sources: [scripts/generate-lods.mjs, scripts/viewer-lods.mjs, scripts/dev-viewer.mjs, scripts/blender-voxel-remesh.py, scripts/lod-manifest.toml, src/entities/config.rs, src/entities/model_rig.rs, src/perf/assets.rs, tests/client/generate-lods.test.js, tests/client/viewer-lods.test.js]
updated: 2026-08-03
---

# LOD Generation

How a model's decimated LOD levels are produced, and how CI knows the ones in
the tree still match what the sidecars ask for (issue #919).

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
issue #905.

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
