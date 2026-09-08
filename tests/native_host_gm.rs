//! The real native composition operates a ship and a private GM together.
//! No SDK/GPU is needed: a recording surface exercises the actual local bridge.
//! This separate process owns the native content cache it populates.

use bevy::prelude::*;
use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::core::messages::GamePhase;
use project_phoenix::entities::spawner::EntityUuid;
use project_phoenix::gm_action::{
    GmActionJournal, GmActionLog, GmActionOutcome, SimulationPaused, NATIVE_GM_OPERATOR_ID,
};
use project_phoenix::gm_projection::BrowserGameMaster;
use project_phoenix::lockstep::{FleetLockstep, FleetRoster};
use project_phoenix::native_host::bridge_display::BridgeLayoutResource;
use project_phoenix::native_host::bridge_layout::{BridgeLayout, LayoutAction};
use project_phoenix::native_host::bridge_profile::{identify, RawMonitor};
use project_phoenix::native_host::host_lobby::LocalHostLobby;
use project_phoenix::native_host::native_gm::bridge::NativeGmBridge;
use project_phoenix::native_host::panes::{PaneId, RecordingSurface};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use project_phoenix::server_app::LocalShip;

const GM_PANE: PaneId = PaneId(900);

fn pump(app: &mut App, bridge: &NativeGmBridge, surface: &mut RecordingSurface, frames: usize) {
    for _ in 0..frames {
        app.update();
        bridge.pump(GM_PANE, surface);
    }
}

#[test]
fn native_ship_and_full_gm_projections_share_one_authoritative_world_and_action_journal() {
    let preload = preload_content_templates(".").expect("repository content preloads");
    let lobby = LocalHostLobby::open("127.0.0.1:8080");
    let bridge = lobby.gm_bridge.clone();
    let mut cfg = NativeHostConfig::new("assets/worlds/combat_test.toml");
    cfg.seed = Some(20260908);
    cfg.solo = true;
    cfg.surface = NativeRenderSurface::Contract;
    cfg.host_lobby = Some(lobby);
    let mut app =
        build_native_host_app(&cfg, &preload).expect("ship plus local GM native host assembles");
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
    app.insert_resource(BridgeLayoutResource {
        layout,
        monitors,
        notices: Vec::new(),
    });
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.finish();
    app.cleanup();
    bridge.activate(GM_PANE);
    let mut surface = RecordingSurface::ready();
    pump(&mut app, &bridge, &mut surface, 2);
    surface.queue_record(r#"{"kind":"loaded"}"#);
    bridge.pump(GM_PANE, &mut surface);
    pump(&mut app, &bridge, &mut surface, 120);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::InProgress
    );
    assert!(
        !app.world().contains_resource::<BrowserGameMaster>(),
        "the ship keeps its native boot identity"
    );
    assert!(!app.world().contains_resource::<FleetLockstep>());
    let fleet = app.world().resource::<FleetRoster>();
    assert!(fleet.is_solo());
    assert!(fleet.gms().is_empty(), "the local GM is no lockstep peer");
    let ship_ids: Vec<_> = app
        .world_mut()
        .query_filtered::<&EntityUuid, With<LocalShip>>()
        .iter(app.world())
        .map(|uuid| uuid.0.clone())
        .collect();
    assert_eq!(
        ship_ids.len(),
        1,
        "one real player ship remains in the authoritative world"
    );
    let scripts = surface.pushed.join("\n");
    for channel in [
        "gm_entity",
        "gm_station",
        "gm_session",
        "gm_activity",
        "gm_mission",
        "gm_spawn",
        "gm_comms",
    ] {
        assert!(
            scripts.contains(&format!("__phoenixNativeGmChannels.{channel}")),
            "real {channel} projection reaches the private surface"
        );
    }
    assert!(
        scripts.contains(&ship_ids[0]),
        "the GM inspects this simulation's real local ship"
    );
    assert!(scripts.contains(NATIVE_GM_OPERATOR_ID));
    assert!(app.world().resource::<GmActionJournal>().is_empty());
    let request = serde_json::json!({"operator_id": NATIVE_GM_OPERATOR_ID, "correlation": "native-contract-pause",
        "action": "set_session_paused", "active": true});
    surface.queue_record(
        serde_json::json!({"kind": "action", "request": request.to_string()}).to_string(),
    );
    bridge.pump(GM_PANE, &mut surface);
    pump(&mut app, &bridge, &mut surface, 3);
    assert!(app.world().resource::<SimulationPaused>().0);
    assert_eq!(app.world().resource::<GmActionJournal>().len(), 1);
    let fact = &app.world().resource::<GmActionLog>().entries()[0];
    assert_eq!(fact.operator_id, NATIVE_GM_OPERATOR_ID);
    assert_eq!(fact.outcome, GmActionOutcome::Applied);
    let stopped_tick = app
        .world()
        .resource::<project_phoenix::sim_tick::SimTick>()
        .0;
    pump(&mut app, &bridge, &mut surface, 10);
    assert_eq!(
        app.world()
            .resource::<project_phoenix::sim_tick::SimTick>()
            .0,
        stopped_tick
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .iter(app.world())
            .count(),
        1
    );
}
