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
//! # Why the `<audio>` element has to go
//!
//! Ultralight ships no media backend: `HTMLMediaElement.play` is simply
//! **undefined**, and calling it throws `TypeError: el.play is not a function`.
//!
//! That would be cosmetic if the page played its UI click *after* sending. It
//! plays it *before* — `client.html`'s `send()` calls `playClick()` and then
//! `link.send(...)` — so the exception propagates out of the `console_action`
//! message listener and the command never reaches the transport at all. The
//! page's `NO_CLICK` set exempts the high-rate ones (`SetThrust`, `SetSteering`,
//! `Identify`, `SetName`), which is exactly why this is so easy to miss: a pane
//! joins, renames itself, flies with the joystick, and every deliberate console
//! command — every `ControlSystem` — is silently dropped, with a clean log on
//! both sides.
//!
//! Removing the element makes `playClick`'s own first line
//! (`const el = document.getElementById('ui-click'); if (!el) return;`) the path
//! it takes, which is the behaviour of a page whose audio has been trimmed —
//! something `gui/settings-panel.js` already tolerates (`audioEls` is
//! `.filter(Boolean)`ed). Fixing it in `client.html` instead would mean changing
//! the page a phone loads to suit a pane, which is the thing this module exists
//! not to do.
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
    format!(
        "http://{host_addr}{}#{PANE_JOIN_CODE}&token={}&name={}",
        pane_document_path(id, nonce),
        fragment_encode(identity.token()),
        fragment_encode(identity.name()),
    )
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
    let head_end = find_tag_end(client_index_html, "<head").ok_or(DocumentError::NoHead)?;
    let boot = format!(
        "\n<script>\n{}\n{}</script>\n",
        queue_shim(PANE_OUT_NAMESPACE),
        PANE_BOOT_JS,
    );
    let mut html = String::with_capacity(client_index_html.len() + boot.len() + PANE_LINK_JS.len());
    html.push_str(&client_index_html[..head_end]);
    html.push_str(&boot);
    html.push_str(&client_index_html[head_end..]);

    let html = strip_elements_matching(&html, "script", Some(PEERJS_SCRIPT_MARKER));
    // Every `<audio>` element, unconditionally: Ultralight has no media
    // backend, and the page calls `play()` BEFORE it sends. See the module note
    // — this is the difference between a pane that works and a pane that joins
    // and then silently drops every console command.
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
    let Some(head_end) = find_tag_end(document, "<head") else {
        return document.to_string();
    };
    let script = format!(
        "\n<script>\n{}\n</script>\n",
        super::os_prefs::os_defaults_script(prefs),
    );
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
mod tests {
    use super::*;

    /// The shape of the real page, reduced to the four things the assembly
    /// depends on: a `<head>`, the PeerJS tag, the click-sound `<audio>`, and a
    /// `</body>`.
    ///
    /// The *real* page is asserted against too — see
    /// [`a_pane_document_assembles_from_the_repositorys_own_client_page`], which
    /// reads `client.html` off disk. This stub is for the cases that need a
    /// document shaped a particular way (no `<head>`, no `</body>`) and for
    /// asserting that nothing but those two elements is disturbed.
    const CLIENT: &str = "<!doctype html>\n<html>\n<head>\n\
         <script src=\"gui/bg-raf-keepalive.js\"></script>\n\
         <script src=\"https://unpkg.com/peerjs@1.5.4/dist/peerjs.min.js\"></script>\n\
         <script type=\"module\" src=\"gui/connection-manager.js\"></script>\n\
         <script type=\"module\" src=\"gui/rendezvous-transport.js\"></script>\n\
         </head>\n<body>\n<div id=\"app\"></div>\n\
         <audio id=\"ui-click\" src=\"assets/sounds/ui_click.ogg\"></audio>\n\
         <script>var myName = 'x';</script>\n\
         </body>\n</html>\n";

    /// The link script's own import, used as the marker for "the link is here".
    /// The relative path only resolves because a pane document is published at
    /// the client directory's own depth, which is the whole reason for it.
    const PANE_LINK_IMPORT: &str = "from './gui/rendezvous-transport.js'";

