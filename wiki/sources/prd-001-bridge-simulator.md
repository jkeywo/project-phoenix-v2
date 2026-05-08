---
title: PRD #1 — Project Phoenix Browser-Based Bridge Simulator
type: source
tags: [prd, foundational, lobby, captain, red-alert, peerjs]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/1
status: closed (2026-05-03)
updated: 2026-05-08
---

# PRD #1 — Browser-Based Bridge Simulator

The foundational PRD. Establishes the entire stack and the PoC.

## Problem

Existing bridge sims (e.g. Artemis SBS) need per-player desktop installs on a shared LAN. There is no mainstream sim that lets a group join instantly from their own phones.

## Solution

A view-screen browser tab (`server.html`) shows a shared 3D space view. Players scan a QR on the screen, open `client.html` on their phone, and play. Bevy + WASM on the host; plain HTML on the phones; WebRTC (PeerJS) star topology.

## Key decisions

- **Star topology** with the server as the sole authoritative host peer. Clients never talk to each other.
- **JS owns networking.** Bevy never touches sockets. The bridge passes typed messages.
- **Session tokens (UUIDv4 in `localStorage`)** are identity. PeerJS peer IDs are ephemeral.
- **Console vacancy on disconnect** is immediate, in any phase.
- **Hybrid tick rate.** Discrete events fire immediately; `SimState` broadcasts at 10 Hz.
- **Single Rust crate, two HTML entry points.** One WASM binary potentially loaded by both pages.
- **WebGL2** for broad browser support.
- **PeerJS public broker.** Self-hosting deferred.
- **Codec abstraction** (`MessageCodec`) so the wire format can later swap to binary.

## Scope shipped in PoC

- Lobby with QR, name picker, console picker (CaptainChair only initially).
- Captain → `StartGame`, `ToggleRedAlert`.
- View screen renders a rotating cube that reacts to Red Alert (later replaced by [PRD #22](./prd-022-helm-and-game-world.md)).
- Reconnect with same token auto-restores console.

## Out of scope (then)

Helm/Weapons/Comms/Science/Engineering consoles, ship simulation, self-hosted PeerJS, binary wire format, multiple rooms, auth, mobile-native apps, spectator mode.

→ Many of these became later PRDs (#22, #36, #66) or live in `docs/` drafts.

## Cross-references

- Concept: [Architecture](../concepts/architecture.md), [Networking](../concepts/networking.md), [Codec Seam](../concepts/codec-seam.md), [Game Phases](../concepts/game-phases.md), [Game Loop](../concepts/game-loop.md), [Build & Deployment](../concepts/build-and-deployment.md)
- Entities: [Player](../entities/player.md), [Session](../entities/session.md), [Console](../entities/console.md), [Captain Console](../entities/captain-console.md)
