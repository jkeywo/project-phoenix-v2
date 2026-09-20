//! Real engine, built shared UI and actual selected-root provider. No monitors,
//! crew identity, live simulation or browser filesystem shim.
#![cfg(all(feature = "ultralight", not(target_arch = "wasm32")))]

use project_phoenix::{
    delivery::{
        args::{ClientSource, HostArgs},
        serve::{HostServer, ShutdownSignal},
    },
    native_host::{
        panes::{
            pane_thread::{PaneInput, PaneKind, PaneRuntime, PaneSpecOwned, PaneView},
            ultralight::{stage_sdk, UltralightHost, UltralightPaneSurface},
            PaneId,
        },
        workshop::{
            bridge::{WorkshopBridge, WorkshopWorker},
            document,
        },
    },
    workshop::{
        provider::{NativeWorkshopProvider, WorkspaceKind},
        WorkshopDependencies,
    },
};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use vellum_ultralight::runtime::{RuntimeOptions, UltralightRuntime};

struct Fixture(PathBuf);
impl Fixture {
    #[allow(clippy::disallowed_methods)] // Private test workspace, never simulation identity.
    fn new() -> Self {
        let fixture = Self(
            std::env::temp_dir().join(format!("phoenix-workshop-engine-{}", uuid::Uuid::new_v4())),
        );
        fs::create_dir_all(fixture.0.join("project/assets/worlds")).unwrap();
        fs::create_dir_all(fixture.0.join("project/assets/models")).unwrap();
        fs::write(fixture.0.join("project/assets/scenarios.toml"), "[content]\nid='phoenix-base'\nepoch=1\n[[scenario]]\nid='test'\nworld='assets/worlds/test.toml'\n").unwrap();
        fs::write(
            fixture.0.join("project/assets/worlds/test.toml"),
            b"# Keep\r\n[global]\r\ntitle='Engine Test'\r\n",
        )
        .unwrap();
        fs::copy(
            "assets/models/alliance_cruiser.glb",
            fixture
                .0
                .join("project/assets/models/native-preview-only.glb"),
        )
        .unwrap();
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Delivery {
    shutdown: ShutdownSignal,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Delivery {
    fn drop(&mut self) {
        self.shutdown.stop();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn wait_for(
    runtime: &mut UltralightHost,
    surface: &mut UltralightPaneSurface,
    bridge: &WorkshopBridge,
    pane: PaneId,
    expression: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        runtime.update();
        surface.refresh_loaded();
        bridge.pump(pane, surface);
        runtime.render();
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
    let diagnostic = surface.view_mut().evaluate("JSON.stringify({ready:document.readyState,reply:typeof window.__phoenixNativeWorkshopReply,body:document.body.innerText.slice(0,3000)})");
    panic!("Workshop did not satisfy {expression}: {diagnostic:?}");
}
fn create(runtime: &mut UltralightHost, pane: PaneId, url: &str) -> UltralightPaneSurface {
    runtime
        .create(
            pane,
            PaneKind::Workshop,
            &PaneSpecOwned {
                width: 1440,
                height: 1000,
                device_scale: 1.0,
            },
            url,
        )
        .unwrap()
}

#[test]
#[ignore = "needs the Ultralight SDK and built dist Workshop/preview targets; no physical monitors required"]
fn native_workshop_shared_page_saves_exact_source_and_restores_after_view_recreation() {
    eprintln!("{}", stage_sdk().unwrap());
    let fixture = Fixture::new();
    let provider = NativeWorkshopProvider::open(
        WorkspaceKind::Project,
        fixture.0.join("project"),
        fixture.0.join("private"),
        WorkshopDependencies::default(),
    )
    .unwrap();
    let worker = WorkshopWorker::spawn(provider).unwrap();
    let bridge = worker.bridge();
    let server = HostServer::bind(&HostArgs {
        addr: "127.0.0.1:0".into(),
        client: ClientSource::Bundled { dir: "dist".into() },
        manifest: "assets/scenarios.toml".into(),
        content_dir: ".".into(),
        skip_bundle_check: true,
        sim: None,
        workshop: None,
        setup: false,
        test_output: None,
        meter_microphone: None,
        preview_camera: None,
        profile: None,
    })
    .unwrap();
    let path = document::document_path("engine-test");
    server.hosted_documents().publish(
        path.clone(),
        document::build_document(&fs::read_to_string("dist/workshop.html").unwrap()).unwrap(),
    );
    let url = format!("http://{}{path}", server.local_addr());
    server.enable_shutdown_polling().unwrap();
    let shutdown = ShutdownSignal::new();
    let stopping = shutdown.clone();
    let _delivery = Delivery {
        shutdown,
        thread: Some(std::thread::spawn(move || {
            let _ = server.serve_until(stopping, |_| {});
        })),
    };
    let mut runtime =
        UltralightHost::new(UltralightRuntime::start(&RuntimeOptions::default()).unwrap());
    let pane = PaneId(51);
    bridge.activate(pane);
    let mut surface = create(&mut runtime, pane, &url);
    wait_for(&mut runtime, &mut surface, &bridge, pane, "!!document.getElementById('workshop-files') && document.getElementById('workshop-files').options.length === 2 && !document.getElementById('workshop-check').disabled");
    wait_for(
        &mut runtime,
        &mut surface,
        &bridge,
        pane,
        "typeof window.__phoenixNativeWorkshopReply === 'function'",
    );
    assert!(bridge.live());
    surface
        .view_mut()
        .evaluate(
            "import('./gui/strings.js').then(module => { window.__workshopTestT = module.t; })",
        )
        .unwrap();
    wait_for(
        &mut runtime,
        &mut surface,
        &bridge,
        pane,
        "typeof window.__workshopTestT === 'function'",
    );
    assert_eq!(surface.view_mut().evaluate("String(typeof window.wasm_init === 'undefined' && typeof window.phoenixPaneOut === 'undefined' && typeof window.__nativeGm === 'undefined')").unwrap(), "true");
    surface.view_mut().evaluate("var files = document.getElementById('workshop-files'); files.value='assets/worlds/test.toml'; files.dispatchEvent(new Event('change')); var source=document.getElementById('workshop-source'); source.value += '# saved edit\\n'; source.dispatchEvent(new Event('input')); document.getElementById('workshop-save').click()").unwrap();
    wait_for(
        &mut runtime,
        &mut surface,
        &bridge,
        pane,
        "document.getElementById('workshop-dirty').textContent === window.__workshopTestT('workshop.native_clean')",
    );
    assert_eq!(
        fs::read(fixture.0.join("project/assets/worlds/test.toml")).unwrap(),
        b"# Keep\r\n[global]\r\ntitle='Engine Test'\r\n# saved edit\r\n"
    );
    surface.view_mut().evaluate("var source=document.getElementById('workshop-source'); source.value += '# recovered edit\\n'; source.dispatchEvent(new Event('input'))").unwrap();
    wait_for(&mut runtime, &mut surface, &bridge, pane, "document.getElementById('workshop-recovery-status').textContent === window.__workshopTestT('workshop.native_recovery_saved')");
    // A lost view must not own the source lifetime. The same provider/worker
    // survives and the replacement offers explicit restore of chronological history.
    drop(surface);
    let pane = PaneId(52);
    bridge.activate(pane);
    let mut surface = create(&mut runtime, pane, &url);
    wait_for(&mut runtime, &mut surface, &bridge, pane, "!!document.getElementById('workshop-restore') && !document.getElementById('workshop-restore').hidden");
    surface
        .view_mut()
        .evaluate("document.getElementById('workshop-restore').click()")
        .unwrap();
    wait_for(
        &mut runtime,
        &mut surface,
        &bridge,
        pane,
        "document.getElementById('workshop-source').value.includes('# recovered edit')",
    );
    surface
        .view_mut()
        .evaluate("document.getElementById('workshop-source').focus()")
        .unwrap();
    surface.input(&PaneInput::WorkshopKey(
        project_phoenix::native_host::workshop::keyboard::WorkshopKey {
            code: "KeyZ".into(),
            key: "z".into(),
            pressed: true,
            repeat: false,
            ctrl_key: true,
            shift_key: false,
            alt_key: false,
            meta_key: false,
            text: None,
        },
    ));
    assert_eq!(surface.view_mut().evaluate("String(!document.getElementById('workshop-source').value.includes('# recovered edit'))").unwrap(), "true");
    // The model exists only under the selected temporary root. The embedded
    // preview must therefore fetch the host's immutable capture route; a fall
    // through to the built bundle cannot draw it.
    surface.view_mut().evaluate("document.querySelector('[data-layout-panel=\"models\"][data-layout-control=\"switcher\"]').click(); var model=document.getElementById('workshop-model'); model.value='assets/models/native-preview-only.glb'; model.dispatchEvent(new Event('change')); document.querySelector('[data-layout-panel=\"model-preview\"][data-layout-control=\"switcher\"]').click(); document.getElementById('workshop-model-preview-refresh').click()").unwrap();
    wait_for(
        &mut runtime,
        &mut surface,
        &bridge,
        pane,
        "/\\d/.test(document.getElementById('workshop-model-preview-stats').textContent) && !!document.querySelector('.workshop-model-preview-frame')",
    );
    let rect: serde_json::Value = serde_json::from_str(
        &surface.view_mut().evaluate("JSON.stringify((()=>{const r=document.querySelector('.workshop-model-preview-frame').getBoundingClientRect();return {x:Math.floor(r.x+r.width*.25),y:Math.floor(r.y+r.height*.25),w:Math.floor(r.width*.5),h:Math.floor(r.height*.5)}})())").unwrap(),
    ).unwrap();
    let mut pixels = vec![0; 1440 * 1000 * 4];
    runtime.render();
    assert!(surface
        .view_mut()
        .copy_frame(&mut pixels, true)
        .unwrap()
        .is_some());
    assert!(
        pixels
            .chunks_exact(4)
            .any(|pixel| pixel[0] > 40 && pixel[1] > 40 && pixel[2] > 40),
        "shared UI text rasterizes"
    );
    let (x, y, width, height) = (
        rect["x"].as_u64().unwrap() as usize,
        rect["y"].as_u64().unwrap() as usize,
        rect["w"].as_u64().unwrap() as usize,
        rect["h"].as_u64().unwrap() as usize,
    );
    let mut colours = std::collections::BTreeSet::new();
    for row in y..(y + height).min(1000) {
        for column in x..(x + width).min(1440) {
            let offset = (row * 1440 + column) * 4;
            colours.insert([pixels[offset], pixels[offset + 1], pixels[offset + 2]]);
            if colours.len() > 8 {
                break;
            }
        }
    }
    assert!(
        colours.len() > 1,
        "draft-only native preview rendered a flat frame"
    );
}
