//! Client-side Power Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns all Power console UI: per-console power allocation rows
//! (Helm/Weapons/Sensors), battery `ProgressBar` (continuous), power level
//! `TextReadout` widgets, lock state indicator, and overflow allocation
//! controls (hidden in Low complexity).
//!
//! No per-button marker-component query systems remain.  All button callbacks
//! are wired via observers at spawn time.  `ProgressValue` drives the battery
//! bar; `ReadoutValue` drives the power level readouts.
//!
//! Extracted from `client/app.rs` as part of the "Client split" series.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::{ClientSet, OutboundClientMessage, HideableElement};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    spawn_gui_button, ButtonPressed, ButtonSize, Disabled, ProgressBar, ProgressBarVariant,
    ProgressValue, ReadoutValue, StateVisuals, TextReadout,
};
use crate::messages::{Console, GamePhase};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the power panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Power`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Power`.
pub fn power_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Power) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Power,
        None => count == 1,
    }
}

// ── Pure helpers ──────────────────────────────────────────────────────

/// Build `StateVisuals` for an increment button.
///
/// Active (enabled) = green; disabled = dim.
pub fn inc_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.50, 0.30), // idle
        Color::srgb(0.14, 0.60, 0.35), // hover
        Color::srgb(0.10, 0.65, 0.35), // active
        Color::srgb(0.18, 0.70, 0.40), // press
        Color::srgb(0.06, 0.06, 0.10), // disabled
    )
}

/// Build `StateVisuals` for a decrement button.
///
/// Active (enabled) = red; disabled = dim.
pub fn dec_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.50, 0.20, 0.10), // idle
        Color::srgb(0.60, 0.24, 0.12), // hover
        Color::srgb(0.65, 0.20, 0.10), // active
        Color::srgb(0.75, 0.25, 0.12), // press
        Color::srgb(0.06, 0.06, 0.10), // disabled
    )
}

/// Build `StateVisuals` for the battery `ProgressBar`.
pub fn battery_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.60, 0.80), // idle
        Color::srgb(0.10, 0.60, 0.80), // hover (unused — non-interactive)
        Color::srgb(0.10, 0.60, 0.80), // active
        Color::srgb(0.10, 0.60, 0.80), // press  (unused)
        Color::srgb(0.15, 0.15, 0.25), // disabled (locked)
    )
}

/// Build `StateVisuals` for a power level `TextReadout`.
pub fn level_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.9, 0.9, 1.0), // idle
        Color::srgb(0.9, 0.9, 1.0), // hover
        Color::srgb(0.3, 1.0, 0.8), // active (locked state)
        Color::srgb(0.9, 0.9, 1.0), // press
        Color::srgb(0.4, 0.4, 0.5), // disabled
    )
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the power console UI; shown only when the local
/// player holds Power and the phase is InProgress.
#[derive(Component)]
pub struct PowerPanel;

/// Marks one power allocation row container.
#[derive(Component)]
struct PowerRow;

/// Marks the `TextReadout` root for the power level display.
/// Carries the console it represents for refresh matching.
#[derive(Component)]
struct PowerLevelReadout(Console);

/// Marks the increment `GuiButton` entity. Carries the target console.
#[derive(Component)]
struct PowerIncButton(Console);

/// Marks the decrement `GuiButton` entity. Carries the target console.
#[derive(Component)]
struct PowerDecButton(Console);

/// Marks the root of the battery `ProgressBar`.
#[derive(Component)]
struct BatteryBar;

/// Marks the battery percentage `TextReadout`.
#[derive(Component)]
struct BatteryReadout;

// ── Plugin ───────────────────────────────────────────────────────────

/// Marker resource set once the power UI has been spawned.
#[derive(Resource)]
pub struct PowerPanelSpawned;

// ── Plugin ───────────────────────────────────────────────────────────

/// Plugin that owns all Power console UI and systems.
pub struct PowerPanelPlugin;

impl Plugin for PowerPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_systems(Update, (
                spawn_power_ui.run_if(not(resource_exists::<PowerPanelSpawned>)),
                toggle_power_panel_visibility.in_set(ClientSet::ConsoleUpdate),
                refresh_power_panel,
                respawn_power_on_orientation_change,
            ));
    }
}

// ── Spawn (ConsoleShell) ─────────────────────────────────────────────

fn spawn_power_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<PowerPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Power {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(PowerPanelSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Power,
        |commands: &mut Commands, primary: Entity| {
            fill_power_primary(commands, primary);
        },
        |_commands: &mut Commands, _secondary: Entity| {
            // Power console uses a single column; secondary slot left empty.
        },
        &assets,
    );

    commands.entity(shell.root).insert((PowerPanel, Visibility::Hidden));
}

