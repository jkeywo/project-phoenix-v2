---
title: Message Flow
type: concept
tags: [messages, bridge, wasm, bevy, routing, delivery-class, coordination]
sources: [src/server/bridge.rs, src/core/codec.rs, src/core/messages.rs, src/core/broadcast/sim.rs, src/lobby/server.rs, src/lobby/handler.rs, src/command_admission/, src/server_app/broadcast_publish.rs, src/ship/shields.rs, src/ship/coordination.rs, src/ship/coordination_systems.rs, server.html, client.html, gui/sim-state.js, gui/console-state.js]
updated: 2026-08-27
---

# Message Flow

```text
phone client (pure HTML/CSS/JS)
  → PeerJS JSON
server.html
  → peer id resolves to session token
  → wasm_receive_message(token, json)
src/server/bridge.rs
  → JsonCodec decode
  → InboundMessage
lobby or command-admission/console systems
  → authoritative ECS/session mutation
  → ServerMessage via lobby/simulation broadcaster
src/server/bridge.rs
  → JsonCodec encode
  → JS callback(target, payload, delivery class)
server.html
  → reliable or snapshot DataChannel
client.html / gui/sim-state.js
  → fold snapshot
  → gui/console-state.js builds per-console render state
```

The host simulation is authoritative. Clients submit intent and render projected state; they do not simulate outcomes or communicate directly with each other.

## Inbound paths

Lobby/session variants are handled by dedicated systems in `src/lobby/server.rs`, with pure state transitions in `src/lobby/handler.rs`. Identification and station selection remain available where reconnect/seat changes require them.

In-game actions use `ClientMessage::ControlSystem { target: SystemId, payload }`. `command_admission` resolves token tenure, station ownership, system damage/availability, control source, and special host-only routes once per logical tick. Accepted commands enter the owning ship's `AdmittedCommands`; the domain applier then treats human and AI emissions identically.

## Outbound routing

`OutboundMessage` carries a `Target`, `ServerMessage`, and `DeliveryClass`:

- `Target::All`, `Token`, and `AllExcept` identify recipients;
- `Reliable` is ordered/retransmitted for lifecycle, setup, and one-shot state changes;
- `Snapshot` is unordered and not retransmitted for replaceable periodic state.

Direct `SimBroadcaster` producers are stamped `Snapshot` by
`src/core/broadcast/sim.rs`; this is the live 10 Hz `ShieldStatus` path.
Messages queued through `SimOutbox` are classified by `delivery_class_for_msg`
beside `sim_outbox_broadcaster` in `src/server_app/broadcast_publish.rs`,
including the targeted one-shot Shields projection rebuilt for reconnect.
`flush_outbound` is the Rust-to-JavaScript boundary. `routeOutbound` prefers
the client's unordered snapshot channel for snapshot traffic and falls back to
reliable when that channel is absent.

## Reconnect and disconnect

Session tokens are persistent identity; peer ids are transport details. On disconnect, station tenure is retained for reconnection while that station's live rating becomes `Backfill`, allowing its systems to continue under AI. A later valid claim or the original token's reconnect updates tenure through the authoritative lobby path and forces the appropriate state projections to the client.

## Coordination

Cross-station facts enter as `CoordinationEnqueue`, serve the hull's authored lag in its per-ship queue, and are routed from the live recipient control source. Human recipients receive `CoordinationPopup`; AI recipients consume the typed delivered fact. Human/AI actor identity affects presentation, not the authoritative payload or domain applier.

Damage-tier destruction alerts target the Captain-owned system. Tactical arc-bearing requests and Navigation clearances use their authored owning systems; they do not fall back to all clients when an address is valid but vacant.

## Codec resilience

All JSON encoding/decoding is centralised in `src/core/codec.rs`. `decode_bridge_client_messages` partitions valid messages from decode errors so one malformed frame does not discard the rest of the inbound batch. The bridge logs rejected payloads and continues processing.

## Related

- [Architecture](./architecture.md)
- [Networking](./networking.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Game Loop](./game-loop.md)
