//! Client-side Weapons Panel plugin.
//!
//! Owns all Tactical console UI: fire button, phaser mode toggle, torpedo
//! tube selection, torpedo count display, and gizmo-based radar overlay.
//!
//! Extracted from `client/app.rs` as part of the "Client split" series.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::{
    WeaponsPanel, WeaponsRadarPanel, OutboundClientMessage, RepairIconLabel,
    HideableElement, ComplexityPopupRoot, ComplexityPresetButton, ComplexityPopupConfirm,
    ComplexityDropdownRoot,
};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::{
    fire_phaser_message, set_phaser_mode_message, fire_torpedo_message, ClientSimState,
};
use crate::messages::{Console, GamePhase, PhaserMode, TorpedoTube};
use crate::ship_view::ShipView;

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the weapons panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Tactical`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Tactical`.
pub fn weapons_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Tactical) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Tactical,
        None => count == 1,
    }
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the "FIRE PHASERS" button on the Weapons console.
#[derive(Component)]
struct FirePhaserButton;

/// Marks the text label inside the Fire button (used to show cooldown status).
#[derive(Component)]
struct FirePhaserLabel;

/// Marks the phaser mode toggle button (Auto / Manual).
#[derive(Component)]
struct PhaserModeButton;

/// Marks the text label inside the mode button.
#[derive(Component)]
struct PhaserModeLabel;

/// Tracks which torpedo tube is currently selected on the Weapons console.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelectedTube(pub Option<TorpedoTube>);

/// Marks a torpedo tube selection button. Contains the tube it represents.
#[derive(Component)]
struct TorpedoTubeButton(TorpedoTube);

/// Marks the "FIRE TORPEDO" button on the Weapons console.
#[derive(Component)]
struct FireTorpedoButton;

/// Marks the text label inside the Fire Torpedo button.
#[derive(Component)]
struct FireTorpedoLabel;

/// Marks the torpedo count text label.
#[derive(Component)]
struct TorpedoCountLabel;

/// Marks the label that shows tube reload status.
#[derive(Component)]
struct TubeStatusLabel(TorpedoTube);

// ── Constants ────────────────────────────────────────────────────────

const RADAR_OUTER_RING_COLOR: Color = Color::srgb(0.55, 0.70, 1.0);
const RADAR_MID_RING_COLOR:   Color = Color::srgb(0.30, 0.40, 0.65);
const RADAR_ASTEROID_COLOR:   Color = Color::srgb(0.85, 0.75, 0.45);
const RADAR_SHIP_COLOR:       Color = Color::srgb(0.95, 0.95, 1.0);

// ── Plugin ───────────────────────────────────────────────────────────

/// Plugin that owns all Tactical console UI and systems.
pub struct WeaponsPanelPlugin;

