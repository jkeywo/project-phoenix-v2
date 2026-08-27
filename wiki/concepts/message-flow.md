---
title: Message Flow
type: concept
tags: [messages, bridge, wasm, bevy, routing, delivery-class, coordination]
sources: [src/server/bridge.rs, src/core/codec.rs, src/core/messages.rs, src/core/broadcast/sim.rs, src/lobby/server.rs, src/lobby/handler.rs, src/command_admission/, src/server_app/broadcast_publish.rs, src/ship/shields.rs, src/ship/coordination.rs, src/ship/coordination_systems.rs, src/console/weapons/server.rs, src/console_bridge.rs, server.html, client.html, gui/client-router.js, gui/sim-state.js, gui/console-state.js, gui/coordination-popup.js]
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

Cross-station facts enter as `CoordinationEnqueue` with an explicit
`CoordinationAddress::Station(StationId)` or `CoordinationAddress::Ship`; the
payload never implies its recipient. They serve the hull's authored lag in its
per-ship queue and are routed from the live recipient control source. Station
delivery emits a popup for a human recipient or `DeliveredCoordination` for an
AI recipient. Ship delivery fans out deterministically in authored Station
order under the same policy. Human/AI actor identity affects presentation, not
the authoritative payload or domain applier.

Every emission also carries a required producer-owned
`CoordinationPresentation`. The router preserves that envelope unchanged beside
the typed payload through the lag queue and into `CoordinationPopup`,
`DeliveredCoordination`, and `AiChatterEvent`; phone and Viewscreen presenters
therefore render the same localised-or-literal title, body, and deterministic
scalar parameters without enumerating `CoordinationPayload`. The authoritative
route heading is derived from the typed address. Its core Ship label is the
undecorated `Ship` display value, matching Station labels; the phone and
Viewscreen layouts each add their own single pair of square brackets. The phone
keeps separate title and body lines, while the Viewscreen joins them onto its
existing single chatter line.

Damage-tier destruction alerts address the Station that owns Captain. Tactical
arc-bearing requests and Navigation clearances resolve their fine System to its
authored owning Station; they do not widen to all clients when an address is
valid but vacant.

## Codec resilience

All JSON encoding/decoding is centralised in `src/core/codec.rs`. `decode_bridge_client_messages` partitions valid messages from decode errors so one malformed frame does not discard the rest of the inbound batch. The bridge logs rejected payloads and continues processing.

## Related

- [Architecture](./architecture.md)
- [Networking](./networking.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Game Loop](./game-loop.md)
