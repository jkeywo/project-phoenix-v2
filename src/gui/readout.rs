//! `TextReadout` widget — labelled text display with idle/active visual states.
//!
//! Game logic writes `ReadoutValue(String)` on the root entity; the widget
//! system propagates the new string to the value text child on the next frame.

use bevy::prelude::*;

use super::{resolve_visual, Disabled, StateVisuals, WidgetState};

// ── Components ────────────────────────────────────────────────────────────────

/// Marker on the root entity of every `TextReadout` widget.
#[derive(Component, Default)]
pub struct TextReadoutMarker;

/// Game logic writes this to update the displayed value string.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ReadoutValue(pub String);

/// Marker on the label text child.
#[derive(Component)]
pub struct TextReadoutLabel;

/// Marker on the value text child.
#[derive(Component)]
pub struct TextReadoutValueNode;

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Namespace struct for the `TextReadout` widget.
pub struct TextReadout;

impl TextReadout {
    /// Spawn a `TextReadout` widget.
    ///
    /// - `label` — static label string displayed to the left of the value.
    /// - `state_visuals` — `idle.color` / `active.color` drive the value text
    ///   colour and root background colour per `WidgetState`.
    ///
    /// Returns the root entity.  The root is a flex row; lay it out inside
    /// any `GuiPanel` or flex container.
    pub fn spawn(
        commands: &mut Commands,
        label: &str,
        state_visuals: StateVisuals,
    ) -> Entity {
        let idle_color = state_visuals.idle.color;

        let root = commands
            .spawn((
                TextReadoutMarker,
                state_visuals,
                WidgetState::default(),
                ReadoutValue(String::new()),
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .id();

        commands.entity(root).with_children(|parent| {
            // Static label
            parent.spawn((
                TextReadoutLabel,
                Text::new(label.to_string()),
                TextColor(Color::srgba(0.7, 0.7, 0.7, 1.0)),
            ));
            // Dynamic value
            parent.spawn((
                TextReadoutValueNode,
                Text::new(String::new()),
                TextColor(idle_color),
            ));
        });

        root
    }
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Propagates `ReadoutValue` changes to the value text child.
fn update_readout_text(
    roots: Query<
        (&ReadoutValue, &Children),
        (With<TextReadoutMarker>, Changed<ReadoutValue>),
    >,
    mut value_nodes: Query<&mut Text, With<TextReadoutValueNode>>,
) {
    for (readout, children) in roots.iter() {
        for child in children.iter() {
            if let Ok(mut text) = value_nodes.get_mut(child) {
                **text = readout.0.clone();
            }
        }
    }
}

/// Drives `BackgroundColor` and value `TextColor` from `StateVisuals` +
/// `WidgetState`.  Panels are not interactive so only idle/active/disabled
/// matter (press and hover treated as idle).
fn drive_readout_visuals(
    roots: Query<
        (&StateVisuals, &WidgetState, Has<Disabled>, &Children),
        (
            With<TextReadoutMarker>,
            Or<(Changed<WidgetState>, Changed<StateVisuals>)>,
        ),
    >,
    mut value_nodes: Query<&mut TextColor, With<TextReadoutValueNode>>,
) {
    for (visuals, state, is_disabled, children) in roots.iter() {
        let visual = resolve_visual(visuals, is_disabled, false, state.active, false);
        let color = visual.color;

        for child in children.iter() {
            if let Ok(mut text_color) = value_nodes.get_mut(child) {
                text_color.0 = color;
            }
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the text readout widget.  Registered automatically by `GuiPlugin`.
pub struct GuiReadoutPlugin;

impl Plugin for GuiReadoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (update_readout_text, drive_readout_visuals));
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_visuals(idle: Color, active: Color) -> StateVisuals {
        StateVisuals::from_colors(
            idle,
            idle,   // hover same as idle
            active,
            idle,   // press same as idle
            Color::srgba(0.2, 0.2, 0.2, 1.0),
        )
    }

    #[test]
    fn readout_value_default_is_empty() {
        let rv = ReadoutValue(String::new());
        assert_eq!(rv.0, "");
    }

    #[test]
    fn readout_value_can_hold_arbitrary_string() {
        let rv = ReadoutValue("HDG 045°".to_string());
        assert_eq!(rv.0, "HDG 045°");
    }

    #[test]
    fn idle_visual_color_differs_from_active() {
        let idle   = Color::srgb(0.5, 0.5, 0.5);
        let active = Color::srgb(1.0, 0.8, 0.2);
        let visuals = make_visuals(idle, active);

        let idle_result   = resolve_visual(&visuals, false, false, false, false);
        let active_result = resolve_visual(&visuals, false, false, true,  false);

        assert_ne!(
            idle_result.color, active_result.color,
            "idle and active colours must differ"
        );
    }

    #[test]
    fn disabled_overrides_active_for_readout() {
        let idle   = Color::srgb(0.5, 0.5, 0.5);
        let active = Color::srgb(1.0, 0.8, 0.2);
        let visuals = make_visuals(idle, active);

        let disabled_result = resolve_visual(&visuals, true, false, true, false);
        let active_result   = resolve_visual(&visuals, false, false, true, false);

        assert_ne!(disabled_result.color, active_result.color);
    }
}
