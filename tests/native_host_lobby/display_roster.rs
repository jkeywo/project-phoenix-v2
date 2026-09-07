//! The actual native selection path updates displays without saved-layout I/O.
use super::*;
use bevy::window::{Monitor, PrimaryMonitor, PrimaryWindow, Window};
use project_phoenix::core::messages::StationId;
use project_phoenix::native_host::bridge_display::{BridgeLayoutResource, BridgeStationSurfaces};
use project_phoenix::native_host::bridge_layout::LayoutAction;
use project_phoenix::native_host::bridge_profile::{BridgeProfile, MonitorIdentity};
use project_phoenix::native_host::layout_store_systems::BridgeLayoutStore;
use project_phoenix::native_host::panes::{transport::PaneBus, PaneBusResource};

const MAIN: &str = "Selection Main@1920x1080";
const SIDE: &str = "Selection Side@1920x1080";
const OTHER: &str = "Selection Other@1920x1080";
const CREW: &str = "3f1a6c2e-0a11-4b3c-9d55-000000000081";

fn selected_display_host(authored: bool) -> (App, LoopbackHandle, PaneBus) {
    let mut cfg = lobby_config();
    if authored {
        cfg.bridge_profile = Some(
            BridgeProfile::from_toml(&format!(
                "version = 1\n[[display]]\nid = '{MAIN}'\nrole = 'viewscreen'\n\
                 [[display]]\nid = '{SIDE}'\nrole = 'station'\n\
                 [[display.pane]]\nlabel = 'helm'\nstation = 'helm'"
            ))
            .unwrap()
            .validate()
            .unwrap(),
        );
    }
    let mut app = build_native_host_app(&cfg, &preload()).unwrap();
    // No saved-layout resource on either route: the display roster is required
    // even on machines without a usable home directory, and must not need I/O.
    app.world_mut().remove_resource::<BridgeLayoutStore>();
    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));
    let bus = PaneBus::default();
    app.insert_resource(PaneBusResource(bus.clone()));
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    for (index, name) in ["Selection Main", "Selection Side", "Selection Other"]
        .into_iter()
        .enumerate()
    {
        let mut entity = app.world_mut().spawn(Monitor {
            name: Some(name.into()),
            physical_width: 1920,
            physical_height: 1080,
            physical_position: IVec2::new(index as i32 * 1920, 0),
            refresh_rate_millihertz: Some(60_000),
            scale_factor: 1.0,
            video_modes: Vec::new(),
        });
        if index == 0 {
            entity.insert(PrimaryMonitor);
        }
    }
    pump(&mut app, 4);
    assert!(app
        .world()
        .resource::<BridgeLayoutResource>()
        .layout
        .roster()
        .is_empty());
    handle.send(
        CREW,
        ClientMessage::Identify {
            token: CREW.into(),
            name: "Captain".into(),
        },
    );
    pump(&mut app, 2);
    let (scenario, hull) = pick();
    handle.send(
        CREW,
        ClientMessage::SelectScenario {
            scenario_id: scenario,
        },
    );
    handle.send(
        CREW,
        ClientMessage::SelectPlayerShip {
            template_path: hull,
        },
    );
    pump(&mut app, 4);
    assert!(app.world().contains_resource::<WorldConfig>());
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby
    );
    let expected: Vec<_> = app
        .world()
        .resource::<project_phoenix::ship::components::PendingShipConfig>()
        .0
        .stations
        .iter()
        .map(|s| s.id.clone())
        .collect();
    let layout = &app.world().resource::<BridgeLayoutResource>().layout;
    assert!(!expected.is_empty());
    assert_eq!(layout.roster(), expected);
    assert_eq!(
        layout.monitor_of(&StationId("helm".into())),
        authored.then(|| MonitorIdentity::new(SIDE)).as_ref()
    );
    if authored {
        assert!(bus.open_pane_for_name("helm").is_some());
        assert!(app
            .world()
            .resource::<BridgeStationSurfaces>()
            .0
            .iter()
            .any(|s| s.identity == SIDE
                && s.panes
                    .iter()
                    .any(|p| p.station.as_ref() == Some(&StationId("helm".into())))));
    } else {
        assert!(layout
            .roster()
            .iter()
            .all(|s| layout.monitor_of(s).is_none()));
        assert_eq!(bus.open_count(), 0);
        assert!(matches!(
            app.world_mut()
                .query_filtered::<&Window, With<PrimaryWindow>>()
                .single(app.world())
                .unwrap()
                .mode,
            bevy::window::WindowMode::Windowed
        ));
    }
    (app, handle, bus)
}

fn assert_return_keeps_selected_arrangement(authored: bool) {
    let (mut app, handle, bus) = selected_display_host(authored);
    let chosen = app
        .world()
        .resource::<BridgeLayoutResource>()
        .layout
        .apply(&LayoutAction::AssignStation {
            station: StationId("helm".into()),
            monitor: MonitorIdentity::new(OTHER),
        })
        .unwrap();
    app.world_mut()
        .resource_mut::<BridgeLayoutResource>()
        .layout = chosen.clone();
    pump(&mut app, 2);
    let pane = bus
        .open_pane_for_name("helm")
        .expect("the selected station opens on the chosen display");
    // Contract hosts have no GPU preload; enter the registered lifecycle, then
    // use the ordinary participant ReturnToLobby rather than a display reset.
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    pump(&mut app, 2);
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::GameOver);
    pump(&mut app, 2);
    handle.drain_outbound();
    handle.send(CREW, ClientMessage::ReturnToLobby);
    pump(&mut app, 4);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby
    );
    assert!(handle
        .drain_outbound()
        .iter()
        .any(|(_, message, _)| matches!(message, ServerMessage::Welcome { .. })));
    assert_eq!(
        app.world().resource::<BridgeLayoutResource>().layout,
        chosen
    );
    assert_eq!(
        bus.open_pane_for_name("helm"),
        Some(pane),
        "return keeps the original console and the operator's current monitor"
    );
}

#[test]
fn selected_world_display_roster_applies_deferred_profile_and_survives_return() {
    assert_return_keeps_selected_arrangement(true);
}

#[test]
fn selected_world_display_roster_without_profile_or_store_survives_return() {
    assert_return_keeps_selected_arrangement(false);
}
