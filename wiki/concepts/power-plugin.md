---
title: PowerPlugin
---

# PowerPlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#254](https://github.com/jkeywo/project-phoenix-v2/issues/254)).

## Ownership

`PowerPlugin` owns all power-allocation state and message handling for the Power console. It does **not** mutate `ShipModifiers` directly — modifier writes flow through `translate_power_modifiers` in `ModifierCoordinationPlugin` (runs after the power systems each frame).

## Per-entity migration (PRD #597 PR 6)

After PR 6 of PRD #597 (2026-07-02), `ShipPowerSystem`, `PowerConfigResource`, `PowerAiConfigResource`, and `PowerMultiplierResource` all derive both `Resource` and `Component`. Every ship entity (player and NPC alike) carries a per-entity `ShipPowerSystem`; the player ship also carries per-entity power config components populated from the ship TOML.

`tick_power_system` iterates ALL ships (`Query<..., With<Ship>>`) rather than being LocalShip-scoped — NPCs tick their own power with the same code path as the player. NPC/AI power reallocation is admission-routed (issue #831): `console_ai::server::ai_power_allocation` decides the group/level from the movement/red-alert rules and emits it as an admitted `SetPowerGroupAllocation` payload through `command_admission::ai_emit::emit_ai_command`, the shared AI-emit helper over the same `validate_and_admit` seam the human path uses. There is no longer a `PowerReactorIntents` component or an `integrate_power_state` adapter — both were deleted. `ship::power::handle_power_messages` is the **single applier**: it consumes admitted `SetPowerGroupAllocation` envelopes (AI and human alike) and calls `PowerSystem::increase` / `decrease`, with `ai_power_allocation.before(handle_power_messages)` so the AI's decision is applied the same tick. Battery charge is then integrated exclusively by `tick_power_system` from the hull's authored allocation-rate curve; active phaser banks do not apply a second, direct drain. Player-facing readers (`handle_power_messages`, `publish_power_blackboard`, `power_state_broadcaster`) prefer the per-entity component with a Resource fallback for tests.

### Systems

| System | Responsibility |
|---|---|
| `handle_power_messages` | Single applier for admitted `SetPowerGroupAllocation` payloads (from both the Power console holder and `ai_power_allocation`); forwards to `PowerSystem::increase` / `decrease`, which enforce the hull's authored `max_commanded_total` |
| `tick_power_system` | Advances battery charge each frame from the total allocation, then handles the exhaustion lock — at a flat battery it forces every group to 1 and locks, unlocking once the charge recovers past `emergency_threshold` — and forwards the lock-changed edge into `PowerBrownoutState::locked_changed` |
| `tick_power_brownout_advisory` | Sends `CoordinationPayload::PowerBrownout` to Helm / Tactical / Shields for any group above idle while the reserve is draining; debounced per group and re-armed by the lock-changed edge |

### Resources

| Resource | Purpose |
|---|---|
| `ShipPowerSystem` | Wraps the pure-Rust `PowerSystem` (helm / weapons / shields levels 1–4, battery charge, and the exhaustion-lock flag) |
| `PowerConfigResource` | Config for battery drain/recharge rates plus sustainable and maximum commanded totals; defaults from `PowerConfig::default()` |
| `PowerMultiplierResource` | Per-console `[f32; 4]` bonus arrays indexed by power level (1→index 0 … 4→index 3); defaults to `[-0.5, 0.0, 0.25, 0.5]` for Helm, Weapons, Shields |

## Registration

```rust
.add_plugins(crate::power_plugin::PowerPlugin)
```

Registered by `add_simulation_plugins()` in `src/server_app.rs`. The module is declared in `src/lib.rs`.

## Broadcaster

`power_state_broadcaster()` (defined in `src/ship/power.rs` and registered by `PowerPlugin`) reads:
- `ShipPowerSystem` — for current levels, battery charge, and the exhaustion-lock flag, plus `PowerConfigResource` for whether the reserve is draining.

Produces `ServerMessage::PowerState` sent to the Power console holder at 10 Hz.

## Modifier Coordination

Power levels do **not** write to `ShipModifiers` directly. The coordinator system `translate_power_modifiers` (in `src/modifiers/coordination.rs`) reads `ShipPowerSystem` + `PowerMultiplierResource` and writes the resulting `Modifier` entries. It is scheduled with `.after(handle_power_messages).after(tick_power_system)` to ensure it always sees the latest power state.

## Tests

Tests live in `src/ship/power.rs` under `#[cfg(test)] mod tests`.

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
| `a_flat_battery_locks_every_group_to_one_and_updates_all_modifiers` | A flat battery locks the reactor, slams every group to 1, and crushes all three modifier slots to x0.667 |
| `power_system::tests::exhaustion_forces_groups_to_one_and_locks` | Nothing degrades until the battery hits zero, then every group is forced to 1 and the reactor locks in the same instant |
| `power_system::tests::recovery_unlocks_at_the_emergency_threshold` | A locked reactor stays locked (controls frozen) until the charge climbs back to `emergency_threshold`, then unfreezes |
| `power_system::tests::increase_when_locked_is_noop` / `decrease_when_locked_is_noop` | While locked, the allocation controls no-op — the operator cannot spend power the reserve cannot pay for |
| `power_system::tests::alliance_reactor_has_six_free_pips_and_refuses_a_ninth` | Six pips recharge, seven/eight drain, and the authored eight-pip ceiling refuses a ninth |

## Sources

- `src/ship/power.rs` (`src/power_plugin.rs` is only a `pub use` alias in `src/lib.rs`)
- `src/console_ai/server.rs` (`ai_power_allocation`, admission-routed NPC power)
- `src/server_app.rs` (ordering constraints within `add_simulation_plugins`)
- `src/modifiers/coordination.rs` (`translate_power_modifiers`)
- Issue [#254](https://github.com/jkeywo/project-phoenix-v2/issues/254)
- [Console UI Authoring Library](./console-ui-library.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Modifier Coordination](./modifier-coordination.md)
