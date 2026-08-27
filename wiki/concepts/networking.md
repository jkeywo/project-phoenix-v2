---
title: Networking
type: concept
tags: [networking, webrtc, rendezvous, join-code, session-token, star-topology, datachannel, snapshot]
sources: [server.html, client.html, gui/rendezvous-transport.js, gui/rendezvous-protocol.js, gui/join-code.js, gui/connection-manager.js, gui/host-peer-routing.js, worker-rendezvous/src/registry.js, worker-rendezvous/src/index.js, gui/session-token.js, src/core/broadcast/sim.rs, src/core/broadcast/lifecycle.rs, src/server/bridge.rs, src/server_app/components.rs, src/server_app/broadcast_publish.rs, src/console/repair/visibility.rs, src/console/weapons/blackboard.rs, src/delivery/mod.rs, AGENTS.md]
updated: 2026-08-28
---

# Networking

Phoenix uses the **Phoenix transport** in a **star topology** with **two DataChannels** per client: a Phoenix-owned rendezvous service carries typed join-code lookup and WebRTC signalling over a secure WebSocket, and the game traffic then runs over direct WebRTC DataChannels. Issue #1112 made this the only route — PeerJS, its public broker and the peer-id-in-the-URL-fragment mechanic are gone, along with the flag that used to choose between them.

## Topology

- The server page (`server.html`) registers with the rendezvous service on every load and is issued a **private five-letter code** in the client namespace. The join panel prints those five letters and a QR of `https://…/client/index.html#<PROJECT_GUID>_<VERSION_GUID>_<CODE>` — the same code, so scanning and typing are one join by two routes.
- Client pages either read that structured code out of `location.hash` or take five typed letters; `gui/join-code.js` decides which a given string is. Both resolve through the service to the same host record and open the same channels.
- **Clients never talk to each other.** All messages flow through the host.

```
client #1 ──┐
            ├──▶  host (server.html WASM)  ──snapshot──▶  all clients
client #2 ──┤                   │               │
client #3 ──┘                   │               └── unordered DataChannel
                                │                   (SimState, etc.)
                                └── reliable DataChannel
                                    (commands, lobby messages)
```

## Joining: code, signalling, handshake

- **Join identifiers** are `PROJECT_GUID_VERSION_GUID_CODE`. `gui/join-code.js` is the pure scheme — canonicalisation (upper-case; `0`→`O`, `1`/`L`→`I`, `J` distinct), five-letter validation, deny-list refusal, compose/parse and minting — reading its alphabet, GUIDs and deny-list from the authored `assets/join/join-codes.toml`, through `assets/join/join-codes.json`, a committed artifact `scripts/build-client.mjs` regenerates for the three consumers that have no TOML parser (the phone, vitest, the Worker bundle). Client and server joining have separate project GUIDs, so a fleet code typed into the crew field is a *wrong-type* answer rather than a miss.
- **The service** is `worker-rendezvous/`, a sibling Cloudflare Worker to the TURN one, with a Durable Object holding the live registry. All of its behaviour is the transport-free state machine in `worker-rendezvous/src/registry.js` (`/v1/host` and `/v1/join` WebSockets plus a `/v1/health` origin check); the Worker adapter decides nothing except who gets a socket, which is what lets `tests/client/rendezvous-registry.test.js` cover the protocol with no wrangler. Registry state is in memory only (a code dies with its host socket), and its bounds — lookup cap per socket, record TTL, peers per record, stored-field shapes — are authored in `[limits]`. Both upgrade endpoints require a present, allow-listed `Origin`; only `/v1/health` does not.
- **The browser halves** are `gui/rendezvous-transport.js`, one module holding both ends of one frame vocabulary. `createRendezvousHost` registers, is issued a code, and hands each ADMITTED connection to `server.html`'s `attachHostConn` — an open DataChannel is not admission, so nothing reaches the Identify gate before the compatibility verdict, and frames sent before it or after a refusal are dropped. `createRendezvousJoiner` resolves a typed code, offers, opens both channels, completes the handshake and sends `Identify`. `localiseTree` runs on every inbound frame, in one place, so no console has to know which of its fields are localisable.
- **The compatibility handshake** (`JoinHandshake` / `JoinAccepted` / `JoinRefused`) is transport-plane, not a `ClientMessage` — `pasm/spec/design/p2p-design-deltas.yaml` forbids layering transport concerns onto the crew protocol. The verdict comes from Rust: `wasm_check_client_stamp` → `delivery::check_join_stamp` → the same `check_client_stamp` the native host enforces over HTTP, so rendezvous version advice can never become the authority. The client's own stamp is written into `<meta name="phoenix-client-stamp">` by `scripts/build-client.mjs`. **Since #1112 a stamp is required**: absent and garbled are both refused as `client-stamp-missing`, because every client that can reach a Phoenix host is now a built Phoenix bundle.
- **Service selection.** `?rendezvous=<url>` points a page at another service (a local `wrangler dev`, a staging deployment); anything else, including no parameter, uses the built-in one. The #1111 opt-in spellings (`?rendezvous`, `=on`, `=off`) are ignored rather than honoured, so an old bookmark still opens the game instead of trying to dial a host called "on".
- **Deploy trap:** the same `ALLOWED_ORIGIN` drift as the TURN worker, except a stale value here means nobody can join at all rather than nobody getting relay. `/v1/health` echoes `origin_allowed` for exactly that check — see `docs/delivery-checklist.md` §3a. **The service is not deployed yet**, and since #1112 there is no second route underneath it.

