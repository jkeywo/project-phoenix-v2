//! Synthetic coverage for the pure setup/layout accessibility model (issue
//! #1128, acceptance criterion 5): the reflow-headroom check at the supported
//! scaling extremes for one- and two-pane layouts, the keyboard-focus order
//! across monitors and split panes, and the setup-action reachability
//! invariant — all with no hardware, which is the whole point of keeping the
//! model Bevy-free. The real-monitor, real-text half is the ignored
//! `tests/native_bridge_accessibility.rs` and the walkthrough in
//! `docs/acceptance/1128-accessibility.md`.

use super::*;
use crate::native_host::bridge_profile::{
    DisplayRole, MonitorGeometry, MonitorIdentity, PaneSlot, PaneSplit, ProfileProblem,
    ResolvedBridge, ResolvedSurface,
};
use crate::native_host::input_routing::FocusRing;
use crate::native_host::panes::os_prefs::OsAccessibilityPrefs;
use crate::native_host::panes::registry::PaneId;

// ── fixtures ─────────────────────────────────────────────────────────────────

fn geometry(w: u32, h: u32, x: i32, y: i32, scale: f64) -> MonitorGeometry {
    MonitorGeometry {
        physical_width: w,
        physical_height: h,
        position_x: x,
        position_y: y,
        scale_factor: scale,
    }
}

fn station(
    id: &str,
    geometry: MonitorGeometry,
    split: PaneSplit,
    labels: &[&str],
) -> ResolvedSurface {
    ResolvedSurface {
        identity: MonitorIdentity::new(id),
        geometry,
        role: DisplayRole::Station {
            split,
            panes: labels
                .iter()
                .copied()
                .map(PaneSlot::for_participant)
                .collect(),
        },
        primary: false,
    }
}

fn viewscreen(id: &str, geometry: MonitorGeometry) -> ResolvedSurface {
    ResolvedSurface {
        identity: MonitorIdentity::new(id),
        geometry,
        role: DisplayRole::Viewscreen,
        primary: true,
    }
}

fn bridge(surfaces: Vec<ResolvedSurface>) -> ResolvedBridge {
    ResolvedBridge {
        surfaces,
        problems: Vec::new(),
    }
}

/// The three monitor resolutions the bridge is authored for, each at a plausible
/// Windows scale factor — the "supported profiles" the reflow check must clear.
fn supported_geometries() -> Vec<MonitorGeometry> {
    vec![
        geometry(1920, 1080, 0, 0, 1.0),
        geometry(1920, 1080, 0, 0, 1.5),
        geometry(2560, 1440, 0, 0, 1.0),
        geometry(3840, 2160, 0, 0, 2.0),
    ]
}

// ── the supported text-scale extremes ────────────────────────────────────────

#[test]
fn the_supported_extremes_match_the_client() {
    // These mirror TEXT_SCALE_MIN/TEXT_SCALE_MAX in gui/accessibility-profile.js,
    // the authority the player's slider and the CSS `--a11y-text-scale` var use.
    // If the client range changes, this must change with it — the reflow check
    // reasons over the range the page can actually produce.
    assert_eq!(SUPPORTED_TEXT_SCALE_MIN, 1.0);
    // 2.0 since issue #1422 (PRD #1418's "usable in-app text enlargement
    // through 200%"). If gui/accessibility-profile.js moves TEXT_SCALE_MAX
    // again, this assertion fails first and the reflow-headroom checks below —
    // which reason over MIN_CONSOLE_LOGICAL_WIDTH_PX x this multiplier — are
    // re-derived with it rather than silently standing behind a range the page
    // can no longer produce.
    assert_eq!(SUPPORTED_TEXT_SCALE_MAX, 2.0);
}

// ── reflow headroom (acceptance criterion 1) ─────────────────────────────────

