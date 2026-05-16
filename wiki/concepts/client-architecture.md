---
title: Client Architecture
type: concept
tags: [client, plugin, wasm, bevy, panel, composition]
sources: [src/client/app.rs, src/client/bridge.rs, src/client_sim.rs, src/ship_view.rs]
updated: 2026-05-16
---

## Summary

The client is a full Bevy/WASM app built with the `client` Cargo feature, loaded by `client.html`. It renders the phone console UI — lobby station picker, then the in-game panel for whichever console(s) the local player holds. The plugin set is assembled in `src/client/app.rs::add_client_plugins` and registered by `wasm_client_init` in `src/client/bridge.rs`.

## Panel plugin inventory

All client-side plugins registered by `add_client_plugins` (see `src/client/app.rs:2018`):

| Plugin | File | What it owns |
|---|---|---|
| `ClientAppPlugin` | `src/client/app.rs` | Lobby UI, sensors/shields/navigation panels, tab bar, complexity UI, hideable elements |
| `ShipViewPlugin` | `src/ship_view.rs` | `ShipView` resource — ship pose, red-alert, power levels, impulse |
| `PhoneBorderPlugin` | `src/client/phone_border/framing.rs` | Diegetic phone bezel frame around all panels |
| `CaptainPanelPlugin` | `src/client/phone_border/captain.rs` | View selector + Red Alert toggle |
| `HelmPanelPlugin` | `src/helm_panel.rs` | Joystick, helm radar gizmo, "On Screen" button |
| `WeaponsPanelPlugin` | `src/weapons_panel.rs` | Phaser / torpedo UI, weapons radar gizmo |
| `RepairPanelPlugin` | `src/repair_panel.rs` | Shape-matching repair console, team status |
| `PowerPanelPlugin` | `src/power_panel.rs` | 6+2 power allocation console |
| `SciencePanelPlugin` | `src/science_panel.rs` | Long-range radar, system chart, cancel-impulse |
| `CommsPanelPlugin` | `src/comms_panel.rs` | Comms console (placeholder) |

`ClientRendererPlugin` (intentionally empty stub retained for backwards compat) is registered separately in the bridge after `add_client_plugins`.

## Composition entry point

```rust
// src/client/app.rs
pub fn add_client_plugins(app: &mut App) {
    app.add_plugins(ClientAppPlugin)
        .add_plugins(ShipViewPlugin)
        .add_plugins(PhoneBorderPlugin)
        .add_plugins(HelmPanelPlugin)
        .add_plugins(WeaponsPanelPlugin)
        .add_plugins(RepairPanelPlugin)
        .add_plugins(PowerPanelPlugin)
        .add_plugins(CaptainPanelPlugin)
        .add_plugins(SciencePanelPlugin)
        .add_plugins(CommsPanelPlugin);
}
```

The bridge (`src/client/bridge.rs::wasm_client_init`) adds `DefaultPlugins`, calls `add_client_plugins`, then adds `ClientRendererPlugin` and four bridge systems (`forward_local_token`, `forward_complexity_presets`, `forward_inbound_messages`, `flush_outbound_messages`).

## Shared client state resources

| Resource | File | Contents |
|---|---|---|
| `LobbyState` | `src/lobby/client_panel.rs` | Station assignments, player list, complexity map |
| `ClientSimState` | `src/client_sim.rs` | Console-specific sim state: repair teams, weapons, shields, world entities, modifiers, power, torpedo state |
| `ShipView` | `src/ship_view.rs` | Ship-level broadcast fields extracted from `ClientSimState` |
| `LocalPlayerToken` | `src/lobby/client_panel.rs` | Session token for the local player |
| `ActiveConsole` | `src/lobby/client_panel.rs` | Which console tab is currently visible |
| `ComplexityStore` | `src/client_complexity.rs` | Per-console complexity preset choices |
| `HideableElementRegistry` | `src/client/elements.rs` | Registry of UI element names that complexity presets can hide |

## Notes on client_sim.rs

`src/client_sim.rs` retains `ClientSimState` and all its console-specific fields. These have not yet been migrated to per-panel resources. The client-split series (#228) extracted ship-level fields into `ShipView` (issue #234) but the remaining console-specific state (repair, weapons, shields, world entities, modifiers, power, torpedoes) still lives in `ClientSimState`. Future splits could extract each panel's state into its own resource alongside its plugin.

## Client-split history

The client-split series (issue #228, closed with issue #263) refactored the client from two large god-modules into per-console plugins:

- **#234** — `ShipView` extracted from `ClientSimState`
- **#240** — `CaptainPanelPlugin` extracted from `client/app.rs`
- **#246** — `HelmPanelPlugin` extracted
- **#251** — `WeaponsPanelPlugin` extracted
- **#255** — `RepairPanelPlugin` extracted
- **#259** — `PowerPanelPlugin` extracted
- **#262** — `SciencePanelPlugin` + `CommsPanelPlugin` extracted
- **#263** — Thin composition `add_client_plugins` added; bridge updated (this issue)

## Cross-links

- [Ship View](ship-view.md)
- [Console Plugin Pattern](console-plugin-pattern.md)
- [Helm Panel](helm-panel.md)
- [Weapons Panel](weapons-panel.md)
- [Repair Panel](repair-panel.md)
- [Power Panel](power-panel.md)
- [Science Panel](science-panel.md)
- [Comms Panel](comms-panel.md)
- [Captain Panel](captain-panel.md)
