use bevy::prelude::*;
use crate::messages::{Console, ConsoleHullStatus, ServerMessage, ViewMode};

/// Shared ship state broadcast to all consoles every 10Hz via `SimState`.
///
/// This is the client-side mirror of the fields every console needs: pose,
/// red-alert, view mode, power levels, and impulse charge. Per-console panels
/// read from this resource instead of the `ClientSimState` god-resource.
#[derive(Clone, Debug, PartialEq, Resource)]
pub struct ShipView {
    pub red_alert: bool,
    pub view_mode: ViewMode,
    pub ship_x: f32,
    pub ship_z: f32,
    pub ship_yaw: f32,
    /// Current forward speed in world units per second (negative = reversing).
    pub forward_speed: f32,
    pub hull_fraction: f32,
    pub power_levels: (u8, u8, u8),
    pub impulse_charge_progress: f32,
    /// Per-console hull status, populated from `SimSnapshot`. Empty when the
    /// ship config has no per-console hull entries.
    pub console_hull: Vec<ConsoleHullStatus>,
}

impl Default for ShipView {
    fn default() -> Self {
        Self {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: 0.0,
            ship_z: 0.0,
            ship_yaw: 0.0,
            forward_speed: 0.0,
            hull_fraction: 1.0,
            power_levels: (2, 2, 2),
            impulse_charge_progress: 0.0,
            console_hull: vec![],
        }
    }
}

impl ShipView {
    /// Returns `(current_hp, max_hp)` for the given console, or `None` if the
    /// ship config has no hull entry for it.
    pub fn hull_for(&self, console: &Console) -> Option<(f32, f32)> {
        self.console_hull.iter()
            .find(|s| &s.console == console)
            .map(|s| (s.current, s.max_hp))
    }
}

impl ShipView {
    /// True iff the captain's view direction selector should highlight
    /// the given direction. Radar mode highlights nothing in the cross.
    pub fn is_active_camera_direction(&self, direction: &crate::messages::ViewDirection) -> bool {
        matches!(&self.view_mode, ViewMode::Camera(d) if d == direction)
    }

    /// Apply one inbound server message to this view, updating whichever
    /// fields the message carries.
    pub fn apply(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::SimState { snapshot } => {
                self.red_alert = snapshot.red_alert;
                self.view_mode = snapshot.view_mode.clone();
                self.ship_x = snapshot.ship_x;
                self.ship_z = snapshot.ship_z;
                self.ship_yaw = snapshot.ship_yaw;
                self.forward_speed = snapshot.forward_speed;
                self.power_levels = snapshot.power_levels;
                self.impulse_charge_progress = snapshot.impulse_charge_progress;
                let total_max: f32 = snapshot.console_hull.iter().map(|c| c.max_hp).sum();
                let total_cur: f32 = snapshot.console_hull.iter().map(|c| c.current).sum();
                self.hull_fraction = if total_max > 0.0 {
                    (total_cur / total_max).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                self.console_hull = snapshot.console_hull.clone();
            }
            ServerMessage::Welcome { .. } => {
                let _ = std::mem::replace(self, ShipView::default());
            }
            _ => {}
        }
    }
}

/// Bevy plugin that owns the `ShipView` resource.
///
/// Registers the resource and runs a system that reads every inbound
/// `ServerMessage` event (via `InboundServerMessage`) and calls
/// `ShipView::apply` to keep the resource current. Consumer systems
/// (captain panel, helm HUD, navigation impulse text, power console)
/// read `Res<ShipView>` instead of reaching into `ClientSimState`.
#[cfg(feature = "client")]
pub struct ShipViewPlugin;

#[cfg(feature = "client")]
impl Plugin for ShipViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShipView>()
            .add_systems(Startup, spawn_console_hull_bar)
            .add_systems(Update, (apply_ship_view_messages, update_console_hull_bar));
    }
}

#[cfg(feature = "client")]
fn apply_ship_view_messages(
    mut reader: MessageReader<crate::client_app::InboundServerMessage>,
    mut ship_view: ResMut<ShipView>,
) {
    for ev in reader.read() {
        ship_view.apply(&ev.0);
    }
}

