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
//! Native request parsing uses `httparse`; path, MIME, cache and stamp policy
//! remain shared with the browser. The parser dependency is native-only.

/// 4 hours — the deliberate cap for a content-addressed asset, matching the
/// Cloudflare dashboard Cache Rule this contract mirrors rather than the
/// year-long ceiling a hashed filename would otherwise licence.
pub const CONTENT_ADDRESSED_MAX_AGE: u32 = 14_400;
/// An hour, for assets that are stable within a deploy but not hash-named.
pub const SHORT_MAX_AGE: u32 = 3_600;

/// The endpoint publishing the host's own [`crate::delivery::stamp`].
pub const STAMP_PATH: &str = "/host/stamp.json";
/// Read-only current native asset revision; no gameplay or content mutation.
pub const ASSET_REVISION_PATH: &str = "/host/asset-revision.json";
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
    pub version: HttpVersion,
    /// Path with the query string removed, percent-decoded.
    pub path: String,
    pub query: Vec<(String, String)>,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
}

/// Supported versions of the native HTTP request head.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HttpVersion {
    Http10,
    #[default]
    Http11,
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

/// Parse an HTTP/1.x request head, including its terminating blank line.
///
/// Returns `None` for anything that is not a well-formed request line, which
/// the socket loop answers with `400` — this host speaks to browsers and to
/// `curl`, and guessing at a malformed head is how a static server grows a
/// parser bug.
#[cfg(not(target_arch = "wasm32"))]
pub fn parse_request(head: &str) -> Option<Request> {
    match parse_request_bytes(head.as_bytes()).ok()? {
        httparse::Status::Complete((end, request)) if end == head.len() => Some(request),
        _ => None,
    }
}

/// Incremental native parsing. The completion offset belongs to the HTTP head;
/// later bytes belong to the upgraded protocol and must travel with the socket.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn parse_request_bytes(bytes: &[u8]) -> Result<httparse::Status<(usize, Request)>, ()> {
    let mut headers = [httparse::EMPTY_HEADER; 128];
    let mut parsed = httparse::Request::new(&mut headers);
    let end = match parsed.parse(bytes).map_err(|_| ())? {
        httparse::Status::Partial => return Ok(httparse::Status::Partial),
        httparse::Status::Complete(end) => end,
    };
    let method = parsed.method.ok_or(())?.to_string();
    let target = parsed.path.ok_or(())?;
    let version = match parsed.version {
        Some(0) => HttpVersion::Http10,
        Some(1) => HttpVersion::Http11,
        _ => return Err(()),
    };
    let (raw_path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };
    let path = percent_decode(raw_path);
    let query = raw_query.map(parse_query).unwrap_or_default();

    let mut headers: Vec<(String, String)> = Vec::new();
    for header in parsed.headers.iter() {
        let name = header.name.to_ascii_lowercase();
        let value = std::str::from_utf8(header.value).map_err(|_| ())?.trim();
        let existing = headers.iter_mut().find(|(key, _)| key == &name);
        match (name.as_str(), existing) {
            ("connection" | "upgrade", Some((_, previous))) => {
                previous.push_str(", ");
                previous.push_str(value);
                continue;
            }
            (
                "host"
                | "sec-websocket-key"
                | "sec-websocket-version"
                | CLIENT_STAMP_HEADER
                | "content-length",
                Some(_),
            ) => return Err(()),
            _ => {}
        }
        headers.push((name, value.to_string()));
    }
    Ok(httparse::Status::Complete((
        end,
        Request {
            method,
            version,
            path,
            query,
            headers,
        },
    )))
}

#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
            c if c.contains(['\\', ':']) || c.chars().any(char::is_control) => {
                return Err(PathRefusal::Traversal);
            }
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
#[path = "http_tests.rs"]
mod tests;
