//! Phone asset loader and device orientation detection.
//!
//! Pre-#442 this module owned the phone bezel frame (corners, edges,
//! status banner) and reparented panels into it. Issue #442 moved all of
//! that to the HTML/JS shell in `client.html`; this module is now a thin
//! wrapper that:
//!
//! - loads the per-console image and font handles into [`PhoneAssets`],
//! - populates the shared [`RadarIconLookup`] from those handles.
//!
//! Device-orientation detection (the `DeviceOrientation` resource,
//! `is_landscape()` helper, and `detect_orientation` system) was ported to
//! pure JS in issue #462; see gui/device-orientation.js (a singleton exposed
//! as `window.currentOrientation()` and refreshed on `window.resize`). Console
//! layouts already consume a `[data-orientation]` attribute driven from there.
//!
//! It deliberately does NOT load the 9-slice bezel border art — the HTML
//! bezel ships those PNGs via CSS, and the server's
//! `ViewscreenBorderPlugin` has its own asset loader for the desktop
//! viewscreen frame. The `BorderAssets` resource is left in place in
//! `src/gui/border.rs` for future reuse but is no longer populated by
//! the client.

use bevy::prelude::*;

use crate::gui::{RadarIcon, RadarIconLookup};

// ── Resources ────────────────────────────────────────────────────────

/// Handles to the six radar-blip icon PNGs.
#[derive(Clone, Debug)]
pub struct RadarIconHandles {
    pub ship: Handle<Image>,
    pub player_ship: Handle<Image>,
    pub asteroid: Handle<Image>,
    pub station: Handle<Image>,
    pub planet: Handle<Image>,
    pub star: Handle<Image>,
    pub torpedo: Handle<Image>,
}

/// Holds the per-console image and font handles every panel uses. Loaded
/// once at startup by [`load_phone_assets`] and consumed via
/// `Option<Res<PhoneAssets>>` by each panel's spawn system.
#[derive(Resource, Debug, Clone)]
pub struct PhoneAssets {
    pub compass_ring: Handle<Image>,
    pub needle: Handle<Image>,
    pub tab_corner: Handle<Image>,
    pub font_display: Handle<Font>,
    pub font_mono: Handle<Font>,
    // ── gui/ button-normal ──
    pub btn_normal_idle: Handle<Image>,
    pub btn_normal_hover: Handle<Image>,
    pub btn_normal_active: Handle<Image>,
    pub btn_normal_press: Handle<Image>,
    // ── gui/ button-small ──
    pub btn_small_idle: Handle<Image>,
    pub btn_small_hover: Handle<Image>,
    pub btn_small_active: Handle<Image>,
    pub btn_small_press: Handle<Image>,
    // ── helm_console/ ──
    pub helm_panel_bg: Handle<Image>,
    pub impulse_ready: Handle<Image>,
    pub impulse_idle: Handle<Image>,
    pub impulse_hover: Handle<Image>,
    pub impulse_active: Handle<Image>,
    pub impulse_press: Handle<Image>,
    pub joystick_knob_idle: Handle<Image>,
    pub joystick_knob_hover: Handle<Image>,
    pub joystick_knob_active: Handle<Image>,
    pub joystick_knob_press: Handle<Image>,
    pub joystick_pad_idle: Handle<Image>,
    pub joystick_pad_active: Handle<Image>,
    pub radar_bg: Handle<Image>,
    pub radar_surround: Handle<Image>,
    // ── captain_console/ ──
    pub captain_panel_bg: Handle<Image>,
    pub red_alert_idle: Handle<Image>,
    pub red_alert_hover: Handle<Image>,
    pub red_alert_active: Handle<Image>,
    pub red_alert_press: Handle<Image>,
    pub red_alert_armed: Handle<Image>,
    pub inset_card: Handle<Image>,
    // ── radar_icons/ ──
    pub radar_icons: RadarIconHandles,
}

// Device orientation (`DeviceOrientation` resource + `is_landscape()` helper)
// was ported to pure JS in issue #462; see gui/device-orientation.js.

// ── Plugin ───────────────────────────────────────────────────────────

/// Loads phone-panel asset handles into [`PhoneAssets`] and the shared
/// [`RadarIconLookup`].
///
/// Pre-#442 this plugin also spawned the Bevy phone bezel frame, drove
/// the red-alert texture swap, and reparented console panels into the
/// bezel safe zone. All three responsibilities moved to the HTML shell
/// (`client.html` issues #439/#440/#441). Orientation detection moved to
/// pure JS in #462 (gui/device-orientation.js); only asset loading remains
/// on the Rust side.
///
/// The plugin name is retained for compatibility with
/// `add_client_plugins`. A rename to `PhoneAssetsPlugin` is deferred to
/// avoid churning the registration sites; the behavioural shape that
/// matters is the resources it inserts.
pub struct PhoneBorderPlugin;

impl Plugin for PhoneBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_phone_assets)
            .add_systems(Update, populate_radar_icon_lookup);
    }
}

