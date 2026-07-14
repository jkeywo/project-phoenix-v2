---
title: Red Alert Intent
type: concept
tags: [red-alert, captain, sensors, ai, pasm]
sources: [src/console/captain/server.rs, src/ai/core.rs, src/ship/state.rs, pasm/spec/architecture/red-alert.yaml]
updated: 2026-07-14
---

# Red Alert Intent

Red Alert is a per-ship authoritative state owned by the host simulation. The current code uses a toggle command, while PASM records the agreed target design without changing the runtime yet.

The Phase 7 PASM design slice records Captain and AI alert decisions, selected-Sensors target visibility, Sensors-to-Captain coordination, and the mandatory NPC capability recovery path in `pasm/spec/design/red-alert.yaml`.

## Planned command contract

`SetRedAlert { active: bool }` replaces `ToggleRedAlert`. Captain UI and AI both request an explicit desired state through normal command admission; the host assigns that state to the addressed ship. This makes retries, duplicate messages, and stale displayed state harmless.

## Sensors target visibility

The Sensors console will display `ALERT: RED` or `ALERT: NOMINAL` only for its selected radar target when that target has a Red Alert capability. Non-ship targets and ships without that capability expose no alert value. This avoids making Red Alert a generic marker on every radar blip.

## AI ships

Every behaviour-driven AI ship must have an AI-only `red-alert` system. Spawn logic will add the capability when absent from authored TOML, so Captain AI can operate its already-existing per-ship alert state. Authors may still declare it explicitly.

## Current implementation gap

`ShipRedAlert` is already per ship in `src/ship/state.rs:30`, and the captain AI already derives a desired alert state from recent combat in `src/console/captain/server.rs:128`. The runtime still emits and applies `ToggleRedAlert`, target scan data does not carry alert status, and NPC configurations can omit the Red Alert system.