#[test]
fn a_pane_box_is_the_physical_rect_over_the_scale() {
    // A 3840×2160 pane at 2.0 scale is a 1920×1080 logical console box — the same
    // division "display scaling" is in input_routing.
    let b = PaneContentBox::of(
        &PaneRect {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
        },
        2.0,
    );
    assert_eq!(b.logical_width, 1920.0);
    assert_eq!(b.logical_height, 1080.0);
}

#[test]
fn one_pane_full_monitor_preserves_the_console_at_both_extremes() {
    // AC1: a single pane covering a supported monitor keeps its console at the
    // minimum AND the maximum supported text scale.
    for g in supported_geometries() {
        let s = station("m", g.clone(), PaneSplit::SideBySide, &["Ada"]);
        let order = bridge_focus_order(&bridge(vec![s]));
        assert_eq!(order.len(), 1);
        let b = order[0].content_box();
        assert!(
            b.preserves_console_at_scale(SUPPORTED_TEXT_SCALE_MIN),
            "one-pane {g:?} fails at min scale ({b:?})"
        );
        assert!(
            b.preserves_console_at_scale(SUPPORTED_TEXT_SCALE_MAX),
            "one-pane {g:?} fails at max scale ({b:?})"
        );
    }
}

#[test]
fn two_pane_layouts_preserve_both_consoles_at_both_extremes() {
    // AC1: a two-pane Station, side by side OR stacked, on any supported monitor,
    // keeps BOTH consoles at both supported scaling extremes — no overlap, no
    // unreachable actions.
    for g in supported_geometries() {
        for split in [PaneSplit::SideBySide, PaneSplit::Stacked] {
            let s = station("m", g.clone(), split, &["Ada", "Grace"]);
            let order = bridge_focus_order(&bridge(vec![s]));
            assert_eq!(order.len(), 2, "two panes expected");
            for pane in &order {
                let b = pane.content_box();
                assert!(
                    b.preserves_console_at_scale(SUPPORTED_TEXT_SCALE_MIN),
                    "{split:?} {g:?} pane {} fails at min scale ({b:?})",
                    pane.label
                );
                assert!(
                    b.preserves_console_at_scale(SUPPORTED_TEXT_SCALE_MAX),
                    "{split:?} {g:?} pane {} fails at max scale ({b:?})",
                    pane.label
                );
            }
        }
    }
}

#[test]
fn the_two_panes_tile_their_monitor_with_no_overlap_and_no_gap() {
    // "no overlap" is exact geometry, independent of scale: the two pane rects
    // exactly partition the monitor along the split axis.
    for split in [PaneSplit::SideBySide, PaneSplit::Stacked] {
        let g = geometry(1920, 1080, 0, 0, 1.0);
        let order = bridge_focus_order(&bridge(vec![station("m", g, split, &["Ada", "Grace"])]));
        let a = order[0].rect;
        let b = order[1].rect;
        // Areas sum to the whole monitor and the two rects do not intersect.
        let total = a.width as u64 * a.height as u64 + b.width as u64 * b.height as u64;
        assert_eq!(
            total,
            1920 * 1080,
            "{split:?} panes do not cover the monitor"
        );
        assert!(
            !rects_overlap(&a, &b),
            "{split:?} panes overlap: {a:?} {b:?}"
        );
    }
}

fn rects_overlap(a: &PaneRect, b: &PaneRect) -> bool {
    let ax2 = a.x + a.width;
    let ay2 = a.y + a.height;
    let bx2 = b.x + b.width;
    let by2 = b.y + b.height;
    a.x < bx2 && b.x < ax2 && a.y < by2 && b.y < ay2
}

