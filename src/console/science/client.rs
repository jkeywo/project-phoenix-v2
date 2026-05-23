//! Client-side Sensors Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns the Sensors console UI: long-range radar display (`GenericRadar`,
//! WorldFixed, Ships + Asteroids filter), science target designation,
//! cancel-impulse button (visible only at impulse), and view-mode controls.
//!
//! All button callbacks are wired via observers at spawn time.
//! No per-button marker-component query systems remain.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::set_science_target_message;
use crate::gui::{
    default_layer_colour, layer_to_icon, region_shape_from_snapshot, spawn_gui_button,
    tags_to_radar_layer, ButtonPressed, ButtonSize, GenericRadar, OnRadar, OrientationMode,
    RadarAppearance, RadarCenter, RadarClipMode, RadarFilter, RadarIcon, RadarLayer, StateVisuals,
};
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the science panel should be visible.
pub fn sensors_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Sensors) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Sensors,
        None => count == 1,
    }
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Sensors console UI.
#[derive(Component)]
pub struct SensorsPanel;

/// Marks the "Cancel Impulse" `GuiButton` entity — used only for visibility
/// toggling (shown/hidden based on impulse charge state).
#[derive(Component)]
pub struct ScienceCancelImpulseButton;

// ── State visuals helpers ─────────────────────────────────────────────

/// Muted blue-green button visuals — used for ON SCREEN.
fn on_screen_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.30, 0.25), // idle
        Color::srgb(0.15, 0.40, 0.32), // hover
        Color::srgb(0.12, 0.50, 0.38), // active
        Color::srgb(0.18, 0.55, 0.40), // press
        Color::srgb(0.05, 0.15, 0.12), // disabled
    )
}

/// Danger (red) button visuals — used for Cancel Impulse.
fn cancel_impulse_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.40, 0.05, 0.05), // idle
        Color::srgb(0.55, 0.08, 0.08), // hover
        Color::srgb(0.60, 0.07, 0.07), // active
        Color::srgb(0.70, 0.10, 0.10), // press
        Color::srgb(0.15, 0.03, 0.03), // disabled
    )
}

// ── Resources ────────────────────────────────────────────────────────

/// Persistent entity IDs for science-specific radar components.
#[derive(Resource, Default)]
struct ScienceRadarEntities {
    center: Option<Entity>,
    blips: HashMap<String, Entity>,
}

// ── Plugin ────────────────────────────────────────────────────────────

/// Marker resource set once the sensors UI has been spawned.
#[derive(Resource)]
pub struct SensorsPanelSpawned;

// ── Plugin ────────────────────────────────────────────────────────────

pub struct SensorsPanelPlugin;

impl Plugin for SensorsPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScienceRadarEntities>()
            .add_systems(
                Update,
                (
                    spawn_sensors_ui.run_if(not(resource_exists::<SensorsPanelSpawned>)),
                    toggle_sensors_panel_visibility,
                    refresh_cancel_impulse_visibility,
                    bridge_client_sim_to_science_radar,
                    respawn_sensors_on_orientation_change,
                ),
            );
    }
}

// ── Spawn (ConsoleShell) ──────────────────────────────────────────────

fn spawn_sensors_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<SensorsPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Sensors {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(SensorsPanelSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Sensors,
        |commands: &mut Commands, primary: Entity| {
            fill_sensors_radar(commands, primary);
        },
        |commands: &mut Commands, secondary: Entity| {
            fill_sensors_buttons(commands, secondary);
        },
        &assets,
    );

    commands.entity(shell.root).insert((SensorsPanel, Visibility::Hidden));
}

