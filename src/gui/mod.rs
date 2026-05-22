//! Generic GUI widget library for all phone console panels.
//!
//! Add `GuiPlugin` once to the `App`; it registers every widget sub-system.
//! Each widget module is also available directly for callers that need just the
//! types.

use bevy::prelude::*;

pub use border::{
    BorderAssets, BorderConfig, BorderContentArea, CornerSlot, EdgeSlot, GuiBorder,
    GuiBorderPlugin, GuiBorderWidget,
};
pub use button::{
    resolve_click_sound, setup_ui_sounds, spawn_gui_button, ButtonPressed, ButtonSize, ClickSound,
    GuiButtonMarker, UiSounds, WidgetActivated, WidgetDeactivated,
};
pub use foundation::{
    resolve_visual, resolve_visuals_system, Disabled, StateVisuals, Visual, WidgetState,
};
pub use joystick::{
    normalize_joystick, GenericJoystick, GenericJoystickKnob, GenericJoystickPad,
    JoystickDragState, JoystickMoved, JoystickResendTimer,
};
pub use light::{
    effective_interval, FlickerLight, FlickerLightConfig, FlickerLightMarker, FlickerLightState,
};
pub use panel::{lerp_size, GuiPanel, GuiPanelMarker, PanelSize};
pub use progress::{
    filled_segments, ProgressBar, ProgressBarMarker, ProgressBarVariant, ProgressValue,
    SegmentCount,
};
pub use radar::{
    blip_local_offset, default_layer_colour, is_on_radar, layer_to_icon, project_radar_entity,
    tags_to_radar_layer, world_size_to_px, AutoScaleRadar, GenericRadar, GenericRadarWidget,
    HelmRadarWidget, OnRadar, OrientationMode, RadarAppearance, RadarCenter, RadarFilter,
    RadarIcon, RadarIconLookup, RadarLayer, WorldCentredRadar,
};
pub use radio::{
    next_radio_selection, on_radio_member_pressed, RadioButtonConfig, RadioGroup, RadioGroupMarker,
    RadioMember, RadioSelected,
};
pub use readout::{ReadoutValue, TextReadout, TextReadoutMarker};
pub use vignette::{
    GuiVignette, GuiVignettePlugin, GuiVignetteWidget, RedAlertIntensity, RedAlertVignetteMaterial,
    VignetteMaterialHandle,
};

pub mod border;
pub mod button;
mod foundation;
pub mod joystick;
pub mod light;
pub mod panel;
pub mod progress;
pub mod radar;
pub mod radio;
pub mod readout;
pub mod vignette;

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
            .add_plugins(radio::GuiRadioPlugin)
            .add_plugins(border::GuiBorderPlugin)
            .add_plugins(vignette::GuiVignettePlugin);
    }
}
