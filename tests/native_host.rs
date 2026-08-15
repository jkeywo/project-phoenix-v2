//! Native delivery host, end to end (PRD #855).
//!
//! An *integration* test, not an inline `mod tests`, for the reason
//! `tests/headless_runner.rs` is one: `delivery::serve::preload_templates`
//! populates the process-global native template cache, and inside the lib test
//! binary that would leak into thousands of unrelated unit tests. Anything
//! calling `config_cache::insert_native_config` belongs here (AGENTS.md).
//!
//! It runs against the REPOSITORY'S OWN content — `assets/scenarios.toml` and
//! `assets/scenarios.demo.toml` — because the claims worth pinning are about
//! the shipped catalogue: that the curated public manifest really does restrict
//! what a host publishes, and that the version pin refuses a client stamped for
//! anything else.

use std::io::{Read, Write};
use std::net::TcpStream;

use project_phoenix::core::codec;
use project_phoenix::delivery::args::{ClientSource, HostArgs};
use project_phoenix::delivery::payload::{catalog_payload, PayloadValue};
use project_phoenix::delivery::serve::{load_content, preload_templates, HostServer};
use project_phoenix::delivery::stamp::DeliveryStamp;
use project_phoenix::delivery::DeliveryManifest;
use project_phoenix::messages::PROTOCOL_VERSION;
use project_phoenix::world::manifest::{build_catalog, build_merged_catalog, parse_manifest};

const BASE_MANIFEST: &str = "assets/scenarios.toml";
const DEMO_MANIFEST: &str = "assets/scenarios.demo.toml";

fn args(manifest: &str) -> HostArgs {
    HostArgs {
        // Port 0: the OS picks a free one and `local_addr()` reports it, so
        // parallel test binaries never collide on a fixed port.
        addr: "127.0.0.1:0".to_string(),
        client: ClientSource::Hosted,
        manifest: manifest.to_string(),
        content_dir: ".".to_string(),
        skip_bundle_check: false,
    }
}

/// Start a host, run one request against it, and return the raw response.
///
/// The listener is dropped with the returned string, which is what stops the
/// serving thread: `serve_forever` exits when `incoming()` errors, and the
/// process ends with the test binary either way.
fn request(manifest: &str, target: &str) -> String {
    let server = HostServer::bind(&args(manifest)).expect("host binds");
    let addr = server.local_addr();
    let handle = std::thread::spawn(move || server.serve_forever(|_| {}));

    let mut stream = TcpStream::connect(&addr).expect("connects to the host");
    write!(stream, "GET {target} HTTP/1.1\r\nHost: {addr}\r\n\r\n").expect("writes a request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("reads the response");
    drop(handle);
    response
}

fn status_line(response: &str) -> &str {
    response.lines().next().unwrap_or_default()
}

fn body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("")
}

fn matching_query(manifest: &str) -> String {
    let content = load_content(".", manifest).expect("content loads");
    let stamp = content.manifest.stamp;
    format!(
        "protocol={}&content_id={}&content_epoch={}",
        stamp.protocol, stamp.content_id, stamp.content_epoch
    )
}

#[test]
fn the_host_publishes_its_stamp_over_a_real_socket() {
    let response = request(BASE_MANIFEST, "/host/stamp.json");
    assert!(
        status_line(&response).starts_with("HTTP/1.1 200 OK"),
        "{response}"
    );
    assert!(response.contains("Content-Type: application/json; charset=utf-8"));
    // The stamp endpoint must never be cached: it is how a client discovers
    // that the host it is talking to has changed underneath it.
    assert!(response.contains("Cache-Control: no-cache"));
    let stamp = body(&response);
    assert!(
        stamp.contains(&format!("\"protocol\":{PROTOCOL_VERSION}")),
        "{stamp}"
    );
    assert!(stamp.contains("\"content_id\":\"phoenix-base\""), "{stamp}");
}

#[test]
fn a_stamped_client_is_served_the_catalogue_the_manifest_authorises() {
    let target = format!("/host/manifest.json?{}", matching_query(BASE_MANIFEST));
    let response = request(BASE_MANIFEST, &target);
    assert!(
        status_line(&response).starts_with("HTTP/1.1 200 OK"),
        "{response}"
    );
    let body = body(&response);
    assert!(body.contains("\"combat_test\""), "{body}");
    assert!(
        body.contains("\"manifest_path\":\"assets/scenarios.toml\""),
        "{body}"
    );
}

