---
title: PowerPanelPlugin
---

# PowerPanelPlugin

Extracted from `client/app.rs` as part of the client split series (issue [#259](https://github.com/jkeywo/project-phoenix-v2/issues/259)).

## Location

`src/power_panel.rs` — compiled under the `client` Cargo feature.

Re-exported as `crate::power_panel::PowerPanelPlugin`.

## Ownership

`PowerPanelPlugin` owns all Power console UI:

- Three power allocation rows: Helm, Weapons (Tactical), Sensors
- Increment (`+`) and decrement (`-`) buttons per row, with enabled/disabled colouring
- Power level labels per row (read from `ShipView.power_levels`)
- Overflow allocation controls row (hidden in Low complexity — AI manages points 7–8)
- Battery bar (fill width tracks battery charge percentage)
- Battery percentage label
- Panel visibility toggling (driven by `LobbyState` + `ActiveConsole`)

Apply paths for `PowerState` remain in `ClientSimState.apply()` in `client_sim.rs` since they
update shared simulation state used by multiple consoles.

### Systems

| System | Responsibility |
|---|---|
| `setup_power_ui` | Spawns the full power panel hierarchy under a `PowerPanel` root (hidden on spawn). One-shot; called at `Startup`. |
| `toggle_power_panel_visibility` | Shows/hides `PowerPanel` based on phase, console assignment, and active tab. Delegates the visibility decision to pure `power_panel_visible`. |
| `refresh_power_panel` | Updates battery bar, battery label, row backgrounds (locked/unlocked), level labels, and button colours from `ClientSimState` + `ShipView`. Change-detected. |
| `handle_increase_power` | Emits `IncreasePower { console }` when an increment button is pressed and `can_increase_power` returns true. |
| `handle_decrease_power` | Emits `DecreasePower { console }` when a decrement button is pressed and `can_decrease_power` returns true. |

### Pure helpers

| Function | Signature | Testability |
|---|---|---|
| `power_panel_visible(lobby, token, active) -> bool` | Pure, Bevy-free | Yes — 6 unit tests |

Message builders `increase_power_message()` and `decrease_power_message()` live in `client_sim.rs`
and are imported here. Logic helpers `can_increase_power()`, `can_decrease_power()`,
`battery_percentage()`, and `is_power_locked()` also live in `client_sim.rs`.

### Visibility rules (`power_panel_visible`)

1. Game phase must be `InProgress`.
2. Local player must hold `Console::Power`.
3. If the player holds **one console only**, show automatically (no tab override).
4. If the player holds **multiple consoles**, show only when `ActiveConsole` is
   explicitly set to `Power`.

### Marker components

All defined privately in `power_panel.rs`:

| Component | Purpose |
|---|---|
| `PowerPanel` | Root node; visibility target. (`pub` for cross-module access.) |
| `PowerRow(Console)` | Row container; carries the console it allocates power for. |
| `PowerRowLevel(Console)` | Level text label within a row; carries the console for refresh matching. |
| `PowerIncButton(Console)` | Increment button; carries the target console. |
| `PowerDecButton(Console)` | Decrement button; carries the target console. |
| `BatteryBar` | Battery fill node (width is set to battery charge %). |
| `BatteryLabel` | Battery percentage text label. |

The `HideableElement("power_overflow_controls")` marker is imported from `client_app` and applied
to the overflow row so complexity hiding can suppress it in Low mode.

## Registration

```rust
.add_plugins(crate::power_panel::PowerPanelPlugin)
```

Registered by `wasm_client_init` in `src/client/bridge.rs`, directly after `RepairPanelPlugin`.

## What was removed from `client/app.rs`

As part of issue [#259](https://github.com/jkeywo/project-phoenix-v2/issues/259):

- `PowerPanel`, `PowerRow`, `PowerRowLevel`, `PowerIncButton`, `PowerDecButton`,
  `BatteryBar`, `BatteryLabel` component definitions
- `POWER_COL_INACTIVE`, `POWER_COL_LOCKED`, `POWER_INC_COLOR`, `POWER_INC_LOCKED`,
  `POWER_DEC_COLOR`, `POWER_DEC_LOCKED`, `POWER_BATTERY_BG`, `POWER_BATTERY_FILL` constants
- `setup_power_ui` function and its `Startup` registration
- `toggle_power_panel_visibility`, `refresh_power_panel`,
  `handle_increase_power`, `handle_decrease_power` systems and their `Update` registrations

Apply paths for `PowerState` remain in `client_sim.rs`.

## Tests

Tests live in `src/power_panel.rs` under `#[cfg(test)]`. Run with:

```bash
cargo test --features client power_panel
```

Coverage (6 tests):

- `power_panel_hidden_in_lobby_phase` — panel stays hidden in Lobby
- `power_panel_visible_in_progress_holding_power` — auto-shown with one console
- `power_panel_hidden_when_player_does_not_hold_power` — wrong token → hidden
- `power_panel_visible_when_active_console_is_power_multi_console` — explicit Power tab
- `power_panel_hidden_when_active_console_is_other_multi_console` — non-Power tab
- `power_panel_hidden_when_no_active_console_and_holding_multiple` — auto → hidden when multiple

## Sources

- `src/power_panel.rs`
- `src/client/app.rs` (post-extraction)
- `src/client/bridge.rs`
- `src/client_sim.rs` (message builders + apply paths remain here)
- Issue [#259](https://github.com/jkeywo/project-phoenix-v2/issues/259)
