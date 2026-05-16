---
title: CaptainPanelPlugin
---

# CaptainPanelPlugin

Extracted from `client/app.rs` as part of the client split series (issue [#240](https://github.com/jkeywo/project-phoenix-v2/issues/240)).

## Location

`src/client/phone_border/captain.rs` — compiled under the `client` Cargo feature.

Re-exported as `crate::phone_border::CaptainPanelPlugin` via `src/client/phone_border/mod.rs`.

## Ownership

`CaptainPanelPlugin` owns all captain console UI: the compass dial with direction pad buttons, the rotating needle, and the red alert toggle. It also controls the panel's visibility.

### Systems

| System | Responsibility |
|---|---|
| `spawn_captain_ui` | Spawns the full compass + direction pad + red alert button once `PhoneAssets` are loaded. One-shot (guarded by `CaptainPanelSpawned` resource). |
| `toggle_captain_panel_visibility` | Shows/hides the `CaptainPanel` node based on phase, captaincy, and active console tab. Delegates the decision to pure `captain_panel_visible`. |
| `refresh_dir_highlights` | Updates direction button background + LED to reflect the active `ViewDirection` from `ShipView`. |
| `refresh_red_alert_ui` | Updates the red alert button colour and armed glow pulse from `ShipView.red_alert`. |
| `rotate_needle_by_direction` | Rotates the compass needle to match the current `ViewMode`. |
| `handle_direction_press` | Emits `SetView { Camera(dir) }` when a direction pad button is pressed. |
| `handle_red_alert_press` | Emits `ToggleRedAlert` when the red alert button is pressed. |

### Pure helpers

| Function | Signature | Testability |
|---|---|---|
| `captain_panel_visible(lobby, token, active) -> bool` | Takes `&LobbyState`, `&str`, `&ActiveConsole` — pure, Bevy-free | Yes — unit tested |
| `needle_rotation(dir) -> Quat` | Maps `ViewDirection` → Z-axis rotation | Yes — unit tested |
| `direction_press_message(dir) -> ClientMessage` | Builds `SetView { Camera(dir) }` | Yes — unit tested |

### Visibility rules (`captain_panel_visible`)

1. Game phase must be `InProgress`.
2. Local player must hold `Console::CaptainChair`.
3. If the player holds **one console only**, show automatically (no tab override needed).
4. If the player holds **multiple consoles**, show only when `ActiveConsole` is explicitly set to `CaptainChair`.

### Marker components

| Component | Purpose |
|---|---|
| `CaptainPanel` | Root node (defined in `client/app.rs`, used here for queries). |
| `DirButton(ViewDirection)` | Marks each direction pad button with its target direction. |
| `DirLed` | LED indicator dot inside a direction button. |
| `CompassDial` | Compass ring root node. |
| `CompassNeedle` | Rotating needle image. |
| `RedAlertToggle` | The red alert button. |
| `ArmedGlow` | Pulsing glow dot inside the red alert button. |
| `CaptainPanelSpawned` | One-shot resource; prevents duplicate spawning. |

## Registration

```rust
.add_plugins(crate::phone_border::CaptainPanelPlugin)
```

Registered by `wasm_client_init` in `src/client/bridge.rs`.

## What was removed from `client/app.rs`

As part of issue [#240](https://github.com/jkeywo/project-phoenix-v2/issues/240):

- `setup_captain_ui` (was already a no-op stub)
- `toggle_captain_panel_visibility` (moved here as the pure `captain_panel_visible` + Bevy wrapper)
- `refresh_view_dir_highlights` (dead — operated on `ViewDirButton` which was never spawned)
- `refresh_red_alert_button` (dead — operated on `RedAlertButton` which was never spawned)
- `handle_view_dir_button_press` (dead)
- `handle_red_alert_button_press` (dead)
- `ViewDirButton`, `RedAlertButton`, `RedAlertLabel` components (dead)
- `VIEW_BTN_BG_*` and `RED_ALERT_BG_*` colour constants (dead)

## Tests

Tests live in `src/client/phone_border/captain.rs` under `#[cfg(test)]`. Run with:

```bash
cargo test --features client client::phone_border::captain
```

Coverage:
- `captain_panel_visible`: 6 cases covering lobby phase, non-captain, single-console auto-show, multi-console tab selection
- `needle_rotation`: all four directions
- `direction_press_message`: two directions

## Sources

- `src/client/phone_border/captain.rs`
- `src/client/app.rs` (post-extraction)
- `src/client/bridge.rs`
- Issue [#240](https://github.com/jkeywo/project-phoenix-v2/issues/240)
- [PRD #187 — Phone Console HUD](../sources/prd-187-phone-bezel.md)
