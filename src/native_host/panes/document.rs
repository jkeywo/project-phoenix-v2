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
//!    everything resolves — and no way in. The page's transport reaches a
//!    rendezvous service over a WebSocket and then a browser-to-browser
//!    DataChannel, neither of which a pane has or wants, and the injection
//!    points that would supply one are all inside the document.
//! 4. **`load_url` an *injected copy* of that same document, served by the host
//!    that is already serving the bundle.** ← this one.
//!
//! So: the pane's document is `client/index.html`'s own bytes with three edits
//! ([`build_pane_document`]), published in memory by the delivery server at
//! `/client/pane-<n>-<nonce>.html` and loaded over `http://<host>/…`. Being *at
//! the client directory's own depth* is the point of that path — every relative
//! URL in the page then resolves exactly as it does for a phone, with no
//! `<base>` tag and no rewriting, and every asset arrives same-origin from the
//! machinery PRD #855 already built. Nothing in `gui/` or in any console page is
//! touched; the pane-specific shim is native-side, here.
//!
//! # The three edits
//!
//! | edit | why |
//! |---|---|
//! | a classic `<script>` first in `<head>` ([`PANE_BOOT_JS`]) | reads the session token and name **out of the URL fragment** at parse time, before the page's own inline script wants them; normalises the fragment down to the join code the page's one route expects; installs the page→host queue |
//! | the PeerJS `<script>` tag removed | a no-op since issue #1112 deleted the tag, and kept because the assertion it backs — no pane document loads PeerJS — is worth making in both states |
//! | the `<audio>` elements removed | see below — this one is not a nicety |
//! | a `<script type="module">` last in `<body>` ([`PANE_LINK_JS`]) | installs `window.PhoenixTransportFactories`, the in-process stand-ins for `WebSocket` and `RTCPeerConnection`, and imports the channel labels from `gui/rendezvous-transport.js` — which is why it is a module and why it runs after the page's own module island |
//!
//! Both scripts are documented in their own files. The page is unchanged in
//! every other byte, which is what makes "the same console surface" true rather
//! than approximately true.
//!
//! # How a pane joins: through the page's own front door
//!
//! A pane is an ORDINARY JOINER in the page's own eyes. `client.html`'s
//! `startPhoenixJoin` runs exactly as it does on a phone: it reads the fragment,
//! resolves a code, builds a `createRendezvousJoiner`, sends `Identify` from its
//! own `getIdent()`, and publishes the `activeLink` façade every console command
//! and the retry control go through. Nothing about that path is pane-aware.
//!
//! What differs is one documented override point:
//! `gui/rendezvous-transport.js`'s `defaultFactories()` reads
//! `window.PhoenixTransportFactories` on every call, and names "a native
//! in-process host" as an intended user of it. [`PANE_LINK_JS`] is that user.
//! Its socket answers the two frames the joiner waits on and relays nothing; its
//! peer connection's reliable channel opens at once, answers the compatibility
//! handshake itself (a pane loads the very bundle this process is serving, so
//! there is no version to disagree about), and is a pipe onto the page→host
//! queue in one direction and `window.__phoenixPaneApply` in the other.
//!
//! The alternative — publishing a link object of the pane's own — is not
//! available and should not be wanted. `currentLink()` reads a closure variable
//! that only `startPhoenixJoin` assigns, so short-circuiting it would mean
//! editing `client.html` to suit a pane, which is the thing this module exists
//! not to do. Going through the front door also means `localiseTree`, the status
//! line, the `#conn-diag` readout and `window.phoenixLink` (which
//! `gui/command-gateway.js` resolves against) are the page's own, not a second
//! implementation of them that can rot the next time the page moves. It rotted
//! once already: the previous arrangement published `window.connectionManager`,
//! a global #1112 retired with PeerJS, and the page never called it.
//!
//! # Private audio and legacy media elements
//!
//! Ultralight ships no media backend: `HTMLMediaElement.play` is simply
//! **undefined**, and calling it throws `TypeError: el.play is not a function`.
//!
//! Older bundles played a click before sending each console command. Removing
//! their media elements keeps that legacy path from throwing before transport.
//! The synthetic legacy-bundle test preserves this compatibility guard.
//!
//! Current bundles use `gui/private-audio.js` for semantic feedback. The native
//! private provider and `native-pane` capability are injected before that module
//! runs, so playback uses the pane's explicitly assigned native output. An
//! unavailable provider remains silent; it never falls back to browser media.
//!
//! # Where the identity is, and where it is emphatically not
//!
//! **The session token and the participant name are in the URL fragment, never
//! in the document.** `phoenix-host` binds `0.0.0.0:8080` by default and has no
//! TLS and no authentication — that is the point of it, it serves a bundle to
//! phones on a LAN — so anything in a served body is readable by anyone on the
//! network. A live participant's session token in that body would be a seat on
//! the bridge handed out by `curl`.
//!
//! A fragment is the one part of a URL a browser never transmits: it is not in
//! the request line, not in a header, and not in any byte this host writes. So
//! [`pane_url`] carries `#<code>&token=…&name=…`, [`PANE_BOOT_JS`] reads
//! `location.hash` at parse time, and [`build_pane_document`] does not take an
//! identity at all — the document is the same bytes for every pane.
//!
//! # Why the fragment also carries a join code
//!
//! The fragment is not free real estate: it is the client page's ONE join input.
//! `joinRouteFromLocation` reads any non-empty fragment as a rendezvous route
//! and hands it to `parseJoinCode`, which refuses `token=…&name=…` and drops the
//! join-entry overlay over the console — a pane that never joins, in front of a
//! field nobody is going to type into.
//!
//! So the pane's route is composed rather than dodged. The fragment leads with
//! [`PANE_JOIN_CODE`], a suffix the authored table in
//! `assets/join/join-codes.toml` accepts, and [`PANE_BOOT_JS`] rewrites
//! `location.hash` down to just that before any page code reads it. From that
//! line on the page's URL is indistinguishable from a phone's that scanned a QR,
//! and the pane's own token has stopped being readable out of `location.hash`
//! into the bargain. The code text itself is never resolved against anything —
//! the pane's socket answers `joined` to whatever it is asked — so it is a
//! sentinel, not a secret.
//!
//! That also retires a second problem rather than fixing it. The identity used
//! to be interpolated into an inline `<script>` element through
//! `vellum_ultralight::bridge::js_string_literal`, which is correct for a
//! JavaScript string literal and **cannot** be correct for an HTML script
//! element: such an element ends at the first case-insensitive `</script`
//! whatever quoting it is inside, so a participant named `x</script><h1>` closed
//! the boot script and injected markup. Nothing operator-supplied is
//! interpolated into the document any more, so there is no escaping question
//! left to get wrong — `js_string_literal` is still used for the one thing it is
//! right for, the `evaluate_script` payloads in [`pane_apply_script`].
//!
//! Three further defences, because one is a single point of failure:
//!
//! 1. the document path carries a per-pane [`mint_document_nonce`], so it cannot
//!    be enumerated as `/client/pane-0.html` once was;
//! 2. `delivery::serve::route` serves a hosted document **only to a loopback
//!    peer** — a pane always connects from this machine, and the bundle and
//!    manifest routes stay LAN-open, which is their job;
//! 3. a closed pane's document is withdrawn (see [`super::PaneBus::close`]), so
//!    the path stops resolving at all.
//!
//! The fragment's own escaping is [`fragment_encode`], and it escapes more than
//! a URL strictly requires — see there.