// ── Console hull bar overlay ──────────────────────────────────────────────────

/// Marks the background container of the per-console hull bar.
///
/// `pub(crate)` so that `client::phone_border::framing` can reparent the
/// bar into the bezel's `BorderContentArea`, anchoring it just above the
/// bottom border edge instead of floating over the raw viewport.
#[cfg(feature = "client")]
#[derive(Component)]
pub(crate) struct ConsoleHullBarBg;

/// Marks the fill node of the per-console hull bar.
#[cfg(feature = "client")]
#[derive(Component)]
struct ConsoleHullBarFill;

/// Marks the `current / max` text overlay shown on top of the hull bar.
#[cfg(feature = "client")]
#[derive(Component)]
struct ConsoleHullBarText;

#[allow(dead_code)]
const HULL_BAR_BG: Color = Color::srgba(0.08, 0.08, 0.12, 0.85);
#[allow(dead_code)]
const HULL_BAR_FILL: Color = Color::srgb(0.2, 0.75, 0.3);
#[allow(dead_code)]
const HULL_BAR_FILL_LOW: Color = Color::srgb(0.85, 0.25, 0.15);

/// Width of the hull bar in logical pixels — wide enough to hold a "999 / 999"
/// text overlay comfortably.
#[cfg(feature = "client")]
const HULL_BAR_WIDTH: f32 = 220.0;

/// Height of the hull bar in logical pixels.
#[cfg(feature = "client")]
const HULL_BAR_HEIGHT: f32 = 18.0;

/// Distance from the bottom edge of the bezel `BorderContentArea`.
/// The reparenting system in `client::phone_border::framing` slots this
/// bar inside the bezel safe zone, so this value is measured from the
/// top of the bezel's bottom border rather than from the raw viewport.
#[cfg(feature = "client")]
const HULL_BAR_BOTTOM: f32 = 6.0;