#[test]
fn a_client_stamped_for_another_protocol_is_refused_with_both_versions_named() {
    let target = format!(
        "/host/manifest.json?protocol={}&content_id=phoenix-base&content_epoch=1",
        PROTOCOL_VERSION + 1
    );
    let response = request(BASE_MANIFEST, &target);
    assert!(
        status_line(&response).starts_with("HTTP/1.1 409 Conflict"),
        "{response}"
    );
    assert!(
        response.contains("X-Phoenix-Refusal: protocol-mismatch"),
        "{response}"
    );
    let body = body(&response);
    assert!(body.contains("protocol-mismatch"), "{body}");
    assert!(body.contains(&PROTOCOL_VERSION.to_string()), "{body}");
    assert!(body.contains(&(PROTOCOL_VERSION + 1).to_string()), "{body}");
    // The catalogue is NOT leaked alongside the refusal.
    assert!(!body.contains("\"scenarios\""), "{body}");
}

#[test]
fn an_unstamped_client_is_refused_rather_than_served_the_catalogue() {
    let response = request(BASE_MANIFEST, "/host/manifest.json");
    assert!(
        status_line(&response).starts_with("HTTP/1.1 409 Conflict"),
        "{response}"
    );
    assert!(
        response.contains("X-Phoenix-Refusal: client-stamp-missing"),
        "{response}"
    );
}

#[test]
fn a_host_serving_no_bundle_answers_404_for_a_client_asset() {
    let response = request(BASE_MANIFEST, "/index.html");
    assert!(
        status_line(&response).starts_with("HTTP/1.1 404 Not Found"),
        "{response}"
    );
}

#[test]
fn the_curated_public_manifest_really_does_restrict_what_the_native_host_publishes() {
    // The enrichment fields (class, hull id, power rating, name) come from the
    // template cache, so preload before comparing — and this is the one test
    // file allowed to touch it.
    preload_templates(".").expect("templates preload");

    let target = format!("/host/manifest.json?{}", matching_query(DEMO_MANIFEST));
    let response = request(DEMO_MANIFEST, &target);
    assert!(
        status_line(&response).starts_with("HTTP/1.1 200 OK"),
        "{response}"
    );
    let demo = load_content(".", DEMO_MANIFEST).expect("demo content loads");
    let base = load_content(".", BASE_MANIFEST).expect("base content loads");

    // Issue #931's curation, asserted against the served document rather than
    // against the TOML: one scenario, one hull.
    assert_eq!(
        demo.manifest.scenarios.len(),
        1,
        "the public demo manifest must publish exactly one scenario"
    );
    let scenario = &demo.manifest.scenarios[0];
    assert_eq!(
        scenario.get("id").and_then(PayloadValue::as_text),
        Some("combat_test")
    );
    assert_eq!(
        scenario.ships().len(),
        1,
        "the public demo manifest must publish exactly one hull"
    );
    assert_eq!(
        scenario.ships()[0]
            .get("template_path")
            .and_then(PayloadValue::as_text),
        Some("assets/entities/alliance_destroyer.toml")
    );

    // And it really is a restriction of the dev catalogue, not a different one:
    // the dev catalogue offers strictly more.
    assert!(
        base.manifest.scenarios.len() > demo.manifest.scenarios.len(),
        "the dev catalogue should offer more scenarios than the curated one"
    );
    let base_combat = base
        .manifest
        .scenarios
        .iter()
        .find(|s| s.get("id").and_then(PayloadValue::as_text) == Some("combat_test"))
        .expect("combat_test is in the dev catalogue");
    assert!(
        base_combat.ships().len() > scenario.ships().len(),
        "combat_test authors more hulls than the curated manifest publishes"
    );

    // Curation filters the catalogue; it never edits the world.
    let world = std::fs::read_to_string("assets/worlds/combat_test.toml").expect("world reads");
    assert!(world.contains("alliance_cruiser.toml"));
}

#[test]
fn the_enrichment_fields_reach_the_published_hull_once_templates_are_loaded() {
    preload_templates(".").expect("templates preload");
    let content = load_content(".", DEMO_MANIFEST).expect("demo content loads");
    let ship = &content.manifest.scenarios[0].ships()[0];
    let keys: Vec<&str> = ship.entries().iter().map(|(k, _)| *k).collect();
    // The exact enrichment a hull carries is authored, but `class` is on every
    // shipped Alliance hull and is what the ship picker reads.
    assert!(keys.contains(&"template_path"), "{keys:?}");
    assert!(keys.contains(&"label"), "{keys:?}");
    assert!(keys.contains(&"class"), "{keys:?}");
}

