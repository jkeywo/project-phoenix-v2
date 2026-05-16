//! Backwards-compatibility shim — all helm panel logic now lives in
//! `crate::helm_panel` (extracted as part of issue #246).
//!
//! Public items re-exported here so any code still importing from
//! `crate::phone_border::helm` continues to compile without change.

pub use crate::helm_panel::{
    // Plugin
    HelmPanelPlugin,
    // Constants
    COMPASS_RADAR_DIAMETER,
    HELM_PAD_SIZE,
    HELM_KNOB_RADIUS,
    helm_max_radius,
    // Pure helpers
    bearing_ticks,
    range_ring_radii,
    range_ring_labels,
    yaw_to_heading,
    BearingTick,
    // Marker components
    PhoneCompassRadar,
    PhoneCompassRing,
    PhoneCompassTick,
    PhoneHdgReadout,
    PhoneSpdReadout,
    PhoneXReadout,
    PhoneZReadout,
    PhoneRangeRing,
    PhoneThumbRing,
    PhoneHelmPad,
    PhoneHelmKnob,
    PhoneHelmReadout,
    // Resources
    PhoneHelmSpawned,
    PhoneShipSpeed,
    HelmTickTimer,
};
