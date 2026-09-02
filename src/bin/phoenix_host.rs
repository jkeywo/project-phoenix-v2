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
    use project_phoenix::delivery::serve::{
        preload_templates, HostServer, ManifestSource, ShutdownSignal,
    };
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

    // The bridge-display profile (issue #1123). Read here, BEFORE
    // `pin_content_root` re-roots the process at `--content-dir`, because the
    // profile is operator configuration resolved against the launch directory,
    // not content resolved against the content tree — reading it after the chdir
    // would look for it in the wrong place. Parsed now so a broken profile fails
    // at the prompt whether the run is `--setup` or authoritative.
    let bridge_profile = match &args.profile {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => {
                match project_phoenix::native_host::bridge_profile::BridgeProfile::from_toml(&text)
                {
                    Ok(profile) => Some(profile),
                    Err(e) => {
                        eprintln!("phoenix-host: --profile {path:?}: {e}");
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("phoenix-host: --profile {path:?}: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    // `--setup` (issue #1123): enumerate the connected monitors, print their
    // stable identities and geometry, validate the profile against them if one
    // was given, and exit. A standalone diagnostic — it opens no HTTP listener
    // and runs no world, so it short-circuits here before any content is read.
    if args.setup {
        std::process::exit(native_host::bridge_display::run_setup(bridge_profile));
    }

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
    // `--world` names the scenario up front; `--lobby` (issue #1326) waits for
    // one to be picked, and carries the catalogue this same process publishes
    // over HTTP so the picker and the catalogue cannot disagree. Both build the
    // same host — see `NativeHostConfig::world_path`.
    let mut cfg = match &sim.world {
        Some(world) => native_host::NativeHostConfig::new(world.clone()),
        None => {
            let manifest_source = match ManifestSource::read(&bound.content_dir, &bound.manifest) {
                Ok(source) => source,
                Err(e) => {
                    // Unreachable in practice: `HostServer::bind` above read
                    // and parsed this exact file.
                    eprintln!("phoenix-host: {e}");
                    std::process::exit(1);
                }
            };
            let merged = manifest_source.merged_catalog();
            for finding in &merged.findings {
                eprintln!(
                    "phoenix-host: lobby catalogue [{}] {}: {}",
                    finding.category, finding.source.reference, finding.message
                );
            }
            eprintln!(
                "phoenix-host: no --world — waiting in the lobby with {} scenario(s) to \
                 choose from",
                merged.catalog.scenarios.len()
            );
            native_host::NativeHostConfig::lobby(merged.catalog)
        }
    };
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
    //
    // A `--lobby` host has no world yet, so there is nothing to curate against
    // here; the allowlist comes from the chosen scenario's own catalogue entry
    // (`scenario_arbiter::curated_ships_for`), which IS the curated list — the
    // same list `build_catalog` filtered when it published the catalogue.
    let manifest_path = std::path::Path::new(&bound.content_dir).join(&bound.manifest);
    cfg.curated_ships = match (&sim.world, std::fs::read_to_string(&manifest_path)) {
        (None, _) => Vec::new(),
        (Some(world), Ok(toml)) => native_host::curated_hulls_for_world(&toml, world),
        (Some(_), Err(e)) => {
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

    // The bridge-display profile (issue #1123), validated here so a bad one — an
    // unknown role, a Station of three panes — fails at the prompt rather than
    // after a window and a listener are up. The density rule and role vocabulary
    // are checked in `validate`; the missing/changed-display reporting happens
    // once real monitors are known, inside `BridgeDisplayPlugin`.
    if let Some(profile) = &bridge_profile {
        match profile.validate() {
            Ok(validated) => {
                eprintln!(
                    "phoenix-host: bridge display profile — {} display(s), {} touch mapping(s), \
                     {} media surface(s)",
                    validated.displays.len(),
                    validated.touch.len(),
                    validated.media.surfaces.len(),
                );
                // A consented device share (issue #1126) is not an error, but it
                // is worth a line so the operator sees the contention they asked
                // for before relying on it.
                for warning in &validated.media.warnings {
                    eprintln!("phoenix-host: --profile media: {warning}");
                }
                cfg.bridge_profile = Some(validated);
            }
            Err(e) => {
                eprintln!("phoenix-host: --profile: {e}");
                std::process::exit(1);
            }
        }
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
    //
    // Both embedded surfaces this host can show — a Station pane and the lobby
    // (issue #1325) — need the Ultralight SDK's shared libraries beside the
    // binary and its `resources/` in the working directory, so the staging is
    // done ONCE here rather than by whichever surface happens to be built first.
    // It reports rather than exits: a `--pane` cannot work without it and says
    // so below, while a host that only wanted the lobby flies its mission with a
    // blank surface, which is far better than refusing to start over chrome.
    //
    // Staged only when something can actually use it: a `--pane`, which IS an
    // embedded view, or a `--client-dir` bundle, which is the only thing the
    // lobby surface can be built from. A host with neither can never open an
    // embedded view, and copying DLLs and a `resources/` tree into the content
    // directory for a surface it will never show is a side effect it did not
    // ask for.
    #[allow(unused_mut)]
    let mut ultralight_ready = false;
    #[cfg(feature = "ultralight")]
    {
        if !sim.panes.is_empty() || matches!(args.client, ClientSource::Bundled { .. }) {
            match native_host::panes::ultralight::stage_sdk() {
                Ok(summary) => {
                    eprintln!("phoenix-host: {summary}");
                    ultralight_ready = true;
                }
                Err(e) => eprintln!("phoenix-host: {e}"),
            }
        }
    }

    // The pane bus. `--pane` opens one console per flag at boot, and since
    // issue #1331 the lobby's per-station screen rows open one on demand — so a
    // host with a client bundle gets a bus either way, armed with the same
    // document body a `--pane` loads. Without it a screen button would seat a
    // console the host has nothing to open.
    //
    // A build with no `ultralight` feature and a host with no bundle both get
    // `None`: the first can composite no view at all, and the second has no
    // client page to load into one. Neither has a lobby surface to press,
    // either, so neither can reach a screen row.
    let wants_consoles = ultralight_ready && matches!(args.client, ClientSource::Bundled { .. });
    let mut pane_bus: Option<native_host::panes::PaneBus> = None;
    if !sim.panes.is_empty() && !cfg!(feature = "ultralight") {
        eprintln!(
            "phoenix-host: --pane needs a build with --features ultralight (this one has \
             none), because a local Station pane is an embedded browser view. Rebuild \
             with `cargo build --release --features ultralight --bin phoenix-host`."
        );
        std::process::exit(2);
    }
    if !sim.panes.is_empty() || wants_consoles {
        // `LocalPanes::open(&[])` is a bus with no pane on it and every other
        // preparation done — which is exactly what a host whose consoles are all
        // still to be opened needs.
        let panes = native_host::panes::LocalPanes::open(&sim.panes, server.local_addr());
        let index = match &args.client {
            ClientSource::Bundled { dir } => {
                std::path::Path::new(dir).join("client").join("index.html")
            }
            ClientSource::Hosted => unreachable!("both arms above require a --client-dir bundle"),
        };
        match std::fs::read_to_string(&index) {
            Ok(html) => match panes.publish(&html, &server.hosted_documents()) {
                Ok(()) => {
                    for pane in &panes.opened {
                        // Deliberately NOT the pane's URL. It carries this
                        // participant's whole session token in its fragment, and
                        // an operator log is a file, a scrollback and a
                        // screenshot; the first eight characters are enough to
                        // correlate a pane with what it says.
                        eprintln!(
                            "phoenix-host: {} is {} on token {}… at http://{}/",
                            pane.id,
                            pane.identity.name(),
                            &pane.identity.token()[..8],
                            panes.host_addr,
                        );
                    }
                    // A pane IS an embedded browser view, so an unstaged SDK is
                    // fatal for it — the reason was printed by the staging
                    // above. A host that only wanted the screen rows carries on
                    // and simply cannot open one, like the lobby surface below.
                    if !sim.panes.is_empty() && !ultralight_ready {
                        std::process::exit(1);
                    }
                    pane_bus = Some(panes.bus.clone());
                    cfg.panes = Some(panes);
                }
                Err(e) => {
                    eprintln!(
                        "phoenix-host: {} cannot become a pane document: {e}",
                        index.display()
                    );
                    // Fatal for a `--pane`, which was asked for by name;
                    // reported for a host that merely wanted the screen rows.
                    if !sim.panes.is_empty() {
                        std::process::exit(1);
                    }
                }
            },
            Err(e) => {
                eprintln!(
                    "phoenix-host: cannot read {} for a console: {e}",
                    index.display()
                );
                if !sim.panes.is_empty() {
                    std::process::exit(1);
                }
            }
        }
    }

    // The host's own lobby surface (issue #1325). Opened after the bind, like a
    // pane, because its document is published at this listener's own address and
    // a `:0` bind does not know its port until it has bound.
    //
    // No flag: it is what a windowed authoritative host SHOWS before a mission,
    // and a native host has had nothing there since #1121. It needs two things,
    // and says so rather than failing when either is missing — a host that flies
    // the mission with a blank lobby is far better than one that refuses to
    // start over its chrome:
    //
    //   1. a build with `--features ultralight`, because the surface is an
    //      embedded browser view;
    //   2. a `--client-dir` bundle whose `index.html` is a Phoenix HOST page,
    //      because the lobby markup and the `gui/` modules that render it come
    //      out of that page rather than out of a copy in this binary.
    //
    // A ready SDK already implies the bundle: the staging above runs only for a
    // `--pane` or a `--client-dir`, and `--pane` itself requires `--client-dir`.
    // A bundle-less host stages nothing and was told "no client bundle —
    // manifest endpoints only" when it printed its client source, which is the
    // whole of why its viewscreen has no lobby.
    let mut host_lobby: Option<native_host::host_lobby::LocalHostLobby> = None;
    if ultralight_ready {
        if let ClientSource::Bundled { dir } = &args.client {
            let index = std::path::Path::new(dir).join("index.html");
            match std::fs::read_to_string(&index) {
                Ok(html) => {
                    let lobby = native_host::host_lobby::LocalHostLobby::open(server.local_addr());
                    match lobby.publish(&html, &server.hosted_documents()) {
                        Ok(()) => {
                            eprintln!(
                                "phoenix-host: the crew lobby is on the viewscreen (press F9 in play to show or hide it)"
                            );
                            host_lobby = Some(lobby);
                        }
                        Err(e) => eprintln!(
                            "phoenix-host: no lobby on the viewscreen — {}: {e}",
                            index.display()
                        ),
                    }
                }
                Err(e) => eprintln!(
                    "phoenix-host: no lobby on the viewscreen — cannot read {}: {e}",
                    index.display()
                ),
            }
        }
    }
    cfg.host_lobby = host_lobby.clone();
    // Kept for the shutdown below, before `server` moves into the delivery
    // thread: the thread outlives `App::run()` by however long the join takes,
    // and nothing should be serving a bridge surface in that window.
    let hosted_documents = server.hosted_documents();

    let mut app = match native_host::build_native_host_app(&cfg, &preload) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("phoenix-host: {e}");
            std::process::exit(1);
        }
    };

    // ── The crew paths ───────────────────────────────────────────────────────
    //
    // Two legs now, and a host may run either, both or neither:
    //
    //   direct  (issue #1353) crew reach this process on its OWN delivery port.
    //           No flag, no service, no internet: a phone loads the bundle from
    //           this host and opens its game socket back to the same origin.
    //   cloud   (issue #1113) `--rendezvous`, one outbound socket to the shared
    //           service, for play beyond one LAN.
    //
    // Both are `RelayTransport`s over the same frame vocabulary — the direct
    // leg's "socket" is `native_host::direct_join`, this process standing in for
    // the service — so everything about admission, projection and the delivery
    // classes is the same code on both, and a phone cannot tell them apart.
    let mut legs: Vec<Box<dyn project_phoenix::native_host::transport::NativeTransport>> =
        Vec::new();
    let mut relay_notices: Option<native_host::relay_transport::RelayNotices> = None;
    let mut direct_code: Option<project_phoenix::core::rendezvous::JoinCode> = None;

    // Direct accept is on whenever this host is BOTH serving a client bundle
    // and expecting a crew. The bundle is the load-bearing half: the client
    // rule is that a page dials the origin that served it
    // (`gui/join-url.js`), so a host serving no page has nobody to be dialled
    // BY — a phone would be reading the deployed bundle, which dials the cloud
    // service. `--solo` is the other half, and it means what it has always
    // meant: every station on Backfill, nobody joining.
    if !sim.solo {
        if let ClientSource::Bundled { .. } = &args.client {
            let table_path =
                std::path::Path::new(&bound.content_dir).join("assets/join/join-codes.toml");
            match native_host::join_codes::JoinCodeTable::read(&table_path)
                .and_then(native_host::direct_join::DirectJoinService::open)
            {
                Ok((service, code)) => {
                    // The gate before the service moves into its transport: one
                    // is the door on the delivery listener, the other is the
                    // wire the simulation talks over, and they share a record.
                    let gate = service.gate();
                    let transport = native_host::relay_transport::RelayTransport::new(
                        service,
                        native_host::relay_transport::RelayHostConfig {
                            namespace: "client".to_string(),
                            version: None,
                            stamp: content.manifest.stamp.clone(),
                        },
                    );
                    relay_notices = Some(transport.notices());
                    server.on_upgrade(std::sync::Arc::new(gate));
                    legs.push(Box::new(transport));
                    eprintln!(
                        "phoenix-host: crew join code {} — phones join this host directly, no \
                         service needed (full: {})",
                        code.suffix, code.full
                    );
                    direct_code = Some(code);
                }
                Err(e) => eprintln!(
                    "phoenix-host: LAN joining is off — {e}. Phones can still load the bundle; \
                     pass --rendezvous <URL> --origin <URL> to take crew over the cloud service."
                ),
            }
        }
    }

    // Installed as a resource into the seam `NativeTransportPlugin` already
    // registered — an `insert_resource`, exactly as
    // `native_host::transport`'s doc comment promised it would be.
    if let Some(base) = sim.rendezvous.as_deref() {
        let origin = sim
            .origin
            .as_deref()
            .expect("parse_args refuses --rendezvous without --origin");
        match native_host::relay_socket::WsRelaySocket::connect(base, origin) {
            Ok(socket) => {
                let mut transport = native_host::relay_transport::RelayTransport::new(
                    socket,
                    native_host::relay_transport::RelayHostConfig {
                        namespace: "client".to_string(),
                        version: None,
                        stamp: content.manifest.stamp.clone(),
                    },
                );
                // One notice queue however many legs there are: it is a channel
                // to the OPERATOR, and there is one operator with one terminal.
                match &relay_notices {
                    Some(shared) => transport.share_notices(shared.clone()),
                    None => relay_notices = Some(transport.notices()),
                }
                legs.push(Box::new(transport));
                eprintln!("phoenix-host: registering with {base} as {origin}");
            }
            Err(e) => {
                // Fail at the prompt. A host that silently carried on would sit
                // in a lobby nobody can enter, which is the exact experience
                // #1121 shipped and #1113 exists to end.
                eprintln!("phoenix-host: {e}");
                std::process::exit(1);
            }
        }
    } else if !sim.solo && direct_code.is_none() {
        eprintln!(
            "phoenix-host: nobody can join this host — it will wait in the lobby forever. Serve \
             a client bundle with --client-dir <DIR> for LAN joins, pass --rendezvous <URL> \
             --origin <URL> to take crew over the cloud service, or --solo to start with every \
             station on Backfill."
        );
    }

    // Whatever legs there are, driven as one transport (issue #1122's
    // `PairedTransport`, folded). PAIRED with the pane bus when there is one,
    // never inserted on top of it: `NativeTransportLink` is one resource, so a
    // plain insert would replace the bus `build_native_host_app` installed and
    // leave every local console on a joinable host talking to nothing.
    if !legs.is_empty() {
        use project_phoenix::native_host::transport::{
            NativeTransport, NativeTransportLink, PairedTransport,
        };
        let mut composed: Option<Box<dyn NativeTransport>> = pane_bus
            .as_ref()
            .map(|bus| Box::new(bus.transport()) as Box<dyn NativeTransport>);
        for leg in legs {
            composed = Some(match composed {
                Some(existing) => Box::new(PairedTransport::new(existing, leg)),
                None => leg,
            });
        }
        if let Some(composed) = composed {
            app.insert_resource(NativeTransportLink(composed));
        }
        if let Some(notices) = &relay_notices {
            app.insert_resource(notices.clone());
            app.add_systems(bevy::prelude::Update, report_relay_notices);
        }
    }

    // The viewscreen's join panel (issue #1329). Here rather than beside the
    // surface itself, because half of what an invitation needs — which service
    // this host registered with — is only known once the block above has run.
    if let Some(lobby) = &host_lobby {
        let mut join = native_host::host_lobby::HostLobbyJoinResource::from_lobby(
            lobby,
            sim.rendezvous.as_deref(),
        );
        if let Some(code) = &direct_code {
            join = join.with_direct_code(code.clone());
        }
        app.insert_resource(join.clone());
        if let Some(code) = &direct_code {
            // Direct accept (issue #1353): the code exists at bind, so the QR
            // goes up now rather than waiting on a service to answer. It is
            // also the ONLY code this panel will show — see
            // `HostLobbyJoinResource::direct`.
            lobby.publish_join(&join.invite(code));
        } else if sim.rendezvous.is_none() {
            // No ingress at all: `--solo`, or a host serving no bundle and given
            // no `--rendezvous`. There will never be a code, and that is said on
            // the viewscreen in words — a framed empty QR would have a crew
            // stand in front of it scanning something that cannot work, and
            // would look identical to a code that has not arrived yet.
            lobby.publish_join(&native_host::host_lobby::JoinInvite::Off);
        }
        if direct_code.is_some() || sim.rendezvous.is_some() {
            eprintln!(
                "phoenix-host: the join QR is on the viewscreen, pointing phones at {}",
                lobby.join_base
            );
            // Everything the QR can be wrong about, said here rather than
            // discovered with a phone in front of a wall. Both checks are pure
            // and classified in `host_lobby::join`; this is only the wording.
            //
            // WHERE it points: loopback is what a `--addr 127.0.0.1` bind asks
            // for and where a machine with no route falls back to, but the
            // default route can equally hand back a VPN address, a link-local
            // one, or a public one — all of which look like success and none of
            // which a phone in the room can open.
            if let Some(reason) =
                native_host::host_lobby::join_addr_reach(&lobby.join_base).unreachable_reason()
            {
                eprintln!(
                    "phoenix-host: …{reason} Bind the LAN address explicitly with \
                     --addr <ip>:<port> if this machine has one."
                );
            }
            // WHICH SERVICE it names. With direct accept live there is no
            // question left: the QR carries no service at all, and the page it
            // opens dials the origin that served it — this host. The warning
            // below is for the remaining CLOUD-ONLY case, where the client's
            // `?rendezvous=` gate reads the parameter's own host and silently
            // swaps a non-loopback override for the built-in service, so the
            // phone joins something and it is not this host.
            if direct_code.is_none()
                && sim.rendezvous.as_deref().is_some_and(|base| {
                    native_host::host_lobby::phone_rendezvous(base)
                        == native_host::host_lobby::PhoneRendezvous::SilentlyIgnored
                })
            {
                eprintln!(
                    "phoenix-host: …but a scanning phone will IGNORE the --rendezvous service in \
                     that QR and dial the client bundle's built-in one ({}) instead, so it will \
                     not find this host. Serve the bundle with --client-dir for direct LAN \
                     joining, or use the built-in service.",
                    native_host::host_lobby::CLIENT_DEFAULT_RENDEZVOUS
                );
            }
            if direct_code.is_some() && sim.rendezvous.is_some() {
                eprintln!(
                    "phoenix-host: …with the LAN code, not the cloud one: the QR carries the page \
                     as well as the code, and a phone that loads the page from this host dials \
                     this host. The cloud service's own code is printed above for anybody joining \
                     from outside the LAN."
                );
            }
        }
    }

    eprintln!(
        "phoenix-host: authoritative simulation on {} — native viewscreen",
        sim.world
            .as_deref()
            .unwrap_or("a scenario yet to be chosen from the lobby")
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
    if let Some(lobby) = &host_lobby {
        lobby.withdraw(&hosted_documents);
    }
    shutdown.stop();
    if let Ok(handle) = delivery {
        let _ = handle.join();
    }
}

/// Print what the crew transport has to say — the issued join code above all.
///
/// A Bevy system rather than a callback because the transport is a resource the
/// scheduler owns, and because the code has to reach the operator's terminal on
/// the frame the service issues it: the five letters are how anybody joins, and
/// a native host has no viewscreen panel to paint them on.
#[cfg(not(target_arch = "wasm32"))]
fn report_relay_notices(
    notices: Option<
        bevy::prelude::Res<project_phoenix::native_host::relay_transport::RelayNotices>,
    >,
    join: Option<
        bevy::prelude::Res<project_phoenix::native_host::host_lobby::HostLobbyJoinResource>,
    >,
    lobby: Option<
        bevy::prelude::Res<project_phoenix::native_host::host_lobby::HostLobbyBridgeResource>,
    >,
) {
    use project_phoenix::native_host::relay_transport::RelayNotice;
    let Some(notices) = notices else {
        return;
    };
    for notice in notices.drain() {
        match notice {
            RelayNotice::Coded(code) => {
                eprintln!(
                    "phoenix-host: crew join code {} (full: {})",
                    code.suffix, code.full
                );
                // …and onto the viewscreen, which is the point of issue #1329:
                // a code printed into a terminal window nobody at the bridge can
                // see is not a code the crew has been given.
                //
                // The SAME code every time it is re-issued, including a reclaim
                // after a dropped service (issue #1115), because the panel is
                // repainted from whatever arrives rather than from a first
                // sighting. A transient fault deliberately leaves the code up:
                // a reclaim usually returns the very same letters, and blanking
                // the wall for a reconnection that succeeds seconds later costs
                // the room more than it tells them.
                //
                // …unless the QR belongs to the other leg. A host running both
                // direct accept and `--rendezvous` is issued two codes, and
                // only the one a page served BY this host can resolve may go on
                // the wall (issue #1353).
                if let (Some(join), Some(lobby)) = (&join, &lobby) {
                    if let Some(invite) = join.viewscreen_invite(&code) {
                        lobby.0.push_join(invite.to_json());
                    }
                }
            }
            RelayNotice::Refused { peer, code } => {
                eprintln!("phoenix-host: refused {peer}: {code}")
            }
            RelayNotice::Fault { reason } => eprintln!("phoenix-host: rendezvous: {reason}"),
            RelayNotice::Shedding { total, .. } => {
                eprintln!("phoenix-host: relay is behind — {total} snapshot frames shed so far")
            }
        }
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
