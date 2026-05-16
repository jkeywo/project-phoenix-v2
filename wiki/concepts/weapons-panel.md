---
title: WeaponsPanelPlugin
---

# WeaponsPanelPlugin

Extracted from `client/app.rs` as part of the client split series (issue [#251](https://github.com/jkeywo/project-phoenix-v2/issues/251)).

## Location

`src/weapons_panel.rs` — compiled under the `client` Cargo feature.

Re-exported as `crate::weapons_panel::WeaponsPanelPlugin`.

## Ownership

`WeaponsPanelPlugin` owns all Tactical console UI:

- Tactical radar (gizmo overlay inside `WeaponsRadarPanel` bounds)
- Torpedo tube selection, count display, and tube status labels
- Fire Torpedo button with state-driven label
- Phaser mode toggle (Auto / Manual)
- Fire Phasers button with cooldown-driven label and colour
- Repair icon label (shared concern shown on the tactical panel)
- Complexity dropdown row and pop-up overlay
- Panel visibility toggling (driven by `LobbyState` + `ActiveConsole`)
- `SelectedTube` resource — tracks which torpedo tube is selected

Apply paths remain in `ClientSimState.apply()` in `client_sim.rs` since they
update shared simulation state used by multiple consoles.

### Systems

| System | Responsibility |
|---|---|
| `setup_weapons_ui` | Spawns the full tactical panel hierarchy under a `WeaponsPanel` root (hidden on spawn). One-shot; called at `Startup`. |
| `toggle_weapons_panel_visibility` | Shows/hides `WeaponsPanel` based on phase, console assignment, and active tab. Delegates the visibility decision to pure `weapons_panel_visible`. |
| `handle_fire_phaser_button_press` | Emits `FirePhaser` when the button is pressed and the bank is not on cooldown. |
| `handle_phaser_mode_toggle_press` | Emits `SetPhaserMode` toggling Auto ↔ Manual. |
| `refresh_weapons_panel` | Updates fire-button colour and label, and mode-button label from `ClientSimState`. Change-detected. |
| `handle_torpedo_tube_button_press` | Toggles `SelectedTube` — press same tube to deselect, press different tube to switch. |
| `handle_fire_torpedo_button_press` | Emits `FireTorpedo { tube, target_uuid: None }` when the selected tube is loaded and torpedoes remain. |
| `refresh_torpedo_ui` | Updates count label, tube status labels, tube-button highlights, and fire-button appearance from `ClientSimState` + `SelectedTube`. Change-detected. |
| `draw_weapons_radar` | Gizmo radar overlay: outer ring, mid ring, asteroid blips, ship triangle. Reads `WeaponsRadarPanel` bounds; skipped while the weapons panel is hidden. |

### Pure helpers

| Function | Signature | Testability |
|---|---|---|
| `weapons_panel_visible(lobby, token, active) -> bool` | Pure, Bevy-free | Yes — 6 unit tests |

Message builders `fire_phaser_message()`, `fire_torpedo_message()`, and
`set_phaser_mode_message()` live in `client_sim.rs` and are imported here.
They are also tested here (3 tests) and in `client_sim.rs`.

### Visibility rules (`weapons_panel_visible`)

1. Game phase must be `InProgress`.
2. Local player must hold `Console::Tactical`.
3. If the player holds **one console only**, show automatically (no tab override).
4. If the player holds **multiple consoles**, show only when `ActiveConsole` is
   explicitly set to `Tactical`.

### Resources

| Resource | Purpose |
|---|---|
| `SelectedTube` | Tracks the currently selected torpedo tube (`None` when no tube is selected). |

### Marker components

These are defined in `client/app.rs` and re-exported via the `client_app` compat module:

| Component | Purpose | Definition |
|---|---|---|
| `WeaponsPanel` | Root node; visibility target. | `client/app.rs` |
| `WeaponsRadarPanel` | Radar display node (gizmo target). | `client/app.rs` |
| `HideableElement` | Marks UI elements hideable by complexity preset. | `client/app.rs` (made `pub` in #251) |
| `ComplexityPopupRoot` | Complexity pop-up overlay root. | `client/app.rs` (made `pub` in #251) |
| `ComplexityPresetButton` | Preset option button (Low/Std). | `client/app.rs` (made `pub` in #251) |
| `ComplexityPopupConfirm` | Confirm button on the pop-up. | `client/app.rs` (made `pub` in #251) |
| `ComplexityDropdownRoot` | Complexity dropdown row root. | `client/app.rs` (made `pub` in #251) |

Private to `weapons_panel.rs`:

| Component | Purpose |
|---|---|
| `FirePhaserButton` | Fire Phasers button. |
| `FirePhaserLabel` | Label inside Fire Phasers button. |
| `PhaserModeButton` | Phaser mode toggle button. |
| `PhaserModeLabel` | Label inside mode button. |
| `TorpedoTubeButton(TorpedoTube)` | Tube selection button (carries the tube). |
| `TorpedoCountLabel` | Torpedo count text. |
| `TubeStatusLabel(TorpedoTube)` | Per-tube reload status text. |
| `FireTorpedoButton` | Fire Torpedo button. |
| `FireTorpedoLabel` | Label inside Fire Torpedo button. |

## Registration

```rust
.add_plugins(crate::weapons_panel::WeaponsPanelPlugin)
```

Registered by `wasm_client_init` in `src/client/bridge.rs`, directly after
`HelmPanelPlugin`.

## What was removed from `client/app.rs`

As part of issue [#251](https://github.com/jkeywo/project-phoenix-v2/issues/251):

- `FirePhaserButton`, `FirePhaserLabel`, `PhaserModeButton`, `PhaserModeLabel`
  component definitions
- `SelectedTube` resource definition and `init_resource::<SelectedTube>()` call
- `TorpedoTubeButton`, `FireTorpedoButton`, `FireTorpedoLabel`, `TorpedoCountLabel`,
  `TubeStatusLabel` component definitions
- `setup_weapons_ui` function and its `Startup` registration
- `toggle_weapons_panel_visibility` system
- `handle_fire_phaser_button_press`, `handle_phaser_mode_toggle_press` systems
- `refresh_weapons_panel` system
- `handle_torpedo_tube_button_press`, `handle_fire_torpedo_button_press` systems
- `refresh_torpedo_ui` system
- `draw_weapons_radar` system and its `RADAR_*` colour constants
- `fire_phaser_message`, `set_phaser_mode_message`, `fire_torpedo_message` imports
- `PhaserMode` import from `messages`

The `WeaponsPanel` and `WeaponsRadarPanel` marker components remain in
`client/app.rs` because other modules reference them via the `client_app`
compat re-export.

The complexity-related components (`ComplexityPopupRoot`, `ComplexityPresetButton`,
`ComplexityPopupConfirm`, `ComplexityDropdownRoot`) were made `pub` in
`client/app.rs` so `weapons_panel.rs` can import and spawn them within the
tactical panel hierarchy.

## Tests

Tests live in `src/weapons_panel.rs` under `#[cfg(test)]`. Run with:

```bash
cargo test --features client weapons_panel
```

Coverage (15 tests):

- `weapons_panel_visible`: 6 cases — lobby phase, non-tactical player, single-console
  auto-show, multi-console tactical tab, multi-console other tab, multi-console no tab
- `fire_phaser_message`: 1 case — produces `ClientMessage::FirePhaser`
- `fire_torpedo_message`: 2 cases — ForePort no target, Aft with target UUID
- `set_phaser_mode_message`: 2 cases — Auto and Manual variants
- `SelectedTube::default`: 1 case — starts as `None`
- Tube toggle logic: 3 cases — select unselected, deselect same, switch to different

## Sources

- `src/weapons_panel.rs`
- `src/client/app.rs` (post-extraction)
- `src/client/bridge.rs`
- `src/client_sim.rs` (message builders + apply paths remain here)
- Issue [#251](https://github.com/jkeywo/project-phoenix-v2/issues/251)