use vellum_ultralight::bridge::queue_shim;

use super::identity::PaneIdentity;
use super::registry::PaneId;

/// The first script in a pane document. See `pane_boot.js`.
pub const PANE_BOOT_JS: &str = include_str!("pane_boot.js");
pub const OPERATOR_STORAGE_JS: &str = include_str!("operator_storage.js");

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
    /// runs after the page's own module island.
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
                 gui/rendezvous-transport.js, and there is nowhere to put it"
            ),
        }
    }
}

impl std::error::Error for DocumentError {}

/// A fresh unguessable path segment for one pane's document.
///
/// UUIDv4 from the OS, for the same reason [`PaneIdentity::mint`] takes its
/// token from there rather than from `SimRng`: this is transport identity, not
/// simulation state, nothing folds it into the world digest, and a seeded
/// generator would hand two replays of one seed the same "secret" path.
///
/// Hence the scoped allow — `clippy.toml` bans `Uuid::new_v4` so a *world entity
/// id* cannot come from the OS, and this is the other kind of value entirely.
#[allow(clippy::disallowed_methods)]
pub fn mint_document_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Where a pane's document is published, relative to the served root.
///
/// At the client directory's own depth, so every relative URL in the page
/// resolves exactly as it does for a phone. See the module note.
///
/// `nonce` comes from [`mint_document_nonce`] and is what stops the path being
/// *enumerable*: `/client/pane-0.html` is one guess, and the host serves it to
/// whoever asks. The loopback gate in `delivery::serve::route` is the defence
/// that does not depend on a secret staying secret; this one is what keeps a
/// second pane's document out of reach of the first pane's page.
pub fn pane_document_path(id: PaneId, nonce: &str) -> String {
    format!("/client/pane-{}-{nonce}.html", id.0)
}

