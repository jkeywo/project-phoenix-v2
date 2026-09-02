---
title: Networking
type: concept
tags: [networking, webrtc, rendezvous, join-code, session-token, gm, star-topology, datachannel, snapshot, fleet, host-mesh, lockstep, ws-relay, diagnostics]
sources: [server.html, client.html, gui/rendezvous-transport.js, gui/rendezvous-relay.js, gui/rendezvous-protocol.js, gui/transport-levers.js, gui/connection-diagnostics.js, gui/join-code.js, gui/host-mesh.js, gui/fleet-session.js, src/gm_roster.rs, src/gm_join.rs, src/lockstep/frame.rs, src/lockstep/host_loss.rs, src/lockstep/transfer.rs, src/lockstep/snapshot_relay.rs, src/lockstep/mod.rs, src/core/codec.rs, gui/connection-manager.js, gui/host-peer-routing.js, worker-rendezvous/src/registry.js, worker-rendezvous/src/relay.js, worker-rendezvous/src/index.js, gui/session-token.js, src/core/rendezvous.rs, src/native_host/relay_transport.rs, src/native_host/relay_socket.rs, src/core/broadcast/sim.rs, src/core/broadcast/lifecycle.rs, src/server/bridge.rs, src/server_app/components.rs, src/server_app/world_setup.rs, src/server_app/broadcast_publish.rs, src/console/repair/visibility.rs, src/console/weapons/blackboard.rs, src/delivery/mod.rs, AGENTS.md]
updated: 2026-09-01
---

# Networking

Phoenix uses the **Phoenix transport** in a **star topology** with **two DataChannels** per client: a Phoenix-owned rendezvous service carries typed join-code lookup and WebRTC signalling over a secure WebSocket, and the game traffic then runs over direct WebRTC DataChannels. Issue #1112 made this the only route — PeerJS, its public broker and the peer-id-in-the-URL-fragment mechanic are gone, along with the flag that used to choose between them.

Issue #1113 added a **third rung** under it. Some networks build no direct link at any price, and since PeerJS went that meant no connection at all rather than a degraded one — so when the WebRTC ladder is spent, the same rendezvous socket carries the game's own frames. See [The transport ladder](#the-transport-ladder-issue-1113) below.

## Topology

