//! A local GM beside the ship in the same authoritative native simulation.
pub mod bridge;
pub mod document;
pub mod recovery;
pub mod start;

use super::bridge_display::BridgeLayoutResource;
use crate::console_bridge::*;
use crate::core::{codec, messages::GamePhase};
use crate::gm_action::{NativeGmAuthority, NATIVE_GM_OPERATOR_ID};
use crate::gm_roster::{GmOperator, GmRoster};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone)]
pub struct NativeGmSurface {
    pub bridge: bridge::NativeGmBridge,
    pub url: String,
}

#[derive(Resource, Default)]
pub struct NativeGmLifecycle {
    pub enabled: bool,
    pub ready: bool,
    pub desired_monitor: Option<super::bridge_profile::MonitorIdentity>,
    /// Loss is a pause edge, not a condition that can undo the operator's Resume.
    lost: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum NativeGmRecord {
    Loaded,
    Ready { ready: bool },
    ForceStart,
    SurfaceFault,
    RecoveryHostLobby,
    Action { request: String },
}

#[derive(Serialize)]
pub struct NativeGmMetadata {
    pub phase: GamePhase,
    pub host_lobby_unavailable: bool,
    pub role_presets: Vec<crate::world::config::GmRolePresetEntry>,
    pub gms: Vec<GmOperator>,
    pub start_policy: start::NativeGmReadinessTotals,
    pub start_result: Option<crate::lobby::start_policy::StartGrantResult>,
}

pub struct NativeGmPlugin;
impl Plugin for NativeGmPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        // Private surface transport and external presence/readiness bookkeeping;
        // canonical actions, rather than this mailbox, enter the world journal.
        app.declare_state::<NativeGmSurface>(StateClass::Timer, "native-local-gm-workspace")
            .declare_state::<NativeGmLifecycle>(StateClass::Timer, "native-local-gm-workspace")
            .declare_state::<NativeGmAuthority>(StateClass::Timer, "native-local-gm-workspace")
            .declare_state::<crate::gm_projection::NativeGmPresentation>(
                StateClass::Presentation,
                "native-local-gm-workspace",
            )
            .init_resource::<NativeGmLifecycle>()
            .init_resource::<NativeGmAuthority>()
            .init_resource::<GmRoster>()
            .add_plugins((
                start::NativeGmStartPlugin,
                crate::gm_projection::GmProjectionPlugin,
                crate::gm_activity::GmActivityPlugin,
            ))
            .add_systems(
                PreUpdate,
                // Ready/Unready and a surface failure must reach the roster and
                // pause authority before this frame's fixed tick can launch or
                // advance the simulation. PostUpdate still catches display work.
                (sync_lobby_role_intent, drain_records, sync_presence)
                    .chain()
                    .after(super::host_lobby::drain_surface_records)
                    .before(crate::gm_action::apply_due_actions)
                    .run_if(resource_exists::<NativeGmSurface>),
            )
            .add_systems(
                PostUpdate,
                (sync_presence, feed_projections)
                    .chain()
                    .after(crate::gm_activity::publish_frame_activity)
                    .after(crate::gm_action::publish_session_projection)
                    .after(crate::gm_event::publish_mission_projection)
                    .after(crate::gm_spawn::publish_spawn_projection)
                    .after(crate::gm_comms::publish_comms_projection)
                    .run_if(resource_exists::<NativeGmSurface>),
            );
    }
}

