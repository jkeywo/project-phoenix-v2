//! Client-side Shields Panel plugin — migrated to `ConsoleShell` (PRD #346).
//!
//! Owns the Shields console UI: 4-quadrant HP bars, focus-facing mechanic
//! (Fore/Port/Aft/Stbd + Clear), and real-time shield status updates.
//!
//! The panel uses `ConsoleShell::spawn` so it shares the bezel-aware root,
//! embedded orientation-aware tab bar, and absolute-positioned help button
//! with the rest of the migrated consoles.
//!
//! Layout:
//! - **Primary slot:** the 4 HP rows (Fore/Port/Aft/Stbd) — the focal display.
//! - **Secondary slot:** the focus `RadioGroup` plus a "Clear" button that
//!   emits `SetShieldFocus { facing: None }` to drop the focus entirely.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    spawn_gui_button, ButtonPressed, ButtonSize, ProgressBar, ProgressBarVariant, ProgressValue,
    ReadoutValue, SegmentCount, StateVisuals, TextReadout, WidgetState,
};
use crate::messages::{ClientMessage, Console, GamePhase, ViewDirection};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the shields panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Shields`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Shields`.
pub fn shields_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Shields) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Shields,
        None => count == 1,
    }
}

// ── Pure helpers ──────────────────────────────────────────────────────

/// Build `StateVisuals` for the shield focus `RadioGroup` buttons.
///
/// Active (selected) = bright cyan; idle = dark blue-grey; disabled = dim.
pub fn focus_button_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.20, 0.35), // idle
        Color::srgb(0.14, 0.28, 0.48), // hover
        Color::srgb(0.10, 0.55, 0.75), // active (selected)
        Color::srgb(0.15, 0.35, 0.55), // press
        Color::srgb(0.05, 0.08, 0.14), // disabled
    )
}

/// Build `StateVisuals` for the Clear-focus button.
///
/// Distinct amber tint to differentiate it from the radio group.
pub fn clear_button_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.35, 0.25, 0.10),
        Color::srgb(0.50, 0.36, 0.16),
        Color::srgb(0.70, 0.50, 0.20),
        Color::srgb(0.55, 0.40, 0.18),
        Color::srgb(0.14, 0.10, 0.05),
    )
}

/// Build `StateVisuals` for a shield HP `ProgressBar`.
///
/// Active = green (focused arc); idle = blue; disabled = dim red (offline).
pub fn hp_bar_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.20, 0.40, 0.80), // idle (online, unfocused)
        Color::srgb(0.20, 0.40, 0.80), // hover (not interactive)
        Color::srgb(0.20, 0.80, 0.40), // active (focused arc)
        Color::srgb(0.20, 0.40, 0.80), // press (not interactive)
        Color::srgb(0.30, 0.10, 0.10), // disabled (offline)
    )
}

/// Build `StateVisuals` for a shield HP `TextReadout`.
pub fn hp_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.60, 0.80, 1.00), // idle
        Color::srgb(0.60, 0.80, 1.00), // hover
        Color::srgb(0.30, 1.00, 0.60), // active (focused arc)
        Color::srgb(0.60, 0.80, 1.00), // press
        Color::srgb(0.35, 0.35, 0.45), // disabled (offline)
    )
}

/// Compute the HP fraction `[0.0, 1.0]` for a shield facing.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn hp_fraction(hp: i32, max_hp: i32) -> f32 {
    if max_hp <= 0 {
        0.0
    } else {
        (hp as f32 / max_hp as f32).clamp(0.0, 1.0)
    }
}

/// Format an HP readout string given current and max hp.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn hp_readout_text(hp: i32, max_hp: i32) -> String {
    format!("{}/{}", hp, max_hp)
}

/// Map a `ViewDirection` to the facing label used in `ShieldFacingStatus`.
pub fn direction_to_label(dir: &ViewDirection) -> &'static str {
    match dir {
        ViewDirection::Fore => "Fore",
        ViewDirection::Port => "Port",
        ViewDirection::Aft => "Aft",
        ViewDirection::Starboard => "Starboard",
    }
}

/// The ordered list of quadrant directions for the focus `RadioGroup`.
///
/// Index 0 = Fore, 1 = Port, 2 = Aft, 3 = Starboard.
pub const FOCUS_DIRECTIONS: [ViewDirection; 4] = [
    ViewDirection::Fore,
    ViewDirection::Port,
    ViewDirection::Aft,
    ViewDirection::Starboard,
];

/// The ordered list of facing labels for the HP bars.
pub const FACING_LABELS: [&str; 4] = ["Fore", "Port", "Aft", "Starboard"];

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Shields console UI.
#[derive(Component)]
pub struct ShieldsPanel;

