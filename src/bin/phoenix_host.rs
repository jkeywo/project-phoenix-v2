//! `phoenix-host` — serve the Phoenix client, manifest and catalogue natively,
//! and (since issue #1121, with `--world`) run the authoritative simulation
//! with a native Bevy/wgpu viewscreen.
//!
//! All the work lives in `project_phoenix::delivery` and
//! `project_phoenix::native_host` so it can be unit-tested; this is just argv
//! in, an operator log out, an exit code out.
//!
//! **One binary, two modes, on purpose.** Issue #1121 asked for the delivered
//! host to be *evolved* rather than duplicated, so PRD #855's bundle serving,
//! catalogue restriction and startup version pin are the same code whether or
//! not a world is running. `--world` adds a simulation to this process; every
//! existing flag keeps its exact meaning, and a bare invocation is byte-for-byte
//! the delivery host it always was.
//!
//! The two halves share the process like this: `HostServer` runs on a worker
//! thread and Bevy owns the main one, because winit requires the main thread on
//! Windows. `serve_until` gives the worker a shutdown path so a clean window
//! close ends both.
//!
//! Exit codes: 0 clean, 1 startup failure (unreadable content, an unusable
//! client bundle, a version-pin mismatch against the bundle, a port already in
//! use, a world that does not load), 2 bad arguments.
//!
//! Printing: `eprintln!` rather than the `plog!` family, on the same footing as
//! `phoenix-headless` — this is a CLI's own operator output, not simulation
//! logging. Once a world is running, the simulation's own `plog!` output goes
//! through the `LogFilterConfig` `--log` builds, exactly as headless does it.

