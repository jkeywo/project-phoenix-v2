//! `GuiPanel` widget — container with background, animated size, and
//! idle/active border colours driven by `WidgetState`.
//!
//! Panels are passive containers (no `Interaction`) so `resolve_visuals_system`
//! from `foundation` does not run on them.  A dedicated system drives
//! `BorderColor` from `StateVisuals` + `WidgetState` each frame.

use bevy::prelude::*;

use super::{resolve_visual, Disabled, StateVisuals, WidgetState};

// ── Marker ────────────────────────────────────────────────────────────────────

/// Marker component on every entity spawned by `GuiPanel::spawn`.
#[derive(Component, Default)]
pub struct GuiPanelMarker;

// ── Animated size ─────────────────────────────────────────────────────────────

/// Attach to a panel entity to animate its size smoothly.  Game logic writes
/// `PanelSize.target` whenever it wants the panel to resize; the system lerps
/// the `Node` width/height toward the target each frame.
///
/// `lerp_speed` is the lerp factor per second (default `5.0`).
#[derive(Component, Clone, Debug)]
pub struct PanelSize {
    pub target: Vec2,
    pub lerp_speed: f32,
}

impl PanelSize {
    /// Convenience constructor with the default lerp speed.
    pub fn new(target: Vec2) -> Self {
        Self { target, lerp_speed: 5.0 }
    }
}

// ── Pure helper ───────────────────────────────────────────────────────────────

/// Return the next lerped size given current and target sizes and delta time.
///
/// `lerp_speed` is the fraction of remaining distance to close per second.
/// Pure function — fully unit-testable without a running `App`.
pub fn lerp_size(current: Vec2, target: Vec2, lerp_speed: f32, delta_secs: f32) -> Vec2 {
    let t = (lerp_speed * delta_secs).clamp(0.0, 1.0);
    current + (target - current) * t
}

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Namespace struct for the `GuiPanel` widget.
pub struct GuiPanel;

impl GuiPanel {
    /// Spawn a `GuiPanel` container entity.
    ///
    /// - `initial_size` — starting width and height in pixels.
    /// - `bg_image` — optional background image.
    /// - `bg_color` — background colour (applied as `BackgroundColor`).
    /// - `state_visuals` — drives the border colour per `WidgetState` state.
    ///   `idle.color` and `active.color` are the two visually distinct colours.
    ///
    /// Returns the panel entity.  Lay out children inside it with
    /// `commands.entity(panel).with_children(...)`.
    pub fn spawn(
        commands: &mut Commands,
        initial_size: Vec2,
        bg_image: Option<Handle<Image>>,
        bg_color: Color,
        state_visuals: StateVisuals,
    ) -> Entity {
        let idle_border = state_visuals.idle.color;
        let mut panel = commands.spawn((
            GuiPanelMarker,
            Node {
                width:  Val::Px(initial_size.x),
                height: Val::Px(initial_size.y),
                border: UiRect::all(Val::Px(2.0)),
                // Children are not clipped when the panel animates to a larger size.
                overflow: Overflow::visible(),
                ..default()
            },
            BackgroundColor(bg_color),
            BorderColor::all(idle_border),
            state_visuals,
            WidgetState::default(),
            PanelSize::new(initial_size),
        ));
        if let Some(img) = bg_image {
            panel.insert(ImageNode::new(img));
        }
        panel.id()
    }
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Lerps `Node` width/height toward `PanelSize.target` each frame.
fn lerp_panel_size(
    time: Res<Time>,
    mut panels: Query<(&PanelSize, &mut Node), With<GuiPanelMarker>>,
) {
    let delta = time.delta_secs();
    for (panel_size, mut node) in panels.iter_mut() {
        let current_w = match node.width  { Val::Px(v) => v, _ => panel_size.target.x };
        let current_h = match node.height { Val::Px(v) => v, _ => panel_size.target.y };
        let current = Vec2::new(current_w, current_h);
        let next = lerp_size(current, panel_size.target, panel_size.lerp_speed, delta);
        node.width  = Val::Px(next.x);
        node.height = Val::Px(next.y);
    }
}

/// Writes `BorderColor` from `StateVisuals` based on `WidgetState.active`.
/// Panels are passive containers (no `Interaction`), so only idle/active/disabled
/// states are relevant; press and hover are treated as idle.
fn drive_panel_border(
    mut panels: Query<
        (&StateVisuals, &WidgetState, Has<Disabled>, &mut BorderColor),
        (With<GuiPanelMarker>, Or<(Changed<WidgetState>, Changed<StateVisuals>)>),
    >,
) {
    for (visuals, state, is_disabled, mut border) in panels.iter_mut() {
        let visual = resolve_visual(visuals, is_disabled, false, state.active, false);
        *border = BorderColor::all(visual.color);
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the panel widget.  Registered automatically by `GuiPlugin`.
pub struct GuiPanelPlugin;

impl Plugin for GuiPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (lerp_panel_size, drive_panel_border));
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── lerp_size ────────────────────────────────────────────────────────────

    #[test]
    fn lerp_size_at_target_returns_target() {
        let t = Vec2::new(200.0, 100.0);
        let result = lerp_size(t, t, 5.0, 0.1);
        assert!((result.x - t.x).abs() < 1e-4);
        assert!((result.y - t.y).abs() < 1e-4);
    }

    #[test]
    fn lerp_size_moves_toward_target() {
        let current = Vec2::new(0.0, 0.0);
        let target  = Vec2::new(100.0, 50.0);
        let result  = lerp_size(current, target, 5.0, 0.1);
        // t = clamp(5.0 * 0.1, 0, 1) = 0.5 → halfway
        assert!((result.x - 50.0).abs() < 1e-4, "x={}", result.x);
        assert!((result.y - 25.0).abs() < 1e-4, "y={}", result.y);
    }

    #[test]
    fn lerp_size_clamps_overshoot() {
        let current = Vec2::new(0.0, 0.0);
        let target  = Vec2::new(100.0, 100.0);
        // Very large delta: should reach target exactly, not overshoot.
        let result = lerp_size(current, target, 5.0, 10.0);
        assert!((result.x - 100.0).abs() < 1e-4);
        assert!((result.y - 100.0).abs() < 1e-4);
    }

    #[test]
    fn lerp_size_zero_delta_returns_current() {
        let current = Vec2::new(80.0, 60.0);
        let target  = Vec2::new(200.0, 200.0);
        let result  = lerp_size(current, target, 5.0, 0.0);
        assert!((result.x - current.x).abs() < 1e-4);
        assert!((result.y - current.y).abs() < 1e-4);
    }

    #[test]
    fn lerp_size_works_in_both_directions() {
        let current = Vec2::new(100.0, 100.0);
        let target  = Vec2::new(0.0, 0.0);
        let result  = lerp_size(current, target, 5.0, 0.1);
        // t = 0.5 → 50.0
        assert!((result.x - 50.0).abs() < 1e-4);
    }
}
