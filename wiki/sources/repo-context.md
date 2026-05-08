---
title: CONTEXT.md (project root)
type: source
tags: [context, vocabulary, glossary, domain]
source_path: CONTEXT.md
status: shipped
updated: 2026-05-08
---

# CONTEXT.md

The canonical glossary. **Use these exact terms in code, comments, PRs, and architecture discussions.**

## Game-domain terms defined

- **Console** — a role on the ship (one seat per console, immediate vacancy on disconnect).
- **Session** — server-side player record, keyed by token, survives reconnects.
- **Session Token** — UUIDv4 in `localStorage`. Persistent identity across refreshes.
- **Lobby Phase** / **In-Progress Phase** — the two game phases.
- **Captain** — the player at `CaptainChair`. Authority for `StartGame`, `ToggleRedAlert`, `SetView`.
- **Helm Input** — `{ thrust: f32, steering: f32 }` at 10 Hz from the Helm console.
- **Red Alert** — a ship-wide state, captain-toggled, visualised as a red vignette/border.
- **View Mode** — server-side viewscreen content (`Camera(direction)`, `Radar`).
- **Radar** — overhead mini-map (asteroid positions ship-relative).
- **World Data** — fixed asteroid layout per session, deterministic.

## Architecture terms defined

- **LobbyHandlerResult** — return type of pure lobby handler functions: `new_phase: Option<GamePhase>`, `outbound: Vec<(Target, ServerMessage)>`.
- **radar_dots** — the shared pure iterator in `radar.rs` projecting asteroids onto the radar plane.
- **Console Plugin** — a Bevy plugin owning all UI / markers / setup / handlers for one console.
- **View-Model** — pure derived snapshot a renderer reads instead of raw session/simulation state. (Client: `LobbyView`. Server: `GameState` resource.)

## Why this is load-bearing

Naming drift is the silent killer of long-lived projects. If "session" and "player" become interchangeable, every code review and PRD review loses time disambiguating. The wiki adopts these names as primary headings — every entity/concept page maps to a `CONTEXT.md` term where one exists.

## Cross-references

- Every entity page; every concept page.
