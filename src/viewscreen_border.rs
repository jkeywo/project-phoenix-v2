//! Viewscreen border asset loading.
//!
//! This module establishes the asset-loading pipeline for the viewscreen
//! frame work tracked by PRD #180. It does not render anything yet; it
//! only loads one PNG, one font, and the placeholder WGSL shader at
//! startup and stores the handles in a resource so later slices can
//! depend on them.
//!
//! The rest of the viewscreen frame (static border layout, red-alert
//! vignette, designation HUD) lands in follow-up issues #182, #183, #184.
//!
//! Server-only — gated by the `server` feature in `lib.rs`.

use bevy::prelude::*;

/// Holds asset handles for the viewscreen border frame.
///
/// Inserted at startup by [`ViewscreenBorderPlugin`]. Holding the handles
/// in a resource keeps the assets alive (Bevy reference-counts handles)
/// and gives later systems a stable place to look them up.
#[derive(Resource, Debug, Clone)]
pub struct ViewscreenAssets {
    /// One representative border PNG. The full set lands in #182.
    pub corner_tl: Handle<Image>,
    /// Display font for HUD readouts (added in #184).
    pub font_display: Handle<Font>,
    /// Placeholder WGSL for the red-alert vignette (body lands in #183).
    pub vignette_shader: Handle<Shader>,
}

/// Loads viewscreen border assets at startup.
///
/// Stub plugin: spawns no entities and renders nothing. Its only job is
/// to prove the asset pipeline is wired correctly.
pub struct ViewscreenBorderPlugin;

impl Plugin for ViewscreenBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_viewscreen_assets);
    }
}

fn load_viewscreen_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    let assets = ViewscreenAssets {
        corner_tl: asset_server.load("viewscreen/corner-tl.png"),
        font_display: asset_server.load("fonts/ChakraPetch-SemiBold.ttf"),
        vignette_shader: asset_server.load("shaders/red_alert_vignette.wgsl"),
    };
    commands.insert_resource(assets);
}
