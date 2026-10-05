use super::*;
use crate::core::messages::StationId;
use crate::native_host::bridge_display::{BridgeStationSurface, StationPane};
use crate::native_host::bridge_profile::{MonitorGeometry, PaneRect};
use crate::native_host::panes::{service_faults, PaneIdentity};
use bevy::prelude::Entity;
use phoenix_transport::transport::Transport;

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

fn queued(bus: &PaneBus, name: &str) -> PaneId {
    let id = bus.open(PaneIdentity::mint(name.to_string()));
    bus.close(id);
    bus.recreate(id).expect("closed pane is recreated").0
}

#[test]
fn preparation_retires_closed_views_without_recovery() {
    let bus = PaneBus::default();
    let closed = queued(&bus, "Ada");
    bus.close(closed);
    assert_eq!(
        prepare_pending_views(&bus, None, None, &[]),
        vec![PendingViewDisposition::Retired { id: closed },]
    );
    assert!(service_faults(&bus).is_empty());
    assert!(prepare_pending_views(&bus, None, None, &[]).is_empty());
}

#[test]
fn preparation_keeps_participant_and_primary_tile_policy() {
    let bus = PaneBus::default();
    let participant = queued(&bus, "Ada");
    let tile = queued(&bus, "helm");
    let prepared = prepare_pending_views(&bus, None, None, &helm_tile());
    assert!(
        matches!(&prepared[0], PendingViewDisposition::Unplaced { id, reason: NoHome::Unplaced, .. } if *id == participant)
    );
    assert!(
        matches!(&prepared[1], PendingViewDisposition::Build { id, home: PaneHome::PrimaryTile { size: (1280, 720), .. }, .. } if *id == tile)
    );
    assert!(service_faults(&bus).is_empty());
    let moved = queued(&bus, "Ada");
    assert!(
        matches!(prepare_pending_views(&bus, Some(&surfaces_with("Ada", None)), None, &[]).as_slice(),
        [PendingViewDisposition::Build { id, home: PaneHome::Station { size: (960, 1080), scale: 1.5, .. }, .. }] if *id == moved)
    );
}

#[test]
fn preparation_faults_missing_seat_once_and_retries_on_the_same_identity() {
    let bus = PaneBus::default();
    let id = queued(&bus, "helm");
    let token = bus.token_of(id).unwrap();
    let layout = layout_seating_helm();
    assert!(matches!(
        prepare_pending_views(&bus, None, Some(&layout), &helm_tile()).as_slice(),
        [PendingViewDisposition::Unplaced {
            reason: NoHome::SeatedButUnplaced,
            ..
        }]
    ));
    assert!(bus.is_open(id), "preparation never services recovery");
    assert!(prepare_pending_views(&bus, None, Some(&layout), &helm_tile()).is_empty());
    let replacement = service_faults(&bus).pop().unwrap().recreated.unwrap().0;
    assert_eq!(bus.token_of(replacement), Some(token));
    let surfaces = surfaces_with("helm", Some(station("helm")));
    assert!(
        matches!(prepare_pending_views(&bus, Some(&surfaces), Some(&layout), &helm_tile()).as_slice(),
        [PendingViewDisposition::Build { id, home: PaneHome::Station { .. }, .. }] if *id == replacement)
    );
}

#[test]
fn preparation_never_builds_or_faults_an_open_superseded_view() {
    use crate::native_host::connections::SharedConnections;

    let bus = PaneBus::default();
    let shared = SharedConnections::default();
    bus.transport().share_connections(shared.clone());
    let pane = queued(&bus, "helm");
    crate::native_host::panes::transport::identify_test_pane(&bus, pane);
    let token = bus.token_of(pane).unwrap();
    {
        let mut connections = shared.lock();
        let leg = connections.new_leg();
        let phone = connections.open(leg);
        connections.bind(phone, &token).unwrap();
    }
    // The bus retires superseded queue entries while locking. The placement
    // lifecycle must neither construct a view nor fault this retained seat.
    assert!(
        prepare_pending_views(&bus, None, Some(&layout_seating_helm()), &helm_tile()).is_empty()
    );
    assert!(bus.is_open(pane));
    assert!(bus.is_superseded(pane));
    assert!(service_faults(&bus).is_empty());
    assert_eq!(bus.open_pane_for_name("helm"), Some(pane));
}