// `required-features` in Cargo.toml gates on features, not targets, so
// `--features host --target wasm32-unknown-unknown` would otherwise try to
// build this against a `crate::delivery::serve` that does not exist there.
#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("phoenix-host is a native binary; it has no wasm32 build.");
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use project_phoenix::delivery::args::{parse_args, ClientSource, ParseOutcome, HELP};
    use project_phoenix::delivery::serve::{preload_templates, HostServer, ShutdownSignal};
    use project_phoenix::entities::template_preload::TemplatePreload;
    use project_phoenix::native_host;

    let mut args = match parse_args(std::env::args().skip(1)) {
        Ok(ParseOutcome::Help) => {
            print!("{HELP}");
            return;
        }
        Ok(ParseOutcome::Run(args)) => *args,
        Err(e) => {
            eprintln!("phoenix-host: {e}");
            eprintln!("try --help");
            std::process::exit(2);
        }
    };

    // Content roots first, before anything reads a file. An authoritative host
    // resolves content two independent ways — Bevy's asset root and the process
    // CWD — and half-loading is silent, so they are pinned together from the one
    // `--content-dir` the operator gave. Delivery-only keeps its historical
    // behaviour of resolving `--content-dir` per read, so nothing about a
    // pre-#1121 invocation changes.
    //
    // `--client-dir` is made absolute FIRST, while the old working directory is
    // still current: it is a sibling of the content tree in the shipped Windows
    // bundle, and re-rooting the process underneath a relative path would leave
    // the host serving 404s for a bundle that is right there.
    if args.sim.is_some() {
        if let ClientSource::Bundled { dir } = &args.client {
            match std::fs::canonicalize(dir) {
                Ok(absolute) => {
                    args.client = ClientSource::Bundled {
                        dir: absolute.to_string_lossy().into_owned(),
                    }
                }
                Err(e) => {
                    eprintln!("phoenix-host: --client-dir {dir:?}: {e}");
                    std::process::exit(1);
                }
            }
        }
        match native_host::pin_content_root(&args.content_dir) {
            Ok(root) => eprintln!("phoenix-host: content root {}", root.display()),
            Err(e) => {
                eprintln!("phoenix-host: {e}");
                std::process::exit(1);
            }
        }
    }

    // The template preload, run EXACTLY ONCE (both walks write the same
    // process-global cache). An authoritative host takes the strict walk — the
    // one that sorts, validates the model-marker contract, and refuses to
    // succeed having cached nothing — because the simulation reads that cache
    // with no filesystem fallback. Delivery-only keeps the lenient walk, whose
    // only job is enriching the published catalogue with each hull's class and
    // rating; a missing field there is cosmetic.
    let sim_preload: Option<TemplatePreload> = match &args.sim {
        Some(_) => match native_host::preload_content_templates(".") {
            Ok(preload) => {
                eprintln!(
                    "phoenix-host: preloaded {} entity templates",
                    preload.loaded()
                );
                Some(preload)
            }
            Err(e) => {
                eprintln!("phoenix-host: {e}");
                std::process::exit(1);
            }
        },
        None => {
            match preload_templates(&args.content_dir) {
                Ok(0) => eprintln!(
                    "phoenix-host: no entity templates under {}/assets/entities — the catalogue \
                     will publish hull paths without class/rating metadata",
                    args.content_dir
                ),
                Ok(n) => eprintln!("phoenix-host: preloaded {n} entity templates"),
                Err(e) => eprintln!("phoenix-host: template preload failed: {e}"),
            }
            None
        }
    };

    // Everything from here down is PRD #855's, unchanged: the startup bundle pin
    // runs before the bind so a host that will refuse every client does not
    // first take the port.
    let bound = bind_args(&args);
    let server = match HostServer::bind(&bound) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("phoenix-host: {e}");
            std::process::exit(1);
        }
    };

    let content = server.content();
    eprintln!(
        "phoenix-host: serving {} — content {:?} epoch {}, protocol {}",
        content.manifest.manifest_path,
        content.manifest.stamp.content_id,
        content.manifest.stamp.content_epoch,
        content.manifest.stamp.protocol,
    );
    for scenario in &content.manifest.scenarios {
        let id = scenario
            .get("id")
            .and_then(project_phoenix::delivery::payload::PayloadValue::as_text)
            .unwrap_or("?");
        eprintln!(
            "phoenix-host:   scenario {id} ({} hulls)",
            scenario.ships().len()
        );
    }
    for finding in &content.findings {
        eprintln!("phoenix-host: manifest finding {finding}");
    }
    match &args.client {
        ClientSource::Bundled { dir } => eprintln!("phoenix-host: client bundle {dir}"),
        ClientSource::Hosted => {
            eprintln!("phoenix-host: no client bundle — manifest endpoints only")
        }
    }

    let Some(sim) = args.sim.clone() else {
        // Delivery only: this loop IS the process, so it keeps the blocking
        // accept it has always had.
        server.serve_forever(log_event);
        return;
    };

    // The authoritative half. Assemble before the serving thread starts, so a
    // world that does not load fails at the prompt rather than after a window
    // and a listener are up.
    let preload = sim_preload.expect("a simulation run preloads its templates");
    let mut cfg = native_host::NativeHostConfig::new(sim.world.clone());
    cfg.ship_path = sim.ship.clone();
    cfg.seed = sim.seed;
    cfg.solo = sim.solo;
    cfg.log_spec = sim.log_spec.clone();
    cfg.surface = project_phoenix::boot::NativeRenderSurface::Window;
    // The catalogue restriction applies to what this process FLIES as well as to
    // what it publishes (issue #917's native half): with a curating `--manifest`
    // in force the default hull is drawn from that manifest's allowlist, so one
    // host cannot serve a curated catalogue and simultaneously run something
    // outside it. An explicit `--ship` still wins, as `?ship=` does in the
    // browser. Read from the manifest the server actually loaded, by the same
    // path, so the two cannot name different files.
    let manifest_path = std::path::Path::new(&bound.content_dir).join(&bound.manifest);
    cfg.curated_ships = match std::fs::read_to_string(&manifest_path) {
        Ok(toml) => native_host::curated_hulls_for_world(&toml, &sim.world),
        Err(e) => {
            // Unreachable in practice: `HostServer::bind` above already read and
            // parsed this exact file, so a failure here is a race with something
            // editing it mid-start. Unrestricted is the pre-#1121 answer.
            eprintln!(
                "phoenix-host: cannot re-read {}: {e} — the default hull is not \
                 restricted to the curated catalogue",
                manifest_path.display()
            );
            Vec::new()
        }
    };
    if !cfg.curated_ships.is_empty() {
        eprintln!(
            "phoenix-host: curated catalogue restricts the default hull to {}",
            cfg.curated_ships.join(", ")
        );
    }
    cfg.log = match project_phoenix::logging::parse_log_spec(&sim.log_spec) {
        Ok(log) => log,
        Err(e) => {
            eprintln!("phoenix-host: --log: {e}");
            std::process::exit(2);
        }
    };
    if !sim.log_entity.is_empty() {
        cfg.log.entity_filter = project_phoenix::logging::parse_log_entities(&sim.log_entity);
    }

    // Local Station panes (issue #1122). Opened AFTER the bind, because a pane's
    // document is published at this listener's own address and a `:0` bind does
    // not know its port until it has bound — then published into the delivery
    // server's own in-memory documents, so a pane loads the client bundle this
    // process is already serving, same origin, at the client directory's own
    // depth.
    //
    // The bus is kept here as well as handed to the app, so the shutdown at the
    // bottom of `main` can withdraw the pane documents: the delivery thread
    // outlives `App::run()` by however long the join takes, and nothing should
    // be serving a bridge console in that window.
    let mut pane_bus: Option<native_host::panes::PaneBus> = None;
    if !sim.panes.is_empty() {
        if !cfg!(feature = "ultralight") {
            eprintln!(
                "phoenix-host: --pane needs a build with --features ultralight (this one has \
                 none), because a local Station pane is an embedded browser view. Rebuild \
                 with `cargo build --release --features ultralight --bin phoenix-host`."
            );
            std::process::exit(2);
        }
        let panes = native_host::panes::LocalPanes::open(&sim.panes, server.local_addr());
        let index = match &args.client {
            ClientSource::Bundled { dir } => {
                std::path::Path::new(dir).join("client").join("index.html")
            }
            ClientSource::Hosted => unreachable!("--pane requires --client-dir; parse_args gates"),
        };
        let html = match std::fs::read_to_string(&index) {
            Ok(html) => html,
            Err(e) => {
                eprintln!(
                    "phoenix-host: cannot read {} for a pane: {e}",
                    index.display()
                );
                std::process::exit(1);
            }
        };
        if let Err(e) = panes.publish(&html, &server.hosted_documents()) {
            eprintln!(
                "phoenix-host: {} cannot become a pane document: {e}",
                index.display()
            );
            std::process::exit(1);
        }
        for pane in &panes.opened {
            // Deliberately NOT the pane's URL. It carries this participant's
            // whole session token in its fragment, and an operator log is a
            // file, a scrollback and a screenshot; the first eight characters
            // are enough to correlate a pane with what it says.
            eprintln!(
                "phoenix-host: {} is {} on token {}… at http://{}/",
                pane.id,
                pane.identity.name(),
                &pane.identity.token()[..8],
                panes.host_addr,
            );
        }
        #[cfg(feature = "ultralight")]
        match native_host::panes::ultralight::stage_sdk() {
            Ok(summary) => eprintln!("phoenix-host: {summary}"),
            Err(e) => {
                eprintln!("phoenix-host: {e}");
                std::process::exit(1);
            }
        }
        pane_bus = Some(panes.bus.clone());
        cfg.panes = Some(panes);
    }

    let app = match native_host::build_native_host_app(&cfg, &preload) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("phoenix-host: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "phoenix-host: authoritative simulation on {} — native viewscreen",
        sim.world
    );

    // Bevy owns the main thread (winit requires it on Windows); delivery moves
    // to a worker with a shutdown path so the window closing ends both.
    //
    // The poll seam is installed HERE, before the thread and before Bevy takes
    // the main thread for the rest of the process's life. A listener that cannot
    // be made non-blocking has no stop path, and a delivery thread with no stop
    // path turns a clean window close into a process that hangs on the join with
    // nothing on screen and port 8080 still held. Failing at the prompt is the
    // only honest answer.
    if let Err(e) = server.enable_shutdown_polling() {
        eprintln!(
            "phoenix-host: the delivery listener cannot be polled for shutdown ({e}), so an \
             authoritative host could not stop it when the window closes"
        );
        std::process::exit(1);
    }
    let shutdown = ShutdownSignal::new();
    let serving = shutdown.clone();
    let delivery = std::thread::Builder::new()
        .name("phoenix-host-delivery".to_string())
        .spawn(move || {
            if let Err(e) = server.serve_until(serving, log_event) {
                eprintln!("phoenix-host: delivery stopped: {e}");
            }
        });
    if let Err(e) = &delivery {
        eprintln!("phoenix-host: cannot start the delivery thread: {e}");
        std::process::exit(1);
    }

    native_host::run(app);

    // The window has closed, so no pane needs its document any more — and the
    // delivery thread is still up until the join below. Withdraw before the
    // stop signal, not after, so that window is empty rather than merely short.
    if let Some(bus) = &pane_bus {
        bus.withdraw_all();
    }
    shutdown.stop();
    if let Ok(handle) = delivery {
        let _ = handle.join();
    }
}

