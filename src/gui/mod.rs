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
pub use radar::{
    GenericRadar, GenericRadarWidget, OnRadar, RadarAppearance, RadarCenter,
    RadarFilter, RadarLayer, RadarShape, OrientationMode,
    is_on_radar, project_radar_entity,
};
pub use panel::{GuiPanel, GuiPanelMarker, PanelSize, lerp_size};
pub use progress::{
    ProgressBar, ProgressBarMarker, ProgressBarVariant, ProgressValue,
    SegmentCount, filled_segments,
};
pub use light::{FlickerLight, FlickerLightConfig, FlickerLightMarker, FlickerLightState, effective_interval};
pub use readout::{TextReadout, TextReadoutMarker, ReadoutValue};
pub use radio::{RadioGroup, RadioGroupMarker, RadioMember, RadioSelected, RadioButtonConfig, next_radio_selection};

mod foundation;
pub mod button;
pub mod joystick;
pub mod radar;
pub mod panel;
pub mod progress;
pub mod light;
pub mod readout;
pub mod radio;

// ── Root plugin ───────────────────────────────────────────────────────────────

/// Root plugin for the gui widget library.  Add it once; it pulls in every
/// widget sub-system.
pub struct GuiPlugin;

impl Plugin for GuiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, resolve_visuals_system)
           .add_plugins(button::GuiButtonPlugin)
           .add_plugins(joystick::GuiJoystickPlugin)
           .add_plugins(radar::GuiRadarPlugin)
           .add_plugins(panel::GuiPanelPlugin)
           .add_plugins(progress::GuiProgressPlugin)
           .add_plugins(light::GuiLightPlugin)
           .add_plugins(readout::GuiReadoutPlugin)
           .add_plugins(radio::GuiRadioPlugin);
    }
}
