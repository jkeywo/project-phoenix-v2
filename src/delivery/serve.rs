//! The native host's socket loop — the only part of `delivery` that touches
//! the network or the filesystem.
//!
//! Routing itself is [`route`], a pure function from a parsed request and the
//! loaded content to a decision, so every endpoint's behaviour (including the
//! version-pin refusal) is unit-testable without binding a port. The loop below
//! is deliberately thin: read a head, route it, write a response, close.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::core::codec;
use crate::delivery::args::{ClientSource, HostArgs};
use crate::delivery::http::{self, CachePolicy, PathRefusal, Request, MANIFEST_PATH, STAMP_PATH};
use crate::delivery::stamp::{check_bundle_content, check_client_stamp, DeliveryStamp};
use crate::delivery::{client_stamp_from_request, DeliveryManifest, DeliveryRefusal};
use crate::world::manifest::{
    build_catalog, build_merged_catalog, parse_manifest, validate_manifest, Manifest,
    MergedCatalog, ScenarioCatalog,
};

/// The largest request head this host will read before giving up. A browser's
/// head is well under a kilobyte; the cap is here so a client that never sends
/// a blank line cannot make the host read forever.
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// The scenario manifest a built client bundle carries, relative to its root.
/// `scripts/build-client.mjs` and `trunk` both place the assets tree here, and
/// `deploy-demo.yml` overwrites exactly this file with the curated manifest —
/// so it is where the bundle states which content set it was built for.
pub const BUNDLE_MANIFEST_REL: &str = "assets/scenarios.toml";

/// Where a request came from, as far as the socket loop could tell.
///
/// The one thing [`route`] needs from the connection itself, and the reason it
/// needs it is [`Route::Document`]: this host binds `0.0.0.0:8080` by default,
/// with no TLS and no authentication, because its job is handing a bundle to
/// phones on a LAN. The bundle, the manifest and the stamp are *for* that
/// audience. A document this process publishes in memory is not — it is a local
/// Station pane's own page, and a pane always connects from this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerOrigin {
    /// The connection's peer address is a loopback address.
    Loopback,
    /// Anything else — **including** an address the OS would not report, which
    /// is the safe way round for a gate.
    Remote,
}

/// Classify a connection's peer address.
///
/// `None` (the OS refused to name the peer) is [`PeerOrigin::Remote`]: a gate
/// that opened on "I could not tell" would be no gate at all.
///
/// The IPv4-mapped case is not pedantry. A dual-stack listener on `[::]`
/// reports an IPv4 loopback connection as `::ffff:127.0.0.1`, and
/// `Ipv6Addr::is_loopback` answers `false` for that — so without the second arm
/// a pane on an IPv6-bound host would be refused its own document.
pub fn peer_origin(addr: Option<std::net::SocketAddr>) -> PeerOrigin {
    let loopback = match addr.map(|a| a.ip()) {
        Some(std::net::IpAddr::V4(v4)) => v4.is_loopback(),
        Some(std::net::IpAddr::V6(v6)) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        None => false,
    };
    if loopback {
        PeerOrigin::Loopback
    } else {
        PeerOrigin::Remote
    }
}

/// Documents this process publishes itself, in addition to whatever is on disk
/// under `--client-dir` (issue #1122).
///
/// One user, and a deliberately general shape rather than a pane-specific one:
/// a native Station pane loads *the shipped client page with two scripts
/// injected* (`native_host::panes::document`), and that document exists only in
/// this process's memory. It has to arrive over HTTP from this host, same
/// origin, at the client directory's own depth — that is what makes every
/// relative URL in the page, every `gui/` module and every console iframe
/// resolve exactly as they do for a phone, with nothing rewritten and no CORS
/// question to answer.
///
/// Checked **before** the static bundle, so a published document shadows a file
/// of the same name rather than racing it, and **only for a loopback peer** —
/// see [`PeerOrigin`]. Nothing here is written to disk, and the bundle is never
/// modified.
#[derive(Clone, Default)]
pub struct HostedDocuments {
    documents: Arc<std::sync::RwLock<std::collections::BTreeMap<String, String>>>,
}

impl HostedDocuments {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, std::collections::BTreeMap<String, String>> {
        self.documents.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Publish `html` at `path` (an absolute request path, e.g.
    /// `/client/pane-0.html`). Replaces whatever was there.
    pub fn publish(&self, path: impl Into<String>, html: String) {
        self.documents
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.into(), html);
    }

    /// Stop publishing `path`.
    pub fn withdraw(&self, path: &str) {
        self.documents
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(path);
    }

    /// The document published at `path`, if any.
    pub fn get(&self, path: &str) -> Option<String> {
        self.read().get(path).cloned()
    }

    /// How many documents are published. Diagnostic.
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Whether nothing is published — the state of every delivery-only host.
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }
}

impl std::fmt::Debug for HostedDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedDocuments")
            .field("paths", &self.read().keys().collect::<Vec<_>>())
            .finish()
    }
}

/// What a host has loaded off disk and is ready to publish.
#[derive(Clone, Debug)]
pub struct LoadedContent {
    pub manifest: DeliveryManifest,
    /// Non-fatal findings from `world::manifest::validate_manifest`, surfaced
    /// once at startup. A typo'd curation entry is otherwise invisible — the
    /// same reasoning as the browser host's console warnings.
    pub findings: Vec<String>,
}