/// Primary slot: title + 3 power rows + overflow + battery.
fn fill_power_primary(commands: &mut Commands, container: Entity) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(12.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    let title = commands
        .spawn((
            Text::new("Power Console"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::srgb(0.3, 1.0, 0.8)),
        ))
        .id();
    commands.entity(col).add_child(title);

    for (console, label) in [
        (Console::Helm, "Helm"),
        (Console::Tactical, "Weapons"),
        (Console::Sensors, "Sensors"),
    ] {
        let row = spawn_power_row(commands, console, label);
        commands.entity(col).add_child(row);
    }

    // Overflow row
    let overflow_row = commands
        .spawn((
            HideableElement("power_overflow_controls".into()),
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(4.0)),
                ..default()
            },
        ))
        .id();
    commands.entity(overflow_row).with_children(|r| {
        r.spawn((
            Text::new("Overflow (pts 7-8): Manual"),
            TextFont { font_size: 13.0, ..default() },
            TextColor(Color::srgb(0.6, 0.7, 0.5)),
        ));
    });
    commands.entity(col).add_child(overflow_row);

    // Battery section
    let battery_section = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(4.0),
            width: Val::Percent(80.0),
            max_width: Val::Px(300.0),
            ..default()
        })
        .id();

    let bar = ProgressBar::spawn(
        commands,
        Vec2::new(280.0, 16.0),
        ProgressBarVariant::Continuous,
        battery_visuals(),
        None,
    );
    commands.entity(bar).insert(BatteryBar);
    commands.entity(battery_section).add_child(bar);

    let battery_dim = Color::srgb(0.5, 0.8, 1.0);
    let bat_readout_visuals = StateVisuals::from_colors(
        battery_dim, battery_dim, battery_dim, battery_dim,
        Color::srgb(0.3, 0.3, 0.4),
    );
    let bat_readout = TextReadout::spawn(commands, "Battery", bat_readout_visuals);
    commands.entity(bat_readout).insert(BatteryReadout);
    commands.entity(battery_section).add_child(bat_readout);

    commands.entity(col).add_child(battery_section);
}

// ── Orientation respawn ──────────────────────────────────────────────

fn respawn_power_on_orientation_change(
    orientation: Option<Res<DeviceOrientation>>,
    panel: Query<Entity, With<PowerPanel>>,
    mut commands: Commands,
) {
    let Some(orientation) = orientation else { return };
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<PowerPanelSpawned>();
}

