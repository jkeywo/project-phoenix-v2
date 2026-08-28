//! What a pane loads, and where it loads it from (issue #1122).
//!
//! # The choice, and why
//!
//! A pane must show *the complete normal console surface* — the same
//! `client/index.html`, the same `gui/*.js` modules, the same per-console
//! `gui/<name>-console.html` iframes a phone loads. Four routes were available
//! and three of them are worse:
//!
//! 1. **Reimplement the client shell natively.** Immediately a second client,
//!    forever behind the first. Refused on sight.
//! 2. **`load_html` an `include_str!`'d document**, as void-and-thunder's HUD
//!    does. A document loaded from a string has **no base URL**, so every
//!    relative `<script src>`, stylesheet, iframe and `fetch` in it fails —
//!    and `fetch` fails *silently*. A `<base href>` fixes URL resolution but
//!    not the security origin, which stays unique, so every cross-origin fetch
//!    to the host's own address is then refused by CORS.
//! 3. **`load_url` the bundle's real `client/index.html`.** Correct origin,
//!    everything resolves — and no way in. The page's transport is PeerJS
//!    (browser JavaScript, no use in a native process, and behind a CDN script
//!    tag this machine may not be able to reach), and the injection points that
//!    would replace it are all inside the document.
//! 4. **`load_url` an *injected copy* of that same document, served by the host
//!    that is already serving the bundle.** ← this one.
//!
//! So: the pane's document is `client/index.html`'s own bytes with three edits
//! ([`build_pane_document`]), published in memory by the delivery server at
//! `/client/pane-<n>.html` and loaded over `http://<host>/…`. Being *at the
//! client directory's own depth* is the point of that path — every relative URL
//! in the page then resolves exactly as it does for a phone, with no `<base>`
//! tag and no rewriting, and every asset arrives same-origin from the machinery
//! PRD #855 already built. Nothing in `gui/` or in any console page is touched;
//! the pane-specific shim is native-side, here.
//!
//! # The three edits
//!
//! | edit | why |
//! |---|---|
//! | a classic `<script>` first in `<head>` ([`PANE_BOOT_JS`]) | seeds the session token and name the page's own inline script reads **at parse time**, and installs the page→host queue |
//! | the PeerJS CDN `<script>` tag removed | a pane never uses PeerJS, and a bridge machine may not be able to reach unpkg.com — waiting for that to fail is pure boot latency |
//! | a `<script type="module">` last in `<body>` ([`PANE_LINK_JS`]) | replaces `window.connectionManager` with the in-process link, which must happen **after** `gui/connection-manager.js` has published its own |
//!
//! Both scripts are documented in their own files. The page is unchanged in
//! every other byte, which is what makes "the same console surface" true rather
//! than approximately true.

use vellum_ultralight::bridge::{js_string_literal, queue_shim};

use super::identity::PaneIdentity;
use super::registry::PaneId;

/// The first script in a pane document. See `pane_boot.js`.
pub const PANE_BOOT_JS: &str = include_str!("pane_boot.js");

/// The last script in a pane document. See `pane_link.js`.
pub const PANE_LINK_JS: &str = include_str!("pane_link.js");

/// The JavaScript namespace the page→host queue lives under.
///
/// `window.phoenixPaneOut.send(record)` queues; `window.__phoenixPaneOutDrain()`
/// is what the host evaluates once a frame. Deliberately distinct from
/// `window.__phoenixPane`, which is the pane's own identity and inbox.
pub const PANE_OUT_NAMESPACE: &str = "phoenixPaneOut";

/// The script the host evaluates once a frame to collect what the page asked
/// for.
pub fn pane_drain_script() -> String {
    vellum_ultralight::bridge::drain_call(&format!("window.__{PANE_OUT_NAMESPACE}Drain"))
}

/// The script that hands the page one encoded `ServerMessage`.
pub fn pane_apply_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixPaneApply", json)
}

/// Why a client bundle could not be turned into a pane document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentError {
    /// The document has no `<head>`, so the boot script has nowhere to go that
    /// runs before the page's own inline script.
    NoHead,
    /// The document has no `</body>`, so the link script has nowhere to go that
    /// runs after `gui/connection-manager.js`.
    NoBody,
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentError::NoHead => write!(
                f,
                "the client page has no <head>; a pane's boot script must run before the \
                 page's own inline script, and there is nowhere to put it"
            ),
            DocumentError::NoBody => write!(
                f,
                "the client page has no </body>; a pane's link script must run after \
                 gui/connection-manager.js, and there is nowhere to put it"
            ),
        }
    }
}

