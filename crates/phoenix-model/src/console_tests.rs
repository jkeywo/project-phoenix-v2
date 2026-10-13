use super::*;
use crate::messages::HelmEngineBlackboard;
use std::collections::HashMap;

fn console(id: &str) -> ConsoleConfig {
    ConsoleConfig {
        id: ConsoleId(id.into()),
        name: id.into(),
        home_station: Some(StationId("pilot-seat".into())),
        screen: Some("gui/helm.html".into()),
        human_seeking: true,
        host_order: vec![
            StationId("engineering-seat".into()),
            StationId("command-seat".into()),
        ],
        visiting_rating: None,
        feeds: vec![],
        ..Default::default()
    }
}

#[test]
fn placement_preserves_identity_and_prefers_home_then_authored_order() {
    let console = console("flight-display");
    assert_eq!(console.resolve_host(|_| true), console.home_station);
    assert_eq!(
        console.resolve_host(|id| id.0 != "pilot-seat"),
        Some(StationId("engineering-seat".into()))
    );
    assert_eq!(console.resolve_host(|_| false), None);
    assert_eq!(console.id, ConsoleId("flight-display".into()));
}

#[test]
fn fixed_and_screenless_consoles_do_not_seek_a_human() {
    let mut console = console("flight-display");
    console.human_seeking = false;
    assert_eq!(console.resolve_host(|id| id.0 == "command-seat"), None);
    console.screen = None;
    assert_eq!(console.resolve_host(|_| true), None);
}

#[test]
fn cohosted_screens_read_only_their_explicit_typed_feeds() {
    let mut first = console("flight-display");
    let mut second = console("diagnostic-display");
    first.feeds.push(ConsoleFeed {
        source: SystemId("port-engine".into()),
        kind: ConsoleObservationKind::HelmEngine,
    });
    second.feeds.push(ConsoleFeed {
        source: SystemId("starboard-engine".into()),
        kind: ConsoleObservationKind::HelmEngine,
    });
    let port = SystemBlackboard::HelmEngine(HelmEngineBlackboard {
        thrust_fraction: 0.4,
        is_online: true,
    });
    let starboard = SystemBlackboard::HelmEngine(HelmEngineBlackboard {
        thrust_fraction: 0.8,
        is_online: true,
    });
    let publications = HashMap::from([
        (SystemId("port-engine".into()), port.clone()),
        (SystemId("starboard-engine".into()), starboard.clone()),
    ]);
    assert_eq!(first.resolve_host(|_| true), second.resolve_host(|_| true));
    assert_eq!(
        first
            .observations(&publications)
            .map(|(_, value)| value)
            .collect::<Vec<_>>(),
        vec![&port]
    );
    assert_eq!(
        second
            .observations(&publications)
            .map(|(_, value)| value)
            .collect::<Vec<_>>(),
        vec![&starboard]
    );
    second.feeds[0].kind = ConsoleObservationKind::PhaserBank;
    assert_eq!(second.observations(&publications).count(), 0);
    first.feeds[0].source = SystemId("missing-engine".into());
    assert_eq!(first.observations(&publications).count(), 0);
}

#[test]
fn working_view_refuses_nonfinite_navigation_and_unbounded_strings() {
    let mut view = ConsoleWorkingView::default();
    assert!(view.is_valid());
    view.navigation = Some(NavigationWorkingView {
        zoom: 1.0,
        center_x: 42.0,
        center_y: -20.0,
        selected_uuid: None,
    });
    assert!(view.is_valid());
    view.navigation.as_mut().unwrap().center_x = f32::NAN;
    assert!(!view.is_valid());
    view.navigation = None;
    view.comms_thread = Some("x".repeat(257));
    assert!(!view.is_valid());
    view.comms_thread = Some("fleet\nprivate".into());
    assert!(!view.is_valid());
}

#[test]
fn console_wire_roster_uses_feeds_while_authoring_uses_feed() {
    let definition = console("flight");
    let authored = serde_json::to_value(&definition).unwrap();
    assert!(authored.get("feed").is_some());
    let wire = crate::wire::ConsoleDef {
        id: definition.id,
        name: definition.name,
        home_station: definition.home_station,
        screen: definition.screen,
        human_seeking: definition.human_seeking,
        host_order: definition.host_order,
        visiting_rating: definition.visiting_rating,
        feeds: definition.feeds,
        systems: vec![],
        description: definition.description,
        short_code: definition.short_code,
        manual_overview: definition.manual_overview,
        tutorials: definition.tutorials,
        command_target: definition.command_target,
        ratings: definition.ratings,
    };
    let value = serde_json::to_value(wire).unwrap();
    assert!(value.get("feeds").is_some());
    assert!(value.get("feed").is_none());
}
