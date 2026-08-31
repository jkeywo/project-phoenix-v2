---
title: Red Alert Runtime
type: concept
tags: [red-alert, captain, sensors, ai, action-feedback]
sources: [src/console/captain/server.rs, src/command_admission/mod.rs, src/ship/state.rs, src/core/messages.rs, gui/action-feedback.js, gui/stations/captain-actions.js]
updated: 2026-08-31
---

# Red Alert Runtime

Red Alert is authoritative per-ship state (`ShipRedAlert`). The command is `SetRedAlert { active }` (issue #748), admitted for the ship's Red Alert system and applied by the Captain console server plugin, which **assigns** the requested state rather than inverting it — so retries, duplicates, and stale-UI commands are idempotent. Captain AI emits the same admitted command for a ship it operates.

The active state is published to the relevant console and viewscreen presentation. The design for explicit set-state commands, Sensors target visibility, and required NPC capability coverage belongs in [PASM's Red Alert slice](../../pasm/spec/design/red-alert.yaml).

A human Captain's Red Alert and Weapons Hold activations also have a transient,
correlated presentation lifecycle: `Pressed` → `Pending` → `Applied`/`Refused`, with a
client-local `TimedOut` result when no targeted host response arrives. Admission
retains the opaque correlation only until the due command reaches the Captain
consumer; the consumer returns `Applied` even for an idempotent same-state
success. This metadata is absent from Red Alert gameplay state, payloads,
command logs, mesh traffic, snapshots and replay. Consequently Pending never
paints either control as active: only the published `ShipRedAlert` and
`ShipWeaponsHold` states can do that.
