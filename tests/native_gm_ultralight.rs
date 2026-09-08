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
const SIZE: (u32, u32) = (1440, 1000);
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
            addr: "127.0.0.1:0".into(),
            client: ClientSource::Bundled { dir: "dist".into() },
            manifest: "assets/scenarios.toml".into(),
            content_dir: ".".into(),
            skip_bundle_check: true,
            sim: None,
            setup: false,
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
        "JSON.stringify({ready:document.readyState,gm:typeof window.phoenixNativeGm,channels:typeof window.__phoenixNativeGmChannels,body:document.body.innerText.slice(0,1500)})",
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
            role_presets: Vec::new(),
            gms: gms.projection(),
            start_policy: readiness_totals(Default::default(), &gms),
            start_result: None,
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
        "document.documentElement.classList.contains('phoenix-gm-page') && typeof window.__hostIssueStationCommand === 'function' && typeof window.__hostTransmitComms === 'function' && customElements.get('ph-navigation-map') && document.getElementById('gm-entity-map').state.blips.length === 1 && !document.getElementById('gm-ready-btn').disabled");
    assert!(records
        .iter()
        .any(|record| matches!(record, NativeGmRecord::Loaded)));
    assert!(bridge.live());
    assert_eq!(surface.view_mut().evaluate(
        "String(['gm-map-panel','gm-inspector','gm-activity','gm-session-controls','gm-mission-panel','gm-spawn-panel','gm-station-controls','gm-knowledge-panel'].every(id => !!document.getElementById(id)))",
    ).unwrap(), "true", "the complete shared workspace was retained");
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
    bridge.close();
}
