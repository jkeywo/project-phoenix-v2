//! Viewscreen border frame.
//!
//! This module owns the viewscreen border frame around the 3D scene
//! (PRD #180). It loads the ten normal-state and ten alert-variant
//! border PNGs, the HUD font, and the Red Alert vignette WGSL shader
//! through `AssetServer` at startup, spawns the frame on `GameStarted`,
//! and despawns it on any transition back to `Lobby`.
//!
//! Server-only — gated by the `server` feature in `lib.rs`.
//!
//! ## Layout
//!
//! Children of a viewport-filling root `Node`, in spawn order (which is
//! Bevy UI's back-to-front order):
//!
//! 1. **Vignette `MaterialNode<RedAlertVignetteMaterial>`** — full-bleed,
//!    spawned first so the border sprites occlude its outermost ring.
//! 2. **4 corners** (240×140 px) anchored to each viewport corner.
//! 3. **Top cap** (320×56 px) centred along the top edge.
//! 4. **Bottom cap** (520×56 px) centred along the bottom edge.
//! 5. **4 edges** using `NodeImageMode::Tiled` to fill the gap between
//!    corners and caps along each side.
//!
//! Bevy UI's default render order layers the frame above the 3D scene
//! cameras. The existing `ViewDirectionLabel` (top-centre) and
//! `FpsText` (top-right) sit at fixed pixel positions outside the
//! corner/cap footprint and remain visible.
//!
//! ## Red Alert
//!
//! When `ShipState.red_alert` flips:
//!
//! - Each border `ImageNode`'s texture handle is swapped instantly
//!   between its normal and alert variant by [`swap_border_textures`].
//! - The vignette material's `intensity` uniform is driven each frame
//!   by [`drive_vignette_intensity`], which calls the pure helper
//!   [`pulse_intensity`] to combine a quarter-second on/off ease with a
//!   1.3-second sine pulse between [`MIN_INTENSITY`] and
//!   [`MAX_INTENSITY`].
//!
//! The Red Alert visual is owned end-to-end here. The previous CSS
//! vignette in `server.html` was removed in the same change.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui::widget::NodeImageMode;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};

use crate::lobby::CurrentPhase;
use crate::messages::GamePhase;
use crate::ship_state::ShipState;

// ── Layout constants ─────────────────────────────────────────────────
//
// All dimensions are in CSS pixels and match the source PNG sizes
// exactly. No proportional scaling — the server is a desktop / large-
// screen target.

const CORNER_W: f32 = 240.0;
const CORNER_H: f32 = 140.0;
const CAP_TOP_W: f32 = 320.0;
const CAP_BOTTOM_W: f32 = 520.0;
const CAP_H: f32 = 56.0;
const EDGE_THICKNESS: f32 = 56.0;

// ── Pulse constants ──────────────────────────────────────────────────
//
// Visual tuning, not gameplay. All module-private.

/// Quarter-second ease window when toggling Red Alert on or off.
const EASE_DURATION: f32 = 0.25;

/// Sine pulse period when Red Alert is steady-on.
const PULSE_PERIOD: f32 = 1.3;

/// Vignette intensity at the trough of the sine pulse.
const MIN_INTENSITY: f32 = 0.55;

/// Vignette intensity at the crest of the sine pulse.
const MAX_INTENSITY: f32 = 1.0;

// ── Resources ────────────────────────────────────────────────────────

/// Holds asset handles for the viewscreen border frame.
///
/// Inserted at startup by [`ViewscreenBorderPlugin`]. Holding the handles
/// in a resource keeps the assets alive (Bevy reference-counts handles)
/// and gives later systems a stable place to look them up.
#[derive(Resource, Debug, Clone)]
pub struct ViewscreenAssets {
    pub corner_tl: Handle<Image>,
    pub corner_tr: Handle<Image>,
    pub corner_bl: Handle<Image>,
    pub corner_br: Handle<Image>,
    pub edge_top: Handle<Image>,
    pub edge_bottom: Handle<Image>,
    pub edge_left: Handle<Image>,
    pub edge_right: Handle<Image>,
    pub cap_top: Handle<Image>,
    pub cap_bottom: Handle<Image>,
    // Alert variants — swapped in by [`swap_border_textures`] on red_alert change.
    pub corner_tl_alert: Handle<Image>,
    pub corner_tr_alert: Handle<Image>,
    pub corner_bl_alert: Handle<Image>,
    pub corner_br_alert: Handle<Image>,
    pub edge_top_alert: Handle<Image>,
    pub edge_bottom_alert: Handle<Image>,
    pub edge_left_alert: Handle<Image>,
    pub edge_right_alert: Handle<Image>,
    pub cap_top_alert: Handle<Image>,
    pub cap_bottom_alert: Handle<Image>,
    /// Display font for HUD readouts (added in #184).
    pub font_display: Handle<Font>,
}