/// Commit a lobby role change before the fixed tick evaluates readiness. The
/// surface itself is created later in Update, so every new placement first
/// enters the roster as unready. Keeping this separate from full presence
/// publication also lets the first Loaded record see the enabled role.
fn sync_lobby_role_intent(
    layout: Option<Res<BridgeLayoutResource>>,
    phase: Res<State<GamePhase>>,
    mut state: ResMut<NativeGmLifecycle>,
    mut authority: ResMut<NativeGmAuthority>,
    mut roster: ResMut<GmRoster>,
    mut outbound: MessageWriter<crate::lobby::OutboundMessage>,
) {
    if *phase.get() != GamePhase::Lobby {
        return;
    }
    let assigned = layout
        .as_ref()
        .and_then(|layout| layout.layout.game_master_monitor());
    if state.enabled == assigned.is_some() && state.desired_monitor.as_ref() == assigned {
        return;
    }
    state.enabled = assigned.is_some();
    state.desired_monitor = assigned.cloned();
    state.ready = false;
    authority.connected = false;
    let mut rows: Vec<_> = roster
        .operators()
        .iter()
        .filter(|gm| gm.id != NATIVE_GM_OPERATOR_ID)
        .cloned()
        .collect();
    if state.enabled {
        rows.push(GmOperator {
            id: NATIVE_GM_OPERATOR_ID.into(),
            name: "GM".into(),
            connected: false,
            ready: false,
        });
    }
    let Ok(replacement) = GmRoster::try_new(rows) else {
        return;
    };
    if *roster != replacement {
        outbound.write(crate::lobby::OutboundMessage {
            target: crate::lobby::Target::All,
            delivery: crate::core::messages::DeliveryClass::Reliable,
            msg: crate::core::messages::ServerMessage::GmRosterChanged {
                gms: replacement.projection(),
            },
        });
        *roster = replacement;
    }
}

