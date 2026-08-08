---
title: Message Flow
type: concept
tags: [messages, bridge, wasm, bevy, events, routing, delivery-class, snapshot]
sources: [src/server/bridge.rs, src/lobby/server.rs, src/lobby/handler.rs, src/server_app.rs, AGENTS.md]
updated: 2026-07-16
---

# Message Flow

The single most important diagram in the project.

```
Phone (client.html — pure HTML/JS)
   │  builds ClientMessage, encodes to JSON, sends over PeerJS
   ▼
server.html JS
   │  resolves peerId → sessionToken
   │  calls wasm_receive_message(token, json)
   ▼
src/server/bridge.rs :: drain_inbound()
   │  decodes via JsonCodec, queues InboundMessage
   ▼
Bevy systems in server_app.rs / console plugins
   │  read InboundMessage events (pull-based)
   │  mutate session / ship state
   │  write OutboundMessage events with routing target + delivery class
   ▼
src/server/bridge.rs :: flush_outbound()
   │  encodes ServerMessage → JSON
   │  invokes JS callback(target, payload, deliveryClass)
   ▼
server.html JS :: routeOutbound(target, payload, deliveryClass)
   │  dispatch to snapshot DataChannel (unordered) or reliable DataChannel
   │  fallback: snapshot-unavailable tokens → reliable channel
   ▼
client.html (one or more phones)
   │  decode → update local view-model → render
```

## Bevy 0.18 message system

This project uses Bevy's **pull-based** message system (not legacy events):

- `app.add_message::<InboundMessage>()`, `add_message::<OutboundMessage>()`, `add_message::<PlayerDisconnected>()`.
- Producers use `MessageWriter<T>`. Consumers use `MessageReader<T>`.
- Messages live one frame and are drained.

## Lobby message dispatch (per-variant systems)

