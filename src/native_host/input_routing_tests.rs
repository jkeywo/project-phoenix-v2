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
