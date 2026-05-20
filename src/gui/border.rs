use bevy::prelude::*;
use bevy::ui::widget::NodeImageMode;

use super::vignette::RedAlertIntensity;

// ── Layout ────────────────────────────────────────────────────────────────

/// Configuration for the 9-slice border frame.
#[derive(Resource, Clone, Debug)]
pub struct BorderConfig {
    pub corner_size: f32,
    pub edge_thickness: f32,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self { corner_size: 40.0, edge_thickness: 16.0 }
    }
}

// ── Corner / edge slot IDs ────────────────────────────────────────────────

/// Identifies one of the four corner positions.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
pub enum CornerSlot {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Identifies one of the four edge positions.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
pub enum EdgeSlot {
    Top,
    Bottom,
    Left,
    Right,
}

// ── BorderAssets resource ─────────────────────────────────────────────────

/// Corner and edge `Handle<Image>` values for the 9-slice border.
///
/// Each position has a normal variant and an alert variant.  Populate this
/// resource at app startup before spawning the `GuiBorder` widget.  Swapping
/// handles before `spawn` changes the border art without library code changes.
#[derive(Resource, Clone, Debug)]
pub struct BorderAssets {
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
}

impl BorderAssets {
    pub fn corner(&self, slot: CornerSlot, alert: bool) -> &Handle<Image> {
        match (slot, alert) {
            (CornerSlot::TopLeft, false) => &self.corner_tl,
            (CornerSlot::TopLeft, true) => &self.corner_tl_alert,
            (CornerSlot::TopRight, false) => &self.corner_tr,
            (CornerSlot::TopRight, true) => &self.corner_tr_alert,
            (CornerSlot::BottomLeft, false) => &self.corner_bl,
            (CornerSlot::BottomLeft, true) => &self.corner_bl_alert,
            (CornerSlot::BottomRight, false) => &self.corner_br,
            (CornerSlot::BottomRight, true) => &self.corner_br_alert,
        }
    }