#[test]
fn the_two_hundred_percent_ceiling_demands_640_logical_px_of_pane_width() {
    // Issue #1422 raised the exposed ceiling to 200%, and the ONLY thing that
    // changes on this side is the width demand: a pane is preserved at the new
    // maximum exactly when it holds MIN_CONSOLE_LOGICAL_WIDTH_PX x 2.0 = 640
    // logical pixels. Pinned as a boundary rather than restated as arithmetic,
    // so a later change to either the constant or the ceiling has to come and
    // look at this line.
    let at = |w: f64| PaneContentBox {
        logical_width: w,
        logical_height: 1080.0,
    };
    assert!(at(640.0).preserves_console_across_supported_scaling());
    assert!(!at(639.0).preserves_console_across_supported_scaling());
    // The old 150% ceiling would have accepted 480; it must not any more, or
    // the raise is cosmetic.
    assert!(!at(480.0).preserves_console_across_supported_scaling());
    assert!(at(480.0).preserves_console_at_scale(1.5));
    // The height floor is deliberately NOT multiplied: consoles scroll.
    assert!(PaneContentBox {
        logical_width: 640.0,
        logical_height: 320.0,
    }
    .preserves_console_across_supported_scaling());
}

#[test]
fn the_tightest_supported_split_still_holds_both_consoles_at_two_hundred_percent() {
    // The narrowest logical pane any `supported_geometries()` profile produces
    // is a side-by-side split of a 1920x1080 monitor at Windows 150% scaling:
    // 1280 logical px halved is 640 — exactly the 200% demand. This is the case
    // that decides whether the new ceiling is supportable on the authored
    // bridge at all, so it is named rather than left inside the loop above.
    let g = geometry(1920, 1080, 0, 0, 1.5);
    let order = bridge_focus_order(&bridge(vec![station(
        "tight",
        g,
        PaneSplit::SideBySide,
        &["Ada", "Grace"],
    )]));
    assert_eq!(order.len(), 2);
    for pane in &order {
        let b = pane.content_box();
        assert_eq!(b.logical_width, 640.0, "pane {} box {b:?}", pane.label);
        assert!(
            pane.preserves_console(),
            "pane {} must still hold its console at 200% ({b:?})",
            pane.label
        );
    }
}

#[test]
fn the_check_rejects_a_pane_too_small_at_the_maximum() {
    // The predicate must be a real gate, not vacuously true: a two-pane
    // side-by-side split on a small 640×480 monitor gives each pane a 320-wide
    // box, which the maximum text scale (×2.0 → 640 demand, issue #1422) no
    // longer holds.
    let order = bridge_focus_order(&bridge(vec![station(
        "small",
        geometry(640, 480, 0, 0, 1.0),
        PaneSplit::SideBySide,
        &["Ada", "Grace"],
    )]));
    let b = order[0].content_box();
    assert!(
        b.preserves_console_at_scale(SUPPORTED_TEXT_SCALE_MIN),
        "still fine at 1.0x"
    );
    assert!(
        !b.preserves_console_at_scale(SUPPORTED_TEXT_SCALE_MAX),
        "must fail at 2.0x: {b:?}"
    );
    assert!(!order[0].preserves_console());
    assert!(!bridge_preserves_all_consoles(&bridge(vec![station(
        "small",
        geometry(640, 480, 0, 0, 1.0),
        PaneSplit::SideBySide,
        &["Ada", "Grace"],
    )])));
}

#[test]
fn a_garbage_scale_never_reports_a_false_failure() {
    let b = PaneContentBox::of(
        &PaneRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        0.0,
    );
    // scale 0 → treated as 1.0.
    assert_eq!(b.logical_width, 1920.0);
    // A non-finite / negative text scale demands nothing, so it never fails.
    assert!(b.preserves_console_at_scale(f64::NAN));
    assert!(b.preserves_console_at_scale(-3.0));
}

// ── focus order across monitors and split panes (acceptance criterion 3) ─────

fn two_monitor_bridge() -> ResolvedBridge {
    bridge(vec![
        viewscreen("vs@1920x1080", geometry(1920, 1080, 0, 0, 1.0)),
        station(
            "left@1920x1080",
            geometry(1920, 1080, 1920, 0, 1.0),
            PaneSplit::SideBySide,
            &["Ada", "Grace"],
        ),
        station(
            "right@1920x1080",
            geometry(1920, 1080, 3840, 0, 1.0),
            PaneSplit::Stacked,
            &["Hopper", "Lovelace"],
        ),
    ])
}