## Identity model

Two distinct identifiers:

| | Lifetime | Used for |
|---|---|---|
| **Rendezvous peer id** | Per join socket — changes on every attempt, including every reconnect | Signalling relay + host-side connection bookkeeping |
| **Session token** (32 hex chars, resolved by `gui/session-token.js`) | Per browser tab during play; reload-safe for that tab; the persistent `localStorage` token is only adopted when not already live elsewhere | Server-side player identity |

`client.html` resolves the local player token just before the transport sends `Identify`. The helper stores the active tab token in `sessionStorage`, uses a short-TTL `localStorage` heartbeat registry (`phoenix-live-tabs`) to detect other live tabs, and mints a fresh token when a duplicated tab inherits a token already in use. This keeps multiple clients on one computer from collapsing into one server-side player while preserving normal reload reconnects.

The JS shell on `server.html` keeps three maps:

| Map | Key → Value | Purpose |
|---|---|---|
| `peerTokens` | `rendezvousPeerId → sessionToken` | Populated on first `Identify` message |
| `tokenConns` | `sessionToken → connection` (reliable) | Routing `Target::Token` messages |
| `tokenSnapshotConns` | `sessionToken → RTCDataChannel` (snapshot) | Routing snapshot-class messages |

The lossy channel and `Identify` travel independently, so the third map is only ever written when both facts are known: `attachHostConn` binds on the channel transition and again when `Identify` names the token, in either order, and each half is guarded on connection identity so a stale connection's teardown cannot delete the live one's entry. When no lossy channel exists, snapshot messages fall back to the reliable one.

## Dual DataChannel architecture

Every client maintains two parallel DataChannels on the same `RTCPeerConnection`. Both are created by the **joiner**, before the offer, so the answering host picks them up by label off one negotiation:

| Channel | Label | Ordered | Retransmit | Carries |
|---|---|---|---|---|
| Reliable | `'reliable'` | Yes | Yes | The join handshake, `ClientMessage` commands, `ServerMessage` lobby/events |
| Snapshot | `'snapshot'` | No | 0 (never) | `SimState`, `BlackboardUpdate`, `ShieldStatus`, `RepairState`, `PowerState`, `WeaponsUpdate`, `SystemHullUpdate` |

