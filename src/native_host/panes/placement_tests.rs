use super::*;
use crate::native_host::bridge_display::{BridgeStationSurface, StationPane};
use crate::native_host::bridge_profile::{MonitorGeometry, PaneRect};

const BENQ: &str = "BenQ EX@1920x1080";

fn station(id: &str) -> StationId {
    StationId(id.to_string())
}

fn geometry() -> MonitorGeometry {
    MonitorGeometry {
        physical_width: 1920,
        physical_height: 1080,
        position_x: 3840,
        position_y: 0,
        scale_factor: 1.5,
    }
}

/// A Station surface on the BenQ carrying one slot for `label`, seated for
/// `station` when the layout placed it.
fn surfaces_with(label: &str, station_of: Option<StationId>) -> BridgeStationSurfaces {
    BridgeStationSurfaces(vec![BridgeStationSurface {
        identity: BENQ.to_string(),
        window: Entity::from_raw_u32(7).expect("a test window handle"),
        monitor: Entity::PLACEHOLDER,
        geometry: geometry(),
        panes: vec![StationPane {
            label: label.to_string(),
            rect: PaneRect {
                x: 960,
                y: 0,
                width: 960,
                height: 1080,
            },
            station: station_of,
        }],
    }])
}

/// A layout over one screen with `helm` seated on it.
fn layout_seating_helm() -> BridgeLayout {
    use crate::native_host::bridge_profile::{DiscoveredMonitor, MonitorIdentity};
    let discovered = vec![
        DiscoveredMonitor {
            identity: MonitorIdentity::new("DELL U2720Q@3840x2160"),
            geometry: MonitorGeometry {
                physical_width: 3840,
                physical_height: 2160,
                position_x: 0,
                position_y: 0,
                scale_factor: 1.0,
            },
            name: Some("DELL U2720Q".to_string()),
            primary: true,
        },
        DiscoveredMonitor {
            identity: MonitorIdentity::new(BENQ),
            geometry: geometry(),
            name: Some("BenQ EX".to_string()),
            primary: false,
        },
    ];
    let seeded = BridgeLayout::from_discovered(&discovered, [station("helm")])
        .expect("two screens and a one-station roster seed a layout");
    seeded
        .apply(
            &crate::native_host::bridge_layout::LayoutAction::AssignStation {
                station: station("helm"),
                monitor: MonitorIdentity::new(BENQ),
            },
        )
        .expect("seating helm on the BenQ is lawful")
}

fn helm_tile() -> Vec<PaneTile> {
    vec![PaneTile {
        name: "helm".to_string(),
        origin: (0, 0),
        size: (1280, 720),
    }]
}

#[test]
fn a_console_with_a_live_station_slot_is_built_on_that_station_window() {
    // Rule 1: the slot wins, and it carries the whole seat — the window, the
    // rectangle the layout gives it NOW, the monitor's scale, and the
    // window's own corner on the virtual desktop.
    let surfaces = surfaces_with("helm", Some(station("helm")));
    let home = home_for_pane(
        "helm",
        Some(&surfaces),
        Some(&layout_seating_helm()),
        &helm_tile(),
    );
    assert_eq!(
        home,
        PaneHome::Station {
            window: Entity::from_raw_u32(7).unwrap(),
            origin: (960, 0),
            size: (960, 1080),
            scale: 1.5,
            window_origin: (3840, 0),
        },
        "even with a stored tile sitting right there, the seat wins"
    );
}

#[test]
fn a_seated_console_with_no_slot_is_left_unbuilt_rather_than_tiled_on_the_viewscreen() {
    // Rule 2, and the whole point of issue #1333. The law still seats helm —
    // the operator's row says so — but the adapter has no slot for it this
    // frame (a monitor between hot-plug frames, a Station window not yet
    // rebuilt). A tile is deliberately present and deliberately NOT used:
    // rebuilding a wall console over the shared view is the failure this
    // rule exists to make impossible.
    let home = home_for_pane(
        "helm",
        Some(&BridgeStationSurfaces::default()),
        Some(&layout_seating_helm()),
        &helm_tile(),
    );
    assert_eq!(home, PaneHome::Nowhere(NoHome::SeatedButUnplaced));
    assert!(
        !matches!(home, PaneHome::PrimaryTile { .. }),
        "never the viewscreen"
    );
    assert!(
        NoHome::SeatedButUnplaced.should_retry(),
        "and not built is not forgotten: this one is faulted so it is retried"
    );
}

