---
title: SciencePanelPlugin
---

# SciencePanelPlugin

Extracted as part of the client split series (issue [#262](https://github.com/jkeywo/project-phoenix-v2/issues/262)).

## Location

`src/science_panel.rs` — compiled under the `client` Cargo feature.

Re-exported as `crate::science_panel::SciencePanelPlugin`.

## Ownership

`SciencePanelPlugin` owns all Science console UI:

- Panel root visibility toggling (driven by `LobbyState` + `ActiveConsole`)
- View-mode selector buttons: **Radar** and **System Chart**
- **On Screen** button — pushes the active view mode to the server viewscreen
- **Cancel Impulse** button — sends `CancelImpulse` to the server
- `ScienceView` resource tracking which sub-view is currently active

Apply paths for science-related state remain in `client_sim.rs`
(`compute_science_long_range_radar_view`, `compute_system_chart_view`,
`cancel_impulse_button_visible`, `set_science_target_message`).

### Resources

| Resource | Purpose |
|---|---|
| `ScienceView` | Tracks the active sub-view (`ScienceRadar` or `SystemChart`). Default: `ScienceRadar`. |

### Systems

| System | Responsibility |
|---|---|
| `setup_science_ui` | Spawns the full science panel hierarchy under a `SciencePanel` root (hidden on spawn). One-shot; called at `Startup`. |
| `toggle_science_panel_visibility` | Shows/hides `SciencePanel` based on phase, console assignment, and active tab. Delegates to pure `science_panel_visible`. |
| `handle_science_radar_button_press` | Sets `ScienceView::ScienceRadar` and emits `SetView { ScienceRadar }` when pressed. |
| `handle_science_system_chart_button_press` | Sets `ScienceView::SystemChart` and emits `SetView { SystemChart }` when pressed. |
| `handle_science_cancel_impulse_button_press` | Emits `CancelImpulse` when pressed. |
| `handle_science_on_screen_button_press` | Emits `SetView` with the current `ScienceView` mode when pressed. |

### Pure helpers

| Function | Signature | Testability |
|---|---|---|
| `science_panel_visible(lobby, token, active) -> bool` | Pure, Bevy-free | Yes — unit tests in `science_panel.rs` |
| `science_target_message(uuid) -> ClientMessage` | Pure builder | Yes |

### Visibility rules (`science_panel_visible`)

1. Game phase must be `InProgress`.
2. Local player must hold `Console::Sensors`.
3. If the player holds **one console only**, show automatically (no tab override).
4. If the player holds **multiple consoles**, show only when `ActiveConsole` is
   explicitly set to `Sensors`.

### Marker components

| Component | Purpose |
|---|---|
| `SciencePanel` | Root node; visibility target. |
| `ScienceRadarButton` | View-mode button that selects long-range radar. |
| `ScienceSystemChartButton` | View-mode button that selects the system chart. |
| `ScienceCancelImpulseButton` | Sends `CancelImpulse` on press. |
| `ScienceOnScreenButton` | Pushes current view mode to the server viewscreen. |

## Registration

```rust
.add_plugins(crate::science_panel::SciencePanelPlugin)
```

Registered by `wasm_client_init` in `src/client/bridge.rs`, after `CaptainPanelPlugin`.

## Tests

Tests live in `src/science_panel.rs` under `#[cfg(test)]`. Run with:

```bash
cargo test science_panel
```

Coverage:

- `science_panel_not_visible_in_lobby_phase` — panel stays hidden in Lobby
- `science_panel_not_visible_when_player_does_not_hold_sensors` — wrong token → hidden
- `science_view_default_is_science_radar` — resource default check
- `science_target_message_produces_set_science_target` — message builder

## Sources

- `src/science_panel.rs`
- `src/client/bridge.rs`
- `src/client_sim.rs` (compute helpers remain here)
- Issue [#262](https://github.com/jkeywo/project-phoenix-v2/issues/262)
