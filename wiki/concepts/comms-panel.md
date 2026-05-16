---
title: CommsPanelPlugin
---

# CommsPanelPlugin

Extracted as part of the client split series (issue [#262](https://github.com/jkeywo/project-phoenix-v2/issues/262)).
Replaces and deletes the old `src/comms_plugin.rs` husk (~24 lines).

## Location

`src/comms_panel.rs` — compiled under the `client` Cargo feature.

Re-exported as `crate::comms_panel::CommsPanelPlugin`.

## Ownership

`CommsPanelPlugin` owns the Comms console UI placeholder and initialises
`ClientCommsState`:

- `ClientCommsState` resource initialisation (folded in from `comms_plugin.rs`)
- `CommsView` resource — placeholder for the sub-view switcher until PRD #119 fills it
- Panel root visibility toggling (driven by `LobbyState` + `ActiveConsole`)
- Minimal panel UI (placeholder label until PRD #119 fills in the full inbox layout)

The pure Comms state logic lives in `client_comms.rs` (unchanged).

### Resources

| Resource | Purpose |
|---|---|
| `ClientCommsState` | Inbox messages, objectives, contacts, and selected-message state. Initialised here; updated by `client_comms::apply()`. |
| `CommsView` | Tracks the active sub-view. Placeholder until PRD #119 fills in the full comms design. Default: `Inbox`. |

### Systems

| System | Responsibility |
|---|---|
| `setup_comms_ui` | Spawns the comms panel root (hidden on spawn). One-shot; called at `Startup`. |
| `toggle_comms_panel_visibility` | Shows/hides `CommsPanel` based on phase, console assignment, and active tab. Delegates to pure `comms_panel_visible`. |

### Pure helpers

| Function | Signature | Testability |
|---|---|---|
| `comms_panel_visible(lobby, token, active) -> bool` | Pure, Bevy-free | Yes — unit tests in `comms_panel.rs` |

### Visibility rules (`comms_panel_visible`)

1. Game phase must be `InProgress`.
2. Local player must hold `Console::Comms`.
3. If the player holds **one console only**, show automatically (no tab override).
4. If the player holds **multiple consoles**, show only when `ActiveConsole` is
   explicitly set to `Comms`.

### Marker components

| Component | Purpose |
|---|---|
| `CommsPanel` | Root node; visibility target. |

## Registration

```rust
.add_plugins(crate::comms_panel::CommsPanelPlugin)
```

Registered by `wasm_client_init` in `src/client/bridge.rs`, after `SciencePanelPlugin`.

## What was deleted

`src/comms_plugin.rs` (the old 24-line husk) was deleted. Its only content — 
`app.init_resource::<ClientCommsState>()` — is now handled by `CommsPanelPlugin::build`.

`pub mod comms_plugin;` removed from `src/lib.rs`.

## Tests

Tests live in `src/comms_panel.rs` under `#[cfg(test)]`. Run with:

```bash
cargo test comms_panel
```

Coverage:

- `comms_panel_not_visible_in_lobby_phase` — panel stays hidden in Lobby
- `comms_panel_not_visible_when_player_does_not_hold_comms` — wrong token → hidden
- `comms_view_default_is_inbox` — resource default check

## Sources

- `src/comms_panel.rs`
- `src/client_comms.rs` (pure state model; unchanged)
- `src/client/bridge.rs`
- Issue [#262](https://github.com/jkeywo/project-phoenix-v2/issues/262)
- PRD [#119](https://github.com/jkeywo/project-phoenix-v2/issues/119) — Stations, Scenarios & Comms (will fill in full panel design)