#[test]
fn the_native_hosts_catalogue_is_the_browser_hosts_catalogue_with_no_packs_applied() {
    preload_templates(".").expect("templates preload");
    let manifest_toml = std::fs::read_to_string(BASE_MANIFEST).expect("manifest reads");
    let manifest = parse_manifest(&manifest_toml).expect("manifest parses");
    let resolve = |rel: &str| std::fs::read_to_string(rel).ok();

    // What the native host publishes: `build_catalog` through the shared payload
    // and the shared encoder.
    let native = codec::encode_delivery_manifest(&DeliveryManifest {
        stamp: DeliveryStamp::for_manifest(&manifest_toml),
        manifest_path: BASE_MANIFEST.to_string(),
        scenarios: catalog_payload(&build_catalog(&manifest, resolve)),
    });

    // What the browser host publishes: `build_merged_catalog` with an empty
    // overlay stack — the same call `bridge::wasm_get_scenario_catalog` makes
    // when no mod pack has been uploaded — through the same payload and encoder.
    let browser = codec::encode_delivery_manifest(&DeliveryManifest {
        stamp: DeliveryStamp::for_manifest(&manifest_toml),
        manifest_path: BASE_MANIFEST.to_string(),
        scenarios: catalog_payload(&build_merged_catalog(&manifest, &[], resolve).catalog),
    });

    assert_eq!(
        native, browser,
        "the native host and the browser host must publish byte-identical \
         catalogue JSON for the same content"
    );
    assert!(native.contains("\"source\":\"base\""), "{native}");
}

#[test]
fn a_client_bundle_built_for_other_content_stops_the_host_before_it_takes_the_port() {
    // A bundle-shaped directory whose manifest declares a different content set.
    let dir = std::env::temp_dir().join("phoenix-host-bundle-mismatch");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("fixture dir");
    std::fs::write(
        dir.join("assets/scenarios.toml"),
        "[content]\nid = \"some-other-game\"\nepoch = 1\n",
    )
    .expect("fixture manifest");

    let mut a = args(BASE_MANIFEST);
    a.client = ClientSource::Bundled {
        dir: dir.to_string_lossy().into_owned(),
    };
    let err = match HostServer::bind(&a) {
        Ok(_) => panic!("a mismatched bundle must not start"),
        Err(e) => e,
    };
    assert!(err.starts_with("content-id-mismatch"), "{err}");
    assert!(err.contains("some-other-game"), "{err}");

    // `--skip-bundle-check` forgives "I could not tell", never a real mismatch.
    a.skip_bundle_check = true;
    let err = match HostServer::bind(&a) {
        Ok(_) => panic!("skip-bundle-check must not forgive a mismatch"),
        Err(e) => e,
    };
    assert!(err.starts_with("content-id-mismatch"), "{err}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_matching_client_bundle_is_served_from_disk_with_the_caching_contract() {
    let dir = std::env::temp_dir().join("phoenix-host-bundle-ok");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    // The bundle states the same content identity the host serves.
    std::fs::create_dir_all(dir.join("assets")).expect("fixture assets");
    std::fs::copy(BASE_MANIFEST, dir.join("assets/scenarios.toml")).expect("bundle manifest");
    std::fs::write(dir.join("index.html"), "<!doctype html><title>x</title>")
        .expect("fixture index");
    std::fs::write(
        dir.join("project-phoenix-0123456789abcdef_bg.wasm"),
        b"\0asm",
    )
    .expect("fixture wasm");

    let mut a = args(BASE_MANIFEST);
    a.client = ClientSource::Bundled {
        dir: dir.to_string_lossy().into_owned(),
    };
    let server = HostServer::bind(&a).expect("a matching bundle starts");
    let addr = server.local_addr();
    std::thread::spawn(move || server.serve_forever(|_| {}));

    let fetch = |target: &str| -> String {
        let mut stream = TcpStream::connect(&addr).expect("connects");
        write!(stream, "GET {target} HTTP/1.1\r\nHost: {addr}\r\n\r\n").expect("writes");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("reads");
        response
    };

    // `/` resolves to index.html and must always be revalidated.
    let index = fetch("/");
    assert!(
        status_line(&index).starts_with("HTTP/1.1 200 OK"),
        "{index}"
    );
    assert!(
        index.contains("Content-Type: text/html; charset=utf-8"),
        "{index}"
    );
    assert!(index.contains("Cache-Control: no-cache"), "{index}");

    // A content-addressed bundle is immutable and served as application/wasm —
    // the header a browser's streaming instantiation refuses to do without.
    let wasm = fetch("/project-phoenix-0123456789abcdef_bg.wasm");
    assert!(wasm.contains("Content-Type: application/wasm"), "{wasm}");
    assert!(
        wasm.contains("Cache-Control: public, max-age=31536000, immutable"),
        "{wasm}"
    );

    // Traversal out of the bundle is refused, not served.
    let escape = fetch("/../../Cargo.toml");
    assert!(
        status_line(&escape).starts_with("HTTP/1.1 404 Not Found"),
        "{escape}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
