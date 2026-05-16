//! Phone bezel frame — corners, edges, vignette, status banner and orientation.
//!
//! This module owns the phone bezel frame that wraps console panels. It mirrors
//! the server-side `ViewscreenBorderPlugin` but for the client WASM app (phone
//! HTML). The bezel is always visible (both Lobby and InProgress). It consists of:
//!
//! - 4 corner sprites (top-left, top-right, bottom-left, bottom-right)
//! - 4 edge sprites (top, bottom, left, right) using `NodeImageMode::Tiled`
//! - A `RedAlertVignetteMaterial` material node behind the bezel (pulsing
//!   red glow when Red Alert is active)
//! - A "RED ALERT" status banner at the top centre during Red Alert
//! - A `DeviceOrientation` resource that auto-detects portrait/landscape
//!
//! When Red Alert is active the bezel textures swap to alert variants and
//! the vignette pulses.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui::widget::NodeImageMode;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};

use crate::ship_view::ShipView;

// ── Layout constants ─────────────────────────────────────────────────

const CORNER_SIZE: f32 = 40.0;
const EDGE_THICKNESS: f32 = 16.0;

// ── Resources ────────────────────────────────────────────────────────

/// Holds asset handles for the phone bezel frame.
#[derive(Resource, Debug, Clone)]
pub struct PhoneAssets {
    pub corner_tl: Handle<Image>,
    pub corner_tr: Handle<Image>,
    pub corner_bl: Handle<Image>,
    pub corner_br: Handle<Image>,
    pub edge_top: Handle<Image>,
    pub edge_bottom: Handle<Image>,
    pub edge_left: Handle<Image>,
    pub edge_right: Handle<Image>,
    pub corner_tl_alert: Handle<Image>,
    pub corner_tr_alert: Handle<Image>,
    pub corner_bl_alert: Handle<Image>,
    pub corner_br_alert: Handle<Image>,
    pub edge_top_alert: Handle<Image>,
    pub edge_bottom_alert: Handle<Image>,
    pub edge_left_alert: Handle<Image>,
    pub edge_right_alert: Handle<Image>,
    pub compass_ring: Handle<Image>,
    pub needle: Handle<Image>,
    pub tab_corner: Handle<Image>,
    pub font_display: Handle<Font>,
    pub font_mono: Handle<Font>,
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

/// Cached handle to the single `RedAlertVignetteMaterial` instance so
/// `drive_vignette_intensity` can mutate its uniform without a query.
#[derive(Resource, Debug, Clone)]
struct VignetteMaterialHandle(Handle<RedAlertVignetteMaterial>);

// ── Red Alert vignette material ──────────────────────────────────────

/// `UiMaterial` driving the inset radial-gradient red vignette behind
/// the bezel. Reuses the same shader as the server-side viewscreen border.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RedAlertVignetteMaterial {
    #[uniform(0)]
    pub intensity: f32,
}

impl UiMaterial for RedAlertVignetteMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/red_alert_vignette.wgsl".into()
    }
}

// ── Pulse constants (mirrors viewscreen_border.rs) ───────────────────

const EASE_DURATION: f32 = 0.25;
const PULSE_PERIOD: f32 = 1.3;
const MIN_INTENSITY: f32 = 0.55;
const MAX_INTENSITY: f32 = 1.0;

// ── Marker components ────────────────────────────────────────────────

/// Marks the root `Node` that owns the entire phone bezel frame.
#[derive(Component)]
struct PhoneBorderRoot;

/// Marks the content area inside the bezel where console panels spawn.
#[derive(Component)]
pub struct BezelContentArea;

/// Identifies which border slot a bezel `ImageNode` occupies.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
enum BezelSlot {
    CornerTl,
    CornerTr,
    CornerBl,
    CornerBr,
    EdgeTop,
    EdgeBottom,
    EdgeLeft,
    EdgeRight,
}

impl BezelSlot {
    fn handle<'a>(self, assets: &'a PhoneAssets, alert: bool) -> &'a Handle<Image> {
        match (self, alert) {
            (Self::CornerTl, false) => &assets.corner_tl,
            (Self::CornerTl, true) => &assets.corner_tl_alert,
            (Self::CornerTr, false) => &assets.corner_tr,
            (Self::CornerTr, true) => &assets.corner_tr_alert,
            (Self::CornerBl, false) => &assets.corner_bl,
            (Self::CornerBl, true) => &assets.corner_bl_alert,
            (Self::CornerBr, false) => &assets.corner_br,
            (Self::CornerBr, true) => &assets.corner_br_alert,
            (Self::EdgeTop, false) => &assets.edge_top,
            (Self::EdgeTop, true) => &assets.edge_top_alert,
            (Self::EdgeBottom, false) => &assets.edge_bottom,
            (Self::EdgeBottom, true) => &assets.edge_bottom_alert,
            (Self::EdgeLeft, false) => &assets.edge_left,
            (Self::EdgeLeft, true) => &assets.edge_left_alert,
            (Self::EdgeRight, false) => &assets.edge_right,
            (Self::EdgeRight, true) => &assets.edge_right_alert,
        }
    }
}

