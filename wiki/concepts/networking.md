---
title: Networking
type: concept
tags: [networking, peerjs, webrtc, session-token, star-topology, datachannel, snapshot]
sources: [server.html, client.html, gui/connection-manager.js, gui/session-token.js, src/core/broadcast/sim.rs, src/core/broadcast/lifecycle.rs, src/server/bridge.rs, src/server_app/components.rs, src/server_app/broadcast_publish.rs, src/console/repair/visibility.rs, src/ship/shields.rs, AGENTS.md]
updated: 2026-08-28
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

The snapshot channel is created on the client side (`gui/connection-manager.js`) during the PeerJS `open` handshake: after the reliable DataConnection opens, it calls `pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 })` immediately before sending `Identify`. Channel-open and Identify travel independently, so the server retains the `pc.ondatachannel` candidate and promotes it into `tokenSnapshotConns` only when both the channel is open and the session token is known, in either order. When unavailable, snapshot messages fall back to the reliable channel.

## Dual DataChannel architecture

Every client maintains two parallel DataChannels on the same `RTCPeerConnection`:

| Channel | Label | Ordered | Retransmit | Carries |
|---|---|---|---|---|
| Reliable | (PeerJS default) | Yes | Yes | `ClientMessage` commands, `ServerMessage` lobby/events |
| Snapshot | `'snapshot'` | No | 0 (never) | `SimState`, `BlackboardUpdate`, `ShieldStatus`, `RepairState`, `PowerState`, `WeaponsUpdate`, `SystemHullUpdate` |

Delivery class is producer-owned. Registered broadcaster phases stamp their
own class; arbitrary-target simulation producers call the explicit
`SimOutbox::push_snapshot` / `push_reliable` seam. The raw queue is private,
and its forwarding drain preserves the chosen class without matching on
`ServerMessage`. `flush_outbound` in `src/server/bridge.rs` passes the delivery
class as a third string argument to the JS callback. `routeOutbound` in
`server.html` dispatches to the appropriate channel map, falling back to the
reliable channel for any token without a snapshot channel.

## Client connection lifecycle (`gui/connection-manager.js`)

`ConnectionManager` owns PeerJS peer creation, connect/retry/timeout,
DataConnection handlers, and `Identify` on every open. `client.html` wires its
callback interface: `onData`, `onStatus`, `onLog`, `onError`, and `getIdent` for
the session token. `tests/client/connection-manager.test.js` covers the pure
module logic.

Reconnect uses persistent exponential backoff from 100 ms to a 30 s cap.
`_identSent` resets on DataChannel close/error so every reopen re-identifies and
enters the server-side seat/rating restore flow. During a round, registered
replication owners reconstruct current Snapshot state in stable key order for
that token only; the Blackboard and Hull adapters reproduce live recipient visibility
without resetting shared caches. The client offers **Retry now**, and a
generation guard ignores callbacks from superseded attempts.
`reconnect-midgame-sever.spec.js` exercises sever and revive during play.

## Host page cues via HUD/lobby push

Host-local presentation cues use dedicated push callbacks rather than inspecting
peer message JSON in `routeOutbound`:
- `__updateHud` — carries `ViewscreenHudState` with `engine_thrust` (engine hum volume), `red_alert` (siren), etc.
- `__updateLobby` — carries `LobbyStatePayload` with `phase` (loading overlay visibility) and `loading_progress` (progress bar value).

`routeOutbound` is now a pure forwarder — it inspects no JSON payloads, only dispatches by `Target` + `DeliveryClass`.

## Connection state machine (PRD #17)

Both pages show a coloured dot in the top-right:

| State | Dot | Action |
|---|---|---|
| `connecting` | green (no label) | Initial `new Peer(...)` |
| `ready` | green (no label) | `peer.on('open')` fired |
| `disconnected` | red + "Disconnected — reconnecting…" | `peer.on('disconnected')`; auto-call `peer.reconnect()` |
| `error` | red + "Error — refresh to retry" | `peer.on('error')` or `'close'` (destroyed) |

`peer.reconnect()` reuses the original peer ID, which avoids the PeerJS broker rate-limiting repeat registrations. A fully destroyed peer needs a manual page refresh.

## ICE servers, TURN relay, and on-device diagnostics (2026-08 hotspot fix)

The base ICE list (`defaultIceServers()`) is **STUN-only**; TURN relay credentials come primarily from the Cloudflare worker (`worker/`, deployed per-target as `phoenix-turn-credentials` / `phoenix-turn-credentials-demo`). If the worker is unreachable, `fetchIceServers()` falls back to Metered's shared OpenRelay TURN (`openRelayFallbackServers()`) and reports `relaySource: 'openrelay'`, which both pages surface as a mild fallback notice. OpenRelay uses `staticauth.openrelay.metered.ca`.

**Relay is mandatory on hotspot/CGNAT networks** (phone tethering, two phones on separate mobile data): STUN hairpinning through carrier NAT essentially never works, and host candidates are mDNS-obfuscated. Three things therefore surface degraded relay instead of failing silently:

- `fetchIceServers()` returns `{ servers, relayAvailable, relaySource }`; both pages show a warning (`client.diag_no_relay` / `server.no_relay_warning`) when `relayAvailable` is false, and a milder notice (`client.diag_relay_fallback` / `server.relay_fallback_notice`) when running on the shared OpenRelay fallback.
- The client join screen shows a live diagnostics readout (`#conn-diag`): relay probe verdict (`probeTurnRelay()`, an `iceTransportPolicy:'relay'` throwaway connection) plus per-attempt ICE state and gathered candidate types. "candidates: host, srflx" with no relay on a failing network is the TURN smoking gun.
- The host lobby mirrors any inbound client stuck mid-ICE under the QR code (listeners attach at `peer.on('connection')` time, because `conn.on('open')` never fires on a failed connection).

The per-attempt connect timeout escalates 8s → 16s → 30s (`connectTimeoutMs()`) since TURN-over-TCP allocation on cellular can exceed the old flat 8s.

The worker validates CORS against the comma-separated `ALLOWED_ORIGIN` list in
`worker/wrangler.toml` and echoes the matching origin. The deployed value only
changes after `wrangler deploy`; an incorrect allowlist blocks credential
fetches and removes TURN availability for clients.

## Why no real backend

- Game data is peer-to-peer. Only the WebRTC handshake touches PeerJS's public broker.
- Hosting is GitHub Pages — pure static.

## Smoke testing without WebRTC

The Playwright suite swaps `window.Peer` for a `BroadcastChannel`-backed shim before any page script runs (`addInitScript`). The shim (`tests/smoke/peerjs-shim.js`) includes a `DataChannel` implementation that propagates sub-channel creation via control messages, so the snapshot DataChannel works end-to-end in smoke tests. Same surface, no signalling server. See [Testing Strategy](./testing-strategy.md) and `tests/smoke/snapshot-channel.spec.js`.

## Related

- [Architecture](./architecture.md) · [Message Flow](./message-flow.md)
- [Player](../entities/player.md) · [Session](../entities/session.md)
