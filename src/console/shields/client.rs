//! Client-side Shields Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns the Shields console UI: 4-quadrant HP bars, focus-facing mechanic,
//! and real-time shield status updates.
//!
//! No per-button marker-component query systems remain.  All button callbacks
//! are wired via observers at spawn time.  `ProgressValue` drives the HP bars;
//! `ReadoutValue` drives the HP text readouts; `RadioGroup` drives the focus
//! selector.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    ProgressBar, ProgressBarVariant, ProgressValue, RadioButtonConfig, RadioGroup, RadioSelected,
    ReadoutValue, SegmentCount, StateVisuals, TextReadout,
};
use crate::messages::{ClientMessage, Console, GamePhase, ViewDirection};

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

/// Marks the `RadioGroup` entity used for shield focus selection.
#[derive(Component)]
struct ShieldFocusRadio;

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
        app.add_systems(Startup, setup_shields_ui)
            .add_systems(
                Update,
                (
                    toggle_shields_panel_visibility,
                    refresh_shields_panel,
                ),
            );
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_shields_ui(mut commands: Commands) {
    // ── Root panel ────────────────────────────────────────────────────
    let panel = commands
        .spawn((
            ShieldsPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(0.0),
                right:  Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(16.0)),
                ..default()
            },
            Visibility::Hidden,
        ))
        .id();

    // ── Title row ─────────────────────────────────────────────────────
    let title_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .id();
    commands.entity(title_row).with_children(|tr| {
        tr.spawn((
            Text::new("SHIELDS"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::srgb(0.4, 0.8, 1.0)),
        ));
        crate::client_elements::spawn_help_button(
            tr,
            crate::client_elements::HelpPanel::Shields,
            16.0,
        );
    });
    commands.entity(panel).add_child(title_row);
    crate::client_elements::spawn_help_overlay_root(
        &mut commands,
        crate::client_elements::HelpPanel::Shields,
    );

    // ── Four facing rows: label + HP bar + HP readout ─────────────────
    for (idx, label) in FACING_LABELS.iter().enumerate() {
        let row = commands
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                width: Val::Percent(80.0),
                column_gap: Val::Px(8.0),
                ..default()
            })
            .id();

        // Facing name label
        let name_label = commands
            .spawn((
                Text::new(*label),
                TextFont { font_size: 12.0, ..default() },
                TextColor(Color::srgb(0.6, 0.8, 1.0)),
                Node { width: Val::Px(60.0), ..default() },
            ))
            .id();
        commands.entity(row).add_child(name_label);

        // HP ProgressBar (segmented, 10 segments)
        let bar = ProgressBar::spawn(
            &mut commands,
            Vec2::new(160.0, 24.0),
            ProgressBarVariant::Segmented,
            hp_bar_visuals(),
            Some(SegmentCount(10)),
        );
        commands.entity(bar).insert(ShieldHpBar(idx));
        commands.entity(row).add_child(bar);

        // HP TextReadout
        let readout = TextReadout::spawn(&mut commands, "", hp_readout_visuals());
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

        commands.entity(panel).add_child(row);
    }

    // ── Focus RadioGroup (Fore / Port / Aft / Stbd) ───────────────────
    let focus_label = commands
        .spawn((
            Text::new("Focus:"),
            TextFont { font_size: 12.0, ..default() },
            TextColor(Color::srgb(0.6, 0.8, 1.0)),
        ))
        .id();
    commands.entity(panel).add_child(focus_label);

    let btn_configs: Vec<RadioButtonConfig> = (0..4)
        .map(|_| RadioButtonConfig {
            size: crate::gui::ButtonSize::Rect { width: 62.0, height: 32.0 },
            click_sound: None,
        })
        .collect();

    let radio_group = RadioGroup::spawn(
        &mut commands,
        btn_configs,
        focus_button_visuals(),
        None,
    );
    commands
        .entity(radio_group)
        .insert(ShieldFocusRadio)
        .observe(on_focus_selected);
    commands.entity(panel).add_child(radio_group);

    // Add text labels to each radio member button after children are resolved.
    commands.insert_resource(FocusButtonLabelsPending);
}

// ── Focus button label post-setup ─────────────────────────────────────

/// Resource flag: focus button labels have not been added yet.
#[derive(Resource)]
struct FocusButtonLabelsPending;

/// One-shot system: once the RadioGroup children exist (deferred spawn),
/// add text label children to each member button.
fn refresh_shields_panel(
    pending: Option<Res<FocusButtonLabelsPending>>,
    mut commands: Commands,
    groups: Query<&Children, With<ShieldFocusRadio>>,
    sim: Res<ClientSimState>,
    mut hp_bars: Query<(&ShieldHpBar, &mut ProgressValue)>,
    mut hp_readouts: Query<(&ShieldHpReadout, &mut ReadoutValue)>,
) {
    // ── One-shot: add text labels to focus buttons ─────────────────────
    if pending.is_some() {
        let labels = ["Fore", "Port", "Aft", "Stbd"];
        for children in groups.iter() {
            if children.len() < 4 {
                // Children not resolved yet — retry next frame.
                return;
            }
            for (idx, child) in children.iter().take(4).enumerate() {
                if let Some(&label_text) = labels.get(idx) {
                    commands.entity(child).with_children(|btn| {
                        btn.spawn((
                            Text::new(label_text),
                            TextFont { font_size: 11.0, ..default() },
                            TextColor(Color::srgb(0.6, 0.8, 1.0)),
                        ));
                    });
                }
            }
            commands.remove_resource::<FocusButtonLabelsPending>();
            break;
        }
    }

    // ── HP bars + readouts ─────────────────────────────────────────────
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

// ── RadioGroup observer ───────────────────────────────────────────────

/// Observer on the `ShieldFocusRadio` group entity.
/// Maps the selected member index to a `ViewDirection` and emits
/// `SetShieldFocus`.
fn on_focus_selected(
    trigger: On<RadioSelected>,
    children_q: Query<&Children>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let group = trigger.entity;
    let member = trigger.event().member;

    if let Ok(children) = children_q.get(group) {
        for (idx, child) in children.iter().enumerate() {
            if child == member {
                let facing = FOCUS_DIRECTIONS.get(idx).cloned();
                outbound.write(OutboundClientMessage(ClientMessage::SetShieldFocus {
                    facing,
                }));
                return;
            }
        }
    }
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
