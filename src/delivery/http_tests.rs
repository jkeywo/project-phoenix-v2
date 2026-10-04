use super::*;

#[test]
fn strict_complete_heads_and_header_duplicates() {
    for head in [
        "GET / HTTP/1.1\r\n",
        "GET / HTTP/2.0\r\n\r\n",
        "GET / HTTP/1.1\r\nBad Header: x\r\n\r\n",
        "GET / HTTP/1.1\r\nBroken\r\n\r\n",
    ] {
        assert!(parse_request(head).is_none(), "accepted {head:?}");
    }
    assert_eq!(
        parse_request("GET / HTTP/1.0\r\n\r\n").unwrap().version,
        HttpVersion::Http10
    );
    for header in [
        "Host",
        "Sec-WebSocket-Key",
        "Sec-WebSocket-Version",
        "X-Phoenix-Client-Stamp",
        "Content-Length",
    ] {
        assert!(parse_request(&format!(
            "GET / HTTP/1.1\r\n{header}: x\r\n{header}: x\r\n\r\n"
        ))
        .is_none());
    }
    let request = parse_request("GET / HTTP/1.1\r\nConnection: keep-alive\r\nconnection: Upgrade\r\nUpgrade: other\r\nupgrade: WebSocket\r\nX-Test: first\r\nX-Test: second\r\n\r\n").unwrap();
    assert_eq!(request.header("connection"), Some("keep-alive, Upgrade"));
    assert_eq!(request.header("upgrade"), Some("other, WebSocket"));
    assert_eq!(request.header("x-test"), Some("first"));
}

#[test]
fn windows_and_encoded_path_escapes_are_refused() {
    for path in [
        "/../secret",
        "/%2e%2e/secret",
        "/C:/secret",
        "/C:secret",
        "/C%3A/secret",
        "/dir%5c..%5csecret",
        "/%5c%5cserver/share",
        "/file%00",
        "/file%1f",
        "/file%7f",
        "/file:stream",
    ] {
        let request = parse_request(&format!("GET {path} HTTP/1.1\r\n\r\n")).unwrap();
        assert!(
            resolve_static_path(&request.path).is_err(),
            "accepted {path}"
        );
    }
    assert_eq!(resolve_static_path("/dir/"), Ok("dir/index.html".into()));
}

