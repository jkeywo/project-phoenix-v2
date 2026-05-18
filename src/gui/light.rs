//! `FlickerLight` widget — cosmetic UI node that toggles between two colours
//! on a per-entity randomised timer.
//!
//! Each entity keeps its own `FlickerLightState` so lights on the same panel
//! drift out of phase naturally.  When `WidgetState.active` is `true` the
//! widget uses a faster interval pair for a strobe/alert effect.

use rand::Rng;
use bevy::prelude::*;

use super::WidgetState;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Stored on the entity at spawn; describes on/off colours and timing.
#[derive(Component, Clone, Debug)]
pub struct FlickerLightConfig {
    pub on_color:     Color,
    pub off_color:    Color,
    /// Minimum seconds between flicker transitions (idle state).
    pub idle_min_secs: f32,
    /// Maximum seconds between flicker transitions (idle state).
    pub idle_max_secs: f32,
}

/// Per-entity mutable flicker state — timer and current lit status.
#[derive(Component)]
pub struct FlickerLightState {
    pub is_on: bool,
    pub timer: Timer,
}

/// Marker on every entity spawned by `FlickerLight::spawn`.
#[derive(Component, Default)]
pub struct FlickerLightMarker;

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// The active/alert flicker interval is always one-fifth of the idle interval,
/// producing a clearly faster strobe effect.
const ACTIVE_INTERVAL_SCALE: f32 = 0.20;

/// Return the `(min_secs, max_secs)` interval pair for the current state.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn effective_interval(config: &FlickerLightConfig, active: bool) -> (f32, f32) {
    if active {
        (
            config.idle_min_secs * ACTIVE_INTERVAL_SCALE,
            config.idle_max_secs * ACTIVE_INTERVAL_SCALE,
        )
    } else {
        (config.idle_min_secs, config.idle_max_secs)
    }
}

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Namespace struct for the `FlickerLight` widget.
pub struct FlickerLight;

impl FlickerLight {
    /// Spawn a `FlickerLight` entity.
    ///
    /// - `size` — node dimensions in pixels.
    /// - `on_color` / `off_color` — colours to alternate between.
    /// - `idle_min_secs` / `idle_max_secs` — random interval range (seconds).
    ///
    /// The entity starts with a randomly-chosen initial duration so that
    /// multiple lights spawned together will desynchronise immediately.
    pub fn spawn(
        commands: &mut Commands,
        size: Vec2,
        on_color: Color,
        off_color: Color,
        idle_min_secs: f32,
        idle_max_secs: f32,
    ) -> Entity {
        let mut rng = rand::rng();
        let initial_duration =
            rng.random_range(idle_min_secs..=idle_max_secs.max(idle_min_secs));

        commands
            .spawn((
                FlickerLightMarker,
                FlickerLightConfig {
                    on_color,
                    off_color,
                    idle_min_secs,
                    idle_max_secs,
                },
                FlickerLightState {
                    is_on: false,
                    timer: Timer::from_seconds(initial_duration, TimerMode::Once),
                },
                WidgetState::default(),
                Node {
                    width:  Val::Px(size.x),
                    height: Val::Px(size.y),
                    ..default()
                },
                BackgroundColor(off_color),
            ))
            .id()
    }
}

// ── System ────────────────────────────────────────────────────────────────────

/// Tick each flicker timer; on expiry flip the light and sample a new duration.
fn tick_flicker_lights(
    time: Res<Time>,
    mut lights: Query<
        (&FlickerLightConfig, &mut FlickerLightState, Option<&WidgetState>, &mut BackgroundColor),
        With<FlickerLightMarker>,
    >,
) {
    let delta = time.delta();
    let mut rng = rand::rng();

    for (config, mut state, widget_state, mut bg) in lights.iter_mut() {
        state.timer.tick(delta);

        if state.timer.just_finished() {
            // Flip the light.
            state.is_on = !state.is_on;
            bg.0 = if state.is_on { config.on_color } else { config.off_color };

            // Sample a new interval based on current activation state.
            let active = widget_state.map_or(false, |s| s.active);
            let (min, max) = effective_interval(config, active);
            let next_duration = rng.random_range(min..=max.max(min));
            state.timer = Timer::from_seconds(next_duration, TimerMode::Once);
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the flicker light widget.  Registered automatically by `GuiPlugin`.
pub struct GuiLightPlugin;

impl Plugin for GuiLightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, tick_flicker_lights);
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(min: f32, max: f32) -> FlickerLightConfig {
        FlickerLightConfig {
            on_color:      Color::WHITE,
            off_color:     Color::BLACK,
            idle_min_secs: min,
            idle_max_secs: max,
        }
    }

    #[test]
    fn idle_interval_returns_config_values() {
        let cfg = make_config(0.5, 1.5);
        let (min, max) = effective_interval(&cfg, false);
        assert!((min - 0.5).abs() < 1e-5);
        assert!((max - 1.5).abs() < 1e-5);
    }

    #[test]
    fn active_interval_is_strictly_shorter_than_idle() {
        let cfg = make_config(0.5, 1.5);
        let (idle_min, idle_max) = effective_interval(&cfg, false);
        let (act_min, act_max)   = effective_interval(&cfg, true);
        assert!(act_min < idle_min, "active min should be shorter");
        assert!(act_max < idle_max, "active max should be shorter");
    }

    #[test]
    fn active_interval_scale_is_applied_uniformly() {
        let cfg = make_config(1.0, 2.0);
        let (act_min, act_max) = effective_interval(&cfg, true);
        assert!((act_min - 1.0 * ACTIVE_INTERVAL_SCALE).abs() < 1e-5);
        assert!((act_max - 2.0 * ACTIVE_INTERVAL_SCALE).abs() < 1e-5);
    }
}
