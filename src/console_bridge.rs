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
/// a console directly rather than through a remote PeerJS session. The host
/// page routes its console actions through `gui/action-map.js` into full
/// `ClientMessage` JSON and submits them via `wasm_receive_message` under this
/// token (issue #822); the gameplay console handlers treat it as an authorized
/// local operator (see the weapons fire guards). Defined here — ungated — so
/// both the wasm bridge and the (non-wasm) gameplay handlers can reference it.
pub const LOCAL_CONSOLE_TOKEN: &str = "__local_console__";

/// Emitted by `viewscreen_border::push_hud_state` when the serialised HUD
/// state changes. `json` is the output of `codec::encode_hud_state`. Drained
/// by `bridge::flush_host_channels` (wasm) onto the `"hud"` host channel.
#[derive(Message, Clone, Debug)]
pub struct HudStateChanged {
    pub json: String,
}

/// Emitted by the lobby push system when the serialised lobby state changes.
/// `json` is the output of `codec::encode_lobby_state`. Drained by
/// `bridge::flush_host_channels` (wasm) onto the `"lobby"` host channel.
#[derive(Message, Clone, Debug)]
pub struct LobbyStateChanged {
    pub json: String,
}

/// Emitted by the coordination lag processor when an AI→AI coordination
/// message is delivered. Carries the origin/target labels, typed payload and
/// the producer-owned presentation envelope. The Viewscreen and phone resolve
/// the same envelope; neither presenter enumerates the payload. Drained by
/// `bridge::flush_host_channels` (wasm), which encodes it via
/// `codec::encode_chatter` onto the `"chatter"` host channel — hence the
/// `Serialize` derive; the wire shape is pinned by `codec`'s `encode_chatter`
/// tests.
#[derive(Message, Clone, Debug, serde::Serialize)]
pub struct AiChatterEvent {
    /// Sender label: a `chatter.sender.*` / `station.*.name` string id resolved
    /// on the client, or a human player's name (which no table row matches, so
    /// it passes through untouched).
    pub from_label: String,
    /// Authoritative destination label: a `station.*.name` or
    /// `chatter.addressee.ship` String Table id. The phone and Viewscreen use
    /// this directly, so neither keeps its own Station-name switch.
    pub to_label: String,
    /// The typed coordination payload retained for semantic consumers.
    pub payload: crate::core::messages::CoordinationPayload,
    /// Localised-or-literal title/body and deterministic typed parameters.
    pub presentation: crate::core::messages::CoordinationPresentation,
}

/// Emitted once by `server::audio::push_audio_config` when the local ship
/// spawns, carrying the merged ship + world audio config. `json` is the output
/// of `codec::encode_audio_config`. Drained by `bridge::flush_host_channels`
/// (wasm) onto the `"audio_config"` host channel, whose JS handler builds the
/// audio graph.
#[derive(Message, Clone, Debug)]
pub struct AudioConfigChanged {
    pub json: String,
}

/// Emitted by `server::audio::push_blaster_cues` for each one-shot positional
/// sound. `json` is the output of `codec::encode_audio_cue` and carries
/// listener-relative coordinates. Drained by `bridge::flush_host_channels`
/// (wasm) onto the `"audio_cue"` host channel.
#[derive(Message, Clone, Debug)]
pub struct AudioCueEvent {
    pub json: String,
}