/// Marks the status banner "RED ALERT" text node.
#[derive(Component)]
struct AlertBannerText;

// ── Plugin ───────────────────────────────────────────────────────────

/// Loads phone bezel assets, registers the Red Alert vignette material,
/// and renders the bezel frame. The bezel is always visible.
pub struct PhoneBorderPlugin;

impl Plugin for PhoneBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<RedAlertVignetteMaterial>::default())
            .init_resource::<DeviceOrientation>()
            .add_systems(Startup, (load_phone_assets, spawn_bezel_on_startup).chain())
            .add_systems(
                Update,
                (
                    detect_orientation,
                    reparent_panels_into_bezel,
                    swap_bezel_textures,
                    drive_vignette_intensity,
                    refresh_alert_banner,
                ),
            );
    }
}

// ── Systems ──────────────────────────────────────────────────────────

fn load_phone_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    let assets = PhoneAssets {
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
        compass_ring: asset_server.load("phone_border/compass-ring.png"),
        needle: asset_server.load("phone_border/needle.png"),
        tab_corner: asset_server.load("phone_border/tab-corner.png"),
        font_display: asset_server.load("fonts/ChakraPetch-SemiBold.ttf"),
        font_mono: asset_server.load("fonts/JetBrainsMono-Regular.ttf"),
    };
    commands.insert_resource(assets);
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

/// Spawn the bezel frame at app startup. The bezel is always visible.
fn spawn_bezel_on_startup(
    mut commands: Commands,
    assets: Res<PhoneAssets>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
) {
    let vignette = materials.add(RedAlertVignetteMaterial { intensity: 0.0 });
    commands.insert_resource(VignetteMaterialHandle(vignette.clone()));
    spawn_bezel_frame(&mut commands, &assets, vignette);
}

/// Marker resource set once the reparenting has been done.
#[derive(Resource, Default)]
#[allow(dead_code)]
struct PanelsReparented;

/// Reparent existing console panel root entities into the bezel content
/// area so they render inside the bezel frame. Runs once when the bezel
/// is first spawned (detected by the presence of both `BezelContentArea`
/// and panel roots that are not yet its children).
fn reparent_panels_into_bezel(
    mut commands: Commands,
    content_area: Query<Entity, With<BezelContentArea>>,
    captain: Query<Entity, With<crate::client_app::CaptainPanel>>,
    helm: Query<Entity, With<crate::client_app::HelmPanel>>,
    lobby: Query<Entity, With<crate::client_app::LobbyRoot>>,
    sensors: Query<Entity, With<crate::client_app::SensorsPanel>>,
    shields: Query<Entity, With<crate::client_app::ShieldsPanel>>,
    navigation: Query<Entity, With<crate::client_app::NavigationPanel>>,
    weapons: Query<Entity, With<crate::client_app::WeaponsPanel>>,
    tab_bar: Query<Entity, With<crate::client_app::TabBarRoot>>,
    mut reparented: Local<HashSet<Entity>>,
) {
    let Ok(target) = content_area.single() else { return };
    for entity in lobby.iter().chain(captain.iter()).chain(helm.iter()).chain(sensors.iter()).chain(shields.iter()).chain(navigation.iter()).chain(weapons.iter()).chain(tab_bar.iter()) {
        if reparented.insert(entity) {
            commands.entity(entity).set_parent_in_place(target);
        }
    }
}

/// Rewrites each bezel `ImageNode`'s image handle to the alert or normal
/// variant whenever `ShipView.red_alert` changes.
fn swap_bezel_textures(
    ship_view: Option<Res<ShipView>>,
    assets: Option<Res<PhoneAssets>>,
    mut q: Query<(&BezelSlot, &mut ImageNode)>,
) {
    let Some(ship_view) = ship_view else { return };
    let Some(assets) = assets else { return };
    if !ship_view.is_changed() {
        return;
    }
    let alert = ship_view.red_alert;
    for (slot, mut image_node) in q.iter_mut() {
        image_node.image = slot.handle(&assets, alert).clone();
    }
}

