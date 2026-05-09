---
title: CONTEXT.md (project root)
type: source
tags: [context, vocabulary, glossary, domain]
source_path: CONTEXT.md
status: shipped
updated: 2026-05-09
---

# CONTEXT.md

The canonical glossary. **Use these exact terms in code, comments, PRs, and architecture discussions.**

## Game-domain terms defined

- **Console** — a role on the ship (one seat per console, immediate vacancy on disconnect). Now four: `CaptainChair`, `Helm`, `Tactical`, `Engineering`. A player may hold multiple.
- **Session** — server-side player record, keyed by token, survives reconnects.
- **Session Token** — UUIDv4 in `localStorage`. Persistent identity across refreshes.
- **Lobby Phase** / **In-Progress Phase** — the two game phases.
- **Captain** — the player at `CaptainChair`. Authority for `StartGame`, `ToggleRedAlert`, `SetView`.
- **Helm Input** — `{ thrust: f32, steering: f32 }` at 10 Hz from the Helm console.
- **Red Alert** — a ship-wide state, captain-toggled, visualised as a red vignette/border.
- **Hull Integrity** — ship HP pool [0–100]. Reduced by collision, restored by successful repair.
- **Breakdown** — a console-repair assignment triggered by hull damage (one per 10 HP cumulative). Held in `BreakdownQueue`; front entry is `authorized_repair_console`.
- **Repair** — action clearing the front breakdown. Only the `authorized_repair_console` may repair without penalty.
- **View Mode** — server-side viewscreen content (`Camera(direction)`, `Radar`).
- **Radar** — overhead mini-map (asteroid positions ship-relative).
- **World Data** — fixed asteroid layout per session, deterministic.

## Architecture terms defined

- **LobbyHandlerResult** — return type of `process_message()` and `process_disconnect()` in `lobby_handler.rs`: `new_phase: Option<GamePhase>`, `outbound: Vec<(Target, ServerMessage)>`. `Target` = `All | Token | AllExcept`.
- **radar_dots** — the shared pure iterator in `radar.rs` projecting asteroids onto the radar plane.
- **Console Plugin** — on the client, all console UI panels live in `client_app.rs`; panel visibility is toggled per phase + console + `ActiveConsole`.
- **View-Model** — pure derived snapshot a renderer reads. Client lobby: `LobbyView` (from `client_lobby.rs`). Client in-game: `ClientSimState` (from `client_sim.rs`). Server: `GameState` resource.
- **ClientSimState** — client-side mirror of `SimSnapshot` plus repair state. Updated by `ServerMessage` in `client_sim.rs`.
- **ActiveConsole** — Bevy `Resource` tracking which console panel the client is viewing (set by JS tab bar via `wasm_client_set_active_console`).

## Why this is load-bearing

Naming drift is the silent killer of long-lived projects. If "session" and "player" become interchangeable, every code review and PRD review loses time disambiguating. The wiki adopts these names as primary headings — every entity/concept page maps to a `CONTEXT.md` term where one exists.

## Cross-references

- Every entity page; every concept page.