impl Plugin for WeaponsPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SelectedTube>()
            .add_systems(Startup, setup_weapons_ui)
            .add_systems(Update, (
                toggle_weapons_panel_visibility,
                handle_fire_phaser_button_press,
                handle_phaser_mode_toggle_press,
                handle_torpedo_tube_button_press,
                handle_fire_torpedo_button_press,
                refresh_weapons_panel,
                refresh_torpedo_ui,
                draw_weapons_radar,
            ));
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_weapons_ui(mut commands: Commands) {
    commands
        .spawn((
            WeaponsPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(0.0),
                right:  Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(16.0),
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            // Tactical radar display (drawn via gizmos using WeaponsRadarPanel bounds)
            panel.spawn((
                WeaponsRadarPanel,
                Node {
                    width:  Val::Px(240.0),
                    height: Val::Px(240.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BorderColor::all(Color::srgb(0.55, 0.70, 1.0)),
                BackgroundColor(Color::srgb(0.06, 0.08, 0.14)),
            ));

            panel.spawn((
                Text::new("Weapons Console"),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::srgb(1.0, 0.5, 0.2)),
            ));

            // ── Torpedo section (hideable as "torpedo_tube_selector") ──
            panel.spawn((
                HideableElement("torpedo_tube_selector".into()),
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
            )).with_children(|container| {
                // Torpedo count label
                container.spawn((
                    TorpedoCountLabel,
                    Text::new("Torpedoes: 10"),
                    TextFont { font_size: 16.0, ..default() },
                    TextColor(Color::srgb(0.8, 0.8, 0.2)),
                ));

                // Torpedo tube selection row
                container.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        ..default()
                    },
                )).with_children(|row| {
                    for (tube, label) in [
                        (TorpedoTube::ForePort, "FWD PORT"),
                        (TorpedoTube::ForeStarboard, "FWD STBD"),
                        (TorpedoTube::Aft, "AFT"),
                    ] {
                        row.spawn((
                            TorpedoTubeButton(tube),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.10, 0.20, 0.30)),
                        )).with_children(|btn| {
                            btn.spawn((
                                Text::new(label),
                                TextFont { font_size: 14.0, ..default() },
                                TextColor(Color::srgb(0.6, 0.8, 1.0)),
                            ));
                        });
                    }
                });

                // Tube status labels row
                container.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        ..default()
                    },
                )).with_children(|row| {
                    for tube in [
                        TorpedoTube::ForePort,
                        TorpedoTube::ForeStarboard,
                        TorpedoTube::Aft,
                    ] {
                        row.spawn((
                            TubeStatusLabel(tube),
                            Text::new("LOADED"),
                            TextFont { font_size: 12.0, ..default() },
                            TextColor(Color::srgb(0.3, 1.0, 0.3)),
                            Node {
                                min_width: Val::Px(70.0),
                                ..default()
                            },
                        ));
                    }
                });

                // Fire torpedo button
                container
                    .spawn((
                        FireTorpedoButton,
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(32.0), Val::Px(16.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.10, 0.30, 0.10)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            FireTorpedoLabel,
                            Text::new("FIRE TORPEDO"),
                            TextFont { font_size: 22.0, ..default() },
                            TextColor(Color::srgb(0.3, 1.0, 0.3)),
                        ));
                    });
            }); // ── end torpedo container ──

            // Phaser mode toggle button (hideable as "phaser_mode_selector")
            panel
                .spawn((
                    HideableElement("phaser_mode_selector".into()),
                    PhaserModeButton,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(24.0), Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.15, 0.15, 0.35)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        PhaserModeLabel,
                        Text::new("Mode: AUTO"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.7, 0.7, 1.0)),
                    ));
                });

            // Fire phasers button
            panel
                .spawn((
                    FirePhaserButton,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(32.0), Val::Px(16.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.40, 0.10, 0.10)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        FirePhaserLabel,
                        Text::new("FIRE PHASERS"),
                        TextFont { font_size: 22.0, ..default() },
                        TextColor(Color::srgb(1.0, 0.5, 0.2)),
                    ));
                });

            // Repair icon label — shows when a breakdown or decoy icon
            // targets this console.
            panel.spawn((
                RepairIconLabel,
                Text::new(""),
                TextFont { font_size: 14.0, ..default() },
                TextColor(Color::srgb(0.8, 0.5, 0.2)),
            ));

            // ── Complexity dropdown row ──────────────────────────────
            panel.spawn((
                ComplexityDropdownRoot,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                    ..default()
                },
                Visibility::Hidden,
                BackgroundColor(Color::srgb(0.08, 0.08, 0.12)),
            )).with_children(|row| {
                row.spawn((
                    Text::new("Complexity:"),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.7, 0.8)),
                ));
                for (preset, label) in [("Low", "Low"), ("Std", "Normal")] {
                    row.spawn((
                        ComplexityPresetButton(preset.to_string()),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.15, 0.20, 0.35)),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new(label),
                            TextFont { font_size: 13.0, ..default() },
                            TextColor(Color::srgb(0.7, 0.8, 1.0)),
                        ));
                    });
                }
            });

            // ── Complexity pop-up overlay ────────────────────────────
            panel.spawn((
                ComplexityPopupRoot,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    right: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(12.0),
                    ..default()
                },
                Visibility::Hidden,
                BackgroundColor(Color::srgba(0.05, 0.05, 0.15, 0.95)),
            )).with_children(|popup| {
                popup.spawn((
                    Text::new("Choose Complexity Preset"),
                    TextFont { font_size: 20.0, ..default() },
                    TextColor(Color::srgb(0.8, 0.8, 1.0)),
                ));
                popup.spawn((
                    Text::new("Select a complexity level for this console."),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.6, 0.8)),
                ));
                for (preset, label) in [("Low", "Low"), ("Std", "Normal")] {
                    popup.spawn((
                        ComplexityPresetButton(preset.to_string()),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(32.0), Val::Px(12.0)),
                            min_width: Val::Px(180.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.15, 0.25, 0.40)),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new(label),
                            TextFont { font_size: 18.0, ..default() },
                            TextColor(Color::srgb(0.7, 0.8, 1.0)),
                        ));
                    });
                }
                popup.spawn((
                    ComplexityPopupConfirm,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(48.0), Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.10, 0.40, 0.20)),
                )).with_children(|btn| {
                    btn.spawn((
                        Text::new("Confirm"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.5, 1.0, 0.5)),
                    ));
                });
            });
        });
}