- The server page (`server.html`) registers with the rendezvous service on every load and is issued a **private five-letter code** in the client namespace. The join panel prints those five letters and a QR of `https://…/client/index.html#<PROJECT_GUID>_<VERSION_GUID>_<CODE>` — the same code, so scanning and typing are one join by two routes.
- Client pages either read that structured code out of `location.hash` or take five typed letters; `gui/join-code.js` decides which a given string is. Both resolve through the service to the same host record and open the same channels.
- **Clients never talk to each other.** All messages flow through the host.
- **Ship hosts talk to ship hosts, on a different wire.** A fleet (issues #1114, #1116) is a second star: each host registers a *second* record in the `server` namespace, and a host that types that fleet code opens its own DataChannel to the lead. That link carries the **host-mesh vocabulary** (`gui/host-mesh.js`) and nothing else — a fleet member never sends `Identify`, never holds a station, and nothing it says reaches `wasm_receive_message`. Each crew star stays attached to its own host because the fleet link is a different socket, in a different namespace, carrying a different vocabulary; not because anything downstream filters it.

```
client #1 ──┐
            ├──▶  host (server.html WASM)  ──snapshot──▶  all clients
client #2 ──┤                   │               │
client #3 ──┘                   │               └── unordered DataChannel
                                │                   (SimState, etc.)
                                └── reliable DataChannel
                                    (commands, lobby messages)

        …and, when the host is in a fleet, one more link per peer:

   host (slot 1) ◀──── host-mesh frames ────▶ host (slot 2)
        │                (m/t/tick/d)               │
   its own crew                              its own crew
```

## Joining: code, signalling, handshake

- **Join identifiers** are `PROJECT_GUID_VERSION_GUID_CODE`. `gui/join-code.js` is the pure scheme — canonicalisation (upper-case; `0`→`O`, `1`/`L`→`I`, `J` distinct), five-letter validation, deny-list refusal, compose/parse and minting — reading its alphabet, GUIDs and deny-list from the authored `assets/join/join-codes.toml`, through `assets/join/join-codes.json`, a committed artifact `scripts/build-client.mjs` regenerates for the three consumers that have no TOML parser (the phone, vitest, the Worker bundle). Client and server joining have separate project GUIDs, so a fleet code typed into the crew field is a *wrong-type* answer rather than a miss.
- **The service** is `worker-rendezvous/`, a sibling Cloudflare Worker to the TURN one, with a Durable Object holding the live registry. All of its behaviour is the transport-free state machine in `worker-rendezvous/src/registry.js` (`/v1/host` and `/v1/join` WebSockets plus a `/v1/health` origin check); the Worker adapter decides nothing except who gets a socket, which is what lets `tests/client/rendezvous-registry.test.js` cover the protocol with no wrangler. Registry state is in memory only (a code dies with its host socket), and its bounds — lookup cap per socket, record TTL, peers per record, stored-field shapes — are authored in `[limits]`. Both upgrade endpoints require a present, allow-listed `Origin`; only `/v1/health` does not.
- **The browser halves** are `gui/rendezvous-transport.js`, one module holding both ends of one frame vocabulary. `createRendezvousHost` registers, is issued a code, and hands each ADMITTED connection to `server.html`'s `attachHostConn` — an open DataChannel is not admission, so nothing reaches the Identify gate before the compatibility verdict, and frames sent before it or after a refusal are dropped. `createRendezvousJoiner` resolves a typed code, offers, opens both channels, completes the handshake and sends `Identify`. `localiseTree` runs on every inbound frame, in one place, so no console has to know which of its fields are localisable.
- **The compatibility handshake** (`JoinHandshake` / `JoinAccepted` / `JoinRefused`) is transport-plane, not a `ClientMessage` — `pasm/spec/design/p2p-design-deltas.yaml` forbids layering transport concerns onto the crew protocol. The verdict comes from Rust: `wasm_check_client_stamp` → `delivery::check_join_stamp` → the same `check_client_stamp` the native host enforces over HTTP, so rendezvous version advice can never become the authority. The client's own stamp is written into `<meta name="phoenix-client-stamp">` by `scripts/build-client.mjs`. **Since #1112 a stamp is required**: absent and garbled are both refused as `client-stamp-missing`, because every client that can reach a Phoenix host is now a built Phoenix bundle.
- **Service selection.** `?rendezvous=<url>` is a development lever and is honoured **only for a loopback origin** (`localhost`, `127.0.0.1`, `[::1]`, `*.localhost`) — a local `wrangler dev`. Every other value, a public staging URL included, falls back to the built-in service, because since #1112 this parameter applies on every ordinary client load: `client/index.html?rendezvous=https://attacker.example#<code>` handed to a guest would otherwise route their SDP, ICE candidates, session token and display name through a third party with nothing on screen saying so. The #1111 opt-in spellings (`?rendezvous`, `=on`, `=off`) are ignored rather than honoured, so an old bookmark still opens the game instead of trying to dial a host called "on".
- **Deploy trap:** the same `ALLOWED_ORIGIN` drift as the TURN worker, except a stale value here means nobody can join at all rather than nobody getting relay. `/v1/health` echoes `origin_allowed` for exactly that check — see `docs/delivery-checklist.md` §3a. **The service is not deployed yet**, and since #1112 there is no second route underneath it.

## Fleet lobby: privileged ship and GM hosts

A fleet session has **one** privileged code, in the `server` namespace, distinct from every ship's private crew code. Its server-only role selector admits either a player-ship host or a GM; a crew code has no GM request surface. A GM receives a public operator identity but consumes no ship slot.

- **A fleet is N single-ship processes plus a coordination layer, not more ships in one ECS world.** `SessionManager`, `GamePhase` and `ShipStations` stay process-singletons; nothing under `src/lobby/` changed for this.
- **The mesh owner** — the host that minted the code — opens a **second** rendezvous registration in the `server` namespace beside its own crew one, and holds the private transport roster. This star-centre role is routing machinery, not a GM leader or permission bit. `gui/fleet-session.js`'s `createFleetOwner`/`createFleetMember` are the two ends; `gui/host-mesh.js` is the pure vocabulary and fleet model behind them.
- **The vocabulary is its own**, per PRD #1093: `{ m, t, tick, d }`. #1114's six lobby frames (`hello`, `welcome`, `refused`, `slot`, `roster`, `admission`) are revision `1`; #1116 added `tick` and `digest` (revision `2`); #1119 added `host-loss` (`3`); fixed-slot recovery moved it to `4`; #1289 added the privileged `ship`/`gm` role and private GM reconnect identity (`5`); #1290 first added `start-state`, `start-policy`, `start-force`, and `force-result` (`6`), then revision `7` removed the asynchronous JavaScript `start-grant` and added the private technical-participant topology. #1292 added the paused-safe typed `gm-action` simulation frame (`8`); #1293 added visible first-time GM request/decision/status controls plus the Rust-owned `gm-join` simulation frame (`9`), then added the owner-sequenced restore-boundary clock (`10`). #1294 makes the join transaction's `first-time`/`reconnect` meaning explicit on the wire (`11`). The start grant now exists only inside the authenticated owner's Rust `TickFrame`. Keys are `m`/`t`/`d` rather than `v`/`type`/`data` so a host frame landing on the crew wire is *refused* by a decoder switching on `.type` instead of half understood. The revision is pinned in both halves (`gui/host-mesh.js` and `src/lockstep/frame.rs`) and a receiver refuses a whole fleet of the wrong revision rather than a frame.
- **A member is never crew.** It sends no `Identify`, gets no session token, never enters `peerTokens`/`tokenConns` and never reaches `wasm_receive_message`. Crew isolation is structural, not filtered. It reaches the host through the ordinary joiner, with three narrow hooks (`onAccepted`, `localise: false`, `sendFrame`) rather than a second copy of the resolve/offer/handshake dance. A GM's public `{id, name, connected, ready}` row is separately full-replaced into Rust through `wasm_set_gm_roster`; the private reconnect capability, rendezvous peer id, and technical mesh slot never cross that projection. The frozen private roster instead carries ordered `owner`, `participants`, and player-ship rows so a zero-ship GM is still a deterministic simulation peer without exposing which public GM owns which mesh slot.
- **A stricter stamp.** `delivery::check_host_stamp` (via `wasm_check_host_stamp`) reuses `check_client_stamp` with both of the crew path's leniencies removed: no content waiver for a host that has not loaded a manifest, and an unreachable export refuses instead of admitting. It also refuses an *empty* content identity on either side rather than comparing two of them — `"" == ""` is not agreement. That bites only because `server.html`'s `pushScenarioManifest()` establishes the identity on **every** boot path, including the `?scenario=` bypass the catalogue build skips; the invitation link carries `?manifest` for the same reason and drops the rest of the query.
- **Typed both ways.** `resolve`/`join` carry an optional `namespace` naming the field the code was typed into; absent means `client`, so a page built before #1114 is unchanged. It is also the fallback a bare five-letter suffix is composed under, which is what stops the fleet field resolving a crew record holding the same word. It is the **field's** namespace, never the parsed code's: a structured code names its own namespace, so echoing that would make the service agree with the asker by construction. The joiner refuses the disagreement locally as well, before opening a socket.
- **Admission and freeze.** The operator's Fleet section on the settings cog closes and reopens new-host admission; closing tells admitted hosts and disconnects nobody. Mission start freezes the roster and every selected hull, and releases the service-side gate so a recovery request can reach the owner. A player-ship slot follows #1120's fixed-slot recovery contract. A first-time GM instead stays a private candidate until an existing host or GM visibly accepts it. Its provisional topology enters Rust only as `GmJoinBootstrap`: it can build the candidate's matching world but installs neither authoritative `FleetRoster` nor `FleetLockstep`, so it is absent from every wait-set. The technical owner then schedules one synchronized pause, and the candidate restores the canonical snapshot plus complete command/GM history and reports the matching digest. A successful restore immediately engages the candidate's technical `GmJoinPauseHold`, including when it restored from behind the pause boundary. Only `GmJoinCommit` installs the roster and lockstep membership; the hold remains paused until an explicit later GM Resume. An accepted candidate disconnect or authenticated restore failure becomes a fleet-visible typed refusal. A stalled not-ready restore advances only through authenticated candidate requests and owner grants on the bounded `RestoreBoundary` protocol clock: duplicate, stale, and render-only updates cannot advance it, so unequal frame cadences reach the same visible terminal refusal. Reliable ordered relay also forwards an authenticated owner-carried `host-loss` to the private candidate: Rust keeps it in non-authoritative `GmJoinPendingHostLoss`, so it changes neither snapshot proof nor admission, then Commit drains it exactly once into `PendingHostLoss` and removes that slot from the newly installed wait-set before Resume. A disconnect callback racing a retained Commit re-emits that same terminal proof rather than deleting it; a candidate lost after Commit takes the ordinary deterministic host-loss path. A disconnected known GM may reclaim its identity while the fleet is still mutable, with readiness cleared. After freeze, only a capability naming the exact operator bound to the exact departed technical slot auto-pends a bounded `Reconnect` transaction; the public row stays disconnected until matching-digest Commit. The returning process is private and ineligible as a canonical/election source even when it is the only other peer. Its Pause freezes and arms the private restore without entering GameStart; after the canonical record passes the shared gate, `MeshRestoreArm` stages the saved authored-row/UUID map through `stage_resume_game_start_entity_uuids` and only then requests `InProgress`. The bounded clock begins after that exact GameStart walk completes. Commit preserves the frozen row and calls `LockstepSession::rejoin`; live, mismatched, corrupt, and competing attempts remain outside commands and the wait-set.
- **A slot's hull.** `server.html`'s `lockPlayerHull()` wraps `wasm_select_ship` — the one call every route to a running mission makes — so the roster carries the hull the operator actually chose and mission start freezes something real. `refused` names which frame it answers, so a slot patch refused after the freeze is a notice rather than an eviction.
- **Where it shows.** `#fleet-panel` under the crew QR on each server page shows ship slots and a separate equal-GM group; the invitation link and QR appear on the issuing host only. Rust publishes the validated GM projection in `Welcome`, `GmRosterChanged`, and the host lobby channel, so crew and viewscreen lobbies render the same connected/disconnected GM presence without turning it into a Station or Spectator row.
- **Collective start.** Each ship publishes one bounded connected/ready crew tally and each GM publishes its own ready flag. The owner combines those authenticated facts, excludes Spectators and disconnected rows, and proposes one immutable `start-N` only after every participant is ready and every host has completed content, compatibility, and terminal presentation-preload validation. A GM force request is authenticated by its connection and skips readiness only. The proposal's `apply_tick` is zero; owner-side Rust assigns the first safe exact tick beyond the bearing frame, freezes validation, and embeds the grant in that authenticated frame. Members adopt that exact value, and all peers enter `InProgress` at its tick. Conflicting, stale, forged, or unsafe grants fail closed. The owner orders this decision but has no extra product permission.
- **Adoption and leaving are acknowledged edges.** `wasm_join_fleet` returns a generation, not success. Until `wasm_fleet_join_status` accepts that exact generation, the browser withholds the frozen topology and grants and holds at most 64 reliable simulation frames. Managed/validation retries are bounded and coalesce only absolute projections while retaining transition edges. A fresh-Lobby `wasm_leave_fleet` uses the same exact-generation status; the browser does not close its transport or clear the adopted roster until Rust accepts, and a late phase-boundary refusal preserves the live fleet. A leave waits behind an in-flight join so one status generation cannot eclipse another.
- **Canonical activation.** Multi-participant peers may arrive with different browser ticks and OS-seeded RNGs, so a successful fresh-Lobby adoption rebases all of them to `FLEET_ACTIVATION_TICK` 1: `SimTick = 1`, fixed elapsed is one authored timestep with zero overstep, `SimRng` restarts from the authored world seed, and `WorldIdMint` begins tick 1. Host-only pause/instagib state is neutralised and its raw mutation paths are refused while lockstep is installed. `discard_multi_participant_overstep` permits at most one complete fixed step per render frame, discarding only whole catch-up debt and retaining the fractional interpolation remainder. Primary authored rig sidecars are bound into the frozen content identity and resolve simulation-owned `ModelMarkers` before activation in every authoritative profile; active rendering and LOD consume that geometry but remain presentation-only.
- **Tests.** `tests/client/host-mesh.test.js` (envelope + fleet model), `tests/client/fleet-session.test.js` (both ends over the real registry), `tests/smoke/fleet-lobby.spec.js` (two WASM host pages), `tests/smoke/fleet-start-policy.spec.js` (collective start), and `tests/smoke/fleet-lockstep.spec.js` (wire plus leave/rejoin lifecycle).

## Running-mission host-mesh frames (issues #1116, #1117, #1118)

Once the roster freezes and the mission runs, the same `{ m, t, tick, d }` wire carries three more frame types — minted and read by Rust (`src/lockstep/`), ferried unread by `gui/host-mesh.js` (`HOST_SIMULATION_FRAME_TYPES`), and encoded in `src/core/codec.rs` because AGENTS.md keeps `serde_json` there. `MeshFrame` is the closed Rust enum; `decodeHostFrame` refuses any `t` it does not know so a mixed-build fleet fails loudly.

- **`tick`** (#1116) — one host's crew input for a tick plus a `ready_through` watermark; the lockstep barrier withholds a tick until every peer is ready through it. Ordered by the peer-independent `CommandOrder`.
- **`digest`** (#1116) — a sampled authoritative fold, exchanged periodically so a divergence is named at the tick it happened. A u64 crossing as a hex string (JSON numbers round above 2^53).
- **`snapshot`** (#1117) — one framed piece of the **portable authoritative record**. The transfer is the transport half of `p2p-delta-snapshot-is-whole-payload-ron`: chunk / reassemble / integrity around the ONE `PhoenixSnapshot` record `src/snapshot.rs` already produces, no second serializer.
  - **Pure protocol:** `src/lockstep/transfer.rs`. `chunk()` splits the RON export (`snapshot::export_artifact`) on UTF-8 boundaries into `<= SNAPSHOT_CHUNK_BYTES` pieces; `SnapshotReceiver` bounds the buffer (a transfer over `SNAPSHOT_MAX_CHUNKS` is refused up front), catches a damaged chunk on arrival with a per-chunk `crc32_ieee`, catches a mis-assembled whole with a payload-wide `fnv1a`, and names a dropped chunk as a distinct `Incomplete` gap.
  - **Adapter + gate:** `src/lockstep/snapshot_relay.rs`. `capture_run`/`frames_for` reuse `snapshot::{capture,run_for,export_artifact}`; `gate_and_restore` reuses `snapshot::import_artifact` (the `Versions::check` build/rules/content-digest gate, run **before** any state is written) then `snapshot::restore` then the `world_digest` post-restore corruption check — the same seams a local file import uses. **Every** refusal now leaves the receiving world byte-identical (issue #1118 closed the asymmetry): the version/content gate still runs before a single component is written, and an integrity/incomplete refusal decided *after* `snapshot::restore` restores a checkpoint captured just before the overwrite, so the world is put back rather than left half-restored.
  - **Isolation (AC4).** The store rides whole; what a crew SEES is gated by the `LocalShip`-scoped projection layer (`src/dossier/server.rs`). The projection-SCOPE marker (`LocalShip`) is host-local and never in the record. The world-global dossier blackboard DOES cross (capture takes `ShipSystemBlackboards` unfiltered) but is inert on a non-local hull — restore is by-uuid so it lands only on the same hull it left, that hull is not the receiver's local ship, and the blackboards are not folded into `world_digest` — so a restore cannot re-point what a crew sees. True per-ship evidence KEYING is open PRD #1016's (`p2p-delta-per-ship-epistemics`).

**Divergence recovery (#1118)** turns the `digest` detection into a heal, in `src/lockstep/recovery.rs` (Bevy adapter) over the Bevy-free decision in `src/lockstep/recovery_plan.rs`. When the exchange names a real fold split, every host derives the SAME plan from the shared ledgers — the divergence tick, the majority-canonical **leader** (lowest slot in the strict-majority fold group; no majority ⇒ no safe leader ⇒ clean failure), and a **boundary tick** past the detection horizon — and a `RecoveryHold` withholds every tick past the boundary beside the barrier's own peer-stall. The leader transfers its `snapshot` record at the boundary; the divergent host is **armed** (`MeshRestoreArm`) to accept a record only during a recovery it is party to and only from that leader — `drain_mesh_restore` drops anything else, so a peer can never overwrite another host's world by sending it a snapshot. The divergent host resumes only once the restore folds to the leader's state; canonical hosts resume once its watermark advances past the boundary. Each host retains a `RecoveryDiagnostic` (command window, per-slot digests, leader, boundary, result) in `RecoveryLog`, JSON-exportable and version-guarded by `RECOVERY_ARTIFACT_VERSION`. The determinism argument is that every decision reads only shared digest-exchange state, so all hosts compute the identical plan; the barrier still governs which ticks actually run.
- **Tests.** `tests/lockstep_mesh.rs` (two hosts, one mission, per-tick agreement), `tests/lockstep_snapshot_transfer.rs` (capture → chunk → mesh → reassemble → gate → restore → 120-frame continuation agreement, plus dropped/corrupt/wrong-build fault injection), `tests/lockstep_recovery.rs` (three hosts, one corrupted mid-mission: same-leader election, boundary hold, restore, bit-identical reconvergence; a two-host split refused cleanly; the receiver arm dropping an unarmed and a non-leader record), the pure suites in `src/lockstep/transfer.rs`, `src/lockstep/recovery_plan.rs` and `src/lockstep/recovery.rs`, and the codec/JS round-trips in `src/core/codec.rs` + `tests/client/host-mesh.test.js`.

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

**A retryable failure is retried whether or not the host has accepted this build
yet**, so the timeout ladder is reachable on a *first* join — which is the case
it exists for, TURN-over-TCP allocation on cellular. Before acceptance the loop
is bounded at `JOIN_ATTEMPTS_BEFORE_ENTRY` (4) attempts, after which the entry
field comes back with the reason, because a guest who has never got in may
simply be reading the wrong five letters. After acceptance it is unbounded.

Only a refusal a retry cannot fix ends the loop early and goes back to the entry
field with its own sentence: an answer about the *code* (unknown, wrong-type,
version-mismatch, admission-closed, host-gone) or about the *build* (the host's
`StampMismatch` codes). Everything else — an unreachable service, a signalling
drop, an ICE timeout — is retried. `reconnect-midgame-sever.spec.js` exercises
sever and revive during play.

**Signalling and media are independent planes.** Once the two DataChannels are
up, the rendezvous socket is expendable: a `closed` or `error` frame, or the
socket itself dying, is logged and ignored while the link is live. A `closed`
frame is a statement about the *record* (its host socket dropped, or the TTL
sweep took it — `dropRecord` in the registry), not about a host whose direct
link is carrying the game. Only the DataChannel's own close drives the reconnect
loop.

The host has a matching loop, and the same separation: a lost record
re-registers on a backoff — while **every admitted crew connection stays up**.
Only the peers still mid-signalling on the dead socket are discarded; an
established `RTCPeerConnection` needs no service, and only its own channel
closing reaches `wasm_player_disconnected`. The same rule holds per peer, not
just for a dead host socket: `peer-left` — sent when ONE joiner's own
rendezvous WebSocket dies (registry.js's `leave()`) — never closes that
joiner's admitted, still-open link either; only that joiner's own DataChannel
closing does.

**Code reclaim across a transient loss (issue #1115).** A host's socket merely
dying no longer frees its record on the spot. `hostOpen` mints every record a
one-time reclaim secret (never sent to a joiner); a socket that dies WITHOUT an
explicit `host-close` frame holds the record in *grace* for the authored
`[limits] reclaim_grace_seconds` (120s) instead of dropping it — un-hostable in
the meantime (a `join` against it answers the retryable `unreachable`, which a
mid-reconnect joiner's own backoff already treats as transient) but otherwise
untouched: same suffix, same secret, same admission state, same presence. The
re-registration above presents that secret, and gets the SAME code back —
`createRendezvousHost` holds it across `lostService()` in `resumeToken`,
independent of the `code` field the panel paints from. A wrong secret burns the
grace-held record outright (denying further guesses) rather than merely
falling through; nobody reclaiming it before the deadline, or a genuine Durable
Object eviction (which loses the secret along with everything else — there is
still no persistent store backing this), is what finally mints a fresh suffix.
An EXPLICIT `host-close` still drops a record for real and immediately, same
as always — a deliberate teardown has nothing to reclaim.

**Explicit rotation (issue #1115 AC2/AC3)** is a separate lever: a `rotate`
frame from a record's own LIVE host mints it a brand-new suffix in place,
dropping the old one for good immediately (stale lookups answer `unknown` from
the very next frame) with a fresh secret. The registry has no notion of
GamePhase, so "only between missions" (`gui/phase-toggle.js`'s
`codesRotatable`: Lobby/GameOver rotatable, Loading/InProgress refused) is
enforced by server.html before the frame is ever sent, not by the registry or
the transport. The lever lives on the settings cog beside each code's readout
— `gui/server-settings.js`'s Join Code section for the crew code,
`__hostFleetRotate` for the fleet lead's own — and is a pure passthrough onto
`createRendezvousHost`'s `rotate()`/`createFleetOwner`'s `rotate()`, which
carries no GamePhase awareness of its own either.

`connectionAdapter.close()` — the host's only eviction mechanism, used by the
reserved-token refusal and the duplicate-token dance — closes **both** channels
and the `RTCPeerConnection`, and marks the peer refused so anything still in
flight on either channel is dropped rather than delivered.

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
| `connecting` | green (no label) | A join attempt is in flight and the host has never accepted this build — re-reported on each of the bounded pre-acceptance retries |
| `ready` | green (no label) | The host accepted this build (`JoinAccepted`), or a code was issued |
| `disconnected` | red + "Disconnected — reconnecting…" | An accepted link dropped **and a retry is scheduled** |
| `error` | red + "Error — refresh to retry" | The loop has stopped: a terminal answer, or a pre-acceptance link failure that used up its bounded attempts |

`connecting` is deliberately **not** re-reported on each reconnect attempt: an
accepted link that is down reads as "reconnecting" until it is actually back,
rather than flashing the dot green several times a second. A guest who has never
been accepted stays on `connecting` across their bounded retries, for the same
reason in reverse — "Disconnected — reconnecting…" would describe a connection
they never had.

The two rows are decided by which branch of `fail()` runs, not by whether the
link was ever accepted: **`disconnected` is only ever reported alongside a
scheduled retry**, so the page can never show "reconnecting…" for a loop that
has stopped.

## ICE servers, TURN relay, and on-device diagnostics (2026-08 hotspot fix)

The base ICE list (`defaultIceServers()`) is **STUN-only**; TURN relay credentials come primarily from the Cloudflare worker (`worker/`, deployed per-target as `phoenix-turn-credentials` / `phoenix-turn-credentials-demo`). If the worker is unreachable, `fetchIceServers()` falls back to Metered's shared OpenRelay TURN (`openRelayFallbackServers()`) and reports `relaySource: 'openrelay'`, which both pages surface as a mild fallback notice. OpenRelay uses `staticauth.openrelay.metered.ca`.

**Relay is mandatory on hotspot/CGNAT networks** (phone tethering, two phones on separate mobile data): STUN hairpinning through carrier NAT essentially never works, and host candidates are mDNS-obfuscated. Three things therefore surface degraded relay instead of failing silently:

- `fetchIceServers()` returns `{ servers, relayAvailable, relaySource }`; both pages show a warning (`client.diag_no_relay` / `server.no_relay_warning`) when `relayAvailable` is false, and a milder notice (`client.diag_relay_fallback` / `server.relay_fallback_notice`) when running on the shared OpenRelay fallback.
- The client join screen shows a live diagnostics readout (`#conn-diag`): relay probe verdict (`probeTurnRelay()`, an `iceTransportPolicy:'relay'` throwaway connection) plus per-attempt ICE state and gathered candidate types, fed by the transport's `onDiag` events. "candidates: host, srflx" with no relay on a failing network is the TURN smoking gun.
- The host lobby mirrors any inbound client stuck mid-ICE under the QR code, fed by `createRendezvousHost`'s `onPeerIce` callback — when ICE never completes no channel ever opens, so nothing else would report it.

The per-attempt connect timeout escalates 8s → 16s → 30s (`connectTimeoutMs()`) since TURN-over-TCP allocation on cellular can exceed the old flat 8s — and because a pre-acceptance failure is retried too, the 16s and 30s rungs are reachable by the guest joining for the first time, which is the case the ladder was added for.

Both workers validate CORS against the comma-separated `ALLOWED_ORIGIN` list in their own `wrangler.toml`. The deployed value only changes after `wrangler deploy`; an incorrect allowlist blocks the TURN credential fetch (removing relay) or the rendezvous upgrade (removing joining altogether).

## The transport ladder (issue #1113)

Three ways onto the wire, tried in order, each a fallback for the last:

| Rung | What it is | When it wins |
| --- | --- | --- |
| `direct` | WebRTC over host/srflx candidates | LAN, or any workable NAT |
| TURN | WebRTC over a relay candidate from the credential worker | CGNAT, hotspots, mobile data |
| `ws-relay` | the game's own frames over the rendezvous WebSocket | nothing else could be built |

**The load-bearing rule is that the third rung does not fork the game protocol.** A relayed payload is the same string the DataChannel would have carried — same `ClientMessage`/`ServerMessage` JSON, same in-band compatibility handshake, same `Identify` gate, same `localiseTree` ingress. It is *enforced* rather than intended: `gui/rendezvous-relay.js` hands back objects shaped like `RTCDataChannel`, and `gui/rendezvous-transport.js` wires them through the same `connectionAdapter` and the same `attachReliableChannel` (hoisted out of `pc.ondatachannel` for exactly this) it wires a real channel through. There is one admission path, and the fallback is a different pair of channels on it.

- **Service side:** `worker-rendezvous/src/relay.js` is a bounded mailbox hub, `registry.js` gains four additive verbs (`relay-open`, `relay`, `relay-close`, plus `relay-peer`/`relay-peer-left`/`relay-ready`/`relay-closed`/`relay-degraded` outbound). The delivery classes stay distinct because that is a transport property: **snapshot sheds oldest-first** at the authored depth, **reliable never sheds** and a full queue ends the session with `relay-overflow` instead. A Durable Object exposes no `bufferedAmount`, so the service's own bound bites only while a target socket cannot take bytes (`setWritable`, driven from `index.js`); the backpressure half that meets a real backlog belongs to whoever is *sending*, in `gui/rendezvous-relay.js` and the native host, both of which do have that signal. Every bound is authored in `[limits]` in `assets/join/join-codes.toml` and advertised to both ends.
- **Escalation:** the joiner falls back after the direct ladder is spent (four attempts, `JOIN_ATTEMPTS_BEFORE_ENTRY`), starts a fresh ladder on the new rung, and never goes back — a network that refused WebRTC four times is not one to keep re-asking mid-mission. It also skips straight to the relay when the host's `joined` frame carries `transports` without `webrtc`.
- **Two planes, and the exception:** #1112's rule is that a signalling event may not touch an established link, because a DataChannel owes the service nothing once it is up. A *relayed* link owes it everything, so for that peer the planes collapse back into one — `socketIsTheLink()` on the joiner and `entry.relay` on the host are the two places that exception is written down.
- **Levers:** `gui/transport-levers.js` pins one rung, and `?transport=direct|turn|ws-relay|auto` names all four. The two WebRTC pins are mirror images, and both are applied to *both* ends because ICE only negotiates a relayed pair when both offer relay candidates — and can only avoid one when neither has a TURN server to allocate from. `?forceRelay=1` (= `?transport=turn`) is TURN-only: `iceTransportPolicy: 'relay'`, server list kept. `?transport=direct` is its opposite: the server list is **withheld** (`useIceServers: false`), because `iceTransportPolicy` has no `'no-relay'` value and giving direct `'all'` — as it did until #1113's review — left ICE free to select a relayed pair, so the one lever whose job is proving a direct link proved nothing. Every value can only make the transport try *less*, which is what makes a URL lever safe to ship, and a pinned transport is named on both readouts.
- **Native hosts** (`phoenix-host --world … --rendezvous <URL> --origin <URL>`) reach their crew this way and no other: a Rust process has no WebRTC. It registers `transports: ["ws-relay"]` so joiners skip the direct ladder. `src/core/rendezvous.rs` is the Rust frame vocabulary, `src/native_host/relay_transport.rs` the protocol, `relay_socket.rs` the `tungstenite` socket. `tests/native_relay_protocol.rs` pins the Rust and JavaScript vocabularies together by reading the JS source, because nothing else can catch the two drifting.

## Connection diagnostics (`gui/connection-diagnostics.js`)

The `#conn-diag` readout on both pages, and the copy-pasteable dump behind the button beside it. `gui/page-chrome.js` still owns only the mechanical half (guard, join, set `textContent`) and there are still two line builders — the host summarises every connected phone, the client summarises one device — but the state behind them is one object with two readers, so the dump and the screen cannot disagree.

What it says, and why each line is actionable: no relay at all (mobile networks will fail; §3 of the delivery checklist), the free shared fallback (the credential worker is unreachable), *carried by the join service* (this network blocks direct links; expect latency), *shedding snapshot updates* (the link cannot keep up; commands still land), and *pinned by a lever* (the restriction is deliberate, so a failure here may not be the network's fault). `readSelectedPair()` in `gui/connection-manager.js` reads the candidate pair ICE actually chose out of `getStats()` — the candidate *list* says what was offered, and on a hotspot the difference between a server-reflexive pair and a relayed one is the difference between a working network and a working credential worker.

`scripts/check-rendezvous.mjs` is the deployed half of the same question: §3 and §3a's curl recipes as code, including the check no human thinks to make — that an origin which is *not* ours is refused, because `ALLOWED_ORIGIN = "*"` passes every other test. `docs/acceptance/1113-networks.md` is the field script that starts by running it.

## Why almost no backend

- Game data is peer-to-peer. Only the WebRTC handshake touches a signalling service — Phoenix's own rendezvous Worker, which holds transport metadata and nothing else.
- Hosting is GitHub Pages / Cloudflare Pages — pure static. No game state lives on the Worker; a private join code never enters simulation state, a snapshot, or another host's state.
- Self-hosted PeerJS was an explicit out-of-scope item from PRD #1. PRD #1093 reversed that by *replacing* PeerJS rather than self-hosting it, which issue #1112 completed.

## Smoke testing without WebRTC

CI has no real WebRTC and no deployed worker, so the Playwright suite installs one stand-in before any page script runs (`addInitScript`): `tests/smoke/rendezvous-shim.js`, published as `window.PhoenixTransportFactories`, which `gui/rendezvous-transport.js` takes its socket and peer from. It fakes a WebSocket onto the **real** `worker-rendezvous` registry running inside the host page, and an `RTCPeerConnection` that pairs two pages' DataChannels over a `BroadcastChannel`, honouring the `createDataChannel` init bag so the lossy channel is distinguishable from the reliable one. Only the transport is faked, never the protocol.

It also carries `window.__wasmReady` (set once the host page has both opened its rendezvous socket and dispatched `PhoenixReady`), `window.__transportShim.sever()/revive()`, a page-scoped kill switch for the "phone's radio slept" failure, and `peerConfigs()`/`dataChannels()` for inspecting what the transport asked for.

`tests/smoke/transport-paths.spec.js` (issue #1113) drives all four paths through it, and its header is explicit about which are real and which are shim-level: direct and `ws-relay` are genuinely end to end (the second over the real registry's real relay hub), reconnect is real, and **TURN-only is shim-level** — the fake peer connection has no ICE to restrict, so what is proved is that the lever reaches both peer connections, not that a TURN allocation succeeds. That last one is `docs/acceptance/1113-networks.md` scenario 3's, against the deployed service, and nothing in CI can stand in for it.

`tests/smoke/transport-fixture.js` is the single seam every transport assumption lives behind — `fixtures.js` and the ~40 specs importing `readHostPeerId`/`createTestClient` know nothing about it. See `tests/smoke/transport-shim.spec.js` (the stand-in itself), `rendezvous-join.spec.js` (typed join, QR link, distinct refusals), `multi-client-crew.spec.js` (four phones on one code) and `snapshot-channel.spec.js` (the delivery-class split and its per-token fallback), plus [Testing Strategy](./testing-strategy.md).

## Related

- [Architecture](./architecture.md) · [Message Flow](./message-flow.md)
- [Player](../entities/player.md) · [Session](../entities/session.md)