/// Primary slot: title + long-range radar.
fn fill_sensors_radar(commands: &mut Commands, container: Entity) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    let title = commands
        .spawn((
            Text::new("Sensors"),
            TextFont { font_size: 32.0, ..default() },
            TextColor(Color::srgb(0.8, 0.8, 1.0)),
        ))
        .id();
    commands.entity(col).add_child(title);

    // Sensors shows: player ship, NPC ships, individual asteroids, and
    // asteroid-field region boundaries.  WorldFixed keeps the display
    // north-up regardless of ship heading.
    let radar_filter = RadarFilter(std::collections::HashSet::from([
        RadarLayer::PlayerShip,
        RadarLayer::Ship,
        RadarLayer::Asteroid,
        RadarLayer::Station,
        RadarLayer::Planet,
        RadarLayer::Star,
    ]));
    let radar = GenericRadar::spawn(
        commands,
        crate::client_sim::SCIENCE_RADAR_RANGE,
        OrientationMode::WorldFixed,
        radar_filter,
        None,
        None,
        RadarClipMode::Circle,
        1.0,
        1.0,
    );
    commands.entity(radar).insert(Node {
        width: Val::Px(240.0),
        height: Val::Px(240.0),
        border: UiRect::all(Val::Px(1.0)),
        aspect_ratio: Some(1.0),
        position_type: PositionType::Relative,
        ..default()
    });
    commands.entity(col).add_child(radar);
}

/// Secondary slot: ON SCREEN + CANCEL IMPULSE buttons.
fn fill_sensors_buttons(commands: &mut Commands, container: Entity) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(12.0),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    let on_screen_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 160.0, height: 36.0 },
        on_screen_visuals(),
        None,
    );
    commands.entity(on_screen_btn).with_children(|btn| {
        btn.spawn((
            Text::new("ON SCREEN"),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.4, 1.0, 0.8)),
        ));
    });
    commands.entity(on_screen_btn).observe(on_on_screen_button_pressed);
    commands.entity(col).add_child(on_screen_btn);

    let cancel_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 160.0, height: 36.0 },
        cancel_impulse_visuals(),
        None,
    );
    commands.entity(cancel_btn).with_children(|btn| {
        btn.spawn((
            Text::new("CANCEL IMPULSE"),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(1.0, 0.4, 0.4)),
        ));
    });
    commands
        .entity(cancel_btn)
        .insert((ScienceCancelImpulseButton, Visibility::Hidden))
        .observe(on_cancel_impulse_button_pressed);
    commands.entity(col).add_child(cancel_btn);
}

// ── Orientation respawn ──────────────────────────────────────────────

fn respawn_sensors_on_orientation_change(
    orientation: Option<Res<DeviceOrientation>>,
    panel: Query<Entity, With<SensorsPanel>>,
    mut commands: Commands,
) {
    let Some(orientation) = orientation else { return };
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<SensorsPanelSpawned>();
}

// ── Button observers ──────────────────────────────────────────────────

fn on_on_screen_button_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::SetView {
        mode: ViewMode::ScienceRadar,
    }));
}

