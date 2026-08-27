---
title: Project Overview
type: concept
tags: [overview, intro]
sources: [README.md, AGENTS.md, assets/scenarios.toml, assets/scenarios.demo.toml]
updated: 2026-08-27
---

# Project Overview

Project Phoenix is a browser-based cooperative spaceship bridge simulator. One
shared host tab runs the authoritative Rust/Bevy simulation and 3D viewscreen.
Players scan its QR code and join from phone browsers; their consoles are pure
HTML, CSS, and JavaScript connected to the host over PeerJS WebRTC.

## Runtime shape

- The host owns game phase, sessions, station tenure, command admission,
  physics, damage, AI, world scripts, objectives, and published state.
- Phones send typed commands and fold targeted or shared snapshots into local
  presentation state. They never simulate outcomes or communicate peer-to-peer
  with each other.
- Session tokens stored by the browser provide reconnect identity; PeerJS ids
  are transport details.
- Human operators and Backfill AI emit the same `ControlSystem` commands. A
  ship's authored station ratings decide which source operates each system.
- Simulation decisions advance on a deterministic fixed tick. AI decision
  cadences are derived from that tick rather than rendered frames or wall time.

## Content

Scenario manifests select a world and the hulls it offers. World TOML provides
the root composition and Rhai scenario script; entity TOML provides ships,
stations, systems, ratings, physics, weapons, rendering, and AI doctrine. Hulls
can expose different direct seats and auxiliary hosted stations, so the server
roster is authoritative and the client mounts consoles dynamically.

The normal catalogue and the curated demo catalogue are separate assets. The
demo build also removes client-reachable debug, pause, and mod-pack upload
routes at compile time while retaining the host's own presentation controls.

## Delivery and testing

The browser host is built to WebAssembly with Trunk. `phoenix-host` can serve a
version-pinned client bundle and scenario catalogue on a LAN, but the browser
host or headless runner still owns simulation authority. CI validates Rust,
client JavaScript, PASM, WASM/smoke rendering, asset performance, and the
ratified Cruiser balance matrix.

## Related

- [Architecture](./architecture.md)
- [Message Flow](./message-flow.md)
- [Stations](./stations.md)
- [World Plugin](./world-plugin.md)
- [Build and Deployment](./build-and-deployment.md)
