---
title: Networking
type: concept
tags: [networking, peerjs, webrtc, session-token, star-topology]
sources: [server.html, client.html, AGENTS.md, PRD-001, PRD-017]
updated: 2026-06-27
---

# Networking

Phoenix uses **PeerJS** (WebRTC + a public signalling broker) in a **star topology**.

## Topology

- Server page (`server.html`) creates a PeerJS host peer with a random UUID. The QR code on the view screen encodes `https://…/client/index.html#<peerId>`.
- Client pages read the peer ID from `location.hash` and call `clientPeer.connect(hostPeerId, { reliable: true })`.
- **Clients never talk to each other.** All messages flow through the host.

```
client #1 ──┐
            ├──▶  host peer (server.html WASM)  ──broadcast──▶  all clients
client #2 ──┤
client #3 ──┘
```

## Identity model

Two distinct identifiers:

| | Lifetime | Used for |
|---|---|---|
| **PeerJS peer ID** | Per `new Peer()` instance — changes every page load | WebRTC routing only |
| **Session token** (UUID-like 32 hex chars, resolved by `gui/session-token.js`) | Per browser tab during play; reload-safe for that tab; persistent `localStorage` token is only adopted when not already live elsewhere | Server-side player identity |

`client.html` resolves the local player token just before sending `Identify`. The helper stores the active tab token in `sessionStorage`, uses a short-TTL `localStorage` heartbeat registry to detect other live tabs, and mints a fresh token when a duplicated tab inherits a token already in use. This keeps multiple clients on one computer from collapsing into one server-side player while preserving normal reload reconnects.

The JS shell on `server.html` keeps two maps:

- `peerTokens: Map<peerId, sessionToken>` — populated on the first `Identify` message of each connection.
- `tokenConns: Map<sessionToken, DataConnection>` — used to route outbound `Target::One(token)` messages.

## Connection state machine (PRD #17)

Both pages show a coloured dot in the top-right:

| State | Dot | Action |
|---|---|---|
| `connecting` | green (no label) | Initial `new Peer(...)` |
| `ready` | green (no label) | `peer.on('open')` fired |
| `disconnected` | red + "Disconnected — reconnecting…" | `peer.on('disconnected')`; auto-call `peer.reconnect()` |
| `error` | red + "Error — refresh to retry" | `peer.on('error')` or `'close'` (destroyed) |

`peer.reconnect()` reuses the original peer ID, which avoids the PeerJS broker rate-limiting repeat registrations. A fully destroyed peer needs a manual page refresh.

## Why no real backend

- Game data is peer-to-peer. Only the WebRTC handshake touches PeerJS's public broker.
- Hosting is GitHub Pages — pure static.
- Self-hosted PeerJS is an explicit out-of-scope item from PRD #1, deferred post-PoC.

## Smoke testing without WebRTC

The Playwright suite swaps `window.Peer` for a `BroadcastChannel`-backed shim before any page script runs (`addInitScript`). Same surface, no signalling server. See [Testing Strategy](./testing-strategy.md) and [PRD #51](../sources/prd-051-smoke-test-harness.md).

## Related

- [Architecture](./architecture.md) · [Message Flow](./message-flow.md)
- [Player](../entities/player.md) · [Session](../entities/session.md)
