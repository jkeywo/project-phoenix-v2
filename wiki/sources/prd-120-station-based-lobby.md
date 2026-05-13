---
title: PRD #120 — Station-Based Lobby & Crew Assignment
type: source
tags: [prd, lobby, station, crew, spectator, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/120
status: shipped
updated: 2026-05-13
---

# PRD #120 — Station-Based Lobby & Crew Assignment

Replace per-console picking with per-station picking. A station is one player's role bundling one or more consoles. Stations are defined per player count in `player_ship.toml`; players auto-shuffle as crew joins or leaves; overflow becomes spectators in a FIFO queue.

## Status

Shipped (2026-05-11). `src/stations.rs` carries the pure model; `SelectStation` / `ReleaseStation` / `StationAssigned` are on the wire; `[stations]` blocks live in `assets/entities/player_ship.toml`.

## Problem

1. **Player-count fragility.** Four hardcoded consoles; a 2-player session leaves two unmanned with no notion of "what should two players be doing."
2. **No graceful degradation.** A drop mid-game leaves a ghost console.
3. **Designer intent is invisible.** "Solo player covers everything; at 2 players one pilots and one fights" lives only in a designer's head.

## Solution

Each ship config gains a `[stations]` section listing, per player count, named stations and the consoles each station bundles. Each station declares explicit `next` (what this player becomes when one more joins) and `previous` (where they go when one leaves). Joining or leaving runs a deterministic reassignment cascade so all stations at the new count are filled. Overflow joiners go to a spectator FIFO queue and are auto-promoted when a slot opens.

The captain is whoever holds the station containing the `CaptainChair` console — at any player count.

## Key decisions

- **New pure module: `stations.rs`.** `parse_and_validate(toml) -> Result<ShipStations, StationConfigError>`, `reassign_on_join`, `reassign_on_leave`, `get_station`, `all_stations_filled`.
- **Validation refuses to boot** on dangling `next`/`previous`, duplicate names within a count, unknown console, empty consoles list, count outside `min/max_players`. Failure surfaces as a fatal overlay on `server.html`.
- **Implicit `next`/`previous` by name.** Omit them when the station name persists across the boundary; declare them explicitly only when the role changes name.
- **Cascade semantics.** *Join N→N+1:* everyone follows their `next`; the new player takes the station at N+1 with no `previous`. *Leave N→N-1:* the no-previous station first claims the leaver's slot, then everyone follows their `previous`. *Spectator pull:* if the cascade leaves a bottom-of-chain station unfilled, the front of the spectator queue fills it.
- **Reconnects = fresh joins.** No preferred-station memory. Same-token reconnects go to the back of the spectator queue.
- **Mid-game vs. lobby:** identical reassignment logic. Mid-game players cannot voluntarily `SelectStation` — they only move via auto-reassignment.
- **Captain identity is dynamic.** Whoever's station bundles `CaptainChair`. `StartGame` validates: sender holds `CaptainChair` AND all stations at the current count are filled.
- **Spectator queue** lives on `SessionManager`. Survives Lobby → InProgress. Position not exposed to clients (just "(spectating)").
- **Active-tab reconciliation** is pure on the client: keep current console if still in new bundle, else jump to `consoles[0]` (TOML order = designer intent).

## Schema additions (planned)

- New module: `stations.rs`.
- New `ServerMessage::StationAssigned { token, station: Option<String>, consoles: Vec<Console> }`.
- New `ClientMessage::SelectStation { station }` and `ReleaseStation`.
- **Removed:** `ConsoleSelected`, `ConsoleCleared`, `SelectConsole`, `ClearConsole` (full replacement).
- Extended `ServerMessage::Welcome` with `ship_stations` field.
- `player_ship.toml` gains `[stations]` section with `min_players`, `max_players`, and `[[stations.config]]` per count.

## Out of scope

- Audio / animation polish for mid-game reassignments.
- Spectator queue position display.
- Multiple ships (schema supports it; only `player_ship.toml` exists).
- Live-edit station configs.
- Custom session-level overrides.
- I18n.
- Player-to-player station swap requests.
- Reconnect stickiness.
- Telemetry dashboards.

## Cross-references

- [Console](../entities/console.md) — replaced as the unit of selection by Station here.
- [Session](../entities/session.md) — extended with spectator queue.
- [Lobby Phase](../concepts/game-phases.md) · [Game Loop](../concepts/game-loop.md)
- [Roadmap Overview](../roadmap/overview.md)
