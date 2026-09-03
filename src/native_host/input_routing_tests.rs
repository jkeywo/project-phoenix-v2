//! Synthetic coverage for the pure input-routing model (issue #1124,
//! acceptance criterion 5): the coordinate transforms, the pane-boundary hit
//! test, the contact-capture map, mouse traversal across surfaces and the
//! keyboard-focus transitions — all with no real hardware, which is the whole
//! point of keeping the model Bevy-free.

use super::*;
use crate::native_host::bridge_profile::{pane_rects, MonitorGeometry, PaneSplit};

fn pane(n: u32) -> PaneId {
    PaneId(n)
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> PaneRect {
    PaneRect {
        x,
        y,
        width,
        height,
    }
}

/// One window at the desktop origin, whose panes tile it left-to-right — the
/// single-window (`--pane` with no `--profile`) host.
fn single_window(rects: &[(PaneId, PaneRect)], scale: f64) -> PaneRouter {
    PaneRouter::new(
        rects
            .iter()
            .map(|(id, r)| PanePlacement {
                pane: *id,
                window: WindowKey(1),
                window_origin_x: 0,
                window_origin_y: 0,
                rect: *r,
                scale_factor: scale,
            })
            .collect(),
    )
}

// ── coordinate transforms (AC3: display scaling) ─────────────────────────────

#[test]
fn a_point_maps_to_pane_local_logical_pixels_at_scale_one() {
    // A single 1920×1080 pane at scale 1: physical equals logical, and the
    // origin offset is zero, so the coordinate passes straight through.
    let router = single_window(&[(pane(0), rect(0, 0, 1920, 1080))], 1.0);
    assert_eq!(
        router.resolve_in_window(WindowKey(1), 640.0, 360.0),
        Some(PaneHit {
            pane: pane(0),
            local_x: 640,
            local_y: 360,
        })
    );
}

#[test]
fn display_scaling_divides_the_physical_offset_by_the_scale_factor() {
    // The whole of "display scaling" in AC3: a 3840×2160 pane at 2.0 scale is a
    // 1920×1080 page, so a physical point halves into its logical coordinate.
    let router = single_window(&[(pane(0), rect(0, 0, 3840, 2160))], 2.0);
    assert_eq!(
        router.resolve_in_window(WindowKey(1), 1000.0, 500.0),
        Some(PaneHit {
            pane: pane(0),
            local_x: 500,
            local_y: 250,
        })
    );
}

#[test]
fn a_fractional_scale_factor_still_lands_in_the_pane() {
    // 1.5 is the common Windows scale. The division is honest floating point,
    // truncated to the integer pixel Ultralight takes.
    let router = single_window(&[(pane(0), rect(0, 0, 2880, 1620))], 1.5);
    let hit = router
        .resolve_in_window(WindowKey(1), 300.0, 150.0)
        .unwrap();
    assert_eq!(hit.pane, pane(0));
    assert_eq!(hit.local_x, 200); // 300 / 1.5
    assert_eq!(hit.local_y, 100); // 150 / 1.5
}

#[test]
fn a_pane_offset_within_its_window_subtracts_its_origin_first() {
    // The right pane of a side-by-side split starts at x=960; a window point at
    // x=1000 is 40 physical pixels into that pane, not 1000.
    let router = single_window(
        &[
            (pane(0), rect(0, 0, 960, 1080)),
            (pane(1), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    assert_eq!(
        router.resolve_in_window(WindowKey(1), 1000.0, 20.0),
        Some(PaneHit {
            pane: pane(1),
            local_x: 40,
            local_y: 20,
        })
    );
}

// ── pane boundaries (AC1/AC3: which pane owns a point) ───────────────────────

#[test]
fn the_shared_boundary_between_two_tiled_panes_belongs_to_the_second() {
    // Low edge inclusive, high edge exclusive: the seam pixel is the right
    // pane's, so no point is ever claimed by two panes and none falls in the gap.
    let router = single_window(
        &[
            (pane(0), rect(0, 0, 960, 1080)),
            (pane(1), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    // Just left of the seam is the left pane…
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 959.0, 10.0)
            .unwrap()
            .pane,
        pane(0)
    );
    // …the seam pixel itself is the right pane…
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 960.0, 10.0)
            .unwrap()
            .pane,
        pane(1)
    );
    // …and one physical pixel more is still the right pane.
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 961.0, 10.0)
            .unwrap()
            .pane,
        pane(1)
    );
}

#[test]
fn a_point_past_the_far_edge_hits_no_pane() {
    let router = single_window(&[(pane(0), rect(0, 0, 1920, 1080))], 1.0);
    assert_eq!(router.resolve_in_window(WindowKey(1), 1920.0, 0.0), None);
    assert_eq!(router.resolve_in_window(WindowKey(1), 0.0, 1080.0), None);
    assert_eq!(router.resolve_in_window(WindowKey(1), -1.0, -1.0), None);
}

#[test]
fn a_side_by_side_split_routes_by_the_profiles_own_geometry() {
    // The router is fed the exact rectangles bridge_profile computes, so the
    // boundary here is the profile's boundary — no second tiling rule to drift.
    let geometry = MonitorGeometry {
        physical_width: 1920,
        physical_height: 1080,
        position_x: 0,
        position_y: 0,
        scale_factor: 1.0,
    };
    let rects = pane_rects(&geometry, PaneSplit::SideBySide, 2);
    let router = single_window(&[(pane(0), rects[0]), (pane(1), rects[1])], 1.0);
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 10.0, 540.0)
            .unwrap()
            .pane,
        pane(0)
    );
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 1900.0, 540.0)
            .unwrap()
            .pane,
        pane(1)
    );
}

