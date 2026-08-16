//! The delivery HTTP surface, as pure functions.
//!
//! Everything here is string/byte in, string/byte out: request-line parsing,
//! the static-path guard, the MIME table and the caching policy. The socket
//! loop that calls it lives in [`crate::delivery::serve`] and is the only part
//! that touches the network, so the contract this module states is testable
//! without binding a port — and the same policy table is what
//! `scripts/check-deploy-headers.mjs` asserts against a *deployed* URL, so the
//! native host and the Cloudflare path are held to one contract.
//!
//! Hand-rolled rather than a web framework, for the reason
//! `headless::args` is hand-rolled rather than `clap`: this crate's primary
//! target is `wasm32-unknown-unknown` with `lto = true`, so every dependency is
//! paid for by the browser build unless it is target-gated, and what is needed
//! here is a static file server and two JSON endpoints.

/// 4 hours — the deliberate cap for a content-addressed asset, matching the
/// Cloudflare dashboard Cache Rule this contract mirrors rather than the
/// year-long ceiling a hashed filename would otherwise licence.
pub const CONTENT_ADDRESSED_MAX_AGE: u32 = 14_400;
/// An hour, for assets that are stable within a deploy but not hash-named.
pub const SHORT_MAX_AGE: u32 = 3_600;

/// The endpoint publishing the host's own [`crate::delivery::stamp`].
pub const STAMP_PATH: &str = "/host/stamp.json";
/// The endpoint publishing the version-pinned content manifest + catalogue.
pub const MANIFEST_PATH: &str = "/host/manifest.json";
/// Header a client may carry its stamp in, as `protocol/content_id/epoch`.
/// The query parameters are the other accepted form; a header keeps the stamp
/// out of the URL, which matters for a cached GET.
pub const CLIENT_STAMP_HEADER: &str = "x-phoenix-client-stamp";

/// How a response may be cached. One enum so the native host and the deployed
/// header check cannot drift into two policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachePolicy {
    /// Content-addressed: the filename changes when the bytes do.
    Immutable,
    /// Must be revalidated every time — the entry points and the authored
    /// manifests that decide what a session even loads.
    Revalidate,
    /// Stable within a deploy but not hash-named: models, textures, audio.
    ShortLived,
}

impl CachePolicy {
    /// The `Cache-Control` value this policy sends.
    pub fn header_value(self) -> String {
        match self {
            CachePolicy::Immutable => {
                format!("public, max-age={CONTENT_ADDRESSED_MAX_AGE}, must-revalidate")
            }
            CachePolicy::Revalidate => "no-cache".to_string(),
            CachePolicy::ShortLived => format!("public, max-age={SHORT_MAX_AGE}"),
        }
    }
}

/// Is this file name content-addressed?
///
/// Trunk emits `<stem>-<hex>.<ext>` (and `<stem>-<hex>_bg.wasm`) with a hash of
/// at least 8 hex digits. A shorter or non-hex trailing segment is treated as
/// authored — `alliance-destroyer.glb` must not be cached for a year because it
/// happens to contain a dash.
pub fn is_hashed_asset(file_name: &str) -> bool {
    let stem = file_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(file_name);
    let stem = stem.strip_suffix("_bg").unwrap_or(stem);
    let Some((_, last)) = stem.rsplit_once('-') else {
        return false;
    };
    last.len() >= 8 && last.chars().all(|c| c.is_ascii_hexdigit())
}

/// The caching policy for a served path.
pub fn cache_policy_for(path: &str) -> CachePolicy {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    // A hashed name wins outright: that is what "immutable" means.
    if is_hashed_asset(file_name) {
        return CachePolicy::Immutable;
    }
    match extension_of(file_name) {
        // The entry points and the authored data that decides what loads.
        Some("html") | Some("toml") | Some("csv") | None => CachePolicy::Revalidate,
        Some("json") => CachePolicy::Revalidate,
        _ => CachePolicy::ShortLived,
    }
}

fn extension_of(file_name: &str) -> Option<&str> {
    file_name.rsplit_once('.').map(|(_, e)| e)
}

/// The `Content-Type` for a served path.
///
/// `.wasm` is the one that is not merely tidy: a browser's
/// `instantiateStreaming` refuses anything but `application/wasm`, and a host
/// that answers `application/octet-stream` fails at instantiation with an error
/// that names neither the file nor the header.
pub fn content_type_for(path: &str) -> &'static str {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    match extension_of(file_name) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json; charset=utf-8",
        Some("toml") => "text/plain; charset=utf-8",
        Some("csv") => "text/csv; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("glb") | Some("gltf") => "model/gltf-binary",
        Some("ktx2") => "image/ktx2",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("gz") => "application/gzip",
        _ => "application/octet-stream",
    }
}

