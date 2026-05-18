//! `GenericJoystick` widget — background + knob images, StateVisuals, 10 Hz resend.
//!
//! The widget uses Bevy pointer drag events attached at spawn time and fires
//! `JoystickMoved { dx, dy }` observer events (entity-targeted) with values
//! normalised to `[-1.0, 1.0]`.  The existing pure math in
//! `console::helm::joystick` is reused for clamping and normalisation.

use bevy::prelude::*;

use crate::console::helm::joystick::clamp_to_circle;
use super::{StateVisuals, WidgetState};

// ── Pure helper ───────────────────────────────────────────────────────────────

/// Normalise a pixel drag offset to the `[-1.0, 1.0]` range expected by
/// `JoystickMoved`, clamping to the pad circle first.
///
/// - Positive `dx` = right.
/// - Positive `dy` = down (screen coordinates).
/// - Zero `pad_radius` returns `(0.0, 0.0)` safely.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn normalize_joystick(pixel_dx: f32, pixel_dy: f32, pad_radius: f32) -> (f32, f32) {
    if pad_radius <= 0.0 {
        return (0.0, 0.0);
    }
    let (cdx, cdy) = clamp_to_circle(pixel_dx, pixel_dy, pad_radius);
    let dx = (cdx / pad_radius).clamp(-1.0, 1.0);
    let dy = (cdy / pad_radius).clamp(-1.0, 1.0);
    (dx, dy)
}

// ── Observer event ────────────────────────────────────────────────────────────

/// Fired on the joystick pad entity while dragged and on release.
///
/// Values are in `[-1.0, 1.0]`.  A `{ dx: 0.0, dy: 0.0 }` event is always
/// sent on release so consumers can zero their outputs.
#[derive(EntityEvent, Clone, Debug, PartialEq)]
pub struct JoystickMoved {
    #[event_target]
    pub entity: Entity,
    pub dx: f32,
    pub dy: f32,
}

// ── Components ────────────────────────────────────────────────────────────────

/// Marker + state for the joystick pad node.
#[derive(Component)]
pub struct GenericJoystickPad {
    /// Effective drag radius in pixels.
    pub pad_radius: f32,
}

/// Marker for the floating knob child node.
#[derive(Component)]
pub struct GenericJoystickKnob {
    /// Half-size of the knob in pixels (used to position from pad centre).
    pub half_size: f32,
}

/// Per-joystick drag state — lives on the pad entity.
#[derive(Component, Default)]
pub struct JoystickDragState {
    pub active: bool,
    /// Current clamped knob position in pixels relative to pad centre.
    pub knob_px_dx: f32,
    pub knob_px_dy: f32,
    /// Last normalised output; resent by the 10 Hz timer.
    pub last_dx: f32,
    pub last_dy: f32,
}

/// Per-joystick 10 Hz resend timer — lives on the pad entity.
#[derive(Component)]
pub struct JoystickResendTimer {
    pub timer: Timer,
}

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Namespace struct for the `GenericJoystick` widget.
pub struct GenericJoystick;

impl GenericJoystick {
    /// Spawn a `GenericJoystick` widget.
    ///
    /// Returns the pad entity.  Callers add additional observers via
    /// `commands.entity(entity).observe(…)`.
    pub fn spawn(
        commands: &mut Commands,
        size: f32,
        bg_image: Handle<Image>,
        knob_image: Handle<Image>,
        state_visuals: StateVisuals,
    ) -> Entity {
        spawn_generic_joystick(commands, size, bg_image, knob_image, state_visuals)
    }
}

