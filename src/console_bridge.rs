//! HTML console bridge messages and registration (issue #422 / PRD #419).
//!
//! These Bevy `Message` types carry encoded JSON from the simulation's push
//! systems (in `server::viewscreen_border` and the weapons console plugin) to
//! the wasm forwarding systems in `server::bridge`. They live at the crate
//! root — outside both the `server` feature gate and any
//! `#[cfg(target_arch = "wasm32")]` gate — so the push systems compile and run
//! on native (for `cargo test`) and in the shared (client/server) weapons
//! plugin, while only the JS-callback forwarding is wasm-gated.

use bevy::prelude::*;

/// Session token used for actions originating from the local HTML consoles
/// (browser server viewscreen / native wry server), where the operator drives
/// a console directly rather than through a remote PeerJS session. Actions
/// decoded by `bridge::drain_ui_actions` are injected as `InboundMessage` under
/// this token, and the gameplay console handlers treat it as an authorized
/// local operator (see the weapons fire guards). Defined here — ungated — so
/// both the wasm bridge and the (non-wasm) gameplay handlers can reference it.
pub const LOCAL_CONSOLE_TOKEN: &str = "__local_console__";

/// Emitted by `viewscreen_border::push_hud_state` when the serialised HUD
/// state changes. `json` is the output of `codec::encode_hud_state`. Read by
/// `bridge::flush_hud_state` (wasm) to call the registered JS HUD callback.
#[derive(Message, Clone, Debug)]
pub struct HudStateChanged {
    pub json: String,
}

/// Emitted by a console's push system when its serialised state changes.
/// `name` is the PascalCase console name (e.g. `"Tactical"`); `json` is the
/// output of `codec::encode_console_state`. Read by `bridge::flush_console_state`
/// (wasm) to call the registered JS console callback with `(name, json)`.
#[derive(Message, Clone, Debug)]
pub struct ConsoleStateChanged {
    pub name: String,
    pub json: String,
}

/// Emitted by the lobby push system when the serialised lobby state changes.
/// `json` is the output of `codec::encode_lobby_state`. Read by
/// `bridge::flush_lobby_state` (wasm) to call the registered JS lobby callback.
#[derive(Message, Clone, Debug)]
pub struct LobbyStateChanged {
    pub json: String,
}
