//! Client-side Comms Panel plugin.
//!
//! Renders a two-panel inbox layout:
//! - **Primary** (left/top) — contacts strip + message inbox list.
//! - **Secondary** (right/bottom) — chat panel + objectives footer.
//!
//! The plugin drives `ClientCommsState` from inbound `ServerMessage`s and
//! wires response buttons back to `ClientMessage` outbound events via
//! the central `detect_comms_clicks` system.
//!
//! **Not unit-tested** — visual / Bevy layer. See `client_comms.rs` for the
//! pure, tested logic that backs this plugin.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::OutboundClientMessage;
use crate::client_comms::ClientCommsState;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::messages::{ClientMessage, Console, GamePhase};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the comms panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Communications`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Communications`.
pub fn comms_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Comms) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Comms,
        None => count == 1,
    }
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Comms console UI; shown only when the local
/// player holds `Console::Communications` and the phase is InProgress.
#[derive(Component)]
pub struct CommsPanel;

/// Marks the horizontal scrollable contacts strip (top of primary).
#[derive(Component)]
pub struct CommsContactsStrip;

/// Marks the vertical scrollable inbox list container.
#[derive(Component)]
pub struct CommsInboxList;

/// Marks the chat panel container (body + responses).
#[derive(Component)]
pub struct CommsChatPanel;

/// Marks the objectives footer strip at the bottom of secondary.
#[derive(Component)]
pub struct CommsObjectivesFooter;

/// Marks the "Clear All" button.
#[derive(Component)]
pub struct CommsClearButton;

/// Marks the "Back" button in the chat panel.
#[derive(Component)]
pub struct CommsBackButton;

/// Marks the "On Screen" button in the chat panel.
/// Carries the message ID to display on the viewscreen.
#[derive(Component)]
pub struct CommsOnScreenButton {
    pub message_id: String,
}

/// Marker + data for a contact pill: carries the target entity UUID.
#[derive(Component)]
pub struct CommsContactPill {
    pub target_uuid: String,
}

/// Marker + data for an inbox row: carries the thread ID.
#[derive(Component)]
pub struct CommsMessageRow {
    pub thread_id: String,
}

/// Marker + data for a response button: carries (response_index, message_id).
#[derive(Component)]
pub struct CommsResponseButton {
    pub response_index: usize,
    pub message_id: String,
}

// ── Plugin ────────────────────────────────────────────────────────────

/// Marker resource set once the comms UI has been spawned.
#[derive(Resource)]
pub struct CommsPanelSpawned;

pub struct CommsPanelPlugin;

impl Plugin for CommsPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientCommsState>()
            .add_systems(Update, (
                spawn_comms_ui.run_if(not(resource_exists::<CommsPanelSpawned>)),
                toggle_comms_panel_visibility,
                respawn_comms_on_orientation_change,
                refresh_all_comms_ui,
                detect_comms_clicks,
            ));
    }
}

// ── Spawn (ConsoleShell) ──────────────────────────────────────────────