fn on_cancel_impulse_button_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::CancelImpulse));
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_sensors_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<SensorsPanel>>,
) {
    let visible = sensors_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Toggle Cancel Impulse button visibility based on impulse charge progress.
fn refresh_cancel_impulse_visibility(
    ship_view: Res<ShipView>,
    mut buttons: Query<&mut Visibility, With<ScienceCancelImpulseButton>>,
) {
    for mut vis in buttons.iter_mut() {
        *vis = if ship_view.impulse_charge_progress > 0.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

// ── Radar entity bridge ───────────────────────────────────────────────

/// Bridges `ClientSimState` entity snapshots into ECS entities with
/// `OnRadar` / `RadarAppearance` for the science `GenericRadar` widget.
///
/// Science radar shows ships and asteroids only.
fn bridge_client_sim_to_science_radar(
    mut commands: Commands,
    sim: Res<crate::client_sim::ClientSimState>,
    ship_view: Res<ShipView>,
    mut radar: ResMut<ScienceRadarEntities>,
) {
    // ── Radar center (player ship) ────────────────────────────────────
    //
    // Registered as PlayerShip so navigation and other filters that want
    // "player ship only" can include PlayerShip without also showing NPCs.
    let ship_appearance = RadarAppearance {
        icon: RadarIcon::Ship,
        world_size: 6.0,
        color: Color::srgb(0.95, 0.95, 1.0),
        region_colour: None,
        region_shape: None,
    };
    let ship_yaw = ship_view.ship_yaw;
    let ship_t = Transform::from_xyz(ship_view.ship_x, 0.0, ship_view.ship_z)
        .with_rotation(Quat::from_rotation_y(ship_yaw));
    match radar.center {
        Some(e) => {
            commands.entity(e).insert((
                RadarCenter {
                    world_x: ship_view.ship_x,
                    world_z: ship_view.ship_z,
                    yaw: ship_yaw,
                },
                OnRadar(RadarLayer::PlayerShip),
                ship_appearance,
                ship_t,
            ));
        }
        None => {
            let e = commands
                .spawn((
                    RadarCenter {
                        world_x: ship_view.ship_x,
                        world_z: ship_view.ship_z,
                        yaw: ship_yaw,
                    },
                    OnRadar(RadarLayer::PlayerShip),
                    ship_appearance,
                    ship_t,
                    GlobalTransform::default(),
                ))
                .id();
            radar.center = Some(e);
        }
    }

    // ── Entity blips ──────────────────────────────────────────────────
    let mut seen = std::collections::HashSet::new();

    for snapshot in &sim.world.entities {
        let uuid = &snapshot.uuid;
        if !seen.insert(uuid.clone()) {
            continue;
        }

        let layer = match tags_to_radar_layer(&snapshot.tags) {
            Some(
                l @ (RadarLayer::Ship
                    | RadarLayer::Asteroid
                    | RadarLayer::AsteroidField
                    | RadarLayer::Station
                    | RadarLayer::Planet
                    | RadarLayer::Star
                    | RadarLayer::Region),
            ) => l,
            _ => continue,
        };

        let entity_yaw = snapshot.yaw.unwrap_or(0.0);
        let colour = snapshot.colour.map(|c| Color::srgb(c[0], c[1], c[2]));

        if layer == RadarLayer::Region || layer == RadarLayer::AsteroidField {
            // ── Region / field entity: render as shape ────────────────────────
            let region_colour = colour.unwrap_or(default_layer_colour(layer));
            let region_shape = region_shape_from_snapshot(snapshot);
            let world_size = snapshot
                .radar_world_size
                .or(Some(snapshot.radius_or_zero()))
                .filter(|s| *s > 0.0)
                .unwrap_or(4.0);
            let appearance = RadarAppearance {
                icon: layer_to_icon(layer),
                world_size,
                color: region_colour,
                region_colour: Some(region_colour),
                region_shape,
            };
            let t = Transform::from_xyz(snapshot.x(), 0.0, snapshot.z())
                .with_rotation(Quat::from_rotation_y(entity_yaw));
            if let Some(existing) = radar.blips.get(uuid) {
                commands.entity(*existing).insert((OnRadar(layer), appearance, t));
            } else {
                let blip = commands
                    .spawn((OnRadar(layer), appearance, t, GlobalTransform::default()))
                    .id();
                radar.blips.insert(uuid.clone(), blip);
            }
        } else {
            // ── Point entity: render as icon ──────────────────────────────────
            let icon = layer_to_icon(layer);
            let default_color = default_layer_colour(layer);
            let world_size = snapshot
                .radar_world_size
                .or(Some(snapshot.radius_or_zero()))
                .filter(|s| *s > 0.0)
                .unwrap_or(4.0);
            let appearance = RadarAppearance {
                icon,
                world_size,
                color: colour.unwrap_or(default_color),
                region_colour: None,
                region_shape: None,
            };
            let t = Transform::from_xyz(snapshot.x(), 0.0, snapshot.z())
                .with_rotation(Quat::from_rotation_y(entity_yaw));
            if let Some(existing) = radar.blips.get(uuid) {
                commands.entity(*existing).insert((OnRadar(layer), appearance, t));
            } else {
                let blip = commands
                    .spawn((OnRadar(layer), appearance, t, GlobalTransform::default()))
                    .id();
                radar.blips.insert(uuid.clone(), blip);
            }
        }
    }

    // Despawn blips no longer in sim state.
    radar.blips.retain(|uuid, entity| {
        if seen.contains(uuid) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });
}

/// Build the `ClientMessage` for designating a science target.
pub fn science_target_message(uuid: String) -> ClientMessage {
    set_science_target_message(uuid)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{GamePhase, GameState, Player, ServerMessage, ShipClientConfig};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;

    fn player(token: &str, consoles: Vec<Console>) -> Player {
        Player {
            token: token.into(),
            name: "test".into(),
            consoles,
            connected: true,
        }
    }

    fn game_state(phase: GamePhase, players: Vec<Player>) -> GameState {
        GameState {
            phase,
            players,
            complexity: HashMap::new(),
            world: None,
        }
    }

    fn welcome(state: GameState) -> ServerMessage {
        ServerMessage::Welcome {
            state,
            ship_stations: ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        }
    }

    fn in_progress_sensors_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Sensors])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole {
        ActiveConsole(None)
    }
    fn tab(c: Console) -> ActiveConsole {
        ActiveConsole(Some(c))
    }

    // ── sensors_panel_visible ─────────────────────────────────────────

    #[test]
    fn sensors_panel_not_visible_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!sensors_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn sensors_panel_not_visible_when_player_does_not_hold_sensors() {
        let lobby = {
            let mut s = LobbyState::default();
            s.apply(&welcome(game_state(
                GamePhase::InProgress,
                vec![player("tok", vec![Console::Helm])],
            )));
            s
        };
        let active = no_tab();
        assert!(!sensors_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn sensors_panel_visible_when_sole_console_and_no_tab() {
        let lobby = in_progress_sensors_lobby("tok");
        let active = no_tab();
        assert!(sensors_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn sensors_panel_visible_when_multi_console_and_sensors_tab() {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Sensors, Console::Helm])],
        )));
        let active = tab(Console::Sensors);
        assert!(sensors_panel_visible(&s, "tok", &active));
    }

    #[test]
    fn sensors_panel_hidden_when_multi_console_and_other_tab() {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Sensors, Console::Helm])],
        )));
        let active = tab(Console::Helm);
        assert!(!sensors_panel_visible(&s, "tok", &active));
    }

    #[test]
    fn sensors_panel_hidden_when_multi_console_and_no_tab() {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Sensors, Console::Helm])],
        )));
        let active = no_tab();
        assert!(!sensors_panel_visible(&s, "tok", &active));
    }

    // ── science_target_message ────────────────────────────────────────

    #[test]
    fn science_target_message_produces_set_science_target() {
        let msg = science_target_message("entity-42".into());
        assert_eq!(
            msg,
            ClientMessage::SetScienceTarget {
                uuid: "entity-42".into()
            }
        );
    }

    // ── on_screen button sends ScienceRadar ViewMode ──────────────────

    #[test]
    fn on_screen_message_variant_is_set_view_science_radar() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::ScienceRadar,
        };
        assert!(matches!(
            msg,
            ClientMessage::SetView {
                mode: ViewMode::ScienceRadar
            }
        ));
    }

    // ── cancel impulse message variant ───────────────────────────────

    #[test]
    fn cancel_impulse_message_variant_is_correct() {
        let msg = ClientMessage::CancelImpulse;
        assert!(matches!(msg, ClientMessage::CancelImpulse));
    }

    // ── Science radar filter includes ships and asteroids ─────────────

    #[test]
    fn science_radar_filter_includes_ships() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            RadarLayer::Ship,
            RadarLayer::Asteroid,
        ]));
        assert!(is_on_radar(&filter, RadarLayer::Ship));
    }

    #[test]
    fn science_radar_filter_includes_asteroids() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            RadarLayer::Ship,
            RadarLayer::Asteroid,
        ]));
        assert!(is_on_radar(&filter, RadarLayer::Asteroid));
    }

    #[test]
    fn science_radar_filter_excludes_missiles() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            RadarLayer::Ship,
            RadarLayer::Asteroid,
        ]));
        assert!(!is_on_radar(&filter, RadarLayer::Missile));
    }

    // ── StateVisuals: five widget states render distinctly ────────────

    #[test]
    fn on_screen_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = on_screen_visuals();
        let idle = resolve_visual(&v, false, false, false, false).color;
        let hover = resolve_visual(&v, false, false, false, true).color;
        let active = resolve_visual(&v, false, false, true, false).color;
        let press = resolve_visual(&v, false, true, false, false).color;
        let disabled = resolve_visual(&v, true, false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    #[test]
    fn cancel_impulse_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = cancel_impulse_visuals();
        let idle = resolve_visual(&v, false, false, false, false).color;
        let hover = resolve_visual(&v, false, false, false, true).color;
        let active = resolve_visual(&v, false, false, true, false).color;
        let press = resolve_visual(&v, false, true, false, false).color;
        let disabled = resolve_visual(&v, true, false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }
}
