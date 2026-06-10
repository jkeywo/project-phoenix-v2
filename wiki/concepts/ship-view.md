---
title: ShipView
---

# ShipView

`ShipView` is a Bevy `Resource` that holds the ship-level state that every console needs: pose, red alert flag, view mode, hull fraction, power levels, and impulse charge progress.

It is the client-side mirror of the fields broadcast in every `SimState` 10 Hz tick. Console panels read from `Res<ShipView>` instead of reaching into the larger `ClientSimState` god-resource.

## Fields

| Field | Type | Source |
|---|---|---|
| `red_alert` | `bool` | `SimSnapshot::red_alert` |
| `view_mode` | `ViewMode` | `SimSnapshot::view_mode` |
| `ship_x` | `f32` | `SimSnapshot::ship_x` |
| `ship_z` | `f32` | `SimSnapshot::ship_z` |
| `ship_yaw` | `f32` | `SimSnapshot::ship_yaw` |
| `hull_fraction` | `f32` | `hull_integrity / 100.0` clamped to `[0, 1]` |
| `power_levels` | `(u8, u8, u8)` | `SimSnapshot::power_levels` (Helm, Weapons, Science) |
| `impulse_charge_progress` | `f32` | `SimSnapshot::impulse_charge_progress` |

## Helper methods

| Method | Description |
|---|---|
| `is_active_camera_direction(&ViewDirection) -> bool` | Returns `true` iff the view mode is `Camera(d)` where `d` matches the argument. Returns `false` in Radar/chart modes. Used by the captain panel direction buttons. |
| `apply(&ServerMessage)` | Updates whichever fields the message carries. Handles `SimState`, `Welcome` (resets to default). |

## ShipViewPlugin

`ShipViewPlugin` (compiled only when the `client` feature is active) owns the resource lifecycle:

```rust
// src/ship_view.rs
pub struct ShipViewPlugin;

impl Plugin for ShipViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShipView>()
            .add_systems(Update, apply_ship_view_messages);
    }
}
```

The `apply_ship_view_messages` system reads every `InboundServerMessage` event and calls `ShipView::apply`. (It was previously one of two readers alongside `ClientAppPlugin`'s `apply_inbound_messages`; that drain was deleted in #460 when lobby/sim/comms/complexity state moved to pure JS modules in `gui/`, leaving this as the sole Bevy-side consumer.)

## Registration

```rust
// src/client/bridge.rs — wasm_client_init()
.add_plugins(ClientAppPlugin)
.add_plugins(crate::ship_view::ShipViewPlugin)
```

`ShipViewPlugin` is added after `ClientAppPlugin` so the `InboundServerMessage` message type (registered by `ClientAppPlugin`) is available.

## Consumers

Systems that read ship-view fields use `Res<ShipView>` rather than `Res<ClientSimState>`:

| System | File | Field(s) read |
|---|---|---|
| `refresh_view_dir_highlights` | `client/app.rs` | `is_active_camera_direction()` |
| `refresh_red_alert_button` | `client/app.rs` | `red_alert` |
| `refresh_on_screen_button_style` | `client/app.rs` | `view_mode` |
| `refresh_navigation_panel` | `client/app.rs` | `impulse_charge_progress` |
| `draw_helm_radar` | `client/app.rs` | via `compute_helm_radar_view` |
| `draw_nav_chart` | `client/app.rs` | via `compute_system_chart_view` |
| `draw_weapons_radar` | `client/app.rs` | via `compute_weapons_radar_view` |
| `refresh_power_panel` | `client/app.rs` | `power_levels` |
| `handle_increase_power` | `client/app.rs` | `power_levels` |
| `handle_decrease_power` | `client/app.rs` | `power_levels` |
| `swap_bezel_textures` | `client/phone_border/framing.rs` | `red_alert` |
| `drive_vignette_intensity` | `client/phone_border/framing.rs` | `red_alert` |
| `refresh_alert_banner` | `client/phone_border/framing.rs` | `red_alert` |
| `refresh_dir_highlights` | `client/phone_border/captain.rs` | `is_active_camera_direction()` |
| `refresh_red_alert_ui` | `client/phone_border/captain.rs` | `red_alert` |
| `rotate_needle_by_direction` | `client/phone_border/captain.rs` | `view_mode` |
| `rotate_compass_ring_by_yaw` | `client/phone_border/helm.rs` | `ship_yaw` |
| `update_radar_readouts` | `client/phone_border/helm.rs` | `ship_x`, `ship_z`, `ship_yaw` |

## Radar helper functions

`compute_helm_radar_view`, `compute_weapons_radar_view`, `compute_system_chart_view`, and `compute_science_long_range_radar_view` in `client_sim.rs` now accept both a `&ClientSimState` (for `world.entities`) and a `&ShipView` (for ship position/yaw).

## Sources

- `src/ship_view.rs`
- `src/client/app.rs`
- `src/client/bridge.rs`
- `src/client/phone_border/`
- Issue [#234](https://github.com/jkeywo/project-phoenix-v2/issues/234)
