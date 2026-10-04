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
use crate::delivery::http::{
    self, CachePolicy, HttpVersion, PathRefusal, Request, MANIFEST_PATH, STAMP_PATH,
};
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

/// How long one connection may take to finish sending its request head.
///
/// [`MAX_HEAD_BYTES`] bounds how MUCH a caller may send before being cut off,
/// and until issue #1353 nothing bounded how LONG it could take to send it: a
/// socket that opened, sent one byte a minute and never sent a blank line held
/// a connection thread for as long as it liked. That is the slow-loris shape,
/// and it became worth closing when this loop grew a second door
/// ([`ConnectionUpgrade`]) that a phone on an untrusted LAN can knock on.
///
/// Generous rather than tight: this is not a rate limit and a head that arrives
/// in three TCP segments over a bad Wi-Fi link is ordinary. Not a gameplay
/// value — it is one socket's own patience (AGENTS.md rule 11).
const HEAD_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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
    documents: Arc<std::sync::RwLock<std::collections::BTreeMap<String, HostedResource>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedResource {
    pub body: Arc<[u8]>,
    pub content_type: String,
    pub immutable: bool,
}

impl HostedDocuments {
    fn read(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, std::collections::BTreeMap<String, HostedResource>> {
        self.documents.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Publish `html` at `path` (an absolute request path, e.g.
    /// `/client/pane-0.html`). Replaces whatever was there.
    pub fn publish(&self, path: impl Into<String>, html: String) {
        self.publish_bytes(path, html.into_bytes(), "text/html; charset=utf-8", false);
    }

    /// Publish an in-memory binary resource. Workshop preview captures use
    /// immutable, nonce-scoped paths so large model and texture bytes never
    /// cross the embedded view's JSON bridge.
    pub fn publish_bytes(
        &self,
        path: impl Into<String>,
        body: Vec<u8>,
        content_type: impl Into<String>,
        immutable: bool,
    ) {
        self.documents
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                path.into(),
                HostedResource {
                    body: Arc::from(body),
                    content_type: content_type.into(),
                    immutable,
                },
            );
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
        let resource = self.read().get(path)?.clone();
        String::from_utf8(resource.body.to_vec()).ok()
    }

    pub fn resource(&self, path: &str) -> Option<HostedResource> {
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
/// Reads the session mod-pack overlay (see [`Self::resolve_world`]) and writes
/// nothing process-global of its own.
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
    ///
    /// **Overlay first, disk second**, which is not a nicety: it is the order
    /// [`build_merged_catalog`]'s contract requires of its caller, the order the
    /// browser twin already resolves in (`bridge`'s
    /// `config_cache::resolved_world_source`), and the order every other native
    /// content channel uses — `include_resolve::FsFragmentSource`,
    /// `config_cache::OverlayScriptResolver`, `world::load::OverlayFsReader`. A
    /// pack that ADDS a scenario carries that scenario's world only in its own
    /// [`ActivePack::files`](crate::entities::config_cache::ActivePack), never on
    /// disk, and `build_catalog` SKIPS an entry whose world will not resolve — so
    /// a disk-only resolver would accept the pack, log it installed, and then
    /// silently drop the very scenario it was installed for.
    ///
    /// The two lookups share a key: the overlay is keyed by the authored
    /// repo-relative path (`assets/worlds/x.toml`) the manifest already names,
    /// which is exactly what `self.root.join(rel)` is built from.
    pub fn resolve_world(&self, rel: &str) -> Option<String> {
        crate::entities::config_cache::mod_pack_overlay_get(rel)
            .or_else(|| std::fs::read_to_string(self.root.join(rel)).ok())
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
    /// With no pack installed this returns exactly what [`Self::catalog`] does;
    /// going through the merge anyway is what keeps the native lobby's catalogue
    /// the browser's catalogue rather than a second derivation of it. Since issue
    /// #1366 the native stack is no longer always empty — `host_lobby`'s
    /// `apply_mod_pack_choice` pushes onto it — which is why
    /// [`Self::resolve_world`] has to be overlay-aware for this method to mean
    /// anything.
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
/// Writes no process-global state, so it is safe from a unit test — unlike
/// [`preload_templates`], which is not. It does READ the session mod-pack
/// overlay through [`ManifestSource::resolve_world`], which is empty unless a
/// test installed a pack (and [`crate::entities::config_cache`]'s
/// `overlay_test_guard` is how one asks for that).
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
    /// Serve a typed in-memory resource at an immutable capture URL.
    Hosted { resource: HostedResource },
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
        http::ASSET_REVISION_PATH => Route::Json {
            status: 200,
            reason: "OK",
            body: codec::encode_native_asset_revision(
                crate::entities::config_cache::mod_pack_revision(),
            ),
            refusal: None,
        },
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
                if let Some(resource) = documents.resource(path) {
                    if resource.content_type == "text/html; charset=utf-8" && !resource.immutable {
                        return Route::Document {
                            body: String::from_utf8(resource.body.to_vec())
                                .expect("published HTML remains UTF-8"),
                        };
                    }
                    return Route::Hosted { resource };
                }
            }
            // A retired or unknown preview member is absent. Never let its
            // logical asset path fall through to the static client tree.
            if path.starts_with("/workshop-preview-capture/")
                || path.starts_with("/workshop-test-frame/")
            {
                return Route::NotFound {
                    detail: "no such member in the captured Workshop preview",
                };
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

// ── The upgrade door (issue #1353) ──────────────────────────────────────────

/// What a request asking to leave HTTP behind turned out to be.
///
/// Pure, so the rule is decided and tested here rather than inside whichever
/// handler took the socket. The three answers are the three the rendezvous
/// worker gives on the same paths (`worker-rendezvous/src/index.js`): serve the
/// request as ordinary HTTP, refuse it with a stated status, or hand the socket
/// over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpgradeVerdict {
    /// Not an upgrade at all — route it as HTTP.
    Http,
    /// It asked for a WebSocket and this is the client's `Sec-WebSocket-Key`,
    /// verbatim. Whoever takes the socket derives the accept value from it.
    WebSocket { key: String },
    /// It asked for something this host will not do, with the status and the
    /// machine reason to answer. `426` mirrors the worker's
    /// `expected-websocket-upgrade`; `400` is a malformed WebSocket request.
    Refused { status: u16, reason: &'static str },
}

/// Classify one parsed request's upgrade intent.
///
/// `claimed` says whether a handler serves this path, and it changes only the
/// NEGATIVE answers: an ordinary bundle path that carries an `Upgrade` header
/// is still just a bundle path (a proxy or a browser extension may add one),
/// while `/v1/join` without a valid upgrade is a caller that has misunderstood
/// the endpoint and is told so instead of being handed a 404 from the static
/// tree.
///
/// The positive answer requires HTTP/1.1 and RFC 6455's request conditions — GET,
/// `Upgrade: websocket`, `Connection: upgrade`, `Sec-WebSocket-Version: 13` —
/// plus a key. Missing any of them on a claimed path is a **clean 400**, never
/// a socket handed to a handshake that will then sit waiting for bytes that
/// will never come.
pub fn websocket_upgrade(req: &Request, claimed: bool) -> UpgradeVerdict {
    use base64::Engine;
    let header_has = |name: &str, token: &str| {
        req.header(name)
            .is_some_and(|v| v.to_ascii_lowercase().split(',').any(|p| p.trim() == token))
    };
    let asks_websocket = header_has("upgrade", "websocket");
    if !claimed {
        return UpgradeVerdict::Http;
    }
    if !asks_websocket {
        // The worker's own answer for a plain GET of an upgrade endpoint.
        return UpgradeVerdict::Refused {
            status: 426,
            reason: "expected-websocket-upgrade",
        };
    }
    if req.method != "GET" {
        return UpgradeVerdict::Refused {
            status: 400,
            reason: "upgrade-must-be-get",
        };
    }
    if req.version != HttpVersion::Http11 {
        return UpgradeVerdict::Refused {
            status: 400,
            reason: "unsupported-http-version",
        };
    }
    if !header_has("connection", "upgrade") {
        return UpgradeVerdict::Refused {
            status: 400,
            reason: "upgrade-without-connection-upgrade",
        };
    }
    if req.header("sec-websocket-version").map(str::trim) != Some("13") {
        return UpgradeVerdict::Refused {
            status: 400,
            reason: "unsupported-websocket-version",
        };
    }
    // RFC 6455 requires a base64-encoded 16-byte nonce, not just 24 characters.
    match req.header("sec-websocket-key").map(str::trim) {
        Some(key)
            if base64::engine::general_purpose::STANDARD
                .decode(key)
                .is_ok_and(|nonce| nonce.len() == 16) =>
        {
            UpgradeVerdict::WebSocket {
                key: key.to_string(),
            }
        }
        _ => UpgradeVerdict::Refused {
            status: 400,
            reason: "missing-websocket-key",
        },
    }
}

/// Something that takes a connection over when its request asked to upgrade.
///
/// The seam issue #1353 needs and the reason it is a TRAIT rather than a
/// function: `delivery::serve` is compiled for every native build and must not
/// name `tungstenite`, which is optional and behind the `host` feature. So this
/// module owns the door — detection, the refusal answers, the timeouts — and
/// [`crate::native_host::direct_join`] owns what is behind it.
///
/// An implementation receives the stream and any bytes read beyond the request
/// head. It writes the `101` and supplies those bytes to its protocol reader.
/// It owns the connection from that moment: the serving loop neither
/// writes to it nor closes it again.
pub trait ConnectionUpgrade: Send + Sync + 'static {
    /// Whether this handler serves `path`. Consulted before anything else, so a
    /// host with a handler installed answers exactly one more path than one
    /// without.
    fn serves(&self, path: &str) -> bool;

    /// Take the stream over. `key` is the validated `Sec-WebSocket-Key`.
    ///
    /// The connection is the handler's from this call onwards — it takes the
    /// stream by value, so a refusal is the handler's to WRITE as well as to
    /// decide. `Err` therefore reports a refusal that has already been answered
    /// (or a client that had already gone); the serving loop only logs it.
    fn accept(
        &self,
        stream: TcpStream,
        req: &Request,
        key: &str,
        prefetched: Vec<u8>,
    ) -> Result<(), UpgradeRefusal>;
}

/// A handler's refusal of a connection it was offered, for the operator log.
/// The handler has already answered the client — see [`ConnectionUpgrade::accept`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeRefusal {
    pub status: u16,
    pub reason: &'static str,
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
    /// The one handler that may take a connection off the HTTP path
    /// (issue #1353). Behind a lock rather than a constructor argument because
    /// it is installed AFTER the bind: the direct-join service needs the port
    /// the listener actually took, which a `:0` bind does not know until then —
    /// the same ordering constraint the panes and the lobby surface have.
    upgrade: std::sync::RwLock<Option<Arc<dyn ConnectionUpgrade>>>,
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
                let root = std::fs::canonicalize(dir)
                    .map_err(|e| format!("cannot resolve --client-dir {dir:?}: {e}"))?;
                if !root.is_dir() {
                    return Err(format!("--client-dir {dir:?} is not a directory"));
                }
                let bundle_manifest = root.join(BUNDLE_MANIFEST_REL);
                let bundle_toml = read_static_file(&root, BUNDLE_MANIFEST_REL)
                    .and_then(|bytes| String::from_utf8(bytes).ok());
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
                upgrade: std::sync::RwLock::new(None),
            }),
        })
    }

    /// Install the handler that takes upgraded connections (issue #1353).
    ///
    /// Idempotent in the sense that a second call replaces the first; there is
    /// one door and one handler behind it. Safe to call while the loop is
    /// running — connections already being served are unaffected.
    pub fn on_upgrade(&self, handler: Arc<dyn ConnectionUpgrade>) {
        *self
            .state
            .upgrade
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(handler);
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
    // A caller gets [`HEAD_READ_TIMEOUT`] to finish its head and no longer. The
    // timeout is CLEARED again before the stream is handed to an upgrade
    // handler, which sets its own pumping cadence (issue #1353).
    let (req, prefetched) = match read_request(&mut stream, HEAD_READ_TIMEOUT) {
        Some(request) => request,
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
    let head_only = req.method == "HEAD";

    // The upgrade door (issue #1353), BEFORE routing: a `/v1/join` upgrade is
    // not a document request, and letting `route` answer it first would hand a
    // WebSocket client a 404 out of the static tree. A host with no handler
    // installed never reaches this branch at all.
    let handler = state
        .upgrade
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let claimed = handler.as_ref().is_some_and(|h| h.serves(&req.path));
    match websocket_upgrade(&req, claimed) {
        UpgradeVerdict::Http => {}
        UpgradeVerdict::WebSocket { key } => {
            // The handler pumps on its own cadence and sets its own timeouts;
            // ours would cut its first blocking read short.
            let _ = stream.set_read_timeout(None);
            let outcome = handler
                .as_ref()
                .expect("claimed implies a handler")
                .accept(stream, &req, &key, prefetched);
            match outcome {
                Ok(()) => on_event(HostEvent::Served {
                    method: req.method.clone(),
                    path: req.path.clone(),
                    status: 101,
                }),
                // The handler owns the socket either way, so it has already
                // answered the client; this is the operator's copy.
                Err(refusal) => on_event(HostEvent::Refused {
                    path: req.path.clone(),
                    code: refusal.reason,
                }),
            }
            return;
        }
        UpgradeVerdict::Refused { status, reason } => {
            // A malformed or mis-addressed upgrade is answered as ordinary
            // HTTP and the connection closes. Never a stall: nothing here waits
            // for more bytes, and the accept loop was never blocked on it —
            // this whole function runs on its own connection thread.
            let body = reason.to_string();
            let head = http::response_head(
                status,
                if status == 426 {
                    "Upgrade Required"
                } else {
                    "Bad Request"
                },
                "text/plain; charset=utf-8",
                CachePolicy::Revalidate,
                body.len(),
                &[],
            );
            write_all(&mut stream, &head, body.as_bytes());
            on_event(HostEvent::Served {
                method: req.method.clone(),
                path: req.path.clone(),
                status,
            });
            return;
        }
    }

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
        Route::Hosted { resource } => {
            let head = http::response_head(
                200,
                "OK",
                &resource.content_type,
                if resource.immutable {
                    CachePolicy::Immutable
                } else {
                    CachePolicy::Revalidate
                },
                resource.body.len(),
                &[],
            );
            write_all(
                &mut stream,
                &head,
                if head_only { &[] } else { &resource.body },
            );
            on_event(HostEvent::Served {
                method: req.method.clone(),
                path: req.path.clone(),
                status: 200,
            });
        }
        Route::Static { rel_path } => {
            let root = state.client_root.as_ref();
            let overlay = crate::entities::config_cache::mod_pack_asset(&rel_path);
            let bytes = overlay
                .as_ref()
                .map(|bytes| bytes.to_vec())
                .or_else(|| root.and_then(|r| read_static_file(r, &rel_path)));
            match bytes {
                Some(bytes) => {
                    let head = http::response_head(
                        200,
                        "OK",
                        http::content_type_for(&rel_path),
                        if overlay.is_some() {
                            CachePolicy::Revalidate
                        } else {
                            http::cache_policy_for(&rel_path)
                        },
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

/// Read only files whose resolved location remains beneath the canonical root.
/// The operator owns the bundle; concurrent local filesystem replacement is
/// outside this request-driven containment contract.
fn read_static_file(root: &Path, relative: &str) -> Option<Vec<u8>> {
    if !Path::new(relative)
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    let full = std::fs::canonicalize(root.join(relative)).ok()?;
    if !full.starts_with(root) {
        return None;
    }
    std::fs::read(full).ok()
}

/// Parse incrementally under a total deadline and retain upgraded-protocol bytes.
fn read_request(
    stream: &mut TcpStream,
    timeout: std::time::Duration,
) -> Option<(Request, Vec<u8>)> {
    let deadline = std::time::Instant::now().checked_add(timeout)?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        if remaining.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(remaining)).ok()?;
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        match http::parse_request_bytes(&buf).ok()? {
            httparse::Status::Complete((end, request)) => {
                if end > MAX_HEAD_BYTES || std::time::Instant::now() >= deadline {
                    return None;
                }
                return Some((request, buf.split_off(end)));
            }
            httparse::Status::Partial => {}
        }
        if buf.len() >= MAX_HEAD_BYTES {
            return None;
        }
    }
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