/// Marker resource set once the phone shields UI has been spawned.
#[derive(Resource)]
pub struct ShieldsPanelSpawned;

/// Marks an individual focus button; carries the facing index (0-3).
#[derive(Component)]
struct ShieldFocusButton(usize);

/// Marks the Clear-focus button entity.
#[derive(Component)]
struct ClearFocusButton;

/// Marks a shield HP `ProgressBar` root; carries the facing index (0-3).
#[derive(Component)]
struct ShieldHpBar(usize);

/// Marks a shield HP `TextReadout` root; carries the facing index (0-3).
#[derive(Component)]
struct ShieldHpReadout(usize);

// ── Plugin ────────────────────────────────────────────────────────────

pub struct ShieldsPanelPlugin;

impl Plugin for ShieldsPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_shields_ui.run_if(not(resource_exists::<ShieldsPanelSpawned>)),
                toggle_shields_panel_visibility,
                refresh_shields_panel,
                handle_clear_focus_press,
                respawn_shields_on_orientation_change,
            ),
        );
    }
}

// ── Spawn (ConsoleShell) ─────────────────────────────────────────────

fn spawn_shields_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<ShieldsPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Shields {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(ShieldsPanelSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Shields,
        |commands: &mut Commands, primary: Entity| {
            fill_shields_hp_rows(commands, primary);
        },
        |commands: &mut Commands, secondary: Entity| {
            fill_shields_focus_controls(commands, secondary);
        },
        &assets,
    );

    commands.entity(shell.root).insert((ShieldsPanel, Visibility::Hidden));
}

// ── Fill helpers ─────────────────────────────────────────────────────

/// Build the four HP rows (Fore/Port/Aft/Stbd) into the primary container,
/// with an inline focus button per row and a Clear button below all rows.
fn fill_shields_hp_rows(commands: &mut Commands, container: Entity) {
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
            Text::new("SHIELDS"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::srgb(0.4, 0.8, 1.0)),
        ))
        .id();
    commands.entity(col).add_child(title);

    for (idx, (label, dir)) in FACING_LABELS.iter().zip(FOCUS_DIRECTIONS.iter()).enumerate() {
        let row = commands
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                width: Val::Percent(90.0),
                column_gap: Val::Px(8.0),
                ..default()
            })
            .id();

        let name_label = commands
            .spawn((
                Text::new(*label),
                TextFont { font_size: 12.0, ..default() },
                TextColor(Color::srgb(0.6, 0.8, 1.0)),
                Node { width: Val::Px(60.0), ..default() },
            ))
            .id();
        commands.entity(row).add_child(name_label);

        let bar = ProgressBar::spawn(
            commands,
            Vec2::new(160.0, 24.0),
            ProgressBarVariant::Segmented,
            hp_bar_visuals(),
            Some(SegmentCount(10)),
        );
        commands.entity(bar).insert(ShieldHpBar(idx));
        commands.entity(row).add_child(bar);

        let readout = TextReadout::spawn(commands, "", hp_readout_visuals());
        commands
            .entity(readout)
            .insert((
                ShieldHpReadout(idx),
                Node {
                    min_width: Val::Px(55.0),
                    justify_content: JustifyContent::FlexEnd,
                    ..default()
                },
            ));
        commands.entity(row).add_child(readout);

        let dir_clone = dir.clone();
        let focus_btn = spawn_gui_button(
            commands,
            ButtonSize::Rect { width: 52.0, height: 24.0 },
            focus_button_visuals(),
            None,
        );
        commands
            .entity(focus_btn)
            .insert(ShieldFocusButton(idx))
            .with_children(|b| {
                b.spawn((
                    Text::new("Focus"),
                    TextFont { font_size: 10.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.8, 1.0)),
                ));
            })
            .observe(move |_trigger: On<ButtonPressed>,
                           mut buttons: Query<(&mut WidgetState, &ShieldFocusButton)>,
                           mut outbound: MessageWriter<OutboundClientMessage>| {
                for (mut ws, btn) in buttons.iter_mut() {
                    ws.active = btn.0 == idx;
                }
                outbound.write(OutboundClientMessage(ClientMessage::SetShieldFocus {
                    facing: Some(dir_clone.clone()),
                }));
            });
        commands.entity(row).add_child(focus_btn);

        commands.entity(col).add_child(row);
    }

    // Clear button centred below all four rows.
    let clear_visuals = clear_button_visuals();
    let clear_btn = commands
        .spawn((
            ClearFocusButton,
            Button,
            Node {
                width: Val::Px(96.0),
                height: Val::Px(32.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                margin: UiRect::top(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(crate::gui::resolve_visual(&clear_visuals, false, false, false, false).color),
            clear_visuals,
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new("Clear"),
                TextFont { font_size: 12.0, ..default() },
                TextColor(Color::srgb(1.0, 0.92, 0.78)),
            ));
        })
        .id();
    commands.entity(col).add_child(clear_btn);
}

