---
title: PRD #116 — Save/Load Game Sessions
type: source
tags: [prd, save, load, persistence, localstorage, serde, planned]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/116
status: open
updated: 2026-05-11
---

# PRD #116 — Save/Load Game Sessions

Persist a full in-progress session to browser `localStorage` so the host can close the tab, return later, and resume exactly where they left off.

## Status

Open. No code yet.

## Problem

All game state lives in the host tab's memory. Closing or refreshing the tab destroys ship position, hull damage, destroyed asteroids, repair state, and weapons state. There is no way to pause a session.

## Solution

A pre-WASM HTML/JS selection screen on `server.html` lists all `phoenix_save_<uuid>` keys in `localStorage` with metadata (created/saved timestamps, player names, version). The host picks New Game, Resume, or Delete. WASM compiles in the background while the screen is visible. On Resume, the JSON is passed into Rust via a new `wasm_load_save` export (same thread-local pattern as `wasm_load_map`) and pre-populates Bevy resources during `App::build()`. Saves fire on `InProgress` transition, every 30 s, and best-effort on `beforeunload`/`visibilitychange`.

## Key decisions

- **`save.rs` is the second sanctioned `serde_json` surface.** Documented in `AGENTS.md`. Owns `SaveState`, `SaveMeta`, all sub-structs. No Bevy. All fields use `#[serde(default)]` for forward compatibility.
- **One slot per game.** Overwritten on each save. `SaveMeta { version: u32, slot_uuid, created_at, saved_at, player_names }`.
- **Version starts at 1.** Mismatches show greyed-out, Resume-disabled, Delete-enabled.
- **`CurrentSaveSlot(String)` Bevy resource** holds the active UUID. Generated on New Game; read from `SaveMeta.slot_uuid` on Resume.
- **Lazy asteroid HP restore.** `PendingHpRestores(HashMap<uuid, f32>)` resource drained as ECS asteroids spawn. Despawned-then-respawned asteroids reset to `config.max_hp` — out-of-range asteroids are not preserved.
- **No lobby save.** Saves only fire on/after `InProgress`. Lobby is ephemeral; players re-identify on reconnect.
- **Captures full fidelity:** ship pose/hull/breakdown queue/cumulative damage/active repair timer/per-player penalty timers/weapons lock/active beam/phaser cooldown/collision cooldown/last helm input/surviving asteroid list with HP/player names.

## Schema additions (planned)

- New module: `save.rs`.
- New Bevy plugin (server feature) for save trigger + restore.
- New wasm-bindgen exports: `wasm_load_save(json)`, `set_save_callback(fn)`.
- `server.html` selection screen + `beforeunload`/`visibilitychange` handlers.
- `AGENTS.md` documents `save.rs` as the second `serde_json` surface.

## Out of scope

- Scenario-file-triggered saves (deferred to PRD #119 follow-up).
- Save migration between versions.
- Backend / server-side storage.
- Export/import of save files.
- Named/labelled slots.
- Multiple slots per game session.
- Multi-tab session locking.
- Client-side persistence beyond existing token + name.

## Cross-references

- [Codec Seam](../concepts/codec-seam.md) — the constraint this PRD explicitly relaxes (one extra surface).
- [Architecture](../concepts/architecture.md)
- [Roadmap Overview](../roadmap/overview.md)
