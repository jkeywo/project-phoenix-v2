//! Hull arrival through the registered display adapter, without a layout store.
use super::*;
use crate::core::messages::StationId;
use crate::native_host::bridge_profile::{BridgeProfile, PaneSplit};
use crate::native_host::panes::{transport::PaneBus, PaneBusResource};
use crate::ship::components::PendingShipConfig;

const MAIN: &str = "Main@1920x1080";
const SIDE: &str = "Side@1920x1080";
const OTHER: &str = "Other@1920x1080";

fn id(value: &str) -> StationId {
    StationId(value.into())
}

fn hull(stations: &[&str]) -> PendingShipConfig {
    PendingShipConfig(
        toml::from_str(
            &stations
                .iter()
                .map(|s| {
                    format!(
                        "[[station]]\nid = '{s}'\nname = '{s}'\ndescription = '-'\nrank = 'Crew'\n"
                    )
                })
                .collect::<String>(),
        )
        .unwrap(),
    )
}

fn profile(station_first: bool) -> ValidatedProfile {
    let panes = if station_first {
        "[[display.pane]]\nlabel = 'helm'\nstation = 'helm'\n[[display.pane]]\nlabel = 'Ada'"
    } else {
        "[[display.pane]]\nlabel = 'Ada'\n[[display.pane]]\nlabel = 'helm'\nstation = 'helm'"
    };
    BridgeProfile::from_toml(&format!(
        "version = 1\n[[display]]\nid = '{MAIN}'\nrole = 'viewscreen'\n\
         [[display]]\nid = '{SIDE}'\nrole = 'station'\nsplit = 'stacked'\n{panes}"
    ))
    .unwrap()
    .validate()
    .unwrap()
}

fn host(profile: Option<ValidatedProfile>) -> (App, PaneBus) {
    host_with_side(profile, true)
}

