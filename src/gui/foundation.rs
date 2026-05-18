//! Foundation types and systems for the gui widget library.
//!
//! `WidgetState` is written by game logic to flag a widget as active or
//! disabled.  `StateVisuals` is configured at spawn time with one `Visual`
//! per state.  The `resolve_visuals_system` runs each frame and writes the
//! winning `Visual` into `ImageNode` and `BackgroundColor` according to the
//! priority order:
//!
//!   Disabled > Press > Active > Hover > Idle

use bevy::prelude::*;

// ── Visual leaf ───────────────────────────────────────────────────────────────

/// The resolved appearance for a single widget state.
#[derive(Clone, Debug, PartialEq)]
pub struct Visual {
    /// Optional override image. `None` leaves the existing `ImageNode` image
    /// unchanged (or clears it when the entity has no image at all).
    pub image: Option<Handle<Image>>,
    /// Background / tint colour applied to `BackgroundColor`.
    pub color: Color,
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            image: None,
            color: Color::NONE,
        }
    }
}

// ── StateVisuals component ────────────────────────────────────────────────────

/// Holds one `Visual` per interaction state.  Attach to any entity that uses
/// the foundation resolution system.
#[derive(Component, Clone, Debug)]
pub struct StateVisuals {
    pub idle: Visual,
    pub hover: Visual,
    pub active: Visual,
    pub press: Visual,
    pub disabled: Visual,
}

impl StateVisuals {
    /// Convenience constructor — build from five colours, no images.
    pub fn from_colors(
        idle: Color,
        hover: Color,
        active: Color,
        press: Color,
        disabled: Color,
    ) -> Self {
        let v = |color| Visual { image: None, color };
        Self {
            idle: v(idle),
            hover: v(hover),
            active: v(active),
            press: v(press),
            disabled: v(disabled),
        }
    }
}

// ── WidgetState component ─────────────────────────────────────────────────────

/// Written by game logic to flag whether a widget is in the active state
/// (e.g. a selected radio button, a shield quadrant that is focused).
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct WidgetState {
    pub active: bool,
}

// ── Disabled marker ───────────────────────────────────────────────────────────

/// Marker component: widget is disabled — it renders the `disabled` visual and
/// does not respond to `Interaction` events.
#[derive(Component, Default, Clone, Debug)]
pub struct Disabled;

// ── Pure selection helper ─────────────────────────────────────────────────────

/// Select the correct `Visual` from `StateVisuals` given the current flags.
///
/// Priority order: **Disabled > Press > Active > Hover > Idle**.
///
/// This is a pure function with no Bevy system dependencies, making it fully
/// unit-testable without a running `App`.
pub fn resolve_visual<'a>(
    visuals: &'a StateVisuals,
    disabled: bool,
    pressed: bool,
    active: bool,
    hovered: bool,
) -> &'a Visual {
    if disabled {
        &visuals.disabled
    } else if pressed {
        &visuals.press
    } else if active {
        &visuals.active
    } else if hovered {
        &visuals.hover
    } else {
        &visuals.idle
    }
}

// ── Bevy system ───────────────────────────────────────────────────────────────

/// Each frame: read `Interaction` + optional `WidgetState` + optional
/// `Disabled`, resolve the winning `Visual`, and apply it to `ImageNode`
/// and `BackgroundColor`.
pub fn resolve_visuals_system(
    mut query: Query<(
        &StateVisuals,
        &Interaction,
        Option<&WidgetState>,
        Has<Disabled>,
        Option<&mut ImageNode>,
        &mut BackgroundColor,
    )>,
) {
    for (state_visuals, interaction, widget_state, is_disabled, image_node, mut bg) in
        query.iter_mut()
    {
        let pressed = *interaction == Interaction::Pressed;
        let hovered = *interaction == Interaction::Hovered;
        let active = widget_state.map_or(false, |s| s.active);

        let visual = resolve_visual(state_visuals, is_disabled, pressed, active, hovered);

        bg.0 = visual.color;
        if let Some(mut img) = image_node {
            if let Some(handle) = &visual.image {
                img.image = handle.clone();
            }
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `StateVisuals` where each state has a uniquely-identifiable
    /// colour so tests can verify which state was selected.
    fn test_visuals() -> StateVisuals {
        StateVisuals::from_colors(
            Color::srgb(0.1, 0.0, 0.0), // idle
            Color::srgb(0.2, 0.0, 0.0), // hover
            Color::srgb(0.3, 0.0, 0.0), // active
            Color::srgb(0.4, 0.0, 0.0), // press
            Color::srgb(0.5, 0.0, 0.0), // disabled
        )
    }

    // Helpers for readability
    fn idle_color() -> Color    { Color::srgb(0.1, 0.0, 0.0) }
    fn hover_color() -> Color   { Color::srgb(0.2, 0.0, 0.0) }
    fn active_color() -> Color  { Color::srgb(0.3, 0.0, 0.0) }
    fn press_color() -> Color   { Color::srgb(0.4, 0.0, 0.0) }
    fn disabled_color() -> Color { Color::srgb(0.5, 0.0, 0.0) }

    #[test]
    fn idle_when_no_flags() {
        let v = test_visuals();
        let result = resolve_visual(&v, false, false, false, false);
        assert_eq!(result.color, idle_color());
    }

    #[test]
    fn hover_when_only_hovered() {
        let v = test_visuals();
        let result = resolve_visual(&v, false, false, false, true);
        assert_eq!(result.color, hover_color());
    }

    #[test]
    fn active_when_only_active() {
        let v = test_visuals();
        let result = resolve_visual(&v, false, false, true, false);
        assert_eq!(result.color, active_color());
    }

    #[test]
    fn press_when_only_pressed() {
        let v = test_visuals();
        let result = resolve_visual(&v, false, true, false, false);
        assert_eq!(result.color, press_color());
    }

    #[test]
    fn disabled_overrides_all() {
        let v = test_visuals();
        // disabled + press + active + hovered — disabled wins
        let result = resolve_visual(&v, true, true, true, true);
        assert_eq!(result.color, disabled_color());
    }

    #[test]
    fn press_beats_active() {
        let v = test_visuals();
        // pressed and active simultaneously — press wins
        let result = resolve_visual(&v, false, true, true, false);
        assert_eq!(result.color, press_color());
    }

    #[test]
    fn press_beats_active_and_hover() {
        let v = test_visuals();
        let result = resolve_visual(&v, false, true, true, true);
        assert_eq!(result.color, press_color());
    }

    #[test]
    fn active_beats_hover() {
        let v = test_visuals();
        let result = resolve_visual(&v, false, false, true, true);
        assert_eq!(result.color, active_color());
    }

    #[test]
    fn hover_beats_idle() {
        let v = test_visuals();
        let result = resolve_visual(&v, false, false, false, true);
        assert_eq!(result.color, hover_color());
    }

    #[test]
    fn disabled_beats_hover() {
        let v = test_visuals();
        let result = resolve_visual(&v, true, false, false, true);
        assert_eq!(result.color, disabled_color());
    }

    #[test]
    fn disabled_beats_active() {
        let v = test_visuals();
        let result = resolve_visual(&v, true, false, true, false);
        assert_eq!(result.color, disabled_color());
    }

    #[test]
    fn disabled_beats_press() {
        let v = test_visuals();
        let result = resolve_visual(&v, true, true, false, false);
        assert_eq!(result.color, disabled_color());
    }
}
