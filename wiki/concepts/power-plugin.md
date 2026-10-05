---
title: Power Runtime
type: concept
tags: [power, battery, brownout, modifiers, ai, semantic-actions, feedback]
sources: [crates/phoenix-simulation/src/ship/power.rs, crates/phoenix-simulation/src/modifiers/power_system.rs, crates/phoenix-simulation/src/modifiers/strike_reserve.rs, crates/phoenix-simulation/src/console/weapons/strike.rs, crates/phoenix-simulation/src/snapshot.rs, tests/snapshot_resume.rs, crates/phoenix-simulation/src/console_ai/server.rs, crates/phoenix-simulation/src/server_app/registration.rs, crates/phoenix-simulation/src/modifiers/coordination.rs, crates/phoenix-simulation/src/command_admission/mod.rs, gui/stations/engineering-actions.js, gui/components/ph-power-controls.js, gui/components/ph-strike-reserve.js, gui/action-map.js, assets/entities/dynasty_player_cruiser.toml, pasm/spec/design/dynasty-balance.yaml]
updated: 2026-09-27
---

# Power Runtime

`ShipPowerPlugin` in `crates/phoenix-simulation/src/ship/power.rs` owns the server adapter for authored reactor allocation, battery state, brownout advisories, and power publication. The pure `PowerSystem` state machine lives in `crates/phoenix-simulation/src/modifiers/power_system.rs`.

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

`PowerSystem::capture_continuation` and `restore_continuation` own the saved reactor projection, `PowerState`, in `crates/phoenix-simulation/src/modifiers/power_system.rs`. Snapshot orchestration calls them after resolving the ship identity and retains the modifier rebuild order. The old `snapshot::PowerState` path is a re-export. Fresh-App continuation and next-tick modifier coverage live in `tests/snapshot_resume.rs`.

## Modifier boundary

Power does not write `ShipModifiers` directly. `translate_power_modifiers` in `crates/phoenix-simulation/src/modifiers/coordination.rs` reads the current allocation and authored multipliers, then writes keyed modifiers for the affected domains. This keeps the modifier cache's single-writer contract intact.

`power_state_broadcaster` publishes the LocalShip reactor state at 10 Hz to the holder of the authored `power-reactor` system. The audience is derived from `ShipConfig`, not from a hardcoded station name.

## Related

- [Modifier Coordination](./modifier-coordination.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [AI Ship Unification](./ai-ship-unification.md)

## Strike reserve policy

The authored `strike_reserve` policy in the Dynasty player hull stores explicitly allocated generation in the same reactor continuation. Its authored charging group shares the demand budget; zero charging preserves charge and an empty reserve does not lock Power. Unallocated generation is not captured. Existing allocation controls and Backfill are shared. The entity digest folds strike-reserve reactor continuation, including allocation order and stored charge; other reactor policies retain their existing digest scope.
# Strike boost

`modifiers/strike_reserve.rs` owns pure attack-time accounting. The admitted toggle adapter is `console/weapons/strike.rs`; firing adapters spend after their no-fire gates. Paid beam and torpedo damage bonuses travel with active attacks through `snapshot.rs`, while blaster bolts retain their boosted damage in the existing projectile state. Gunnery and Power expose the same reserve readout; `gui/components/ph-strike-reserve.js` supplies the shared status and Gunnery toggle.

Dynasty's authored Backfill coordinates charging with that same switch: Power charges while boost is off, then yields the charging allocation during a strike. Gunnery re-enables at its authored threshold. The Engines and Steering state machines use `strike_boost_enabled` in their transition guards to move through charge, approach, attack and recovery; these states remain visible in ordinary AI policy inspection. `seed_strike_reserve_facts` supplies own-ship charge and switch observations to Power evaluation and Helm transitions. The full two-peer outcome test is `tests/dynasty_backfill.rs`.

Playable Dynasty tuning lives only in `assets/entities/dynasty_player_cruiser.toml`; the NPC cruiser keeps its separate content. `scripts/balance-dynasty.mjs` evaluates the fixed duel and team matrix using scenario result flags and the simulation limit. The evaluation rule and reproducible evidence are indexed by [the cruiser evaluation guide](../../docs/balance/dynasty-evaluation.md).
