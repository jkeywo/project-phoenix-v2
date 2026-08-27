---
title: Power Runtime
type: concept
tags: [power, battery, brownout, modifiers, ai]
sources: [src/ship/power.rs, src/modifiers/power_system.rs, src/console_ai/server.rs, src/server_app/registration.rs, src/modifiers/coordination.rs]
updated: 2026-08-27
---

# Power Runtime

`ShipPowerPlugin` in `src/ship/power.rs` owns the server adapter for authored reactor allocation, battery state, brownout advisories, and power publication. The pure `PowerSystem` state machine lives in `src/modifiers/power_system.rs`.

## Command and tick path

Human Power controls and `ai_power_allocation` emit the same admitted `SetPowerGroupAllocation` payload. `handle_power_messages` is the shared applier. `tick_power_system` advances battery drain/recharge and applies the exhaustion lock; `tick_power_brownout_advisory` sends typed coordination facts when a draining allocation is unsafe.

The primary runtime state is per ship (`ShipPowerSystem`, `PowerConfigResource`, and `PowerMultiplierResource` components). Resource fallbacks remain for isolated fixtures and compatibility paths; production ships use their own authored components.

## Modifier boundary

Power does not write `ShipModifiers` directly. `translate_power_modifiers` in `src/modifiers/coordination.rs` reads the current allocation and authored multipliers, then writes keyed modifiers for the affected domains. This keeps the modifier cache's single-writer contract intact.

`power_state_broadcaster` publishes the LocalShip reactor state at 10 Hz to the holder of the authored `power-reactor` system. The audience is derived from `ShipConfig`, not from a hardcoded station name.

## Related

- [Modifier Coordination](./modifier-coordination.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [AI Ship Unification](./ai-ship-unification.md)