#[test]
fn the_focus_order_lists_every_station_pane_once_across_monitors() {
    // AC3 "across monitors and split panes": every configured pane appears
    // exactly once, in monitor-then-split order; the viewscreen contributes none.
    let order = bridge_focus_order(&two_monitor_bridge());
    let labels: Vec<&str> = order.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["Ada", "Grace", "Hopper", "Lovelace"]);
    // Grouped by monitor in profile order.
    assert_eq!(order[0].monitor.as_str(), "left@1920x1080");
    assert_eq!(order[1].monitor.as_str(), "left@1920x1080");
    assert_eq!(order[2].monitor.as_str(), "right@1920x1080");
    assert_eq!(order[3].monitor.as_str(), "right@1920x1080");
}

#[test]
fn the_split_order_is_left_to_right_then_top_to_bottom() {
    let order = bridge_focus_order(&two_monitor_bridge());
    // Side-by-side monitor: first pane is the left half, second the right half.
    assert_eq!(order[0].rect.x, 0);
    assert!(order[1].rect.x >= order[0].rect.width);
    // Stacked monitor: first pane is the top half, second the bottom half.
    assert_eq!(order[2].rect.y, 0);
    assert!(order[3].rect.y >= order[2].rect.height);
}

#[test]
fn a_focus_ring_cycles_every_pane_across_monitors() {
    // The order drives a FocusRing (the input_routing model): Ctrl+Tab from the
    // last pane on one monitor wraps to the first on the next, and a full cycle
    // returns to the start — no pane is unreachable by keyboard.
    let order = bridge_focus_order(&two_monitor_bridge());
    let ids: Vec<PaneId> = (0..order.len() as u32).map(PaneId).collect();
    let mut ring = FocusRing::focused_on_first_pane(ids.clone());
    assert_eq!(ring.focused(), Some(ids[0]));
    let mut visited = vec![ring.focused().unwrap()];
    for _ in 0..order.len() - 1 {
        visited.push(ring.focus_next().unwrap());
    }
    assert_eq!(visited, ids, "a forward cycle visits every pane in order");
    // One more step wraps back to the first.
    assert_eq!(ring.focus_next(), Some(ids[0]));
    // And backward from the first wraps to the last.
    assert_eq!(ring.focus_prev(), Some(ids[order.len() - 1]));
}

#[test]
fn every_pane_across_monitors_preserves_its_console() {
    assert!(bridge_preserves_all_consoles(&two_monitor_bridge()));
}

#[test]
fn a_bridge_with_no_stations_has_an_empty_focus_order() {
    let b = bridge(vec![viewscreen("vs", geometry(1920, 1080, 0, 0, 1.0))]);
    assert!(bridge_focus_order(&b).is_empty());
    // Vacuously preserves — there is nothing to fail.
    assert!(bridge_preserves_all_consoles(&b));
}

// ── setup-action reachability (acceptance criterion 2) ───────────────────────

#[test]
fn every_setup_action_is_reachable_by_keyboard_and_mouse() {
    // AC2: display, pane, touch AND media assignment are all reachable by
    // keyboard and mouse — none is touch-first-only.
    for action in SetupAction::ALL {
        let routes = action.routes();
        assert!(
            routes.keyboard_and_mouse(),
            "{} is not keyboard+mouse reachable",
            action.label()
        );
        assert!(!routes.is_touch_only(), "{} is touch-only", action.label());
    }
    assert!(every_setup_action_is_keyboard_and_mouse_reachable());
}

#[test]
fn the_four_assignment_actions_are_all_covered() {
    // The set is exactly the display/pane/touch/media assignments AC2 names.
    assert!(SetupAction::ALL.contains(&SetupAction::DisplayRole));
    assert!(SetupAction::ALL.contains(&SetupAction::PaneAssignment));
    assert!(SetupAction::ALL.contains(&SetupAction::TouchMapping));
    assert!(SetupAction::ALL.contains(&SetupAction::MediaAssignment));
    assert_eq!(SetupAction::ALL.len(), 4);
}

