---
title: Message Flow
type: concept
tags: [messages, bridge, wasm, bevy, routing, delivery-class, coordination]
sources: [src/server/bridge.rs, src/core/codec.rs, src/core/messages.rs, src/core/broadcast/, src/lobby/server.rs, src/lobby/handler.rs, src/command_admission/, src/server_app/components.rs, src/server_app/broadcast_publish.rs, src/ship/shields.rs, src/ship/coordination.rs, src/ship/coordination_systems.rs, src/console/helm/server.rs, src/console/weapons/server.rs, src/console/repair/server.rs, src/console_bridge.rs, server.html, client.html, gui/client-router.js, gui/sim-state.js, gui/console-state.js, gui/coordination-popup.js]
updated: 2026-08-28
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
  → ServerMessage via registered broadcaster, SimOutbox, or LobbyOutbox
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

Registered `SimBroadcaster` producers inherit the simulation broadcaster's
`Snapshot` class. Arbitrary-target simulation producers choose explicitly at
the insertion site with `SimOutbox::push_snapshot` or `push_reliable`; its raw
queue is private, and `sim_outbox_broadcaster` forwards the stored class without
matching on the `ServerMessage` variant. `LobbyOutbox` remains intrinsically
Reliable. `flush_outbound` is the Rust-to-JavaScript boundary. `routeOutbound`
prefers the client's unordered snapshot channel for snapshot traffic and falls
back to reliable when that channel is absent.

## Reconnect and disconnect

Session tokens are persistent identity; peer ids are transport details. On disconnect, station tenure is retained for reconnection while that station's live rating becomes `Backfill`, allowing its systems to continue under AI. A later valid claim or the original token's reconnect updates tenure through the authoritative lobby path and forces the appropriate state projections to the client.

During `InProgress`, the token-targeted `Welcome` also invokes registered
replication owners in stable key order. Each owner reconstructs its current
Snapshot projection only for that token without mutating shared delta caches.
The Blackboard adapter applies the same Repair visibility policy as live
publication.

## Coordination

Cross-station facts enter as `CoordinationEnqueue` with an explicit
`CoordinationAddress::Station(StationId)` or `CoordinationAddress::Ship`; the
payload never implies its recipient. They serve the hull's authored lag in its
per-ship queue and are routed from the live recipient control source. Station
delivery emits `DeliveredCoordination` tagged with the selected
`CoordinationDelivery::Ai` or owning-module `HumanPopup` outcome. Generic
Station and Ship popups enter `OrderedCoordinationPopup` directly; an owning
receiver enters an accepted human popup there after its decision. One shared
flush sorts those candidates by the router's same-tick sequence before the
outbox, while Ship fan-out stays in authored Station order. Human/AI
actor identity affects presentation, not the authoritative payload or domain
applier.

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

For an AI Helm recipient, the router emits an AI-tagged
`DeliveredCoordination`. `receive_helm_coordination` accepts no other delivery
outcome, rechecks the explicit Station and live `helm-steering` policy, then
preserves exact `NavigateTo` generations and arc geometry. `ArcBearingWithdraw`
clears the standing request across weapon families in delivered queue order.

Shields owns the AI outcome for `ThreatBearing`.
`receive_shields_coordination` verifies both the authored shield-arc Station
and the live authored Shields focus capability before latching the bearing in
`PendingShieldsThreatBearing` for the following Shields AI decision. The
generic router no longer reads Shields' private pending state.

Repair owns both outcomes for a `RepairRequest`. An AI delivery retains the
exact host-internal deficit and merges into `RepairRequestQueue`. A human-popup
delivery already carries the resolved token, sender label, recipient-projected
payload, and unchanged presentation envelope; `receive_repair_coordination`
applies the first sub-Disabled / every Disabled-or-Destroyed latch before
emitting it to the shared ordered popup flush. The generic router no longer
reads Repair's queue or alert latch, and Repair does not reorder same-tick
generic Station or Ship popups while making that decision.

## Codec resilience

All JSON encoding/decoding is centralised in `src/core/codec.rs`. `decode_bridge_client_messages` partitions valid messages from decode errors so one malformed frame does not discard the rest of the inbound batch. The bridge logs rejected payloads and continues processing.

## Related

- [Architecture](./architecture.md)
- [Networking](./networking.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Game Loop](./game-loop.md)