/// One scenario manifest, read off disk, with the world-resolver every
/// catalogue build over it shares.
///
/// The native answer to the two things the browser gets from its JS preload:
/// the manifest TOML (`wasm_push_scenario_manifest`) and a way to resolve each
/// `[[scenario]] world` to its raw text (`wasm_push_world_toml`, read back by
/// `resolved_world_source`). Native has neither — `config_cache::
/// resolved_world_source` is a `None` stub off the browser — so the resolver is
/// a filesystem read rooted at `content_dir`, and it lives HERE rather than
/// being re-typed at each call site: [`load_content`] publishes a catalogue over
/// HTTP and [`Self::merged_catalog`] hands one to the native host's lobby
/// (issue #1326), and the two must never resolve a world differently.
///
/// Touches no process-global state.
pub struct ManifestSource {
    /// The content tree every relative path resolves against.
    root: PathBuf,
    /// The manifest's raw text — also this host's content identity, which
    /// [`DeliveryStamp::for_manifest`] is taken over.
    pub toml: String,
    /// The path the manifest was read from, as given (relative to `root`).
    pub manifest_rel: String,
    /// The parsed manifest.
    pub manifest: Manifest,
}

impl ManifestSource {
    /// Read and parse `<content_dir>/<manifest_rel>`.
    pub fn read(content_dir: &str, manifest_rel: &str) -> Result<Self, String> {
        let root = Path::new(content_dir).to_path_buf();
        let manifest_path = root.join(manifest_rel);
        let toml = std::fs::read_to_string(&manifest_path).map_err(|e| {
            format!(
                "cannot read scenario manifest {}: {e}",
                manifest_path.display()
            )
        })?;
        let manifest = parse_manifest(&toml).map_err(|e| {
            format!(
                "scenario manifest {} is malformed: {e}",
                manifest_path.display()
            )
        })?;
        Ok(Self {
            root,
            toml,
            manifest_rel: manifest_rel.to_string(),
            manifest,
        })
    }

    /// The raw text of one manifest-listed world, or `None` when it cannot be
    /// read. The whole of native's `resolve_world`.
    pub fn resolve_world(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }

    /// `validate_manifest`'s findings, flattened to one line each for the
    /// startup summary.
    pub fn findings(&self) -> Vec<String> {
        validate_manifest(&self.manifest, &self.toml, |rel| self.resolve_world(rel))
            .into_iter()
            .map(|f| format!("[{}] {}: {}", f.category, f.source.reference, f.message))
            .collect()
    }

    /// The published catalogue — base manifest only, which is what a delivery
    /// host serves.
    pub fn catalog(&self) -> ScenarioCatalog {
        build_catalog(&self.manifest, |rel| self.resolve_world(rel))
    }

    /// The catalogue a host *offers*: the base manifest merged with every
    /// active mod pack, in load order — the same
    /// [`build_merged_catalog`] call `wasm_get_scenario_catalog` makes
    /// (issue #1326).
    ///
    /// On native the overlay stack is empty today (nothing calls
    /// `config_cache::push_mod_pack` off the browser), so this returns exactly
    /// what [`Self::catalog`] does; going through the merge anyway is what keeps
    /// the native lobby's catalogue the browser's catalogue rather than a second
    /// derivation of it, and is the seam a native mod-pack path would land on.
    pub fn merged_catalog(&self) -> MergedCatalog {
        let active = crate::entities::config_cache::active_packs();
        let parsed: Vec<(String, Manifest)> = active
            .iter()
            .filter_map(|p| {
                parse_manifest(&p.manifest_toml)
                    .ok()
                    .map(|m| (p.id.clone(), m))
            })
            .collect();
        let mods: Vec<(&str, &Manifest)> = parsed.iter().map(|(id, m)| (id.as_str(), m)).collect();
        build_merged_catalog(&self.manifest, &mods, |rel| self.resolve_world(rel))
    }

    /// The published document this manifest yields.
    pub fn loaded_content(&self) -> LoadedContent {
        LoadedContent {
            manifest: DeliveryManifest {
                stamp: DeliveryStamp::for_manifest(&self.toml),
                manifest_path: self.manifest_rel.clone(),
                scenarios: crate::delivery::payload::catalog_payload(&self.catalog()),
            },
            findings: self.findings(),
        }
    }
}

/// Read the manifest and its worlds off disk and build the published document.
///
/// Touches no process-global state, so it is safe from a unit test — unlike
/// [`preload_templates`], which is not.
pub fn load_content(content_dir: &str, manifest_rel: &str) -> Result<LoadedContent, String> {
    Ok(ManifestSource::read(content_dir, manifest_rel)?.loaded_content())
}

/// Populate the process-global native entity-template cache so the published
/// catalogue carries each hull's class, hull id, power rating and name.
///
/// **Process-global.** Like everything else that calls
/// `config_cache::insert_native_config`, this belongs in a binary or an
/// *integration* test, never in an inline unit test — see that function's docs
/// and AGENTS.md's testing strategy. The catalogue is correct without it; the
/// enrichment fields are simply absent, exactly as they are in the browser
/// before the templates have been fetched.
///
/// Returns how many templates were loaded.
pub fn preload_templates(content_dir: &str) -> Result<usize, String> {
    let dir = Path::new(content_dir).join("assets/entities");
    let mut loaded = 0;
    let mut stack = vec![dir];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // Key on the repo-relative, forward-slashed path a world names the
            // template with — the same key `headless::app` uses, and the same
            // key `ship_payload` looks up.
            let Ok(rel) = path.strip_prefix(content_dir) else {
                continue;
            };
            let key = rel.to_string_lossy().replace('\\', "/");
            let Ok(resolved) = crate::entities::include_resolve::resolve_from_disk(&key) else {
                continue;
            };
            if let Ok(cfg) = resolved.parse() {
                crate::entities::config_cache::insert_native_config(key, cfg);
                loaded += 1;
            }
        }
    }
    Ok(loaded)
}

