---
title: Session
type: entity
tags: [session, server, identity, reconnect]
sources: [src/server/session.rs, PRD-001]
updated: 2026-05-08
---

# Session

The server-side record of a [Player](./player.md). Lives in `SessionManager` (`src/server/session.rs`). Survives disconnects and reconnects.

## Why sessions, not peer IDs

PeerJS assigns a random peer ID per `new Peer()`. That changes every refresh, every transient drop. A player who refreshes their phone would otherwise lose their console.

The session token (UUIDv4 in `localStorage`) gives stable identity. The JS shell in `server.html` resolves *peer ID → session token* on the first `Identify` message of every connection.

## What `SessionManager` owns

- The set of known players (by token).
- Console assignments — at most one player per console.
- Connection status.
- Helpers used by the simulation: `helm_token()`, `captain_token()`.

## Vacancy and reconnect rules

- **On disconnect:** the player's console becomes immediately vacant, in *any* game phase.
- **On reconnect with a known token:**
  - If the previous console is still free → auto-reassign it.
  - If it's been claimed by someone else → player rejoins consoleless and re-picks.
- **Console picks during In-Progress phase** are allowed (late joiners can take any free console).

## Test surface

`src/server/session.rs` is one of the most-tested modules in the repo: registration, duplicates, console assignment/clearing, vacancy, auto-reconnect, conflict resolution, captain/helm lookup. See `#[cfg(test)] mod tests` at the bottom of the file.

## Related

- [Player](./player.md) — the wire-side counterpart
- [Console](./console.md) — what a Session may hold
- [Game Phases](../concepts/game-phases.md)