    pub fn edge(&self, slot: EdgeSlot, alert: bool) -> &Handle<Image> {
        match (slot, alert) {
            (EdgeSlot::Top, false) => &self.edge_top,
            (EdgeSlot::Top, true) => &self.edge_top_alert,
            (EdgeSlot::Bottom, false) => &self.edge_bottom,
            (EdgeSlot::Bottom, true) => &self.edge_bottom_alert,
            (EdgeSlot::Left, false) => &self.edge_left,
            (EdgeSlot::Left, true) => &self.edge_left_alert,
            (EdgeSlot::Right, false) => &self.edge_right,
            (EdgeSlot::Right, true) => &self.edge_right_alert,
        }
    }
}

// ── Root marker ───────────────────────────────────────────────────────────

/// Marker on the root `Node` of every `GuiBorder` widget.
#[derive(Component)]
pub struct GuiBorder;

/// Marker on the content area child node inside the border.
#[derive(Component)]
pub struct BorderContentArea;

// ── Widget ────────────────────────────────────────────────────────────────

/// Placeholder struct — the `spawn` method creates the frame.
pub struct GuiBorderWidget;

impl GuiBorderWidget {
    /// Spawn a 9-slice border frame.
    ///
    /// Returns the root entity.  The frame occupies the full viewport
    /// (`PositionType::Absolute`, 100% × 100%) and includes:
    ///
    /// - 4 corner sprites
    /// - 4 edge sprites (tiled)
    /// - A `BorderContentArea` safe zone for inner content
    ///
    /// Corner and edge marker components (`CornerSlot` / `EdgeSlot`) are
    /// attached so that `update_border_textures` can swap image handles
    /// when `RedAlertIntensity` changes.
    pub fn spawn(
        commands: &mut Commands,
        assets: &BorderAssets,
        config: &BorderConfig,
        alert: bool,
    ) -> Entity {
        let cs = config.corner_size;
        let et = config.edge_thickness;

        commands
            .spawn((
                GuiBorder,
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
                // Safe content area
                parent.spawn((
                    BorderContentArea,
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(cs),
                        left: Val::Px(et),
                        right: Val::Px(et),
                        bottom: Val::Px(cs),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                ));

                // 4 corners
                let corners: [(CornerSlot, Val, Val, Val, Val); 4] = [
                    (CornerSlot::TopLeft,     Val::Px(0.0), Val::Px(0.0), Val::Auto, Val::Auto),
                    (CornerSlot::TopRight,    Val::Px(0.0), Val::Auto,    Val::Px(0.0), Val::Auto),
                    (CornerSlot::BottomLeft,  Val::Auto,    Val::Px(0.0), Val::Auto,    Val::Px(0.0)),
                    (CornerSlot::BottomRight, Val::Auto,    Val::Auto,    Val::Px(0.0), Val::Px(0.0)),
                ];
                for (slot, top, left, right, bottom) in corners {
                    parent.spawn((
                        slot,
                        Node {
                            position_type: PositionType::Absolute,
                            top,
                            left,
                            right,
                            bottom,
                            width: Val::Px(cs),
                            height: Val::Px(cs),
                            ..default()
                        },
                        ImageNode::new(assets.corner(slot, alert).clone()),
                    ));
                }

                // 4 edges (tiled)
                let edges: [(EdgeSlot, Val, Val, Val, Val, bool, bool); 4] = [
                    (EdgeSlot::Top,    Val::Px(0.0), Val::Px(cs),  Val::Px(cs),  Val::Auto,    true,  false),
                    (EdgeSlot::Bottom, Val::Auto,    Val::Px(cs),  Val::Px(cs),  Val::Px(0.0), true,  false),
                    (EdgeSlot::Left,   Val::Px(cs),  Val::Px(0.0), Val::Auto,    Val::Px(cs),  false, true ),
                    (EdgeSlot::Right,  Val::Px(cs),  Val::Auto,    Val::Px(0.0), Val::Px(cs),  false, true ),
                ];
                for (slot, top, left, right, bottom, tile_x, tile_y) in edges {
                    let node = if tile_x {
                        Node {
                            position_type: PositionType::Absolute,
                            top,
                            left,
                            right,
                            bottom,
                            height: Val::Px(et),
                            ..default()
                        }
                    } else {
                        Node {
                            position_type: PositionType::Absolute,
                            top,
                            left,
                            right,
                            bottom,
                            width: Val::Px(et),
                            ..default()
                        }
                    };
                    parent.spawn((
                        slot,
                        node,
                        ImageNode::new(assets.edge(slot, alert).clone())
                            .with_mode(NodeImageMode::Tiled {
                                tile_x,
                                tile_y,
                                stretch_value: 1.0,
                            }),
                    ));
                }
            })
            .id()
    }
}

// ── Systems ───────────────────────────────────────────────────────────────

/// Swaps all border corner/edge `ImageNode` handles between normal and alert
/// variants whenever `RedAlertIntensity` crosses the zero threshold.
fn update_border_textures(
    intensity: Option<Res<RedAlertIntensity>>,
    assets: Option<Res<BorderAssets>>,
    mut corners: Query<(&CornerSlot, &mut ImageNode), Without<EdgeSlot>>,
    mut edges: Query<(&EdgeSlot, &mut ImageNode), Without<CornerSlot>>,
) {
    let Some(intensity) = intensity else { return };
    let Some(assets) = assets else { return };
    let alert = intensity.0 > 0.0;

    for (slot, mut image) in corners.iter_mut() {
        image.image = assets.corner(*slot, alert).clone();
    }
    for (slot, mut image) in edges.iter_mut() {
        image.image = assets.edge(*slot, alert).clone();
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────

/// Sub-plugin for the border widget.  Provides the `update_border_textures`
/// system that responds to `RedAlertIntensity`.
pub struct GuiBorderPlugin;

impl Plugin for GuiBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_border_textures);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::uuid::Uuid;

    fn test_assets() -> BorderAssets {
        let h = |n: u128| -> Handle<Image> { Uuid::from_u128(n).into() };
        BorderAssets {
            corner_tl: h(1), corner_tr: h(2), corner_bl: h(3), corner_br: h(4),
            edge_top: h(5), edge_bottom: h(6), edge_left: h(7), edge_right: h(8),
            corner_tl_alert: h(9), corner_tr_alert: h(10),
            corner_bl_alert: h(11), corner_br_alert: h(12),
            edge_top_alert: h(13), edge_bottom_alert: h(14),
            edge_left_alert: h(15), edge_right_alert: h(16),
        }
    }

    #[test]
    fn corner_selector_returns_normal_when_not_alert() {
        let a = test_assets();
        assert_eq!(a.corner(CornerSlot::TopLeft, false).id(), a.corner_tl.id());
        assert_eq!(a.corner(CornerSlot::TopRight, false).id(), a.corner_tr.id());
        assert_eq!(a.corner(CornerSlot::BottomLeft, false).id(), a.corner_bl.id());
        assert_eq!(a.corner(CornerSlot::BottomRight, false).id(), a.corner_br.id());
    }

    #[test]
    fn corner_selector_returns_alert_when_alert() {
        let a = test_assets();
        assert_eq!(a.corner(CornerSlot::TopLeft, true).id(), a.corner_tl_alert.id());
        assert_eq!(a.corner(CornerSlot::TopRight, true).id(), a.corner_tr_alert.id());
        assert_eq!(a.corner(CornerSlot::BottomLeft, true).id(), a.corner_bl_alert.id());
        assert_eq!(a.corner(CornerSlot::BottomRight, true).id(), a.corner_br_alert.id());
    }

    #[test]
    fn edge_selector_returns_normal_when_not_alert() {
        let a = test_assets();
        assert_eq!(a.edge(EdgeSlot::Top, false).id(), a.edge_top.id());
        assert_eq!(a.edge(EdgeSlot::Bottom, false).id(), a.edge_bottom.id());
        assert_eq!(a.edge(EdgeSlot::Left, false).id(), a.edge_left.id());
        assert_eq!(a.edge(EdgeSlot::Right, false).id(), a.edge_right.id());
    }

    #[test]
    fn edge_selector_returns_alert_when_alert() {
        let a = test_assets();
        assert_eq!(a.edge(EdgeSlot::Top, true).id(), a.edge_top_alert.id());
        assert_eq!(a.edge(EdgeSlot::Bottom, true).id(), a.edge_bottom_alert.id());
        assert_eq!(a.edge(EdgeSlot::Left, true).id(), a.edge_left_alert.id());
        assert_eq!(a.edge(EdgeSlot::Right, true).id(), a.edge_right_alert.id());
    }

    #[test]
    fn normal_and_alert_handles_differ() {
        let a = test_assets();
        assert_ne!(a.corner_tl.id(), a.corner_tl_alert.id());
        assert_ne!(a.edge_top.id(), a.edge_top_alert.id());
    }
}