/// What [`route`] decided to do with a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// Answer with this JSON body and status. `refusal` carries the version-pin
    /// reason code when the status is a refusal, so the socket loop can echo it
    /// as a header and the operator log can name it.
    Json {
        status: u16,
        reason: &'static str,
        body: String,
        refusal: Option<&'static str>,
    },
    /// Serve this in-memory document, published by the host itself.
    Document { body: String },
    /// Serve this bundle-relative file.
    Static { rel_path: String },
    /// No bundle is being served, or the path escaped it.
    NotFound { detail: &'static str },
    /// Only GET and HEAD are served.
    MethodNotAllowed,
}

/// Decide what a request gets. Pure.
///
/// `documents` are the host's own in-memory publications (issue #1122) and are
/// checked after the two version-pin endpoints and **before** the client bundle,
/// so a published document shadows a file of the same name deterministically
/// rather than racing it. A delivery-only host passes an empty set and routes
/// exactly as it always did.
///
/// `peer` is the only thing here that comes from the connection rather than
/// from the request, and it gates exactly one decision: a hosted document is
/// served to [`PeerOrigin::Loopback`] and to nothing else. Everything the LAN is
/// meant to fetch — the bundle, the manifest, the stamp — is unaffected.
pub fn route(
    req: &Request,
    content: &LoadedContent,
    client: &ClientSource,
    documents: &HostedDocuments,
    peer: PeerOrigin,
) -> Route {
    if req.method != "GET" && req.method != "HEAD" {
        return Route::MethodNotAllowed;
    }
    match req.path.as_str() {
        STAMP_PATH => Route::Json {
            status: 200,
            reason: "OK",
            body: codec::encode_delivery_stamp(&content.manifest.stamp),
            refusal: None,
        },
        MANIFEST_PATH => {
            let client_stamp = client_stamp_from_request(req);
            match check_client_stamp(&content.manifest.stamp, client_stamp.as_ref()) {
                Ok(()) => Route::Json {
                    status: 200,
                    reason: "OK",
                    body: codec::encode_delivery_manifest(&content.manifest),
                    refusal: None,
                },
                Err(mismatch) => {
                    let code = mismatch.code();
                    Route::Json {
                        status: 409,
                        reason: "Conflict",
                        body: codec::encode_delivery_refusal(&DeliveryRefusal {
                            mismatch,
                            host: content.manifest.stamp.clone(),
                        }),
                        refusal: Some(code),
                    }
                }
            }
        }
        path => {
            // A hosted document is a local pane's own console page, carrying a
            // live participant's view of the bridge. It is looked up at all
            // only for a peer on this machine — and a remote caller then gets
            // whatever any unknown path gets, so the refusal does not even
            // confirm the path exists.
            if peer == PeerOrigin::Loopback {
                if let Some(body) = documents.get(path) {
                    return Route::Document { body };
                }
            }
            match client {
                ClientSource::Hosted => Route::NotFound {
                    detail: "this host serves no client assets (started without --client-dir)",
                },
                ClientSource::Bundled { .. } => match http::resolve_static_path(path) {
                    Ok(rel_path) => Route::Static { rel_path },
                    Err(PathRefusal::Traversal) => Route::NotFound {
                        detail: "path escapes the client directory",
                    },
                    Err(PathRefusal::NotAbsolute) => Route::NotFound {
                        detail: "path is not absolute",
                    },
                },
            }
        }
    }
}

/// Something worth telling the operator about. The module itself never prints —
/// `phoenix-host` renders these, so `serve` stays free of AGENTS.md's logging
/// question and a test can assert on events instead of scraping stdout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    Bound {
        addr: String,
    },
    Served {
        method: String,
        path: String,
        status: u16,
    },
    Refused {
        path: String,
        code: &'static str,
    },
    Failed {
        detail: String,
    },
}

/// A bound native host, ready to serve.
pub struct HostServer {
    listener: TcpListener,
    state: Arc<ServerState>,
}

struct ServerState {
    content: LoadedContent,
    client: ClientSource,
    client_root: Option<PathBuf>,
    documents: HostedDocuments,
}