    fn identity() -> PaneIdentity {
        PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap()
    }

    /// How many `<script>` elements in `html` load PeerJS.
    fn peerjs_script_tags(html: &str) -> usize {
        html.match_indices("<script")
            .filter(|(start, _)| {
                html[*start..]
                    .find('>')
                    .map(|end| {
                        html[*start..*start + end]
                            .to_ascii_lowercase()
                            .contains("peerjs")
                    })
                    .unwrap_or(false)
            })
            .count()
    }

    #[test]
    fn the_boot_script_runs_before_the_pages_own_scripts() {
        // Two reasons, both load-bearing. It reads the session token and the
        // player name out of the fragment, which the page's inline script wants
        // at parse time — one script too late and the pane joins under a random
        // name on a token nothing else knows. And it replaces
        // `requestAnimationFrame`, which `gui/bg-raf-keepalive.js` — the very
        // first script the page loads — captures once and then delegates to.
        let html = build_pane_document(CLIENT).unwrap();
        let boot = html.find("__phoenixPane").unwrap();
        let raf = html.find("window.requestAnimationFrame =").unwrap();
        // The TAG, not the name: the boot script's own comment says why it has
        // to precede that module, so a bare name matches inside the prose.
        let first_page_script = html.find("src=\"gui/bg-raf-keepalive.js\"").unwrap();
        assert!(
            boot < first_page_script,
            "the boot script must be the first script in the document"
        );
        assert!(
            raf < first_page_script,
            "bg-raf-keepalive.js captures whatever rAF it finds; a pane's has to be there first"
        );
        assert!(html.find("<head>").unwrap() < boot);
    }

    #[test]
    fn the_link_script_runs_after_the_pages_transport_module() {
        // Module scripts evaluate in document order, and the link IMPORTS the
        // channel labels from gui/rendezvous-transport.js — the same module
        // instance the page loads, which has to be in the graph first.
        let html = build_pane_document(CLIENT).unwrap();
        let transport = html.find("src=\"gui/rendezvous-transport.js\"").unwrap();
        let link = html.find(PANE_LINK_IMPORT).unwrap();
        assert!(transport < link);
        assert!(link < html.rfind("</body>").unwrap());
    }

    #[test]
    fn the_link_script_installs_the_transport_factories_and_nothing_else() {
        // The seam it attaches through, named here so a rewrite that quietly
        // went back to publishing a global of its own fails. `currentLink()`
        // reads a closure `startPhoenixJoin` assigns, so a pane-owned link
        // object is unreachable without editing client.html; the factories are
        // the override the transport module documents for exactly this.
        let html = build_pane_document(CLIENT).unwrap();
        assert!(html.contains("window.PhoenixTransportFactories ="));
        assert!(
            !html.contains("window.connectionManager ="),
            "that global went with PeerJS in #1112; assigning it reaches nothing"
        );
    }

    #[test]
    fn the_peerjs_tag_is_dropped_and_nothing_else_is() {
        // A pane never uses PeerJS, and a bridge machine may not be able to
        // reach a CDN at all — waiting for that fetch to time out is boot
        // latency for nothing.
        let html = build_pane_document(CLIENT).unwrap();
        assert_eq!(peerjs_script_tags(&html), 0);
        assert!(html.contains("gui/bg-raf-keepalive.js"));
        assert!(html.contains("gui/connection-manager.js"));
        assert!(html.contains("var myName = 'x';"));
        assert!(html.contains("<div id=\"app\"></div>"));
    }

    #[test]
    fn the_click_sound_is_dropped_because_playing_it_would_eat_the_command() {
        // Not a nicety. Ultralight has no media backend — `el.play` is
        // undefined — and `client.html`'s `send()` calls `playClick()` BEFORE
        // `link.send(...)`. The TypeError propagates out of the
        // `console_action` listener and the command never reaches the
        // transport. `NO_CLICK` exempts SetThrust/SetSteering/Identify/SetName,
        // so a pane joins, renames itself and flies — and every deliberate
        // console command is silently dropped.
        //
        // With no element, `playClick`'s own `if (!el) return` is the path.
        let html = build_pane_document(CLIENT).unwrap();
        assert!(!html.contains("<audio"));
        assert!(!html.contains("ui_click.ogg"));
        assert!(html.contains("<div id=\"app\"></div>"));
        assert!(html.contains("var myName = 'x';"));
    }

