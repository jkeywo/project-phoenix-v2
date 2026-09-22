//! Real-engine contract for the private native GM document and shared workspace.
//!
//! Run from the repository root after building the matching host bundle:
//! `cargo test --features ultralight --test native_gm_ultralight -- --ignored --nocapture`.
//! The SDK and `dist/index.html` are required; monitors and controllers are not.
//! One test owns one renderer, because Ultralight supports one VM per process.
//! This verifies module loading, rendering and the actual private queue adapter;
//! authoritative GM admission and physical screen recovery have separate tests.

#![cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]

use std::time::{Duration, Instant};

use project_phoenix::core::{codec, messages::GamePhase};
use project_phoenix::delivery::args::{ClientSource, HostArgs};
use project_phoenix::delivery::serve::{HostServer, HostedDocuments, ShutdownSignal};
use project_phoenix::gm_action::{GmAction, GmSessionProjection, NATIVE_GM_OPERATOR_ID};
use project_phoenix::gm_projection::{
    GmEntityKind, GmEntityProjection, GmEntityProjectionPayload, GmEntityStatus, GmRadarAppearance,
};
use project_phoenix::gm_roster::{GmOperator, GmRoster};
use project_phoenix::native_host::native_gm::{
    bridge::NativeGmBridge, document, start::readiness_totals, NativeGmMetadata, NativeGmRecord,
};
use project_phoenix::native_host::panes::pane_thread::{PaneKind, PaneRuntime, PaneSpecOwned};
use project_phoenix::native_host::panes::ultralight::{
    stage_sdk, UltralightHost, UltralightPaneSurface,
};
use project_phoenix::native_host::panes::PaneId;
use vellum_ultralight::runtime::{RuntimeOptions, UltralightRuntime};

const PATIENCE: Duration = Duration::from_secs(30);
const SIZE: (u32, u32) = (1920, 1080);
const SURFACE: PaneId = PaneId(41);
const SHIP_ID: &str = "00000000-0000-4000-8000-000000000041";

