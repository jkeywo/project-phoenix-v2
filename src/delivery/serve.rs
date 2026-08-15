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
use crate::world::manifest::{build_catalog, parse_manifest, validate_manifest};

/// The largest request head this host will read before giving up. A browser's
/// head is well under a kilobyte; the cap is here so a client that never sends
/// a blank line cannot make the host read forever.
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// The scenario manifest a built client bundle carries, relative to its root.
/// `scripts/build-client.mjs` and `trunk` both place the assets tree here, and
/// `deploy-demo.yml` overwrites exactly this file with the curated manifest —
/// so it is where the bundle states which content set it was built for.
pub const BUNDLE_MANIFEST_REL: &str = "assets/scenarios.toml";

/// What a host has loaded off disk and is ready to publish.
#[derive(Clone, Debug)]
pub struct LoadedContent {
    pub manifest: DeliveryManifest,
    /// Non-fatal findings from `world::manifest::validate_manifest`, surfaced
    /// once at startup. A typo'd curation entry is otherwise invisible — the
    /// same reasoning as the browser host's console warnings.
    pub findings: Vec<String>,
}

/// Read the manifest and its worlds off disk and build the published document.
///
/// Touches no process-global state, so it is safe from a unit test — unlike
/// [`preload_templates`], which is not.
pub fn load_content(content_dir: &str, manifest_rel: &str) -> Result<LoadedContent, String> {
    let root = Path::new(content_dir);
    let manifest_path = root.join(manifest_rel);
    let manifest_toml = std::fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "cannot read scenario manifest {}: {e}",
            manifest_path.display()
        )
    })?;
    let manifest = parse_manifest(&manifest_toml).map_err(|e| {
        format!(
            "scenario manifest {} is malformed: {e}",
            manifest_path.display()
        )
    })?;

    let resolve_world = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
    let findings = validate_manifest(&manifest, &manifest_toml, &resolve_world)
        .into_iter()
        .map(|f| format!("[{}] {}: {}", f.category, f.source.reference, f.message))
        .collect();

    let catalog = build_catalog(&manifest, &resolve_world);
    Ok(LoadedContent {
        manifest: DeliveryManifest {
            stamp: DeliveryStamp::for_manifest(&manifest_toml),
            manifest_path: manifest_rel.to_string(),
            scenarios: crate::delivery::payload::catalog_payload(&catalog),
        },
        findings,
    })
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
            let Ok(resolved) = crate::entity_includes::resolve_from_disk(&key) else {
                continue;
            };
            if let Ok(cfg) = resolved.parse() {
                crate::config_cache::insert_native_config(key, cfg);
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
    /// Serve this bundle-relative file.
    Static { rel_path: String },
    /// No bundle is being served, or the path escaped it.
    NotFound { detail: &'static str },
    /// Only GET and HEAD are served.
    MethodNotAllowed,
}

/// Decide what a request gets. Pure.
pub fn route(req: &Request, content: &LoadedContent, client: &ClientSource) -> Route {
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
        path => match client {
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
        },
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
            }),
        })
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
                Ok(stream) => {
                    let state = Arc::clone(&self.state);
                    let events = Arc::clone(&on_event);
                    // A panic in one connection must not take the host down, and
                    // a spawn failure is worth saying out loud rather than
                    // silently dropping the client.
                    if let Err(e) = std::thread::Builder::new()
                        .name("phoenix-host-conn".to_string())
                        .spawn(move || handle_connection(stream, &state, events.as_ref()))
                    {
                        on_event(HostEvent::Failed {
                            detail: format!("cannot spawn connection thread: {e}"),
                        });
                    }
                }
                Err(e) => on_event(HostEvent::Failed {
                    detail: format!("accept failed: {e}"),
                }),
            }
        }
    }
}

fn handle_connection<F: Fn(HostEvent)>(mut stream: TcpStream, state: &ServerState, on_event: &F) {
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

    match route(&req, &state.content, &state.client) {
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
    use crate::delivery::http::CLIENT_STAMP_HEADER;
    use crate::delivery::payload::PayloadValue;
    use crate::messages::PROTOCOL_VERSION;

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
        match route(&request(&head), &content, &ClientSource::Hosted) {
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
        match route(&request(&head), &content, &ClientSource::Hosted) {
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
                &ClientSource::Hosted
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
            route(&request("GET / HTTP/1.1\r\n"), &content, &bundled),
            Route::Static {
                rel_path: "index.html".to_string()
            }
        );
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
                &bundled
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
                &ClientSource::Hosted
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
        match route(&request(&head), &content, &ClientSource::Hosted) {
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