fn spawn_comms_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<CommsPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Comms {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(CommsPanelSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Comms,
        |commands: &mut Commands, primary: Entity| {
            // Primary: vertical column — contacts strip + inbox list
            let col = commands
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                })
                .id();
            commands.entity(primary).add_child(col);

            // Contacts strip (horizontal, scrollable)
            let contacts_strip = commands
                .spawn((
                    CommsContactsStrip,
                    Node {
                        flex_direction: FlexDirection::Row,
                        width: Val::Percent(100.0),
                        height: Val::Px(40.0),
                        overflow: Overflow::scroll(),
                        column_gap: Val::Px(6.0),
                        padding: UiRect::horizontal(Val::Px(6.0)),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                ))
                .id();
            commands.entity(col).add_child(contacts_strip);

            // Inbox header row: label + Clear All button
            let inbox_header = commands
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        width: Val::Percent(100.0),
                        height: Val::Px(26.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::horizontal(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb_u8(25, 25, 35)),
                ))
                .with_child((
                    Text::new("INBOX"),
                    TextFont { font_size: 11.0, ..default() },
                    TextColor(Color::srgb(0.5, 0.5, 0.6)),
                ))
                .with_child((
                    CommsClearButton,
                    Button,
                    Node {
                        padding: UiRect::horizontal(Val::Px(8.0)),
                        height: Val::Px(22.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb_u8(60, 40, 40)),
                    Text::new("Clear All"),
                    TextFont { font_size: 11.0, ..default() },
                    TextColor(Color::srgb(0.9, 0.5, 0.5)),
                ))
                .id();
            commands.entity(col).add_child(inbox_header);

            // Inbox list (vertical, scrollable, flex_grow)
            let inbox_list = commands
                .spawn((
                    CommsInboxList,
                    Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Percent(100.0),
                        flex_grow: 1.0,
                        overflow: Overflow::scroll(),
                        row_gap: Val::Px(2.0),
                        ..default()
                    },
                ))
                .id();
            commands.entity(col).add_child(inbox_list);
        },
        |commands: &mut Commands, secondary: Entity| {
            // Secondary: vertical column — chat panel + objectives footer
            let col = commands
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(100.0),
                    min_width: Val::Px(0.0),
                    height: Val::Percent(100.0),
                    ..default()
                })
                .id();
            commands.entity(secondary).add_child(col);

            // Chat panel (flex_grow, scrollable)
            let chat_panel = commands
                .spawn((
                    CommsChatPanel,
                    Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Percent(100.0),
                        min_width: Val::Px(0.0),
                        flex_grow: 1.0,
                        overflow: Overflow { x: OverflowAxis::Clip, y: OverflowAxis::Scroll },
                        row_gap: Val::Px(4.0),
                        padding: UiRect::all(Val::Px(4.0)),
                        ..default()
                    },
                ))
                .id();
            commands.entity(col).add_child(chat_panel);

            // Objectives footer (fixed height)
            let objectives_footer = commands
                .spawn((
                    CommsObjectivesFooter,
                    Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Percent(100.0),
                        max_height: Val::Px(120.0),
                        overflow: Overflow { x: OverflowAxis::Clip, y: OverflowAxis::Scroll },
                        row_gap: Val::Px(4.0),
                        padding: UiRect::all(Val::Px(6.0)),
                        ..default()
                    },
                ))
                .id();
            commands.entity(col).add_child(objectives_footer);
        },
        &assets,
    );

    commands.entity(shell.root).insert((CommsPanel, Visibility::Hidden));
}

// ── Orientation respawn ──────────────────────────────────────────────

fn respawn_comms_on_orientation_change(
    orientation: Option<Res<DeviceOrientation>>,
    panel: Query<Entity, With<CommsPanel>>,
    mut commands: Commands,
) {
    let Some(orientation) = orientation else { return };
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<CommsPanelSpawned>();
}

// ── Visibility ───────────────────────────────────────────────────────

fn toggle_comms_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<CommsPanel>>,
) {
    let visible = comms_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

// ── Central click router ─────────────────────────────────────────────

/// Detects `Interaction::Pressed` on comms interactive entities and
/// writes the corresponding `OutboundClientMessage`.
fn detect_comms_clicks(
    contacts: Query<(&Interaction, &CommsContactPill), Changed<Interaction>>,
    messages: Query<(&Interaction, &CommsMessageRow), Changed<Interaction>>,
    responses: Query<(&Interaction, &CommsResponseButton), Changed<Interaction>>,
    clears: Query<&Interaction, (Changed<Interaction>, With<CommsClearButton>)>,
    backs: Query<&Interaction, (Changed<Interaction>, With<CommsBackButton>)>,
    on_screens: Query<(&Interaction, &CommsOnScreenButton), Changed<Interaction>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
    mut state: ResMut<ClientCommsState>,
) {
    for (interaction, pill) in contacts.iter() {
        if *interaction == Interaction::Pressed {
            if !state.can_hail(&pill.target_uuid) {
                continue;
            }
            outbound.write(OutboundClientMessage(
                ClientMessage::Hail { target_uuid: pill.target_uuid.clone() },
            ));
        }
    }
    for (interaction, row) in messages.iter() {
        if *interaction == Interaction::Pressed {
            state.select_thread(&row.thread_id);
            outbound.write(OutboundClientMessage(
                ClientMessage::SelectCommsMessage { message_id: row.thread_id.clone() },
            ));
        }
    }
    for (interaction, btn) in responses.iter() {
        if *interaction == Interaction::Pressed {
            // Skip if the sender of this message is out of range.
            let in_range = state.messages.iter()
                .find(|m| m.id == btn.message_id)
                .map(|m| m.sender_in_range)
                .unwrap_or(true);
            if !in_range {
                continue;
            }
            outbound.write(OutboundClientMessage(
                ClientMessage::RespondToMessage {
                    message_id: btn.message_id.clone(),
                    response_index: btn.response_index,
                },
            ));
        }
    }
    for interaction in clears.iter() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(ClientMessage::ClearComms));
        }
    }
    for interaction in backs.iter() {
        if *interaction == Interaction::Pressed {
            state.clear_selection();
        }
    }
    for (interaction, btn) in on_screens.iter() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(
                ClientMessage::ShowOnScreen { message_id: btn.message_id.clone() },
            ));
        }
    }
}