// ── Systems ──────────────────────────────────────────────────────────

fn load_phone_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    let phone = PhoneAssets {
        compass_ring: asset_server.load("phone_border/compass-ring.png"),
        needle: asset_server.load("phone_border/needle.png"),
        tab_corner: asset_server.load("phone_border/tab-corner.png"),
        font_display: asset_server.load("fonts/ChakraPetch-SemiBold.ttf"),
        font_mono: asset_server.load("fonts/JetBrainsMono-Regular.ttf"),
        // ── gui/ button-normal ──
        btn_normal_idle: asset_server.load("gui/button-normal-idle.png"),
        btn_normal_hover: asset_server.load("gui/button-normal-hover.png"),
        btn_normal_active: asset_server.load("gui/button-normal-active.png"),
        btn_normal_press: asset_server.load("gui/button-normal-press.png"),
        // ── gui/ button-small ──
        btn_small_idle: asset_server.load("gui/button-small-idle.png"),
        btn_small_hover: asset_server.load("gui/button-small-hover.png"),
        btn_small_active: asset_server.load("gui/button-small-active.png"),
        btn_small_press: asset_server.load("gui/button-small-press.png"),
        // ── helm_console/ ──
        helm_panel_bg: asset_server.load("helm_console/panel-bg.png"),
        impulse_ready: asset_server.load("helm_console/impulse-ready.png"),
        impulse_idle: asset_server.load("helm_console/impulse-idle.png"),
        impulse_hover: asset_server.load("helm_console/impulse-hover.png"),
        impulse_active: asset_server.load("helm_console/impulse-active.png"),
        impulse_press: asset_server.load("helm_console/impulse-press.png"),
        joystick_knob_idle: asset_server.load("helm_console/joystick-knob-idle.png"),
        joystick_knob_hover: asset_server.load("helm_console/joystick-knob-hover.png"),
        joystick_knob_active: asset_server.load("helm_console/joystick-knob-active.png"),
        joystick_knob_press: asset_server.load("helm_console/joystick-knob-press.png"),
        joystick_pad_idle: asset_server.load("helm_console/joystick-pad-idle.png"),
        joystick_pad_active: asset_server.load("helm_console/joystick-pad-active.png"),
        radar_bg: asset_server.load("helm_console/radar-bg.png"),
        radar_surround: asset_server.load("helm_console/radar-surround.png"),
        // ── captain_console/ ──
        captain_panel_bg: asset_server.load("captain_console/panel-bg.png"),
        red_alert_idle: asset_server.load("captain_console/red-alert-idle.png"),
        red_alert_hover: asset_server.load("captain_console/red-alert-hover.png"),
        red_alert_active: asset_server.load("captain_console/red-alert-active.png"),
        red_alert_press: asset_server.load("captain_console/red-alert-press.png"),
        red_alert_armed: asset_server.load("captain_console/red-alert-armed.png"),
        inset_card: asset_server.load("captain_console/inset-card.png"),
        radar_icons: RadarIconHandles {
            ship: asset_server.load("radar_icons/Icon-Ship.png"),
            player_ship: asset_server.load("radar_icons/Icon-PlayerShip.png"),
            asteroid: asset_server.load("radar_icons/Icon-Asteroid.png"),
            station: asset_server.load("radar_icons/Icon-Station.png"),
            planet: asset_server.load("radar_icons/Icon-Planet.png"),
            star: asset_server.load("radar_icons/Icon-Star.png"),
            torpedo: asset_server.load("radar_icons/Icon-Torpedo.png"),
        },
    };
    commands.insert_resource(phone);
}

/// Populate the shared `RadarIconLookup` from `PhoneAssets.radar_icons`
/// once those assets are loaded. Idempotent: runs every frame but is a
/// no-op after first population.
fn populate_radar_icon_lookup(
    assets: Option<Res<PhoneAssets>>,
    mut lookup: ResMut<RadarIconLookup>,
) {
    if !lookup.0.is_empty() {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    lookup
        .0
        .insert(RadarIcon::Ship, assets.radar_icons.ship.clone());
    lookup
        .0
        .insert(RadarIcon::PlayerShip, assets.radar_icons.player_ship.clone());
    lookup
        .0
        .insert(RadarIcon::Asteroid, assets.radar_icons.asteroid.clone());
    lookup
        .0
        .insert(RadarIcon::Station, assets.radar_icons.station.clone());
    lookup
        .0
        .insert(RadarIcon::Planet, assets.radar_icons.planet.clone());
    lookup
        .0
        .insert(RadarIcon::Star, assets.radar_icons.star.clone());
    lookup
        .0
        .insert(RadarIcon::Torpedo, assets.radar_icons.torpedo.clone());
}

// Device-orientation detection (`detect_orientation`) and its tests
// (`orientation_default_is_portrait`, `is_landscape_*`) were ported to pure JS
// in issue #462; the equivalent logic + tests now live in
// gui/device-orientation.js and tests/client/device-orientation.test.js.
