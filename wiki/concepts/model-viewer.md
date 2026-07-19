---
title: Model Viewer
type: concept
tags: [tooling, rendering, shaders, wasm, trunk]
sources: [viewer.html, viewer-trunk.toml, start-viewer.bat, scripts/dev-viewer.mjs, src/viewer/, src/render_setup.rs, src/entities/glb_visual.rs, src/entities/celestial_visual.rs]
updated: 2026-07-19
---

# Model Viewer

A second Trunk target (`viewer.html`, port 8081) that renders **one** subject —
a GLB model, or a star/planet from an entity TOML — through the game's own
render path, with lighting switchable between off / ambient / directional.

It exists because judging appearance previously meant booting `server.html`,
joining a lobby, starting a scenario and flying to the thing. That loop is too
slow for tuning lighting or WGSL.

```bash
npm run dev:viewer     # → :8081
start-viewer.bat       # Windows: same, plus a compile check and opens the browser
```

| URL parameter | Effect |
|---|---|
| `model=assets/models/alliance_cruiser.glb` | GLB to render (default) |
| `variant=large` | rig sidecar variant (`large`, `small`, `cosmetic`, `lod1`…) |
| `entity=assets/entities/sol.toml` | render that config's `[star]`, `[planet]` or `[mesh]` |
| `lighting=off\|ambient\|directional` | initial lighting mode (default `ambient`) |
| `gizmos=1` | overlay rig markers, target points and the extents box |

An HTML control panel drives the same settings live (`viewer_set_lighting`,
`viewer_set_ambient`, `viewer_set_directional`, `viewer_set_skybox_brightness`,
`viewer_set_gizmos`, `viewer_load_model` — all `#[wasm_bindgen]` in
`src/viewer/mod.rs`). The panel lives in JS so tweaking a slider costs an HTML
edit rather than a wasm rebuild.

## Render parity is structural, not copied

The viewer would be worthless as a reference if it reimplemented the render
setup, so the shared pieces were extracted and both callers now use them:

| Module | Owns | Used by |
|---|---|---|
| `src/render_setup.rs` | space skybox + cubemap conversion, camera optics (`far = 5000`), default ambient fill | `RendererPlugin`, viewer |
| `src/entities/glb_visual.rs` | GLB scene load + `.model.toml` base-rig composition, sidecar resolution | `render_spawned_entities`, `update_mesh_lod`, viewer |
| `src/entities/celestial_visual.rs` | star surface + halo, planet surface + cloud shell (all custom WGSL) | `render_spawned_entities`, viewer |

`spawn_glb_visual` is deliberately ignorant of the simulation: it returns the
`SceneRoot` child it spawned, and callers decorate it. The game's local ship
adds `Visibility::Hidden` + `NoFrustumCulling` that way
(`decorate_local_ship_model` in `server_app.rs`).

## Lighting modes

- **Off** — no scene lights; only the skybox reaches the surface. Shows raw
  albedo and emissive.
- **Ambient** — `render_setup::default_ambient_light()`, i.e. what a world
  without an `[ambient_light]` block actually renders with. The mode to judge
  "does this look right in game".
- **Directional** — ambient plus a steerable key light, for normal maps,
  specular response and self-shadowing.

## Notes

- The `viewer` cargo feature depends on `server`, because the simulation
  modules reference `crate::bridge` and `crate::server::renderer` directly and
  the crate does not compile without it. The viewer registers none of the
  simulation systems, so this costs binary size, not behaviour.
- Bevy asset hot-reload does not work on wasm. Editing a `.wgsl` triggers a
  Trunk rebuild and page reload — that reload *is* the iteration loop.
- The model dropdown reads `assets/models/index.json`, regenerated on every
  build by `scripts/generate-model-index.mjs` (a Trunk `pre_build` hook) so it
  cannot go stale. The file is gitignored.
- `trunk build` (as opposed to `trunk serve`) currently fails on Windows for
  *any* target in this repo when the output directory is fresh — Trunk cannot
  rename the staged `assets` tree into place (`os error 5`). Pre-existing;
  unrelated to this tool.
