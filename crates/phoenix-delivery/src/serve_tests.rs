use super::*;

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
    let fixture = Fixture::new();
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
        content: EmptyContent.prepared_catalogue().unwrap(),
        source: Arc::new(EmptyContent),
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
struct EmptyContent;
impl ContentSource for EmptyContent {
    fn prepared_catalogue(&self) -> Result<LoadedContent, String> {
        Ok(LoadedContent {
            manifest: DeliveryManifest {
                stamp: crate::stamp::DeliveryStamp {
                    protocol: phoenix_model::messages::PROTOCOL_VERSION,
                    content_id: "phoenix-base".into(),
                    content_epoch: 1,
                },
                manifest_path: "assets/scenarios.toml".into(),
                scenarios: vec![],
            },
            findings: vec![],
        })
    }
    fn overlay_text(&self, _: &str) -> Option<String> {
        None
    }
    fn overlay_asset(&self, _: &str) -> Option<Arc<[u8]>> {
        None
    }
    fn revision(&self) -> u64 {
        0
    }
}
struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "phoenix-delivery-containment-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
