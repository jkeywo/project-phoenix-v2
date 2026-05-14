//! Bevy plugin for the Comms console phone UI.
//!
//! Renders a two-panel inbox layout:
//! - **Left panel** — message list (sender name + subject line).
//! - **Right panel** — expanded chat view when a message is selected.
//!
//! This plugin drives `ClientCommsState` from inbound `ServerMessage`s and
//! wires response buttons back to `ClientMessage` outbound events. The
//! response buttons are rendered but inert in this slice (no scenario-driven
//! message production yet); they will be activated in the next slice.
//!
//! **Not unit-tested** — visual / Bevy layer. See `client_comms.rs` for the
//! pure, tested logic that backs this plugin.

use bevy::prelude::*;
use crate::client_comms::ClientCommsState;

pub struct CommsPlugin;

impl Plugin for CommsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientCommsState>();
    }
}