Inbound `ClientMessage` lobby variants are no longer routed through a single
monolithic `process_lobby` / `process_message` dispatch (deleted in #734). Each
lobby variant is now handled by its own dedicated Bevy system in `LobbySystemSet`
(`src/lobby/server.rs`), reading the inbound bus with its own `MessageReader`
cursor and calling the matching pure handler in `src/lobby/handler.rs`:

| System | Variant | Phase gate |
|---|---|---|
| `handle_identify_system` | `Identify` | Lobby / Loading / InProgress (mid-game reconnect) |
| `handle_set_name_system` | `SetName` | Lobby / Loading |
| `handle_return_to_lobby_system` | `ReturnToLobby` | GameOver |
| `handle_select_station_system` | `SelectStation` | Lobby / Loading / InProgress |
| `handle_release_station_system` | `ReleaseStation` | Lobby / Loading / InProgress |
| `handle_set_ready_system` | `SetReady` | Lobby / Loading / InProgress |
| `handle_set_station_rating_system` | `SetStationRating` | Lobby / Loading / InProgress |

Ordering within the set: `handle_disconnect` runs first (so a same-frame
disconnect+reconnect vacates then restores a seat in the right order), then the
per-variant message systems, then `tick_countdown → update_game_state_cache`.
The seven in-game runtime variants (`ControlSystem`, `FirePhaser`, `FireTorpedo`,
`SetPhaserFrequency`, `SendCoordination`, `LoadTube`, `UnloadTube`) are simply
not read by any lobby system — the console server plugins own them under
`SimSet::Input`.

## Routing targets

`OutboundMessage` carries a routing target enum and a delivery class:

| Field | Type | Description |
|---|---|---|
| `target` | `Target` | Who receives the message |
| `msg` | `ServerMessage` | The payload |
| `delivery` | `DeliveryClass` | How to send: `Reliable` or `Snapshot` |

Target variants:

- `Target::All` — broadcast to every connected client.
- `Target::Token(SessionToken)` — direct to one player (used for `Welcome`, per-station payloads).
- `Target::AllExcept(SessionToken)` — broadcast minus one (e.g. `PlayerJoined` to everyone but the joiner).

## Delivery classes

Defined in `src/core/messages.rs:138`:

```rust
pub enum DeliveryClass {
    Reliable,  // ordered, retransmit — PeerJS default DataChannel
    Snapshot,  // unordered, no retransmit — second DataChannel
}
```

- **`Reliable`** — guarantee delivery in order. Used for commands (`FirePhaser`, `SelectStation`, `SetReady`), lobby messages (`Welcome`, `PlayerJoined`), and anything where dropping a message would cause visible state corruption.
- **`Snapshot`** — best-effort, unordered, never retransmitted. Used for periodic state broadcasts (`SimState`, `BlackboardUpdate`, `ShieldStatus`, `RepairState`, `PowerState`, `WeaponsUpdate`, `SystemHullUpdate`) where a stale snapshot is worse than a dropped one. The server classifies each message variant via `delivery_class_for_msg()` in `src/server_app.rs:664`.

On the wire `routeOutbound` (server.html:1510) dispatches snapshot-class messages to the client's `tokenSnapshotConns` (unordered DataChannel with `maxRetransmits: 0`) when available, falling back to the reliable channel. The reliable channel is always the final fallback — no snapshot-class message is ever dropped due to an absent snapshot channel.

## Disconnect lifecycle

JS detects a peer drop and calls `wasm_player_disconnected(token)`. The bridge fires a `PlayerDisconnected` event that:

1. The session manager handles → frees the console immediately.
2. The lobby/simulation systems handle → broadcast `PlayerLeft` to remaining clients.

## Tick rates

| Channel | Rate | Delivery |
|---|---|---|
| Discrete events (`PlayerJoined`, `StationAssigned`, `GameStarted`, …) | Immediate (per inbound message) | Reliable |
| `SetThrust` / `SetSteering` / commands from clients | 10 Hz while controls are active | Reliable |
| `SimState` broadcast | 10 Hz | Snapshot |
| Per-console state (`PowerState`, `WeaponsUpdate`, `RepairState`, `ShieldStatus`) | 10 Hz | Snapshot |
| `BlackboardUpdate` / `SystemHullUpdate` | 10 Hz | Snapshot |
| Bevy frame loop | `requestAnimationFrame` (browser tab) | — |

See [Game Loop](./game-loop.md).

## Why the codec seam matters here

`src/server/bridge.rs:836` (`flush_outbound`) is the *only* production call site of `JsonCodec::encode_server` / `decode` outside tests. This means the wire format is one module change away from being binary. See [Codec Seam](./codec-seam.md).

## Snapshot channel lifecycle

1. When a client connects, `connection-manager.js` creates an ordered DataChannel (PeerJS `DataConnection` with `{ reliable: true }`).
2. Once the reliable channel opens, `connection-manager.js` creates a second unordered DataChannel on the same `RTCPeerConnection` via `pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 })`.
3. The server's `server.html` listens for `pc.ondatachannel` with `label === 'snapshot'` and registers the channel in `tokenSnapshotConns` keyed by session token.
4. `flush_outbound` (bridge.rs:836) passes each `OutboundMessage`'s delivery class as a third `"reliable"` / `"snapshot"` string argument to the JS callback.
5. `routeOutbound` (server.html:1510) dispatches to `tokenSnapshotConns` for snapshot-class messages or `tokenConns` for reliable ones. When a token has no snapshot channel, it falls back to the reliable channel.
6. On disconnect, both maps are cleaned up. On reconnect, a new snapshot channel is created alongside the new reliable connection.

See [Networking](./networking.md) for the DataChannel creation details.

## Inbound decode resilience (#602)

`drain_inbound` (`src/server/bridge.rs`) now calls `decode_bridge_client_messages` (`src/core/codec.rs`) which partitions decode results into successes and `DecodeError` entries instead of failing the whole batch on one bad message. Each decode failure is logged at `warn!` level with the raw JSON snippet. `Identify` token is clamped to 64 chars and name to 32 chars at the lobby handler boundary using Unicode-safe char truncation — caught by the aforementioned clamping before they reach session bookkeeping.

## Bridge debug-toggles (#609)

Six near-identical `RefCell<bool>` thread-locals (one per debug overlay) were replaced by one `DebugToggleKind` enum-keyed pending set plus `apply_pending_toggles` — a pure function (no Bevy/wasm dependency, unit-testable natively) that flips the corresponding `bool` flag for each variant present in the set. Adding a new debug overlay means: add a variant to `DebugToggleKind`, add its resource field to `apply_pending_toggles`, and add one `wasm_bindgen` export — no new thread-local, no new drain block. The six `wasm_bindgen` export names/signatures are unchanged so `server.html`'s hotkey wiring needed no changes.

Since issue #939 these toggles live on the **Debug/Cheat** tab of the server
page's settings cog (`gui/server-settings.js`), which is omitted entirely from
the public demo build (`PHOENIX_DEMO_BUILD=true`, read back via
`wasm_is_demo_build()` and the `phoenix-build-demo` meta tag). That is
deliberately NOT `TRUNK_BUILD_RELEASE`: the GitHub Pages dev host sets that one
too, and it keeps its debug tooling. Its overlay, damage, entity, and inspector views are
host-local diagnostics — selecting one enables just that resource and pops out
the output panel — but god mode and instagib alter the authoritative simulation
and are host-only powers rather than diagnostic-only controls.

Since issue #940 a connected **phone** can flip most of them too, from the same
three tabs on its own cog (`gui/settings-panel.js`), by three routes — and none
of the three exists in a demo build:

- **The overlay flags** travel as `ClientMessage::ToggleDebugFlag`, a top-level
  *session* control, drained frame-driven in `PreUpdate` by
  `debug_overlay::drain_client_debug_flags` into the *same*
  `apply_pending_toggles` the host's own drain calls. It never touches command
  admission: the flags are presentation and belong in no replay.
- **Pause** travels as `ClientMessage::TogglePause`, its own top-level variant,
  drained by `debug_overlay::drain_client_pause`. It could not use command
  admission even if it wanted to — pausing starves `FixedUpdate`, where
  admission has run since #895, so an admitted pause could never be undone —
  and it needs a build gate of its own regardless.
- **God mode** travels as the ordinary `ControlSystem` / `ToggleGodMode`
  command #900 already built, because it changes damage outcomes and so has to
  be tick-stamped, logged and replayable.

The gate is `phoenix_demo_build`, the cfg `build.rs` derives from the same
`PHOENIX_DEMO_BUILD`. The first two routes are gated at the **variant**: both
`ClientMessage` variants carry `#[cfg(not(phoenix_demo_build))]`, so a demo
binary cannot decode either frame and neither drain is compiled. The third is
gated by `command_admission::debug_route`, which is `#[cfg]`-split because its
message (`ControlSystem`) exists in every build and only one `(target, payload)`
pair may vanish. In a demo build all three are *absent*, not merely refused —
the hidden control and the closed route cannot come apart.

Pause is gated for a blunter reason than the overlays. Nothing on the server
side checks station, captaincy or `GamePhase` before honouring a client pause,
and a demo is N strangers on N phones — so any one of them could otherwise
freeze the mission for everyone, repeatedly. That is why the phone's pause is
dev-only while the **host's** pause, on the server cog, keeps working in every
build: the host is one trusted operator standing at the viewscreen.

The host reports the result back as `ServerMessage::DebugState` — flags, pause
and god mode — broadcast on change from `PreUpdate` so it still reaches a phone
whose sim is frozen, and the phone paints its buttons from that rather than from
its own click. That read-back is sent in **every** build, demo included: it
grants no authority, so a demo phone can learn that the host paused the mission
and still has no way to pause it itself. **Instagib** stays host-only: unlike
god mode it was never routed through command admission, so there is nothing to
admit.

The same tab carries **Teleport to Waypoint**: enabled only while the shared
authoritative Navigation waypoint exists, and immediately moves the local ship
there. Like the other simulation overrides, it is host-only and absent from the
demo build.

Pause is deliberately NOT on that tab. It is a gameplay control on the cog's
**Gameplay** tab, alongside exit-to-lobby, so both survive into the demo build;
`wasm_toggle_pause()` / `SimulationPaused` are named accordingly.

## Diagnosing WASM panics

`wasm_init` calls `console_error_panic_hook::set_once()` before `App::new()`. Without this, any Rust panic anywhere in the Bevy app traps the wasm instance and every *subsequent* JS→WASM call surfaces as `RuntimeError: memory access out of bounds`, almost always pointing at `wasm_receive_message` (the next entry point fired by PeerJS). The hook routes the real panic message + Rust source location to `console.error` so the actual fault is visible.

If a "memory access out of bounds" trace ever points at `wasm_receive_message` again, look earlier in the console for the *real* panic — and check that the hook is still installed in `src/server/bridge.rs::wasm_init`.

## Channel-3 Coordination flow

Channel-3 coordination messages (issue #494) let one system send a typed message to another system's human operator, or be consumed silently by AI:

```
detect_damage_tier_crossings (SimSet::Damage)
  │  writes CoordinationEnqueue with target + payload
  ▼
handle_coordination_enqueue (SimSet::Input, next frame)
  │  reads CoordinationEnqueue, enqueues to
  │  CoordinationLagQueue with due_time = now + coordination_lag_secs
  ▼
process_coordination_lag (SimSet::Modifiers)
  │  drains due messages, resolves live target control source
  │  route_coordination(sender_origin, target_control):
  │    AI→AI         → Consume (AI handles silently)
  │    AI→Human      → Popup (CoordinationPopup to station holder)
  │    Human→Human   → Suppress (they talk IRL)
  │    Human→AI      → Popup (request routed to AI system)
  ▼
LobbyOutbox → OutboundMessage → routeOutbound → PeerJS
```

New in #684: `detect_damage_tier_crossings` emits a `CoordinationPayload::Alert { title, body }` targeting `captain_system_id()` when a system crosses into `DamageTier::Destroyed`. The `sender_origin` is the destroyed system's control source, so:
- AI-controlled system destroyed → alert shown as popup to human captain.
- Human-controlled system destroyed → alert suppressed (the player at that station can observe the destruction directly).

## Related

- [Architecture](./architecture.md) · [Networking](./networking.md)
- [Game Phases](./game-phases.md) — when what's allowed