#[test]
fn the_invariant_would_catch_a_touch_only_action() {
    // A guard on the guard: an action reachable only by touch is flagged.
    let touch_only = InputRoutes {
        keyboard: false,
        mouse: false,
        touch: true,
    };
    assert!(touch_only.is_touch_only());
    assert!(!touch_only.keyboard_and_mouse());
}

// ── the accessibility half of the --setup report ─────────────────────────────

#[test]
fn the_report_states_the_os_defaults_and_supported_range() {
    let prefs = OsAccessibilityPrefs {
        reduced_motion: true,
        high_contrast: true,
        text_scale: 1.25,
        ..Default::default()
    };
    let report = render_accessibility_setup_report(None, &prefs);
    assert!(report.contains("Accessibility:"));
    assert!(report.contains("contrast on"));
    assert!(report.contains("reduced motion on"));
    // The exposed range the operator is told about, since issue #1422 raised
    // the ceiling to 200%. Spelled out rather than formatted from the constants
    // so the printed report is pinned, not merely self-consistent.
    assert!(report.contains("1x to 2x"), "{report}");
    assert!(report.contains("No profile resolved"));
}

#[test]
fn the_report_distinguishes_unavailable_from_neutral_os_values() {
    let prefs = OsAccessibilityPrefs {
        availability: Some(super::super::panes::os_prefs::OsPreferenceAvailability {
            text_scale: false,
            high_contrast: true,
            reduced_motion: false,
        }),
        ..Default::default()
    };
    let report = render_accessibility_setup_report(None, &prefs);
    assert!(report.contains("OS text size: unavailable"));
    assert!(report.contains("OS motion: unavailable"));
    assert!(!report.contains("OS contrast: unavailable"));
}

#[test]
fn the_report_lists_the_focus_order_and_reflow_verdict_per_pane() {
    let report = render_accessibility_setup_report(
        Some(&two_monitor_bridge()),
        &OsAccessibilityPrefs::default(),
    );
    assert!(report.contains("Keyboard focus order across 4 pane(s)"));
    for label in ["Ada", "Grace", "Hopper", "Lovelace"] {
        assert!(report.contains(label), "missing pane {label} in report");
    }
    // Every pane on these supported monitors is preserved to the maximum.
    assert!(report.contains("preserved to the supported maximum"));
    assert!(!report.contains("TOO SMALL"));
}

#[test]
fn the_report_flags_a_pane_too_small_at_the_maximum() {
    let small = bridge(vec![station(
        "small@640x480",
        geometry(640, 480, 0, 0, 1.0),
        PaneSplit::SideBySide,
        &["Ada", "Grace"],
    )]);
    let report = render_accessibility_setup_report(Some(&small), &OsAccessibilityPrefs::default());
    assert!(report.contains("TOO SMALL"));
}

#[test]
fn the_report_says_when_a_resolved_bridge_has_no_panes() {
    let vs_only = bridge(vec![viewscreen("vs", geometry(1920, 1080, 0, 0, 1.0))]);
    let report =
        render_accessibility_setup_report(Some(&vs_only), &OsAccessibilityPrefs::default());
    assert!(report.contains("No Station panes configured"));
}

#[test]
fn the_report_is_unbothered_by_resolution_problems() {
    // A resolved bridge that also carries problems still reports its panes.
    let mut b = two_monitor_bridge();
    b.problems.push(ProfileProblem::MonitorUnassigned {
        id: MonitorIdentity::new("spare@1024x768"),
        name: None,
    });
    let report = render_accessibility_setup_report(Some(&b), &OsAccessibilityPrefs::default());
    assert!(report.contains("Keyboard focus order across 4 pane(s)"));
}
