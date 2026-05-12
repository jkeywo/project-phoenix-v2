//! Viewscreen border frame.
//!
//! This module owns the static viewscreen border frame around the 3D
//! scene. Tracked by PRD #180; this slice (#182) renders the ten
//! normal-state border sprites only — no alert state, no HUD readouts,
//! no vignette. Those land in follow-up issues #183, #184.
//!
//! The frame appears on the `GameStarted` phase transition and is
//! despawned cleanly on any transition back to `Lobby` (defensive — the
//! current code never returns to lobby, but the plugin must not leak
//! entities if it ever does).
//!
//! Server-only — gated by the `server` feature in `lib.rs`.
//!
//! ## Layout
//!
//! Ten `ImageNode` children of a viewport-filling root `Node`:
//!
//! - **4 corners** (240×140 px) anchored to each viewport corner.
//! - **Top cap** (320×56 px) centred along the top edge.
//! - **Bottom cap** (520×56 px) centred along the bottom edge.
//! - **4 edges** using `NodeImageMode::Tiled` to fill the gap between
//!   corners and caps along each side.
//!
//! Bevy UI's default render order layers the frame above the 3D scene
//! cameras. The existing `ViewDirectionLabel` (top-centre) and
//! `FpsText` (top-right) sit at fixed pixel positions outside the
//! corner/cap footprint and remain visible.

use bevy::prelude::*;
use bevy::ui::widget::NodeImageMode;

use crate::lobby::CurrentPhase;
use crate::messages::GamePhase;

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

// ── Resources ────────────────────────────────────────────────────────

/// Holds asset handles for the viewscreen border frame.
///
/// Inserted at startup by [`ViewscreenBorderPlugin`]. Holding the handles
/// in a resource keeps the assets alive (Bevy reference-counts handles)
/// and gives later systems a stable place to look them up.
///
/// The alert-variant handles and HUD font/shader land in #183 and #184.
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
    /// Display font for HUD readouts (added in #184).
    pub font_display: Handle<Font>,
    /// Placeholder WGSL for the red-alert vignette (body lands in #183).
    pub vignette_shader: Handle<Shader>,
}

// ── Marker components ────────────────────────────────────────────────

/// Marker for the root `Node` that owns the entire border frame.
///
/// Despawning this entity (with descendants) tears down all ten border
/// `ImageNode` children in one shot.
#[derive(Component)]
struct ViewscreenBorderRoot;

// ── Plugin ───────────────────────────────────────────────────────────

/// Loads viewscreen border assets at startup and renders the static
/// frame during `GameState::InProgress`.
pub struct ViewscreenBorderPlugin;

impl Plugin for ViewscreenBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_viewscreen_assets)
            .add_systems(Update, sync_border_to_phase);
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
        font_display: asset_server.load("fonts/ChakraPetch-SemiBold.ttf"),
        vignette_shader: asset_server.load("shaders/red_alert_vignette.wgsl"),
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
            spawn_border_frame(&mut commands, &assets);
        }
        GamePhase::Lobby => {
            for entity in existing.iter() {
                commands.entity(entity).despawn();
            }
        }
    }
}

fn spawn_border_frame(commands: &mut Commands, assets: &ViewscreenAssets) {
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
            // ── Corners ──────────────────────────────────────────────
            parent.spawn((
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
            // The left/right edges fill the vertical gap between the
            // top and bottom corners.

            // Top edge — left segment (between TL corner and top cap).
            parent.spawn((
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
