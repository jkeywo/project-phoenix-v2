//! Phone bezel frame — corners, edges, status banner and orientation.
//!
//! This module owns the phone bezel frame that wraps console panels.  It
//! is a slimmer cousin of the server-side `ViewscreenBorderPlugin`: the
//! phone gets the 9-slice frame and red-alert texture swap, but NOT the
//! fullscreen radial vignette (that overlay only makes sense on the
//! viewscreen).  The bezel is always visible (both Lobby and InProgress).
//!
//! The 9-slice border itself is now built by `GuiBorderWidget::spawn` from
//! the `gui` library; this module handles phone-specific wiring:
//!
//! - Populating `BorderAssets` with the phone bezel textures
//! - Spawning the `GuiBorderWidget` at startup
//! - Driving the shared `RedAlertIntensity` resource (pulse math)
//! - Showing/hiding the "RED ALERT" status banner
//! - Reparenting console panels into the safe content area
//! - Detecting device orientation

use bevy::prelude::*;

use crate::gui::{
    BorderAssets, BorderConfig, BorderContentArea,
    GuiBorderWidget,
    RedAlertIntensity,
};
use crate::ship_view::ShipView;

// ── Resources ────────────────────────────────────────────────────────

/// Holds non-border phone assets: compass ring, needle, tab corner, fonts,
/// plus console-panel widget textures (buttons, panels, joysticks, radar, etc.).
/// Border textures (corners + edges) now live in `BorderAssets`.
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
}

/// Auto-detected device orientation, updated each frame from the window
/// aspect ratio.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub enum DeviceOrientation {
    Portrait,
    Landscape,
}

impl Default for DeviceOrientation {
    fn default() -> Self {
        Self::Portrait
    }
}

/// Returns `true` when the device is in landscape orientation.
///
/// Accepts `Option<&DeviceOrientation>` so callers can pass
/// `orientation.as_deref()` directly from an `Option<Res<DeviceOrientation>>`.
pub fn is_landscape(orientation: Option<&DeviceOrientation>) -> bool {
    matches!(orientation, Some(DeviceOrientation::Landscape))
}

// ── Pulse constants (mirrors viewscreen_border.rs) ───────────────────

const EASE_DURATION: f32 = 0.25;
const PULSE_PERIOD: f32 = 1.3;
const MIN_INTENSITY: f32 = 0.55;
const MAX_INTENSITY: f32 = 1.0;

// ── Marker components ────────────────────────────────────────────────

/// Marks the status banner "RED ALERT" text node.
#[derive(Component)]
struct AlertBannerText;

// ── Plugin ───────────────────────────────────────────────────────────

/// Loads phone bezel assets, borders, and spawns the bezel frame at
/// startup.  The bezel is always visible in both lobby and in-progress
/// phases.
///
/// Unlike the viewscreen, this plugin deliberately does NOT spawn a
/// fullscreen `RedAlertVignetteMaterial` overlay — the phone is a control
/// surface and a screen-filling red gradient there only adds noise.  The
/// shared `RedAlertIntensity` resource is still driven so that the border
/// texture swap continues to work.
pub struct PhoneBorderPlugin;

impl Plugin for PhoneBorderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DeviceOrientation>()
            .init_resource::<RedAlertIntensity>()
            .add_systems(Startup, (load_phone_assets, spawn_bezel_on_startup).chain())
            .add_systems(
                Update,
                (
                    detect_orientation,
                    reparent_panels_into_bezel,
                    update_red_alert_intensity,
                    refresh_alert_banner,
                ),
            );
    }
}

// ── Systems ──────────────────────────────────────────────────────────

fn load_phone_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    // Populate PhoneAssets (non-border resources used by other consoles)
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
    };
    commands.insert_resource(phone);

    // Populate BorderAssets (9-slice border textures)
    let border = BorderAssets {
        corner_tl: asset_server.load("phone_border/bezel-corner-tl.png"),
        corner_tr: asset_server.load("phone_border/bezel-corner-tr.png"),
        corner_bl: asset_server.load("phone_border/bezel-corner-bl.png"),
        corner_br: asset_server.load("phone_border/bezel-corner-br.png"),
        edge_top: asset_server.load("phone_border/bezel-edge-top.png"),
        edge_bottom: asset_server.load("phone_border/bezel-edge-bottom.png"),
        edge_left: asset_server.load("phone_border/bezel-edge-left.png"),
        edge_right: asset_server.load("phone_border/bezel-edge-right.png"),
        corner_tl_alert: asset_server.load("phone_border/bezel-corner-tl-alert.png"),
        corner_tr_alert: asset_server.load("phone_border/bezel-corner-tr-alert.png"),
        corner_bl_alert: asset_server.load("phone_border/bezel-corner-bl-alert.png"),
        corner_br_alert: asset_server.load("phone_border/bezel-corner-br-alert.png"),
        edge_top_alert: asset_server.load("phone_border/bezel-edge-top-alert.png"),
        edge_bottom_alert: asset_server.load("phone_border/bezel-edge-bottom-alert.png"),
        edge_left_alert: asset_server.load("phone_border/bezel-edge-left-alert.png"),
        edge_right_alert: asset_server.load("phone_border/bezel-edge-right-alert.png"),
    };
    commands.insert_resource(border);
}

