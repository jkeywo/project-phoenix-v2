---
title: Red Alert Runtime
type: concept
tags: [red-alert, captain, sensors, ai]
sources: [src/console/captain/server.rs, src/ship/state.rs, src/core/messages.rs]
updated: 2026-07-14
---

# Red Alert Runtime

Red Alert is authoritative per-ship state (`ShipRedAlert`). The current command is `ToggleRedAlert`, admitted for the ship's Red Alert system and applied by the Captain console server plugin. Captain AI can emit the same admitted command for a ship it operates.

The active state is published to the relevant console and viewscreen presentation. The design for explicit set-state commands, Sensors target visibility, and required NPC capability coverage belongs in [PASM's Red Alert slice](../../pasm/spec/design/red-alert.yaml).
