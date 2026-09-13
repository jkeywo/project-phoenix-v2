//! Room adapter for the shared presentation continuation boundary.
//! The shared owner compiles without `server`; only these current room/HUD
//! baselines require the optional presentation stack.

use bevy::prelude::*;

// Preserve the existing adapter path for host callers and registration sites.
pub use crate::audio_lifecycle::RoomAudioLifecycle;

pub fn publish_audio_lifecycle(world: &mut World) {
    if crate::audio_lifecycle::advance_audio_lifecycle(world) {
        crate::server::audio::rebase_audio_presentation(world);
        crate::server::viewscreen_border::rebase_hud_state(world);
    }
}