fn drain_records(world: &mut World) {
    let Some(surface) = world.get_resource::<NativeGmSurface>().cloned() else {
        return;
    };
    for json in surface.bridge.take_records() {
        let Some(record) = codec::decode_native_gm_record(&json) else {
            continue;
        };
        if !world.resource::<NativeGmLifecycle>().enabled {
            continue;
        }
        match record {
            NativeGmRecord::RecoveryHostLobby => {
                recovery::request(world);
            }
            NativeGmRecord::SurfaceFault => {
                surface.bridge.fault();
                break;
            }
            NativeGmRecord::Loaded => surface.bridge.mark_live(),
            NativeGmRecord::Ready { ready }
                if world.resource::<State<GamePhase>>().get() == &GamePhase::Lobby =>
            {
                world.resource_mut::<NativeGmLifecycle>().ready = ready;
            }
            NativeGmRecord::ForceStart
                if surface.bridge.live()
                    && world.resource::<State<GamePhase>>().get() == &GamePhase::Lobby =>
            {
                world
                    .resource_mut::<start::NativeGmStartRequests>()
                    .request();
            }
            NativeGmRecord::Action { request } => {
                let Some(request) = codec::decode_gm_action_request(&request) else {
                    continue;
                };
                if let Err(reason) = crate::gm_action::submit_native(world, request.clone()) {
                    let tick = world.resource::<crate::sim_tick::SimTick>().0;
                    world
                        .resource_mut::<crate::gm_action::LocalGmActionRefusals>()
                        .push(crate::gm_action::LoggedGmAction::refused_request(
                            &request, tick, reason,
                        ));
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_presence(
    surface: Res<NativeGmSurface>,
    layout: Option<Res<BridgeLayoutResource>>,
    phase: Res<State<GamePhase>>,
    mut state: ResMut<NativeGmLifecycle>,
    mut authority: ResMut<NativeGmAuthority>,
    mut roster: ResMut<GmRoster>,
    mut commands: Commands,
    mut paused: ResMut<crate::gm_action::SimulationPaused>,
    config: Option<Res<crate::world::config::WorldConfig>>,
    mut outbound: MessageWriter<crate::lobby::OutboundMessage>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    starts: Option<Res<start::NativeGmStartRequests>>,
) {
    let assigned = layout.as_ref().and_then(|l| l.layout.game_master_monitor());
    if *phase.get() == GamePhase::Lobby {
        state.enabled = assigned.is_some();
        state.desired_monitor = assigned.cloned();
        authority.screen_pause = false;
        state.lost = false;
    } else if assigned.is_some() {
        // An explicit --solo launch may enter its mission before the display
        // adapter adopts the saved bridge layout on its first rendered frame.
        state.enabled = true;
        state.desired_monitor = assigned.cloned();
    }
    let present = assigned.is_some_and(|id| {
        layout
            .as_ref()
            .is_some_and(|l| l.monitors.iter().any(|m| &m.identity == id))
    });
    let failed = surface.bridge.take_failure();
    let connected = state.enabled && present && surface.bridge.live() && !surface.bridge.failed();
    if state.enabled {
        commands.insert_resource(crate::gm_projection::NativeGmPresentation);
    } else {
        commands.remove_resource::<crate::gm_projection::NativeGmPresentation>();
    }
    if !connected || failed {
        state.ready = false;
    }
    if state.enabled
        && (!present || failed || surface.bridge.failed())
        && *phase.get() == GamePhase::InProgress
        && !state.lost
    {
        paused.0 = true;
        authority.screen_pause = true;
        state.lost = true;
    }
    if connected {
        state.lost = false;
    }
    authority.connected = connected;
    let mut rows: Vec<_> = roster
        .operators()
        .iter()
        .filter(|gm| gm.id != NATIVE_GM_OPERATOR_ID)
        .cloned()
        .collect();
    if state.enabled {
        rows.push(GmOperator {
            id: NATIVE_GM_OPERATOR_ID.into(),
            name: "GM".into(),
            connected,
            ready: state.ready,
        });
    }
    let Ok(replacement) = GmRoster::try_new(rows) else {
        authority.connected = false;
        return;
    };
    if *roster != replacement {
        outbound.write(crate::lobby::OutboundMessage {
            target: crate::lobby::Target::All,
            delivery: crate::core::messages::DeliveryClass::Reliable,
            msg: crate::core::messages::ServerMessage::GmRosterChanged {
                gms: replacement.projection(),
            },
        });
        *roster = replacement;
    }
    if let Ok(json) = codec::encode_native_gm_metadata(&NativeGmMetadata {
        phase: phase.get().clone(),
        host_lobby_unavailable: recovery::host_lobby_unavailable(
            phase.get(),
            state.enabled,
            layout.as_deref(),
        ),
        role_presets: config
            .as_ref()
            .map(|c| c.gm_role_presets.clone())
            .unwrap_or_default(),
        gms: roster.projection(),
        start_policy: start::readiness_totals(
            sessions
                .as_deref()
                .map_or_else(Default::default, |s| s.0.readiness_tally()),
            &roster,
        ),
        start_result: starts.as_deref().and_then(|s| s.last_result().cloned()),
    }) {
        surface.bridge.publish("metadata", json);
    }
}

#[allow(clippy::too_many_arguments)]
fn feed_projections(
    surface: Res<NativeGmSurface>,
    mut entity: MessageReader<GmEntityProjectionChanged>,
    mut activity: MessageReader<GmActivityFeedChanged>,
    mut station: MessageReader<GmStationProjectionChanged>,
    mut session: MessageReader<GmSessionChanged>,
    mut mission: MessageReader<GmMissionChanged>,
    mut spawn: MessageReader<GmSpawnChanged>,
    mut comms: MessageReader<GmCommsChanged>,
) {
    macro_rules! feed {
        ($reader:ident, $channel:literal, $encode:ident) => {
            if let Some(message) = $reader.read().last() {
                if let Ok(json) = codec::$encode(&message.payload) {
                    surface.bridge.publish($channel, json);
                }
            }
        };
    }
    feed!(entity, "gm_entity", encode_gm_entity_projection);
    feed!(activity, "gm_activity", encode_gm_activity_feed);
    feed!(station, "gm_station", encode_gm_station_projection);
    feed!(session, "gm_session", encode_gm_session_projection);
    feed!(mission, "gm_mission", encode_gm_mission_projection);
    feed!(spawn, "gm_spawn", encode_gm_spawn_projection);
    feed!(comms, "gm_comms", encode_gm_comms_projection);
}

#[cfg(test)]
mod tests {
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
}
