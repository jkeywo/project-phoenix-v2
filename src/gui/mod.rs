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
    spawn_gui_button, ButtonPressed, ButtonSize,
    GuiButtonMarker, WidgetActivated, WidgetDeactivated,
};
pub use foundation::{
    resolve_visual, resolve_visuals_system, Disabled, StateVisuals, Visual, WidgetState,
};
pub use joystick::{
    normalize_joystick, reset_joystick_drag, should_emit_resend, GenericJoystick,
    GenericJoystickKnob, GenericJoystickPad, JoystickDragState, JoystickMoved,
    JoystickResendTimer,
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
    blip_local_offset, bridge_sim_to_radar, icon_from_radar_icon_str, is_on_radar,
    project_radar_entity, region_shape_from_snapshot, world_size_to_px, AutoScaleRadar,
    BlipWorldPose, ConsoleRadar, GenericRadar, GenericRadarWidget, OnRadar, OrientationMode,
    RadarAppearance, RadarArc, RadarArcKind, RadarArcs, RadarBlipClicked, RadarBlipMap,
    RadarCenter, RadarCenterPose, RadarClipMode, RadarEntityUuid, RadarFilter, RadarIcon,
    RadarIconLookup, RadarRegionNode, RadarTargetHighlight, RegionRadarShape, WorldCentredRadar,
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