#[cfg(feature = "client")]
fn spawn_console_hull_bar(mut commands: Commands) {
    // Outer wrapper centres the bar horizontally at the bottom of the
    // viewport using a full-width flex row anchored at the bottom edge.
    commands
        .spawn((
            ConsoleHullBarBg,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(HULL_BAR_BOTTOM),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                height: Val::Px(HULL_BAR_HEIGHT),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|wrapper| {
            // Inner bar: fixed-width pill that holds the fill + text overlay.
            wrapper
                .spawn((
                    Node {
                        width: Val::Px(HULL_BAR_WIDTH),
                        height: Val::Px(HULL_BAR_HEIGHT),
                        position_type: PositionType::Relative,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(HULL_BAR_BG),
                ))
                .with_children(|bar| {
                    // Fill: anchored to the left edge so width % maps to HP %.
                    bar.spawn((
                        ConsoleHullBarFill,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            width: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(HULL_BAR_FILL),
                        Visibility::Hidden,
                    ));
                    // Text overlay on top of the fill, centred over the bar.
                    bar.spawn((
                        ConsoleHullBarText,
                        Text::new(""),
                        TextFont { font_size: 12.0, ..default() },
                        TextColor(Color::srgb(0.95, 0.95, 0.95)),
                        Node {
                            position_type: PositionType::Absolute,
                            ..default()
                        },
                    ));
                });
        });
}

#[cfg(feature = "client")]
fn update_console_hull_bar(
    ship_view: Res<ShipView>,
    active: Res<crate::lobby::client_panel::ActiveConsole>,
    mut bg_q: Query<&mut Visibility, (With<ConsoleHullBarBg>, Without<ConsoleHullBarFill>, Without<ConsoleHullBarText>)>,
    mut fill_q: Query<(&mut Node, &mut Visibility, &mut BackgroundColor), (With<ConsoleHullBarFill>, Without<ConsoleHullBarText>)>,
    mut text_q: Query<&mut Text, With<ConsoleHullBarText>>,
) {
    if !ship_view.is_changed() && !active.is_changed() {
        return;
    }
    let hp_data = active.0.as_ref().and_then(|c| ship_view.hull_for(c));
    for mut vis in &mut bg_q {
        *vis = if hp_data.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    }
    for (mut node, mut vis, mut color) in &mut fill_q {
        match hp_data {
            None => *vis = Visibility::Hidden,
            Some((cur, max)) => {
                *vis = Visibility::Inherited;
                let fraction = if max > 0.0 { (cur / max).clamp(0.0, 1.0) } else { 0.0 };
                node.width = Val::Percent(fraction * 100.0);
                color.0 = if fraction < 0.35 { HULL_BAR_FILL_LOW } else { HULL_BAR_FILL };
            }
        }
    }
    for mut text in &mut text_q {
        text.0 = match hp_data {
            None => String::new(),
            Some((cur, max)) => format!("{} / {}", cur.round() as i32, max.round() as i32),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{RadarStateSnapshot, ShipClientConfig, SimSnapshot};

    fn base_snapshot() -> SimSnapshot {
        SimSnapshot {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: 0.0,
            ship_z: 0.0,
            ship_yaw: 0.0,
            forward_speed: 0.0,
            power_levels: (2, 2, 2),
            impulse_charge_progress: 0.0,
            flags: vec![],
            entity_states: vec![],
            radar_state: RadarStateSnapshot::default(),
            console_hull: vec![],
        }
    }

    #[test]
    fn sim_state_updates_red_alert() {
        let mut view = ShipView::default();
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot { red_alert: true, ..base_snapshot() },
        });
        assert!(view.red_alert);
    }

    #[test]
    fn sim_state_clears_red_alert() {
        let mut view = ShipView { red_alert: true, ..Default::default() };
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot { red_alert: false, ..base_snapshot() },
        });
        assert!(!view.red_alert);
    }

    #[test]
    fn welcome_resets_to_default() {
        let mut view = ShipView { red_alert: true, ship_x: 100.0, ..Default::default() };
        view.apply(&ServerMessage::Welcome {
            state: crate::messages::GameState {
                phase: crate::messages::GamePhase::InProgress,
                players: vec![],
                complexity: std::collections::HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        });
        assert!(!view.red_alert);
        assert_eq!(view.ship_x, 0.0);
    }

    #[test]
    fn sim_state_updates_position_and_power() {
        let mut view = ShipView::default();
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                ship_x: 12.5,
                ship_z: -7.25,
                power_levels: (3, 2, 4),
                ..base_snapshot()
            },
        });
        assert_eq!(view.ship_x, 12.5);
        assert_eq!(view.ship_z, -7.25);
        assert_eq!(view.power_levels, (3, 2, 4));
    }

    #[test]
    fn sim_state_updates_forward_speed() {
        let mut view = ShipView::default();
        assert_eq!(view.forward_speed, 0.0);
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                forward_speed: 18.5,
                ..base_snapshot()
            },
        });
        assert!((view.forward_speed - 18.5).abs() < 1e-6);
    }

    #[test]
    fn sim_state_negative_forward_speed_propagates() {
        let mut view = ShipView::default();
        view.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                forward_speed: -5.0,
                ..base_snapshot()
            },
        });
        assert!((view.forward_speed - (-5.0)).abs() < 1e-6);
    }

    #[test]
    fn is_active_camera_direction_only_matches_in_camera_mode() {
        use crate::messages::ViewDirection;
        let mut view = ShipView::default();
        // Default view mode is Camera(Fore).
        assert!( view.is_active_camera_direction(&ViewDirection::Fore));
        assert!(!view.is_active_camera_direction(&ViewDirection::Aft));

        view.view_mode = ViewMode::Camera(ViewDirection::Port);
        assert!( view.is_active_camera_direction(&ViewDirection::Port));
        assert!(!view.is_active_camera_direction(&ViewDirection::Fore));

        view.view_mode = ViewMode::Radar;
        for d in [ViewDirection::Fore, ViewDirection::Aft, ViewDirection::Port, ViewDirection::Starboard] {
            assert!(!view.is_active_camera_direction(&d), "Radar mode highlights no cross arrow");
        }
    }
}