/// Spawn one power allocation row (console label | dec button | level readout | inc button).
///
/// Returns the row root entity.
fn spawn_power_row(commands: &mut Commands, console: Console, label: &str) -> Entity {
    let row = commands
        .spawn((
            PowerRow,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.08, 0.08, 0.12)),
        ))
        .id();

    // Console name label
    let name_label = commands
        .spawn((
            Text::new(label),
            TextFont { font_size: 16.0, ..default() },
            TextColor(Color::srgb(0.7, 0.9, 1.0)),
            Node { width: Val::Px(80.0), ..default() },
        ))
        .id();
    commands.entity(row).add_child(name_label);

    // Decrement GuiButton
    let dec_console = console.clone();
    let dec_btn = spawn_gui_button(
        commands,
        ButtonSize::Square(36.0),
        dec_visuals(),
    );
    commands.entity(dec_btn)
        .insert((PowerDecButton(dec_console.clone()), Disabled))
        .with_children(|btn| {
            btn.spawn((
                Text::new("-"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.9, 0.9, 1.0)),
            ));
        })
        .observe(move |_trigger: On<ButtonPressed>,
                       sim: Res<ClientSimState>,
                       ship_view: Res<ShipView>,
                       mut outbound: MessageWriter<OutboundClientMessage>| {
            let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
            if crate::client_sim::can_decrease_power(&ship_view.power_levels, &dec_console, locked) {
                outbound.write(OutboundClientMessage(
                    crate::client_sim::decrease_power_message(dec_console.clone()),
                ));
            }
        });
    commands.entity(row).add_child(dec_btn);

    // Power level TextReadout
    let level_readout = TextReadout::spawn(commands, "", level_readout_visuals());
    commands.entity(level_readout)
        .insert((
            PowerLevelReadout(console.clone()),
            Node {
                min_width: Val::Px(36.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ));
    commands.entity(row).add_child(level_readout);

    // Increment GuiButton
    let inc_console = console.clone();
    let inc_btn = spawn_gui_button(
        commands,
        ButtonSize::Square(36.0),
        inc_visuals(),
    );
    commands.entity(inc_btn)
        .insert((PowerIncButton(inc_console.clone()), Disabled))
        .with_children(|btn| {
            btn.spawn((
                Text::new("+"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.9, 0.9, 1.0)),
            ));
        })
        .observe(move |_trigger: On<ButtonPressed>,
                       sim: Res<ClientSimState>,
                       ship_view: Res<ShipView>,
                       mut outbound: MessageWriter<OutboundClientMessage>| {
            let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
            if crate::client_sim::can_increase_power(&ship_view.power_levels, &inc_console, locked) {
                outbound.write(OutboundClientMessage(
                    crate::client_sim::increase_power_message(inc_console.clone()),
                ));
            }
        });
    commands.entity(row).add_child(inc_btn);

    row
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_power_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<PowerPanel>>,
) {
    let visible = power_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Refresh the power panel each frame when `ClientSimState` or `ShipView` changes:
/// - Update `ProgressValue` on the battery bar.
/// - Update `ReadoutValue` on each power level readout.
/// - Insert/remove `Disabled` on each inc/dec button.
fn refresh_power_panel(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut battery_bar: Query<&mut ProgressValue, With<BatteryBar>>,
    mut battery_readout: Query<&mut ReadoutValue, (With<BatteryReadout>, Without<PowerLevelReadout>)>,
    mut level_readouts: Query<(Entity, &mut ReadoutValue, &PowerLevelReadout), Without<BatteryReadout>>,
    inc_buttons: Query<(Entity, &PowerIncButton, Has<Disabled>)>,
    dec_buttons: Query<(Entity, &PowerDecButton, Has<Disabled>)>,
) {
    let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
    let battery_raw = crate::client_sim::battery_percentage(&sim.power_state_payload);
    // battery_percentage returns a raw value in [0, capacity]; capacity is typically 100.
    let battery_fraction = (battery_raw / 100.0).clamp(0.0, 1.0);

    // Update battery bar ProgressValue
    for mut pv in battery_bar.iter_mut() {
        pv.0 = battery_fraction;
    }

    // Update battery percentage ReadoutValue
    for mut rv in battery_readout.iter_mut() {
        rv.0 = format!("{:.0}%", battery_raw);
    }

    // Update power level readouts
    for (_entity, mut rv, level_comp) in level_readouts.iter_mut() {
        let lvl = match level_comp.0 {
            Console::Helm     => ship_view.power_levels.0,
            Console::Tactical => ship_view.power_levels.1,
            Console::Sensors  => ship_view.power_levels.2,
            _ => 0,
        };
        rv.0 = format!("{}", lvl);
    }

    // Sync Disabled on increment buttons
    for (entity, inc, currently_disabled) in inc_buttons.iter() {
        let can_inc = crate::client_sim::can_increase_power(
            &ship_view.power_levels, &inc.0, locked,
        );
        if can_inc && currently_disabled {
            commands.entity(entity).remove::<Disabled>();
        } else if !can_inc && !currently_disabled {
            commands.entity(entity).insert(Disabled);
        }
    }

    // Sync Disabled on decrement buttons
    for (entity, dec, currently_disabled) in dec_buttons.iter() {
        let can_dec = crate::client_sim::can_decrease_power(
            &ship_view.power_levels, &dec.0, locked,
        );
        if can_dec && currently_disabled {
            commands.entity(entity).remove::<Disabled>();
        } else if !can_dec && !currently_disabled {
            commands.entity(entity).insert(Disabled);
        }
    }
}

// ── Unit tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::gui::resolve_visual;
    use crate::messages::{Console, GamePhase, Player, ShipClientConfig};
    use std::collections::HashMap;

    fn lobby_with_power_player(token: &str, phase: GamePhase) -> LobbyState {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase,
                players: vec![Player {
                    token: token.into(),
                    name: "Powertrain".into(),
                    consoles: vec![Console::Power],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        });
        lobby
    }

    // ── power_panel_visible ──────────────────────────────────────────

    #[test]
    fn power_panel_hidden_in_lobby_phase() {
        let lobby = lobby_with_power_player("tok", GamePhase::Lobby);
        let active = ActiveConsole::default();
        assert!(!power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_visible_in_progress_holding_power() {
        let lobby = lobby_with_power_player("tok", GamePhase::InProgress);
        let active = ActiveConsole::default();
        assert!(power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_hidden_when_player_does_not_hold_power() {
        let lobby = lobby_with_power_player("tok", GamePhase::InProgress);
        let active = ActiveConsole::default();
        assert!(!power_panel_visible(&lobby, "other", &active));
    }

    #[test]
    fn power_panel_visible_when_active_console_is_power_multi_console() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Power, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        });
        let active = ActiveConsole(Some(Console::Power));
        assert!(power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_hidden_when_active_console_is_other_multi_console() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Power, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        });
        let active = ActiveConsole(Some(Console::Tactical));
        assert!(!power_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn power_panel_hidden_when_no_active_console_and_holding_multiple() {
        let mut lobby = LobbyState::default();
        use crate::messages::GameState;
        lobby.apply(&crate::messages::ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![Player {
                    token: "tok".into(),
                    name: "Multi".into(),
                    consoles: vec![Console::Power, Console::Tactical],
                    connected: true,
                }],
                complexity: HashMap::new(),
                world: None,
            },
            ship_stations: crate::stations_config::ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        });
        let active = ActiveConsole::default(); // None → auto → count != 1
        assert!(!power_panel_visible(&lobby, "tok", &active));
    }

    // ── inc_visuals / dec_visuals: five distinct states ───────────────

    #[test]
    fn inc_visuals_has_distinct_five_states() {
        let v = inc_visuals();
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

    #[test]
    fn dec_visuals_has_distinct_five_states() {
        let v = dec_visuals();
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

    #[test]
    fn battery_visuals_disabled_differs_from_idle() {
        let v = battery_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, disabled);
    }

    // ── ReadoutValue helpers ─────────────────────────────────────────

    #[test]
    fn readout_value_formats_power_level() {
        let rv = ReadoutValue(format!("{}", 3u8));
        assert_eq!(rv.0, "3");
    }

    #[test]
    fn battery_readout_formats_percentage() {
        let battery_raw = 75.0_f32;
        let text = format!("{:.0}%", battery_raw);
        assert_eq!(text, "75%");
    }

    #[test]
    fn battery_fraction_clamped_to_unit_interval() {
        // battery_percentage returns raw value; clamping to [0,1] via /100
        let raw = 120.0_f32;
        let fraction = (raw / 100.0).clamp(0.0, 1.0);
        assert_eq!(fraction, 1.0);

        let raw_neg = -10.0_f32;
        let fraction_neg = (raw_neg / 100.0).clamp(0.0, 1.0);
        assert_eq!(fraction_neg, 0.0);
    }

    // ── can_increase / can_decrease integration with is_power_locked ─

    #[test]
    fn increase_blocked_when_locked() {
        use crate::client_sim::{can_increase_power, is_power_locked};
        let payload = Some((2u8, 2u8, 2u8, 50.0_f32, true));
        let locked = is_power_locked(&payload);
        assert!(!can_increase_power(&(2, 2, 2), &Console::Helm, locked));
    }

    #[test]
    fn decrease_blocked_when_locked() {
        use crate::client_sim::{can_decrease_power, is_power_locked};
        let payload = Some((2u8, 2u8, 2u8, 50.0_f32, true));
        let locked = is_power_locked(&payload);
        assert!(!can_decrease_power(&(2, 2, 2), &Console::Helm, locked));
    }

    #[test]
    fn increase_allowed_when_not_locked_and_under_cap() {
        use crate::client_sim::{can_increase_power, is_power_locked};
        let payload = Some((2u8, 2u8, 2u8, 50.0_f32, false));
        let locked = is_power_locked(&payload);
        assert!(can_increase_power(&(2, 2, 2), &Console::Helm, locked));
    }

    #[test]
    fn decrease_allowed_when_not_locked_and_above_min() {
        use crate::client_sim::{can_decrease_power, is_power_locked};
        let payload = Some((3u8, 2u8, 2u8, 50.0_f32, false));
        let locked = is_power_locked(&payload);
        assert!(can_decrease_power(&(3, 2, 2), &Console::Helm, locked));
    }

    // ── increase_power_message / decrease_power_message ─────────────

    #[test]
    fn increase_power_message_produces_correct_variant() {
        use crate::client_sim::increase_power_message;
        use crate::messages::ClientMessage;
        let msg = increase_power_message(Console::Helm);
        assert_eq!(msg, ClientMessage::IncreasePower { console: Console::Helm });
    }

    #[test]
    fn decrease_power_message_produces_correct_variant() {
        use crate::client_sim::decrease_power_message;
        use crate::messages::ClientMessage;
        let msg = decrease_power_message(Console::Sensors);
        assert_eq!(msg, ClientMessage::DecreasePower { console: Console::Sensors });
    }

    #[test]
    fn increase_power_message_tactical() {
        use crate::client_sim::increase_power_message;
        use crate::messages::ClientMessage;
        let msg = increase_power_message(Console::Tactical);
        assert_eq!(msg, ClientMessage::IncreasePower { console: Console::Tactical });
    }

    #[test]
    fn decrease_power_message_tactical() {
        use crate::client_sim::decrease_power_message;
        use crate::messages::ClientMessage;
        let msg = decrease_power_message(Console::Tactical);
        assert_eq!(msg, ClientMessage::DecreasePower { console: Console::Tactical });
    }

    #[test]
    fn increase_power_message_sensors() {
        use crate::client_sim::increase_power_message;
        use crate::messages::ClientMessage;
        let msg = increase_power_message(Console::Sensors);
        assert_eq!(msg, ClientMessage::IncreasePower { console: Console::Sensors });
    }
}