Delivery class is producer-owned. Registered broadcaster phases stamp their
own class; arbitrary-target simulation producers call the explicit
`SimOutbox::push_snapshot` / `push_reliable` seam. The raw queue is private,
and its forwarding drain preserves the chosen class without matching on
`ServerMessage`. `flush_outbound` in `src/server/bridge.rs` passes the delivery
class as a third string argument to the JS callback. `routeOutbound` in
`server.html` delegates to the pure `outboundTargets()` in
`gui/host-peer-routing.js`, which prefers the lossy channel for `snapshot` and
falls back **per token** to the reliable one for any client whose snapshot
channel has not negotiated or has gone — one old phone cannot drag the whole
bridge onto the reliable channel. The joiner's own `send` applies the same rule
upward.

## Client connection lifecycle (`gui/rendezvous-transport.js`)

`createRendezvousJoiner` owns code resolution, the offer, both channels, the
compatibility handshake, `Identify`, and the reconnect loop. `client.html` wires
its callback interface: `onData`, `onStatus`, `onLog`, `onError`, `onDiag` and
`getIdent` for the session token, and publishes a **stable façade** over
whichever joiner is current as `window.phoenixLink` — the joiner behind it is
replaced on every reconnect, so nothing may hold the joiner itself.

Reconnect uses persistent exponential backoff from 100 ms to a 30 s cap
(`nextBackoffDelay`), with a per-attempt connect timeout escalating 8 s → 16 s →
30 s (`connectTimeoutMs`); both live in `gui/connection-manager.js`, which since
#1112 holds ICE configuration and connection *timing policy* only. Every attempt
re-resolves **the same code** and re-sends `Identify` with the **same token**, so
the host restores the held station and pushes the current projection and nobody
re-types five letters. During a round, registered replication owners reconstruct
current Snapshot state in stable key order for that token only. The Blackboard
and Hull adapters reproduce live recipient visibility. The cache-free Shields
adapter reuses the periodic publisher's authored-System audience and current
payload builder, while the Weapons adapter produces the live projection only for
a session holding the ship's authored Weapons Station; none resets shared caches.
A generation guard ignores callbacks from superseded
attempts. The client offers **Retry now**, which skips the backoff wait.

Only a refusal a retry cannot fix ends the loop and goes back to the entry field
with its own sentence: an answer about the *code* (unknown, wrong-type,
version-mismatch, admission-closed, host-gone) or about the *build* (the host's
`StampMismatch` codes). Everything else — an unreachable service, a signalling
drop, an ICE timeout — is retried. `reconnect-midgame-sever.spec.js` exercises
sever and revive during play.

The host has a matching loop: a lost record re-registers on a backoff and is
issued a **fresh** code, because the old record really is gone and the letters on
screen resolve to nothing. Keeping the *same* code across a host drop needs
persistence in the service and is issue #1115's.

## Host page cues via HUD/lobby push

Host-local presentation cues use dedicated push callbacks rather than inspecting
peer message JSON in `routeOutbound`:
- `__updateHud` — carries `ViewscreenHudState` with `engine_thrust` (engine hum volume), `red_alert` (siren), etc.
- `__updateLobby` — carries `LobbyStatePayload` with `phase` (loading overlay visibility) and `loading_progress` (progress bar value).

`routeOutbound` is a pure forwarder — it inspects no JSON payloads, only dispatches by `Target` + `DeliveryClass`.

## Connection state machine (PRD #17)

Both pages show a coloured dot in the top-right:

| State | Dot | Reached when |
|---|---|---|
| `connecting` | green (no label) | A first join attempt is in flight |
| `ready` | green (no label) | The host accepted this build (`JoinAccepted`), or a code was issued |
| `disconnected` | red + "Disconnected — reconnecting…" | An accepted link dropped; the transport is retrying on its own |
| `error` | red + "Error — refresh to retry" | An answer a retry cannot change, before ever being accepted |

`connecting` is deliberately **not** re-reported on each reconnect attempt: an
accepted link that is down reads as "reconnecting" until it is actually back,
rather than flashing the dot green several times a second.

## ICE servers, TURN relay, and on-device diagnostics (2026-08 hotspot fix)