struct Delivery {
    addr: String,
    documents: HostedDocuments,
    shutdown: ShutdownSignal,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Delivery {
    fn start() -> Self {
        let server = HostServer::bind(&HostArgs {
            workshop: None,
            addr: "127.0.0.1:0".into(),
            client: ClientSource::Bundled { dir: "dist".into() },
            manifest: "assets/scenarios.toml".into(),
            content_dir: ".".into(),
            skip_bundle_check: true,
            sim: None,
            setup: false,
            test_output: None,
            meter_microphone: None,
            preview_camera: None,
            profile: None,
        })
        .expect("the local delivery host binds");
        let addr = server.local_addr();
        let documents = server.hosted_documents();
        server.enable_shutdown_polling().unwrap();
        let shutdown = ShutdownSignal::new();
        let serving = shutdown.clone();
        let thread = std::thread::spawn(move || {
            let _ = server.serve_until(serving, |_| {});
        });
        Self {
            addr,
            documents,
            shutdown,
            thread: Some(thread),
        }
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        self.shutdown.stop();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn frame(
    runtime: &mut UltralightHost,
    surface: &mut UltralightPaneSurface,
    bridge: &NativeGmBridge,
    records: &mut Vec<NativeGmRecord>,
) {
    runtime.update();
    surface.refresh_loaded();
    bridge.pump(SURFACE, surface);
    runtime.render();
    for json in bridge.take_records() {
        let record = codec::decode_native_gm_record(&json)
            .unwrap_or_else(|| panic!("the actual GM queue produced an invalid record: {json}"));
        if matches!(record, NativeGmRecord::Loaded) {
            bridge.mark_live();
        }
        records.push(record);
    }
}

fn wait_for(
    runtime: &mut UltralightHost,
    surface: &mut UltralightPaneSurface,
    bridge: &NativeGmBridge,
    records: &mut Vec<NativeGmRecord>,
    expression: &str,
) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        frame(runtime, surface, bridge, records);
        if surface
            .view_mut()
            .evaluate(&format!("String({expression})"))
            .ok()
            .as_deref()
            == Some("true")
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let diagnostic = surface.view_mut().evaluate(
        "JSON.stringify({ready:document.readyState,gm:typeof window.phoenixNativeGm,channels:typeof window.__phoenixNativeGmChannels,readyDisabled:document.getElementById('gm-ready-btn')?.disabled,map:document.getElementById('gm-entity-map')?.getBoundingClientRect().toJSON(),canvas:document.getElementById('gm-entity-map')?.shadowRoot?.querySelector('canvas')?.getBoundingClientRect().toJSON(),body:document.body.innerText.slice(0,1500)})",
    );
    panic!("GM workspace did not satisfy {expression}; engine diagnostic: {diagnostic:?}");
}

fn publish_metadata(bridge: &NativeGmBridge, phase: GamePhase) {
    let gms = GmRoster::try_new(vec![GmOperator::new(
        NATIVE_GM_OPERATOR_ID.into(),
        "Local GM".into(),
        true,
    )])
    .unwrap();
    bridge.publish(
        "metadata",
        codec::encode_native_gm_metadata(&NativeGmMetadata {
            phase,
            host_lobby_unavailable: false,
            local_operator_id: Some(NATIVE_GM_OPERATOR_ID.into()),
            role_presets: Vec::new(),
            gms: gms.projection(),
            start_policy: readiness_totals(Default::default(), &gms),
            start_result: None,
            ship_slots: Vec::new(),
        })
        .unwrap(),
    );
}

fn publish_ship(bridge: &NativeGmBridge, name: &str, x: f32, hull: u8) {
    let projection = GmEntityProjectionPayload {
        entities: vec![GmEntityProjection {
            removable: false,
            entity_id: SHIP_ID.into(),
            name: name.into(),
            kind: GmEntityKind::PlayerShip,
            position: [x, 2.0, -30.0],
            faction: None,
            status: GmEntityStatus {
                hull_percent: Some(hull),
                condition_percent: None,
                destroyed: false,
                hull_current_milli_hp: Some(u32::from(hull) * 1000),
                hull_max_milli_hp: Some(100_000),
                systems: Vec::new(),
            },
            current_target: None,
            geometry: None,
            radar: GmRadarAppearance {
                icon: Some("playerShip".into()),
                colour: Some([0.2, 0.8, 1.0]),
                size: Some(4.0),
                region_colour: None,
            },
        }],
        ..Default::default()
    };
    bridge.publish(
        "gm_entity",
        codec::encode_gm_entity_projection(&projection).unwrap(),
    );
}

fn publish_session(bridge: &NativeGmBridge, paused: bool) {
    bridge.publish(
        "gm_session",
        codec::encode_gm_session_projection(&GmSessionProjection {
            paused,
            results: Vec::new(),
            journal: Default::default(),
            factions: Vec::new(),
        })
        .unwrap(),
    );
}

#[test]
#[ignore = "needs the Ultralight SDK and matching trunk-built dist; no monitors or controllers required"]
fn native_gm_shared_workspace_loads_and_uses_the_private_engine_bridge() {
    eprintln!("{}", stage_sdk().expect("the Ultralight SDK is staged"));
    let host_html = std::fs::read_to_string("dist/index.html")
        .expect("build the matching host bundle with trunk build before this test");
    let delivery = Delivery::start();
    let path = document::document_path("engine-contract");
    delivery
        .documents
        .publish(path.clone(), document::build_document(&host_html));
    let bridge = NativeGmBridge::default();
    bridge.activate(SURFACE);
    let mut runtime = UltralightHost::new(
        UltralightRuntime::start(&RuntimeOptions::default()).expect("one real renderer starts"),
    );
    // The production factory chooses both the ephemeral session and the GM
    // drain script. Bypassing it would miss a broken PaneKind queue mapping.
    let mut surface = runtime
        .create(
            SURFACE,
            PaneKind::GameMaster,
            &PaneSpecOwned {
                width: SIZE.0,
                height: SIZE.1,
                device_scale: 1.0,
            },
            &format!("http://{}{path}", delivery.addr),
        )
        .expect("the actual GM surface loads over HTTP");
    let mut records = Vec::new();
    publish_metadata(&bridge, GamePhase::Lobby);
    publish_ship(&bridge, "Engine Test Ship", 10.0, 73);
    publish_session(&bridge, false);
    wait_for(&mut runtime, &mut surface, &bridge, &mut records,
        "document.documentElement.classList.contains('phoenix-gm-page') && typeof window.__hostIssueStationCommand === 'function' && typeof window.__hostTransmitComms === 'function' && customElements.get('ph-navigation-map') && document.getElementById('gm-entity-map').state.blips.length === 1 && document.getElementById('gm-entity-map').shadowRoot.querySelector('canvas').width > 0 && document.getElementById('gm-entity-map').shadowRoot.querySelector('canvas').height > 0 && !document.getElementById('gm-ready-btn').disabled");
    assert!(records
        .iter()
        .any(|record| matches!(record, NativeGmRecord::Loaded)));
    assert!(bridge.live());
    assert_eq!(surface.view_mut().evaluate(
        "String(['gm-map-panel','gm-inspector','gm-activity','gm-session-controls','gm-mission-panel','gm-spawn-panel','gm-station-controls','gm-knowledge-panel'].every(id => !!document.getElementById(id)))",
    ).unwrap(), "true", "the complete shared workspace was retained");
    let menu_contract = surface.view_mut().evaluate(
        "JSON.stringify({menus:document.querySelectorAll('.workshop-panel-switcher.is-menu-bar > details').length, misplaced:!!document.querySelector('.workshop-dock-canvas > .workshop-panel-switcher'), raw:(document.body.textContent.match(/server\\.gm\\.shell\\.layout\\.panel\\.[a-z_-]+/)||[])[0]||false})",
    ).unwrap();
    assert_eq!(
        menu_contract, r#"{"menus":6,"misplaced":false,"raw":false}"#,
        "the native desk uses categorised menus and resolves every panel label"
    );
    assert_eq!(surface.view_mut().evaluate(
        "String([...document.querySelectorAll('.workshop-tab-stack')].every(stack => stack.querySelector('.workshop-tab-list .workshop-dock-actions') && !stack.querySelector('.workshop-dock-panel > .workshop-panel-header')) && parseFloat(getComputedStyle(document.querySelector('.workshop-tab-list')).minHeight) < 40)",
    ).unwrap(), "true", "docked tabs and icon actions share one compact chrome row");
    assert_eq!(surface.view_mut().evaluate(
        "String(typeof window.phoenixPaneOut === 'undefined' && typeof window.__phoenixPaneApply === 'undefined' && typeof window.wasm_init === 'undefined')",
    ).unwrap(), "true", "neither the crew queue nor a second simulation boot is installed");

    surface
        .view_mut()
        .evaluate(&format!(
            "document.getElementById('gm-entity-map').navigationSelect({{uuid:'{SHIP_ID}'}})",
        ))
        .expect("the real map's public selection interaction runs");
    wait_for(&mut runtime, &mut surface, &bridge, &mut records,
        "document.getElementById('gm-entity-name').textContent === 'Engine Test Ship' && document.getElementById('gm-entity-hull').value === 73");
    publish_ship(&bridge, "Updated Engine Ship", 42.0, 39);
    wait_for(&mut runtime, &mut surface, &bridge, &mut records,
        "document.getElementById('gm-entity-name').textContent === 'Updated Engine Ship' && document.getElementById('gm-entity-hull').value === 39 && document.getElementById('gm-entity-map').state.blips[0].world_x === 42");

    records.clear();
    surface
        .view_mut()
        .evaluate("document.getElementById('gm-ready-btn').click()")
        .unwrap();
    frame(&mut runtime, &mut surface, &bridge, &mut records);
    assert!(records
        .iter()
        .any(|record| matches!(record, NativeGmRecord::Ready { ready: true })));

    publish_metadata(&bridge, GamePhase::InProgress);
    wait_for(&mut runtime, &mut surface, &bridge, &mut records,
        "document.getElementById('gm-ready-btn').disabled && document.getElementById('gm-session-state').dataset.paused === 'false' && !document.getElementById('gm-session-pause').disabled");
    records.clear();
    surface
        .view_mut()
        .evaluate("document.getElementById('gm-session-pause').click()")
        .unwrap();
    frame(&mut runtime, &mut surface, &bridge, &mut records);
    let actions: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            NativeGmRecord::Action { request } => Some(
                codec::decode_gm_action_request(request)
                    .expect("the shared session button emits the existing typed GM request"),
            ),
            _ => None,
        })
        .collect();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].operator_id, NATIVE_GM_OPERATOR_ID);
    assert_eq!(
        actions[0].action,
        GmAction::SetSessionPaused { active: true }
    );

    // Delivery of the authoritative result state updates the actual session
    // presenter; receipt itself must never invent a Resume request.
    records.clear();
    publish_session(&bridge, true);
    wait_for(&mut runtime, &mut surface, &bridge, &mut records,
        "document.getElementById('gm-session-state').dataset.paused === 'true' && !document.getElementById('gm-session-resume').disabled");
    assert!(!records
        .iter()
        .any(|record| matches!(record, NativeGmRecord::Action { .. })));
    let mut pixels = vec![0; (SIZE.0 * SIZE.1 * 4) as usize];
    assert!(surface
        .view_mut()
        .copy_frame(&mut pixels, true)
        .unwrap()
        .is_some());
    assert!(
        pixels.iter().any(|byte| *byte != 0),
        "the loaded workspace rasterizes"
    );
    exercise_live_station(&mut runtime, &mut surface, &bridge);
    bridge.close();
}

