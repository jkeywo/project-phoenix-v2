---
title: Networking
type: concept
tags: [networking, peerjs, webrtc, session-token, star-topology, datachannel, snapshot]
sources: [server.html, client.html, gui/connection-manager.js, AGENTS.md, PRD-001, PRD-017]
updated: 2026-07-05
---

# Networking

Phoenix uses **PeerJS** (WebRTC + a public signalling broker) in a **star topology** with **two DataChannels** per client.

## Topology

- Server page (`server.html`) creates a PeerJS host peer with a random UUID. The QR code on the view screen encodes `https://…/client/index.html#<peerId>`.
- Client pages read the peer ID from `location.hash` and call `clientPeer.connect(hostPeerId, { reliable: true })`.
- **Clients never talk to each other.** All messages flow through the host.

```
client #1 ──┐
            ├──▶  host peer (server.html WASM)  ──snapshot──▶  all clients
client #2 ──┤                        │               │
client #3 ──┘                        │               └── unordered DataChannel
                                     │                   (SimState, etc.)
                                     └── reliable DataChannel
                                         (commands, lobby messages)
```

## Identity model

Two distinct identifiers:

| | Lifetime | Used for |
|---|---|---|
| **PeerJS peer ID** | Per `new Peer()` instance — changes every page load | WebRTC routing only |
| **Session token** (UUID-like 32 hex chars, resolved by `gui/session-token.js`) | Per browser tab during play; reload-safe for that tab; persistent `localStorage` token is only adopted when not already live elsewhere | Server-side player identity |

`client.html` resolves the local player token just before sending `Identify`. The helper stores the active tab token in `sessionStorage`, uses a short-TTL `localStorage` heartbeat registry to detect other live tabs, and mints a fresh token when a duplicated tab inherits a token already in use. This keeps multiple clients on one computer from collapsing into one server-side player while preserving normal reload reconnects.

The JS shell on `server.html` keeps three maps:

| Map | Key → Value | Purpose |
|---|---|---|
| `peerTokens` | `peerId → sessionToken` | Populated on first `Identify` message |
| `tokenConns` | `sessionToken → DataConnection` (reliable) | Routing `Target::Token` messages |
| `tokenSnapshotConns` | `sessionToken → DataChannel` (snapshot) | Routing snapshot-class messages |

The snapshot channel is created on the client side (`gui/connection-manager.js`) during the PeerJS `open` handshake: after the reliable DataConnection opens, it calls `pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 })`. The server registers it via `pc.ondatachannel` with `label === 'snapshot'`. When unavailable, snapshot messages fall back to the reliable channel.

## Dual DataChannel architecture

Every client maintains two parallel DataChannels on the same `RTCPeerConnection`:

| Channel | Label | Ordered | Retransmit | Carries |
|---|---|---|---|---|
| Reliable | (PeerJS default) | Yes | Yes | `ClientMessage` commands, `ServerMessage` lobby/events |
| Snapshot | `'snapshot'` | No | 0 (never) | `SimState`, `BlackboardUpdate`, `ShieldStatus`, `RepairState`, `PowerState`, `WeaponsUpdate`, `SystemHullUpdate` |

The server classifies each `ServerMessage` via `delivery_class_for_msg()` (`src/server_app.rs:664`). `flush_outbound` (`src/server/bridge.rs:836`) passes the delivery class as a third string argument to the JS callback. `routeOutbound` (`server.html:1510`) dispatches to the appropriate channel map, falling back to the reliable channel for any token without a snapshot channel.

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

The Playwright suite swaps `window.Peer` for a `BroadcastChannel`-backed shim before any page script runs (`addInitScript`). The shim (`tests/smoke/peerjs-shim.js`) includes a `DataChannel` implementation that propagates sub-channel creation via control messages, so the snapshot DataChannel works end-to-end in smoke tests. Same surface, no signalling server. See [Testing Strategy](./testing-strategy.md), [PRD #51](../sources/prd-051-smoke-test-harness.md), and `tests/smoke/snapshot-channel.spec.ts`.

## Related

- [Architecture](./architecture.md) · [Message Flow](./message-flow.md)
- [Player](../entities/player.md) · [Session](../entities/session.md)