    #[test]
    fn a_peerjs_tag_from_anywhere_but_unpkg_is_dropped_too() {
        // The marker used to be the literal CDN host, so self-hosting the
        // library or moving CDN would have made this strip a silent no-op.
        let page = "<html><head>\
             <script SRC=\"gui/vendor/PeerJS.min.js\"></script>\
             </head><body></body></html>";
        assert_eq!(peerjs_script_tags(&build_pane_document(page).unwrap()), 0);
    }

    #[test]
    fn a_panes_identity_never_appears_in_the_document_the_host_serves() {
        // The finding this whole arrangement answers: `phoenix-host` binds
        // 0.0.0.0 with no TLS and no auth, so a session token in a served body
        // is a seat on the bridge available to anyone on the LAN. It is in the
        // URL FRAGMENT, which a browser never transmits and this host never
        // writes.
        //
        // The name is deliberately hostile on two axes at once: `</script>`
        // would close the inline script element it used to be interpolated
        // into (no JavaScript string escape prevents that — an HTML script
        // element ends at the first `</script` regardless of quoting), and the
        // apostrophe would close the literal itself.
        let awkward = PaneIdentity::adopt("tok'en</script>", "</script><h1>O'Neil_x").unwrap();
        let html = build_pane_document(CLIENT).unwrap();
        assert!(!html.contains(awkward.token()));
        assert!(!html.contains(awkward.name()));
        assert!(!html.contains("<h1>"));
        assert!(
            !html.contains("__phoenixPaneIdentity"),
            "no identity is embedded at all, so there is nothing to escape"
        );

        // And it does reach the page — by the one route that costs nothing on
        // the wire.
        let url = pane_url("127.0.0.1:8080", PaneId(0), "n0nce", &awkward);
        let fragment = url.split_once('#').unwrap().1;
        assert!(fragment.contains(&fragment_encode(awkward.token())));
        assert!(fragment.contains(&fragment_encode(awkward.name())));
        assert!(!fragment.contains('<'));
        // The fragment is a `&`-separated list of `key=value` pairs, so a value
        // that carried either separator would swallow the field after it — or,
        // in the first field's case, look like the join code.
        assert_eq!(
            fragment.split('&').count(),
            3,
            "one code and two pairs, whatever the participant is called: {fragment}"
        );
        assert!(fragment.starts_with(&format!("{PANE_JOIN_CODE}&")));
    }

