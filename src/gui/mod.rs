//! Radar widget used by the server viewscreen
//! (`ServerViewscreenRadarPlugin`).
//!
//! Historical note: this module was once a full widget library (buttons,
//! joysticks, panels, …) for the Bevy/WASM phone consoles. Those consoles
//! are now pure HTML/JS (`gui/*.js` + `client.html`), so only the radar
//! widget — still rendered on the server's 3D viewscreen — remains.

pub use radar::{
    apply_zoom_step, blip_local_offset, bridge_sim_to_radar, icon_asset_path, is_on_radar,
    pinch_zoom, project_radar_entity, px_to_world_delta, region_shape_from_snapshot,
    world_size_to_px, AutoScaleRadar, BlipWorldPose, ConsoleRadar, GenericRadar,
    GenericRadarWidget, OnRadar, OrientationMode, RadarAppearance, RadarArc, RadarArcKind,
    RadarArcs, RadarBlipClicked, RadarBlipLabels, RadarBlipMap, RadarCenter, RadarCenterPose,
    RadarClipMode, RadarEntityUuid, RadarFilter, RadarIconLookup, RadarLastGeom,
    RadarRegionNode, RadarTargetHighlight, RadarTargetRing, RadarViewControl, RegionRadarShape,
    WorldCentredRadar, RADAR_MAX_ZOOM, RADAR_MIN_ZOOM,
};

pub mod radar;