fn spawn_generic_joystick(
    commands: &mut Commands,
    size: f32,
    bg_image: Handle<Image>,
    knob_image: Handle<Image>,
    state_visuals: StateVisuals,
) -> Entity {
    let radius = size / 2.0;
    let knob_half = size * 0.12; // knob radius ≈ 12% of pad diameter

    let pad = commands
        .spawn((
            GenericJoystickPad { pad_radius: radius },
            Button,
            Node {
                width: Val::Px(size),
                height: Val::Px(size),
                position_type: PositionType::Relative,
                ..default()
            },
            ImageNode::new(bg_image),
            BackgroundColor(state_visuals.idle.color),
            state_visuals,
            WidgetState::default(),
            Interaction::default(),
            JoystickDragState::default(),
            JoystickResendTimer {
                timer: Timer::from_seconds(0.1, TimerMode::Repeating),
            },
        ))
        .observe(|_: On<JoystickMoved>| {})
        .id();

    // Knob child
    commands.entity(pad).with_children(|parent| {
        parent.spawn((
            GenericJoystickKnob { half_size: knob_half },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(radius - knob_half),
                top: Val::Px(radius - knob_half),
                width: Val::Px(knob_half * 2.0),
                height: Val::Px(knob_half * 2.0),
                ..default()
            },
            ImageNode::new(knob_image),
        ));
    });

    // Attach pointer observers for drag handling
    commands
        .entity(pad)
        .observe(on_joystick_drag_start)
        .observe(on_joystick_drag)
        .observe(on_joystick_drag_end)
        .observe(on_joystick_cancel);

    pad
}

// ── Pointer observers ─────────────────────────────────────────────────────────

fn on_joystick_drag_start(
    trigger: On<Pointer<DragStart>>,
    mut pads: Query<&mut JoystickDragState, With<GenericJoystickPad>>,
) {
    let entity = trigger.event().entity;
    if let Ok(mut state) = pads.get_mut(entity) {
        state.active = true;
    }
}

fn on_joystick_drag(
    trigger: On<Pointer<Drag>>,
    mut pads: Query<(&GenericJoystickPad, &mut JoystickDragState)>,
    mut commands: Commands,
) {
    let entity = trigger.event().entity;
    let Ok((pad_comp, mut state)) = pads.get_mut(entity) else {
        return;
    };
    if !state.active {
        return;
    }

    let new_px_dx = state.knob_px_dx + trigger.event().delta.x;
    let new_px_dy = state.knob_px_dy + trigger.event().delta.y;
    let (cdx, cdy) = clamp_to_circle(new_px_dx, new_px_dy, pad_comp.pad_radius);
    state.knob_px_dx = cdx;
    state.knob_px_dy = cdy;

    let (dx, dy) = normalize_joystick(cdx, cdy, pad_comp.pad_radius);
    state.last_dx = dx;
    state.last_dy = dy;

    commands.entity(entity).trigger(|e| JoystickMoved { entity: e, dx, dy });
}

fn on_joystick_drag_end(
    trigger: On<Pointer<DragEnd>>,
    mut pads: Query<&mut JoystickDragState, With<GenericJoystickPad>>,
    mut commands: Commands,
) {
    let entity = trigger.event().entity;
    if let Ok(mut state) = pads.get_mut(entity) {
        release_joystick(&mut state, entity, &mut commands);
    }
}

fn on_joystick_cancel(
    trigger: On<Pointer<Cancel>>,
    mut pads: Query<&mut JoystickDragState, With<GenericJoystickPad>>,
    mut commands: Commands,
) {
    let entity = trigger.event().entity;
    if let Ok(mut state) = pads.get_mut(entity) {
        release_joystick(&mut state, entity, &mut commands);
    }
}