/// Rewrite a *bind* address into one a client can connect to (issue #1122).
///
/// `phoenix-host` binds `0.0.0.0:8080` by default, and `0.0.0.0` is not a
/// connectable address — it is "every interface", which is a meaningful thing to
/// listen on and a meaningless thing to dial. A pane URL built from it navigates
/// to `http://0.0.0.0:8080/…`, and that it *often* reaches loopback anyway is
/// platform behaviour of `connect()` rather than anything this code establishes.
///
/// A pane always connects to its own process, so loopback is both correct and
/// the least exposed answer. The port is kept; a specific bind is left alone.
pub fn connectable_host_addr(bound: &str) -> String {
    match bound.rsplit_once(':') {
        Some((host, port)) => {
            let replacement = match host.trim() {
                "0.0.0.0" => Some("127.0.0.1"),
                "::" | "[::]" => Some("[::1]"),
                _ => None,
            };
            match replacement {
                Some(loopback) => format!("{loopback}:{port}"),
                None => bound.to_string(),
            }
        }
        None => bound.to_string(),
    }
}

/// Percent-encode `raw` for the URL fragment [`pane_url`] builds.
///
/// Exactly RFC 3986's unreserved set — `A-Za-z0-9-._~` — and everything else
/// escaped. The fragment is a `&`-separated list of `key=value` pairs read by
/// [`PANE_BOOT_JS`], so `&`, `=` and `%` are the characters that must not
/// survive a value; escaping the whole of the rest is simply the rule that has
/// no edge cases.
///
/// **The underscore is no longer special, and that is a change worth naming.**
/// It used to be escaped as `%5F` for a routing reason rather than a safety one:
/// the page read an underscore in the fragment as a rendezvous join code, so a
/// participant named `ada_lovelace` silently changed the route. Since #1112
/// *any* non-empty fragment is that route, escaping one character could not
/// avoid it, and the pane now takes the route deliberately — see the module
/// note's "Why the fragment also carries a join code". The identity is consumed
/// and the fragment rewritten before the page reads it, so nothing a
/// participant can be called reaches the route decision at all.
pub fn fragment_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The join code a pane's fragment leads with.
///
/// A suffix of the authored length in `assets/join/join-codes.toml`'s alphabet
/// (`ABCDEFGHIJKMNOPQRSTUVWXYZ`) that is not on its deny list, so
/// `gui/join-code.js`'s `parseJoinCode` composes it into a full identifier and
/// `client.html`'s `startPhoenixJoin` takes its ordinary rendezvous route
/// instead of opening the join-entry overlay. `tests/client/pane-scripts.test.js`
/// checks this literal against that authored table, because a value only Rust
/// knows and only JavaScript validates is a value nothing checks.
///
/// It resolves to nothing and is meant to: the pane's own socket stand-in
/// answers `joined` to whatever it is handed, so this is a sentinel that gets
/// the page onto its join route, not a code any service has heard of.
pub const PANE_JOIN_CODE: &str = "PANESEAT";

/// The URL a pane's view navigates to — **including its identity**.
///
/// The fragment does two jobs. It carries the session token and the participant
/// name, which is the whole of how a pane's page learns who it is: a fragment is
/// never sent to the server and never appears in a served byte, so nothing on
/// the LAN can read it out of the document the way it could when this was an
/// injected `window.__phoenixPaneIdentity`. See the module note.
///
/// And it is what puts the page on its join route at all. `joinRouteFromLocation`
/// reads an empty fragment as "nothing to join" — the page prints a status line
/// and stops — and any non-empty one as a code to resolve, which
/// `parseJoinCode` then has to accept or the join-entry overlay covers the
/// console. [`PANE_JOIN_CODE`] leads the fragment for exactly that reason, and
/// [`PANE_BOOT_JS`] leaves only it behind once it has taken the identity out.
pub fn pane_url(host_addr: &str, id: PaneId, nonce: &str, identity: &PaneIdentity) -> String {
    let mut url = format!(
        "http://{host_addr}{}#{PANE_JOIN_CODE}&token={}&name={}",
        pane_document_path(id, nonce),
        fragment_encode(identity.token()),
        fragment_encode(identity.name()),
    );
    if let Some(station) = identity.assigned_station() {
        url.push_str("&station=");
        url.push_str(&fragment_encode(station));
    }
    url
}

