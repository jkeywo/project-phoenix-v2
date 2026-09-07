---
title: Power Runtime
type: concept
tags: [power, battery, brownout, modifiers, ai, semantic-actions, feedback]
sources: [src/ship/power.rs, src/modifiers/power_system.rs, src/snapshot.rs, tests/snapshot_resume.rs, src/console_ai/server.rs, src/server_app/registration.rs, src/modifiers/coordination.rs, src/command_admission/mod.rs, gui/stations/engineering-actions.js, gui/components/ph-power-controls.js, gui/action-map.js]
updated: 2026-09-07
---

# Power Runtime

`ShipPowerPlugin` in `src/ship/power.rs` owns the server adapter for authored reactor allocation, battery state, brownout advisories, and power publication. The pure `PowerSystem` state machine lives in `src/modifiers/power_system.rs`.

## Command and tick path

Human Power controls and `ai_power_allocation` emit the same admitted `SetPowerGroupAllocation` payload. `handle_power_messages` is the shared applier. `tick_power_system` advances battery drain/recharge and applies the exhaustion lock; `tick_power_brownout_advisory` sends typed coordination facts when a draining allocation is unsafe.

`power.decrease-allocation` and `power.increase-allocation` are the shared
semantic entries for dedicated Power, composite Engineering and Courier Captain.
Pointer controls preserve the selected authored group and absolute requested
level; a binding steps the first operable group from its authoritative
`commanded_level` within its published limits. The family projection carries the
exact authored reactor SystemId into the action map; it is not reconstructed from
the Power family name. A correlated request completes `Applied` only
when `PowerSystem::set_group_allocation` accepts it, otherwise `Refused`; the
client never changes allocation before the next authoritative projection.

The primary runtime state is per ship (`ShipPowerSystem`, `PowerConfigResource`, and `PowerMultiplierResource` components). Resource fallbacks remain for isolated fixtures and compatibility paths; production ships use their own authored components.

`PowerSystem::capture_continuation` and `restore_continuation` own the saved reactor projection, `PowerState`, in `src/modifiers/power_system.rs`. Snapshot orchestration calls them after resolving the ship identity and retains the modifier rebuild order. The old `snapshot::PowerState` path is a re-export. Fresh-App continuation and next-tick modifier coverage live in `tests/snapshot_resume.rs`.

## Modifier boundary

Power does not write `ShipModifiers` directly. `translate_power_modifiers` in `src/modifiers/coordination.rs` reads the current allocation and authored multipliers, then writes keyed modifiers for the affected domains. This keeps the modifier cache's single-writer contract intact.

`power_state_broadcaster` publishes the LocalShip reactor state at 10 Hz to the holder of the authored `power-reactor` system. The audience is derived from `ShipConfig`, not from a hardcoded station name.

## Related

- [Modifier Coordination](./modifier-coordination.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [AI Ship Unification](./ai-ship-unification.md)