fn release_joystick(state: &mut JoystickDragState, entity: Entity, commands: &mut Commands) {
    state.active = false;
    state.knob_px_dx = 0.0;
    state.knob_px_dy = 0.0;
    state.last_dx = 0.0;
    state.last_dy = 0.0;
    commands
        .entity(entity)
        .trigger(|e| JoystickMoved { entity: e, dx: 0.0, dy: 0.0 });
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Mirrors `JoystickDragState.active` into `WidgetState.active` so the `active`
/// visual persists for the full drag duration, even when the pointer moves off-entity
/// (at which point Bevy's `Interaction` would otherwise revert to `Hovered`/`None`).
fn sync_drag_active_to_widget_state(
    mut pads: Query<
        (&JoystickDragState, &mut WidgetState),
        (Changed<JoystickDragState>, With<GenericJoystickPad>),
    >,
) {
    for (drag, mut widget) in pads.iter_mut() {
        widget.active = drag.active;
    }
}

/// Fires `JoystickMoved` at 10 Hz regardless of new input (handles lossy
/// connections where the server needs periodic updates).
fn tick_joystick_resend(
    time: Res<Time>,
    mut pads: Query<(Entity, &mut JoystickResendTimer, &JoystickDragState), With<GenericJoystickPad>>,
    mut commands: Commands,
) {
    for (entity, mut resend, state) in pads.iter_mut() {
        resend.timer.tick(time.delta());
        if resend.timer.just_finished() {
            let (dx, dy) = (state.last_dx, state.last_dy);
            commands.entity(entity).trigger(|e| JoystickMoved { entity: e, dx, dy });
        }
    }
}

/// Keeps the knob node position in sync with `JoystickDragState`.
fn update_joystick_knob_position(
    pads: Query<
        (&GenericJoystickPad, &JoystickDragState, &Children),
        (Changed<JoystickDragState>, With<GenericJoystickPad>),
    >,
    mut knobs: Query<(&mut Node, &GenericJoystickKnob)>,
) {
    for (pad_comp, state, children) in pads.iter() {
        let centre = pad_comp.pad_radius;
        for child in children.iter() {
            if let Ok((mut node, knob)) = knobs.get_mut(child) {
                node.left = Val::Px(centre - knob.half_size + state.knob_px_dx);
                node.top = Val::Px(centre - knob.half_size + state.knob_px_dy);
            }
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the joystick widget.  Registered automatically by `GuiPlugin`.
pub struct GuiJoystickPlugin;

impl Plugin for GuiJoystickPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (tick_joystick_resend, update_joystick_knob_position, sync_drag_active_to_widget_state),
        );
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_joystick ───────────────────────────────────────────────────

    #[test]
    fn center_position_gives_zero_output() {
        let (dx, dy) = normalize_joystick(0.0, 0.0, 100.0);
        assert_eq!(dx, 0.0);
        assert_eq!(dy, 0.0);
    }

    #[test]
    fn full_right_gives_positive_dx() {
        let (dx, dy) = normalize_joystick(100.0, 0.0, 100.0);
        assert!((dx - 1.0).abs() < 1e-5, "expected dx 1.0, got {dx}");
        assert_eq!(dy, 0.0);
    }

    #[test]
    fn full_left_gives_negative_dx() {
        let (dx, _) = normalize_joystick(-100.0, 0.0, 100.0);
        assert!((dx - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn full_down_gives_positive_dy() {
        let (_, dy) = normalize_joystick(0.0, 100.0, 100.0);
        assert!((dy - 1.0).abs() < 1e-5, "expected dy 1.0, got {dy}");
    }

    #[test]
    fn full_up_gives_negative_dy() {
        let (_, dy) = normalize_joystick(0.0, -100.0, 100.0);
        assert!((dy - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn outside_radius_is_clamped_to_unit_range() {
        let (dx, dy) = normalize_joystick(500.0, 500.0, 100.0);
        assert!(dx.abs() <= 1.0, "dx out of range: {dx}");
        assert!(dy.abs() <= 1.0, "dy out of range: {dy}");
    }

    #[test]
    fn zero_radius_returns_zero_safely() {
        let (dx, dy) = normalize_joystick(50.0, 50.0, 0.0);
        assert_eq!(dx, 0.0);
        assert_eq!(dy, 0.0);
    }

    #[test]
    fn negative_radius_returns_zero_safely() {
        let (dx, dy) = normalize_joystick(50.0, 50.0, -10.0);
        assert_eq!(dx, 0.0);
        assert_eq!(dy, 0.0);
    }

    #[test]
    fn half_radius_right_gives_half_dx() {
        let (dx, _) = normalize_joystick(50.0, 0.0, 100.0);
        assert!((dx - 0.5).abs() < 1e-5, "expected dx 0.5, got {dx}");
    }
}