/// What a pane document drops: any `<script>` element that loads PeerJS.
///
/// Matched on `peerjs` anywhere in the opening tag, case-insensitively, rather
/// than on a CDN host — the previous marker was the literal `unpkg.com/peerjs`,
/// which a move to another CDN or a self-hosted `gui/peerjs.min.js` would have
/// turned into a silent no-op.
///
/// Issue #1112 deletes the tag from `client.html` outright, at which point this
/// strips nothing. That is a correct outcome rather than a silent one:
/// `a_pane_document_of_the_repositorys_own_client_page_carries_no_peerjs_script`
/// asserts that **zero** PeerJS script elements survive, which is true whether
/// one was removed here or never existed.
const PEERJS_SCRIPT_MARKER: &str = "peerjs";

/// Build a pane's document from the client bundle's own `index.html`.
///
/// Pure: bytes in, bytes out, and **identity-free**. The whole of what makes a
/// pane document different from the page a phone loads is decided here, so it is
/// decided somewhere a unit test can read without an SDK, a GPU or an HTTP
/// server.
///
/// It takes no [`PaneIdentity`] because nothing about who a pane is may appear
/// in a body this host serves over an unauthenticated LAN listener — that moved
/// to [`pane_url`]'s fragment. Two panes' documents are therefore byte-identical;
/// they are still published one per pane, at one path each, because a path is
/// what gets withdrawn when a pane closes.
pub fn build_pane_document(client_index_html: &str) -> Result<String, DocumentError> {
    build_pane_document_with(client_index_html, &PaneDocumentOptions::default())
}

/// Host-side knobs a pane document is built with.
///
/// The default is exactly the document [`build_pane_document`] builds; every
/// field is a diagnostic the host may set for one run, never something a page
/// could ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneDocumentOptions {
    /// Run the page's render loop at this interval instead of `pane_boot.js`'s
    /// own 16 ms — the `raf33` frame experiment
    /// (`super::frame_stats::PaneExperiments`). Injected as
    /// `window.PhoenixPaneFrameMs` ahead of the boot script, which reads it
    /// once when it installs the timer.
    pub frame_ms: Option<u32>,
}

/// [`build_pane_document`] with [`PaneDocumentOptions`].
pub fn build_pane_document_with(
    client_index_html: &str,
    options: &PaneDocumentOptions,
) -> Result<String, DocumentError> {
    let head_end = find_tag_end(client_index_html, "<head").ok_or(DocumentError::NoHead)?;
    let frame_ms = match options.frame_ms {
        Some(ms) => format!("window.PhoenixPaneFrameMs = {ms};\n"),
        None => String::new(),
    };
    let boot = format!(
        "\n<script>\n{}{}\n{}\n{}\n{}</script>\n",
        frame_ms,
        queue_shim(PANE_OUT_NAMESPACE),
        OPERATOR_STORAGE_JS,
        include_str!("../audio/private_boot.js"),
        PANE_BOOT_JS,
    );
    let mut html = String::with_capacity(client_index_html.len() + boot.len() + PANE_LINK_JS.len());
    html.push_str(&client_index_html[..head_end]);
    html.push_str(&boot);
    html.push_str(&client_index_html[head_end..]);

    let html = strip_elements_matching(&html, "script", Some(PEERJS_SCRIPT_MARKER));
    // Legacy bundles may call media.play() before sending a command. Keep the
    // media strip while current bundles use the injected private audio owner.
    let html = strip_elements_matching(&html, "audio", None);

    let body_close = html.rfind("</body>").ok_or(DocumentError::NoBody)?;
    let mut out = String::with_capacity(html.len() + PANE_LINK_JS.len() + 64);
    out.push_str(&html[..body_close]);
    out.push_str("\n<script type=\"module\">\n");
    out.push_str(PANE_LINK_JS);
    out.push_str("\n</script>\n");
    out.push_str(&html[body_close..]);
    Ok(out)
}

