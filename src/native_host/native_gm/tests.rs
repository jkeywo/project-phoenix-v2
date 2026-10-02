use super::*;
use crate::native_host::bridge_layout::{BridgeLayout, LayoutAction};
use crate::native_host::bridge_profile::{identify, MonitorIdentity, RawMonitor};
use crate::native_host::panes::PaneId;
use bevy::ecs::system::RunSystemOnce;

fn fixture() -> World {
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
    let mut world = World::new();
    world.insert_resource(BridgeLayoutResource {
        layout,
        monitors,
        notices: Vec::new(),
    });
    world.insert_resource(NativeGmSurface {
        bridge: Default::default(),
        url: "/gm".into(),
    });
    world.init_resource::<NativeGmLifecycle>();
    world.init_resource::<NativeGmAuthority>();
    world.init_resource::<GmRoster>();
    world.init_resource::<crate::gm_action::SimulationPaused>();
    world.init_resource::<Messages<crate::lobby::OutboundMessage>>();
    world.insert_resource(State::new(GamePhase::Lobby));
    world
}

#[test]
fn host_cannot_disable_gm_after_launch_is_queued_for_the_next_transition() {
    let mut world = fixture();
    world.resource_mut::<NativeGmLifecycle>().enabled = true;
    world.insert_resource(NextState::Pending(GamePhase::InProgress));
    world.init_resource::<Messages<crate::lobby::InboundMessage>>();
    world.init_resource::<Messages<AppExit>>();
    let bridge = crate::native_host::host_lobby::HostLobbyBridge::new();
    assert!(bridge.submit_record(
        &crate::native_host::host_lobby::HostLobbyRecord::SetGameMaster { monitor: None },
    ));
    world.insert_resource(crate::native_host::host_lobby::HostLobbyBridgeResource(
        bridge,
    ));
    world
        .run_system_once(crate::native_host::host_lobby::drain_surface_records)
        .unwrap();
    let layout = world.resource::<BridgeLayoutResource>();
    assert!(layout.layout.game_master_monitor().is_some());
    assert!(layout.notices.iter().any(|notice| matches!(
        notice,
        crate::native_host::host_lobby::layout::LayoutNotice::Refused(
            crate::native_host::bridge_layout::LayoutRefusal::GameMasterRoleFrozen
        )
    )));
}

#[test]
fn admitted_gm_can_ready_before_freeze_and_frozen_membership_replaces_admission() {
    use crate::command_admission::HostSlot;
    use crate::native_host::host_lobby::fleet::NativeFleetGmAdmission;
    use crate::native_host::session_role::{NativeSessionRole, NativeSessionRoleState};
    let mut world = fixture();
    let mut role = NativeSessionRoleState::default();
    role.request(NativeSessionRole::FleetGameMaster);
    role.commit();
    world.insert_resource(role);
    let bridge = world.resource::<NativeGmSurface>().bridge.clone();
    bridge.activate(PaneId(1));
    bridge.mark_live();
    world.run_system_once(sync_presence).unwrap();
    assert!(!world.resource::<NativeGmAuthority>().connected);
    assert!(world.resource::<GmRoster>().operators().is_empty());
    world.insert_resource(NativeFleetGmAdmission(GmOperator::new(
        "gm-2".into(),
        "GM".into(),
        false,
    )));
    world.run_system_once(sync_presence).unwrap();
    world.resource_mut::<NativeGmLifecycle>().ready = true;
    world.run_system_once(sync_presence).unwrap();
    assert!(world.resource::<NativeGmAuthority>().connected);
    assert!(world.resource::<GmRoster>().operators()[0].ready);
    let roster = crate::lockstep::FleetRoster::with_participants(
        vec![crate::lockstep::FleetShip::new(HostSlot(1))],
        vec![HostSlot(1), HostSlot(2)],
        HostSlot(2),
        HostSlot(1),
    )
    .unwrap();
    world.insert_resource(roster);
    world.run_system_once(sync_presence).unwrap();
    assert!(
        !world.resource::<NativeGmAuthority>().connected,
        "frozen membership without this GM must not retain provisional authority"
    );
}