#[test]
fn a_request_line_yields_method_path_and_query() {
    let r = parse_request("GET /host/manifest.json?protocol=1&content_id=phoenix-base HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    assert_eq!(r.method, "GET");
    assert_eq!(r.path, "/host/manifest.json");
    assert_eq!(r.query_param("protocol"), Some("1"));
    assert_eq!(r.query_param("content_id"), Some("phoenix-base"));
    assert_eq!(r.query_param("absent"), None);
    assert_eq!(r.header("host"), Some("localhost"));
}

#[test]
fn header_names_are_matched_case_insensitively() {
    let r = parse_request("GET / HTTP/1.1\r\nX-Phoenix-Client-Stamp: 1/phoenix-base/1\r\n\r\n")
        .unwrap();
    assert_eq!(r.header(CLIENT_STAMP_HEADER), Some("1/phoenix-base/1"));
}

#[test]
fn a_malformed_request_line_is_refused_rather_than_guessed_at() {
    assert!(parse_request("GET\r\n\r\n").is_none());
    assert!(parse_request("").is_none());
    assert!(parse_request("GET /only-two-fields\r\n\r\n").is_none());
}

#[test]
fn a_percent_escape_in_the_path_is_decoded_before_the_traversal_guard_runs() {
    let r = parse_request("GET /%2e%2e/secrets HTTP/1.1\r\n\r\n").unwrap();
    assert_eq!(r.path, "/../secrets");
    assert_eq!(
        resolve_static_path(&r.path).unwrap_err(),
        PathRefusal::Traversal
    );
}

#[test]
fn a_directory_request_resolves_to_its_index() {
    assert_eq!(resolve_static_path("/").unwrap(), "index.html");
    assert_eq!(
        resolve_static_path("/client/").unwrap(),
        "client/index.html"
    );
}

#[test]
fn a_nested_asset_keeps_its_path() {
    assert_eq!(
        resolve_static_path("/assets/worlds/combat_test.toml").unwrap(),
        "assets/worlds/combat_test.toml"
    );
}

#[test]
fn a_backslash_component_is_refused_because_windows_would_split_on_it() {
    assert_eq!(
        resolve_static_path("/assets\\..\\..\\secrets").unwrap_err(),
        PathRefusal::Traversal
    );
}

#[test]
fn a_path_that_is_not_absolute_is_refused() {
    assert_eq!(
        resolve_static_path("assets/x.toml").unwrap_err(),
        PathRefusal::NotAbsolute
    );
}

#[test]
fn a_trunk_hashed_bundle_is_immutable_and_an_authored_asset_is_not() {
    assert!(is_hashed_asset("project-phoenix-6f3a91b2c4d5e607.js"));
    assert!(is_hashed_asset("project-phoenix-6f3a91b2c4d5e607_bg.wasm"));
    assert!(!is_hashed_asset("alliance-destroyer.glb"));
    assert!(!is_hashed_asset("index.html"));
    // Too short to be a content hash — an authored suffix, not an address.
    assert!(!is_hashed_asset("thing-abc123.js"));
}

#[test]
fn the_entry_points_and_the_manifests_always_revalidate() {
    assert_eq!(cache_policy_for("/index.html"), CachePolicy::Revalidate);
    assert_eq!(
        cache_policy_for("/assets/scenarios.toml"),
        CachePolicy::Revalidate
    );
    assert_eq!(cache_policy_for(MANIFEST_PATH), CachePolicy::Revalidate);
    assert_eq!(
        cache_policy_for("/assets/strings/strings.csv"),
        CachePolicy::Revalidate
    );
}

#[test]
fn a_hashed_bundle_is_cached_for_4_hours_and_a_model_for_an_hour() {
    assert_eq!(
        cache_policy_for("/project-phoenix-6f3a91b2c4d5e607_bg.wasm"),
        CachePolicy::Immutable
    );
    assert!(CachePolicy::Immutable
        .header_value()
        .contains("max-age=14400"));
    assert!(CachePolicy::Immutable
        .header_value()
        .contains("must-revalidate"));
    assert_eq!(
        cache_policy_for("/assets/models/alliance_destroyer.glb"),
        CachePolicy::ShortLived
    );
}

#[test]
fn wasm_is_served_as_application_wasm_because_streaming_instantiation_demands_it() {
    assert_eq!(
        content_type_for("/project-phoenix-6f3a91b2c4d5e607_bg.wasm"),
        "application/wasm"
    );
    assert_eq!(content_type_for("/index.html"), "text/html; charset=utf-8");
    assert_eq!(
        content_type_for("/assets/models/x.glb"),
        "model/gltf-binary"
    );
    assert_eq!(content_type_for("/unknown.xyz"), "application/octet-stream");
}

#[test]
fn a_response_head_states_type_length_cache_and_nosniff() {
    let head = response_head(
        200,
        "OK",
        "application/json",
        CachePolicy::Revalidate,
        17,
        &[],
    );
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains("Content-Type: application/json\r\n"));
    assert!(head.contains("Content-Length: 17\r\n"));
    assert!(head.contains("Cache-Control: no-cache\r\n"));
    assert!(head.contains("X-Content-Type-Options: nosniff\r\n"));
    assert!(head.ends_with("\r\n\r\n"));
}

#[test]
fn extra_headers_are_appended_before_the_blank_line() {
    let head = response_head(
        409,
        "Conflict",
        "application/json",
        CachePolicy::Revalidate,
        2,
        &[("X-Phoenix-Refusal", "protocol-mismatch".to_string())],
    );
    assert!(head.contains("X-Phoenix-Refusal: protocol-mismatch\r\n"));
    assert!(head.ends_with("\r\n\r\n"));
}
