---
title: Asset Preload
type: concept
tags: [assets, gltf, sidecar, preload, lobby, loading-phase, race-condition]
sources: [
  src/server/asset_preload.rs,
  src/server_app.rs,
  src/entities/config_cache.rs,
  src/entities/model_rig.rs,
  src/lobby/server.rs,
  server.html,
]
updated: 2026-06-17
---

# Asset Preload

Server-side discovery + pre-cache of every renderable asset (GLB scenes,
radar icon PNGs, model-rig sidecar TOMLs, sub-world TOMLs) referenced by the
loaded scenario. Runs during the [Lobby phase](./game-phases.md) so models
are already resident in `Assets<Scene>` / `Assets<Image>` by the time the
captain presses Engage.

Implemented in `src/server/asset_preload.rs:1`. Registered under the `server`
feature in `src/server_app.rs:239-257`.

## Pipeline

1. **Discover** (`asset_preload.rs:210` `discover_base_assets`) — walks the
   loaded `WorldConfig` and every entity template it references (via
   `[asteroid_field].asteroid_type_paths`, `[asteroid_field].cosmetic_type_paths`,
   `[[trigger]]` `LoadWorld`/`SpawnEntity` actions, and `[[comms]]` response
   actions). Builds an `AssetManifest` of unique GLB / icon / sidecar /
   sub-world paths.
2. **Start loading** (`asset_preload.rs:347` `begin_asset_preload`) — calls
   `asset_server.load(path)` for every GLB and icon, fires
   `request_sidecar_fetch(path)` for every sidecar (WASM only — JS resolves
   via `set_world_fetch_callback` in `server.html:1384-1405`), and stores the
   resulting handles in `AssetPreloadResource.glb_handles` / `.icon_handles`
   so the asset server keeps them alive.
3. **Poll** (`asset_preload.rs:494` `poll_asset_preload`) — runs every
   `Update`. Peeks the sidecar inbox, checks GLB `LoadState`s, ingests any
   sub-world TOMLs JS has delivered, and recomputes `total_count` /
   `ready_count` / `complete`.
4. **Gate** (`lobby/server.rs:283-296` `process_lobby`) — `preload_complete`
   is forwarded into the lobby handler; `StartGame` only transitions to
   `InProgress` once preload reports done. While preload is still running
   the captain's Engage transitions to `GamePhase::Loading` instead, and
   `broadcast_loading_progress` (`asset_preload.rs:649`) pushes
   `LoadingProgress { fraction }` to clients at ~2 Hz until
   `auto_transition_from_loading` (`asset_preload.rs:671`) flips to
   `InProgress`.

## The sidecar inbox contract

Model-rig sidecars (`.model.toml` files alongside each `.glb`, e.g.
`assets/models/dynasty_destroyer.model.toml`) are NOT loaded through the
Bevy `AssetServer` — they're plain TOML the renderer parses synchronously
into a `ModelRig` (`src/entities/model_rig.rs:1`). The JS fetch callback in
`server.html:1395-1404` routes paths matching `^assets/models/.+\.toml$` to
`wasm_push_sidecar_toml(path, body)`, which stuffs the body into a
process-wide thread-local map (`PENDING_SIDECAR_TOML` in
`src/entities/config_cache.rs:104-115`).

The inbox has **one consumer**: the renderer
(`server_app::load_sidecar_toml` at `src/server_app.rs:1554-1568`). It uses
`take_pending_sidecar_toml(path)` (`src/entities/config_cache.rs:407-420`),
which `remove()`s the entry — destructive by design, so the same body cannot
be consumed twice.

Any other caller that needs to know whether the sidecar has arrived must use
`is_pending_sidecar_delivered(path)` (`src/entities/config_cache.rs:422-431`),
which is a non-destructive `contains_key()` check. The preload poller uses
this at `src/server/asset_preload.rs:507-517`.

### Why the contract matters — the prefetch race (2026-06-17 fix)

Before this contract was made explicit, both the renderer's
`load_sidecar_toml` and the preload poller called the same destructive
`pop_pending_sidecar_toml`. Both raced for the same `HashMap` entry. The
poller usually won (it runs every `Update`, the renderer only runs while
walking `Without<RenderProcessed>` entities). The poller silently discarded
the TOML body. The renderer's subsequent `take` returned `None`. The
renderer then refired `request_sidecar_fetch`, which was deduped against
`SIDECAR_FETCH_REQUESTED` (`config_cache.rs:441-451`) — the original fetch
had already completed, so no re-fetch ever happened. The renderer waited
forever, `continue`'d each frame at `server_app.rs:1707-1712`, and never
spawned the `SceneRoot`. The entity still existed with its collider,
transform, weapons, etc. — it just had no visible mesh.

