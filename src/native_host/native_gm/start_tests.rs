use super::*;
use crate::gm_roster::GmOperator;
use crate::native_host::bridge_display::BridgeLayoutResource;
use crate::native_host::bridge_layout::{BridgeLayout, LayoutAction};
use crate::native_host::bridge_profile::{identify, RawMonitor};
use crate::native_host::panes::PaneId;

fn fixture() -> App {
    let mut app = App::new();
    let monitors = identify(&[
        RawMonitor {
            name: Some("Viewscreen".into()),
            physical_width: 1920,
            physical_height: 1080,
            position_x: 0,
            position_y: 0,
            scale_factor: 1.0,
            primary: true,
        },
        RawMonitor {
            name: Some("GM".into()),
            physical_width: 1920,
            physical_height: 1080,
            position_x: 1920,
            position_y: 0,
            scale_factor: 1.0,
            primary: false,
        },
    ]);
    let layout = BridgeLayout::new(
        monitors.iter().map(|m| m.identity.clone()),
        Vec::new(),
        &monitors[0].identity,
    )
    .unwrap()
    .apply(&LayoutAction::SetGameMaster {
        monitor: Some(monitors[1].identity.clone()),
    })
    .unwrap();
    let bridge = super::super::bridge::NativeGmBridge::default();
    bridge.activate(PaneId(1));
    bridge.mark_live();
    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions.register("crew".into(), "Ada".into()).unwrap();
    app.insert_resource(BridgeLayoutResource {
        layout,
        monitors,
        notices: Vec::new(),
    })
    .insert_resource(NativeGmSurface {
        bridge,
        url: "/gm".into(),
    })
    .init_resource::<NativeGmLifecycle>()
    .insert_resource(NativeGmAuthority {
        connected: true,
        screen_pause: false,
    })
    .insert_resource(
        GmRoster::try_new(vec![GmOperator::new(
            NATIVE_GM_OPERATOR_ID.into(),
            "GM".into(),
            true,
        )])
        .unwrap(),
    )
    .insert_resource(Sessions(sessions))
    .insert_resource(State::new(GamePhase::Lobby))
    .init_resource::<NextState<GamePhase>>()
    .init_resource::<crate::world::config::WorldConfig>()
    .init_resource::<FleetManagedLobby>()
    .init_resource::<PendingForceStart>()
    .init_resource::<StartGrantResults>()
    .insert_resource(crate::sim_tick::SimTick(17))
    .add_plugins(NativeGmStartPlugin);
    app.world_mut().resource_mut::<NativeGmLifecycle>().enabled = true;
    app
}

fn request(app: &mut App) -> StartGrantResult {
    app.world_mut()
        .resource_mut::<NativeGmStartRequests>()
        .request();
    app.world_mut().run_schedule(FixedUpdate);
    app.world()
        .resource::<NativeGmStartRequests>()
        .last_result()
        .unwrap()
        .clone()
}

#[test]
fn connected_gm_forces_unready_crew_with_one_attributed_result() {
    let mut app = fixture();
    assert!(!app.world().resource::<Sessions>().0.all_ready());
    assert!(!app.world().resource::<GmRoster>().operators()[0].ready);
    {
        let mut state = app.world_mut().resource_mut::<NativeGmStartRequests>();
        assert!(state.request());
        for _ in 0..100 {
            assert!(!state.request());
        }
    }
    app.world_mut().run_schedule(FixedUpdate);
    assert!(app.world().resource::<PendingForceStart>().0);
    let results = app.world().resource::<StartGrantResults>();
    assert_eq!(results.iter().count(), 1);
    assert_eq!(
        results.iter().next().unwrap(),
        &StartGrantResult {
            tick: 17,
            status: StartGrantStatus::Applied,
            operator_id: Some(NATIVE_GM_OPERATOR_ID.into()),
            reason: None,
            grant_id: Some("start-1".into()),
        }
    );
    app.world_mut().run_schedule(FixedUpdate);
    assert_eq!(
        app.world().resource::<StartGrantResults>().iter().count(),
        1
    );
}

#[test]
fn accepted_request_uses_the_existing_phase_applier() {
    let mut app = fixture();
    app.init_resource::<crate::lobby::LobbyOutbox>()
        .add_systems(FixedUpdate, crate::server::bridge::apply_force_start);
    assert_eq!(request(&mut app).status, StartGrantStatus::Applied);
    assert!(!app.world().resource::<PendingForceStart>().0);
    assert!(matches!(
        app.world().resource::<NextState<GamePhase>>(),
        NextState::Pending(GamePhase::InProgress)
    ));
}