/// A parsed request head. Only what the delivery surface reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// Path with the query string removed, percent-decoded.
    pub path: String,
    pub query: Vec<(String, String)>,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn query_param(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Parse an HTTP/1.x request head (everything before the blank line).
///
/// Returns `None` for anything that is not a well-formed request line, which
/// the socket loop answers with `400` — this host speaks to browsers and to
/// `curl`, and guessing at a malformed head is how a static server grows a
/// parser bug.
pub fn parse_request(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n").filter(|l| !l.is_empty());
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    // The version is present but unread: this host answers HTTP/1.1 either way
    // and closes the connection, so there is nothing to negotiate.
    parts.next()?;

    let (raw_path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };
    let path = percent_decode(raw_path);
    let query = raw_query.map(parse_query).unwrap_or_default();

    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();

    Some(Request {
        method,
        path,
        query,
        headers,
    })
}

fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// Percent-decode, treating `+` as a literal `+` (this is a path/query reader,
/// not a form decoder) and leaving malformed escapes as written.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Why a URL path may not be served from the client directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathRefusal {
    /// The path escapes the client directory, or tries to.
    Traversal,
    /// The path is not rooted at `/`.
    NotAbsolute,
}

/// Resolve a URL path to a `/`-joined path relative to the client directory.
///
/// The whole traversal guard is here, on components rather than on the raw
/// string, because a substring check for `".."` is defeated by `%2e%2e` (which
/// [`parse_request`] has already decoded by this point) and by a trailing
/// `..%2f`. Rejecting the *component* is what makes the decode order safe.
///
/// A directory request (`/`, or any path ending in `/`) resolves to its
/// `index.html`, matching what Pages and every static host do.
pub fn resolve_static_path(url_path: &str) -> Result<String, PathRefusal> {
    if !url_path.starts_with('/') {
        return Err(PathRefusal::NotAbsolute);
    }
    let mut parts: Vec<&str> = Vec::new();
    for component in url_path.split('/') {
        match component {
            "" | "." => continue,
            ".." => return Err(PathRefusal::Traversal),
            // A backslash cannot appear in a path component on the wire, and on
            // Windows it would be a second separator the guard above never saw.
            c if c.contains('\\') => return Err(PathRefusal::Traversal),
            c => parts.push(c),
        }
    }
    if parts.is_empty() || url_path.ends_with('/') {
        parts.push("index.html");
    }
    Ok(parts.join("/"))
}

/// Build a response head. The body is written by the caller so a large asset
/// never has to be concatenated into one string.
pub fn response_head(
    status: u16,
    reason: &str,
    content_type: &str,
    cache: CachePolicy,
    content_length: usize,
    extra: &[(&str, String)],
) -> String {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {content_length}\r\n\
         Cache-Control: {}\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n",
        cache.header_value()
    );
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_line_yields_method_path_and_query() {
        let r = parse_request("GET /host/manifest.json?protocol=1&content_id=phoenix-base HTTP/1.1\r\nHost: localhost\r\n").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/host/manifest.json");
        assert_eq!(r.query_param("protocol"), Some("1"));
        assert_eq!(r.query_param("content_id"), Some("phoenix-base"));
        assert_eq!(r.query_param("absent"), None);
        assert_eq!(r.header("host"), Some("localhost"));
    }

    #[test]
    fn header_names_are_matched_case_insensitively() {
        let r = parse_request("GET / HTTP/1.1\r\nX-Phoenix-Client-Stamp: 1/phoenix-base/1\r\n")
            .unwrap();
        assert_eq!(r.header(CLIENT_STAMP_HEADER), Some("1/phoenix-base/1"));
    }

    #[test]
    fn a_malformed_request_line_is_refused_rather_than_guessed_at() {
        assert!(parse_request("GET\r\n").is_none());
        assert!(parse_request("").is_none());
        assert!(parse_request("GET /only-two-fields\r\n").is_none());
    }

    #[test]
    fn a_percent_escape_in_the_path_is_decoded_before_the_traversal_guard_runs() {
        let r = parse_request("GET /%2e%2e/secrets HTTP/1.1\r\n").unwrap();
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
}