// ── Visibility system ────────────────────────────────────────────────

fn toggle_weapons_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<WeaponsPanel>>,
) {
    let visible = weapons_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

// ── Phaser systems ───────────────────────────────────────────────────

fn handle_fire_phaser_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<FirePhaserButton>)>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Suppress if on cooldown.
        if sim.on_cooldown {
            continue;
        }
        outbound.write(OutboundClientMessage(fire_phaser_message()));
    }
}

fn handle_phaser_mode_toggle_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<PhaserModeButton>)>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Toggle between Auto and Manual.
        let new_mode = match sim.phaser_mode {
            PhaserMode::Auto => PhaserMode::Manual,
            PhaserMode::Manual => PhaserMode::Auto,
        };
        outbound.write(OutboundClientMessage(set_phaser_mode_message(new_mode)));
    }
}

fn refresh_weapons_panel(
    sim: Res<ClientSimState>,
    mut fire_bg: Query<&mut BackgroundColor, (With<FirePhaserButton>, Without<PhaserModeButton>)>,
    mut fire_label: Query<(&mut Text, &mut TextColor), With<FirePhaserLabel>>,
    mut mode_label: Query<(&mut Text, &mut TextColor), (With<PhaserModeLabel>, Without<FirePhaserLabel>)>,
) {
    if !sim.is_changed() {
        return;
    }
    // Update fire button appearance.
    for mut bg in fire_bg.iter_mut() {
        *bg = if sim.on_cooldown {
            BackgroundColor(Color::srgb(0.20, 0.05, 0.05))
        } else if sim.fire_ready {
            BackgroundColor(Color::srgb(0.60, 0.10, 0.10))
        } else {
            BackgroundColor(Color::srgb(0.30, 0.08, 0.08))
        };
    }
    for (mut text, mut color) in fire_label.iter_mut() {
        if sim.on_cooldown {
            **text = "COOLING DOWN".to_string();
            *color = TextColor(Color::srgb(0.5, 0.2, 0.2));
        } else {
            **text = "FIRE PHASERS".to_string();
            *color = if sim.fire_ready {
                TextColor(Color::srgb(1.0, 0.5, 0.2))
            } else {
                TextColor(Color::srgb(0.5, 0.3, 0.2))
            };
        }
    }
    // Update mode button label.
    for (mut text, _) in mode_label.iter_mut() {
        **text = match sim.phaser_mode {
            PhaserMode::Auto => "Mode: AUTO".to_string(),
            PhaserMode::Manual => "Mode: MANUAL".to_string(),
        };
    }
}