/// Cached handle to the single `RedAlertVignetteMaterial` instance,
/// so `drive_vignette_intensity` can mutate its uniform without a query.
#[derive(Resource, Debug, Clone)]
struct VignetteMaterialHandle(Handle<RedAlertVignetteMaterial>);

// ── Marker components ────────────────────────────────────────────────

/// Marker for the root `Node` that owns the entire border frame.
///
/// Despawning this entity (with descendants) tears down all border
/// `ImageNode` children and the vignette material node in one shot.
#[derive(Component)]
struct ViewscreenBorderRoot;

/// Identifies which border slot an `ImageNode` occupies, so
/// [`swap_border_textures`] can rewrite each handle on `red_alert`
/// change without coupling spawn order to lookup order.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
enum BorderSlot {
    CornerTl,
    CornerTr,
    CornerBl,
    CornerBr,
    EdgeTop,
    EdgeBottom,
    EdgeLeft,
    EdgeRight,
    CapTop,
    CapBottom,
}

impl BorderSlot {
    fn handle<'a>(self, assets: &'a ViewscreenAssets, alert: bool) -> &'a Handle<Image> {
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
            (Self::CapTop, false) => &assets.cap_top,
            (Self::CapTop, true) => &assets.cap_top_alert,
            (Self::CapBottom, false) => &assets.cap_bottom,
            (Self::CapBottom, true) => &assets.cap_bottom_alert,
        }
    }
}

// ── Red Alert vignette material ──────────────────────────────────────

/// `UiMaterial` driving the inset radial-gradient red vignette behind
/// the border. The single `intensity` uniform is in `[0.0, 1.0]`; the
/// shader fades the red glow from invisible at 0.0 to fully bright at
/// 1.0. Driven each frame by [`drive_vignette_intensity`] via the pure
/// helper [`pulse_intensity`].
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

// ── Plugin ───────────────────────────────────────────────────────────

/// Loads viewscreen border assets at startup, registers the Red Alert
/// vignette material, and renders the frame during `GameState::InProgress`.
pub struct ViewscreenBorderPlugin;

impl Plugin for ViewscreenBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<RedAlertVignetteMaterial>::default())
            .add_systems(Startup, load_viewscreen_assets)
            .add_systems(
                Update,
                (
                    sync_border_to_phase,
                    swap_border_textures,
                    drive_vignette_intensity,
                ),
            );
    }
}

// ── Systems ──────────────────────────────────────────────────────────

fn load_viewscreen_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    let assets = ViewscreenAssets {
        corner_tl: asset_server.load("viewscreen/corner-tl.png"),
        corner_tr: asset_server.load("viewscreen/corner-tr.png"),
        corner_bl: asset_server.load("viewscreen/corner-bl.png"),
        corner_br: asset_server.load("viewscreen/corner-br.png"),
        edge_top: asset_server.load("viewscreen/edge-top.png"),
        edge_bottom: asset_server.load("viewscreen/edge-bottom.png"),
        edge_left: asset_server.load("viewscreen/edge-left.png"),
        edge_right: asset_server.load("viewscreen/edge-right.png"),
        cap_top: asset_server.load("viewscreen/cap-top.png"),
        cap_bottom: asset_server.load("viewscreen/cap-bottom.png"),
        corner_tl_alert: asset_server.load("viewscreen/corner-tl-alert.png"),
        corner_tr_alert: asset_server.load("viewscreen/corner-tr-alert.png"),
        corner_bl_alert: asset_server.load("viewscreen/corner-bl-alert.png"),
        corner_br_alert: asset_server.load("viewscreen/corner-br-alert.png"),
        edge_top_alert: asset_server.load("viewscreen/edge-top-alert.png"),
        edge_bottom_alert: asset_server.load("viewscreen/edge-bottom-alert.png"),
        edge_left_alert: asset_server.load("viewscreen/edge-left-alert.png"),
        edge_right_alert: asset_server.load("viewscreen/edge-right-alert.png"),
        cap_top_alert: asset_server.load("viewscreen/cap-top-alert.png"),
        cap_bottom_alert: asset_server.load("viewscreen/cap-bottom-alert.png"),
        font_display: asset_server.load("fonts/ChakraPetch-SemiBold.ttf"),
    };
    commands.insert_resource(assets);
}

