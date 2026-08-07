---
title: PowerPlugin
---

# PowerPlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#254](https://github.com/jkeywo/project-phoenix-v2/issues/254)).

## Ownership

`PowerPlugin` owns all power-allocation state and message handling for the Power console. It does **not** mutate `ShipModifiers` directly — modifier writes flow through `translate_power_modifiers` in `ModifierCoordinationPlugin` (runs after the power systems each frame).

## Per-entity migration (PRD #597 PR 6)

After PR 6 of PRD #597 (2026-07-02), `ShipPowerSystem`, `PowerConfigResource`, `PowerAiConfigResource`, and `PowerMultiplierResource` all derive both `Resource` and `Component`. Every ship entity (player and NPC alike) carries a per-entity `ShipPowerSystem`; the player ship also carries per-entity power config components populated from the ship TOML.

`tick_power_system` iterates ALL ships (`Query<..., With<Ship>>`) rather than being LocalShip-scoped — NPCs tick their own power with the same code path as the player. NPC/AI power reallocation is admission-routed (issue #831): `console_ai::server::ai_power_allocation` decides the group/level from the movement/red-alert rules and emits it as an admitted `SetPowerGroupAllocation` payload through `command_admission::ai_emit::emit_ai_command`, the shared AI-emit helper over the same `validate_and_admit` seam the human path uses. There is no longer a `PowerReactorIntents` component or an `integrate_power_state` adapter — both were deleted. `ship::power::handle_power_messages` is the **single applier**: it consumes admitted `SetPowerGroupAllocation` envelopes (AI and human alike) and calls `PowerSystem::increase` / `decrease`, and is scheduled `.before(...)` nothing but with `ai_power_allocation.before(handle_power_messages)` so the AI's decision is applied the same tick. Player-facing readers (`handle_power_messages`, `handle_power_inter_system`, `publish_power_blackboard`, `power_state_broadcaster`) remain LocalShip-scoped but prefer the per-entity component with a Resource fallback for tests.

### Systems

| System | Responsibility |
|---|---|
| `handle_power_messages` | Single applier for admitted `SetPowerGroupAllocation` payloads (from both the Power console holder and `ai_power_allocation`); forwards to `PowerSystem::increase` / `decrease`, which enforce the 6-base + 2-battery cap against the COMMANDED total |
| `tick_power_system` | Advances battery charge each frame from the EFFECTIVE total, then re-derives which groups their `[power.battery_floor]` is holding down (cut at the floor, released at floor + `battery_floor_release_margin` once no higher rung is engaged), and forwards that edge into `PowerBrownoutState::floors_changed` |
| `tick_power_brownout_advisory` | Sends `CoordinationPayload::PowerBrownout` to Helm / Tactical / Shields for any group above idle while the reserve is draining **or** the group is being held down by its floor; debounced per group and re-armed by the floor edge |

### Resources

| Resource | Purpose |
|---|---|
| `ShipPowerSystem` | Wraps the pure-Rust `PowerSystem` (helm / weapons / shields levels 1–4, battery charge, per-group battery floors) |
| `PowerConfigResource` | Config for battery drain/recharge rates; defaults from `PowerConfig::default()` |
| `PowerMultiplierResource` | Per-console `[f32; 4]` bonus arrays indexed by power level (1→index 0 … 4→index 3); defaults to `[-0.5, 0.0, 0.25, 0.5]` for Helm, Weapons, Shields |

## Registration

```rust
.add_plugins(crate::power_plugin::PowerPlugin)
```

Registered by `add_simulation_plugins()` in `src/server_app.rs`. The module is declared in `src/lib.rs`.

## Broadcaster

`power_state_broadcaster()` (defined in `src/ship/power.rs` and registered by `PowerPlugin`) reads:
- `ShipPowerSystem` — for current EFFECTIVE levels and battery charge, plus `PowerConfigResource` for whether the reserve is draining. (There is no locked flag since issue #952 retired the brownout lock.)

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
| `a_flat_battery_floors_every_group_and_updates_all_modifiers` | A flat battery holds every group at its authored `[power.battery_floor]` level and updates all three modifier slots (issue #952 — no lock) |
| `every_hulls_battery_floors_descend` | Every shipped hull ships helm > weapons > shields floors, each landing on that group's own `min_level`; any floor that can bite requires a positive `battery_floor_release_margin` |
| `every_hulls_fully_floored_total_recharges` | Each hull's fully-floored effective total — floored groups at their clamped `min_level`, and no unfloored group commandable — lands on a strictly positive `rates` rung, so a browned-out ship can always climb back through its release margin |
| `a_human_commanded_destroyer_climbs_back_out_of_its_own_floor_ladder` | A human Power officer spending the full 8-point budget on the shipped destroyer drains, floors helm and weapons, and climbs all the way back past helm's release threshold with every group returning — the ladder releases from the top, so a lower rung cannot re-impose the draw |
| `a_group_crossing_its_floor_does_not_re_advise_every_tick` | The two-threshold floor keeps `PowerBrownout` advisories (and the effective level itself) off a tick-rate flip-flop |
| `authored_floors_take_their_landing_level_from_the_groups_own_min_level` | `[power.battery_floor]` percentages pair with `[power_groups.*] min_level` |
| `console_ai::server::tests::the_battery_floor_ladder_cuts_an_ai_crewed_hull_in_floor_order` | On a shipped hull driven by its own `[power.ai_policy]`, the ladder cuts helm then weapons as the reserve falls, holds the guns until helm's own rung above them releases, and never touches shields |
| `power_increase_respects_total_cap_of_eight` | Total allocation cap of 8 (6 base + 2 battery) is enforced |

## Sources

- `src/ship/power.rs` (`src/power_plugin.rs` is only a `pub use` alias in `src/lib.rs`)
- `src/console_ai/server.rs` (`ai_power_allocation`, admission-routed NPC power)
- `src/server_app.rs` (ordering constraints within `add_simulation_plugins`)
- `src/modifiers/coordination.rs` (`translate_power_modifiers`)
- Issue [#254](https://github.com/jkeywo/project-phoenix-v2/issues/254)
- [Console UI Authoring Library](./console-ui-library.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Modifier Coordination](./modifier-coordination.md)
