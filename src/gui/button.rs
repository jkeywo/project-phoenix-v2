//! `GuiButton` widget — rect/square variants, observer events.

use bevy::prelude::*;
use bevy::picking::events::Press;

use super::{StateVisuals, WidgetState, Disabled};

// ── Size variant ──────────────────────────────────────────────────────────────

/// Controls the layout dimensions of a `GuiButton`.
#[derive(Clone, Debug, PartialEq)]
pub enum ButtonSize {
    /// Rectangular button with explicit width and height.
    Rect { width: f32, height: f32 },
    /// Square button: width == height.
    Square(f32),
}

impl ButtonSize {
    fn to_node(&self) -> Node {
        match self {
            ButtonSize::Rect { width, height } => Node {
                width: Val::Px(*width),
                height: Val::Px(*height),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            ButtonSize::Square(size) => Node {
                width: Val::Px(*size),
                height: Val::Px(*size),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        }
    }
}

// ── Observer events ───────────────────────────────────────────────────────────

/// Fired on a button entity exactly once per press transition — when
/// `Interaction` first becomes `Pressed`.  The payload carries the button
/// entity so callers can also observe this as a global event.
#[derive(EntityEvent, Clone, Debug)]
pub struct ButtonPressed(pub Entity);

/// Fired when `WidgetState.active` transitions to `true`.
#[derive(EntityEvent, Clone, Debug)]
pub struct WidgetActivated(pub Entity);

/// Fired when `WidgetState.active` transitions to `false`.
#[derive(EntityEvent, Clone, Debug)]
pub struct WidgetDeactivated(pub Entity);

// ── Marker ────────────────────────────────────────────────────────────────────

/// Marker component on every entity spawned by `spawn_gui_button`.
#[derive(Component, Default)]
pub struct GuiButtonMarker;

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Spawn a `GuiButton` entity.
///
/// Observers for `ButtonPressed`, `WidgetActivated`, and `WidgetDeactivated`
/// are **attached at spawn time** (stub no-ops).  Callers receive the spawned
/// `Entity` and can add additional `.observe()` handlers immediately:
///
/// ```rust,ignore
/// let entity = spawn_gui_button(&mut commands, size, visuals);
/// commands.entity(entity).observe(|trigger: On<ButtonPressed>| { /* … */ });
/// ```
pub fn spawn_gui_button(
    commands: &mut Commands,
    size: ButtonSize,
    state_visuals: StateVisuals,
) -> Entity {
    let initial_color = state_visuals.idle.color;
    let initial_image = state_visuals.idle.image.clone();
    let mut builder = commands.spawn((
        GuiButtonMarker,
        Button,
        size.to_node(),
        BackgroundColor(initial_color),
        state_visuals,
        WidgetState::default(),
        Interaction::default(),
    ));
    // `resolve_visuals_system` only updates an existing `ImageNode`; insert
    // one at spawn time so the per-state image actually renders.  Buttons
    // configured with color-only `Visual`s (no image handle) skip this and
    // render via `BackgroundColor` alone.
    if let Some(image) = initial_image {
        builder.insert(ImageNode::new(image));
    }
    // Attach stub observers at spawn so the observer infrastructure is wired.
    // Callers layer additional `.observe()` calls on the returned entity.
    // `on_gui_button_press` replaces the old `detect_button_press` polling
    // system: it fires `ButtonPressed` via the picking pipeline (`PreUpdate`)
    // rather than polling `Changed<Interaction>` in `Update`, which missed
    // fast taps where press+release completed within a single frame.
    builder
        .observe(on_gui_button_press)
        .observe(|_: On<ButtonPressed>| {})
        .observe(|_: On<WidgetActivated>| {})
        .observe(|_: On<WidgetDeactivated>| {});
    builder.id()
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Stamps `Pickable::IGNORE` onto every direct child of a `GuiButtonMarker`
/// entity that doesn't already have a `Pickable` component.
///
/// This prevents decorative children — text labels, icon nodes — from sitting
/// in front of the button in the pick order and silently absorbing pointer
/// events before they reach the button entity.  The system only runs when a
/// button's `Children` list changes, so the overhead is negligible.
fn auto_ignore_button_children(
    buttons: Query<&Children, (With<GuiButtonMarker>, Changed<Children>)>,
    without_pickable: Query<Entity, Without<Pickable>>,
    mut commands: Commands,
) {
    for kids in buttons.iter() {
        for child in kids.iter() {
            if without_pickable.contains(child) {
                commands.entity(child).insert(Pickable::IGNORE);
            }
        }
    }
}

/// Observer attached at spawn time to every `GuiButtonMarker` entity.
/// Fires `ButtonPressed` when the picking system reports a pointer press
/// directly on the button entity.
///
/// Using `Pointer<Press>` (fired in `PreUpdate` via the picking pipeline)
/// instead of polling `Changed<Interaction>` in `Update` eliminates the
/// fast-tap race where touchstart + touchend both land in the same Bevy frame
/// and `Interaction` has already returned to `None` by the time any `Update`
/// system runs.
///
/// `Pickable::IGNORE` on child nodes (stamped by `auto_ignore_button_children`)
/// ensures that text labels do not intercept the pointer before it reaches
/// this entity.
fn on_gui_button_press(
    trigger: On<Pointer<Press>>,
    buttons: Query<Has<Disabled>, With<GuiButtonMarker>>,
    mut commands: Commands,
) {
    let entity = trigger.entity;
    // Guard: only fire on button entities that are not disabled.
    let Ok(is_disabled) = buttons.get(entity) else { return };
    if !is_disabled {
        commands.entity(entity).trigger(ButtonPressed);
    }
}

/// Fires `WidgetActivated` / `WidgetDeactivated` when `WidgetState.active`
/// changes on a `GuiButtonMarker` entity.
fn detect_widget_state_change(
    query: Query<
        (Entity, &WidgetState),
        (Changed<WidgetState>, With<GuiButtonMarker>),
    >,
    mut commands: Commands,
) {
    for (entity, state) in query.iter() {
        if state.active {
            commands.entity(entity).trigger(WidgetActivated);
        } else {
            commands.entity(entity).trigger(WidgetDeactivated);
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the button widget.  Registered automatically by `GuiPlugin`.
pub struct GuiButtonPlugin;

impl Plugin for GuiButtonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (detect_widget_state_change, auto_ignore_button_children));
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_size_rect_produces_correct_node() {
        let size = ButtonSize::Rect { width: 120.0, height: 40.0 };
        let node = size.to_node();
        assert_eq!(node.width, Val::Px(120.0));
        assert_eq!(node.height, Val::Px(40.0));
    }

    #[test]
    fn button_size_square_produces_equal_dimensions() {
        let size = ButtonSize::Square(48.0);
        let node = size.to_node();
        assert_eq!(node.width, Val::Px(48.0));
        assert_eq!(node.height, Val::Px(48.0));
    }
}