The race only affected entities whose sidecars happened to arrive on a
frame where the poller ran before the renderer; entities lucky enough to
have the renderer run first appeared correctly. Hence the partial,
intermittent "some models present, some missing" symptom.

The fix: split `pop_pending_sidecar_toml` into two functions —
`take_pending_sidecar_toml` (destructive, renderer-only) and
`is_pending_sidecar_delivered` (peek, anyone). The poller switched to the
peek API. Backed by a Rust unit test
(`src/entities/config_cache.rs:953-1015`) that simulates push → peek (×2)
→ take and asserts the renderer still sees the body.

## GLB readiness and `LoadState::Failed`

Each GLB `Handle<Scene>` is queried via `AssetServer::load_state`
(`asset_preload.rs:610-630`). `Loaded` and `Failed` are both terminal;
`Failed` additionally logs a single warn! per asset path and counts as
"ready" so an authoring error in one model cannot deadlock the preload
gate. Other entities continue to load, and the offending entity will
render without a mesh (the renderer also handles `LoadState::Failed` at
`src/server_app.rs:1698-1709`, inserting `RenderProcessed` so the entity
isn't revisited every frame).

GLBs were previously excluded from `total_count` (commits `2aee8ff` and
`0afa818`) to avoid stalling the gate on long-running CI parses. With the
new `Failed`-as-terminal handling that risk is gone, so they're back in
the gate (`asset_preload.rs:455-465`). This makes the captain's Engage
properly wait until models are visible-ready, eliminating the "models pop
in mid-game" UX issue.

## Why the lobby gate was bypassed (and why it's safe to re-enable)

`src/lobby/server.rs:275-296` used to hardcode `preload_complete = true`
with a comment citing "async completion bugs in CI (icon/sidecar fetch
timing)". The underlying bug was the sidecar race: the gate would never
flip because the poller had drained the inbox into the void, leaving
`pending_sidecars` permanently non-empty. With the inbox contract above
the gate now flips reliably and the workaround can be (and has been)
removed.

The gate treats two non-`complete` cases as still-pass-through:

1. **Resource missing** — `process_lobby` doesn't require
   `AssetPreloadResource`. Native unit tests and headless harnesses that
   don't register the server plugin see no resource and proceed.
2. **`!started`** — the preload system runs every `Update` but may not have
   observed the lobby state yet on the first frame, especially under
   `init_state()` which doesn't fire `OnEnter`. Treating "not yet started"
   as "complete" avoids a one-frame deadlock the first time the lobby opens.

## Diagnostic logging

At the renderer site (`src/server_app.rs:1684-1697`):

- `info!` on first request per GLB: `render_spawned_entities: requesting
  scene <path> (load_state=<LoadState>)`. Distinguishes prefetch hits
  (`LoadState::Loaded` immediately) from cold loads (`NotLoaded` then
  `Loading`).
- `warn!` on `LoadState::Failed`: `render_spawned_entities: GLB failed to
  load for entity <Entity>, path=<path> — entity will exist without a mesh`.

In `poll_asset_preload`:

- `info!` once when all gates clear:
  `asset_preload: all assets ready (icons=true, sidecars_done=true,
  sub_worlds_done=true, GLBs=<N>+<F> failed)`.
- `warn!` once per `Failed` GLB: `asset_preload: GLB failed to load: <path>
  — entities referencing this model will render without a mesh`.

## Cross-references

- [Game Phases](./game-phases.md) — `Lobby` / `Loading` / `InProgress` /
  `GameOver`. Preload sits between the first two.
- [Server HTML Lobby UI](./server-lobby-ui.md) — the lobby panel that
  surfaces the preload progress bar.
- [WorldPlugin](./world-plugin.md) — owner of `WorldConfig`, the input to
  preload discovery.
- [Build & Deployment](./build-and-deployment.md) — Trunk's `copy-dir`
  directives that place `assets/` under `dist/` so the JS fetches resolve.

## Open questions

- Should the prefetch hold a strong handle for the sub-world TOMLs the way
  it does for GLBs/icons? Today they're "fire-and-forget" via the JS
  callback (`src/entities/config_cache.rs:431-441` `request_world_fetch`).
  If JS forgets to push back, the world entry sits in `pending_sub_worlds`
  forever. Not currently observed but a similar single-consumer concern
  applies.
- `SIDECAR_FETCH_REQUESTED` is a permanent dedupe set — a failed fetch
  (browser dropped the request, JS error) cannot be retried. Probably
  acceptable because sidecars fall back to identity rigs on absence, but
  worth revisiting if a real fetch-retry scenario emerges.