#[test]
fn no_world_and_failed_validation_are_terminal_refusals() {
    let mut app = fixture();
    app.world_mut()
        .remove_resource::<crate::world::config::WorldConfig>();
    let first = request(&mut app);
    assert_eq!(first.reason, Some(StartGrantReason::ValidationFailed));
    assert!(!app.world().resource::<PendingForceStart>().0);
    app.init_resource::<crate::world::config::WorldConfig>();
    app.world_mut().run_schedule(FixedUpdate);
    assert!(!app.world().resource::<PendingForceStart>().0);
    app.world_mut()
        .resource_mut::<FleetManagedLobby>()
        .validation_passed = false;
    let second = request(&mut app);
    assert_eq!(second.reason, Some(StartGrantReason::ValidationFailed));
    assert_eq!(second.grant_id.as_deref(), Some("start-2"));
}

#[test]
fn surface_authority_and_monitor_loss_are_rechecked_on_the_tick() {
    for loss in 0..5 {
        let mut app = fixture();
        app.world_mut()
            .resource_mut::<NativeGmStartRequests>()
            .request();
        match loss {
            0 => app.world().resource::<NativeGmSurface>().bridge.close(),
            1 => app.world().resource::<NativeGmSurface>().bridge.fault(),
            2 => {
                app.world_mut()
                    .resource_mut::<NativeGmAuthority>()
                    .connected = false
            }
            3 => {
                app.world_mut()
                    .resource_mut::<BridgeLayoutResource>()
                    .monitors
                    .pop();
            }
            _ => {
                app.world_mut().remove_resource::<NativeGmSurface>();
            }
        }
        app.world_mut().run_schedule(FixedUpdate);
        let result = app
            .world()
            .resource::<NativeGmStartRequests>()
            .last_result()
            .unwrap();
        assert_eq!(result.reason, Some(StartGrantReason::GmNotConnected));
        assert!(!app.world().resource::<PendingForceStart>().0);
    }
}

#[test]
fn disabled_role_and_fleet_cannot_use_private_standalone_force() {
    for denied in 0..3 {
        let mut app = fixture();
        match denied {
            0 => app.world_mut().resource_mut::<NativeGmLifecycle>().enabled = false,
            1 => app.world_mut().resource_mut::<FleetManagedLobby>().enabled = true,
            _ => {
                app.insert_resource(crate::lockstep::FleetLockstep(
                    crate::lockstep::LockstepSession::new(
                        crate::command_admission::HostSlot::SOLO,
                        [],
                        0,
                    ),
                ));
            }
        }
        assert_eq!(
            request(&mut app).reason,
            Some(StartGrantReason::UnauthorizedGrant)
        );
        assert!(!app.world().resource::<PendingForceStart>().0);
    }
}

#[test]
fn roster_admission_and_phase_are_rechecked() {
    let mut app = fixture();
    app.insert_resource(GmRoster::default());
    assert_eq!(
        request(&mut app).reason,
        Some(StartGrantReason::GmNotConnected)
    );
    let mut app = fixture();
    app.insert_resource(State::new(GamePhase::InProgress));
    assert_eq!(
        request(&mut app).reason,
        Some(StartGrantReason::AlreadyStarted)
    );
    assert!(!app.world().resource::<PendingForceStart>().0);
    let mut app = fixture();
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::Loading);
    assert_eq!(request(&mut app).status, StartGrantStatus::NoOp);
}

#[test]
fn readiness_metadata_includes_unready_stationless_crew_and_gms() {
    let app = fixture();
    let totals = readiness_totals(
        app.world().resource::<Sessions>().0.readiness_tally(),
        app.world().resource::<GmRoster>(),
    );
    assert_eq!(
        totals.crew,
        ReadinessTally {
            connected: 1,
            ready: 0
        }
    );
    assert_eq!(
        totals.gms,
        ReadinessTally {
            connected: 1,
            ready: 0
        }
    );
    assert_eq!(
        totals.participants,
        Some(ReadinessTally {
            connected: 2,
            ready: 0
        })
    );
    assert!(readiness_totals(
        ReadinessTally {
            connected: 0,
            ready: 1
        },
        &GmRoster::default()
    )
    .participants
    .is_none());
}