/// Per-frame system that drives the vignette material's `intensity`
/// uniform via the pure `pulse_intensity` helper.
fn drive_vignette_intensity(
    time: Res<Time>,
    ship_view: Option<Res<ShipView>>,
    handle: Option<Res<VignetteMaterialHandle>>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
) {
    let Some(ship_view) = ship_view else { return };
    let Some(handle) = handle else { return };
    let Some(material) = materials.get_mut(&handle.0) else { return };
    material.intensity = pulse_intensity(
        time.elapsed_secs(),
        ship_view.red_alert,
        material.intensity,
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

// ── Spawning ─────────────────────────────────────────────────────────

fn spawn_bezel_frame(
    commands: &mut Commands,
    assets: &PhoneAssets,
    vignette: Handle<RedAlertVignetteMaterial>,
) {
    commands
        .spawn((
            PhoneBorderRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ))
        .with_children(|parent| {
            // ── Vignette (spawned first so border sprites occlude it) ─
            parent.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                MaterialNode(vignette),
            ));

            // ── Content area ────────────────────────────────────────
            parent.spawn((
                BezelContentArea,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(CORNER_SIZE),
                    left: Val::Px(EDGE_THICKNESS),
                    right: Val::Px(EDGE_THICKNESS),
                    bottom: Val::Px(CORNER_SIZE),
                    overflow: Overflow::clip(),
                    ..default()
                },
            ));

            // ── Corners ──────────────────────────────────────────────
            parent.spawn((
                BezelSlot::CornerTl,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Px(CORNER_SIZE),
                    height: Val::Px(CORNER_SIZE),
                    ..default()
                },
                ImageNode::new(assets.corner_tl.clone()),
            ));
            parent.spawn((
                BezelSlot::CornerTr,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    right: Val::Px(0.0),
                    width: Val::Px(CORNER_SIZE),
                    height: Val::Px(CORNER_SIZE),
                    ..default()
                },
                ImageNode::new(assets.corner_tr.clone()),
            ));
            parent.spawn((
                BezelSlot::CornerBl,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Px(CORNER_SIZE),
                    height: Val::Px(CORNER_SIZE),
                    ..default()
                },
                ImageNode::new(assets.corner_bl.clone()),
            ));
            parent.spawn((
                BezelSlot::CornerBr,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    right: Val::Px(0.0),
                    width: Val::Px(CORNER_SIZE),
                    height: Val::Px(CORNER_SIZE),
                    ..default()
                },
                ImageNode::new(assets.corner_br.clone()),
            ));

            // ── Edges ────────────────────────────────────────────────
            // Top edge — between TL and TR corners.
            parent.spawn((
                BezelSlot::EdgeTop,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(CORNER_SIZE),
                    right: Val::Px(CORNER_SIZE),
                    height: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_top.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: true,
                        tile_y: false,
                        stretch_value: 1.0,
                    }),
            ));

            // Bottom edge — between BL and BR corners.
            parent.spawn((
                BezelSlot::EdgeBottom,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Px(CORNER_SIZE),
                    right: Val::Px(CORNER_SIZE),
                    height: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_bottom.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: true,
                        tile_y: false,
                        stretch_value: 1.0,
                    }),
            ));

            // Left edge — between TL and BL corners.
            parent.spawn((
                BezelSlot::EdgeLeft,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(CORNER_SIZE),
                    bottom: Val::Px(CORNER_SIZE),
                    width: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_left.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: false,
                        tile_y: true,
                        stretch_value: 1.0,
                    }),
            ));

            // Right edge — between TR and BR corners.
            parent.spawn((
                BezelSlot::EdgeRight,
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(0.0),
                    top: Val::Px(CORNER_SIZE),
                    bottom: Val::Px(CORNER_SIZE),
                    width: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_right.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: false,
                        tile_y: true,
                        stretch_value: 1.0,
                    }),
            ));

            // ── Status banner — "RED ALERT" at top centre ────────────
            parent
                .spawn((
                    AlertBannerText,
                    Text::new("RED ALERT"),
                    TextFont {
                        font: assets.font_display.clone(),
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.2, 0.2)),
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(CORNER_SIZE + 4.0),
                        left: Val::Percent(50.0),
                        margin: UiRect {
                            left: Val::Px(-60.0),
                            ..default()
                        },
                        ..default()
                    },
                    Visibility::Hidden,
                ));
        });
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
        // Portrait: width < height → Portrait
        assert_eq!(
            DeviceOrientation::default(),
            DeviceOrientation::Portrait,
            "default should be Portrait"
        );
    }

    #[test]
    fn slot_handle_picks_normal_or_alert_variant() {
        let assets = test_assets();
        let normal = BezelSlot::CornerTl.handle(&assets, false);
        let alert = BezelSlot::CornerTl.handle(&assets, true);
        assert_ne!(normal.id(), alert.id());
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

    fn test_assets() -> PhoneAssets {
        use bevy::asset::uuid::Uuid;
        let h = |n: u128| -> Handle<Image> { Uuid::from_u128(n).into() };
        let f = |n: u128| -> Handle<Font> { Uuid::from_u128(n).into() };
        PhoneAssets {
            corner_tl: h(1),
            corner_tr: h(2),
            corner_bl: h(3),
            corner_br: h(4),
            edge_top: h(5),
            edge_bottom: h(6),
            edge_left: h(7),
            edge_right: h(8),
            corner_tl_alert: h(9),
            corner_tr_alert: h(10),
            corner_bl_alert: h(11),
            corner_br_alert: h(12),
            edge_top_alert: h(13),
            edge_bottom_alert: h(14),
            edge_left_alert: h(15),
            edge_right_alert: h(16),
            compass_ring: h(17),
            needle: h(18),
            tab_corner: h(19),
            font_display: f(20),
            font_mono: f(21),
        }
    }
}