/// Spawn the border root on transition into `InProgress`; despawn it on
/// any transition back to `Lobby`.
///
/// Idempotent on phase change — re-entering `InProgress` while the root
/// already exists is a no-op (defensive; current code never re-enters).
fn sync_border_to_phase(
    mut commands: Commands,
    phase: Res<CurrentPhase>,
    assets: Option<Res<ViewscreenAssets>>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
    existing: Query<Entity, With<ViewscreenBorderRoot>>,
) {
    if !phase.is_changed() {
        return;
    }

    match phase.0 {
        GamePhase::InProgress => {
            let Some(assets) = assets else { return };
            if !existing.is_empty() {
                return;
            }
            let vignette = materials.add(RedAlertVignetteMaterial { intensity: 0.0 });
            commands.insert_resource(VignetteMaterialHandle(vignette.clone()));
            spawn_border_frame(&mut commands, &assets, vignette);
        }
        GamePhase::Lobby => {
            for entity in existing.iter() {
                commands.entity(entity).despawn();
            }
            commands.remove_resource::<VignetteMaterialHandle>();
        }
    }
}

/// Rewrites each border `ImageNode`'s `image` handle to the alert or
/// normal variant whenever `ShipState.red_alert` changes.
///
/// The swap is instant (one frame) — matches the demo's pop. The pulsing
/// vignette carries the temporal energy.
fn swap_border_textures(
    ship: Option<Res<ShipState>>,
    assets: Option<Res<ViewscreenAssets>>,
    mut q: Query<(&BorderSlot, &mut ImageNode)>,
) {
    let Some(ship) = ship else { return };
    let Some(assets) = assets else { return };
    if !ship.is_changed() {
        return;
    }
    let alert = ship.red_alert();
    for (slot, mut image_node) in q.iter_mut() {
        image_node.image = slot.handle(&assets, alert).clone();
    }
}

/// Per-frame system that drives the vignette material's `intensity`
/// uniform via the pure [`pulse_intensity`] helper.
fn drive_vignette_intensity(
    time: Res<Time>,
    ship: Option<Res<ShipState>>,
    handle: Option<Res<VignetteMaterialHandle>>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
) {
    let Some(ship) = ship else { return };
    let Some(handle) = handle else { return };
    let Some(material) = materials.get_mut(&handle.0) else { return };
    material.intensity = pulse_intensity(
        time.elapsed_secs(),
        ship.red_alert(),
        material.intensity,
        time.delta_secs(),
    );
}

// ── Pure helpers ─────────────────────────────────────────────────────

/// State-transition function for the Red Alert vignette intensity.
///
/// Combines a quarter-second on/off ease with a 1.3-second sine pulse
/// between [`MIN_INTENSITY`] and [`MAX_INTENSITY`] when active. Returns
/// `0.0` when fully off.
///
/// - `time_secs`: monotonic time (drives the sine phase).
/// - `red_alert`: the current alert state.
/// - `prev_intensity`: the previous frame's intensity (for ease ramping).
/// - `dt`: seconds since last frame.
///
/// Behaviour:
/// - `red_alert == false && prev_intensity == 0.0` → returns `0.0`.
/// - `red_alert == true` → ramps `prev_intensity` toward the current
///   sine target by at most `MAX_INTENSITY / EASE_DURATION * dt`, so the
///   rise from `0.0` to the band takes ~`EASE_DURATION` seconds.
/// - `red_alert == false && prev_intensity > 0.0` → ramps toward `0.0`
///   by the same per-second rate (~`EASE_DURATION` seconds to fully off).
///
/// Once `prev_intensity` is inside the pulse band, the sine target moves
/// strictly slower than the ease ramp, so the helper tracks the sine
/// directly — the band stays within `[MIN_INTENSITY, MAX_INTENSITY]`.
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

/// Sine-wave value in `[MIN_INTENSITY, MAX_INTENSITY]` with period
/// [`PULSE_PERIOD`]. Phase 0 sits at the midpoint; the crest is at a
/// quarter period.
fn sine_pulse(time_secs: f32) -> f32 {
    let mid = (MIN_INTENSITY + MAX_INTENSITY) * 0.5;
    let amp = (MAX_INTENSITY - MIN_INTENSITY) * 0.5;
    mid + amp * (std::f32::consts::TAU * time_secs / PULSE_PERIOD).sin()
}

