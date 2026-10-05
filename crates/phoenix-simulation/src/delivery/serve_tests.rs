use super::*;
use crate::core::messages::PROTOCOL_VERSION;
use crate::delivery::http::CLIENT_STAMP_HEADER;

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

/// A world a MOD PACK carries. Never written to the fixture's content tree,
/// so anything that finds it found it in the overlay.
const PACK_WORLD: &str = "\
[global]
title = \"Pack Only\"
description = \"A scenario only the pack knows about.\"

[[available_ships]]
template_path = \"assets/entities/alliance_destroyer.toml\"
";

/// That pack's own scenario manifest, naming the world above.
const PACK_MANIFEST: &str = "\
[[scenario]]
id = \"pack_only\"
world = \"assets/worlds/pack_only.toml\"
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

/// A well-formed WebSocket upgrade head, with `extra` lines spliced in.
fn upgrade_head(path: &str, lines: &[&str]) -> String {
    let mut head = format!("GET {path} HTTP/1.1\r\nHost: 192.168.1.5:8080\r\n");
    for line in lines {
        head.push_str(line);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    head
}

const WS_LINES: &[&str] = &[
    "Upgrade: websocket",
    "Connection: Upgrade",
    "Sec-WebSocket-Version: 13",
    "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
];

/// The acceptance criterion of issue #1366's install path, and the case a
/// disk-only `resolve_world` silently dropped: an accepted pack that ADDS a
/// scenario has to appear in the catalogue the lobby publishes.
///
/// A pack's own world exists ONLY in its `files` map — nothing writes it to
/// the content tree — so this passes exactly when the resolver consults the
/// overlay before the disk.
#[test]
fn an_accepted_pack_widens_the_merged_catalogue_with_its_own_world() {
    let _overlay = crate::entities::config_cache::overlay_test_guard();
    let fixture = Fixture::new("merged-catalog-pack", MANIFEST);
    let source = ManifestSource::read(&fixture.path(), "assets/scenarios.toml")
        .expect("the fixture manifest reads");
    assert_eq!(
        source.merged_catalog().catalog.scenarios.len(),
        1,
        "the base catalogue is the one scenario on disk"
    );

    crate::entities::config_cache::push_mod_pack(crate::entities::config_cache::ActivePack {
        id: "widening-pack".to_string(),
        name: "Widening Pack".to_string(),
        version: "1".to_string(),
        files: [(
            "assets/worlds/pack_only.toml".to_string(),
            PACK_WORLD.to_string(),
        )]
        .into_iter()
        .collect(),
        manifest_toml: PACK_MANIFEST.to_string(),
        ..Default::default()
    });

    let merged = source.merged_catalog();
    let added = merged
        .catalog
        .scenarios
        .iter()
        .find(|s| s.id == "pack_only")
        .expect("a pack's own world lives only in the overlay — disk cannot resolve it");
    assert_eq!(added.origin.as_deref(), Some("widening-pack"));
    assert_eq!(added.label.as_deref(), Some("Pack Only"));
    assert_eq!(added.ships.len(), 1);
    assert!(
        merged
            .catalog
            .scenarios
            .iter()
            .any(|s| s.id == "combat_test"),
        "a pack WIDENS the catalogue; it does not replace what was there"
    );
}

#[test]
fn a_well_formed_upgrade_on_a_claimed_path_hands_the_key_over() {
    let req = request(&upgrade_head("/v1/join", WS_LINES));
    assert_eq!(
        websocket_upgrade(&req, true),
        UpgradeVerdict::WebSocket {
            key: "dGhlIHNhbXBsZSBub25jZQ==".to_string()
        }
    );
}

#[test]
fn an_upgrade_header_on_an_unclaimed_path_is_still_just_a_file_request() {
    // A proxy or a browser extension may add an `Upgrade` header to an
    // ordinary GET. Nothing about the bundle changes because it did.
    let req = request(&upgrade_head("/index.html", WS_LINES));
    assert_eq!(websocket_upgrade(&req, false), UpgradeVerdict::Http);
    // …and a host with no handler installed claims nothing at all, so even
    // the join path routes as HTTP and 404s out of the static tree.
    let req = request(&upgrade_head("/v1/join", WS_LINES));
    assert_eq!(websocket_upgrade(&req, false), UpgradeVerdict::Http);
}

#[test]
fn a_plain_get_of_the_join_endpoint_is_told_what_it_is_for() {
    // The worker's own 426 rather than a 404 out of the bundle: a caller
    // that has misunderstood the endpoint learns which mistake it made.
    let req = request("GET /v1/join HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(
        websocket_upgrade(&req, true),
        UpgradeVerdict::Refused {
            status: 426,
            reason: "expected-websocket-upgrade"
        }
    );
}

#[test]
fn a_malformed_upgrade_is_a_clean_400_and_never_a_handshake_that_stalls() {
    // Each of these reached `tungstenite` would be a socket sitting waiting
    // for bytes that are not coming, on a thread nothing reclaims. They are
    // refused before the stream is handed anywhere.
    let cases: [(&[&str], &str); 4] = [
        (
            &[
                "Upgrade: websocket",
                "Sec-WebSocket-Version: 13",
                "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
            ],
            "upgrade-without-connection-upgrade",
        ),
        (
            &[
                "Upgrade: websocket",
                "Connection: Upgrade",
                "Sec-WebSocket-Version: 8",
                "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
            ],
            "unsupported-websocket-version",
        ),
        (
            &[
                "Upgrade: websocket",
                "Connection: Upgrade",
                "Sec-WebSocket-Version: 13",
            ],
            "missing-websocket-key",
        ),
        (
            &[
                "Upgrade: websocket",
                "Connection: Upgrade",
                "Sec-WebSocket-Version: 13",
                "Sec-WebSocket-Key: short",
            ],
            "missing-websocket-key",
        ),
    ];
    for (lines, reason) in cases {
        let req = request(&upgrade_head("/v1/join", lines));
        assert_eq!(
            websocket_upgrade(&req, true),
            UpgradeVerdict::Refused {
                status: 400,
                reason
            },
            "{lines:?}"
        );
    }
}

#[test]
fn the_header_tokens_are_read_the_way_browsers_write_them() {
    // `Connection: keep-alive, Upgrade` is what a real browser sends, and a
    // whole-value comparison would refuse every genuine client. Case is
    // likewise not a browser's promise.
    let req = request(&upgrade_head(
        "/v1/join",
        &[
            "Upgrade: WebSocket",
            "Connection: keep-alive, Upgrade",
            "Sec-WebSocket-Version: 13",
            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
        ],
    ));
    assert!(matches!(
        websocket_upgrade(&req, true),
        UpgradeVerdict::WebSocket { .. }
    ));
}

#[test]
fn an_upgrade_that_is_not_a_get_is_refused_rather_than_upgraded() {
    let mut head = upgrade_head("/v1/join", WS_LINES);
    head = head.replacen("GET", "POST", 1);
    let req = request(&head);
    assert_eq!(
        websocket_upgrade(&req, true),
        UpgradeVerdict::Refused {
            status: 400,
            reason: "upgrade-must-be-get"
        }
    );
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
    assert_eq!(scenario.label.as_deref(), Some("Combat Test"));
    assert_eq!(scenario.ships.len(), 2);
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
    let ships = &content.manifest.scenarios[0].ships;
    assert_eq!(ships.len(), 1);
    assert_eq!(
        ships[0].template_path,
        "assets/entities/alliance_destroyer.toml"
    );
    // The world file still authors both hulls — curation filtered the
    // catalogue, it did not rewrite the content.
    let world =
        crate::repo_fixtures::fs::read_to_string(fx.dir.join("assets/worlds/combat_test.toml"))
            .unwrap();
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
        &request("GET /host/stamp.json HTTP/1.1\r\n\r\n"),
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
fn native_asset_revision_capability_tracks_the_current_accepted_stack_without_mutating_it() {
    use crate::entities::config_cache::{
        mod_pack_revision, push_mod_pack, remove_mod_pack, ActivePack,
    };
    let _guard = crate::entities::config_cache::overlay_test_guard();
    let fx = Fixture::new("audio-revision", MANIFEST);
    let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
    let get = |method: &str| {
        route(
            &request(&format!(
                "{method} /host/asset-revision.json HTTP/1.1\r\n\r\n"
            )),
            &content,
            &ClientSource::Hosted,
            &HostedDocuments::default(),
            PeerOrigin::Remote,
        )
    };
    let before = mod_pack_revision();
    for method in ["GET", "HEAD"] {
        let Route::Json {
            status,
            body,
            refusal,
            ..
        } = get(method)
        else {
            panic!("revision capability");
        };
        assert_eq!(status, 200);
        assert_eq!(refusal, None);
        assert_eq!(body, codec::encode_native_asset_revision(before));
        assert!(body.contains("phoenix-native-asset-revision"));
    }
    assert_eq!(mod_pack_revision(), before);
    assert!(matches!(get("POST"), Route::MethodNotAllowed));
    push_mod_pack(ActivePack {
        id: "audio-revision-capability".into(),
        ..Default::default()
    });
    let after = mod_pack_revision();
    let response = get("GET");
    remove_mod_pack("audio-revision-capability");
    assert_ne!(before, after);
    let Route::Json { body, .. } = response else {
        panic!("revision capability");
    };
    assert_eq!(body, codec::encode_native_asset_revision(after));
}

#[test]
fn a_matching_client_gets_the_manifest() {
    let fx = Fixture::new("manifest-ok", MANIFEST);
    let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
    let head = format!(
        "GET /host/manifest.json?{} HTTP/1.1\r\n\r\n",
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
        "GET /host/manifest.json?protocol={}&content_id=phoenix-base&content_epoch=1 HTTP/1.1\r\n\r\n",
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
        &request("GET /host/manifest.json HTTP/1.1\r\n\r\n"),
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
            &request("GET /index.html HTTP/1.1\r\n\r\n"),
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
            &request("GET / HTTP/1.1\r\n\r\n"),
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
            &request("GET /client/pane-0.html HTTP/1.1\r\n\r\n"),
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
            &request("GET /client/index.html HTTP/1.1\r\n\r\n"),
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
            &request("GET /client/pane-0.html HTTP/1.1\r\n\r\n"),
            &content,
            &bundled,
            &documents,
            PeerOrigin::Loopback,
        ),
        Route::Static { .. }
    ));
}