/// Secondary slot — focus controls are now inline in the primary HP rows.
fn fill_shields_focus_controls(_commands: &mut Commands, _container: Entity) {}

fn refresh_shields_panel(
    sim: Res<ClientSimState>,
    mut hp_bars: Query<(&ShieldHpBar, &mut ProgressValue)>,
    mut hp_readouts: Query<(&ShieldHpReadout, &mut ReadoutValue)>,
) {
    if !sim.is_changed() {
        return;
    }
    let facings = &sim.shield_facings;
    if facings.is_empty() {
        return;
    }

    for (bar_comp, mut pv) in hp_bars.iter_mut() {
        if let Some(f) = facings.get(bar_comp.0) {
            pv.0 = hp_fraction(f.hp, f.max_hp);
        } else if let Some(f) = facings.iter().find(|f| f.label == FACING_LABELS[bar_comp.0]) {
            pv.0 = hp_fraction(f.hp, f.max_hp);
        }
    }

    for (readout_comp, mut rv) in hp_readouts.iter_mut() {
        if let Some(f) = facings.get(readout_comp.0) {
            rv.0 = hp_readout_text(f.hp, f.max_hp);
        } else if let Some(f) = facings.iter().find(|f| f.label == FACING_LABELS[readout_comp.0]) {
            rv.0 = hp_readout_text(f.hp, f.max_hp);
        }
    }
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_shields_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<ShieldsPanel>>,
) {
    let visible = shields_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Sends `SetShieldFocus { facing: None }` when the Clear button is pressed,
/// and deactivates all focus buttons.
fn handle_clear_focus_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<ClearFocusButton>)>,
    mut focus_buttons: Query<&mut WidgetState, With<ShieldFocusButton>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction == Interaction::Pressed {
            for mut ws in focus_buttons.iter_mut() {
                ws.active = false;
            }
            outbound.write(OutboundClientMessage(ClientMessage::SetShieldFocus {
                facing: None,
            }));
        }
    }
}

// ── Orientation respawn ──────────────────────────────────────────────

