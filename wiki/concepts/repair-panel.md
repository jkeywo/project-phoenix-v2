---
title: RepairPanelPlugin
---

# RepairPanelPlugin

Extracted from `client/app.rs` as part of the client split series (issue [#255](https://github.com/jkeywo/project-phoenix-v2/issues/255)).

## Location

`src/repair_panel.rs` — compiled under the `client` Cargo feature.

Re-exported as `crate::repair_panel::RepairPanelPlugin`.

## Ownership

`RepairPanelPlugin` owns all Repair console UI:

- Breakdown label (shows current console + shape, or "All Systems Nominal")
- Shape buttons row (SQUARE, TRIANGLE, CIRCLE — sends `Repair { shape }`)
- Three team status rows with progress bars and text
- Repair icon label update (`RefreshRepairIcon`) — shown on consoles that receive
  `ShowRepairIcon` / `ClearRepairIcon` decoy messages
- Panel visibility toggling (driven by `LobbyState` + `ActiveConsole`)

Apply paths for `RepairState`, `ShowRepairIcon`, and `ClearRepairIcon` remain in
`ClientSimState.apply()` in `client_sim.rs` since they update shared state used
across multiple consoles.

### Systems

| System | Responsibility |
|---|---|
| `setup_repair_ui` | Spawns the full repair panel hierarchy under a `RepairPanel` root (hidden on spawn). One-shot; called at `Startup`. |
| `toggle_repair_panel_visibility` | Shows/hides `RepairPanel` based on phase, console assignment, and active tab. Delegates the decision to pure `repair_panel_visible`. |
| `refresh_repair_panel` | Updates breakdown label, shape button colours, team progress bars and status text from `ClientSimState`. Change-detected. |
| `handle_repair_shape_button_press` | Emits `Repair { shape }` when a shape button is pressed. |
| `refresh_repair_icon` | Updates `RepairIconLabel` text on any panel when `ClientSimState.repair_icon` changes. |

### Pure helpers

| Function | Signature | Testability |
|---|---|---|
| `repair_panel_visible(lobby, token, active) -> bool` | Pure, Bevy-free | Yes — 6 unit tests |

### Visibility rules (`repair_panel_visible`)

1. Game phase must be `InProgress`.
2. Local player must hold `Console::Repair`.
3. If the player holds **one console only**, show automatically (no tab override).
4. If the player holds **multiple consoles**, show only when `ActiveConsole` is
   explicitly set to `Repair`.

### Marker components

Defined in `repair_panel.rs` itself:

| Component | Purpose |
|---|---|
| `RepairPanel` | Root node; visibility target. |
| `RepairBreakdownLabel` | Shows the current breakdown (console + shape) or "All Systems Nominal". |
| `RepairShapeButton(Shape)` | Shape selection button (carries the `Shape` it fires). |
| `RepairShapeButtonRoot` | Container for the three shape buttons. |
| `RepairTeamRow(usize)` | Team row container (index 0, 1, 2). |
| `RepairTeamFill(usize)` | Progress bar fill node inside a team row. |
| `RepairTeamStatusText(usize)` | Status text overlaid on a team row. |

From `client/app.rs` (via `client_app` compat re-export):

| Component | Purpose |
|---|---|
| `RepairButton` | Repair button on Helm and other panels (remains in `client/app.rs`). |
| `RepairButtonLabel` | Label inside the `RepairButton`. |
| `RepairIconLabel` | Shows the current repair icon shape on panels that receive decoy icons. |

## Registration

```rust
.add_plugins(crate::repair_panel::RepairPanelPlugin)
```

Registered by `wasm_client_init` in `src/client/bridge.rs`, directly after
`WeaponsPanelPlugin`.

## What was removed from `client/app.rs`

As part of issue [#255](https://github.com/jkeywo/project-phoenix-v2/issues/255):

- `RepairPanel`, `RepairBreakdownLabel`, `RepairShapeButton`, `RepairShapeButtonRoot`,
  `RepairTeamRow`, `RepairTeamFill`, `RepairTeamStatusText` component definitions
- `setup_repair_ui` function and its `Startup` registration
- `toggle_repair_panel_visibility` system
- `refresh_repair_panel` system
- `handle_repair_shape_button_press` system
- `refresh_repair_icon` system
- `Shape` import from `messages` (was only used by repair-panel code)

The `RepairButton`, `RepairButtonLabel`, and `RepairIconLabel` marker components remain
in `client/app.rs` because `handle_repair_button_press` and `refresh_repair_button`
still operate there (those systems handle the repair button on the Helm panel, not the
Repair console panel).

## Tests

Tests live in `src/repair_panel.rs` under `#[cfg(test)]`. Run with:

```bash
cargo test --features client repair_panel
```

Coverage (6 tests):

- `repair_panel_visible`: 6 cases — lobby phase hides panel, InProgress+Repair shows it,
  non-repair player hidden, multi-console with Repair active shows, multi-console with
  other active hides, multi-console with no active hides

## Sources

- `src/repair_panel.rs`
- `src/client/app.rs` (post-extraction)
- `src/client/bridge.rs`
- `src/client_sim.rs` (apply paths for RepairState, ShowRepairIcon, ClearRepairIcon remain here)
- Issue [#255](https://github.com/jkeywo/project-phoenix-v2/issues/255)