// A real native simulation and real Ultralight iframe in the same test process.
// Only monitor placement is omitted. Requests still leave the document through
// its private queue and enter the primary-GM submit_local admission boundary.
fn exercise_live_station(
    runtime: &mut UltralightHost,
    surface: &mut UltralightPaneSurface,
    bridge: &NativeGmBridge,
) {
    use bevy::prelude::*;
    use phoenix::gm_action::{GmActionId, GmActionRequest};
    use phoenix::native_host::session_role::{NativeSessionRole, NativeSessionRoleState};
    use project_phoenix as phoenix;
    let preload = phoenix::native_host::preload_content_templates(".").unwrap();
    let catalog = phoenix::delivery::serve::ManifestSource::read(".", "assets/scenarios.toml")
        .unwrap()
        .merged_catalog()
        .catalog;
    let mut config = phoenix::native_host::NativeHostConfig::lobby(catalog);
    config.surface = phoenix::boot::NativeRenderSurface::Contract;
    config.seed = Some(42);
    let mut app = phoenix::native_host::build_native_host_app(&config, &preload).unwrap();
    app.add_plugins(phoenix::gm_projection::GmProjectionPlugin);
    app.insert_resource(phoenix::gm_projection::NativeGmPresentation);
    let mut role = NativeSessionRoleState::default();
    role.request(NativeSessionRole::StandaloneGameMaster);
    app.insert_resource(role);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }
    app.world_mut()
        .write_message(phoenix::lobby::server::InboundMessage {
            token: "native-gm".into(),
            msg: phoenix::core::messages::ClientMessage::SelectScenario {
                scenario_id: "combat_test".into(),
            },
        });
    for _ in 0..90 {
        app.update();
    }
    phoenix::gm_action::submit_local(
        app.world_mut(),
        GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("engine-backfill").unwrap(),
            action: GmAction::BackfillShipSlot {
                slot: "player".into(),
            },
        },
    )
    .unwrap();
    for _ in 0..4 {
        app.update();
    }
    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::InProgress);
    for _ in 0..10 {
        app.update();
    }
    let uuid = app.world_mut().query_filtered::<&phoenix::entities::spawner::EntityUuid, With<phoenix::lockstep::FleetSlotOf>>()
        .iter(app.world()).next().unwrap().0.clone();
    assert!(!app
        .world()
        .contains_resource::<phoenix::lobby::SelectedShipResource>());
    let roster = app.world().resource::<GmRoster>();
    bridge.publish(
        "metadata",
        codec::encode_native_gm_metadata(&NativeGmMetadata {
            phase: GamePhase::InProgress,
            host_lobby_unavailable: false,
            local_operator_id: Some("gm-1".into()),
            role_presets: Vec::new(),
            gms: roster.projection(),
            start_policy: readiness_totals(Default::default(), roster),
            start_result: None,
            ship_slots: Vec::new(),
        })
        .unwrap(),
    );
    let mut live_frame = |surface: &mut UltralightPaneSurface, runtime: &mut UltralightHost| {
        runtime.update();
        surface.refresh_loaded();
        bridge.pump(SURFACE, surface);
        for json in bridge.take_records() {
            if let Some(NativeGmRecord::Action { request }) = codec::decode_native_gm_record(&json)
            {
                phoenix::gm_action::submit_local(
                    app.world_mut(),
                    codec::decode_gm_action_request(&request).unwrap(),
                )
                .expect("real iframe action is admitted through the primary-GM boundary");
            }
        }
        app.update();
        if let Some(message) = app
            .world_mut()
            .resource_mut::<Messages<phoenix::console_bridge::GmStationProjectionChanged>>()
            .drain()
            .last()
        {
            bridge.publish(
                "gm_station",
                codec::encode_gm_station_projection(&message.payload).unwrap(),
            );
        }
        if let Some(message) = app
            .world_mut()
            .resource_mut::<Messages<phoenix::console_bridge::GmEntityProjectionChanged>>()
            .drain()
            .last()
        {
            bridge.publish(
                "gm_entity",
                codec::encode_gm_entity_projection(&message.payload).unwrap(),
            );
        }
        runtime.render();
    };
    for _ in 0..5 {
        live_frame(surface, runtime);
    }
    surface.view_mut().evaluate(&format!("window.__hostGmConfirmationProfile.setMode('station.takeover','immediate'); window.__hostGmConfirmationProfile.setMode('station.release','immediate'); window.__hostGmConfirmationProfile.setMode('station.command','immediate'); window.__hostGmFocusStation('{uuid}','helm')")).unwrap();
    let mut await_condition = |expression: &str| {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            live_frame(surface, runtime);
            if surface
                .view_mut()
                .evaluate(&format!("String({expression})"))
                .ok()
                .as_deref()
                == Some("true")
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        panic!(
            "native live station did not satisfy {expression}: {:?}",
            surface
                .view_mut()
                .evaluate("document.getElementById('gm-station-connection').textContent")
        );
    };
    await_condition("document.getElementById('gm-station-frame').contentWindow && typeof document.getElementById('gm-station-frame').contentWindow.sendAction === 'function' && document.getElementById('gm-station-connection').textContent.includes('Observation')");
    drop(await_condition);
    surface.view_mut().evaluate("window.__nativeReadings = []; const stationWindow = document.getElementById('gm-station-frame').contentWindow; const stationUpdate = stationWindow.__updateConsole; stationWindow.__updateConsole = function(id, json) { window.__nativeReadings.push(typeof json === 'string' ? JSON.parse(json).speed : json.speed); return stationUpdate.apply(this, arguments); };").unwrap();
    surface.view_mut().evaluate("window.__nativeStationFrame = document.getElementById('gm-station-frame'); document.getElementById('gm-station-toggle').click()").unwrap();
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        live_frame(surface, runtime);
        if surface
            .view_mut()
            .evaluate(
                "String(document.getElementById('gm-station-toggle').dataset.active === 'true')",
            )
            .unwrap()
            == "true"
        {
            break;
        }
    }
    assert_eq!(
        surface
            .view_mut()
            .evaluate(
                "String(document.getElementById('gm-station-toggle').dataset.active === 'true')"
            )
            .unwrap(),
        "true"
    );
    surface.view_mut().evaluate("document.getElementById('gm-station-frame').contentWindow.sendAction('set_helm_thrust', {value:0.65, correlation:'native-live-thrust'})").unwrap();
    for _ in 0..30 {
        live_frame(surface, runtime);
        std::thread::sleep(Duration::from_millis(16));
    }
    assert_eq!(
        surface
            .view_mut()
            .evaluate("String(new Set(window.__nativeReadings.filter(Number.isFinite)).size > 1)")
            .unwrap(),
        "true",
        "the real authored Helm console receives changing live speed readings"
    );
    surface
        .view_mut()
        .evaluate("window.__hostSetSessionPaused(true, 'native-pause-before-release')")
        .unwrap();
    for _ in 0..5 {
        live_frame(surface, runtime);
    }
    surface
        .view_mut()
        .evaluate("document.getElementById('gm-station-toggle').click()")
        .unwrap();
    for _ in 0..15 {
        live_frame(surface, runtime);
    }
    assert_eq!(surface.view_mut().evaluate("String(document.getElementById('gm-station-toggle').dataset.active === 'false' && document.getElementById('gm-station-frame') === window.__nativeStationFrame)").unwrap(), "true", "release hands back without replacing the console");
    drop(live_frame);
    assert!(
        app.world()
            .resource::<phoenix::gm_action::SimulationPaused>()
            .0,
        "release remains possible with the simulation paused"
    );
    assert!(
        app.world()
            .resource::<phoenix::gm_action::GmActionLog>()
            .entries()
            .iter()
            .any(|row| row.correlation.as_str() == "native-live-thrust"
                && row.outcome == phoenix::gm_action::GmActionOutcome::Applied),
        "an action sent by the authentic native iframe reaches the authoritative journal"
    );
}
