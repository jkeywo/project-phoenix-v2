---
title: Asset Preload
type: concept
tags: [assets, gltf, sidecar, preload, lobby, loading-phase]
sources: [src/server/asset_preload.rs, src/server/pfx.rs, src/server_app/registration.rs, src/server_app_render.rs, src/entities/config_cache.rs, src/entities/world_preload.rs, gui/host-content-fetch.js, src/entities/model_rig.rs, src/entities/model_markers.rs, src/lobby/server.rs, src/core/messages.rs, server.html, client.html]
updated: 2026-09-08
---

# Asset Preload

The server discovers and pre-caches render assets referenced by the selected scenario while the session is still in Lobby/Loading. A game does not enter `InProgress` until the manifest reaches a terminal state.

## Pipeline

Before the browser creates its App, `entities::world_preload::discover` resolves
the root and its direct static `extra_worlds`, lifts their exact script sources,
and records the shared script-source digest. Literal spawn templates enter the
existing template/include queue. Resident fragments promoted to templates are
resolved again as roots. The world-fetch callback is registered before discovery;
active pack content takes precedence over fetched base bytes. All required
sources and composed templates must be ready before local-slot/import version
checks or `wasm_init`. Missing required content retains a path-specific terminal
refusal; requests time out after 30 seconds. This content gate precedes the
renderer asset gate below. Computed runtime spawn paths retain their existing
post-freeze diagnostics.

1. `discover_base_assets` walks `WorldConfig`, referenced entity templates, script template paths, asteroid type lists, PFX defaults, and sub-world declarations.
2. `begin_asset_preload` starts Bevy loads for GLBs/images and asks the host page to fetch plain-TOML model sidecars and sub-worlds.
3. `poll_asset_preload` incorporates delivered sidecars, expands their authored LOD ladders, ingests sub-world TOML, and recomputes ready/total counts as the manifest grows.
4. `process_lobby` either starts immediately when complete or enters `Loading`; loading progress is broadcast until `auto_transition_from_loading` moves to `InProgress`.

Headless/minimal fixtures may omit `AssetPreloadResource`; that absence is an intentional pass-through rather than a renderer dependency.

## Sidecar cache

Model rig sidecars are fetched as text by `server.html` and placed in the thread-local cache in `src/entities/config_cache.rs`. Reads are persistent and multi-consumer: preload can observe delivery while entities using the same model later resolve the same body. An empty delivered body is terminal absence, preventing repeated requests.

`model_markers::sync_authoritative_model_markers` attaches simulation-owned marker geometry in PreUpdate and FixedLast. Within one invocation it resolves each candidate sidecar path once and copies that geometry to entities in their existing order, retaining their own transforms. The temporary reuse ends on return, so later spawns observe changed content and pending WASM deliveries are retried. It does not defer marker availability or cache across World loads.

LOD discovery is deliberately two-phase because the base entity template names one model while the sidecar names the rest of the ladder.

## Failure semantics

Bevy `Loaded` and `Failed` states are both terminal for the preload gate. A failed GLB is warned once and the owning entity remains authoritative without a mesh; `render_spawned_entities` in `src/server_app_render.rs` marks it processed so it is not retried every frame. One bad visual therefore cannot deadlock the scenario.

## LoadingProgress wire shape

`ServerMessage` uses `#[serde(tag = "type", content = "data")]`, so progress is encoded as:

```json
{"type":"LoadingProgress","data":{"fraction":0.5}}
```

The host and client pages read `data.fraction`. Codec and smoke tests pin the exact shape and an observable intermediate value.

## Related

- [Game Phases](./game-phases.md)
- [Model Viewer](./model-viewer.md)
- [LOD Generation](./lod-generation.md)
- [Build & Deployment](./build-and-deployment.md)