/// Seed a pane document's OS accessibility default layer (issue #1127).
///
/// A browser reads the machine's accessibility preferences through `matchMedia`;
/// an Ultralight pane has no OS-backed `matchMedia`, so the host reads them
/// natively and injects them here as a `window.PhoenixOsAccessibilityDefaults`
/// assignment ([`super::os_prefs::os_defaults_script`]) that
/// `gui/accessibility-profile.js`'s `osAccessibilityDefaults` overlays.
///
/// The script goes in a classic `<script>` right after `<head>`, so it runs
/// before any `gui/` module evaluates — the profile then initialises from the OS
/// exactly as a browser's does. An explicit player choice still overrides it,
/// and the profile itself stays client-local; nothing here rides the transport
/// seam (see the module note on `os_prefs`).
///
/// Takes an already-built pane document. Idempotent in the sense that matters:
/// the assignment simply re-runs if injected twice, so a caller need not track
/// whether it has run. Returns the input unchanged if there is no `<head>` — the
/// same "nowhere to inject" a browser tolerates by falling back to no OS layer,
/// rather than an error that would fail a pane over a default it could live
/// without.
pub fn inject_os_accessibility_defaults(
    document: &str,
    prefs: &super::os_prefs::OsAccessibilityPrefs,
) -> String {
    inject_head_script(document, &super::os_prefs::os_defaults_script(prefs))
}

/// Put `body` in a classic `<script>` immediately after `<head>`.
///
/// The mechanics of the injection above, named separately once a SECOND seed
/// needed them (issue #1427's saved viewscreen presentation): both are one
/// assignment to a documented `window.Phoenix…` global that a `gui/` module
/// reads, and both must run before any module evaluates.
///
/// The safety rule travels with it and is the caller's: `body` is interpolated
/// verbatim into an HTML `<script>`, so every value inside it must already be a
/// bool, a finite number or a JSON-escaped string. Each caller's own function
/// states that invariant over the string it builds
/// ([`super::os_prefs::os_defaults_script`],
/// [`crate::native_host::viewscreen_presentation::presentation_script`]).
///
/// A document with no `<head>` is returned unchanged — the same "nowhere to
/// inject" a browser tolerates by falling back to no injected layer, rather than
/// an error that would fail a surface over a default it can live without.
pub fn inject_head_script(document: &str, body: &str) -> String {
    let Some(head_end) = find_tag_end(document, "<head") else {
        return document.to_string();
    };
    let script = format!("\n<script>\n{body}\n</script>\n");
    let mut out = String::with_capacity(document.len() + script.len());
    out.push_str(&document[..head_end]);
    out.push_str(&script);
    out.push_str(&document[head_end..]);
    out
}

/// Byte index just past the first `<head…>` tag, or `None`.
fn find_tag_end(html: &str, open: &str) -> Option<usize> {
    let start = html.find(open)?;
    let close = html[start..].find('>')?;
    Some(start + close + 1)
}

/// Remove every `<tag …>…</tag>` element whose opening tag contains `marker`,
/// compared case-insensitively. `None` removes every one of them.
///
/// Deliberately narrow — it matches an opening `<tag`, finds its `>`, and
/// requires the matching `</tag>` — because the alternative is a general HTML
/// parser for two elements this repository authors itself.
///
/// `pub(crate)` because [`super::super::host_lobby::document`] borrowed it for
/// the AI-launch `<button>` in issue #1325 — which #1328 stopped stripping, the
/// force-start path having ceased to be wasm-only. That module now removes only
/// NESTED elements (the picker's two host-tooling blocks), which this matcher
/// cannot do: it takes the FIRST closing tag after the opening one, so it would
/// truncate a `<div>` containing `<div>`s. It has its own depth-counted
/// `remove_element` instead, and this one stays exactly as narrow as the two
/// flat elements it was written for.
pub(crate) fn strip_elements_matching(html: &str, tag: &str, marker: Option<&str>) -> String {
    let open_tag = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(&open_tag) {
        let Some(open_end) = rest[start..].find('>').map(|i| start + i + 1) else {
            break;
        };
        let Some(close) = rest[open_end..]
            .find(&close_tag)
            .map(|i| open_end + i + close_tag.len())
        else {
            break;
        };
        let matched = match marker {
            Some(marker) => rest[start..open_end].to_ascii_lowercase().contains(marker),
            None => true,
        };
        if matched {
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
#[path = "document_tests.rs"]
mod tests;
