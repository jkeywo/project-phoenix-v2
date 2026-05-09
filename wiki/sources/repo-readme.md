---
title: README.md (project root)
type: source
tags: [readme, user-facing, intro, build]
source_path: README.md
status: shipped
updated: 2026-05-09
---

# README.md

The project's user-facing front door, hosted on the GitHub repo.

## What it covers

- One-paragraph product pitch (browser-based bridge sim, scan QR, no install).
- Live URL: https://jkeywo.github.io/project-phoenix-v2/
- "How to play" — setup flow, **four-console table** with controls (Captain, Helm, Tactical, Engineering).
- Tech stack table (Bevy server + client, bevy_rapier3d, PeerJS, Trunk ×2, GitHub Pages).
- Star-topology architecture diagram (ASCII).
- Local dev commands (`trunk serve`, `trunk serve --config client-trunk.toml --port 8081`).
- Test commands (`cargo test`, then the Playwright smoke flow).
- **Updated project structure tree** — now shows all client Rust modules alongside server modules.

## Audience

Players (top half) and contributors (bottom half).

## Cross-references

This file is the high-level summary the rest of the wiki *unpacks*. See:

- [Project Overview](../concepts/project-overview.md) — the wiki's version of the same story
- [Architecture](../concepts/architecture.md) · [Build & Deployment](../concepts/build-and-deployment.md)
- [Testing Strategy](../concepts/testing-strategy.md)
