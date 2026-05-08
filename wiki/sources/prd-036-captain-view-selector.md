---
title: PRD #36 — Captain View Selector
type: source
tags: [prd, captain, view, camera, viewscreen]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/36
status: closed (2026-05-07)
updated: 2026-05-08
---

# PRD #36 — Captain View Selector

Gives the captain control of the viewscreen camera direction.

## Problem

The viewscreen was hard-locked to a forward-follow camera. The crew had no way to see threats behind, port, or starboard.

## Solution

Four directional buttons on the captain's console — **fore / aft / port / starboard** — laid out as a compass. Pressing one repositions the viewscreen to a first-person camera looking out from that side of the hull. A top-centre text label on the viewscreen shows the current direction.

## Key decisions

- **`ViewDirection` enum** with `Fore`, `Aft`, `Port`, `Starboard`. Default `Fore`.
- **`SetView { direction }`** added to `ClientMessage`. Captain-only, InProgress-only — silently ignored otherwise.
- **`view_direction`** added to `ShipState` and broadcast in `SimSnapshot` so reconnecting clients restore the correct button highlight.
- **Camera offset = 6.0 units** (capsule radius), looking outward parallel to the ground.
- **Buttons in a 3×3 grid** above the Red Alert button; centre cell is the "View" label.
- **Default `Fore`** so initialisation requires no special-casing.

> Note: a later iteration (visible in `messages.rs:16`) generalised this to `ViewMode::Camera(direction) | ViewMode::Radar`. The Radar variant is not in PRD #36 — see [View Modes](../concepts/view-modes.md).

## Out of scope

Non-captain camera control; animated transitions; diagonal views (e.g. fore-port).

## Cross-references

- Entity: [Captain Console](../entities/captain-console.md)
- Concept: [View Modes](../concepts/view-modes.md)
