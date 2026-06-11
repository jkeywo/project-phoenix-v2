//! Client-side help system.
//!
//! Each bridge panel can spawn a "?" help button and a click-to-dismiss
//! help overlay describing its controls.
//!
//! The hideable-element registry that previously lived here (used by the
//! complexity preset system) was ported to pure JS in issue #461; see
//! gui/hideable-elements.js. Only the help system (issue #462) remains.

use bevy::prelude::*;

// ── Help System ────────────────────────────────────────────────────

/// Identifies which panel a help button or overlay belongs to.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelpPanel {
    CaptainChair,
    Helm,
    Tactical,
    Repair,
    Power,
    Shields,
    Sensors,
    Navigation,
    Comms,
}

/// Marker for a help "?" button.
#[derive(Component)]
pub struct HelpButton(pub HelpPanel);

/// Marker for a help overlay (semi-transparent background, white text, click-to-dismiss).
#[derive(Component)]
pub struct HelpOverlay(pub HelpPanel);

/// Returns the help sections for the given panel.
pub fn help_sections(panel: HelpPanel) -> &'static [(&'static str, &'static str)] {
    match panel {
        HelpPanel::CaptainChair => &[
            ("Red Alert", "Toggle ship-wide alert status."),
            ("View Selector", "Switch viewscreen camera angle."),
        ],
        HelpPanel::Helm => &[
            ("Thrust", "Drag up to accelerate, down to reverse."),
            ("Steering", "Drag left/right to yaw the ship."),
            ("On Screen", "Push your radar to the viewscreen."),
            ("Impulse Drive", "10× speed burst. Cancelled by damage."),
        ],
        HelpPanel::Tactical => &[
            ("Target Lock", "Select a target within range and arc."),
            ("Phasers", "Fire at locked target. Auto mode fires when in arc."),
            ("Torpedoes", "Launch homing torpedoes from loaded tubes."),
        ],
        HelpPanel::Repair => &[
            ("Hull Status", "Aggregate hull integrity across all systems."),
            ("Repair Teams", "Dispatch teams to damaged consoles."),
            ("Target Console", "Select which console to repair."),
        ],
        HelpPanel::Power => &[
            ("Power Allocation", "Distribute 6 base power points."),
            ("Battery Reserve", "Up to 2 emergency points. Exhaustion locks all."),
            ("Level Effects", "Higher levels improve system performance."),
        ],
        HelpPanel::Shields => &[
            ("Shield Facings", "Four quadrants: Fore, Aft, Port, Starboard."),
            ("Focus", "Direct capacity to one facing."),
        ],
        HelpPanel::Sensors => &[
            ("Long-Range Scan", "Extended-range radar overlay."),
            ("Target Hand-off", "Suggest targets to Tactical."),
        ],
        HelpPanel::Navigation => &[
            ("System Chart", "Push the navigation chart to the viewscreen."),
            ("Cancel Impulse", "Abort an active impulse drive charge."),
        ],
        HelpPanel::Comms => &[
            ("Contacts", "List of hailable ships and stations."),
            ("Messages", "Inbox of incoming transmissions."),
            ("Objectives", "Current mission objectives."),
        ],
    }
}

/// Spawn a "?" help button into the given parent node.
pub fn spawn_help_button(parent: &mut ChildSpawnerCommands, panel: HelpPanel, font_size: f32) {
    parent.spawn((
        HelpButton(panel),
        Button,
        Node {
            width: Val::Px(28.0),
            height: Val::Px(28.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            margin: UiRect::left(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.2, 0.2, 0.3, 0.5)),
    )).with_children(|btn| {
        btn.spawn((
            Text::new("?"),
            TextFont { font_size, ..default() },
            TextColor(Color::srgb(0.7, 0.8, 1.0)),
        ));
    });
}

/// Spawn a help overlay into the given parent node.  Initially hidden.
pub fn spawn_help_overlay(parent: &mut ChildSpawnerCommands, panel: HelpPanel) {
    let sections = help_sections(panel);
    parent.spawn((
        HelpOverlay(panel),
        Button,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(16.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(Color::srgba(0.05, 0.05, 0.1, 0.92)),
        Visibility::Hidden,
    )).with_children(|ov| {
        ov.spawn((
            Text::new("HELP"),
            TextFont { font_size: 18.0, ..default() },
            TextColor(Color::srgb(0.7, 0.8, 1.0)),
        ));
        for (label, desc) in sections {
            ov.spawn((
                Text::new(format!("{}\n{}", label, desc)),
                TextFont { font_size: 12.0, ..default() },
                TextColor(Color::srgb(0.8, 0.85, 0.9)),
            ));
        }
    });
}

/// Spawn a help overlay as a top-level (window-root) entity that covers
/// the entire viewport when visible. Uses [`GlobalZIndex`] to render
/// above the bezel and tab strip — acts as a click-to-dismiss modal
/// that dims everything underneath.
///
/// Matched to its [`HelpButton`] by [`HelpPanel`] discriminant in
/// [`handle_help_button_press`], so it doesn't need a hierarchical
/// relationship to the triggering button.
pub fn spawn_help_overlay_root(commands: &mut Commands, panel: HelpPanel) {
    let sections = help_sections(panel);
    commands
        .spawn((
            HelpOverlay(panel),
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(24.0)),
                row_gap: Val::Px(8.0),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.05, 0.88)),
            GlobalZIndex(1000),
            Visibility::Hidden,
        ))
        .with_children(|ov| {
            ov.spawn((
                Text::new("HELP — tap to dismiss"),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.8, 1.0)),
            ));
            for (label, desc) in sections {
                ov.spawn((
                    Text::new(format!("{}\n{}", label, desc)),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.8, 0.85, 0.9)),
                ));
            }
        });
}

/// When a help button is pressed, show the matching overlay.
pub fn handle_help_button_press(
    buttons: Query<(&HelpButton, &Interaction), Changed<Interaction>>,
    mut overlays: Query<(&HelpOverlay, &mut Visibility)>,
) {
    for (btn, interaction) in buttons.iter() {
        if *interaction != Interaction::Pressed { continue; }
        for (overlay, mut vis) in overlays.iter_mut() {
            if overlay.0 == btn.0 {
                *vis = Visibility::Visible;
            }
        }
    }
}

/// When a help overlay is pressed (clicked anywhere), dismiss it.
pub fn handle_help_overlay_dismiss(
    buttons: Query<(&HelpOverlay, &Interaction), Changed<Interaction>>,
    mut overlays: Query<(&HelpOverlay, &mut Visibility)>,
) {
    for (pressed, interaction) in buttons.iter() {
        if *interaction != Interaction::Pressed { continue; }
        for (overlay, mut vis) in overlays.iter_mut() {
            if overlay.0 == pressed.0 {
                *vis = Visibility::Hidden;
            }
        }
    }
}
