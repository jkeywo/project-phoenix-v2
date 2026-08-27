---
title: Asset Preload
type: concept
tags: [assets, gltf, sidecar, preload, lobby, loading-phase]
sources: [src/server/asset_preload.rs, src/server/pfx.rs, src/server_app/registration.rs, src/server_app_render.rs, src/entities/config_cache.rs, src/entities/model_rig.rs, src/lobby/server.rs, src/core/messages.rs, server.html, client.html]
updated: 2026-08-27
---

# Asset Preload

The server discovers and pre-caches render assets referenced by the selected scenario while the session is still in Lobby/Loading. A game does not enter `InProgress` until the manifest reaches a terminal state.

## Pipeline

1. `discover_base_assets` walks `WorldConfig`, referenced entity templates, script template paths, asteroid type lists, PFX defaults, and sub-world declarations.
2. `begin_asset_preload` starts Bevy loads for GLBs/images and asks the host page to fetch plain-TOML model sidecars and sub-worlds.
3. `poll_asset_preload` incorporates delivered sidecars, expands their authored LOD ladders, ingests sub-world TOML, and recomputes ready/total counts as the manifest grows.
4. `process_lobby` either starts immediately when complete or enters `Loading`; loading progress is broadcast until `auto_transition_from_loading` moves to `InProgress`.

Headless/minimal fixtures may omit `AssetPreloadResource`; that absence is an intentional pass-through rather than a renderer dependency.

## Sidecar cache

Model rig sidecars are fetched as text by `server.html` and placed in the thread-local cache in `src/entities/config_cache.rs`. Reads are persistent and multi-consumer: preload can observe delivery while every entity using the same model later parses the same body. An empty delivered body is terminal absence, preventing repeated requests.

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