impl std::error::Error for DocumentError {}

/// Where a pane's document is published, relative to the served root.
///
/// At the client directory's own depth, so every relative URL in the page
/// resolves exactly as it does for a phone. See the module note.
pub fn pane_document_path(id: PaneId) -> String {
    format!("/client/pane-{}.html", id.0)
}

/// The URL a pane's view navigates to.
///
/// The fragment is what puts the page on its "join a host" route at all:
/// `gui/rendezvous-transport.js`'s `joinRouteFromLocation` reads a fragment with
/// no underscore as a peer id, and an empty fragment as "nothing to join", which
/// makes the page print a status line and stop. The value itself is never used
/// — the pane link ignores the host id it is handed — but it has to be there.
pub fn pane_url(host_addr: &str, id: PaneId) -> String {
    format!("http://{host_addr}{}#native", pane_document_path(id))
}

/// The PeerJS CDN script tag a pane document drops.
const PEERJS_CDN_MARKER: &str = "unpkg.com/peerjs";

/// Build a pane's document from the client bundle's own `index.html`.
///
/// Pure: bytes in, bytes out. The whole of what makes a pane document different
/// from the page a phone loads is decided here, so it is decided somewhere a
/// unit test can read without an SDK, a GPU or an HTTP server.
pub fn build_pane_document(
    client_index_html: &str,
    identity: &PaneIdentity,
) -> Result<String, DocumentError> {
    let head_end = find_tag_end(client_index_html, "<head").ok_or(DocumentError::NoHead)?;
    let boot = format!(
        "\n<script>\nwindow.__phoenixPaneIdentity = {{ token: '{}', name: '{}' }};\n{}\n{}</script>\n",
        js_string_literal(identity.token()),
        js_string_literal(identity.name()),
        queue_shim(PANE_OUT_NAMESPACE),
        PANE_BOOT_JS,
    );
    let mut html = String::with_capacity(client_index_html.len() + boot.len() + PANE_LINK_JS.len());
    html.push_str(&client_index_html[..head_end]);
    html.push_str(&boot);
    html.push_str(&client_index_html[head_end..]);

    let html = strip_script_tags_matching(&html, PEERJS_CDN_MARKER);

    let body_close = html.rfind("</body>").ok_or(DocumentError::NoBody)?;
    let mut out = String::with_capacity(html.len() + PANE_LINK_JS.len() + 64);
    out.push_str(&html[..body_close]);
    out.push_str("\n<script type=\"module\">\n");
    out.push_str(PANE_LINK_JS);
    out.push_str("\n</script>\n");
    out.push_str(&html[body_close..]);
    Ok(out)
}

/// Byte index just past the first `<head…>` tag, or `None`.
fn find_tag_end(html: &str, open: &str) -> Option<usize> {
    let start = html.find(open)?;
    let close = html[start..].find('>')?;
    Some(start + close + 1)
}

