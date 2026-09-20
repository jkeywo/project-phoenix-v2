---
title: Workshop Model Preview
type: concept
tags: [tooling, rendering, shaders, workshop, native]
sources: [workshop.html, editor/workshop-launch.js, editor/workshop-model-preview.js, editor/workshop-model-structure.js, gui/workshop-models-panel.js, gui/workshop-model-preview-panel.js, scripts/dev-workshop.mjs, scripts/generate-lods.mjs, scripts/capture-billboards.mjs, scripts/viewer-lods.mjs, scripts/lod-capture-manifest.toml, src/viewer/preview.rs, src/viewer/lod.rs, src/viewer/stats.rs, src/render_setup.rs, src/entities/glb_visual.rs, src/entities/celestial_visual.rs, src/entities/mesh_stats.rs]
updated: 2026-09-20
---

# Workshop Model Preview

Workshop is the only supported model, rig and entity preview surface. Its Models
document previews a GLB, one named sidecar variant, or a composed entity such as
a ship, star or planet. Preview input is an immutable capture of the selected
draft revision; source changes mark that picture stale until the author refreshes.

`src/viewer/` remains the shared renderer plugin. It is used by the Workshop
preview and by render-parity tests, but no longer owns an independent HTML shell,
history, project state or filesystem API. `viewer.html` is a small compatibility
redirect which translates safe legacy query parameters into the versioned
Workshop launch fragment. The old `editor.html` URL does the same for a selected
source file. Invalid or unavailable selections are reported in Workshop and do
not broaden filesystem authority.

```bash
npm run dev:viewer
start-viewer.bat
```

Both commands build and open native Workshop directly on the current project
with the Models panel selected. The npm command accepts the retained selection
vocabulary: `--model=assets/models/...glb`, `--variant=name`,
`--entity=assets/entities/...toml`, `--lighting=off|ambient|directional` and
`--gizmos=0|1`; unknown, duplicate or invalid selectors refuse. Windows batch
launchers deliberately interpolate no command-line arguments. Their optional
selection is the `PHOENIX_WORKSHOP_OPEN` URL query environment variable, read
and validated directly by Node (for example `model=assets/models/ship.glb&gizmos=1`,
or `file=assets/worlds/demo.toml` for `start-editor.bat`). Native Workshop owns
project writes and external model tools; browser Workshop remains archive-only.

The model preview uses the game's render path rather than a copied scene:

- `render_setup` owns skybox, camera optics and default ambient light.
- `glb_visual` loads GLB scenes and applies model-rig sidecars.
- `celestial_visual` draws authored star and planet materials.
- `viewer::lod` uses the runtime LOD selector and hysteresis.
- `viewer::stats` uses the same mesh and texture accounting as performance
  baselines.

The structured Models panel edits markers, target points, named variants, LOD
records and capture metadata as exact-source grouped transactions. Preview,
LOD generation/remesh and billboard capture all operate on a selected captured
revision. Adoption validates one candidate and lands its sidecars and generated
assets as one undoable transaction. Cancellation, selection changes, stale
revisions, Test entry, pane failure and Workshop exit retire any native job and
leave the draft unchanged.

Lighting and camera controls are presentation-only. Gizmos show authored rig
markers, target points and extents. Auto, Base and fixed LOD modes all drive the
same retained renderer state. The browser omits native generation and capture
capabilities while still rendering packaged preview inputs.

CI keeps the `viewer` Rust feature as an architectural boundary test: it must
compile without the server feature, its focused tests must run, and the shared
preview plugin must continue to render non-flat models through Workshop. This
feature name describes the renderer, not a standalone application.