#[test]
fn only_a_console_a_rebuild_could_land_on_is_retried() {
    // The fault-or-skip decision, pinned per reason in the half CI actually
    // compiles — the one part of this rule the adapter could still get wrong
    // on its own. `open_pending_views` has already DRAINED the pending-view
    // entry by the time it asks, so a `Nowhere` it merely skips is a pane the
    // bus still lists as open with no view and nothing left to rebuild it.
    assert!(
        NoHome::SeatedButUnplaced.should_retry(),
        "a seated console is faulted, so #1125's bounded path retries it on the \
             same identity and `reconcile_seated_consoles` ends the story — a \
             rebuild that lands, or the seat given back with a notice"
    );
    assert!(
        !NoHome::Unplaced.should_retry(),
        "and a pane with no slot, no seat and no tile is left alone: a retry has \
             nowhere to aim, so it would flap to the budget and close a console \
             nobody asked to close"
    );
}

#[test]
fn an_authored_participants_console_that_lost_its_slot_is_left_as_it_is() {
    // The residue this module accepts, stated as a test so it is a decision
    // rather than a gap: `Ada`'s Station surface is gone (unplugged, or a
    // window not yet rebuilt), the law seats no station called `Ada` because
    // she is a person, and #1333 records no tile for a pane on a Station
    // window. So the rebuild reaches rule 4 and stops there — and nothing
    // else picks it up, since `reconcile_seated_consoles` iterates the
    // roster's seated stations only.
    let home = home_for_pane(
        "Ada",
        Some(&BridgeStationSurfaces::default()),
        Some(&layout_seating_helm()),
        &[],
    );
    assert_eq!(home, PaneHome::Nowhere(NoHome::Unplaced));
    let PaneHome::Nowhere(reason) = home else {
        unreachable!("just asserted")
    };
    assert!(
        !reason.should_retry(),
        "left open with no view, deliberately: the behaviour this replaced \
             rebuilt her console over the viewscreen instead"
    );
}

#[test]
fn a_legacy_tiled_pane_still_rebuilds_on_its_own_tile() {
    // Rule 3, untouched since issue #1125: a `--pane` on a host with no
    // `--profile` has no Station window anywhere, is seated by no law, and
    // rebuilds exactly where it was.
    assert_eq!(
        home_for_pane(
            "Ada",
            Some(&BridgeStationSurfaces::default()),
            Some(&layout_seating_helm()),
            &[PaneTile {
                name: "Ada".to_string(),
                origin: (960, 0),
                size: (960, 1080),
            }],
        ),
        PaneHome::PrimaryTile {
            origin: (960, 0),
            size: (960, 1080)
        }
    );
}

#[test]
fn a_host_with_no_display_adapter_at_all_still_tiles_its_panes() {
    // The `NativeRenderSurface::Contract` composition: no surfaces, no
    // layout, and the pre-#1123 single-window tiling is all there is.
    assert_eq!(
        home_for_pane("Ada", None, None, &helm_tile()),
        PaneHome::Nowhere(NoHome::Unplaced),
        "a name with no tile of its own is still nowhere"
    );
    assert_eq!(
        home_for_pane(
            "Ada",
            None,
            None,
            &[PaneTile {
                name: "Ada".to_string(),
                origin: (0, 0),
                size: (1280, 720),
            }],
        ),
        PaneHome::PrimaryTile {
            origin: (0, 0),
            size: (1280, 720)
        }
    );
}

#[test]
fn an_authored_participants_slot_is_a_station_home_like_any_other() {
    // A `--profile` `[[display.pane]]` with a label and no station key is a
    // participant's pane on a Station window (`StationPane::station` is
    // `None`). It is still SEATED — it has a screen of its own — so it is
    // built there, not tiled.
    let surfaces = surfaces_with("Ada", None);
    assert!(
        home_for_pane("Ada", Some(&surfaces), Some(&layout_seating_helm()), &[]).is_station(),
        "an authored participant's console lives on its Station window too"
    );
}

#[test]
fn a_name_nothing_knows_is_refused_rather_than_guessed_at() {
    assert_eq!(
        home_for_pane(
            "Grace",
            Some(&BridgeStationSurfaces::default()),
            Some(&layout_seating_helm()),
            &helm_tile(),
        ),
        PaneHome::Nowhere(NoHome::Unplaced)
    );
}

#[test]
fn a_console_the_law_stopped_seating_may_take_a_tile_again() {
    // The rule is about what the LAW says now, not about what a name once
    // was: a station whose seat was surrendered is no longer a wall console,
    // so nothing is being kept off the viewscreen any more.
    let free = layout_seating_helm()
        .apply(
            &crate::native_host::bridge_layout::LayoutAction::UnassignStation {
                station: station("helm"),
            },
        )
        .expect("giving a seat back is lawful");
    assert_eq!(
        home_for_pane(
            "helm",
            Some(&BridgeStationSurfaces::default()),
            Some(&free),
            &helm_tile(),
        ),
        PaneHome::PrimaryTile {
            origin: (0, 0),
            size: (1280, 720)
        }
    );
}