#[test]
fn retired_workshop_preview_members_never_fall_through_to_bundle_files() {
    let fx = Fixture::new("preview-capture", MANIFEST);
    let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
    let bundled = ClientSource::Bundled { dir: "dist".into() };
    let documents = HostedDocuments::default();
    let path = "/workshop-preview-capture/nonce/0";
    documents.publish_bytes(path, vec![1, 2, 3], "model/gltf-binary", true);
    assert!(matches!(
        route(
            &request(&format!("GET {path} HTTP/1.1\r\n\r\n")),
            &content,
            &bundled,
            &documents,
            PeerOrigin::Loopback,
        ),
        Route::Hosted {
            resource: HostedResource {
                immutable: true,
                ..
            }
        }
    ));
    documents.withdraw(path);
    assert!(matches!(
        route(
            &request(&format!("GET {path} HTTP/1.1\r\n\r\n")),
            &content,
            &bundled,
            &documents,
            PeerOrigin::Loopback,
        ),
        Route::NotFound { .. }
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

    let pane_request = request("GET /client/pane-0-abcd.html HTTP/1.1\r\n\r\n");
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
            &request(&format!("GET {path} HTTP/1.1\r\n\r\n")),
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
fn test_frames_are_private_mutable_and_retired_routes_never_fall_through() {
    let fx = Fixture::new("test-frames", MANIFEST);
    let content = load_content(&fx.path(), "assets/scenarios.toml").unwrap();
    let bundled = ClientSource::Bundled { dir: "dist".into() };
    let documents = HostedDocuments::default();
    for path in [
        "/workshop-test-frame/nonce/view.png",
        "/workshop-test-frame/nonce/presentation.json",
    ] {
        documents.publish_bytes(path, vec![1, 2], "application/octet-stream", false);
        let req = request(&format!("GET {path} HTTP/1.1\r\n\r\n"));
        assert!(matches!(
            route(&req, &content, &bundled, &documents, PeerOrigin::Loopback),
            Route::Hosted { .. }
        ));
        assert!(matches!(
            route(&req, &content, &bundled, &documents, PeerOrigin::Remote),
            Route::NotFound { .. }
        ));
        documents.withdraw(path);
        assert!(matches!(
            route(&req, &content, &bundled, &documents, PeerOrigin::Loopback),
            Route::NotFound { .. }
        ));
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
                    &request(&format!("GET {path} HTTP/1.1\r\n\r\n")),
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
            &request("GET /../../etc/passwd HTTP/1.1\r\n\r\n"),
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
            &request("POST /host/manifest.json HTTP/1.1\r\n\r\n"),
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
            "GET /host/manifest.json HTTP/1.1\r\n{CLIENT_STAMP_HEADER}: {PROTOCOL_VERSION}/phoenix-base/1\r\n\r\n"
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

fn socket_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    (client, listener.accept().unwrap().0)
}

fn read_bytes(bytes: &[u8]) -> Option<(Request, Vec<u8>)> {
    let (mut client, mut server) = socket_pair();
    client.write_all(bytes).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    read_request(&mut server, std::time::Duration::from_secs(1))
}

#[test]
fn incremental_heads_are_bounded_and_preserve_protocol_bytes() {
    let (mut client, mut server) = socket_pair();
    let reader =
        std::thread::spawn(move || read_request(&mut server, std::time::Duration::from_secs(1)));
    client.write_all(b"GET / HTTP/1.1\r\nHo").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    client.write_all(b"st: localhost\r\n\r\nFRAME").unwrap();
    let (request, prefetched) = reader.join().unwrap().unwrap();
    assert_eq!(request.path, "/");
    assert_eq!(prefetched, b"FRAME");
    let prefix = "GET / HTTP/1.1\r\nX-Padding: ";
    for length in [MAX_HEAD_BYTES, MAX_HEAD_BYTES + 1] {
        let head = format!("{prefix}{}\r\n\r\n", "x".repeat(length - prefix.len() - 4));
        assert_eq!(head.len(), length);
        assert_eq!(
            read_bytes(head.as_bytes()).is_some(),
            length == MAX_HEAD_BYTES
        );
    }
    for count in [128, 129] {
        let head = format!("GET / HTTP/1.1\r\n{}\r\n", "X-Test: x\r\n".repeat(count));
        assert_eq!(read_bytes(head.as_bytes()).is_some(), count == 128);
    }
    for head in [
        b"GET / HTTP/1.1\r\n".as_slice(),
        b"GET / HTTP/1.9\r\n\r\n",
        b"GET / HTTP/1.1\r\nBad Header: x\r\n\r\n",
    ] {
        assert!(read_bytes(head).is_none());
    }
}

#[test]
fn fragmented_reads_do_not_renew_the_total_deadline() {
    let (mut client, mut server) = socket_pair();
    let started = std::time::Instant::now();
    let reader = std::thread::spawn(move || {
        read_request(&mut server, std::time::Duration::from_millis(150))
    });
    for _ in 0..8 {
        let _ = client.write_all(b"G");
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    assert!(reader.join().unwrap().is_none());
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
}

#[cfg(windows)]
fn directory_link(target: &Path, link: &Path) {
    assert!(std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap()
        .status
        .success());
}

#[cfg(unix)]
fn directory_link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn filesystem_reads_contain_links_component_wise_and_return_404() {
    let fixture = Fixture::new("containment", MANIFEST);
    let bundle = fixture.dir.join("bundle");
    let sibling = fixture.dir.join("bundle-secret");
    std::fs::create_dir_all(bundle.join("inside")).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(bundle.join("index.html"), b"host").unwrap();
    std::fs::write(bundle.join("inside/index.html"), b"internal").unwrap();
    std::fs::write(sibling.join("index.html"), b"SECRET OUTSIDE BUNDLE").unwrap();
    directory_link(&bundle.join("inside"), &bundle.join("internal"));
    directory_link(&sibling, &bundle.join("external"));
    let root = std::fs::canonicalize(&bundle).unwrap();
    assert_eq!(read_static_file(&root, "index.html").unwrap(), b"host");
    assert_eq!(
        read_static_file(&root, "internal/index.html").unwrap(),
        b"internal"
    );
    assert!(read_static_file(&root, "external/index.html").is_none());
    assert!(read_static_file(&root, "../bundle-secret/index.html").is_none());
    let state = ServerState {
        content: load_content(&fixture.path(), "assets/scenarios.toml").unwrap(),
        client: ClientSource::Bundled {
            dir: bundle.to_string_lossy().into_owned(),
        },
        client_root: Some(root),
        documents: HostedDocuments::default(),
        upgrade: std::sync::RwLock::new(None),
    };
    for (target, status, expected) in [
        ("/ HTTP/1.1", 200, "host"),
        ("/ HTTP/1.0", 200, "host"),
        ("/internal/ HTTP/1.1", 200, "internal"),
        ("/external/ HTTP/1.1", 404, ""),
        ("/%2e%2e/bundle-secret/ HTTP/1.1", 404, ""),
        ("/C%3A/secret HTTP/1.1", 404, ""),
        ("/ HTTP/2.0", 400, ""),
        ("/ HTTP/1.1\r\nBroken", 400, ""),
        ("/ HTTP/1.1\r\nHost: x\r\nHost: x", 400, ""),
    ] {
        let (mut client, server) = socket_pair();
        let worker = std::thread::scope(|scope| {
            scope.spawn(|| handle_connection(server, &state, &|_| {}));
            client
                .write_all(format!("GET {target}\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            response
        });
        assert!(
            worker.starts_with(&format!("HTTP/1.1 {status}")),
            "{worker}"
        );
        assert!(worker.ends_with(expected));
        assert!(!worker.contains("SECRET OUTSIDE BUNDLE"));
    }
}

#[test]
fn upgrade_requires_http11_and_a_valid_16_byte_nonce() {
    let head = upgrade_head("/v1/join", WS_LINES);
    let req = request(&head.replace("HTTP/1.1", "HTTP/1.0"));
    assert_eq!(
        websocket_upgrade(&req, true),
        UpgradeVerdict::Refused {
            status: 400,
            reason: "unsupported-http-version"
        }
    );
    for key in [
        "!!!!!!!!!!!!!!!!!!!!!!!!",
        "YWJj",
        "AAAAAAAAAAAAAAAAAAAAAA=A",
        "AAAAAAAAAAAAAAAAAAAAAAA=",
    ] {
        let req = request(&head.replace("dGhlIHNhbXBsZSBub25jZQ==", key));
        assert_eq!(
            websocket_upgrade(&req, true),
            UpgradeVerdict::Refused {
                status: 400,
                reason: "missing-websocket-key"
            }
        );
    }
    let req = request(
        &head
            .replace("Upgrade: websocket", "Upgrade: other\r\nUpgrade: WebSocket")
            .replace(
                "Connection: Upgrade",
                "Connection: keep-alive\r\nConnection: uPgRaDe",
            ),
    );
    assert!(matches!(
        websocket_upgrade(&req, true),
        UpgradeVerdict::WebSocket { .. }
    ));
}
