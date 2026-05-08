---
title: README.md (project root)
type: source
tags: [readme, user-facing, intro, build]
source_path: README.md
status: shipped
updated: 2026-05-08
---

# README.md

The project's user-facing front door, hosted on the GitHub repo.

## What it covers

- One-paragraph product pitch (browser-based bridge sim, scan QR, no install).
- Live URL: https://jkeywo.github.io/project-phoenix-v2/
- "How to play" — three-line setup, table of consoles and controls.
- Tech stack table (Bevy, bevy_rapier3d, PeerJS, Trunk, GitHub Pages).
- Star-topology architecture diagram (ASCII).
- Local dev commands (`trunk serve`, `trunk serve --config client-trunk.toml --port 8081`).
- Test commands (`cargo test`, then the Playwright smoke flow).
- Project structure tree.

## Audience

Players (top half) and contributors (bottom half).

## Cross-references

This file is the high-level summary the rest of the wiki *unpacks*. See:

- [Project Overview](../concepts/project-overview.md) — the wiki's version of the same story
- [Architecture](../concepts/architecture.md) · [Build & Deployment](../concepts/build-and-deployment.md)
- [Testing Strategy](../concepts/testing-strategy.md)