impl HostServer {
    /// Load content, run the startup bundle pin, and bind the listener.
    ///
    /// The bundle pin runs BEFORE the bind on purpose: a host that will refuse
    /// every client should not first take the port.
    pub fn bind(args: &HostArgs) -> Result<Self, String> {
        let content = load_content(&args.content_dir, &args.manifest)?;

        let client_root = match &args.client {
            ClientSource::Hosted => None,
            ClientSource::Bundled { dir } => {
                let root = PathBuf::from(dir);
                if !root.is_dir() {
                    return Err(format!("--client-dir {dir:?} is not a directory"));
                }
                let bundle_manifest = root.join(BUNDLE_MANIFEST_REL);
                let bundle_toml = std::fs::read_to_string(&bundle_manifest).ok();
                let display = bundle_manifest.display().to_string();
                match check_bundle_content(
                    &content.manifest.stamp,
                    bundle_toml.as_deref(),
                    &display,
                ) {
                    Ok(()) => {}
                    Err(mismatch) => {
                        // `--skip-bundle-check` forgives only "I could not tell",
                        // never "these are different content sets". A real
                        // mismatch is the case the pin exists for.
                        let unverifiable = mismatch.code() == "bundle-content-missing";
                        if !(unverifiable && args.skip_bundle_check) {
                            return Err(format!("{}: {}", mismatch.code(), mismatch.detail()));
                        }
                    }
                }
                Some(root)
            }
        };

        let listener =
            TcpListener::bind(&args.addr).map_err(|e| format!("cannot bind {}: {e}", args.addr))?;

        Ok(Self {
            listener,
            state: Arc::new(ServerState {
                content,
                client: args.client.clone(),
                client_root,
                documents: HostedDocuments::default(),
            }),
        })
    }

    /// The host's own in-memory publications (issue #1122), for a caller that
    /// wants to publish into them.
    ///
    /// Cheap to clone and shared with the serving thread, so a caller may keep
    /// its handle and publish or withdraw while the host runs — which is what a
    /// pane opening or closing mid-mission does.
    pub fn hosted_documents(&self) -> HostedDocuments {
        self.state.documents.clone()
    }

    /// The address actually bound — the port a `:0` bind was given.
    pub fn local_addr(&self) -> String {
        self.listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_default()
    }

    /// What this host loaded, for the startup summary.
    pub fn content(&self) -> &LoadedContent {
        &self.state.content
    }

    /// Accept and serve until the listener errors. One thread per connection,
    /// each closed when its response is written: a client fetching a 38 MiB
    /// WASM must not block the phone asking for the manifest behind it.
    ///
    /// Blocks forever and cannot be stopped — which is right for the
    /// delivery-only host, whose whole job is this loop, and wrong for a host
    /// that also runs a simulation. That one uses [`serve_until`](Self::serve_until).
    pub fn serve_forever<F>(&self, on_event: F)
    where
        F: Fn(HostEvent) + Send + Sync + 'static,
    {
        let on_event = Arc::new(on_event);
        on_event(HostEvent::Bound {
            addr: self.local_addr(),
        });
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => self.spawn_connection(stream, &on_event),
                Err(e) => on_event(HostEvent::Failed {
                    detail: format!("accept failed: {e}"),
                }),
            }
        }
    }

    /// Install the shutdown poll seam [`serve_until`](Self::serve_until) needs,
    /// so a caller can find out it is unavailable **before** it commits to the
    /// arrangement that depends on it (issue #1121).
    ///
    /// `phoenix-host` calls this before it spawns the delivery thread, because
    /// the next thing it does is hand the main thread to Bevy for the rest of
    /// the process's life. Idempotent; `serve_until` calls it again itself, so a
    /// caller that does not care may simply not call it.
    pub fn enable_shutdown_polling(&self) -> std::io::Result<()> {
        self.listener.set_nonblocking(true)
    }

    /// [`serve_forever`](Self::serve_forever), with a way out (issue #1121).
    ///
    /// A native *authoritative* host runs this on a worker thread while Bevy
    /// owns the main one — winit requires the main thread on Windows — and the
    /// two have to be able to stop together. `serve_forever`'s blocking
    /// `accept()` has no such seam: nothing short of process exit unblocks it,
    /// so the delivery thread would outlive a clean `AppExit`.
    ///
    /// So this polls instead. The listener goes non-blocking and the loop
    /// checks `shutdown` between accepts, sleeping [`ACCEPT_POLL`] when there
    /// is nothing waiting. Each accepted stream is put **back** into blocking
    /// mode before it is handled: on Windows an accepted socket inherits the
    /// listener's non-blocking flag, and a non-blocking read would make
    /// `read_head` see `WouldBlock`, give up, and answer 400 to a
    /// perfectly good request.
    ///
    /// **A listener that cannot be made non-blocking is fatal, not a fallback.**
    /// This used to log and drop into `serve_forever`'s blocking loop, which
    /// reads as graceful and is not: the caller's shutdown path is
    /// `shutdown.stop()` followed by `handle.join()`, and a thread parked in
    /// `accept()` never observes the flag — so the window would close, the join
    /// would block forever, and the process would sit on port 8080 with nothing
    /// on screen and no way out but the task manager. Returning `Err` lets
    /// `enable_shutdown_polling`'s caller refuse to start at the prompt instead.
    pub fn serve_until<F>(&self, shutdown: ShutdownSignal, on_event: F) -> Result<(), String>
    where
        F: Fn(HostEvent) + Send + Sync + 'static,
    {
        let on_event = Arc::new(on_event);
        if let Err(e) = self.enable_shutdown_polling() {
            let detail =
                format!("cannot poll for shutdown ({e}); refusing to serve without a stop path");
            on_event(HostEvent::Failed {
                detail: detail.clone(),
            });
            return Err(detail);
        }
        on_event(HostEvent::Bound {
            addr: self.local_addr(),
        });
        while !shutdown.is_stopped() {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    self.spawn_connection(stream, &on_event);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL);
                }
                Err(e) => on_event(HostEvent::Failed {
                    detail: format!("accept failed: {e}"),
                }),
            }
        }
        Ok(())
    }

    /// Hand one accepted stream to its own thread. A panic in one connection
    /// must not take the host down, and a spawn failure is worth saying out
    /// loud rather than silently dropping the client.
    fn spawn_connection<F>(&self, stream: TcpStream, on_event: &Arc<F>)
    where
        F: Fn(HostEvent) + Send + Sync + 'static,
    {
        let state = Arc::clone(&self.state);
        let events = Arc::clone(on_event);
        if let Err(e) = std::thread::Builder::new()
            .name("phoenix-host-conn".to_string())
            .spawn(move || handle_connection(stream, &state, events.as_ref()))
        {
            on_event(HostEvent::Failed {
                detail: format!("cannot spawn connection thread: {e}"),
            });
        }
    }
}

