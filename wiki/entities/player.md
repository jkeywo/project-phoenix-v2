---
title: Player
type: entity
tags: [player, session, identity]
sources: [src/shared/messages.rs, src/server/session.rs, PRD-001]
updated: 2026-05-08
---

# Player

A connected human at the table. From the server's perspective, a `Player` is a record keyed by **session token**, not by PeerJS peer ID.

## Wire shape

Defined in `src/shared/messages.rs:61`:

```rust
pub struct Player {
    pub token: String,        // UUIDv4, persisted in localStorage
    pub name: String,         // editable, defaults to a random space-themed name
    pub consoles: Vec<Console>,
    pub connected: bool,
}
```

## Lifecycle

1. First visit: client generates a UUIDv4 → stores in `localStorage` → sends `Identify { token, name }`.
2. Server creates a new [Session](./session.md) and broadcasts `PlayerJoined`.
3. Player picks a [Console](./console.md) → server validates and broadcasts `ConsoleSelected`.
4. On disconnect: console becomes vacant immediately (in any phase).
5. On reconnect with the same token: previous console is auto-restored if still free.

## Identity vs presence

- **Identity** lives in `localStorage` on the device. Same browser, same player, even after refresh.
- **Presence** is the `connected: bool` on the server-side record.

The PeerJS peer ID is **ephemeral** — it changes every reconnect. The session token is the stable identity. See [Networking](../concepts/networking.md).

## Related

- [Session](./session.md) — server-side state for a Player
- [Console](./console.md) — what a Player can occupy
- [PRD #1](../sources/prd-001-bridge-simulator.md) — original identity model