/// Detect device orientation from window aspect ratio. Updated each frame
/// but only inserted once; change detection avoids pointless writes.
fn detect_orientation(
    windows: Query<&Window>,
    mut orientation: ResMut<DeviceOrientation>,
) {
    let Ok(window) = windows.single() else { return };
    let aspect = window.width() / window.height();
    let new = if aspect >= 1.0 {
        DeviceOrientation::Landscape
    } else {
        DeviceOrientation::Portrait
    };
    if new != *orientation {
        *orientation = new;
    }
}

/// Spawn the bezel frame at app startup using `GuiBorderWidget`. Also
/// spawns the "RED ALERT" status banner.
///
/// The radial vignette overlay deliberately lives ONLY on the server
/// viewscreen (`ViewscreenBorderPlugin`).  The phone client is meant to be
/// a control surface, not a viewport into space, so a fullscreen red
/// gradient there only adds visual noise without conveying useful state.
fn spawn_bezel_on_startup(
    mut commands: Commands,
    border_assets: Res<BorderAssets>,
    phone_assets: Res<PhoneAssets>,
) {
    // Spawn the 9-slice border via the gui library widget.
    GuiBorderWidget::spawn(&mut commands, &border_assets, &BorderConfig::default(), false);

    // Spawn the "RED ALERT" banner (phone-specific, not part of generic border).
    //
    // Positioned BELOW the tab bar so it doesn't overlap the console
    // selection tabs.  Bezel top inset is 40px (corner_size); tab bar
    // adds another ~36px height.  Banner sits just below at ~84px.
    commands.spawn((
        AlertBannerText,
        Text::new("RED ALERT"),
        TextFont {
            font: phone_assets.font_display.clone(),
            font_size: 18.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.2, 0.2)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(84.0),
            left: Val::Percent(50.0),
            margin: UiRect { left: Val::Px(-60.0), ..default() },
            ..default()
        },
        Visibility::Hidden,
    ));
}

/// Reparent existing console panel root entities into the bezel content
/// area so they render inside the bezel frame.
///
/// Runs every frame but is effectively a no-op once all panels are parented.
/// Rather than caching entity IDs in a `Local` set (which becomes stale when
/// orientation changes force a full despawn+respawn of a panel), we check
/// each panel's current parent every frame. If it is already `target`, we
/// skip it — no command queued, minimal overhead.  This correctly handles
/// respawned panels (new entity, no parent yet) without any bookkeeping.
fn reparent_panels_into_bezel(
    mut commands: Commands,
    content_area: Query<Entity, With<BorderContentArea>>,
    captain: Query<Entity, With<crate::client_app::CaptainPanel>>,
    helm: Query<Entity, With<crate::client_app::HelmPanel>>,
    lobby: Query<Entity, With<crate::client_app::LobbyRoot>>,
    sensors: Query<Entity, With<crate::sensors_panel::SensorsPanel>>,
    shields: Query<Entity, With<crate::shields_panel::ShieldsPanel>>,
    navigation: Query<Entity, With<crate::navigation_panel::NavigationPanel>>,
    weapons: Query<Entity, With<crate::client_app::WeaponsPanel>>,
    hull_bar: Query<Entity, With<crate::ship_view::ConsoleHullBarBg>>,
    parents: Query<&ChildOf>,
) {
    let Ok(target) = content_area.single() else { return };
    for entity in lobby.iter().chain(captain.iter()).chain(helm.iter()).chain(sensors.iter()).chain(shields.iter()).chain(navigation.iter()).chain(weapons.iter()).chain(hull_bar.iter()) {
        // Skip if already a direct child of the content area.
        if parents.get(entity).map(|p| p.parent() == target).unwrap_or(false) {
            continue;
        }
        commands.entity(entity).set_parent_in_place(target);
    }
}

/// Each frame: writes the pulse-computed intensity into the shared
/// `RedAlertIntensity` resource, which drives both the vignette material
/// (`GuiVignettePlugin`) and the border texture swap (`GuiBorderPlugin`).
fn update_red_alert_intensity(
    time: Res<Time>,
    ship_view: Option<Res<ShipView>>,
    mut intensity: ResMut<RedAlertIntensity>,
) {
    let Some(ship_view) = ship_view else { return };
    let prev = intensity.0;
    intensity.0 = pulse_intensity(
        time.elapsed_secs(),
        ship_view.red_alert,
        prev,
        time.delta_secs(),
    );
}

