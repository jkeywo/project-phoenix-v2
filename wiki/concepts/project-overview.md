---
title: Project Overview
type: concept
tags: [overview, intro]
sources: [README.md, AGENTS.md]
updated: 2026-05-08
---

# Project Overview

**Project Phoenix** is a browser-based spaceship bridge simulator for groups.

- One browser tab on a shared screen (TV / monitor) is the **view screen** — a 3D view of space.
- Each player joins from their own phone by **scanning a QR code**. No app install. No network setup.
- The view screen is the authoritative server; phones are stateless spokes connected over **WebRTC (PeerJS)** in a star topology.

Live: https://jkeywo.github.io/project-phoenix-v2/

## Why this exists

Existing bridge sims (e.g. Artemis SBS) require dedicated installs on a shared LAN. Phoenix removes both barriers: open a URL, scan a code, play. The tradeoff is that the simulation has to fit in a browser tab.

## What's in the box today

- **Lobby** with QR code, player list, name editing, console picking, session-token reconnect.
- **Two consoles:** Captain's Chair (Red Alert + view selector) and Helm (thrust + steering + radar).
- **Physics-simulated ship** (Bevy + Rapier) on the XZ plane.
- **Deterministic asteroid field** spawned per session.
- **Hull-camera viewscreen** with four directional views (Fore/Aft/Port/Starboard) + a top-down Radar mode.
- **End-to-end smoke tests** (Playwright + Chromium with a BroadcastChannel PeerJS shim).
- Auto-deploy to GitHub Pages on every push to `main`.

## What's in flight or drafted

- PRD #66 (open) — Weapons + Engineering consoles, Hull Integrity, repair loop.
- Drafts 1–8 in `docs/` — entity config files, multi-system maps, science console, combat update (torpedoes + 4-quadrant shields), ship's power, space stations, scenario files, comms console.
- Architecture note — per-console message subscriptions to reduce traffic as consoles multiply.

See the Roadmap Overview for the synthesis.

## Tech stack at a glance

| Layer | Tech |
|---|---|
| Game engine | Bevy 0.18 (Rust) |
| Physics | bevy_rapier3d 0.33 |
| Networking | PeerJS (WebRTC) |
| Build | Trunk → WASM |
| Hosting | GitHub Pages |
| Smoke tests | Playwright + Chromium |

## Related

- [Architecture](./architecture.md) · [Networking](./networking.md) · [Game Loop](./game-loop.md)
- PRD #1 — original product spec