#[test]
fn loss_retains_identity_and_recovery_never_resumes_the_simulation() {
    let mut world = fixture();
    let bridge = world.resource::<NativeGmSurface>().bridge.clone();
    bridge.activate(PaneId(1));
    bridge.mark_live();
    world.run_system_once(sync_presence).unwrap();
    world.resource_mut::<NativeGmLifecycle>().ready = true;
    world.run_system_once(sync_presence).unwrap();
    assert!(world.resource::<GmRoster>().operators()[0].ready);
    world.insert_resource(State::new(GamePhase::InProgress));
    world.resource_mut::<BridgeLayoutResource>().monitors.pop();
    bridge.close();
    world.run_system_once(sync_presence).unwrap();
    let operator = &world.resource::<GmRoster>().operators()[0];
    assert_eq!(operator.id, NATIVE_GM_OPERATOR_ID);
    assert!(!operator.connected);
    assert!(!operator.ready);
    assert!(world.resource::<NativeGmAuthority>().screen_pause);
    assert!(world.resource::<crate::gm_action::SimulationPaused>().0);
    // Presence handling raises one loss edge, instead of repeatedly writing
    // a product pause; the authority hold enforces explicit Resume separately.
    world.resource_mut::<crate::gm_action::SimulationPaused>().0 = false;
    world.run_system_once(sync_presence).unwrap();
    assert!(!world.resource::<crate::gm_action::SimulationPaused>().0);
    world.resource_mut::<crate::gm_action::SimulationPaused>().0 = true;
    let original = fixture().remove_resource::<BridgeLayoutResource>().unwrap();
    world.resource_mut::<BridgeLayoutResource>().monitors = original.monitors;
    bridge.activate(PaneId(2));
    bridge.mark_live();
    world.run_system_once(sync_presence).unwrap();
    assert!(world.resource::<GmRoster>().operators()[0].connected);
    assert!(world.resource::<NativeGmAuthority>().screen_pause);
    assert!(world.resource::<crate::gm_action::SimulationPaused>().0);
    assert_eq!(
        world.resource::<NativeGmLifecycle>().desired_monitor,
        Some(MonitorIdentity::new("GM@1920x1080"))
    );
}

#[test]
fn view_fault_survives_same_frame_recreation_and_lobby_off_removes_operator() {
    let mut world = fixture();
    let bridge = world.resource::<NativeGmSurface>().bridge.clone();
    bridge.activate(PaneId(1));
    bridge.mark_live();
    world.run_system_once(sync_presence).unwrap();
    world.insert_resource(State::new(GamePhase::InProgress));
    bridge.fault();
    bridge.activate(PaneId(2));
    bridge.mark_live();
    world.run_system_once(sync_presence).unwrap();
    assert!(world.resource::<crate::gm_action::SimulationPaused>().0);
    assert!(world.resource::<NativeGmAuthority>().screen_pause);
    world.insert_resource(State::new(GamePhase::Lobby));
    let mut layout = world.resource_mut::<BridgeLayoutResource>();
    layout.layout = layout
        .layout
        .apply(&LayoutAction::SetGameMaster { monitor: None })
        .unwrap();
    world.run_system_once(sync_presence).unwrap();
    assert!(world.resource::<GmRoster>().operators().is_empty());
    assert!(!world.resource::<NativeGmAuthority>().connected);
    assert!(!world.contains_resource::<crate::gm_projection::NativeGmPresentation>());
}

#[test]
fn enabling_gm_on_the_final_countdown_tick_blocks_launch_and_keeps_loaded() {
    use crate::native_host::host_lobby::{
        drain_surface_records, pump_host_lobby, HostLobbyBridge, HostLobbyBridgeResource,
    };
    use crate::native_host::panes::RecordingSurface;

    let mut configured = fixture();
    let mut layout = configured
        .remove_resource::<BridgeLayoutResource>()
        .unwrap();
    layout.layout = layout
        .layout
        .apply(&LayoutAction::SetGameMaster { monitor: None })
        .unwrap();
    let surface = configured.remove_resource::<NativeGmSurface>().unwrap();
    let gm_bridge = surface.bridge.clone();
    let lobby_bridge = HostLobbyBridge::new();
    let mut app = App::new();
    app.add_plugins((crate::lobby::LobbyPlugin, bevy::time::TimePlugin))
        .insert_resource(layout)
        .insert_resource(surface)
        .insert_resource(HostLobbyBridgeResource(lobby_bridge.clone()))
        .init_resource::<NativeGmLifecycle>()
        .init_resource::<NativeGmAuthority>()
        .init_resource::<crate::gm_action::SimulationPaused>()
        .init_resource::<crate::world::config::WorldConfig>()
        .add_systems(
            PreUpdate,
            (
                drain_surface_records,
                sync_lobby_role_intent,
                drain_records,
                sync_presence,
            )
                .chain(),
        )
        .add_systems(PostUpdate, sync_presence);
    crate::sim_tick::register_sim_tick(&mut app);
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_secs(1),
    );
    app.update();
    {
        let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
        sessions.0.register("crew".into(), "Ada".into()).unwrap();
        sessions.0.set_ready("crew", true);
    }
    {
        let mut countdown = app
            .world_mut()
            .resource_mut::<crate::lobby::CountdownTimer>();
        countdown.remaining_secs = 0.001;
        countdown.pending_phase = Some(GamePhase::InProgress);
    }
    let mut lobby_surface = RecordingSurface::ready();
    lobby_surface.queue_record(r#"{"kind":"set-game-master","monitor":"GM@1920x1080"}"#);
    pump_host_lobby(&lobby_bridge, &mut lobby_surface);
    gm_bridge.activate(PaneId(7));
    let mut gm_surface = RecordingSurface::ready();
    gm_surface.queue_record(r#"{"kind":"loaded"}"#);
    gm_bridge.pump(PaneId(7), &mut gm_surface);
    app.update();

    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::Lobby
    );
    assert!(matches!(
        app.world().resource::<NextState<GamePhase>>(),
        NextState::Unchanged
    ));
    assert_eq!(
        app.world()
            .resource::<crate::lobby::CountdownTimer>()
            .remaining_secs,
        0.0
    );
    let gm = &app.world().resource::<GmRoster>().operators()[0];
    assert!(
        gm.connected,
        "the first Loaded record was accepted after role enable"
    );
    assert!(!gm.ready);
}

