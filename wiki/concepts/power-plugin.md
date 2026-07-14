---
title: PowerPlugin
---

# PowerPlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#254](https://github.com/jkeywo/project-phoenix-v2/issues/254)).

## Ownership

`PowerPlugin` owns all power-allocation state and message handling for the Power console. It does **not** mutate `ShipModifiers` directly — modifier writes flow through `translate_power_modifiers` in `ModifierCoordinationPlugin` (runs after the power systems each frame).

## Per-entity migration (PRD #597 PR 6)

After PR 6 of PRD #597 (2026-07-02), `ShipPowerSystem`, `PowerConfigResource`, `PowerAiConfigResource`, and `PowerMultiplierResource` all derive both `Resource` and `Component`. Every ship entity (player and NPC alike) carries a per-entity `ShipPowerSystem`; the player ship also carries per-entity power config components populated from the ship TOML.

`operate_power_ai` and `tick_power_system` now iterate ALL ships (`Query<..., With<Ship>>`) rather than being LocalShip-scoped — NPCs tick their own power with the same code path as the player. Player-facing readers (`handle_power_messages`, `handle_power_inter_system`, `publish_power_blackboard`, `power_state_broadcaster`) remain LocalShip-scoped but prefer the per-entity component with a Resource fallback for tests.

### Systems

| System | Responsibility |
|---|---|
| `handle_power_messages` | Processes `IncreasePower` / `DecreasePower` from the Power console holder; forwards to `PowerSystem::increase` / `decrease` which enforce 6-base + 2-battery cap and exhaustion lock |
| `tick_power_system` | Advances battery charge each frame; triggers exhaustion lock when charge reaches zero, re-engages at recharge threshold |

### Resources

| Resource | Purpose |
|---|---|
| `ShipPowerSystem` | Wraps the pure-Rust `PowerSystem` (helm / weapons / sensors levels 1–4, battery charge, locked flag) |
| `PowerConfigResource` | Config for battery drain/recharge rates; defaults from `PowerConfig::default()` |
| `PowerMultiplierResource` | Per-console `[f32; 4]` bonus arrays indexed by power level (1→index 0 … 4→index 3); defaults to `[-0.5, 0.0, 0.25, 0.5]` for Helm, Tactical, Sensors |

## Registration

```rust
.add_plugins(crate::power_plugin::PowerPlugin)
```

Registered by `add_simulation_plugins()` in `src/server_app.rs`. The module is declared in `src/lib.rs`.

## Broadcaster

`power_state_broadcaster()` (defined in `power_plugin.rs` and registered by `PowerPlugin`) reads:
- `ShipPowerSystem` — for current levels, battery charge, and locked flag

Produces `ServerMessage::PowerState` sent to the Power console holder at 10 Hz.

## Modifier Coordination

Power levels do **not** write to `ShipModifiers` directly. The coordinator system `translate_power_modifiers` (in `src/modifiers/coordination.rs`) reads `ShipPowerSystem` + `PowerMultiplierResource` and writes the resulting `Modifier` entries. It is scheduled with `.after(handle_power_messages).after(tick_power_system)` to ensure it always sees the latest power state.

## Tests

Tests live in `src/power_plugin.rs` under `#[cfg(test)] mod tests`.

| Test | Behaviour verified |
|---|---|
| `non_power_sender_increase_power_is_ignored` | Non-Power holder sending `IncreasePower` is a no-op |
| `non_power_sender_decrease_power_is_ignored` | Non-Power holder sending `DecreasePower` is a no-op |
| `power_sender_increase_reflected_in_next_power_state` | Power holder increasing Helm shows helm=3 in next `PowerState` |
| `power_sender_decrease_reflected_in_next_power_state` | Power holder decreasing Tactical shows weapons=1 in next `PowerState` |
| `power_state_only_sent_to_power_holder` | All `PowerState` messages are targeted to the Power console holder |
| `no_power_station_holder_no_power_state_broadcast` | No `PowerState` sent when no player holds the Power station (issue #618 rename) |
| `sim_state_includes_power_levels` | `SimState.power_levels` reflects the current power system state |
| `power_increase_respects_bounds_noop_at_four` | Increasing past level 4 is a no-op |
| `increasing_helm_power_updates_max_speed_via_modifiers` | Helm power 3 with custom multipliers yields the expected MaxSpeed modifier |
| `decreasing_weapons_power_updates_phaser_damage_via_modifiers` | Weapons power 1 with default multipliers yields the expected PhaserDamage modifier |
| `exhaustion_forces_consoles_to_one_and_updates_all_modifiers` | Battery exhaustion locks all consoles to level 1 and updates all three modifier slots |
| `power_increase_respects_total_cap_of_eight` | Total allocation cap of 8 (6 base + 2 battery) is enforced |

## Sources

- `src/power_plugin.rs`
- `src/server_app.rs` (ordering constraints within `add_simulation_plugins`)
- `src/modifiers/coordination.rs` (`translate_power_modifiers`)
- Issue [#254](https://github.com/jkeywo/project-phoenix-v2/issues/254)
- [Console UI Authoring Library](./console-ui-library.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Modifier Coordination](./modifier-coordination.md)
