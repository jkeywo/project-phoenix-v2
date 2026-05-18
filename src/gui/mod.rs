//! Generic GUI widget library for all phone console panels.
//!
//! Add `GuiPlugin` once to the `App`; it registers every widget sub-system.
//! Each widget module is also available directly for callers that need just the
//! types.

use bevy::prelude::*;

pub use foundation::{
    resolve_visual, resolve_visuals_system, Disabled, StateVisuals, Visual, WidgetState,
};
pub use button::{
    ButtonPressed, ButtonSize, ClickSound, GuiButtonMarker, UiSounds,
    WidgetActivated, WidgetDeactivated, resolve_click_sound, setup_ui_sounds,
};
pub use joystick::{
    normalize_joystick, GenericJoystick, GenericJoystickPad, GenericJoystickKnob,
    JoystickDragState, JoystickMoved, JoystickResendTimer,
};

mod foundation;
pub mod button;
pub mod joystick;

// ── Root plugin ───────────────────────────────────────────────────────────────

/// Root plugin for the gui widget library.  Add it once; it pulls in every
/// widget sub-system.
pub struct GuiPlugin;

impl Plugin for GuiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, resolve_visuals_system)
           .add_plugins(button::GuiButtonPlugin)
           .add_plugins(joystick::GuiJoystickPlugin);
    }
}