#[test]
fn unready_or_surface_loss_on_the_final_countdown_tick_cancels_launch() {
    use crate::native_host::panes::RecordingSurface;

    for event in [
        "unready",
        "surface-fault",
        "worker-fault",
        "recovered-fault",
        "monitor-loss",
    ] {
        let mut configured = fixture();
        let layout = configured
            .remove_resource::<BridgeLayoutResource>()
            .unwrap();
        let surface = configured.remove_resource::<NativeGmSurface>().unwrap();
        let bridge = surface.bridge.clone();
        bridge.activate(PaneId(7));
        bridge.mark_live();
        let mut app = App::new();
        app.add_plugins((crate::lobby::LobbyPlugin, bevy::time::TimePlugin))
            .insert_resource(layout)
            .insert_resource(surface)
            .init_resource::<NativeGmLifecycle>()
            .init_resource::<NativeGmAuthority>()
            .init_resource::<crate::gm_action::SimulationPaused>()
            .init_resource::<crate::world::config::WorldConfig>()
            .add_systems(
                PreUpdate,
                (sync_lobby_role_intent, drain_records, sync_presence).chain(),
            )
            .add_systems(PostUpdate, sync_presence);
        crate::sim_tick::register_sim_tick(&mut app);
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_secs(1),
        );
        app.update();
        {
            let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
            sessions.0.register("crew".into(), "Ada".into()).unwrap();
            sessions.0.set_ready("crew", true);
        }
        app.world_mut().resource_mut::<NativeGmLifecycle>().ready = true;
        app.world_mut().run_system_once(sync_presence).unwrap();
        assert!(app.world().resource::<GmRoster>().operators()[0].ready);
        {
            let mut countdown = app
                .world_mut()
                .resource_mut::<crate::lobby::CountdownTimer>();
            countdown.remaining_secs = 0.001;
            countdown.pending_phase = Some(GamePhase::InProgress);
        }
        match event {
            "unready" | "surface-fault" => {
                let mut page = RecordingSurface::ready();
                page.queue_record(if event == "unready" {
                    r#"{"kind":"ready","ready":false}"#
                } else {
                    r#"{"kind":"surface-fault"}"#
                });
                bridge.pump(PaneId(7), &mut page);
            }
            "worker-fault" => bridge.fault(),
            "recovered-fault" => {
                bridge.fault();
                bridge.activate(PaneId(8));
                bridge.mark_live();
            }
            "monitor-loss" => {
                app.world_mut()
                    .resource_mut::<BridgeLayoutResource>()
                    .monitors
                    .pop();
            }
            _ => unreachable!(),
        }
        app.update();

        assert_eq!(
            app.world().resource::<State<GamePhase>>().get(),
            &GamePhase::Lobby,
            "{event} must cancel before launch"
        );
        assert!(
            matches!(
                app.world().resource::<NextState<GamePhase>>(),
                NextState::Unchanged
            ),
            "{event} must not leave a mission transition queued"
        );
        let countdown = app.world().resource::<crate::lobby::CountdownTimer>();
        assert_eq!(countdown.remaining_secs, 0.0, "{event}");
        assert!(countdown.pending_phase.is_none(), "{event}");
        let gm = &app.world().resource::<GmRoster>().operators()[0];
        assert!(!gm.ready, "{event}");
        assert_eq!(
            gm.connected,
            matches!(event, "unready" | "recovered-fault"),
            "{event}"
        );
    }
}