/// Move `current` toward `target` by at most `max_step` (always
/// non-negative). If the difference is already within `max_step`,
/// snap to `target`.
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

fn spawn_border_frame(
    commands: &mut Commands,
    assets: &ViewscreenAssets,
    vignette: Handle<RedAlertVignetteMaterial>,
) {
    commands
        .spawn((
            ViewscreenBorderRoot,
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
            // ── Vignette (spawned FIRST so border sprites occlude it) ─
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

            // ── Corners ──────────────────────────────────────────────
            parent.spawn((
                BorderSlot::CornerTl,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_tl.clone()),
            ));
            parent.spawn((
                BorderSlot::CornerTr,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    right: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_tr.clone()),
            ));
            parent.spawn((
                BorderSlot::CornerBl,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_bl.clone()),
            ));
            parent.spawn((
                BorderSlot::CornerBr,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    right: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_br.clone()),
            ));

            // ── Top cap (centred along top edge) ─────────────────────
            parent.spawn((
                BorderSlot::CapTop,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    margin: UiRect {
                        left: Val::Px(-CAP_TOP_W / 2.0),
                        ..default()
                    },
                    width: Val::Px(CAP_TOP_W),
                    height: Val::Px(CAP_H),
                    ..default()
                },
                ImageNode::new(assets.cap_top.clone()),
            ));

            // ── Bottom cap (centred along bottom edge) ───────────────
            parent.spawn((
                BorderSlot::CapBottom,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    margin: UiRect {
                        left: Val::Px(-CAP_BOTTOM_W / 2.0),
                        ..default()
                    },
                    width: Val::Px(CAP_BOTTOM_W),
                    height: Val::Px(CAP_H),
                    ..default()
                },
                ImageNode::new(assets.cap_bottom.clone()),
            ));

            // ── Edges ────────────────────────────────────────────────
            //
            // The top/bottom edges fill the horizontal gap between each
            // corner and the centre cap. We spawn two segments per
            // horizontal edge (left-of-cap, right-of-cap) so the cap
            // can sit on top without a stretched section underneath.
            //
            // Both segments share the `BorderSlot::EdgeTop` (or
            // `EdgeBottom`) marker so the swap system rewrites both in
            // one query iteration.
            //
            // The left/right edges fill the vertical gap between the
            // top and bottom corners.

            // Top edge — left segment (between TL corner and top cap).
            parent.spawn((
                BorderSlot::EdgeTop,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(CORNER_W),
                    right: Val::Percent(50.0),
                    margin: UiRect {
                        right: Val::Px(CAP_TOP_W / 2.0),
                        ..default()
                    },
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
            // Top edge — right segment (between top cap and TR corner).
            parent.spawn((
                BorderSlot::EdgeTop,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    right: Val::Px(CORNER_W),
                    margin: UiRect {
                        left: Val::Px(CAP_TOP_W / 2.0),
                        ..default()
                    },
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

            // Bottom edge — left segment.
            parent.spawn((
                BorderSlot::EdgeBottom,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Px(CORNER_W),
                    right: Val::Percent(50.0),
                    margin: UiRect {
                        right: Val::Px(CAP_BOTTOM_W / 2.0),
                        ..default()
                    },
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
            // Bottom edge — right segment.
            parent.spawn((
                BorderSlot::EdgeBottom,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    right: Val::Px(CORNER_W),
                    margin: UiRect {
                        left: Val::Px(CAP_BOTTOM_W / 2.0),
                        ..default()
                    },
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
                BorderSlot::EdgeLeft,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(CORNER_H),
                    bottom: Val::Px(CORNER_H),
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
                BorderSlot::EdgeRight,
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(0.0),
                    top: Val::Px(CORNER_H),
                    bottom: Val::Px(CORNER_H),
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
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Per-frame `dt` at 60 Hz; matches the typical render cadence.
    const DT_60HZ: f32 = 1.0 / 60.0;

    /// Per-frame ramp ceiling used internally by `pulse_intensity` —
    /// kept in sync with the implementation so tests can reason about
    /// the ease window without hard-coding magic numbers.
    fn max_step_per_frame() -> f32 {
        (MAX_INTENSITY / EASE_DURATION) * DT_60HZ
    }

    #[test]
    fn idle_stays_at_zero() {
        // red_alert=false, prev=0 → exactly 0, no drift.
        let out = pulse_intensity(0.0, false, 0.0, DT_60HZ);
        assert_eq!(out, 0.0);

        // Time advancing while idle never lifts off zero.
        let out = pulse_intensity(123.4, false, 0.0, DT_60HZ);
        assert_eq!(out, 0.0);
    }

    #[test]
    fn alert_on_rises_monotonically_during_ease_window() {
        // From a cold start, simulate the quarter-second ease at 60 Hz
        // and confirm the intensity only ever goes up until it reaches
        // the pulse band.
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
        // After the ease window, the intensity should have reached at
        // least MIN_INTENSITY (i.e. it has caught up with the pulse band).
        assert!(
            prev >= MIN_INTENSITY - 1e-3,
            "intensity {prev} did not reach pulse band after {EASE_DURATION}s ease"
        );
    }

    #[test]
    fn alert_off_decays_smoothly_to_zero_within_ease_window() {
        // Start at peak intensity, toggle off, simulate forward —
        // the intensity must fall monotonically and hit exactly 0.0
        // within the ease window (plus a small slack frame).
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

        // Once at zero, further idle frames must stay at zero.
        let next = pulse_intensity(t, false, 0.0, DT_60HZ);
        assert_eq!(next, 0.0);
    }

    #[test]
    fn steady_state_pulse_stays_within_band() {
        // After the ease window, simulate a few full pulse periods and
        // confirm the intensity stays inside [MIN_INTENSITY, MAX_INTENSITY]
        // within a small numeric slack.
        let mut prev = MIN_INTENSITY; // pretend we already eased in
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
    fn sine_phase_points_match_target_band() {
        // sine_pulse(t) at t=0 should be the band midpoint.
        let mid = (MIN_INTENSITY + MAX_INTENSITY) * 0.5;
        assert!((sine_pulse(0.0) - mid).abs() < 1e-5);

        // Quarter period → crest (MAX_INTENSITY).
        let quarter = PULSE_PERIOD / 4.0;
        assert!((sine_pulse(quarter) - MAX_INTENSITY).abs() < 1e-5);

        // Half period → midpoint again (descending).
        let half = PULSE_PERIOD / 2.0;
        assert!((sine_pulse(half) - mid).abs() < 1e-5);

        // Three-quarter period → trough (MIN_INTENSITY).
        let three_quarter = 3.0 * PULSE_PERIOD / 4.0;
        assert!((sine_pulse(three_quarter) - MIN_INTENSITY).abs() < 1e-5);

        // Full period → back to midpoint.
        assert!((sine_pulse(PULSE_PERIOD) - mid).abs() < 1e-5);
    }

    #[test]
    fn approach_snaps_when_within_step() {
        // delta smaller than max_step → returns target exactly.
        assert_eq!(approach(0.5, 0.51, 0.1), 0.51);
        assert_eq!(approach(0.5, 0.49, 0.1), 0.49);
    }

    #[test]
    fn approach_steps_toward_target_when_outside_step() {
        // Larger delta → moves by exactly max_step.
        assert!((approach(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
        assert!((approach(1.0, 0.0, 0.25) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn slot_handle_picks_normal_or_alert_variant() {
        // Sanity check that BorderSlot::handle returns distinct asset
        // ids for the normal and alert variants of one slot.
        let assets = test_assets();
        let normal = BorderSlot::CornerTl.handle(&assets, false);
        let alert = BorderSlot::CornerTl.handle(&assets, true);
        assert_ne!(normal.id(), alert.id());
    }

    fn test_assets() -> ViewscreenAssets {
        // Construct dummy handles — none of the fields are dereffed in
        // these tests, we only compare `Handle::id()`. In Bevy 0.18 a
        // weak handle is built from a `Uuid` via `Handle::from(uuid)`.
        use bevy::asset::uuid::Uuid;
        let h = |n: u128| -> Handle<Image> { Uuid::from_u128(n).into() };
        let f = |n: u128| -> Handle<Font> { Uuid::from_u128(n).into() };
        ViewscreenAssets {
            corner_tl: h(1),
            corner_tr: h(2),
            corner_bl: h(3),
            corner_br: h(4),
            edge_top: h(5),
            edge_bottom: h(6),
            edge_left: h(7),
            edge_right: h(8),
            cap_top: h(9),
            cap_bottom: h(10),
            corner_tl_alert: h(11),
            corner_tr_alert: h(12),
            corner_bl_alert: h(13),
            corner_br_alert: h(14),
            edge_top_alert: h(15),
            edge_bottom_alert: h(16),
            edge_left_alert: h(17),
            edge_right_alert: h(18),
            cap_top_alert: h(19),
            cap_bottom_alert: h(20),
            font_display: f(21),
        }
    }
}
