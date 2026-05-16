---
title: HelmPanelPlugin
---

# HelmPanelPlugin

Extracted from `client/app.rs` and `client/phone_border/helm.rs` as part of the client split series (issue [#246](https://github.com/jkeywo/project-phoenix-v2/issues/246)).

## Location

`src/helm_panel.rs` — compiled under the `client` Cargo feature.

Re-exported as `crate::helm_panel::HelmPanelPlugin`. The old path
`crate::phone_border::HelmPanelPlugin` continues to work via a thin shim in
`src/client/phone_border/helm.rs`.

## Ownership

`HelmPanelPlugin` owns all helm console UI:

- Compass-ring radar with corner readouts (HDG, SPD, X, Z)
- Polished thumbstick joystick with pointer-drag observers
- Panel visibility toggling (driven by `LobbyState` + `ActiveConsole`)
- 10 Hz joystick input resend (`HelmTickTimer`)
- On Screen button + background colour refresh
- Gizmo-based radar overlay (`draw_helm_radar`)

Joystick input logic is **not** duplicated here — `HelmPanelPlugin` imports the
pure helpers `drag`, `release`, and `tick` from `crate::client_helm`.

### Systems

| System | Responsibility |
|---|---|
| `spawn_phone_helm_ui` | Spawns compass-ring radar + polished thumbstick once `PhoneAssets` are loaded. One-shot, guarded by `PhoneHelmSpawned` resource. Respects landscape/portrait orientation via `DeviceOrientation`. |
| `toggle_helm_panel_visibility` | Shows/hides `HelmPanel` based on phase, console assignment, and active tab. Emits a release message if the panel hides mid-drag. Delegates the visibility decision to pure `helm_panel_visible`. |
| `helm_resend_tick` | Fires every 100 ms to resend the last `HelmInput` so the server keeps applying thrust/steering. |
| `refresh_phone_helm_readout` | Updates the `"Thrust X% / Steering Y%"` text below the thumbstick from `HelmJoystickState`. |
| `update_phone_helm_knob` | Moves the knob node to match `knob_dx`/`knob_dy` in `HelmJoystickState`. |
| `rotate_compass_ring_by_yaw` | Rotates `PhoneCompassRing` transform to match `ShipView.ship_yaw`. |
| `update_radar_readouts` | Updates HDG/SPD/X/Z corner text nodes from `ShipView`; computes speed from position delta via `PhoneShipSpeed`. |
| `handle_on_screen_button_press` | Emits `SetView { mode: Radar }` when the ON SCREEN button is pressed. |
| `refresh_on_screen_button_style` | Updates ON SCREEN button background colour based on `ShipView.view_mode`. |
| `draw_helm_radar` | Gizmo radar overlay: outer ring, mid ring, asteroid blips, ship triangle. Reads `RadarPanel` bounds; skipped while the helm panel is hidden. |

Pointer observers `on_phone_helm_drag_start`, `on_phone_helm_drag`,
`on_phone_helm_drag_end` are attached directly to the thumbstick pad entity.

### Pure helpers

| Function | Signature | Testability |
|---|---|---|
| `helm_panel_visible(lobby, token, active) -> bool` | Pure, Bevy-free | Yes — 5 unit tests |
| `helm_max_radius() -> f32` | Derived from `HELM_PAD_SIZE` / `HELM_KNOB_RADIUS` | Yes |
| `bearing_ticks() -> [BearingTick; 36]` | 36 ticks at 10° intervals, every 3rd is major | Yes — 5 unit tests |
| `range_ring_radii(radar_radius_px) -> [f32; 3]` | Three proportional ring radii | Yes — 3 unit tests |
| `range_ring_labels() -> [String; 3]` | `["200", "400", "600"]` | Yes — 1 unit test |
| `yaw_to_heading(yaw_rad) -> String` | Converts ship yaw to `"XXX°"` heading string | Yes — 6 unit tests |

### Visibility rules (`helm_panel_visible`)

1. Game phase must be `InProgress`.
2. Local player must hold `Console::Helm`.
3. If the player holds **one console only**, show automatically (no tab override).
4. If the player holds **multiple consoles**, show only when `ActiveConsole` is
   explicitly set to `Helm`.

### Resources

| Resource | Purpose |
|---|---|
| `HelmJoystickState` | Shared with `client_helm`; owns `knob_dx`, `knob_dy`, `active`, `last_thrust`, `last_steering`. |
| `HelmTickTimer` | 100 ms repeating timer for the 10 Hz resend loop. |
| `PhoneShipSpeed` | Tracks computed speed from `ship_x`/`ship_z` position deltas. |
| `PhoneHelmSpawned` | One-shot marker; prevents duplicate UI spawning. |

### Marker components

| Component | Purpose |
|---|---|
| `HelmPanel` | Root node (defined in `client/app.rs`, used here for queries). |
| `RadarPanel` | Compass-ring radar container (defined in `client/app.rs`). |
| `OnScreenButton` | The "ON SCREEN" button (defined in `client/app.rs`). |
| `PhoneCompassRadar` | Outermost radar container. |
| `PhoneCompassRing` | Rotating ring driven by ship yaw. |
| `PhoneCompassTick` | Single tick mark on the ring. |
| `PhoneHdgReadout` | HDG corner text. |
| `PhoneSpdReadout` | SPD corner text. |
| `PhoneXReadout` | X coordinate corner text. |
| `PhoneZReadout` | Z coordinate corner text. |
| `PhoneRangeRing` | Range ring node. |
| `PhoneThumbRing` | Outer ring visual on the thumbstick. |
| `PhoneHelmPad` | Drag-event capture pad. |
| `PhoneHelmKnob` | The draggable knob disc. |
| `PhoneHelmReadout` | Thrust/steering text below the thumbstick. |

## Registration

```rust
.add_plugins(crate::helm_panel::HelmPanelPlugin)
```

Registered by `wasm_client_init` in `src/client/bridge.rs`.

## What was removed from `client/app.rs`

As part of issue [#246](https://github.com/jkeywo/project-phoenix-v2/issues/246):

- `toggle_helm_panel_visibility` (moved here as `helm_panel_visible` + Bevy wrapper)
- `helm_resend_tick`
- `refresh_helm_knob_position` (now `update_phone_helm_knob`)
- `refresh_helm_readout` (now `refresh_phone_helm_readout`)
- `handle_on_screen_button_press`
- `refresh_on_screen_button_style`
- `draw_helm_radar`
- `HelmTickTimer` struct and resource insertion
- `HelmJoystickState` resource insertion (now inserted by `HelmPanelPlugin`)
- `HELM_PAD_SIZE`, `HELM_KNOB_RADIUS` constants
- `ON_SCREEN_BG_IDLE`, `ON_SCREEN_BG_ACTIVE`, radar colour constants
- `client_helm::{release, tick, HelmJoystickState}` import
- `client_sim::on_screen_message` import

The marker components `HelmPanel`, `RadarPanel`, `OnScreenButton`, `HelmKnob`,
`HelmReadout` remain in `client/app.rs` because `client_bridge.rs` imports them
via the `client_app` compat module.

`setup_helm_ui` is retained as a no-op stub in `client/app.rs` to avoid
touching the `add_systems(Startup, ...)` call.

## What was removed from `client/phone_border/helm.rs`

All plugin code was replaced with a backwards-compatibility re-export shim that
forwards every public item from `crate::helm_panel`. The 840-line implementation
(plugin, spawn functions, update systems, pointer observers, tests) now lives
exclusively in `src/helm_panel.rs`.

## Tests

Tests live in `src/helm_panel.rs` under `#[cfg(test)]`. Run with:

```bash
cargo test
```

Coverage:

- `helm_panel_visible`: 5 cases — lobby phase, non-helm player, single-console auto-show, multi-console tab selection (helm tab and other tab)
- `bearing_ticks`: 5 cases — count, first tick, major/minor pattern, sequential labels, last label, minor-tick empty label
- `range_ring_radii`: 3 cases — count, proportional spacing, scale invariance
- `range_ring_labels`: 1 case — string values
- `yaw_to_heading`: 6 cases — zero, 45°, 180°, negative yaw wrap, 2π wrap, always positive

## Sources

- `src/helm_panel.rs`
- `src/client/app.rs` (post-extraction)
- `src/client/bridge.rs`
- `src/client/phone_border/helm.rs` (now a shim)
- `src/client_helm.rs` (pure joystick logic, imported not moved)
- Issue [#246](https://github.com/jkeywo/project-phoenix-v2/issues/246)
- [PRD #187 — Phone Console HUD](../sources/prd-187-phone-bezel.md)
