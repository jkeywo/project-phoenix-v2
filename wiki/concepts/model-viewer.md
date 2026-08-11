---
title: Model Viewer
type: concept
tags: [tooling, rendering, shaders, wasm, trunk]
sources: [viewer.html, viewer-trunk.toml, start-viewer.bat, scripts/dev-viewer.mjs, scripts/generate-entity-index.mjs, scripts/stitch-planet-textures.mjs, scripts/viewer-lods.mjs, assets/planets/, assets/shaders/planet_surface.wgsl, assets/shaders/planet_clouds.wgsl, src/viewer/, src/render_setup.rs, src/entities/glb_visual.rs, src/entities/celestial_visual.rs, src/entities/mesh_stats.rs]
updated: 2026-08-11
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
`viewer_set_gizmos`, `viewer_load_model`, `viewer_load_entity`, `viewer_set_lod_mode`,
`viewer_set_camera_distance`, `viewer_stats` — all `#[wasm_bindgen]` in
`src/viewer/mod.rs`). The panel lives in JS so tweaking a slider costs an HTML
edit rather than a wasm rebuild.

The **Subject** selector switches between the model/variant workflow and an
entity picker generated from top-level `assets/entities/*.toml` files. Entity
mode uses the authored `[star]`, `[planet]`, or `[mesh]` visual and deliberately
disables model-LOD authoring; the existing model picker, variants and LOD tools
are otherwise unchanged.

## The LOD panel

The viewer is where a decimated model is judged, so it also authors and
regenerates the ladder. Three parts:

**Showing a level** (`src/viewer/lod.rs`). *Base* renders the selected `.glb`.
*Hold* pins one level at any distance — the mode for orbiting a decimated hull.
*Auto* is the game's own behaviour: distance runs through
`entity_config::select_lod`, hysteresis and all, so "does it pop" is answered by
the function that decides it in play. Drag the **camera distance** slider in
Auto and the ladder walks its real thresholds. A far level declared as
`shape = "sphere"` is built through `server_app::procedural_mesh_material`, the
same constructor `update_mesh_lod` uses. **range** puts the camera at a band's
far edge.

Every GLB level is requested up front and its handle held on `LadderState`
(`preload_levels`), so a swap is a handle change rather than a fetch — matching
the game, which preloads a whole ladder the frame the sidecar lands
(`discover_sidecar_lod_assets`).

The panel edits a *working copy* and pushes it into the engine on every
keystroke (`viewer_ladder_begin`/`_push`/`_commit`), so a switch distance can be
dragged and judged in Auto before it is saved. The sidecar stays the authority;
the push only survives until the model changes.

**Costing it** (`src/viewer/stats.rs`). Triangles and texture pixels of what is
on screen, counted with `entities::mesh_stats` — the same two functions the perf
baselines use (issue #905), so a number here and a number in `assets-mesh.ron`
mean the same thing. File bytes come from the dev server, which can `stat` them.

**Editing and generating** (`scripts/viewer-lods.mjs`, `/api/` in
`scripts/dev-viewer.mjs`). The panel writes `[[lod]]` and `[lod.generate]` back
into *every* rig sidecar of the model — the variants share the generated files
and are required to agree about them — and runs `scripts/generate-lods.mjs` over
it, streaming the transcript back. See [LOD Generation](./lod-generation.md).

Two rules the panel does not own: the ladder's shape rules live in
`viewer-lods.mjs` and the decimation parameter rules in the generator's own
`collectTargets`, which is run over the *proposed* sidecar text before anything
is written. A save that the generator would refuse is refused here instead, with
the tree untouched.

`Copy ladder from` rebuilds an existing model's ladder for the current one:
same ratios and texture sizes, switch distances scaled by the extents ratio. It
lands in the editor as a proposal to check, not a saved file — the precedent
lives in the sidecars rather than in a table inside the tool.

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

The planet materials do their star-relative lighting in custom WGSL rather
than through Bevy's scene lights. In the viewer, `PlanetLightingOverride`
therefore receives the same off/ambient/directional controls so the buttons and
sliders drive both GLB materials and celestial materials.

Planet maps are authored as periodic equirectangular textures. The checked-in
maps have their generator-feathered vertical border removed and a clean periodic
join rebuilt by `scripts/stitch-planet-textures.mjs`; every aligned map in a
texture set (surface, clouds, normal, roughness, emissive and masks) receives the
same longitude remap. Normal vectors are renormalised after resampling.

## Notes

- The `viewer` cargo feature depends on `server`, because the simulation
  modules reference `crate::bridge` and `crate::server::renderer` directly and
  the crate does not compile without it. The viewer registers none of the
  simulation systems, so this costs binary size, not behaviour.
- Bevy asset hot-reload does not work on wasm. Editing a `.wgsl` triggers a
  Trunk rebuild and page reload — that reload *is* the iteration loop.
- The model dropdown reads `assets/models/index.json`, regenerated on every
  build by `scripts/generate-model-index.mjs` (a Trunk `pre_build` hook) so it
  cannot go stale. The file is gitignored, and `dev-viewer.mjs` also writes it
  once *before* starting Trunk: the file is on Trunk's `[watch] ignore` list,
  and Trunk canonicalises those paths at startup, so in a fresh checkout the
  hook that creates it never got to run.
- `assets/models/` is deliberately outside Trunk's `watch_path`. It was inside
  it, and every Save (three sidecars) and every generated level cost a full wasm
  rebuild and a page reload — the tool reloading the page underneath its own
  edits. The panel owns that refresh instead: a finished run calls
  `viewer_reload_assets`, which re-fetches through `AssetServer::reload` and
  rebuilds the subject when the new bytes land (`respawn_on_asset_reload` waits
  for `AssetEvent::Modified`, because the old value stays in `Assets<Scene>`
  until the new one arrives). Hand-editing a sidecar now needs a manual browser
  reload; editing a `.wgsl` still reloads on its own.
- The panel keeps its subject kind, model/entity, variant, LOD mode, complete
  orbit camera state, lighting controls, skybox brightness and gizmo toggle in
  `sessionStorage`, and picks a running generator's transcript back up if the
  page does reload.
- `start-viewer.bat` waits for port 8081 to accept a connection before opening
  the browser. Trunk does not bind it until the first wasm build finishes,
  which is minutes from cold.
- `trunk build` (as opposed to `trunk serve`) currently fails on Windows for
  *any* target in this repo when the output directory is fresh — Trunk cannot
  rename the staged `assets` tree into place (`os error 5`). Pre-existing;
  unrelated to this tool.