/// Remove every `<script …>…</script>` element whose opening tag contains
/// `marker`.
///
/// Deliberately narrow — it matches an opening `<script`, finds its `>`, and
/// requires the matching `</script>` — because the alternative is a general HTML
/// parser for one tag this repository authors itself.
fn strip_script_tags_matching(html: &str, marker: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let Some(open_end) = rest[start..].find('>').map(|i| start + i + 1) else {
            break;
        };
        let Some(close) = rest[open_end..]
            .find("</script>")
            .map(|i| open_end + i + "</script>".len())
        else {
            break;
        };
        if rest[start..open_end].contains(marker) {
            out.push_str(&rest[..start]);
        } else {
            out.push_str(&rest[..close]);
        }
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the real page, reduced to the three things the assembly
    /// depends on: a `<head>`, the PeerJS CDN tag, and a `</body>`.
    const CLIENT: &str = "<!doctype html>\n<html>\n<head>\n\
         <script src=\"gui/bg-raf-keepalive.js\"></script>\n\
         <script src=\"https://unpkg.com/peerjs@1.5.4/dist/peerjs.min.js\"></script>\n\
         <script type=\"module\" src=\"gui/connection-manager.js\"></script>\n\
         </head>\n<body>\n<div id=\"app\"></div>\n<script>var myName = 'x';</script>\n\
         </body>\n</html>\n";

    fn identity() -> PaneIdentity {
        PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap()
    }

    #[test]
    fn the_boot_script_runs_before_the_pages_own_scripts() {
        // It seeds the session token and the player name, both of which the
        // page's inline script reads at parse time. One script too late and the
        // pane joins under a random name on a token nothing else knows.
        let html = build_pane_document(CLIENT, &identity()).unwrap();
        let boot = html.find("__phoenixPaneIdentity").unwrap();
        let first_page_script = html.find("gui/bg-raf-keepalive.js").unwrap();
        assert!(
            boot < first_page_script,
            "the boot script must be the first script in the document"
        );
        assert!(html.find("<head>").unwrap() < boot);
    }

    #[test]
    fn the_link_script_runs_after_the_pages_connection_manager() {
        // Module scripts evaluate in document order, and the link replaces
        // `window.connectionManager` — which gui/connection-manager.js has not
        // published yet if the link runs first.
        let html = build_pane_document(CLIENT, &identity()).unwrap();
        let manager = html.find("gui/connection-manager.js").unwrap();
        let link = html.find("import { localiseTree }").unwrap();
        assert!(manager < link);
        assert!(link < html.rfind("</body>").unwrap());
    }

    #[test]
    fn the_peerjs_cdn_tag_is_dropped_and_nothing_else_is() {
        // A pane never uses PeerJS, and a bridge machine may not be able to
        // reach unpkg.com at all — waiting for that fetch to time out is boot
        // latency for nothing.
        let html = build_pane_document(CLIENT, &identity()).unwrap();
        assert!(!html.contains("unpkg.com/peerjs"));
        assert!(html.contains("gui/bg-raf-keepalive.js"));
        assert!(html.contains("gui/connection-manager.js"));
        assert!(html.contains("var myName = 'x';"));
        assert!(html.contains("<div id=\"app\"></div>"));
    }

    #[test]
    fn the_identity_is_escaped_into_the_document() {
        // A participant name is operator input, and an apostrophe in it would
        // otherwise close the JavaScript string literal it sits inside and turn
        // the rest of the name into code.
        let awkward = PaneIdentity::adopt("tok'en", "O'Neil\\").unwrap();
        let html = build_pane_document(CLIENT, &awkward).unwrap();
        assert!(html.contains(r"token: 'tok\'en'"));
        assert!(html.contains(r"name: 'O\'Neil\\'"));
    }

    #[test]
    fn the_page_to_host_queue_is_installed_under_the_namespace_the_host_drains() {
        let html = build_pane_document(CLIENT, &identity()).unwrap();
        assert!(html.contains("window.phoenixPaneOut.send"));
        assert!(html.contains("window.__phoenixPaneOutDrain"));
        assert_eq!(pane_drain_script(), "window.__phoenixPaneOutDrain()");
    }

    #[test]
    fn a_pushed_message_is_escaped_into_the_apply_call() {
        assert_eq!(
            pane_apply_script(r#"{"type":"Welcome","data":{"name":"O'Neil"}}"#),
            r#"window.__phoenixPaneApply('{"type":"Welcome","data":{"name":"O\'Neil"}}')"#
        );
    }

    #[test]
    fn a_document_with_nowhere_to_inject_is_refused_by_name() {
        assert_eq!(
            build_pane_document("<html><body></body></html>", &identity()),
            Err(DocumentError::NoHead)
        );
        assert_eq!(
            build_pane_document("<html><head></head></html>", &identity()),
            Err(DocumentError::NoBody)
        );
    }

    #[test]
    fn a_panes_document_sits_at_the_client_directorys_own_depth() {
        // This is what makes every relative URL in the page — gui modules,
        // stylesheets, console iframes — resolve exactly as they do for a
        // phone, with no <base> tag and no rewriting.
        assert_eq!(pane_document_path(PaneId(0)), "/client/pane-0.html");
        assert_eq!(
            pane_url("127.0.0.1:8080", PaneId(3)),
            "http://127.0.0.1:8080/client/pane-3.html#native"
        );
    }

    #[test]
    fn the_url_carries_a_fragment_because_an_empty_one_means_nothing_to_join() {
        // `joinRouteFromLocation` reads an empty fragment as `route: 'none'`,
        // which makes the page print a status line and stop before it ever
        // reaches the link.
        let url = pane_url("127.0.0.1:8080", PaneId(0));
        let fragment = url.split('#').nth(1).unwrap();
        assert!(!fragment.is_empty());
        assert!(
            !fragment.contains('_'),
            "an underscore would put the page on the rendezvous route instead"
        );
    }
}