// ── Refresh (single combined system) ─────────────────────────────────

/// Helper: despawn all children of a container.
fn clear_children(children: &Children, commands: &mut Commands) {
    for child in children.iter() {
        commands.entity(child).despawn();
    }
}

/// Helper: spawn an empty-state label into a container.
fn spawn_empty_label(container: Entity, text: &str, commands: &mut Commands) {
    commands.entity(container).with_children(|p| {
        p.spawn((
            Text::new(text.to_string()),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.5, 0.5, 0.5)),
        ));
    });
}

/// Refreshes all four comms UI sections when state is dirty, then marks clean.
fn refresh_all_comms_ui(
    mut state: ResMut<ClientCommsState>,
    contacts_strip_q: Query<Entity, With<CommsContactsStrip>>,
    inbox_list_q: Query<Entity, With<CommsInboxList>>,
    chat_panel_q: Query<Entity, With<CommsChatPanel>>,
    objectives_footer_q: Query<Entity, With<CommsObjectivesFooter>>,
    children: Query<&Children>,
    mut commands: Commands,
) {
    if !state.is_dirty() { return; }

    // ── Contacts strip ──────────────────────────────────────────────────
    // Out-of-range contacts are hidden entirely (spec): no greyed pill.
    // The inbox row + chat panel still surface stale-but-known messages
    // from those contacts so the operator can read them — they're just no
    // longer hailable. We render an "All contacts out of range" fallback
    // when every contact has dropped out so the strip doesn't read empty.
    if let Ok(container) = contacts_strip_q.single() {
        if let Ok(existing) = children.get(container) {
            clear_children(existing, &mut commands);
        }
        let visible_contacts: Vec<_> = state.contacts.iter().filter(|c| c.in_range).collect();
        if state.contacts.is_empty() {
            spawn_empty_label(container, "No contacts", &mut commands);
        } else if visible_contacts.is_empty() {
            spawn_empty_label(container, "All contacts out of range", &mut commands);
        } else {
            for contact in visible_contacts {
                let pill_label = if contact.is_urgent {
                    format!("! {}", contact.name)
                } else {
                    contact.name.clone()
                };
                let (pill_bg, pill_fg) = if contact.is_urgent {
                    (Color::srgb_u8(70, 55, 15), Color::srgb(1.0, 0.8, 0.2))
                } else {
                    (Color::srgb_u8(60, 60, 80), Color::srgb(0.8, 0.8, 1.0))
                };
                commands.entity(container).with_children(|p| {
                    p.spawn((
                        CommsContactPill { target_uuid: contact.uuid.clone() },
                        Button,
                        Node {
                            flex_direction: FlexDirection::Row,
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            height: Val::Px(30.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(pill_bg),
                        Text::new(pill_label),
                        TextFont { font_size: 13.0, ..default() },
                        TextColor(pill_fg),
                    ));
                });
            }
        }
    }

    // ── Inbox list ──────────────────────────────────────────────────────
    if let Ok(container) = inbox_list_q.single() {
        if let Ok(existing) = children.get(container) {
            clear_children(existing, &mut commands);
        }
        let threads = state.sorted_threads();
        if threads.is_empty() {
            spawn_empty_label(container, "Inbox empty", &mut commands);
        } else {
            for thread in &threads {
                let sender_text = if thread.latest_orphaned {
                    format!("{} (disconnected)", thread.sender_name)
                } else {
                    thread.sender_name.clone()
                };
                let subject_text = if thread.subject.len() > 32 {
                    format!("{}...", &thread.subject[..32])
                } else {
                    thread.subject.clone()
                };
                let row_label = if thread.latest_out_of_range {
                    format!("{} \u{2014} {} [OUT OF RANGE]", sender_text, subject_text)
                } else if thread.any_urgent {
                    format!("! {} \u{2014} {}", sender_text, subject_text)
                } else {
                    format!("{} \u{2014} {}", sender_text, subject_text)
                };
                let (bg, fg) = if thread.latest_out_of_range {
                    // Alert red to match viewscreen_border.rs COLOR_ALERT_RED.
                    (Color::srgb_u8(35, 25, 25), Color::srgb(1.0, 0.2, 0.267))
                } else if thread.any_urgent {
                    // Amber tint for urgent unread messages.
                    (Color::srgb_u8(50, 42, 20), Color::srgb(1.0, 0.8, 0.2))
                } else if !thread.any_unread {
                    (Color::srgb_u8(30, 30, 40), Color::srgb(0.4, 0.4, 0.5))
                } else {
                    (Color::srgb_u8(40, 40, 55), Color::srgb(0.9, 0.9, 1.0))
                };
                let tid = thread.thread_id.clone();
                commands.entity(container).with_children(|p| {
                    p.spawn((
                        CommsMessageRow { thread_id: tid },
                        Button,
                        Node {
                            flex_direction: FlexDirection::Row,
                            padding: UiRect::all(Val::Px(6.0)),
                            width: Val::Percent(100.0),
                            min_height: Val::Px(32.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(bg),
                        Text::new(row_label),
                        TextFont { font_size: 12.0, ..default() },
                        TextColor(fg),
                    ));
                });
            }
        }
    }

    // ── Chat panel ──────────────────────────────────────────────────────
    if let Ok(container) = chat_panel_q.single() {
        if let Ok(existing) = children.get(container) {
            clear_children(existing, &mut commands);
        }
        if let Some(ref tid) = state.selected_thread_id.clone() {
            let thread_msgs: Vec<_> = state.thread_messages(tid);
            let latest_msg_id = thread_msgs.last().map(|m| m.id.clone()).unwrap_or_default();
            let active_msg = state.active_message_for_thread(tid).cloned();

            // Back button
            commands.entity(container).with_children(|p| {
                p.spawn((
                    CommsBackButton,
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(4.0)),
                        margin: UiRect::bottom(Val::Px(4.0)),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb_u8(50, 50, 70)),
                    Text::new("\u{2190} Back"),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgb(0.7, 0.7, 0.9)),
                ));
            });

            // On Screen button (uses the latest message in the thread)
            commands.entity(container).with_children(|p| {
                p.spawn((
                    CommsOnScreenButton { message_id: latest_msg_id },
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(4.0)),
                        margin: UiRect::bottom(Val::Px(6.0)),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgb_u8(40, 70, 60)),
                    Text::new("\u{25a6} On Screen"),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgb(0.6, 1.0, 0.8)),
                ));
            });

            // Chat messages: render each message and any player reply inline.
            for msg in &thread_msgs {
                // Contact message bubble: sender name + body
                commands.entity(container).with_children(|p| {
                    p.spawn(Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Percent(100.0),
                        min_width: Val::Px(0.0),
                        padding: UiRect::all(Val::Px(6.0)),
                        row_gap: Val::Px(3.0),
                        margin: UiRect::bottom(Val::Px(2.0)),
                        ..default()
                    })
                    .with_children(|bubble| {
                        bubble.spawn((
                            Text::new(msg.sender_name.clone()),
                            TextFont { font_size: 13.0, ..default() },
                            TextColor(Color::srgb(0.8, 0.7, 1.0)),
                        ));
                        // Body text wrapped in a row so flex_grow assigns a
                        // definite pixel width, which lets Bevy wrap the text.
                        bubble.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            width: Val::Percent(100.0),
                            min_width: Val::Px(0.0),
                            ..default()
                        }).with_children(|row| {
                            row.spawn((
                                Text::new(msg.body.clone()),
                                TextFont { font_size: 13.0, ..default() },
                                TextColor(Color::srgb(0.8, 0.8, 0.9)),
                                Node {
                                    flex_grow: 1.0,
                                    flex_shrink: 1.0,
                                    width: Val::Px(0.0),
                                    min_width: Val::Px(0.0),
                                    ..default()
                                },
                            ));
                        });
                    });
                });

                // Player reply bubble shown inline after the message it responded to.
                if let Some(selected_idx) = msg.selected_response {
                    let reply_text = msg.responses
                        .get(selected_idx)
                        .map(|s| format!("You: {}", s))
                        .unwrap_or_else(|| "You: [response]".to_string());
                    commands.entity(container).with_children(|p| {
                        p.spawn((
                            Node {
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::FlexEnd,
                                width: Val::Percent(100.0),
                                padding: UiRect::axes(Val::Px(6.0), Val::Px(4.0)),
                                margin: UiRect::bottom(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb_u8(30, 50, 35)),
                        ))
                        .with_children(|row| {
                            row.spawn((
                                Text::new(reply_text),
                                TextFont { font_size: 12.0, ..default() },
                                TextColor(Color::srgb(0.5, 0.9, 0.6)),
                            ));
                        });
                    });
                }
            }

            // Response area at the bottom of the thread.
            if let Some(active) = active_msg {
                for (idx, response) in active.responses.iter().enumerate() {
                    let mid = active.id.clone();
                    commands.entity(container).with_children(|p| {
                        p.spawn((
                            CommsResponseButton { response_index: idx, message_id: mid },
                            Button,
                            Node {
                                padding: UiRect::all(Val::Px(6.0)),
                                margin: UiRect::top(Val::Px(2.0)),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgb_u8(50, 60, 80)),
                            Text::new(response.clone()),
                            TextFont { font_size: 13.0, ..default() },
                            TextColor(Color::srgb(0.7, 0.9, 1.0)),
                        ));
                    });
                }
            } else if let Some(latest) = thread_msgs.last() {
                // No active message — show status based on the latest message.
                if latest.is_orphaned {
                    commands.entity(container).with_children(|p| {
                        p.spawn((
                            Text::new("Transmission ended \u{2014} source no longer available."),
                            TextFont { font_size: 12.0, ..default() },
                            TextColor(Color::srgb(0.6, 0.4, 0.4)),
                        ));
                    });
                } else if !latest.sender_in_range {
                    commands.entity(container).with_children(|p| {
                        p.spawn((
                            Text::new("OUT OF RANGE \u{2014} cannot respond."),
                            TextFont { font_size: 12.0, ..default() },
                            TextColor(Color::srgb(1.0, 0.2, 0.267)),
                        ));
                    });
                }
                // Info-only message (no responses) — nothing extra to show.
            }
        } else {
            spawn_empty_label(container, "Select a message", &mut commands);
        }
    }

    // ── Objectives footer ───────────────────────────────────────────────
    if let Ok(container) = objectives_footer_q.single() {
        if let Ok(existing) = children.get(container) {
            clear_children(existing, &mut commands);
        }
        if state.objectives.is_empty() {
            spawn_empty_label(container, "No active objectives", &mut commands);
        } else {
            for obj in state.objectives.iter() {
                let status_str = match obj.status {
                    crate::messages::ObjectiveStatus::Active => "ACTIVE",
                    crate::messages::ObjectiveStatus::Completed => "DONE",
                    crate::messages::ObjectiveStatus::Failed => "FAILED",
                };
                let status_color = match obj.status {
                    crate::messages::ObjectiveStatus::Active => Color::srgb(0.3, 0.8, 0.3),
                    crate::messages::ObjectiveStatus::Completed => Color::srgb(0.5, 0.5, 0.5),
                    crate::messages::ObjectiveStatus::Failed => Color::srgb(0.8, 0.3, 0.3),
                };
                commands.entity(container).with_children(|p| {
                    p.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        width: Val::Percent(100.0),
                        column_gap: Val::Px(6.0),
                        align_items: AlignItems::FlexStart,
                        ..default()
                    }).with_children(|row| {
                        row.spawn((
                            Text::new(format!("[{}]", status_str)),
                            TextFont { font_size: 11.0, ..default() },
                            TextColor(status_color),
                        ));
                        row.spawn((
                            Text::new(obj.text.clone()),
                            TextFont { font_size: 11.0, ..default() },
                            TextColor(Color::srgb(0.7, 0.7, 0.8)),
                            Node {
                                flex_grow: 1.0,
                                ..default()
                            },
                        ));
                    });
                });
            }
        }
    }

    state.mark_clean();
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{LobbyState, ActiveConsole};
    use crate::messages::GamePhase;

    fn lobby_in_progress() -> LobbyState {
        let mut s = LobbyState::default();
        s.phase = GamePhase::InProgress;
        s
    }

    #[test]
    fn comms_panel_not_visible_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = ActiveConsole::default();
        assert!(!comms_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn comms_panel_not_visible_when_player_does_not_hold_comms() {
        let lobby = lobby_in_progress();
        let active = ActiveConsole::default();
        assert!(!comms_panel_visible(&lobby, "tok", &active));
    }
}