    #[test]
    fn the_page_to_host_queue_is_installed_under_the_namespace_the_host_drains() {
        let html = build_pane_document(CLIENT).unwrap();
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
            build_pane_document("<html><body></body></html>"),
            Err(DocumentError::NoHead)
        );
        assert_eq!(
            build_pane_document("<html><head></head></html>"),
            Err(DocumentError::NoBody)
        );
    }

    #[test]
    fn a_panes_document_sits_at_the_client_directorys_own_depth() {
        // This is what makes every relative URL in the page — gui modules,
        // stylesheets, console iframes — resolve exactly as they do for a
        // phone, with no <base> tag and no rewriting.
        assert_eq!(
            pane_document_path(PaneId(0), "abcd"),
            "/client/pane-0-abcd.html"
        );
        assert_eq!(
            pane_url("127.0.0.1:8080", PaneId(3), "abcd", &identity()),
            "http://127.0.0.1:8080/client/pane-3-abcd.html\
             #PANESEAT&token=3f1a6c2e-0a11-4b3c-9d55-000000000001&name=Ada"
        );
    }

    #[test]
    fn two_panes_document_paths_cannot_be_guessed_from_each_other() {
        // `/client/pane-0.html` was one guess away from a live participant's
        // page. The nonce is per pane, so knowing one path tells you nothing
        // about the next.
        let a = mint_document_nonce();
        let b = mint_document_nonce();
        assert_ne!(a, b);
        assert!(a.len() >= 32, "a guessable nonce is not a nonce: {a}");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(
            pane_document_path(PaneId(0), &a),
            pane_document_path(PaneId(0), &b)
        );
    }

    #[test]
    fn the_fragment_leads_with_a_join_code_the_page_can_actually_resolve() {
        // The two ways this page load can fail before a pane ever speaks:
        // an EMPTY fragment is `route: 'entry'`, which shows the join field and
        // stops; a fragment `parseJoinCode` refuses is `route: 'rendezvous'`
        // and then the same field with a reason on it. The code has to come
        // first, on its own, with no `=` in it — that is how PANE_BOOT_JS tells
        // it apart from the identity pairs, and it is what gets left in
        // `location.hash` for the page to read.
        let url = pane_url("127.0.0.1:8080", PaneId(0), "abcd", &identity());
        let fragment = url.split('#').nth(1).unwrap();
        assert!(!fragment.is_empty());
        let code = fragment.split('&').next().unwrap();
        assert_eq!(code, PANE_JOIN_CODE);
        assert!(!code.contains('='));
    }

    #[test]
    fn the_join_code_is_one_the_authored_table_accepts() {
        // The Rust half of a claim `tests/client/pane-scripts.test.js` makes
        // against the real table: the authored length, all in the authored
        // alphabet,
        // none of them the confusables the canonicaliser rewrites. A code that
        // did not round-trip would compose into an identifier the page then
        // refuses, and the console would come up behind the join overlay.
        const ALPHABET: &str = "ABCDEFGHIJKMNOPQRSTUVWXYZ";
        assert_eq!(PANE_JOIN_CODE.len(), 8);
        assert!(
            PANE_JOIN_CODE.chars().all(|c| ALPHABET.contains(c)),
            "{PANE_JOIN_CODE} is not spelled in assets/join/join-codes.toml's alphabet"
        );
    }

    #[test]
    fn a_wildcard_bind_becomes_a_loopback_url_because_nothing_can_dial_0_0_0_0() {
        // `--addr 0.0.0.0:8080` is the DOCUMENTED default, so this is the
        // ordinary invocation rather than an edge case: 0.0.0.0 means "every
        // interface" to `bind` and nothing at all to `connect`. A pane always
        // connects to its own process.
        assert_eq!(connectable_host_addr("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(connectable_host_addr("[::]:8080"), "[::1]:8080");
        assert_eq!(connectable_host_addr(":::8080"), "[::1]:8080");
        // The port a `:0` bind was actually given is kept, which is the whole
        // point of taking the address from `local_addr()` rather than `--addr`.
        assert_eq!(connectable_host_addr("0.0.0.0:52341"), "127.0.0.1:52341");
        // A specific bind is left exactly alone.
        assert_eq!(
            connectable_host_addr("192.168.1.5:8080"),
            "192.168.1.5:8080"
        );
        assert_eq!(connectable_host_addr("127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(connectable_host_addr("[::1]:8080"), "[::1]:8080");
    }

    #[test]
    fn the_fragment_encoding_escapes_everything_that_is_not_unreserved() {
        // The three that matter are the fragment's own grammar — `&` separates
        // fields, `=` separates a key from its value, `%` is the escape — and
        // escaping the whole of the rest is the rule with no edge cases.
        assert_eq!(fragment_encode("a&b=c%d"), "a%26b%3Dc%25d");
        assert_eq!(fragment_encode("O'Neil"), "O%27Neil");
        assert_eq!(fragment_encode("a b"), "a%20b");
        assert_eq!(fragment_encode("</script>"), "%3C%2Fscript%3E");
        // The whole unreserved set survives, the underscore now included: it
        // was escaped for a routing reason that no longer exists (see
        // `fragment_encode`), and `PANE_BOOT_JS` strips the identity out of the
        // fragment before the page's route is decided at all.
        assert_eq!(fragment_encode("ada_lovelace"), "ada_lovelace");
        assert_eq!(fragment_encode("Ada-1.0~x_y"), "Ada-1.0~x_y");
        // Non-ASCII goes out as UTF-8 bytes, which decodeURIComponent restores.
        assert_eq!(fragment_encode("é"), "%C3%A9");
    }

    #[test]
    fn a_pane_document_assembles_from_the_repositorys_own_client_page() {
        // Every other test here runs against a ten-line stub, so nothing
        // checked the assembly against the page it actually operates on. The
        // repository's `client.html` is checked in, so this needs no build
        // step — and `scripts/build-client.mjs` copies it to
        // `dist/client/index.html` without touching its script tags.
        let client = std::fs::read_to_string("client.html")
            .expect("the repository's own client page is checked in");
        let html = build_pane_document(&client).expect("the real page becomes a pane document");
        // The strips run over the real page, and both look for an opening tag
        // and then its closing one. A page that put `<audio` or `<script` in a
        // comment or a string literal could make one of them swallow the span
        // between — so bound the damage before asserting anything finer.
        assert!(
            html.len() > client.len(),
            "the two injected scripts are far bigger than the two stripped elements; a \
             shorter document means a strip ran away"
        );

        let boot = html
            .find("__phoenixPane")
            .expect("the boot script is injected");
        let first_gui_script = html
            .find("src=\"gui/")
            .expect("the real page loads gui/ modules");
        assert!(
            boot < first_gui_script,
            "the boot script must precede every gui/ module the page loads"
        );

        let transport = html
            .find("src=\"gui/rendezvous-transport.js\"")
            .expect("the real page loads the crew transport");
        let link = html
            .find(PANE_LINK_IMPORT)
            .expect("the link script is injected");
        assert!(
            transport < link,
            "the link imports the transport's channel labels; it must not run first"
        );
        assert!(link < html.rfind("</body>").unwrap());
    }

    #[test]
    fn a_pane_document_of_the_repositorys_own_client_page_carries_no_peerjs_script() {
        // Belt and braces, and honest in both states: today `client.html`
        // carries the CDN tag and this strips it; issue #1112 deletes the tag
        // outright, at which point this strips nothing and the assertion is
        // still exactly the claim being made — a pane never loads PeerJS.
        let client = std::fs::read_to_string("client.html").unwrap();
        let html = build_pane_document(&client).unwrap();
        assert_eq!(
            peerjs_script_tags(&html),
            0,
            "no <script> element in a pane document may load PeerJS"
        );
        // The page's own prose and its inline fallback still MENTION PeerJS,
        // and must: this strips a script element, it does not rewrite the
        // client.
        assert!(html.contains("gui/connection-manager.js"));
    }

    #[test]
    fn the_repositorys_own_client_page_has_the_audio_element_this_strips() {
        // The strip above is only worth anything if it is aimed at something
        // real. If `client.html` ever loses its `<audio>` element this fails
        // loudly rather than leaving a no-op behind — and if it gains another
        // one, the pane document must lose that too.
        let client = std::fs::read_to_string("client.html").unwrap();
        assert!(
            client.contains("<audio"),
            "the page a phone loads has a UI click sound; if that changed, so did the \
             reason for this strip"
        );
        assert!(!build_pane_document(&client).unwrap().contains("<audio"));
    }

    #[test]
    fn a_native_pane_declares_profile_capabilities_before_the_shared_adapter() {
        // #1280 does not create a native profile, input sampler or
        // Accessibility apply path. The first injected script declares only
        // capability facts; the repository's ordinary profile and surface
        // adapter then consume them. Keyboard still arrives through #1124 and
        // OS defaults are still injected separately by #1127 below.
        let client = std::fs::read_to_string("client.html").unwrap();
        let html = build_pane_document(&client).unwrap();
        let declaration = html
            .find("window.PhoenixOperatorCapabilities =")
            .expect("pane boot declares its operator capabilities");
        let profile = html
            .find("src=\"gui/operator-profile.js\"")
            .expect("the ordinary versioned profile remains the schema owner");
        let adapter = html
            .find("src=\"gui/operator-surface-adapter.js\"")
            .expect("the shared surface adapter remains in the pane document");
        assert!(declaration < profile);
        assert!(profile < adapter);
        assert!(html.contains("surface: 'native-pane'"));
        // The native pane now DOES support a gamepad: the host feeds one from
        // gilrs and the boot script installs a `navigator.getGamepads()` shim, so
        // the ordinary client runtime samples it. Vibration still has no native
        // backend and stays off.
        assert!(html.contains("gamepad: true"));
        assert!(
            html.contains("navigator.getGamepads"),
            "the native pane installs the host-fed gamepad shim so the runtime can sample it"
        );
        assert!(html.contains("vibration: false"));
        assert_eq!(
            html.matches("src=\"gui/operator-profile.js\"").count(),
            1,
            "a pane must not gain a competing native profile"
        );
    }

    // ── OS accessibility default injection (issue #1127) ─────────────────────

    #[test]
    fn os_accessibility_defaults_are_injected_before_the_pages_own_scripts() {
        use super::super::os_prefs::OsAccessibilityPrefs;
        // The page reads them at resolve time through `osAccessibilityDefaults`,
        // which runs inside the boot the first `gui/` module drives — so the
        // assignment has to be in <head>, before any gui/ script.
        let html = build_pane_document(CLIENT).unwrap();
        let seeded = inject_os_accessibility_defaults(
            &html,
            &OsAccessibilityPrefs {
                reduced_motion: true,
                high_contrast: true,
                text_scale: 1.25,
            },
        );
        let assign = seeded
            .find("window.PhoenixOsAccessibilityDefaults =")
            .expect("the OS default layer is injected");
        let first_gui = seeded
            .find("src=\"gui/")
            .expect("the page still loads its gui/ modules");
        assert!(
            assign < first_gui,
            "the OS defaults must be seeded before any gui/ module reads them"
        );
        assert!(seeded.find("<head>").unwrap() < assign);
        // The exact values the page overlays: reducedMotion + contrast (from
        // high-contrast) true, and the imported text scale.
        assert!(seeded.contains("\"reducedMotion\":true"));
        assert!(seeded.contains("\"contrast\":true"));
        assert!(seeded.contains("\"textScale\":1.25"));
        // Injection adds only the one script; nothing the page owns is lost.
        assert!(seeded.contains("<div id=\"app\"></div>"));
        assert!(seeded.contains("var myName = 'x';"));
    }

    #[test]
    fn os_accessibility_defaults_of_a_quiet_machine_match_a_silent_browser() {
        use super::super::os_prefs::OsAccessibilityPrefs;
        // A machine with nothing set injects the SAME default layer a browser
        // computes from a silent matchMedia, so a pane and a phone on that
        // machine start from the identical baseline (AC2/AC5 equivalence).
        let seeded = inject_os_accessibility_defaults(CLIENT, &OsAccessibilityPrefs::default());
        assert!(seeded.contains("\"reducedMotion\":false"));
        assert!(seeded.contains("\"contrast\":false"));
        assert!(seeded.contains("\"textScale\":1"));
    }

    #[test]
    fn a_document_with_no_head_keeps_its_os_layer_absent_rather_than_failing() {
        use super::super::os_prefs::OsAccessibilityPrefs;
        // No <head> means no OS layer, which is exactly a browser with no
        // matchMedia — the page falls back to "no preference", not an error.
        let no_head = "<html><body></body></html>";
        assert_eq!(
            inject_os_accessibility_defaults(no_head, &OsAccessibilityPrefs::default()),
            no_head
        );
    }

    #[test]
    fn the_injected_os_layer_never_carries_a_setting_the_seam_could_leak() {
        use super::super::os_prefs::OsAccessibilityPrefs;
        // AC4 at the document: the injected layer is machine OS defaults, not a
        // player's private profile. It names the three matchMedia-shaped keys
        // and nothing resembling a stored setting, diagnosis or assistance
        // request — the page keeps the profile client-local and only the
        // anonymous ineligible-station set ever crosses the seam.
        let seeded = inject_os_accessibility_defaults(
            CLIENT,
            &OsAccessibilityPrefs {
                reduced_motion: true,
                high_contrast: true,
                text_scale: 1.5,
            },
        );
        let line = seeded
            .lines()
            .find(|l| l.contains("PhoenixOsAccessibilityDefaults"))
            .unwrap();
        for forbidden in ["assistance", "diagnos", "presentation", "request", "token"] {
            assert!(
                !line.contains(forbidden),
                "the OS default layer must not carry `{forbidden}`: {line}"
            );
        }
    }
}
