---
title: Red Alert Runtime
type: concept
tags: [red-alert, captain, sensors, ai]
sources: [src/console/captain/server.rs, src/ship/state.rs, src/core/messages.rs]
updated: 2026-07-23
---

# Red Alert Runtime

Red Alert is authoritative per-ship state (`ShipRedAlert`). The command is `SetRedAlert { active }` (issue #748), admitted for the ship's Red Alert system and applied by the Captain console server plugin, which **assigns** the requested state rather than inverting it — so retries, duplicates, and stale-UI commands are idempotent. Captain AI emits the same admitted command for a ship it operates.

The active state is published to the relevant console and viewscreen presentation. The design for explicit set-state commands, Sensors target visibility, and required NPC capability coverage belongs in [PASM's Red Alert slice](../../pasm/spec/design/red-alert.yaml).