#[test]
fn a_stacked_split_routes_top_and_bottom() {
    let geometry = MonitorGeometry {
        physical_width: 1920,
        physical_height: 1080,
        position_x: 0,
        position_y: 0,
        scale_factor: 1.0,
    };
    let rects = pane_rects(&geometry, PaneSplit::Stacked, 2);
    let router = single_window(&[(pane(0), rects[0]), (pane(1), rects[1])], 1.0);
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 960.0, 10.0)
            .unwrap()
            .pane,
        pane(0)
    );
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 960.0, 1070.0)
            .unwrap()
            .pane,
        pane(1)
    );
}

// ── mouse traversal across surfaces (AC1) ────────────────────────────────────

#[test]
fn a_mouse_path_crossing_a_pane_boundary_changes_the_pane_it_reports() {
    // The traversal claim, as a sequence: as the pointer walks left to right it
    // reports the left pane, then the right, and the transition is exactly at the
    // seam — one pointer operating both panes with no mode switch.
    let router = single_window(
        &[
            (pane(0), rect(0, 0, 960, 1080)),
            (pane(1), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    let panes: Vec<Option<PaneId>> = (0..2000)
        .step_by(200)
        .map(|x| {
            router
                .resolve_in_window(WindowKey(1), x as f64, 500.0)
                .map(|h| h.pane)
        })
        .collect();
    assert_eq!(
        panes,
        vec![
            Some(pane(0)), // 0
            Some(pane(0)), // 200
            Some(pane(0)), // 400
            Some(pane(0)), // 600
            Some(pane(0)), // 800
            Some(pane(1)), // 1000
            Some(pane(1)), // 1200
            Some(pane(1)), // 1400
            Some(pane(1)), // 1600
            Some(pane(1)), // 1800
        ]
    );
}

#[test]
fn an_event_is_routed_only_among_its_own_windows_panes() {
    // Two Station windows, each with its own pane. A point that would be inside
    // window 1's pane geometry must NOT resolve when it is delivered as window
    // 2's event — winit reports against the window it happened on, and this is
    // what stops a mouse on Station 2 reaching a pane on Station 1.
    let router = PaneRouter::new(vec![
        PanePlacement {
            pane: pane(0),
            window: WindowKey(10),
            window_origin_x: 0,
            window_origin_y: 0,
            rect: rect(0, 0, 1920, 1080),
            scale_factor: 1.0,
        },
        PanePlacement {
            pane: pane(1),
            window: WindowKey(20),
            window_origin_x: 1920,
            window_origin_y: 0,
            rect: rect(0, 0, 1920, 1080),
            scale_factor: 1.0,
        },
    ]);
    assert_eq!(
        router
            .resolve_in_window(WindowKey(10), 100.0, 100.0)
            .unwrap()
            .pane,
        pane(0)
    );
    assert_eq!(
        router
            .resolve_in_window(WindowKey(20), 100.0, 100.0)
            .unwrap()
            .pane,
        pane(1)
    );
}

// ── desktop-global resolution (AC3: physical coords → the correct monitor) ───

#[test]
fn a_desktop_point_maps_to_the_monitor_covering_it_and_then_the_pane() {
    // Two monitors side by side on the virtual desktop; the second at x=1920.
    // A desktop point at x=2880 is 960 into the second monitor, whose right pane
    // starts there — so it resolves to that pane at local x 0.
    let router = PaneRouter::new(vec![
        PanePlacement {
            pane: pane(0),
            window: WindowKey(10),
            window_origin_x: 0,
            window_origin_y: 0,
            rect: rect(0, 0, 1920, 1080),
            scale_factor: 1.0,
        },
        PanePlacement {
            pane: pane(1),
            window: WindowKey(20),
            window_origin_x: 1920,
            window_origin_y: 0,
            rect: rect(0, 0, 960, 1080),
            scale_factor: 1.0,
        },
        PanePlacement {
            pane: pane(2),
            window: WindowKey(20),
            window_origin_x: 1920,
            window_origin_y: 0,
            rect: rect(960, 0, 960, 1080),
            scale_factor: 1.0,
        },
    ]);
    // First monitor.
    assert_eq!(router.resolve_desktop(100.0, 100.0).unwrap().pane, pane(0));
    // Second monitor, left pane.
    assert_eq!(router.resolve_desktop(2000.0, 100.0).unwrap().pane, pane(1));
    // Second monitor, right pane — 2880 desktop is 960 into the monitor, local 0.
    assert_eq!(
        router.resolve_desktop(2880.0, 100.0),
        Some(PaneHit {
            pane: pane(2),
            local_x: 0,
            local_y: 100,
        })
    );
}

#[test]
fn a_desktop_point_honours_a_scaled_monitor_at_a_nonzero_origin() {
    // A 2.0-scale monitor placed to the right: the desktop offset into it is
    // physical, and the logical coordinate halves it, so scaling and position are
    // handled together.
    let router = PaneRouter::new(vec![PanePlacement {
        pane: pane(0),
        window: WindowKey(20),
        window_origin_x: 1920,
        window_origin_y: 0,
        rect: rect(0, 0, 3840, 2160),
        scale_factor: 2.0,
    }]);
    assert_eq!(
        router.resolve_desktop(1920.0 + 1000.0, 500.0),
        Some(PaneHit {
            pane: pane(0),
            local_x: 500,
            local_y: 250,
        })
    );
}

#[test]
fn a_desktop_point_resolves_a_monitor_left_of_the_primary_at_a_negative_origin() {
    // Windows numbers a secondary monitor to the LEFT of the primary with a
    // negative desktop x: its top-left is at x=-1920. A desktop point at x=-1000
    // is 920 physical pixels into that monitor, so it must resolve to its pane at
    // local x 920 — resolve_desktop's subtraction handles the negative origin
    // with no special case (issue #1124, finding: negative-origin coverage).
    let router = PaneRouter::new(vec![
        PanePlacement {
            pane: pane(0),
            window: WindowKey(10),
            window_origin_x: -1920,
            window_origin_y: 0,
            rect: rect(0, 0, 1920, 1080),
            scale_factor: 1.0,
        },
        PanePlacement {
            pane: pane(1),
            window: WindowKey(20),
            window_origin_x: 0,
            window_origin_y: 0,
            rect: rect(0, 0, 1920, 1080),
            scale_factor: 1.0,
        },
    ]);
    assert_eq!(
        router.resolve_desktop(-1000.0, 100.0),
        Some(PaneHit {
            pane: pane(0),
            local_x: 920, // -1000 − (−1920)
            local_y: 100,
        })
    );
    // The primary at the origin still resolves for a positive point.
    assert_eq!(router.resolve_desktop(100.0, 100.0).unwrap().pane, pane(1));
    // The seam between the two monitors is the primary's low edge (inclusive).
    assert_eq!(router.resolve_desktop(0.0, 100.0).unwrap().pane, pane(1));
    assert_eq!(router.resolve_desktop(-1.0, 100.0).unwrap().pane, pane(0));
}

#[test]
fn a_desktop_point_resolves_a_monitor_above_the_primary_at_a_negative_y() {
    // A monitor stacked ABOVE the primary has a negative desktop y: its top-left
    // is at y=-1080. A point at y=-800 is 280 down into it, so it resolves to its
    // pane at local y 280.
    let router = PaneRouter::new(vec![PanePlacement {
        pane: pane(0),
        window: WindowKey(30),
        window_origin_x: 0,
        window_origin_y: -1080,
        rect: rect(0, 0, 1920, 1080),
        scale_factor: 1.0,
    }]);
    assert_eq!(
        router.resolve_desktop(500.0, -800.0),
        Some(PaneHit {
            pane: pane(0),
            local_x: 500,
            local_y: 280, // -800 − (−1080)
        })
    );
}

#[test]
fn focus_order_is_placement_order() {
    let router = single_window(
        &[
            (pane(2), rect(0, 0, 960, 1080)),
            (pane(5), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    assert_eq!(router.focus_order(), vec![pane(2), pane(5)]);
}

// ── keyboard focus transitions (AC2) ─────────────────────────────────────────

#[test]
fn focus_next_from_nothing_lands_on_the_first_pane_and_then_cycles() {
    let mut ring = FocusRing::from_order(vec![pane(0), pane(1), pane(2)]);
    assert_eq!(ring.focused(), None);
    assert_eq!(ring.focus_next(), Some(pane(0)));
    assert_eq!(ring.focus_next(), Some(pane(1)));
    assert_eq!(ring.focus_next(), Some(pane(2)));
    assert_eq!(ring.focus_next(), Some(pane(0)), "traversal wraps");
}

#[test]
fn focus_prev_from_nothing_lands_on_the_last_pane_and_then_cycles_backward() {
    let mut ring = FocusRing::from_order(vec![pane(0), pane(1), pane(2)]);
    assert_eq!(ring.focus_prev(), Some(pane(2)));
    assert_eq!(ring.focus_prev(), Some(pane(1)));
    assert_eq!(ring.focus_prev(), Some(pane(0)));
    assert_eq!(ring.focus_prev(), Some(pane(2)), "traversal wraps backward");
}

#[test]
fn pointing_at_a_pane_focuses_it_and_a_repeat_reports_no_change() {
    let mut ring = FocusRing::from_order(vec![pane(0), pane(1)]);
    assert!(ring.focus(pane(1)), "focus moved");
    assert_eq!(ring.focused(), Some(pane(1)));
    assert!(!ring.focus(pane(1)), "already there, no change");
    assert!(ring.focus(pane(0)), "moved again");
}

#[test]
fn focusing_a_pane_that_is_not_placed_is_refused() {
    let mut ring = FocusRing::from_order(vec![pane(0)]);
    assert!(!ring.focus(pane(9)));
    assert_eq!(ring.focused(), None);
}

#[test]
fn a_closed_focused_pane_is_cleared_not_carried_onto_its_successor() {
    // The subtle one: pane 1 held focus and closes. Reconciling to the surviving
    // panes must drop focus rather than leave it pointing at a pane id that now
    // means whoever took the slot — that would send keystrokes to the wrong
    // participant.
    let mut ring = FocusRing::from_order(vec![pane(0), pane(1), pane(2)]);
    ring.focus(pane(1));
    ring.sync_order(vec![pane(0), pane(2)]);
    assert_eq!(ring.focused(), None);
    assert_eq!(ring.order(), &[pane(0), pane(2)]);
}

#[test]
fn a_still_focused_pane_survives_a_layout_change_that_keeps_it() {
    let mut ring = FocusRing::from_order(vec![pane(0), pane(1), pane(2)]);
    ring.focus(pane(2));
    ring.sync_order(vec![pane(2), pane(0)]);
    assert_eq!(
        ring.focused(),
        Some(pane(2)),
        "kept because it is still present"
    );
}

#[test]
fn an_empty_ring_traverses_to_nothing() {
    let mut ring = FocusRing::new();
    assert_eq!(ring.focus_next(), None);
    assert_eq!(ring.focus_prev(), None);
    assert_eq!(ring.focused(), None);
}

#[test]
fn a_freshly_built_ring_seeds_focus_onto_its_first_pane() {
    // Finding: a freshly-built pane host must start with a focused pane, so a
    // pure-keyboard operator sees the reticle and has a defined target the moment
    // panes exist — not a blank ring in which keys go nowhere until the first
    // Ctrl+Tab. `from_order` (nothing focused) was the defect;
    // `focused_on_first_pane` is the seed.
    let ring = FocusRing::focused_on_first_pane(vec![pane(3), pane(7)]);
    assert_eq!(
        ring.focused(),
        Some(pane(3)),
        "the first pane holds focus the instant the host is built"
    );
    assert_eq!(ring.order(), &[pane(3), pane(7)]);
}

#[test]
fn a_freshly_built_ring_with_no_panes_focuses_nothing() {
    let ring = FocusRing::focused_on_first_pane(vec![]);
    assert_eq!(ring.focused(), None);
}

#[test]
fn a_ring_holding_only_the_host_lobby_surface_seeds_nothing() {
    // Finding (issue #1325): `phoenix-host --client-dir dist --world <w>` with no
    // `--pane` builds a router whose ONLY entry is the lobby surface. Seeding the
    // first *entry* focused it, and the adapter then drew a whole-primary-window
    // focus frame and corner brackets over chrome that has no typeable control —
    // a reticle advertising a keyboard target that accepts nothing. The seed
    // skips the surface; the honest initial state is no focus and no reticle.
    let ring = FocusRing::focused_on_first_pane(vec![HOST_LOBBY_SURFACE_ID]);
    assert_eq!(
        ring.focused(),
        None,
        "the lobby surface is never the seeded focus"
    );
    assert_eq!(
        ring.order(),
        &[HOST_LOBBY_SURFACE_ID],
        "but it stays in the order, so a deliberate Ctrl+Tab still reaches it"
    );
}

#[test]
fn seeding_skips_the_lobby_surface_and_lands_on_the_first_real_pane() {
    // The surface is placed LAST in the router, so this is the ordinary
    // `--pane` host; the guard also holds if a later layout ever puts it first.
    let ring = FocusRing::focused_on_first_pane(vec![HOST_LOBBY_SURFACE_ID, pane(4), pane(9)]);
    assert_eq!(ring.focused(), Some(pane(4)));

    let ring = FocusRing::focused_on_first_pane(vec![pane(4), pane(9), HOST_LOBBY_SURFACE_ID]);
    assert_eq!(ring.focused(), Some(pane(4)));
}

#[test]
fn the_lobby_surface_is_still_reachable_by_keyboard_traversal() {
    // Excluding it from the SEED must not exclude it from the CYCLE: acceptance
    // criterion 5 is that a keyboard operates every surface, and later slices put
    // real controls on this one.
    let mut ring = FocusRing::focused_on_first_pane(vec![pane(0), HOST_LOBBY_SURFACE_ID]);
    assert_eq!(ring.focused(), Some(pane(0)));
    assert_eq!(ring.focus_next(), Some(HOST_LOBBY_SURFACE_ID));
    assert_eq!(ring.focus_next(), Some(pane(0)));
}

#[test]
fn a_resting_pointer_does_not_revert_a_keyboard_focus_change() {
    // Finding (HIGH): focus-follows-pointer must fire on genuine pointer MOTION,
    // not on the cursor merely being present over a pane every frame — otherwise
    // a Ctrl+Tab selection is reverted the next frame whenever the mouse rests
    // over another pane, defeating AC2's predictable keyboard traversal. This
    // exercises `pointer_follow_focus`, the exact rule the adapter applies, so
    // reverting the fix (following on presence) fails here.
    let router = single_window(
        &[
            (pane(0), rect(0, 0, 960, 1080)),
            (pane(1), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    let mut focus = FocusRing::from_order(router.focus_order());
    let mut motion = PointerMotion::new();
    let win = WindowKey(1);

    // One pointer sample, exactly as route_pointer_input performs it: resolve the
    // pane under the physical point, then let focus follow only on motion.
    let sample = |focus: &mut FocusRing, motion: &mut PointerMotion, x: f64, y: f64| {
        let hit = router.resolve_in_window(win, x, y).unwrap();
        pointer_follow_focus(focus, motion, win, x, y, hit.pane)
    };

    // The pointer first appears over pane 0 → focus follows to it.
    assert!(sample(&mut focus, &mut motion, 100.0, 100.0));
    assert_eq!(focus.focused(), Some(pane(0)));

    // Ctrl+Tab moves focus to pane 1 (keyboard traversal, not the pointer).
    focus.focus_next();
    assert_eq!(focus.focused(), Some(pane(1)));

    // A frame in which the cursor RESTS over pane 0 (same position, no motion):
    // the presence must NOT steal focus back to pane 0.
    assert!(!sample(&mut focus, &mut motion, 100.0, 100.0));
    assert_eq!(
        focus.focused(),
        Some(pane(1)),
        "a resting pointer must not revert the keyboard's focus selection"
    );

    // A genuine move into pane 0 DOES pull focus back — focus follows real motion.
    assert!(sample(&mut focus, &mut motion, 130.0, 140.0));
    assert_eq!(
        focus.focused(),
        Some(pane(0)),
        "a real pointer move into a pane follows the pointer to it"
    );
}

#[test]
fn pointer_motion_reports_presence_versus_movement_per_window() {
    // The primitive under the focus rule: a first sample and any change are
    // motion; a repeated identical sample is the resting cursor. Tracked per
    // window, so the cursor moving between two windows is motion in each.
    let mut motion = PointerMotion::new();
    let a = WindowKey(1);
    let b = WindowKey(2);
    assert!(
        motion.moved(a, 10.0, 10.0),
        "first sample in a window is motion"
    );
    assert!(
        !motion.moved(a, 10.0, 10.0),
        "the same position is presence"
    );
    assert!(motion.moved(a, 11.0, 10.0), "a changed position is motion");
    assert!(
        motion.moved(b, 10.0, 10.0),
        "a different window is its own first sample"
    );
    assert!(!motion.moved(b, 10.0, 10.0));
}

// ── touch contact capture (AC4) ──────────────────────────────────────────────

#[test]
fn a_contact_stays_pinned_to_its_starting_pane_when_the_finger_drifts_away() {
    // The capture claim: a gesture that began on pane 0 is routed to pane 0 for
    // its whole life, even after the finger has physically moved into pane 1's
    // region. The router would resolve the drifted point to pane 1; the capture
    // map overrides that.
    let router = single_window(
        &[
            (pane(0), rect(0, 0, 960, 1080)),
            (pane(1), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    let mut contacts = ContactCaptureMap::new();

    // Started on the left pane.
    let start = router
        .resolve_in_window(WindowKey(1), 100.0, 100.0)
        .unwrap();
    assert_eq!(start.pane, pane(0));
    assert!(contacts.start(7, start.pane));

    // The finger drifts into the right pane's region. The router alone would say
    // pane 1…
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 1400.0, 100.0)
            .unwrap()
            .pane,
        pane(1)
    );
    // …but the contact is captured by pane 0, and the move routes there.
    assert_eq!(contacts.pane_for(7), Some(pane(0)));

    // Lift releases it, and its final event routes to pane 0 too.
    assert_eq!(contacts.end(7), Some(pane(0)));
    assert_eq!(contacts.pane_for(7), None);
    assert!(contacts.is_empty());
}

#[test]
fn a_drifted_contact_projects_past_its_pinned_panes_edge() {
    // The capture path's coordinate: a finger pinned to the left pane that has
    // slid into the right pane's region projects to a local x PAST the left
    // pane's width — a legitimate drag beyond the edge, not clamped and not lost.
    let router = single_window(
        &[
            (pane(0), rect(0, 0, 960, 1080)),
            (pane(1), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    // Pinned to pane 0, finger now at window x=1400.
    assert_eq!(
        router.project_into_pane(pane(0), 1400.0, 100.0),
        Some((1400, 100))
    );
    // A pane that is not placed projects to nothing.
    assert_eq!(router.project_into_pane(pane(9), 10.0, 10.0), None);
}

#[test]
fn a_left_drag_keeps_its_down_and_up_on_the_pane_it_began_on() {
    // Finding (MEDIUM): the mouse mirror of touch capture. A left press in pane 0
    // that drifts into pane 1's region and releases there must deliver BOTH its
    // down and its up to pane 0 — never a down to pane 0 and an unmatched up to
    // pane 1, which leaves pane 0 stuck in a pressed/selecting state and gives
    // pane 1 a stray release. Exercises `MouseCapture` + `project_into_pane`, the
    // exact pieces the adapter uses, so reverting the capture fails here.
    let router = single_window(
        &[
            (pane(0), rect(0, 0, 960, 1080)),
            (pane(1), rect(960, 0, 960, 1080)),
        ],
        1.0,
    );
    let mut capture = MouseCapture::new();

    // Press in pane 0: the button captures the pane it went down on.
    let down = router
        .resolve_in_window(WindowKey(1), 100.0, 100.0)
        .unwrap();
    assert_eq!(down.pane, pane(0));
    assert!(
        capture.press(down.pane),
        "the left button captures the pressed pane"
    );
    assert_eq!(capture.captured(), Some(pane(0)));

    // The cursor drifts into pane 1's region. The router alone would say pane 1…
    assert_eq!(
        router
            .resolve_in_window(WindowKey(1), 1400.0, 120.0)
            .unwrap()
            .pane,
        pane(1)
    );
    // …but the capture pins the drag to pane 0, and the move projects PAST pane
    // 0's edge rather than crossing to pane 1 — a legitimate drag-beyond-edge.
    assert_eq!(capture.captured(), Some(pane(0)));
    assert_eq!(
        router.project_into_pane(pane(0), 1400.0, 120.0),
        Some((1400, 120))
    );

    // Release: the up routes to pane 0 and the capture clears. Pane 1 never saw a
    // down and never sees an up.
    assert_eq!(capture.release(), Some(pane(0)));
    assert_eq!(capture.captured(), None);
}

#[test]
fn a_second_press_does_not_rehome_a_live_mouse_capture() {
    // A press with no intervening release is a driver quirk, not a reason to move
    // the capture off the pane the drag began on — the same rule the touch map
    // enforces for a duplicate `Started`.
    let mut capture = MouseCapture::new();
    assert!(capture.press(pane(0)));
    assert!(
        !capture.press(pane(1)),
        "a live capture keeps its original pane"
    );
    assert_eq!(capture.captured(), Some(pane(0)));
    assert_eq!(capture.release(), Some(pane(0)));
    assert_eq!(capture.release(), None, "releasing again captures nothing");
}

#[test]
fn simultaneous_contacts_on_different_screens_are_independent() {
    // Two fingers, two contact ids, two panes on two windows. Each routes to its
    // own pane and neither interferes with the other — the acceptance criterion's
    // "simultaneous contacts on different screens remain independent".
    let mut contacts = ContactCaptureMap::new();
    assert!(contacts.start(1, pane(0)));
    assert!(contacts.start(2, pane(3)));
    assert_eq!(contacts.len(), 2);
    assert_eq!(contacts.pane_for(1), Some(pane(0)));
    assert_eq!(contacts.pane_for(2), Some(pane(3)));

    // Lifting one leaves the other untouched.
    assert_eq!(contacts.end(1), Some(pane(0)));
    assert_eq!(contacts.pane_for(2), Some(pane(3)));
    assert_eq!(contacts.len(), 1);
}

#[test]
fn a_duplicate_start_does_not_rehome_a_live_contact() {
    let mut contacts = ContactCaptureMap::new();
    assert!(contacts.start(1, pane(0)));
    assert!(
        !contacts.start(1, pane(1)),
        "a live id keeps its original pane"
    );
    assert_eq!(contacts.pane_for(1), Some(pane(0)));
}

#[test]
fn an_end_for_an_unknown_contact_is_none() {
    let mut contacts = ContactCaptureMap::new();
    assert_eq!(contacts.end(99), None);
}

#[test]
fn closing_a_pane_releases_the_contacts_it_had_captured() {
    // A pane closing under a finger must not leave that contact pinned to a view
    // that has gone. The released ids come back so the adapter can synthesise the
    // lift.
    let mut contacts = ContactCaptureMap::new();
    contacts.start(1, pane(0));
    contacts.start(2, pane(0));
    contacts.start(3, pane(1));
    let mut released = contacts.release_pane(pane(0));
    released.sort_unstable();
    assert_eq!(released, vec![1, 2]);
    assert_eq!(contacts.pane_for(1), None);
    assert_eq!(
        contacts.pane_for(3),
        Some(pane(1)),
        "another pane's contact is untouched"
    );
}
