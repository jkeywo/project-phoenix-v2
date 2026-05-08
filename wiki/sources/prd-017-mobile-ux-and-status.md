---
title: PRD #17 — Mobile UX, Canvas Resize, and PeerJS Connection Status
type: source
tags: [prd, mobile, ux, peerjs, resize, status]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/17
status: closed (2026-05-04)
updated: 2026-05-08
---

# PRD #17 — Mobile UX, Canvas Resize, and Connection Status

Three targeted UX fixes after PRD #1's PoC.

## Problems

1. No fullscreen toggle on phones — browser chrome ate console UI.
2. The Bevy lobby UI used world-space `Text2d` and didn't scale with the canvas — tiny text in the centre on a TV.
3. PeerJS connection state was opaque to both host and players.

## Solution

1. **Fullscreen button** in the top-right of both pages (`⛶` ↔ `✕`). Always visible, silent on iOS Safari where the API is unavailable.
2. **Bevy node-based UI** for the server lobby — full-screen flex container, top-left panel, "Bridge Crew" title, player list. Scales correctly with `fit_canvas_to_parent: true`.
3. **Connection status indicator** — coloured dot + label in the top-right of both pages.

## Key decisions

- **Status state machine:** `connecting` (green, no label), `ready` (green, no label), `disconnected` (red + "Disconnected — reconnecting…", auto-`peer.reconnect()`), `error` (red + "Error — refresh to retry").
- **`peer.reconnect()` not `new Peer()`** — reuses the original peer ID so the QR URL stays valid and avoids the broker's rate-limit on repeat registrations.
- **Status bar is HTML/CSS/JS only** — no Rust/WASM bridge changes.
- **QR code moved bottom-right** so the top-right is free for status + fullscreen.
- **No mobile detection** — fullscreen button always rendered.

## Out of scope

iOS Safari fullscreen quirks; recovery from a destroyed peer without page refresh; client.html → Bevy/WASM migration.

## Cross-references

- Concept: [Networking](../concepts/networking.md) (status state machine), [Architecture](../concepts/architecture.md)
- Entity: [Player](../entities/player.md)
