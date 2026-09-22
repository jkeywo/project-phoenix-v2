//! Current-working-tree native GM measurement harness (not a shipping route).
//! Run with --profile measure --features ultralight; PHOENIX_FRAME_CAPTURE must name a new file.
#[cfg(feature = "ultralight")]
#[path = "profile_systems/timing.rs"]
mod timing;
#[cfg(not(feature = "ultralight"))]
fn main() {
    eprintln!("This measurement requires --features ultralight");
}

#[cfg(feature = "ultralight")]
fn main() {
    use bevy::prelude::*;
    use project_phoenix::{
        boot::NativeRenderSurface,
        delivery::{
            args::{ClientSource, HostArgs},
            serve::{HostServer, ManifestSource, ShutdownSignal},
        },
        native_host::{
            self,
            host_lobby::LocalHostLobby,
            panes::LocalPanes,
            session_role::{NativeSessionRole, NativeSessionRoleState},
        },
    };
    use std::io::{Read, Write};
    native_host::panes::ultralight::stage_sdk().expect("SDK staging");
    let capture =
        std::path::PathBuf::from(std::env::var_os("PHOENIX_FRAME_CAPTURE").expect("capture path"));
    let markers = capture.with_file_name("stages.jsonl");
    let mut marker_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(markers)
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let marker_address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0u8; 8192];
            let count = stream.read(&mut bytes).unwrap_or(0);
            let request = String::from_utf8_lossy(&bytes[..count]);
            let path = request.split_whitespace().nth(1).unwrap_or("");
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            #[derive(serde::Serialize)]
            struct Marker<'a> {
                #[serde(rename = "unixMs")]
                unix_ms: u128,
                path: &'a str,
            }
            writeln!(
                marker_file,
                "{}",
                project_phoenix::core::codec::encode_presentation_capture(&Marker {
                    unix_ms: now,
                    path
                },)
                .unwrap()
            )
            .unwrap();
            marker_file.flush().unwrap();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK");
        }
    });
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
    .unwrap();
    let documents = server.hosted_documents();
    let html = std::fs::read_to_string("dist/index.html").unwrap();
    let lobby = LocalHostLobby::open(server.local_addr());
    lobby.publish(&html, &documents).unwrap();
    let stage = std::env::var("PHOENIX_GM_PROFILE_STAGE").expect("profile stage");
    assert!([
        "idle",
        "running",
        "observed",
        "controlled",
        "raster",
        "player",
        "multi"
    ]
    .contains(&stage.as_str()));
    let multi = stage == "multi";
    let player = stage == "player" || multi;
    let script = std::fs::read_to_string("examples/profile_gm.js")
        .expect("measurement driver")
        .replace("MARKER_ORIGIN", &format!("http://{marker_address}"))
        .replace("PROFILE_STAGE", if multi { "running" } else { &stage });
    let script = if multi {
        script.replace("stage, ...extra", "stage: 'gm-' + stage, ...extra")
    } else {
        script
    };
    // Windows known-folder resolution can ignore APPDATA overrides. This
    // benchmark alone uses an in-memory preference adapter: no user profile
    // migration/read/write, while the ordinary layout and storage reply API run.
    let gm = native_host::native_gm::document::build_document(&html).replace(
        "window.PhoenixInstallNativeOperatorStorage(record => window.phoenixNativeGmOperatorOut.send(record));",
        "window.PhoenixInstallNativeOperatorStorage((() => { let profile = null; return record => { const request = JSON.parse(record); if (request.operation === 'save') profile = request.profile; setTimeout(() => window.__phoenixOperatorReply({operation: request.operation, status:'ok', profile}), 0); return true; }; })());",
    ).replace(
        "</body>",
        &format!("<script type=\"module\">{script}</script></body>"),
    );
    documents.publish(
        native_host::native_gm::document::document_path(&lobby.nonce),
        gm,
    );
    let names = if player {
        vec!["Performance pilot".into()]
    } else {
        vec![]
    };
    let panes = LocalPanes::open(&names, server.local_addr());
    let player_script = std::fs::read_to_string("examples/profile_player.js")
        .expect("player measurement driver")
        .replace("MARKER_ORIGIN", &format!("http://{marker_address}"))
        .replace("PROFILE_STAGE", &stage);
    panes
        .publish(
            &std::fs::read_to_string("dist/client/index.html")
                .unwrap()
                .replace(
                    "</body>",
                    &format!(
                        "<script type=\"module\">{}</script></body>",
                        if player { &player_script } else { "" }
                    ),
                ),
            &documents,
        )
        .unwrap();
    server.enable_shutdown_polling().unwrap();
    let shutdown = ShutdownSignal::new();
    let serving = shutdown.clone();
    let delivery = std::thread::spawn(move || {
        let _ = server.serve_until(serving, |_| {});
    });
    let catalog = ManifestSource::read(".", "assets/scenarios.toml")
        .unwrap()
        .merged_catalog()
        .catalog;
    let mut cfg = native_host::NativeHostConfig::lobby(catalog);
    if player {
        cfg = native_host::NativeHostConfig::new("assets/worlds/combat_test.toml");
        cfg.ship_path = Some("assets/entities/alliance_destroyer.toml".into());
        cfg.solo = true;
    }
    cfg.surface = NativeRenderSurface::Window;
    cfg.seed = Some(42);
    cfg.frame_stats = true;
    cfg.remember_layout = false;
    cfg.log = project_phoenix::logging::parse_log_spec("info").unwrap();
    cfg.log_spec = "info".into();
    cfg.host_lobby = if player && !multi { None } else { Some(lobby) };
    if multi {
        cfg.solo = false;
        let path = std::env::var("PHOENIX_GM_BRIDGE_PROFILE").expect("three-monitor profile");
        cfg.bridge_profile = Some(
            native_host::bridge_profile::BridgeProfile::from_toml(
                &std::fs::read_to_string(path).expect("read bridge profile"),
            )
            .expect("parse bridge profile")
            .validate()
            .expect("validate bridge profile"),
        );
    }
    cfg.panes = Some(panes);
    let preload = native_host::preload_content_templates(".").unwrap();
    let mut app = native_host::build_native_host_app(&cfg, &preload).unwrap();
    if !player {
        app.world_mut()
            .resource_mut::<NativeSessionRoleState>()
            .request(NativeSessionRole::StandaloneGameMaster);
    }
    let mut windows = app.world_mut().query::<&mut Window>();
    for mut window in windows.iter_mut(app.world_mut()) {
        window.resolution =
            bevy::window::WindowResolution::new(1920, 1080).with_scale_factor_override(1.0);
        window.title = "Phoenix — GM performance capture (1920 × 1080)".into();
        window.resizable = false;
    }
    if !player {
        app.add_systems(Update, |world: &mut World, mut sent: Local<bool>| {
            if !*sent {
                world.write_message(project_phoenix::lobby::server::InboundMessage {
                    token: "native-gm".into(),
                    msg: project_phoenix::core::messages::ClientMessage::SelectScenario {
                        scenario_id: "combat_test".into(),
                    },
                });
                *sent = true;
            }
        });
    }
    // Diagnostics wrap the existing systems; acceptance has no wrappers or
    // per-frame file IO. Both retain lightweight workload samples in memory.
    use std::sync::{atomic::Ordering, Arc, Mutex};
    let frame_capture =
        project_phoenix::perf::native_frames::NativeFrameCapture::install_from_environment(
            &mut app,
        )
        .unwrap()
        .unwrap();
    let epoch = frame_capture.clock_origin();
    let surface_capture =
        native_host::panes::surface_stats::SurfaceCapture::install_from_environment(
            &mut app,
            Some((epoch, frame_capture.started_unix_ms())),
        )
        .unwrap();
    let mut control = timing::Control::new(20_000);
    control.epoch = epoch;
    control.window = Some((
        std::time::Duration::from_secs(40),
        std::time::Duration::from_secs(125),
    ));
    let control = Arc::new(control);
    let rows = Arc::new(Mutex::new(Vec::<Arc<timing::Row>>::new()));
    if std::env::var_os("PHOENIX_GM_DIAGNOSTIC").is_some_and(|v| v == "1") {
        struct Timers {
            control: Arc<timing::Control>,
            rows: Arc<Mutex<Vec<Arc<timing::Row>>>>,
        }
        impl Plugin for Timers {
            fn build(&self, _: &mut App) {}
            fn finish(&self, app: &mut App) {
                self.rows.lock().unwrap().extend(timing::instrument_world(
                    app.world_mut(),
                    self.control.clone(),
                ));
            }
        }
        app.add_plugins(Timers {
            control: control.clone(),
            rows: rows.clone(),
        });
    }
    #[derive(serde::Serialize)]
    struct Workload {
        seconds: f64,
        tick: u64,
        virtual_seconds: f64,
        entities: usize,
        ecs_entities: usize,
        ships: usize,
    }
    let samples = Arc::new(Mutex::new(Vec::<Workload>::new()));
    let recording = samples.clone();
    let clock = control.clone();
    app.add_systems(Last, move |world: &mut World, mut next: Local<f64>| {
        clock.update.fetch_add(1, Ordering::Relaxed);
        let seconds = epoch.elapsed().as_secs_f64();
        if seconds < *next {
            return;
        }
        *next = seconds + 1.0;
        let mut ships = world.query_filtered::<Entity, With<project_phoenix::server_app::Ship>>();
        let mut mission_entities = world.query_filtered::<Entity, Or<(
            With<project_phoenix::entities::spawner::EntityUuid>,
            With<project_phoenix::server_app::AsteroidUuid>,
        )>>();
        let mut all_entities = world.query::<Entity>();
        recording.lock().unwrap().push(Workload {
            seconds,
            tick: world.resource::<project_phoenix::sim_tick::SimTick>().0,
            virtual_seconds: world.resource::<Time<Virtual>>().elapsed_secs_f64(),
            entities: mission_entities.iter(world).count(),
            ecs_entities: all_entities.iter(world).count(),
            ships: ships.iter(world).count(),
        });
    });
    let exit = app.run();
    if let Some(surfaces) = surface_capture {
        surfaces.finish(&exit).unwrap();
    }
    frame_capture.finish(&exit).unwrap();
    #[derive(serde::Serialize)]
    struct Diagnostics<'a> {
        systems: Vec<&'a timing::Row>,
        workload: &'a [Workload],
        truncated: bool,
    }
    let rows = rows.lock().unwrap();
    let samples = samples.lock().unwrap();
    std::fs::write(
        capture.with_file_name("diagnostics.json"),
        project_phoenix::core::codec::encode_presentation_capture(&Diagnostics {
            systems: rows.iter().map(AsRef::as_ref).collect(),
            workload: &samples,
            truncated: control.truncated.load(Ordering::Relaxed),
        })
        .unwrap(),
    )
    .unwrap();
    shutdown.stop();
    delivery.join().unwrap();
}