/// Shows/hides the "RED ALERT" status banner text based on Red Alert state.
fn refresh_alert_banner(
    ship_view: Option<Res<ShipView>>,
    mut banner: Query<&mut Visibility, With<AlertBannerText>>,
) {
    let Some(ship_view) = ship_view else { return };
    for mut vis in banner.iter_mut() {
        *vis = if ship_view.red_alert {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

// ── Pure helpers ─────────────────────────────────────────────────────

/// State-transition function for the Red Alert vignette intensity.
/// Mirrors `viewscreen_border::pulse_intensity`.
pub fn pulse_intensity(time_secs: f32, red_alert: bool, prev_intensity: f32, dt: f32) -> f32 {
    let max_step = (MAX_INTENSITY / EASE_DURATION) * dt;
    if red_alert {
        let target = sine_pulse(time_secs);
        approach(prev_intensity, target, max_step)
    } else if prev_intensity <= 0.0 {
        0.0
    } else {
        approach(prev_intensity, 0.0, max_step).max(0.0)
    }
}

fn sine_pulse(time_secs: f32) -> f32 {
    let mid = (MIN_INTENSITY + MAX_INTENSITY) * 0.5;
    let amp = (MAX_INTENSITY - MIN_INTENSITY) * 0.5;
    mid + amp * (std::f32::consts::TAU * time_secs / PULSE_PERIOD).sin()
}

fn approach(current: f32, target: f32, max_step: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= max_step {
        target
    } else if delta > 0.0 {
        current + max_step
    } else {
        current - max_step
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const DT_60HZ: f32 = 1.0 / 60.0;

    fn max_step_per_frame() -> f32 {
        (MAX_INTENSITY / EASE_DURATION) * DT_60HZ
    }

    #[test]
    fn idle_stays_at_zero() {
        let out = pulse_intensity(0.0, false, 0.0, DT_60HZ);
        assert_eq!(out, 0.0);

        let out = pulse_intensity(123.4, false, 0.0, DT_60HZ);
        assert_eq!(out, 0.0);
    }

    #[test]
    fn alert_on_rises_monotonically_during_ease_window() {
        let mut prev = 0.0;
        let mut t = 0.0;
        let frames_in_ease = (EASE_DURATION / DT_60HZ).ceil() as usize;
        for _ in 0..frames_in_ease {
            let next = pulse_intensity(t, true, prev, DT_60HZ);
            assert!(
                next >= prev - 1e-6,
                "intensity decreased during alert-on ease (prev={prev}, next={next})"
            );
            prev = next;
            t += DT_60HZ;
        }
        assert!(
            prev >= MIN_INTENSITY - 1e-3,
            "intensity {prev} did not reach pulse band after {EASE_DURATION}s ease"
        );
    }

    #[test]
    fn alert_off_decays_smoothly_to_zero_within_ease_window() {
        let mut prev = MAX_INTENSITY;
        let mut t = 0.0;
        let frames = (EASE_DURATION / DT_60HZ).ceil() as usize + 2;
        let mut hit_zero = false;
        for _ in 0..frames {
            let next = pulse_intensity(t, false, prev, DT_60HZ);
            assert!(
                next <= prev + 1e-6,
                "intensity increased during alert-off decay (prev={prev}, next={next})"
            );
            assert!(next >= 0.0, "intensity went negative");
            if next == 0.0 {
                hit_zero = true;
            }
            prev = next;
            t += DT_60HZ;
        }
        assert!(hit_zero, "intensity did not reach 0 within {EASE_DURATION}s ease");

        let next = pulse_intensity(t, false, 0.0, DT_60HZ);
        assert_eq!(next, 0.0);
    }

    #[test]
    fn steady_state_pulse_stays_within_band() {
        let mut prev = MIN_INTENSITY;
        let mut t = 0.0;
        let frames = ((PULSE_PERIOD * 3.0) / DT_60HZ).ceil() as usize;
        let mut min_seen = f32::INFINITY;
        let mut max_seen = f32::NEG_INFINITY;
        for _ in 0..frames {
            let next = pulse_intensity(t, true, prev, DT_60HZ);
            min_seen = min_seen.min(next);
            max_seen = max_seen.max(next);
            prev = next;
            t += DT_60HZ;
        }
        let slack = max_step_per_frame() + 1e-3;
        assert!(
            min_seen >= MIN_INTENSITY - slack,
            "min {min_seen} below band lower bound {MIN_INTENSITY}"
        );
        assert!(
            max_seen <= MAX_INTENSITY + 1e-3,
            "max {max_seen} above band upper bound {MAX_INTENSITY}"
        );
    }

    #[test]
    fn orientation_detects_both_modes() {
        assert_eq!(
            DeviceOrientation::default(),
            DeviceOrientation::Portrait,
            "default should be Portrait"
        );
    }

    #[test]
    fn approach_snaps_when_within_step() {
        assert_eq!(approach(0.5, 0.51, 0.1), 0.51);
        assert_eq!(approach(0.5, 0.49, 0.1), 0.49);
    }

    #[test]
    fn approach_steps_toward_target_when_outside_step() {
        assert!((approach(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
        assert!((approach(1.0, 0.0, 0.25) - 0.75).abs() < 1e-6);
    }
}