/// The operator log for one [`HostEvent`].
#[cfg(not(target_arch = "wasm32"))]
fn log_event(event: project_phoenix::delivery::serve::HostEvent) {
    use project_phoenix::delivery::serve::HostEvent;
    match event {
        HostEvent::Bound { addr } => eprintln!("phoenix-host: listening on http://{addr}/"),
        HostEvent::Served {
            method,
            path,
            status,
        } => eprintln!("phoenix-host: {status} {method} {path}"),
        HostEvent::Refused { path, code } => {
            eprintln!("phoenix-host: 409 {path} ({code})")
        }
        HostEvent::Failed { detail } => eprintln!("phoenix-host: {detail}"),
    }
}

/// The delivery half of a parsed invocation.
///
/// An authoritative host has already made `--content-dir` its working
/// directory, so the delivery half reads from `.` — the same tree, named the
/// way every other read in the process names it, and the same manifest and pin
/// PRD #855 always applied. Delivery-only leaves the value alone, so a
/// pre-#1121 invocation resolves exactly as it always did.
#[cfg(not(target_arch = "wasm32"))]
fn bind_args(
    args: &project_phoenix::delivery::args::HostArgs,
) -> project_phoenix::delivery::args::HostArgs {
    let mut out = args.clone();
    if args.sim.is_some() {
        out.content_dir = ".".to_string();
    }
    out
}