The base ICE list (`defaultIceServers()`) is **STUN-only**; TURN relay credentials come primarily from the Cloudflare worker (`worker/`, deployed per-target as `phoenix-turn-credentials` / `phoenix-turn-credentials-demo`). If the worker is unreachable, `fetchIceServers()` falls back to Metered's shared OpenRelay TURN (`openRelayFallbackServers()`) and reports `relaySource: 'openrelay'`, which both pages surface as a mild fallback notice. OpenRelay uses `staticauth.openrelay.metered.ca`.

**Relay is mandatory on hotspot/CGNAT networks** (phone tethering, two phones on separate mobile data): STUN hairpinning through carrier NAT essentially never works, and host candidates are mDNS-obfuscated. Three things therefore surface degraded relay instead of failing silently:

- `fetchIceServers()` returns `{ servers, relayAvailable, relaySource }`; both pages show a warning (`client.diag_no_relay` / `server.no_relay_warning`) when `relayAvailable` is false, and a milder notice (`client.diag_relay_fallback` / `server.relay_fallback_notice`) when running on the shared OpenRelay fallback.
- The client join screen shows a live diagnostics readout (`#conn-diag`): relay probe verdict (`probeTurnRelay()`, an `iceTransportPolicy:'relay'` throwaway connection) plus per-attempt ICE state and gathered candidate types, fed by the transport's `onDiag` events. "candidates: host, srflx" with no relay on a failing network is the TURN smoking gun.
- The host lobby mirrors any inbound client stuck mid-ICE under the QR code, fed by `createRendezvousHost`'s `onPeerIce` callback — when ICE never completes no channel ever opens, so nothing else would report it.

The per-attempt connect timeout escalates 8s → 16s → 30s (`connectTimeoutMs()`) since TURN-over-TCP allocation on cellular can exceed the old flat 8s.

Both workers validate CORS against the comma-separated `ALLOWED_ORIGIN` list in their own `wrangler.toml`. The deployed value only changes after `wrangler deploy`; an incorrect allowlist blocks the TURN credential fetch (removing relay) or the rendezvous upgrade (removing joining altogether).

## Why almost no backend

- Game data is peer-to-peer. Only the WebRTC handshake touches a signalling service — Phoenix's own rendezvous Worker, which holds transport metadata and nothing else.
- Hosting is GitHub Pages / Cloudflare Pages — pure static. No game state lives on the Worker; a private join code never enters simulation state, a snapshot, or another host's state.
- Self-hosted PeerJS was an explicit out-of-scope item from PRD #1. PRD #1093 reversed that by *replacing* PeerJS rather than self-hosting it, which issue #1112 completed.

## Smoke testing without WebRTC

CI has no real WebRTC and no deployed worker, so the Playwright suite installs one stand-in before any page script runs (`addInitScript`): `tests/smoke/rendezvous-shim.js`, published as `window.PhoenixTransportFactories`, which `gui/rendezvous-transport.js` takes its socket and peer from. It fakes a WebSocket onto the **real** `worker-rendezvous` registry running inside the host page, and an `RTCPeerConnection` that pairs two pages' DataChannels over a `BroadcastChannel`, honouring the `createDataChannel` init bag so the lossy channel is distinguishable from the reliable one. Only the transport is faked, never the protocol.

It also carries `window.__wasmReady` (set once the host page has both opened its rendezvous socket and dispatched `PhoenixReady`) and `window.__transportShim.sever()/revive()`, a page-scoped kill switch for the "phone's radio slept" failure.

`tests/smoke/transport-fixture.js` is the single seam every transport assumption lives behind — `fixtures.js` and the ~40 specs importing `readHostPeerId`/`createTestClient` know nothing about it. See `tests/smoke/transport-shim.spec.js` (the stand-in itself), `rendezvous-join.spec.js` (typed join, QR link, distinct refusals), `multi-client-crew.spec.js` (four phones on one code) and `snapshot-channel.spec.js` (the delivery-class split and its per-token fallback), plus [Testing Strategy](./testing-strategy.md).

## Related

- [Architecture](./architecture.md) · [Message Flow](./message-flow.md)
- [Player](../entities/player.md) · [Session](../entities/session.md)
