//! `ProgressBar` widget — continuous (solid fill) and segmented (discrete blocks) variants.
//!
//! Both share `ProgressValue(f32)` in `[0.0, 1.0]` and `StateVisuals` for
//! Disabled / Idle / Active colouring.  Writing `ProgressValue` triggers an
//! update on the next `Update` frame.

use bevy::prelude::*;

use super::{resolve_visual, Disabled, StateVisuals, WidgetState};

// ── Public types ──────────────────────────────────────────────────────────────

/// Current fill level in `[0.0, 1.0]`.  Write this component to change the bar.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ProgressValue(pub f32);

/// Number of discrete segments for the segmented variant.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentCount(pub u8);

/// Which layout variant a `ProgressBar` uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressBarVariant {
    Continuous,
    Segmented,
}

// ── Internal component ─────────────────────────────────────────────────────────

/// Marker on the root progress-bar entity.
#[derive(Component, Default)]
pub struct ProgressBarMarker;

/// Stored configuration on the root entity.
#[derive(Component, Clone, Debug)]
pub struct ProgressBarConfig {
    pub variant: ProgressBarVariant,
    /// Number of segments; only meaningful for `Segmented`.
    pub segment_count: u8,
}

/// Marker on the continuous fill child node.
#[derive(Component)]
pub struct ProgressBarFill;

/// Marker on each discrete segment child node; `index` starts at 0.
#[derive(Component)]
pub struct ProgressBarSegment {
    pub index: u8,
}

// ── Pure helpers ───────────────────────────────────────────────────────────────

/// Number of filled segments for the segmented variant.
///
/// `floor(value.clamp(0,1) * count)`, clamped to `[0, count]`.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn filled_segments(value: f32, count: u8) -> u8 {
    if count == 0 {
        return 0;
    }
    let v = value.clamp(0.0, 1.0);
    ((v * count as f32).floor() as u8).min(count)
}

// ── Spawn helper ───────────────────────────────────────────────────────────────

/// Namespace struct for the `ProgressBar` widget.
pub struct ProgressBar;

impl ProgressBar {
    /// Spawn a `ProgressBar` entity.
    ///
    /// - `size` — outer node dimensions in pixels.
    /// - `variant` — `Continuous` or `Segmented`.
    /// - `state_visuals` — fill colour per state (idle / active / disabled).
    /// - `segment_count` — number of discrete segments (`Segmented` only;
    ///   ignored for `Continuous`).  Defaults to `SegmentCount(10)` when `None`.
    ///
    /// Returns the root entity.
    pub fn spawn(
        commands: &mut Commands,
        size: Vec2,
        variant: ProgressBarVariant,
        state_visuals: StateVisuals,
        segment_count: Option<SegmentCount>,
    ) -> Entity {
        let n_segs = segment_count.map(|s| s.0).unwrap_or(10);
        let initial_color = state_visuals.idle.color;

        let root = commands
            .spawn((
                ProgressBarMarker,
                ProgressBarConfig {
                    variant: variant.clone(),
                    segment_count: n_segs,
                },
                ProgressValue(0.0),
                state_visuals,
                WidgetState::default(),
                Node {
                    width:  Val::Px(size.x),
                    height: Val::Px(size.y),
                    overflow: Overflow::hidden(),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .id();

        match variant {
            ProgressBarVariant::Continuous => {
                commands.entity(root).with_children(|parent| {
                    parent.spawn((
                        ProgressBarFill,
                        Node {
                            width:  Val::Percent(0.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(initial_color),
                    ));
                });
            }
            ProgressBarVariant::Segmented => {
                let seg_pct = if n_segs > 0 {
                    100.0 / n_segs as f32 - 1.0 // leave ~1% gap per segment
                } else {
                    0.0
                };
                commands.entity(root).with_children(|parent| {
                    for i in 0..n_segs {
                        parent.spawn((
                            ProgressBarSegment { index: i },
                            Node {
                                width:  Val::Percent(seg_pct),
                                height: Val::Percent(100.0),
                                margin: UiRect {
                                    right: Val::Percent(1.0),
                                    ..default()
                                },
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                        ));
                    }
                });
            }
        }

        root
    }
}

// ── Update system ──────────────────────────────────────────────────────────────

/// Each frame: propagate `ProgressValue` changes to fill children and apply
/// `StateVisuals` colouring.
fn update_progress_bars(
    bars: Query<
        (
            &ProgressValue,
            &ProgressBarConfig,
            &StateVisuals,
            Option<&WidgetState>,
            Has<Disabled>,
            &Children,
        ),
        (
            With<ProgressBarMarker>,
            Or<(
                Changed<ProgressValue>,
                Changed<WidgetState>,
                Changed<StateVisuals>,
            )>,
        ),
    >,
    mut fills: Query<&mut Node, With<ProgressBarFill>>,
    mut segments: Query<(&ProgressBarSegment, &mut BackgroundColor)>,
) {
    for (progress, config, visuals, widget_state, is_disabled, children) in bars.iter() {
        let active = widget_state.map_or(false, |s| s.active);
        let fill_color = resolve_visual(visuals, is_disabled, false, active, false).color;
        let value = progress.0.clamp(0.0, 1.0);

        match config.variant {
            ProgressBarVariant::Continuous => {
                for child in children.iter() {
                    if let Ok(mut node) = fills.get_mut(child) {
                        node.width = Val::Percent(value * 100.0);
                    }
                }
            }
            ProgressBarVariant::Segmented => {
                let filled = filled_segments(value, config.segment_count);
                for child in children.iter() {
                    if let Ok((seg, mut bg)) = segments.get_mut(child) {
                        bg.0 = if seg.index < filled { fill_color } else { Color::NONE };
                    }
                }
            }
        }
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

/// Sub-plugin for the progress bar widget.  Registered automatically by `GuiPlugin`.
pub struct GuiProgressPlugin;

impl Plugin for GuiProgressPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_progress_bars);
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── filled_segments ──────────────────────────────────────────────────────

    #[test]
    fn zero_value_fills_no_segments() {
        assert_eq!(filled_segments(0.0, 10), 0);
    }

    #[test]
    fn full_value_fills_all_segments() {
        assert_eq!(filled_segments(1.0, 10), 10);
    }

    #[test]
    fn half_value_fills_half_segments() {
        assert_eq!(filled_segments(0.5, 10), 5);
    }

    #[test]
    fn partial_fill_floors_to_complete_segments() {
        // 0.35 * 10 = 3.5 → floor = 3
        assert_eq!(filled_segments(0.35, 10), 3);
    }

    #[test]
    fn just_below_next_segment_threshold_does_not_advance() {
        // 0.39 * 10 = 3.9 → floor = 3, not 4
        assert_eq!(filled_segments(0.39, 10), 3);
    }

    #[test]
    fn exactly_at_segment_boundary_fills_that_segment() {
        // 0.4 * 10 = 4.0 → floor = 4
        assert_eq!(filled_segments(0.40, 10), 4);
    }

    #[test]
    fn zero_segment_count_always_returns_zero() {
        assert_eq!(filled_segments(0.5, 0), 0);
        assert_eq!(filled_segments(1.0, 0), 0);
    }

    #[test]
    fn value_above_one_is_clamped() {
        assert_eq!(filled_segments(2.0, 10), 10);
    }

    #[test]
    fn value_below_zero_is_clamped() {
        assert_eq!(filled_segments(-0.5, 10), 0);
    }

    #[test]
    fn single_segment_fills_at_one() {
        assert_eq!(filled_segments(1.0, 1), 1);
        assert_eq!(filled_segments(0.9, 1), 0); // floor(0.9*1)=0
    }
}