// ── Torpedo systems ──────────────────────────────────────────────────

/// Handle torpedo tube selection button presses — update `SelectedTube`.
fn handle_torpedo_tube_button_press(
    interactions: Query<(&Interaction, &TorpedoTubeButton), Changed<Interaction>>,
    mut selected: ResMut<SelectedTube>,
) {
    for (interaction, tube_btn) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Toggle: press same tube again to deselect.
        if selected.0 == Some(tube_btn.0) {
            selected.0 = None;
        } else {
            selected.0 = Some(tube_btn.0);
        }
    }
}

/// Handle the Fire Torpedo button press. Fires from the selected tube with
/// the current target as the homing target (if any).
fn handle_fire_torpedo_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<FireTorpedoButton>)>,
    selected: Res<SelectedTube>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(tube) = selected.0 else { continue };
        // Check the chosen tube is loaded.
        let loaded = match tube {
            TorpedoTube::ForePort => sim.fore_port_loaded,
            TorpedoTube::ForeStarboard => sim.fore_starboard_loaded,
            TorpedoTube::Aft => sim.aft_loaded,
        };
        if !loaded || sim.torpedo_count == 0 {
            continue;
        }
        outbound.write(OutboundClientMessage(fire_torpedo_message(tube, None)));
    }
}

/// Refresh the torpedo UI (count, tube status labels, fire button, tube selection highlights).
fn refresh_torpedo_ui(
    sim: Res<ClientSimState>,
    selected: Res<SelectedTube>,
    mut count_label: Query<&mut Text, With<TorpedoCountLabel>>,
    mut tube_status: Query<(&mut Text, &mut TextColor, &TubeStatusLabel), (Without<TorpedoCountLabel>, Without<FireTorpedoLabel>)>,
    mut fire_bg: Query<&mut BackgroundColor, With<FireTorpedoButton>>,
    mut fire_label: Query<(&mut Text, &mut TextColor), (With<FireTorpedoLabel>, Without<TorpedoCountLabel>, Without<TubeStatusLabel>)>,
    mut tube_btn_bg: Query<(&mut BackgroundColor, &TorpedoTubeButton), Without<FireTorpedoButton>>,
) {
    if !sim.is_changed() && !selected.is_changed() {
        return;
    }

    // Update torpedo count label.
    for mut text in count_label.iter_mut() {
        **text = format!("Torpedoes: {}", sim.torpedo_count);
    }

    // Update per-tube status labels.
    for (mut text, mut color, label) in tube_status.iter_mut() {
        let (loaded, reload_secs) = match label.0 {
            TorpedoTube::ForePort =>
                (sim.fore_port_loaded, sim.fore_port_reload_secs),
            TorpedoTube::ForeStarboard =>
                (sim.fore_starboard_loaded, sim.fore_starboard_reload_secs),
            TorpedoTube::Aft =>
                (sim.aft_loaded, sim.aft_reload_secs),
        };
        if loaded {
            **text = "LOADED".to_string();
            *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
        } else {
            **text = format!("{:.0}s", reload_secs.ceil());
            *color = TextColor(Color::srgb(1.0, 0.6, 0.2));
        }
    }

    // Update tube selection button highlights.
    for (mut bg, tube_btn) in tube_btn_bg.iter_mut() {
        let is_selected = selected.0 == Some(tube_btn.0);
        *bg = if is_selected {
            BackgroundColor(Color::srgb(0.10, 0.50, 0.70))
        } else {
            BackgroundColor(Color::srgb(0.10, 0.20, 0.30))
        };
    }

    // Update Fire Torpedo button appearance.
    let tube_ready = selected.0.map(|t| match t {
        TorpedoTube::ForePort => sim.fore_port_loaded,
        TorpedoTube::ForeStarboard => sim.fore_starboard_loaded,
        TorpedoTube::Aft => sim.aft_loaded,
    }).unwrap_or(false);
    let can_fire = tube_ready && sim.torpedo_count > 0 && selected.0.is_some();

    for mut bg in fire_bg.iter_mut() {
        *bg = if can_fire {
            BackgroundColor(Color::srgb(0.10, 0.50, 0.10))
        } else {
            BackgroundColor(Color::srgb(0.05, 0.20, 0.05))
        };
    }
    for (mut text, mut color) in fire_label.iter_mut() {
        if selected.0.is_none() {
            **text = "SELECT TUBE".to_string();
            *color = TextColor(Color::srgb(0.5, 0.5, 0.5));
        } else if sim.torpedo_count == 0 {
            **text = "NO TORPEDOES".to_string();
            *color = TextColor(Color::srgb(0.6, 0.3, 0.3));
        } else if !tube_ready {
            **text = "TUBE LOADING".to_string();
            *color = TextColor(Color::srgb(0.8, 0.5, 0.2));
        } else {
            **text = "FIRE TORPEDO".to_string();
            *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
        }
    }
}