fn host_with_side(profile: Option<ValidatedProfile>, side_present: bool) -> (App, PaneBus) {
    let mut app = App::new();
    app.add_plugins(BridgeDisplayPlugin);
    let bus = PaneBus::default();
    app.insert_resource(PaneBusResource(bus.clone()));
    if let Some(profile) = profile {
        app.insert_resource(BridgeDisplayConfig {
            profile,
            authored: true,
        });
    }
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    for (index, name) in ["Main", "Side", "Other"].into_iter().enumerate() {
        if name == "Side" && !side_present {
            continue;
        }
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
    app.update();
    (app, bus)
}

fn arrange(app: &mut App, action: LayoutAction) {
    let mut layout = app.world_mut().resource_mut::<BridgeLayoutResource>();
    layout.layout = layout.layout.apply(&action).unwrap();
}

#[test]
fn selected_roster_completes_authored_order_once_without_overwriting_live_choices() {
    for station_first in [true, false] {
        let (mut app, bus) = host(Some(profile(station_first)));
        assert!(app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .roster()
            .is_empty());
        assert!(bus.open_pane_for_name("helm").is_none());
        let original_profile = app
            .world()
            .resource::<BridgeDisplayConfig>()
            .profile
            .clone();
        app.insert_resource(hull(&["helm", "tactical"]));
        app.update();
        let layout = &app.world().resource::<BridgeLayoutResource>().layout;
        let side = MonitorIdentity::new(SIDE);
        assert_eq!(layout.roster(), &[id("helm"), id("tactical")]);
        assert_eq!(layout.reserved_on(&side), vec!["Ada"]);
        assert_eq!(layout.split_on(&side), PaneSplit::Stacked);
        assert_eq!(
            layout.occupants_on(&side),
            if station_first {
                vec!["helm", "Ada"]
            } else {
                vec!["Ada", "helm"]
            }
        );
        let surface = app
            .world()
            .resource::<BridgeStationSurfaces>()
            .0
            .iter()
            .find(|s| s.identity == SIDE)
            .unwrap();
        assert_eq!(surface.panes[0].rect.y, 0);
        assert_eq!(surface.panes[1].rect.y, 540);
        assert!(bus.open_pane_for_name("helm").is_some());

        arrange(
            &mut app,
            LayoutAction::AssignStation {
                station: id("helm"),
                monitor: MonitorIdentity::new(OTHER),
            },
        );
        app.update();
        arrange(
            &mut app,
            LayoutAction::UnassignStation {
                station: id("helm"),
            },
        );
        app.update();
        let chosen = app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .clone();
        // A same-hull reinsertion/return is not another profile application.
        app.insert_resource(hull(&["helm", "tactical"]));
        app.update();
        assert_eq!(
            app.world().resource::<BridgeLayoutResource>().layout,
            chosen
        );
        assert!(bus.open_pane_for_name("helm").is_none());
        assert_eq!(
            app.world().resource::<BridgeDisplayConfig>().profile,
            original_profile
        );
    }
}

#[test]
fn selected_roster_removes_invalid_seats_and_never_retries_deferred_intent() {
    let (mut app, bus) = host(Some(profile(true)));
    app.insert_resource(hull(&["tactical", "science"]));
    app.update();
    assert!(app.world().resource::<BridgeLayoutResource>().notices.iter().any(|n| matches!(n,
        LayoutNotice::Adopted(LayoutAdoption::SeatRefused { station, .. }) if station == &id("helm"))));
    arrange(
        &mut app,
        LayoutAction::AssignStation {
            station: id("tactical"),
            monitor: MonitorIdentity::new(OTHER),
        },
    );
    arrange(
        &mut app,
        LayoutAction::AssignStation {
            station: id("science"),
            monitor: MonitorIdentity::new(SIDE),
        },
    );
    app.update();
    app.insert_resource(hull(&["science", "helm"]));
    app.update();
    let layout = app.world().resource::<BridgeLayoutResource>();
    assert_eq!(layout.layout.roster(), &[id("science"), id("helm")]);
    assert_eq!(
        layout.layout.monitor_of(&id("science")),
        Some(&MonitorIdentity::new(SIDE))
    );
    assert!(layout.layout.monitor_of(&id("helm")).is_none());
    assert_eq!(
        layout.layout.reserved_on(&MonitorIdentity::new(SIDE)),
        vec!["Ada"]
    );
    assert!(layout.notices.iter().any(|n| matches!(n,
        LayoutNotice::Adopted(LayoutAdoption::StationOffRoster { station, .. }) if station == &id("tactical"))));
    assert!(bus.open_pane_for_name("tactical").is_none());
    assert!(bus.open_pane_for_name("science").is_some());
    assert!(bus.open_pane_for_name("helm").is_none());
}

#[test]
fn selected_roster_does_not_revive_authored_monitors_absent_or_lost_before_selection() {
    for present_at_boot in [false, true] {
        let (mut app, bus) = host_with_side(Some(profile(true)), present_at_boot);
        if present_at_boot {
            let side = app
                .world_mut()
                .query::<(Entity, &Monitor)>()
                .iter(app.world())
                .find(|(_, monitor)| monitor.name.as_deref() == Some("Side"))
                .unwrap()
                .0;
            app.world_mut().despawn(side);
            for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 2 {
                app.update();
            }
            assert!(!app
                .world()
                .resource::<BridgeLayoutResource>()
                .layout
                .monitors()
                .contains(&MonitorIdentity::new(SIDE)));
        }
        app.world_mut().spawn(Monitor {
            name: Some("Side".into()),
            physical_width: 1920,
            physical_height: 1080,
            physical_position: IVec2::new(1920, 0),
            refresh_rate_millihertz: Some(60_000),
            scale_factor: 1.0,
            video_modes: Vec::new(),
        });
        for _ in 0..DISPLAY_LOSS_DEBOUNCE_FRAMES + 2 {
            app.update();
        }
        assert!(app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .monitors()
            .contains(&MonitorIdentity::new(SIDE)));
        app.insert_resource(hull(&["helm"]));
        app.update();
        let layout = &app.world().resource::<BridgeLayoutResource>().layout;
        assert!(layout.monitor_of(&id("helm")).is_none());
        assert!(layout.reserved_on(&MonitorIdentity::new(SIDE)).is_empty());
        assert!(
            bus.open_pane_for_name("helm").is_none(),
            "selection cannot silently repair an absent/replugged authored monitor"
        );
    }
}

#[test]
fn selected_roster_respects_a_viewscreen_move_made_before_selection() {
    let mut profile = profile(true);
    let DisplayRole::Station { panes, .. } = &mut profile.displays[1].role else {
        unreachable!();
    };
    panes.retain(|pane| pane.station.is_some());
    let (mut app, bus) = host(Some(profile));
    arrange(
        &mut app,
        LayoutAction::SetViewscreen {
            monitor: MonitorIdentity::new(SIDE),
        },
    );
    app.update();
    app.insert_resource(hull(&["helm"]));
    app.update();
    let layout = app.world().resource::<BridgeLayoutResource>();
    assert_eq!(layout.layout.viewscreen(), &MonitorIdentity::new(SIDE));
    assert!(layout.layout.monitor_of(&id("helm")).is_none());
    assert!(bus.open_pane_for_name("helm").is_none());
    assert!(layout.notices.iter().any(|n| matches!(
        n,
        LayoutNotice::Adopted(LayoutAdoption::SeatRefused {
            refusal: super::super::bridge_layout::LayoutRefusal::StationOnViewscreenMonitor { .. },
            ..
        })
    )));
}

#[test]
fn selected_roster_is_available_after_game_start_consumes_pending_hull() {
    let (mut app, bus) = host(Some(profile(true)));
    let config = hull(&["helm", "tactical"]);
    app.insert_resource(crate::lobby::stations_config::stations_from_ship_config(
        &config.0,
    ));
    app.update();
    assert!(
        app.world()
            .resource::<BridgeLayoutResource>()
            .layout
            .roster()
            .is_empty(),
        "an unselected lobby fallback is not authoritative hull selection"
    );
    app.insert_resource(crate::lobby::SelectedShipResource(
        "selected-hull.toml".into(),
    ));
    app.update();
    assert!(!app.world().contains_resource::<PendingShipConfig>());
    assert_eq!(
        app.world()
            .resource::<BridgeLayoutResource>()
            .layout
            .roster(),
        &[id("helm"), id("tactical")]
    );
    assert!(bus.open_pane_for_name("helm").is_some());
}
