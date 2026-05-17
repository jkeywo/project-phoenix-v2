//! Client-side Shields Panel plugin.
//!
//! Owns the Shields console UI: 4-quadrant HP bars, focus-facing mechanic,
//! and real-time shield status updates.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::messages::{ClientMessage, Console, GamePhase, ViewDirection};

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Shields console UI.
#[derive(Component)]
pub struct ShieldsPanel;

/// Marks a shield focus selector button. Carries the `ViewDirection` it targets,
/// or `None` to clear focus.
#[derive(Component)]
pub struct ShieldFocusButton(pub Option<ViewDirection>);

/// Marks a single shield facing HP bar container; carries the facing label.
#[derive(Component)]
pub struct ShieldFacingBar(pub String);

/// Marks the fill node whose width reflects HP fraction; carries the facing label.
#[derive(Component)]
pub struct ShieldFacingHP(pub String);

/// Marks the label text node inside a facing bar.
#[derive(Component)]
pub struct ShieldFacingLabel;

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
                    handle_shield_focus_button_press,
                ),
            );
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_shields_ui(mut commands: Commands) {
    let facings = ["Fore", "Port", "Aft", "Starboard"];
    let dirs = [
        Some(ViewDirection::Fore),
        Some(ViewDirection::Port),
        Some(ViewDirection::Aft),
        Some(ViewDirection::Starboard),
        None,
    ];

    commands.spawn((
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
    .with_children(|panel| {
        panel.spawn((
            Text::new("SHIELDS"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::srgb(0.4, 0.8, 1.0)),
        ));

        // Four facing HP bars
        for label in &facings {
            panel.spawn((
                ShieldFacingBar(label.to_string()),
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    width: Val::Percent(80.0),
                    height: Val::Px(32.0),
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.10, 0.12, 0.16, 1.0)),
            ))
            .with_children(|row| {
                row.spawn((
                    ShieldFacingLabel,
                    Text::new(*label),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.8, 1.0)),
                    Node { width: Val::Px(60.0), ..default() },
                ));
                row.spawn((
                    ShieldFacingHP(label.to_string()),
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.2, 0.4, 0.8)),
                ));
                row.spawn((
                    Text::new("100/100"),
                    TextFont { font_size: 10.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.8, 1.0)),
                    Node { width: Val::Px(70.0), ..default() },
                ));
            });
        }

        // Focus buttons row
        panel.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            padding: UiRect::top(Val::Px(8.0)),
            ..default()
        })
        .with_children(|row| {
            for dir in &dirs {
                let label = match dir {
                    Some(ViewDirection::Fore) => "Fore",
                    Some(ViewDirection::Port) => "Port",
                    Some(ViewDirection::Aft) => "Aft",
                    Some(ViewDirection::Starboard) => "Stbd",
                    None => "Clear",
                };
                row.spawn((
                    ShieldFocusButton(dir.clone()),
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.15, 0.18, 0.25)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new(label),
                        TextFont { font_size: 11.0, ..default() },
                        TextColor(Color::srgb(0.6, 0.8, 1.0)),
                    ));
                });
            }
        });
    });
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_shields_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<ShieldsPanel>>,
) {
    let view = LobbyView::new(&lobby, &token.0);
    let holds = lobby.phase == GamePhase::InProgress
        && view.my_consoles().contains(&Console::Shields);
    let my_consoles_count = view.my_consoles().len();
    let tab_active = match &active.0 {
        Some(c) => *c == Console::Shields,
        None => my_consoles_count == 1,
    };
    let visible = holds && tab_active;
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Refresh the Shields panel: HP bars, focus indicators, and arc colors.
fn refresh_shields_panel(
    sim: Res<ClientSimState>,
    mut bars: Query<(&ShieldFacingBar, &mut BackgroundColor), Without<ShieldFocusButton>>,
    mut hp_nodes: Query<(&ShieldFacingHP, &mut Node)>,
    mut hp_texts: Query<(&ShieldFacingHP, &mut Text)>,
    mut focus_btns: Query<(&ShieldFocusButton, &mut BackgroundColor), Without<ShieldFacingBar>>,
) {
    if !sim.is_changed() {
        return;
    }
    let facings = &sim.shield_facings;
    if facings.is_empty() {
        return;
    }

    for (bar, mut bg) in bars.iter_mut() {
        if let Some(f) = facings.iter().find(|f| f.label == bar.0) {
            if f.online {
                bg.0 = if f.is_focused {
                    Color::srgb(0.2, 0.8, 0.4)
                } else {
                    Color::srgb(0.2, 0.4, 0.8)
                };
            } else {
                bg.0 = Color::srgb(0.3, 0.1, 0.1);
            }
        }
    }

    for (hp, mut node) in hp_nodes.iter_mut() {
        if let Some(f) = facings.iter().find(|f| f.label == hp.0) {
            let pct = if f.max_hp > 0 {
                (f.hp as f32 / f.max_hp as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            node.width = Val::Percent(pct * 100.0);
        }
    }

    for (hp, mut text) in hp_texts.iter_mut() {
        if let Some(f) = facings.iter().find(|f| f.label == hp.0) {
            let focus_marker = if f.is_focused { " [F]" } else { "" };
            **text = format!("{}/{} {}", f.hp, f.max_hp, focus_marker);
        }
    }

    for (btn, mut bg) in focus_btns.iter_mut() {
        let is_active = match btn.0 {
            Some(ref d) => facings.iter().any(|f| {
                let facing_dir = match f.label.as_str() {
                    "Fore" | "Fore (Focused)" => ViewDirection::Fore,
                    "Port" | "Port (Focused)" => ViewDirection::Port,
                    "Aft" | "Aft (Focused)" => ViewDirection::Aft,
                    "Starboard" | "Starboard (Focused)" => ViewDirection::Starboard,
                    _ => return false,
                };
                &facing_dir == d && f.is_focused
            }),
            None => facings.iter().all(|f| !f.is_focused),
        };
        bg.0 = if is_active {
            Color::srgb(0.25, 0.40, 0.55)
        } else {
            Color::srgb(0.15, 0.18, 0.25)
        };
    }
}

fn handle_shield_focus_button_press(
    interactions: Query<(&Interaction, &ShieldFocusButton), Changed<Interaction>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, btn) in interactions.iter() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(
                ClientMessage::SetShieldFocus { facing: btn.0.clone() },
            ));
        }
    }
}