/// How long [`HostServer::serve_until`] waits between accept attempts when
/// nothing is connecting. Short enough that shutdown is imperceptible, long
/// enough that an idle host is not a busy loop.
const ACCEPT_POLL: std::time::Duration = std::time::Duration::from_millis(25);

/// The stop lever for [`HostServer::serve_until`]. Cheap to clone; every clone
/// refers to the same flag, so the thread that runs the simulation can stop the
/// thread that serves the bundle.
#[derive(Clone, Default)]
pub struct ShutdownSignal(Arc<std::sync::atomic::AtomicBool>);

impl ShutdownSignal {
    /// A signal that has not been raised.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the serving loop to return after at most one [`ACCEPT_POLL`].
    pub fn stop(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether [`stop`](Self::stop) has been called.
    pub fn is_stopped(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn handle_connection<F: Fn(HostEvent)>(mut stream: TcpStream, state: &ServerState, on_event: &F) {
    // Read BEFORE anything else can fail: this is the only point at which the
    // connection's own origin is knowable, and `route` gates the host's
    // in-memory documents on it. See `PeerOrigin`.
    let peer = peer_origin(stream.peer_addr().ok());
    let head = match read_head(&mut stream) {
        Some(head) => head,
        None => {
            write_all(
                &mut stream,
                &http::response_head(
                    400,
                    "Bad Request",
                    "text/plain; charset=utf-8",
                    CachePolicy::Revalidate,
                    0,
                    &[],
                ),
                &[],
            );
            return;
        }
    };
    let Some(req) = http::parse_request(&head) else {
        write_all(
            &mut stream,
            &http::response_head(
                400,
                "Bad Request",
                "text/plain; charset=utf-8",
                CachePolicy::Revalidate,
                0,
                &[],
            ),
            &[],
        );
        return;
    };
    let head_only = req.method == "HEAD";

    match route(&req, &state.content, &state.client, &state.documents, peer) {
        Route::Json {
            status,
            reason,
            body,
            refusal,
        } => {
            let extra: Vec<(&str, String)> = refusal
                .map(|code| vec![("X-Phoenix-Refusal", code.to_string())])
                .unwrap_or_default();
            let head = http::response_head(
                status,
                reason,
                "application/json; charset=utf-8",
                CachePolicy::Revalidate,
                body.len(),
                &extra,
            );
            write_all(
                &mut stream,
                &head,
                if head_only { &[] } else { body.as_bytes() },
            );
            match refusal {
                Some(code) => on_event(HostEvent::Refused {
                    path: req.path.clone(),
                    code,
                }),
                None => on_event(HostEvent::Served {
                    method: req.method.clone(),
                    path: req.path.clone(),
                    status,
                }),
            }
        }
        Route::Document { body } => {
            // Revalidated, never cached as immutable: a pane document carries
            // that pane's own session token, and the pane it belongs to may be
            // closed and reopened within one process lifetime. The same policy
            // the client's own `index.html` gets.
            let head = http::response_head(
                200,
                "OK",
                "text/html; charset=utf-8",
                CachePolicy::Revalidate,
                body.len(),
                &[],
            );
            write_all(
                &mut stream,
                &head,
                if head_only { &[] } else { body.as_bytes() },
            );
            on_event(HostEvent::Served {
                method: req.method.clone(),
                path: req.path.clone(),
                status: 200,
            });
        }
        Route::Static { rel_path } => {
            let root = state.client_root.as_ref();
            let full = root.map(|r| r.join(&rel_path));
            let bytes = full.as_ref().and_then(|p| std::fs::read(p).ok());
            match bytes {
                Some(bytes) => {
                    let head = http::response_head(
                        200,
                        "OK",
                        http::content_type_for(&rel_path),
                        http::cache_policy_for(&rel_path),
                        bytes.len(),
                        &[],
                    );
                    write_all(&mut stream, &head, if head_only { &[] } else { &bytes });
                    on_event(HostEvent::Served {
                        method: req.method.clone(),
                        path: req.path.clone(),
                        status: 200,
                    });
                }
                None => {
                    write_not_found(&mut stream, "no such file in the client bundle", head_only);
                    on_event(HostEvent::Served {
                        method: req.method.clone(),
                        path: req.path.clone(),
                        status: 404,
                    });
                }
            }
        }
        Route::NotFound { detail } => {
            write_not_found(&mut stream, detail, head_only);
            on_event(HostEvent::Served {
                method: req.method.clone(),
                path: req.path.clone(),
                status: 404,
            });
        }
        Route::MethodNotAllowed => {
            let head = http::response_head(
                405,
                "Method Not Allowed",
                "text/plain; charset=utf-8",
                CachePolicy::Revalidate,
                0,
                &[("Allow", "GET, HEAD".to_string())],
            );
            write_all(&mut stream, &head, &[]);
            on_event(HostEvent::Served {
                method: req.method.clone(),
                path: req.path.clone(),
                status: 405,
            });
        }
    }
}

fn write_not_found(stream: &mut TcpStream, detail: &str, head_only: bool) {
    let head = http::response_head(
        404,
        "Not Found",
        "text/plain; charset=utf-8",
        CachePolicy::Revalidate,
        detail.len(),
        &[],
    );
    write_all(
        stream,
        &head,
        if head_only { &[] } else { detail.as_bytes() },
    );
}

fn write_all(stream: &mut TcpStream, head: &str, body: &[u8]) {
    // A client that hung up mid-response is ordinary, not an error worth
    // surfacing: there is nobody left to tell.
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Read up to the blank line that ends an HTTP head, or give up.
fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find_head_end(&buf) {
            return String::from_utf8(buf[..end].to_vec()).ok();
        }
        if buf.len() > MAX_HEAD_BYTES {
            return None;
        }
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::PROTOCOL_VERSION;
    use crate::delivery::http::CLIENT_STAMP_HEADER;
    use crate::delivery::payload::PayloadValue;

    const MANIFEST: &str = "\
[content]
id = \"phoenix-base\"
epoch = 1

[[scenario]]
id = \"combat_test\"
world = \"assets/worlds/combat_test.toml\"
";

    const WORLD: &str = "\
[global]
title = \"Combat Test\"
description = \"A skirmish.\"

[[available_ships]]
template_path = \"assets/entities/alliance_destroyer.toml\"

[[available_ships]]
template_path = \"assets/entities/alliance_cruiser.toml\"
";

    /// A content tree on disk, in a directory this test owns.
    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str, manifest: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("phoenix-delivery-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("assets/worlds")).unwrap();
            std::fs::write(dir.join("assets/scenarios.toml"), manifest).unwrap();
            std::fs::write(dir.join("assets/worlds/combat_test.toml"), WORLD).unwrap();
            Self { dir }
        }

        fn path(&self) -> String {
            self.dir.to_string_lossy().into_owned()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn request(head: &str) -> Request {
        http::parse_request(head).expect("well-formed head")
    }

    fn matching_stamp_query() -> String {
        format!("protocol={PROTOCOL_VERSION}&content_id=phoenix-base&content_epoch=1")
    }

    #[test]
    fn loading_content_builds_the_catalogue_from_the_manifest_and_its_worlds() {
        let fx = Fixture::new("load", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        assert_eq!(content.manifest.stamp.content_id, "phoenix-base");
        assert_eq!(content.manifest.manifest_path, "assets/scenarios.toml");
        assert_eq!(content.manifest.scenarios.len(), 1);
        let scenario = &content.manifest.scenarios[0];
        assert_eq!(
            scenario.get("label").and_then(PayloadValue::as_text),
            Some("Combat Test")
        );
        assert_eq!(scenario.ships().len(), 2);
        assert!(content.findings.is_empty());
    }

    #[test]
    fn a_curated_manifest_restricts_the_published_hulls_without_editing_the_world() {
        let curated = "\
[content]
id = \"phoenix-base\"
epoch = 1

[[scenario]]
id = \"combat_test\"
world = \"assets/worlds/combat_test.toml\"
ships = [\"assets/entities/alliance_destroyer.toml\"]
";
        let fx = Fixture::new("curated", curated);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let ships = content.manifest.scenarios[0].ships();
        assert_eq!(ships.len(), 1);
        assert_eq!(
            ships[0]
                .get("template_path")
                .and_then(PayloadValue::as_text),
            Some("assets/entities/alliance_destroyer.toml")
        );
        // The world file still authors both hulls — curation filtered the
        // catalogue, it did not rewrite the content.
        let world = std::fs::read_to_string(fx.dir.join("assets/worlds/combat_test.toml")).unwrap();
        assert!(world.contains("alliance_cruiser.toml"));
    }

    #[test]
    fn a_missing_manifest_is_reported_with_the_path_that_was_tried() {
        let err = load_content("no/such/dir", "assets/scenarios.toml").unwrap_err();
        assert!(err.contains("scenarios.toml"));
    }

    #[test]
    fn the_stamp_endpoint_publishes_the_hosts_own_stamp() {
        let fx = Fixture::new("stamp", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let r = route(
            &request("GET /host/stamp.json HTTP/1.1\r\n"),
            &content,
            &ClientSource::Hosted,
            &HostedDocuments::default(),
            PeerOrigin::Loopback,
        );
        match r {
            Route::Json { status, body, .. } => {
                assert_eq!(status, 200);
                assert!(body.contains("\"content_id\":\"phoenix-base\""));
                assert!(body.contains(&format!("\"protocol\":{PROTOCOL_VERSION}")));
            }
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    #[test]
    fn a_matching_client_gets_the_manifest() {
        let fx = Fixture::new("manifest-ok", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let head = format!(
            "GET /host/manifest.json?{} HTTP/1.1\r\n",
            matching_stamp_query()
        );
        match route(
            &request(&head),
            &content,
            &ClientSource::Hosted,
            &HostedDocuments::default(),
            PeerOrigin::Loopback,
        ) {
            Route::Json {
                status,
                body,
                refusal,
                ..
            } => {
                assert_eq!(status, 200);
                assert_eq!(refusal, None);
                assert!(body.contains("\"combat_test\""));
                assert!(body.contains("\"manifest_path\":\"assets/scenarios.toml\""));
            }
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    #[test]
    fn a_mismatched_protocol_is_refused_with_a_body_naming_both_sides() {
        let fx = Fixture::new("manifest-protocol", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let head = format!(
            "GET /host/manifest.json?protocol={}&content_id=phoenix-base&content_epoch=1 HTTP/1.1\r\n",
            PROTOCOL_VERSION + 7
        );
        match route(
            &request(&head),
            &content,
            &ClientSource::Hosted,
            &HostedDocuments::default(),
            PeerOrigin::Loopback,
        ) {
            Route::Json {
                status,
                body,
                refusal,
                ..
            } => {
                assert_eq!(status, 409);
                assert_eq!(refusal, Some("protocol-mismatch"));
                assert!(body.contains("protocol-mismatch"));
                assert!(body.contains(&(PROTOCOL_VERSION + 7).to_string()));
                // The host's own stamp rides along so the caller sees the target.
                assert!(body.contains("\"host\""));
            }
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    #[test]
    fn an_unstamped_client_is_refused_rather_than_served_the_catalogue() {
        let fx = Fixture::new("manifest-unstamped", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        match route(
            &request("GET /host/manifest.json HTTP/1.1\r\n"),
            &content,
            &ClientSource::Hosted,
            &HostedDocuments::default(),
            PeerOrigin::Loopback,
        ) {
            Route::Json {
                status, refusal, ..
            } => {
                assert_eq!(status, 409);
                assert_eq!(refusal, Some("client-stamp-missing"));
            }
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    #[test]
    fn a_host_with_no_bundle_serves_no_static_paths() {
        let fx = Fixture::new("hosted", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        assert!(matches!(
            route(
                &request("GET /index.html HTTP/1.1\r\n"),
                &content,
                &ClientSource::Hosted,
                &HostedDocuments::default(),
                PeerOrigin::Loopback,
            ),
            Route::NotFound { .. }
        ));
    }

    #[test]
    fn a_bundled_host_routes_a_directory_request_to_its_index() {
        let fx = Fixture::new("bundled", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let bundled = ClientSource::Bundled {
            dir: "dist".to_string(),
        };
        assert_eq!(
            route(
                &request("GET / HTTP/1.1\r\n"),
                &content,
                &bundled,
                &HostedDocuments::default(),
                PeerOrigin::Loopback,
            ),
            Route::Static {
                rel_path: "index.html".to_string()
            }
        );
    }

    #[test]
    fn a_document_the_host_publishes_itself_is_served_ahead_of_the_bundle() {
        // Issue #1122's pane document: the shipped client page with two scripts
        // injected, existing only in this process's memory, arriving from this
        // host at the client directory's own depth so every relative URL in it
        // resolves exactly as it does for a phone.
        let fx = Fixture::new("documents", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let bundled = ClientSource::Bundled {
            dir: "dist".to_string(),
        };
        let documents = HostedDocuments::default();
        assert!(documents.is_empty());
        documents.publish("/client/pane-0.html", "<html>pane</html>".to_string());
        assert_eq!(documents.len(), 1);

        assert_eq!(
            route(
                &request("GET /client/pane-0.html HTTP/1.1\r\n"),
                &content,
                &bundled,
                &documents,
                PeerOrigin::Loopback,
            ),
            Route::Document {
                body: "<html>pane</html>".to_string()
            }
        );
        // Everything else still routes to the bundle, unchanged.
        assert_eq!(
            route(
                &request("GET /client/index.html HTTP/1.1\r\n"),
                &content,
                &bundled,
                &documents,
                PeerOrigin::Loopback,
            ),
            Route::Static {
                rel_path: "client/index.html".to_string()
            }
        );

        documents.withdraw("/client/pane-0.html");
        assert!(matches!(
            route(
                &request("GET /client/pane-0.html HTTP/1.1\r\n"),
                &content,
                &bundled,
                &documents,
                PeerOrigin::Loopback,
            ),
            Route::Static { .. }
        ));
    }

    #[test]
    fn a_hosted_document_is_never_served_to_a_peer_that_is_not_this_machine() {
        // The finding this gate answers. `phoenix-host` binds 0.0.0.0:8080 by
        // default, with no TLS and no authentication — that is the shape PRD
        // #855 wanted, because the audience is phones on a LAN. A pane's own
        // console page is not for that audience: it belongs to a live
        // participant on this machine, and a pane always connects from here.
        //
        // The bundle and the two version-pin endpoints stay LAN-open, which is
        // their job, and this test says so rather than leaving it implied.
        let fx = Fixture::new("documents-remote", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let bundled = ClientSource::Bundled {
            dir: "dist".to_string(),
        };
        let documents = HostedDocuments::default();
        documents.publish("/client/pane-0-abcd.html", "<html>pane</html>".to_string());

        let pane_request = request("GET /client/pane-0-abcd.html HTTP/1.1\r\n");
        let remote = route(
            &pane_request,
            &content,
            &bundled,
            &documents,
            PeerOrigin::Remote,
        );
        assert!(
            !matches!(remote, Route::Document { .. }),
            "a LAN caller must not be handed a pane's document: {remote:?}"
        );
        // And it is refused the way any unknown path is, so the refusal does
        // not even confirm the document exists. (`dist/` holds no such file, so
        // the socket loop answers this Static route 404.)
        assert_eq!(
            remote,
            Route::Static {
                rel_path: "client/pane-0-abcd.html".to_string()
            }
        );

        // The same request from this machine is served, so the gate is about
        // the peer and nothing else.
        assert!(matches!(
            route(
                &pane_request,
                &content,
                &bundled,
                &documents,
                PeerOrigin::Loopback
            ),
            Route::Document { .. }
        ));

        // The LAN keeps everything it is meant to have.
        for path in [STAMP_PATH, "/client/index.html"] {
            let r = route(
                &request(&format!("GET {path} HTTP/1.1\r\n")),
                &content,
                &bundled,
                &documents,
                PeerOrigin::Remote,
            );
            assert!(
                !matches!(r, Route::NotFound { .. }),
                "{path} is what this host exists to serve to a phone: {r:?}"
            );
        }
    }

    #[test]
    fn a_peer_address_is_loopback_only_when_it_really_is() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
        let at = |ip: IpAddr| SocketAddr::new(ip, 51234);
        assert_eq!(
            peer_origin(Some(at(IpAddr::V4(Ipv4Addr::LOCALHOST)))),
            PeerOrigin::Loopback
        );
        // The whole 127/8 block, not just .0.0.1.
        assert_eq!(
            peer_origin(Some(at(IpAddr::V4(Ipv4Addr::new(127, 3, 2, 1))))),
            PeerOrigin::Loopback
        );
        assert_eq!(
            peer_origin(Some(at(IpAddr::V6(Ipv6Addr::LOCALHOST)))),
            PeerOrigin::Loopback
        );
        // A dual-stack listener reports an IPv4 loopback connection like this,
        // and `Ipv6Addr::is_loopback` says false for it — so a pane on an
        // IPv6-bound host would be refused its own document without this arm.
        assert_eq!(
            peer_origin(Some(at(IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped())))),
            PeerOrigin::Loopback
        );

        assert_eq!(
            peer_origin(Some(at(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))))),
            PeerOrigin::Remote
        );
        assert_eq!(
            peer_origin(Some(at(IpAddr::V6(Ipv6Addr::new(
                0x2001, 0xdb8, 0, 0, 0, 0, 0, 1
            ))))),
            PeerOrigin::Remote
        );
        // "I could not tell" is Remote: a gate that opened on an unknown peer
        // would be no gate.
        assert_eq!(peer_origin(None), PeerOrigin::Remote);
    }

    #[test]
    fn the_version_pin_endpoints_cannot_be_shadowed_by_a_published_document() {
        // A host publishes its own documents; it does not get to replace the
        // compatibility handshake with one. The stamp and the manifest are
        // matched before anything else in `route` for exactly this reason.
        let fx = Fixture::new("shadow", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let documents = HostedDocuments::default();
        documents.publish(STAMP_PATH, "<html>not the stamp</html>".to_string());
        documents.publish(MANIFEST_PATH, "<html>not the manifest</html>".to_string());
        for path in [STAMP_PATH, MANIFEST_PATH] {
            assert!(
                matches!(
                    route(
                        &request(&format!("GET {path} HTTP/1.1\r\n")),
                        &content,
                        &ClientSource::Hosted,
                        &documents,
                        PeerOrigin::Loopback,
                    ),
                    Route::Json { .. }
                ),
                "{path} must stay the version pin's"
            );
        }
    }

    #[test]
    fn a_traversal_attempt_never_becomes_a_static_route() {
        let fx = Fixture::new("traversal", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let bundled = ClientSource::Bundled {
            dir: "dist".to_string(),
        };
        assert!(matches!(
            route(
                &request("GET /../../etc/passwd HTTP/1.1\r\n"),
                &content,
                &bundled,
                &HostedDocuments::default(),
                PeerOrigin::Loopback,
            ),
            Route::NotFound { .. }
        ));
    }

    #[test]
    fn a_write_method_is_refused_before_anything_else_is_considered() {
        let fx = Fixture::new("method", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        assert_eq!(
            route(
                &request("POST /host/manifest.json HTTP/1.1\r\n"),
                &content,
                &ClientSource::Hosted,
                &HostedDocuments::default(),
                PeerOrigin::Loopback,
            ),
            Route::MethodNotAllowed
        );
    }

    #[test]
    fn the_stamp_header_is_accepted_where_the_query_string_would_be() {
        let fx = Fixture::new("header-stamp", MANIFEST);
        let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
        let head = format!(
            "GET /host/manifest.json HTTP/1.1\r\n{CLIENT_STAMP_HEADER}: {PROTOCOL_VERSION}/phoenix-base/1\r\n"
        );
        match route(
            &request(&head),
            &content,
            &ClientSource::Hosted,
            &HostedDocuments::default(),
            PeerOrigin::Loopback,
        ) {
            Route::Json { status, .. } => assert_eq!(status, 200),
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    #[test]
    fn a_head_ending_is_found_at_the_blank_line() {
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n\r\n"), Some(16));
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n"), None);
    }
}