fn respawn_shields_on_orientation_change(
    orientation: Option<Res<DeviceOrientation>>,
    panel: Query<Entity, With<ShieldsPanel>>,
    mut commands: Commands,
) {
    let Some(orientation) = orientation else { return };
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<ShieldsPanelSpawned>();
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::gui::resolve_visual;
    use crate::messages::{Console, GamePhase, GameState, Player, ServerMessage, ShipClientConfig};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;

    // ── Helpers ──────────────────────────────────────────────────────────

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

    fn in_progress_shields_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Shields])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole {
        ActiveConsole(None)
    }
    fn tab(c: Console) -> ActiveConsole {
        ActiveConsole(Some(c))
    }

    // ── shields_panel_visible ────────────────────────────────────────────

    #[test]
    fn shields_panel_hidden_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_visible_in_progress_holding_shields() {
        let lobby = in_progress_shields_lobby("tok");
        let active = no_tab();
        assert!(shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_hidden_when_player_does_not_hold_shields() {
        let lobby = in_progress_shields_lobby("tok");
        let active = no_tab();
        assert!(!shields_panel_visible(&lobby, "other", &active));
    }

    #[test]
    fn shields_panel_visible_when_active_console_is_shields_multi_console() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Shields, Console::Tactical])],
        )));
        let active = tab(Console::Shields);
        assert!(shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_hidden_when_active_console_is_other_multi_console() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Shields, Console::Tactical])],
        )));
        let active = tab(Console::Tactical);
        assert!(!shields_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn shields_panel_hidden_when_no_active_console_and_holding_multiple() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Shields, Console::Tactical])],
        )));
        let active = no_tab();
        assert!(!shields_panel_visible(&lobby, "tok", &active));
    }

    // ── hp_fraction ───────────────────────────────────────────────────────

    #[test]
    fn hp_fraction_full_hp_returns_one() {
        assert_eq!(hp_fraction(100, 100), 1.0);
    }

    #[test]
    fn hp_fraction_zero_hp_returns_zero() {
        assert_eq!(hp_fraction(0, 100), 0.0);
    }

    #[test]
    fn hp_fraction_half_hp_returns_half() {
        assert!((hp_fraction(50, 100) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn hp_fraction_zero_max_returns_zero() {
        assert_eq!(hp_fraction(10, 0), 0.0);
    }

    #[test]
    fn hp_fraction_clamped_above_one() {
        assert_eq!(hp_fraction(150, 100), 1.0);
    }

    #[test]
    fn hp_fraction_negative_hp_clamped_to_zero() {
        assert_eq!(hp_fraction(-5, 100), 0.0);
    }

    // ── hp_readout_text ───────────────────────────────────────────────────

    #[test]
    fn hp_readout_text_formats_correctly() {
        assert_eq!(hp_readout_text(75, 100), "75/100");
    }

    #[test]
    fn hp_readout_text_zero_hp() {
        assert_eq!(hp_readout_text(0, 100), "0/100");
    }

    #[test]
    fn hp_readout_text_full_hp() {
        assert_eq!(hp_readout_text(100, 100), "100/100");
    }

    // ── direction_to_label ────────────────────────────────────────────────

    #[test]
    fn direction_to_label_fore() {
        assert_eq!(direction_to_label(&ViewDirection::Fore), "Fore");
    }

    #[test]
    fn direction_to_label_port() {
        assert_eq!(direction_to_label(&ViewDirection::Port), "Port");
    }

    #[test]
    fn direction_to_label_aft() {
        assert_eq!(direction_to_label(&ViewDirection::Aft), "Aft");
    }

    #[test]
    fn direction_to_label_starboard() {
        assert_eq!(direction_to_label(&ViewDirection::Starboard), "Starboard");
    }

    // ── focus_button_visuals: five distinct states ────────────────────────

    #[test]
    fn focus_button_visuals_has_distinct_five_states() {
        let v = focus_button_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let hover    = resolve_visual(&v, false, false, false, true ).color;
        let active   = resolve_visual(&v, false, false, true,  false).color;
        let press    = resolve_visual(&v, false, true,  false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    // ── clear_button_visuals ──────────────────────────────────────────────

    #[test]
    fn clear_button_visuals_idle_differs_from_press() {
        let v = clear_button_visuals();
        let idle  = resolve_visual(&v, false, false, false, false).color;
        let press = resolve_visual(&v, false, true,  false, false).color;
        assert_ne!(idle, press);
    }

    // ── hp_bar_visuals: active (focused) differs from idle ────────────────

    #[test]
    fn hp_bar_visuals_active_differs_from_idle() {
        let v = hp_bar_visuals();
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true,  false).color;
        assert_ne!(idle, active, "focused HP bar should look different from unfocused");
    }

    #[test]
    fn hp_bar_visuals_disabled_differs_from_idle() {
        let v = hp_bar_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, disabled, "offline HP bar should look different from online");
    }

    // ── hp_readout_visuals: active differs from idle ──────────────────────

    #[test]
    fn hp_readout_visuals_active_differs_from_idle() {
        let v = hp_readout_visuals();
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true,  false).color;
        assert_ne!(idle, active);
    }

    // ── FOCUS_DIRECTIONS order ────────────────────────────────────────────

    #[test]
    fn focus_directions_order_matches_facing_labels() {
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[0]), FACING_LABELS[0]);
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[1]), FACING_LABELS[1]);
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[2]), FACING_LABELS[2]);
        assert_eq!(direction_to_label(&FOCUS_DIRECTIONS[3]), FACING_LABELS[3]);
    }

    #[test]
    fn focus_directions_has_four_entries() {
        assert_eq!(FOCUS_DIRECTIONS.len(), 4);
    }

    // ── SetShieldFocus message construction ───────────────────────────────

    #[test]
    fn set_shield_focus_fore_message() {
        let facing = Some(ViewDirection::Fore);
        let msg = ClientMessage::SetShieldFocus { facing: facing.clone() };
        assert_eq!(msg, ClientMessage::SetShieldFocus { facing });
    }

    #[test]
    fn set_shield_focus_none_clears_focus() {
        let msg = ClientMessage::SetShieldFocus { facing: None };
        assert_eq!(msg, ClientMessage::SetShieldFocus { facing: None });
    }

    #[test]
    fn set_shield_focus_all_directions() {
        for dir in &FOCUS_DIRECTIONS {
            let msg = ClientMessage::SetShieldFocus {
                facing: Some(dir.clone()),
            };
            assert!(matches!(msg, ClientMessage::SetShieldFocus { facing: Some(_) }));
        }
    }

    // ── ReadoutValue helpers ──────────────────────────────────────────────

    #[test]
    fn readout_value_holds_hp_string() {
        let rv = ReadoutValue(hp_readout_text(80, 100));
        assert_eq!(rv.0, "80/100");
    }

    #[test]
    fn readout_value_default_is_empty() {
        let rv = ReadoutValue(String::new());
        assert_eq!(rv.0, "");
    }
}
