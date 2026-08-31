//! Synthetic coverage for the pure setup/layout accessibility model (issue
//! #1128, acceptance criterion 5): the reflow-headroom check at the supported
//! scaling extremes for one- and two-pane layouts, the keyboard-focus order
//! across monitors and split panes, the non-colour focus-reticle geometry and
//! its contrast/reduced-motion response, and the setup-action reachability
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
    assert_eq!(SUPPORTED_TEXT_SCALE_MAX, 1.5);
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
fn the_check_rejects_a_pane_too_small_at_the_maximum() {
    // The predicate must be a real gate, not vacuously true: a two-pane
    // side-by-side split on a small 640×480 monitor gives each pane a 320-wide
    // box, which the maximum text scale (×1.5 → 480 demand) no longer holds.
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
        "must fail at 1.5x: {b:?}"
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
    let mut ring = FocusRing::focused_on_first(ids.clone());
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

// ── the non-colour focus indicator (acceptance criteria 3 & 4) ───────────────

#[test]
fn the_reticle_is_a_frame_plus_four_corner_brackets() {
    // AC3: focus is shown by shape — a full frame AND four corner brackets — not
    // by colour. The count is fixed at four corners.
    assert_eq!(FocusReticle::BRACKET_CORNERS, 4);
    let style = FocusReticleStyle::standard();
    let r = FocusReticle::for_pane(0.0, 0.0, 1920.0, 1080.0, &style);
    // The frame is inset on every side, and it is a real rectangle (a frame),
    // distinct from the pane it sits in.
    assert_eq!(r.frame.left, style.inset_px);
    assert_eq!(r.frame.top, style.inset_px);
    assert_eq!(r.frame.width, 1920.0 - 2.0 * style.inset_px);
    assert_eq!(r.frame.height, 1080.0 - 2.0 * style.inset_px);
    assert_eq!(r.bracket_px, FocusReticleStyle::BRACKET_PX);
    assert!(r.thickness_px > 0.0);
}

#[test]
fn a_tiny_pane_yields_a_non_negative_frame() {
    // Smaller than twice the inset: the frame clamps to zero rather than going
    // negative, matching the adapter's `.max(0.0)`.
    let r = FocusReticle::for_pane(0.0, 0.0, 1.0, 1.0, &FocusReticleStyle::standard());
    assert!(r.frame.width >= 0.0);
    assert!(r.frame.height >= 0.0);
}

#[test]
fn high_contrast_bolds_and_opaques_the_reticle_without_making_colour_load_bearing() {
    // AC4: the contrast preference applies to host-drawn setup chrome — the
    // reticle gets a thicker, fully opaque frame. Focus is STILL conveyed by the
    // frame's presence, so the standard reticle remains a valid focus cue too.
    let standard = FocusReticleStyle::for_prefs(false, false);
    let contrast = FocusReticleStyle::for_prefs(true, false);
    assert_eq!(
        standard.thickness_px,
        FocusReticleStyle::STANDARD_THICKNESS_PX
    );
    assert_eq!(standard.alpha, FocusReticleStyle::STANDARD_ALPHA);
    assert!(contrast.thickness_px > standard.thickness_px);
    assert_eq!(contrast.alpha, FocusReticleStyle::CONTRAST_ALPHA);
    assert!(contrast.alpha >= standard.alpha);
}

#[test]
fn the_reticle_never_animates_so_reduced_motion_is_satisfied_by_construction() {
    // AC4: reduced motion applies to host-drawn setup chrome. The reticle is
    // static — spawned/despawned as focus moves, no transition — so it satisfies
    // reduced motion whether the preference is set or not. `for_prefs` consults
    // the flag either way (no panic, same static result).
    for reduced in [false, true] {
        let style = FocusReticleStyle::for_prefs(false, reduced);
        assert!(!style.animates());
    }
}

#[test]
fn the_reticle_style_reads_a_whole_os_prefs() {
    let prefs = OsAccessibilityPrefs {
        reduced_motion: true,
        high_contrast: true,
        text_scale: 1.25,
    };
    let style = FocusReticleStyle::for_os_prefs(&prefs);
    assert_eq!(style.thickness_px, FocusReticleStyle::CONTRAST_THICKNESS_PX);
    assert!(!style.animates());
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
    };
    let report = render_accessibility_setup_report(None, &prefs);
    assert!(report.contains("Accessibility:"));
    assert!(report.contains("contrast on"));
    assert!(report.contains("reduced motion on"));
    assert!(report.contains("1x to 1.5x"));
    assert!(report.contains("No profile resolved"));
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
