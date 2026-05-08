---
title: Message Flow
type: concept
tags: [messages, bridge, wasm, bevy, events, routing]
sources: [src/server/bridge.rs, src/server/lobby.rs, src/server/simulation.rs, AGENTS.md]
updated: 2026-05-08
---

# Message Flow

The single most important diagram in the project.

```
Phone (client.html — Bevy/WASM)
   │  builds ClientMessage, encodes to JSON, sends over PeerJS
   ▼
server.html JS
   │  resolves peerId → sessionToken
   │  calls wasm_receive_message(token, json)
   ▼
src/server/bridge.rs :: drain_inbound()
   │  decodes via JsonCodec, queues InboundMessage
   ▼
Bevy systems in lobby.rs / simulation.rs
   │  read InboundMessage events (pull-based)
   │  mutate SessionManager / ShipState
   │  write OutboundMessage events with routing target
   ▼
src/server/bridge.rs :: flush_outbound()
   │  encodes ServerMessage → JSON
   │  invokes the JS callback registered with set_message_callback
   ▼
server.html JS :: routeOutbound()
   │  fans out: All / Token(t) / AllExcept(t)
   ▼
client.html (one or more phones)
   │  decode → update local view-model → render
```

## Bevy 0.18 message system

This project uses Bevy's **pull-based** message system (not legacy events):

- `app.add_message::<InboundMessage>()`, `add_message::<OutboundMessage>()`, `add_message::<PlayerDisconnected>()`.
- Producers use `MessageWriter<T>`. Consumers use `MessageReader<T>`.
- Messages live one frame and are drained.

## Routing targets

`OutboundMessage` carries a routing target enum:

- `All` — broadcast to every connected client.
- `Token(SessionToken)` — direct to one player (used for `Welcome`, future per-console payloads).
- `AllExcept(SessionToken)` — broadcast minus one (e.g. `PlayerJoined` to everyone but the joiner).

PRD #66 adds `Target::One(token)` per-console payloads at 10 Hz so Weapons/Engineering only get the messages they need.

## Disconnect lifecycle

JS detects a peer drop and calls `wasm_player_disconnected(token)`. The bridge fires a `PlayerDisconnected` event that:

1. The session manager handles → frees the console immediately.
2. The lobby/simulation systems handle → broadcast `PlayerLeft` to remaining clients.

## Tick rates

| Channel | Rate |
|---|---|
| Discrete events (`PlayerJoined`, `ConsoleSelected`, `GameStarted`, …) | Immediate (per inbound message) |
| `HelmInput` from clients | 10 Hz while controls are active |
| `SimState` broadcast | 10 Hz |
| Bevy frame loop | `requestAnimationFrame` (browser tab) |

See [Game Loop](./game-loop.md).

## Why the codec seam matters here

`bridge.rs` is the *only* call site of `JsonCodec::decode` / `encode` outside tests. This means the wire format is one module change away from being binary. See [Codec Seam](./codec-seam.md).

## Related

- [Architecture](./architecture.md) · [Networking](./networking.md)
- [Game Phases](./game-phases.md) — when what's allowed