// ── Radar gizmo ──────────────────────────────────────────────────────

fn draw_weapons_radar(
    mut gizmos: Gizmos,
    panel: Query<(&ComputedNode, &GlobalTransform, &ViewVisibility), With<WeaponsRadarPanel>>,
    weapons_panel: Query<&Visibility, With<WeaponsPanel>>,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    windows: Query<&Window>,
) {
    if !weapons_panel
        .iter()
        .any(|v| matches!(v, Visibility::Visible | Visibility::Inherited))
    {
        return;
    }
    let Ok((node, gt, view_vis)) = panel.single() else { return };
    if !view_vis.get() {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let viewport_w = window.width();
    let viewport_h = window.height();

    let node_size = node.size();
    let node_centre_screen = gt.translation().truncate();
    let centre_world_x = node_centre_screen.x - viewport_w / 2.0;
    let centre_world_y = viewport_h / 2.0 - node_centre_screen.y;
    let centre = Vec2::new(centre_world_x, centre_world_y);

    let radius = node_size.x.min(node_size.y) * 0.5;
    if radius <= 0.0 {
        return;
    }

    gizmos.circle_2d(centre, radius, RADAR_OUTER_RING_COLOR);
    let weapons_range = crate::client_sim::weapons_radar_config().range;
    let mid_ratio = crate::radar::RADAR_MID_RING / weapons_range;
    gizmos.circle_2d(centre, radius * mid_ratio, RADAR_MID_RING_COLOR);

    let weapons_view = crate::client_sim::compute_weapons_radar_view(&sim, &ship_view);
    for dot in &weapons_view.dots {
        let pos = centre + Vec2::new(dot.radar_x * radius, dot.radar_y * radius);
        let pix_radius = (dot.scaled_radius * radius).max(2.0);
        gizmos.circle_2d(pos, pix_radius, RADAR_ASTEROID_COLOR);
    }

    // Ship triangle at centre
    let nose_len  = radius * 0.10;
    let half_base = radius * 0.06;
    let nose  = centre + Vec2::new(0.0,  nose_len);
    let left  = centre + Vec2::new(-half_base, -nose_len * 0.6);
    let right = centre + Vec2::new( half_base, -nose_len * 0.6);
    gizmos.line_2d(nose, left,  RADAR_SHIP_COLOR);
    gizmos.line_2d(left, right, RADAR_SHIP_COLOR);
    gizmos.line_2d(right, nose, RADAR_SHIP_COLOR);
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{GamePhase, Console, GameState, Player, ServerMessage};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;

    fn player(token: &str, consoles: Vec<Console>) -> Player {
        Player { token: token.into(), name: "test".into(), consoles, connected: true }
    }

    fn game_state(phase: GamePhase, players: Vec<Player>) -> GameState {
        GameState { phase, players, complexity: HashMap::new(), world: None }
    }

    fn welcome(state: GameState) -> ServerMessage {
        ServerMessage::Welcome { state, ship_stations: ShipStations::default() }
    }

    fn in_progress_tactical_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Tactical])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole { ActiveConsole(None) }
    fn tab(c: Console) -> ActiveConsole { ActiveConsole(Some(c)) }

    // ── weapons_panel_visible ─────────────────────────────────────────

    #[test]
    fn weapons_panel_hidden_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_hidden_when_player_not_tactical() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm])],
        )));
        let active = no_tab();
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_visible_when_sole_console_and_no_tab() {
        let lobby = in_progress_tactical_lobby("tok");
        let active = no_tab();
        assert!(weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_visible_when_multi_console_and_tactical_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = tab(Console::Tactical);
        assert!(weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_hidden_when_multi_console_and_other_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = tab(Console::Helm);
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_hidden_when_multi_console_and_no_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = no_tab();
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    // ── fire_phaser_message builder ───────────────────────────────────

    #[test]
    fn fire_phaser_message_produces_fire_phaser() {
        use crate::messages::ClientMessage;
        let msg = fire_phaser_message();
        assert_eq!(msg, ClientMessage::FirePhaser);
    }

    // ── fire_torpedo_message builder ──────────────────────────────────

    #[test]
    fn fire_torpedo_message_fore_port_no_target() {
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message(TorpedoTube::ForePort, None);
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: TorpedoTube::ForePort,
            target_uuid: None,
        });
    }

    #[test]
    fn fire_torpedo_message_aft_with_target() {
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message(TorpedoTube::Aft, Some("uuid-1".into()));
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: TorpedoTube::Aft,
            target_uuid: Some("uuid-1".into()),
        });
    }

    // ── set_phaser_mode_message builder ──────────────────────────────

    #[test]
    fn set_phaser_mode_auto_produces_correct_message() {
        use crate::messages::ClientMessage;
        let msg = set_phaser_mode_message(PhaserMode::Auto);
        assert_eq!(msg, ClientMessage::SetPhaserMode { mode: PhaserMode::Auto });
    }

    #[test]
    fn set_phaser_mode_manual_produces_correct_message() {
        use crate::messages::ClientMessage;
        let msg = set_phaser_mode_message(PhaserMode::Manual);
        assert_eq!(msg, ClientMessage::SetPhaserMode { mode: PhaserMode::Manual });
    }

    // ── SelectedTube default ──────────────────────────────────────────

    #[test]
    fn selected_tube_defaults_to_none() {
        let s = SelectedTube::default();
        assert_eq!(s.0, None);
    }

    #[test]
    fn selected_tube_toggle_selects_tube() {
        let mut selected = SelectedTube::default();
        // Simulate pressing ForePort when none selected → ForePort selected.
        let pressed = TorpedoTube::ForePort;
        if selected.0 == Some(pressed) {
            selected.0 = None;
        } else {
            selected.0 = Some(pressed);
        }
        assert_eq!(selected.0, Some(TorpedoTube::ForePort));
    }

    #[test]
    fn selected_tube_toggle_deselects_same_tube() {
        let mut selected = SelectedTube(Some(TorpedoTube::ForePort));
        // Simulate pressing ForePort when ForePort already selected → deselect.
        let pressed = TorpedoTube::ForePort;
        if selected.0 == Some(pressed) {
            selected.0 = None;
        } else {
            selected.0 = Some(pressed);
        }
        assert_eq!(selected.0, None);
    }

    #[test]
    fn selected_tube_toggle_switches_tube() {
        let mut selected = SelectedTube(Some(TorpedoTube::ForePort));
        // Simulate pressing Aft when ForePort selected → switch to Aft.
        let pressed = TorpedoTube::Aft;
        if selected.0 == Some(pressed) {
            selected.0 = None;
        } else {
            selected.0 = Some(pressed);
        }
        assert_eq!(selected.0, Some(TorpedoTube::Aft));
    }
}
