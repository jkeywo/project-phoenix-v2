//! What the native host's lobby surface loads, and where it loads it from
//! (issue #1325).
//!
//! # The same choice [`panes::document`](crate::native_host::panes::document)
//! made, for the same reasons
//!
//! That module's four options and its verdict apply here unchanged: a document
//! `load_html`'d from a string has **no base URL and no origin**, so every
//! relative `<script src>`, stylesheet and `fetch` in it fails — `fetch`
//! silently — and a `<base href>` fixes the first without fixing the second. So
//! the lobby document is **assembled in memory and published by the delivery
//! server this process is already running**, at the depth the page it borrows
//! from sits at, and loaded over `http://<host>/…`.
//!
//! The depth here is the **served root**, not `/client/`, and that is the whole
//! trick rather than an incidental: the markup and the modules come from the
//! HOST page's bundle (`dist/index.html`, `dist/gui/…`, `dist/assets/strings/…`
//! — trunk's `copy-dir` output), so a document at `/host-lobby-<nonce>.html`
//! resolves `gui/host-lobby-render.js` and `../assets/strings/strings.csv`
//! exactly as `index.html` does. A phone's console pages live one directory
//! down, at `/client/`, which is why the pane document is published there and
//! this one is not.
//!
//! # Why the markup is sliced out of the served page instead of authored here
//!
//! The lobby is thirteen element ids in a particular nest — `#lobby-panel`,
//! `#lobby-crew-dots`, `#station-grid`, `#reserved-aggregate` and the rest —
//! and `gui/host-lobby-render.js` writes into every one of them. A hand-written
//! copy of that nest in this file would be a second lobby markup: it would
//! start identical, and the first time somebody added a row to the web lobby
//! the native one would render it nowhere, with nothing failing. So
//! [`build_host_lobby_document`] takes the host page's own bytes and lifts
//! `#lobby-panel` out of them, the way [`build_pane_document`] takes the client
//! page's own bytes and injects into them.
//!
//! [`build_pane_document`]: crate::native_host::panes::document::build_pane_document
//!
//! What the assembled document adds around that fragment is only what a
//! fragment cannot carry: a `<head>` naming the three stylesheets the web host
//! links (`gui/tokens.css`, `gui/host-lobby.css`, `gui/host-qr.css` — the last
//! two exist *because* of this surface), a ground colour, the vendored QR
//! encoder, the bridge scripts, and the module island that wires the shared
//! view model to the shared renderer.
//!
//! It carries no `<title>`, deliberately: an embedded view has no tab bar, so a
//! title would be player-visible English nothing ever shows — and every string a
//! player CAN see on this surface comes through `gui/strings.js` from
//! `assets/strings/strings.csv`, like the web lobby's (AGENTS.md rule 11).
//!
//! It also carries no Google Fonts `<link>`, which `server.html` does have. A
//! bridge machine is not assumed to have internet, and a render-blocking
//! stylesheet on a host that does not would stall the surface's first paint for
//! however long the DNS lookup takes. The shared sheet's faces are all declared
//! with a fallback stack (`'Chakra Petch', system-ui, monospace`), so the cost
//! is the substitute face rather than an unstyled lobby.
//!
//! # What the document adds and takes away
//!
//! | edit | why |
//! |---|---|
//! | the AI-launch `<button>` removed | this surface is READ-ONLY in this slice — selection stays on the CLI — and a control that silently does nothing is worse than no control |
//! | the page's `#qr-panel` carried too, in an `#overlay` of this document's own (issue #1329) | the crew have to be shown how to JOIN the lobby they are looking at. The panel is the page's own markup for the same reason the lobby is; the overlay around it is not, because the page's also carries the fleet panel and the diagnostics readout, and neither has anything to say on a viewscreen |
//! | `#host-lobby-qr-toggle` added (issue #1329) | a host page toggles the QR from its settings cog; this window has no cog, so the one decision that surface genuinely needs gets the one control it needs |
//!
//! Everything else the document does to the markup it does by *omission*: it
//! leaves `--settings-cog-keepout` undefined, which selects the `0px` fallback
//! `gui/host-lobby.css` names, because there is no settings cog on the
//! viewscreen window to reserve a corner for.
//!
//! # No identity, and therefore nothing to leak
//!
//! A pane document is careful never to carry a session token, because
//! `phoenix-host` binds `0.0.0.0` with no TLS: see [`panes::document`]'s note on
//! the URL fragment. This document has the same property for a simpler reason —
//! **there is no identity to carry**. The lobby surface is not a participant: it
//! holds no session token, sends no `Identify`, claims no station, and renders
//! only what the host is already broadcasting to every phone in the room. Its
//! URL has no fragment at all.
//!
//! It still gets a [`mint_document_nonce`]'d path and is still served only to a
//! loopback peer (`delivery::serve::route`), for the same reason every hosted
//! document is: the set of published documents is one gate, not one gate per
//! kind of document.
//!
//! [`panes::document`]: crate::native_host::panes::document

use vellum_ultralight::bridge::queue_shim;

use crate::native_host::panes::document::strip_elements_matching;

/// The first script in a lobby document. See `host_lobby_boot.js`.
pub const HOST_LOBBY_BOOT_JS: &str = include_str!("host_lobby_boot.js");

/// The last script in a lobby document. See `host_lobby_link.js`.
pub const HOST_LOBBY_LINK_JS: &str = include_str!("host_lobby_link.js");

/// The JavaScript namespace the page→host queue lives under.
///
/// Nothing rides it in this slice — the lobby is read-only, selection stays on
/// the CLI — and it is installed anyway so the bridge has ONE shape from the
/// start. The slices that put a QR overlay, settings and layout rows on this
/// same permanent surface send through this queue rather than growing a second
/// channel beside it.
///
/// Deliberately distinct from `phoenixPaneOut`: a pane's records are a
/// participant's `ClientMessage`s and are admitted as such, and these will never
/// be. Two namespaces make a record that arrived on the wrong one unrepresentable
/// rather than merely wrong.
pub const HOST_LOBBY_OUT_NAMESPACE: &str = "phoenixHostLobbyOut";

/// The script the host evaluates once a frame to collect what the surface asked
/// for.
pub fn host_lobby_drain_script() -> String {
    vellum_ultralight::bridge::drain_call(&format!("window.__{HOST_LOBBY_OUT_NAMESPACE}Drain"))
}

/// The script that hands the surface one encoded `LobbyStatePayload`.
///
/// The **exact JSON the web host consumes** on its `lobby` host channel
/// (`codec::encode_lobby_state`), untouched: the two surfaces run the same view
/// model over the same bytes, so "the native lobby shows something different"
/// cannot be a question about the payload.
pub fn host_lobby_apply_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostLobbyApply", json)
}

/// The script that hands the surface one encoded [`JoinInvite`] (issue #1329).
///
/// The crew's join code, the structured code its QR carries, and the address a
/// phone should be sent to — decided in [`super::join`], because the surface's
/// own `location.href` is the loopback URL the embedded view had to dial and is
/// the one address in the building no phone can open.
///
/// [`JoinInvite`]: super::join::JoinInvite
pub fn host_lobby_join_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostLobbyJoin", json)
}

/// The script that flips the join QR (issue #1329).
///
/// One press, from a phone's `ClientMessage::ToggleQrCode` — the same button on
/// the same phone that a browser host answers in its own JavaScript. The
/// surface's own control does not come through here: it is a click inside the
/// document, on state the document already owns.
///
/// Takes no argument, and is the only call on this bridge that does not: the
/// panel's visibility lives in `#overlay` (`gui/host-qr.js`), so there is no
/// value to carry — a `true`/`false` here would be a second opinion about a
/// state the page can already read, and the two would eventually disagree.
pub fn host_lobby_qr_toggle_script() -> String {
    "window.__phoenixHostLobbyQrToggle()".to_string()
}

/// The script that tells the surface whether to force its chrome visible.
///
/// The bridge's only primitive is a call taking a single **string** argument
/// (`vellum_ultralight::bridge::push_call`), so the flag crosses as `"true"` /
/// `"false"` and `host_lobby_boot.js` compares the string. Encoding it as a bare
/// JavaScript literal instead would work and would be the one place in this
/// bridge where a payload was not a string.
pub fn host_lobby_reveal_script(force_chrome: bool) -> String {
    vellum_ultralight::bridge::push_call(
        "window.__phoenixHostLobbyReveal",
        if force_chrome { "true" } else { "false" },
    )
}

/// Why a host page could not be turned into a lobby document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostLobbyDocumentError {
    /// The page has no `#lobby-panel` element, so there is no lobby to show.
    ///
    /// The honest reading of this is "the bundle under `--client-dir` is not a
    /// Phoenix host bundle, or was built before the lobby existed" — not
    /// something to paper over with an empty surface.
    NoLobbyPanel,
    /// `#lobby-panel` opens and never closes: its `<div>`s do not balance
    /// before the end of the document.
    UnbalancedLobbyPanel,
    /// The page has no `#qr-panel` element, so there is no join panel to show
    /// (issue #1329).
    ///
    /// Refused rather than assembled without one, for the same reason as the
    /// lobby: a viewscreen showing a crew lobby that cannot show them how to
    /// join it is worse than a host that says at the prompt what is wrong with
    /// the bundle it was pointed at.
    NoJoinPanel,
    /// `#qr-panel` opens and never closes.
    UnbalancedJoinPanel,
}

impl std::fmt::Display for HostLobbyDocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostLobbyDocumentError::NoLobbyPanel => write!(
                f,
                "the host page has no #lobby-panel element, so there is no lobby markup to \
                 composite onto the viewscreen; check that --client-dir points at a bundle \
                 built from this checkout's server.html"
            ),
            HostLobbyDocumentError::UnbalancedLobbyPanel => write!(
                f,
                "the host page's #lobby-panel never closes — its <div> elements do not \
                 balance before the end of the document"
            ),
            HostLobbyDocumentError::NoJoinPanel => write!(
                f,
                "the host page has no #qr-panel element, so the viewscreen lobby would have \
                 no way to show a crew how to join it; check that --client-dir points at a \
                 bundle built from this checkout's server.html"
            ),
            HostLobbyDocumentError::UnbalancedJoinPanel => write!(
                f,
                "the host page's #qr-panel never closes — its <div> elements do not balance \
                 before the end of the document"
            ),
        }
    }
}

impl std::error::Error for HostLobbyDocumentError {}

/// Where the lobby document is published, relative to the served root.
///
/// At the **host page's** own depth, so `gui/…` and `assets/…` resolve exactly
/// as they do for `index.html`. See the module note; the pane document's
/// `/client/` depth is the same rule applied to the other page.
///
/// `nonce` comes from `panes::document::mint_document_nonce` and does the same
/// job it does there: the loopback gate in `delivery::serve::route` is the
/// defence that does not depend on a secret, and this is what stops the path
/// being enumerable from a page that is allowed to ask.
pub fn host_lobby_document_path(nonce: &str) -> String {
    format!("/host-lobby-{nonce}.html")
}

/// The URL the lobby surface's view navigates to.
///
/// **No fragment.** A pane's URL carries its session token there because a
/// fragment is the one part of a URL a browser never transmits; this surface has
/// no identity to carry, so it has nothing to hide anywhere.
pub fn host_lobby_url(host_addr: &str, nonce: &str) -> String {
    format!("http://{host_addr}{}", host_lobby_document_path(nonce))
}

/// The element the lobby markup hangs off, in the host page.
const LOBBY_PANEL_MARKER: &str = "<div id=\"lobby-panel\"";

/// The element the join panel hangs off, in the host page (issue #1329).
///
/// `#qr-panel` and not its parent `#overlay`, deliberately: that parent also
/// carries the fleet panel (which admits other ship HOSTS, issue #1114) and the
/// connection diagnostics readout, neither of which this surface has anything
/// to say about. The overlay itself is one `<div>` with no content of its own,
/// so the document supplies it below rather than taking the page's.
const JOIN_PANEL_MARKER: &str = "<div id=\"qr-panel\"";

/// Assemble the lobby document from the host page's own `index.html`.
///
/// Pure: bytes in, bytes out. Everything that makes this document different
/// from the lobby a browser shows is decided here, so it is decided somewhere a
/// unit test can read without an SDK, a GPU or an HTTP server.
pub fn build_host_lobby_document(host_index_html: &str) -> Result<String, HostLobbyDocumentError> {
    let panel = extract_element(
        host_index_html,
        LOBBY_PANEL_MARKER,
        HostLobbyDocumentError::NoLobbyPanel,
        HostLobbyDocumentError::UnbalancedLobbyPanel,
    )?;
    // The one control in that markup, and it must not appear on a read-only
    // surface: nothing on this document is wired to launch anything.
    let panel = strip_elements_matching(panel, "button", None);

    let join_panel = extract_element(
        host_index_html,
        JOIN_PANEL_MARKER,
        HostLobbyDocumentError::NoJoinPanel,
        HostLobbyDocumentError::UnbalancedJoinPanel,
    )?;

    let head = format!(
        "\n<script>\n{}\n{}</script>\n",
        queue_shim(HOST_LOBBY_OUT_NAMESPACE),
        HOST_LOBBY_BOOT_JS,
    );
    Ok(format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"UTF-8\" />\n\
         <link rel=\"stylesheet\" href=\"gui/tokens.css\" />\n\
         <link rel=\"stylesheet\" href=\"gui/host-lobby.css\" />\n\
         <link rel=\"stylesheet\" href=\"gui/host-qr.css\" />\n\
         <style>\n{GROUND_CSS}</style>{head}\
         <script src=\"{QR_ENCODER_SRC}\"></script>\n\
         </head>\n\
         <body>\n\
         {panel}\n\
         <div id=\"overlay\">\n{join_panel}\n</div>\n\
         {QR_TOGGLE_MARKUP}\n\
         <script type=\"module\">\n{HOST_LOBBY_LINK_JS}\n</script>\n\
         </body>\n\
         </html>\n"
    ))
}

/// The QR encoder, from this host's own delivery server (issue #1329).
///
/// The whole reason it is vendored: a bridge machine is not assumed to have
/// internet, and until #1329 this was a `<script src="https://cdn…">` on the
/// host page — which would have meant a native lobby that could show a crew
/// everything except how to join. Same file, same path, same bytes as
/// `server.html` loads; see `gui/vendor/README.md`.
///
/// A CLASSIC script, and after the boot block above rather than before it: it
/// assigns the global `QRCode` that `gui/host-qr.js` is handed, and nothing
/// needs it until the module island renders the first invitation.
const QR_ENCODER_SRC: &str = "gui/vendor/qrcode.js";

/// The native surface's own QR control (issue #1329).
///
/// A host PAGE has a settings cog whose Gameplay tab carries "toggle the QR"
/// (`gui/server-settings.js` → `__hostToggleQrCode`). The native viewscreen
/// window has no cog and no chrome of its own, so this document grows the one
/// control that decision needs — the smallest honest affordance, reachable
/// exactly when the surface is: in the lobby, and in play once F9 has revealed
/// it (`super::reveal`).
///
/// It carries **no text of its own**. `data-i18n` is resolved by `applyToDom`
/// in `host_lobby_link.js`, from the same string the host page's settings menu
/// uses for the same action — one button, one name, whatever language the room
/// is in (AGENTS.md rule 11). An English fallback inside the element would be
/// player-visible prose authored in a Rust source file.
const QR_TOGGLE_MARKUP: &str = "<div id=\"host-lobby-qr-toggle\" role=\"button\" tabindex=\"0\" \
     data-i18n=\"settings.toggle_qr\"></div>";

/// The page ground this document supplies, because a fragment cannot.
///
/// Two declarations, and both are load-bearing:
///
/// * `html, body` reset — the shared lobby sheet positions `.lobby-panel` at
///   `inset: 0`, which needs a viewport-sized ground with no default margin.
/// * the black background — the host copies this view's pixels into an opaque
///   texture, so an unpainted body would composite as white over the viewscreen
///   for the frames before the first push arrives.
///
/// What is deliberately NOT here is `--settings-cog-keepout`.
/// `gui/host-lobby.css` floors `.lobby-panel-wrap`'s left padding at the host
/// PAGE's settings-cog corner, and names the token with a `0px` fallback. There
/// is no cog on the viewscreen window, so leaving the property undefined *is*
/// this surface's answer — and defining it to zero here would be a second way of
/// saying the same thing, in the document rather than in the sheet that knows
/// why the floor exists.
const GROUND_CSS: &str = "\
html, body { margin: 0; padding: 0; width: 100%; height: 100%; background: #000; overflow: hidden; }\n\
body { font-family: monospace; }\n";

/// One `<div>` element of `html`, opening tag to closing tag.
///
/// Depth-counted over `<div>`/`</div>` rather than "find the next `</div>`",
/// because the panels are several levels of nesting deep. HTML comments are
/// skipped while counting: the real markup carries several, and one of them
/// mentioning a `div` would otherwise unbalance the count and truncate the
/// element at a plausible-looking place.
///
/// The two errors are passed in rather than derived, so a caller says which
/// panel it could not find — "the host page has no join panel" and "the host
/// page has no lobby" are different things to be told at a prompt.
fn extract_element<'a>(
    html: &'a str,
    marker: &str,
    missing: HostLobbyDocumentError,
    unbalanced: HostLobbyDocumentError,
) -> Result<&'a str, HostLobbyDocumentError> {
    let start = html.find(marker).ok_or(missing)?;
    let rest = &html[start..];
    let bytes = rest.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if rest[i..].starts_with("<!--") {
            match rest[i..].find("-->") {
                Some(end) => {
                    i += end + 3;
                    continue;
                }
                // An unterminated comment swallows the rest of the document, so
                // the panel never closes — which is exactly what this says.
                None => return Err(unbalanced),
            }
        }
        if rest[i..].starts_with("</div>") {
            // `depth` cannot be 0 here — the scan starts on the panel's own
            // opening tag — but a saturating decrement keeps a future caller
            // that started it elsewhere out of an arithmetic panic.
            depth = depth.saturating_sub(1);
            i += "</div>".len();
            if depth == 0 {
                return Ok(&rest[..i]);
            }
            continue;
        }
        if rest[i..].starts_with("<div") {
            depth += 1;
            i += "<div".len();
            continue;
        }
        // Advance by one CHARACTER, not one byte: `html` is `&str` and slicing
        // it mid-codepoint panics. The lobby markup is ASCII but the page around
        // it is not (its comments are full of box-drawing rules).
        i += rest[i..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(bytes.len());
    }
    Err(unbalanced)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the real host page, reduced to what the assembly depends
    /// on: a nested `#lobby-panel` with the AI-launch button inside it, and
    /// enough surrounding page to prove nothing else is taken.
    ///
    /// The *real* page is asserted against too — see
    /// [`a_lobby_document_assembles_from_the_repositorys_own_host_page`], which
    /// reads `server.html` off disk.
    const HOST_PAGE: &str = "<!doctype html>\n<html>\n<head>\n\
         <link rel=\"stylesheet\" href=\"gui/tokens.css\" />\n\
         </head>\n<body>\n\
         <div id=\"scenario-panel\"><div id=\"world-list\">pick</div></div>\n\
         <div id=\"lobby-panel\" class=\"lobby-panel\" style=\"display:none;\">\n\
         <div class=\"lobby-bg\"></div>\n\
         <!-- AI-only launch -->\n\
         <button id=\"ai-launch-btn\">Launch AI Ship</button>\n\
         <div class=\"lobby-panel-wrap\">\n\
         <div id=\"station-grid\" class=\"lobby-grid\"></div>\n\
         <aside class=\"lobby-rail\"><div id=\"lobby-status-hint\"></div></aside>\n\
         </div>\n\
         </div>\n\
         <div id=\"overlay\">\n\
         <div id=\"qr-panel\">\n\
         <div id=\"qr-caption\" data-i18n=\"server.qr_caption\"></div>\n\
         <a id=\"qr-link\"><canvas id=\"qr\"></canvas></a>\n\
         <div id=\"qr-url-row\"><span id=\"qr-url\"></span></div>\n\
         <div id=\"join-code-row\" style=\"display:none\"><span id=\"join-code\"></span></div>\n\
         </div>\n\
         <div id=\"fleet-panel\"><div id=\"fleet-slots\"></div></div>\n\
         </div>\n\
         <canvas id=\"canvas\"></canvas>\n\
         </body>\n</html>\n";

    #[test]
    fn the_lobby_markup_is_the_host_pages_own_nest_and_stops_where_it_stops() {
        // The claim that makes "one lobby" true: the document carries the
        // page's own element ids, and nothing that is not the lobby.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(html.contains("id=\"lobby-panel\""));
        assert!(html.contains("id=\"station-grid\""));
        assert!(html.contains("id=\"lobby-status-hint\""));
        assert!(html.contains("class=\"lobby-panel-wrap\""));
        assert!(
            !html.contains("id=\"scenario-panel\""),
            "the scenario picker is the host page's, not the lobby's"
        );
        assert!(
            !html.contains("id=\"canvas\""),
            "the depth count must stop at the panel's own closing tag"
        );
        // The `<canvas>` that IS here is the join panel's own (issue #1329) —
        // the QR is drawn into it — and it comes from the page's `#qr-panel`,
        // never from a runaway count over the viewscreen's.
        assert!(html.contains("id=\"qr\""));
    }

    #[test]
    fn the_ai_launch_button_is_removed_because_this_surface_is_read_only() {
        // Selection stays on the CLI in this slice. A button that silently does
        // nothing is worse than no button: it invites the one press that would
        // launch a mission, and answers it with nothing at all.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(!html.contains("<button"));
        assert!(!html.contains("ai-launch-btn"));
        // …and removing it took nothing else with it.
        assert!(html.contains("class=\"lobby-bg\""));
        assert!(html.contains("id=\"station-grid\""));
    }

    #[test]
    fn the_boot_script_runs_before_the_module_island_that_needs_it() {
        // `host_lobby_link.js` assigns `window.__phoenixHostLobby.render`, so
        // the object the boot script publishes has to exist first — and the
        // host starts pushing the moment the document reports itself loaded,
        // which is before either has necessarily run.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        let boot = html.find("window.__phoenixHostLobbyApply").unwrap();
        let link = html.find("host-lobby-render.js").unwrap();
        let panel = html.find("id=\"lobby-panel\"").unwrap();
        assert!(
            boot < panel,
            "the bridge is installed before the markup parses"
        );
        assert!(
            panel < link,
            "the renderer runs after the markup it writes into"
        );
        assert!(html.find("<head>").unwrap() < boot);
        assert!(link < html.rfind("</body>").unwrap());
    }

    #[test]
    fn the_page_to_host_queue_is_installed_under_the_namespace_the_host_drains() {
        // Nothing rides it yet. It is installed now so the slices that will —
        // the QR overlay, settings, layout rows — extend one bridge instead of
        // opening a second.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(html.contains("window.phoenixHostLobbyOut.send"));
        assert!(html.contains("window.__phoenixHostLobbyOutDrain"));
        assert_eq!(
            host_lobby_drain_script(),
            "window.__phoenixHostLobbyOutDrain()"
        );
    }

    #[test]
    fn the_document_loads_the_same_stylesheets_the_web_lobby_does() {
        // gui/host-lobby.css and gui/host-qr.css exist BECAUSE of this surface
        // (issues #1325 and #1329 lifted them out of server.html's inline
        // <style>), so a document that did not link them would have re-created
        // the duplication the extractions removed.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(html.contains("href=\"gui/tokens.css\""));
        assert!(html.contains("href=\"gui/host-lobby.css\""));
        assert!(html.contains("href=\"gui/host-qr.css\""));
        // Relative, not absolute: the path is what makes the served depth do the
        // resolving, exactly as it does for a pane document.
        assert!(!html.contains("href=\"/gui/"));
    }

    #[test]
    fn the_join_panel_comes_with_the_lobby_because_a_crew_has_to_get_in() {
        // Issue #1329. The page's OWN join markup, for the same reason the
        // lobby's is the page's own: a hand-written copy here would render
        // nowhere the first time somebody added a row to the web panel.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        for id in [
            "overlay",
            "qr-panel",
            "qr-caption",
            "qr-link",
            "qr",
            "qr-url",
            "join-code",
        ] {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "the lobby document must carry #{id}, which gui/host-qr.js writes into"
            );
        }
    }

    #[test]
    fn the_overlay_carries_the_join_panel_and_nothing_else_the_page_puts_there() {
        // #overlay is this document's own one-line wrapper, not the page's: the
        // page's also holds the fleet panel (which admits other ship HOSTS,
        // issue #1114) and the connection diagnostics, and a viewscreen has
        // nothing to say about either.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(!html.contains("id=\"fleet-panel\""));
        assert!(!html.contains("id=\"fleet-slots\""));
    }

    #[test]
    fn the_encoder_is_the_local_copy_because_a_bridge_has_no_internet() {
        // The whole point of vendoring it (issue #1329): the host serves this
        // document AND the encoder it loads, so a room with no network still
        // gets a join code. A CDN here would have been a lobby that shows a
        // crew everything except how to join.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(html.contains("<script src=\"gui/vendor/qrcode.js\"></script>"));
        assert!(!html.contains("http://cdn."));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn the_surfaces_own_qr_control_carries_a_string_id_and_no_english() {
        // A host page toggles the QR from its settings cog; this window has no
        // cog. The control's text is resolved by `applyToDom` from the same
        // string id the page's own menu uses — prose in this file would be
        // player-visible English authored in a Rust source (AGENTS.md rule 11).
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(html.contains("id=\"host-lobby-qr-toggle\""));
        assert!(html.contains("data-i18n=\"settings.toggle_qr\""));
        assert!(html.contains("role=\"button\" tabindex=\"0\""));
    }

    #[test]
    fn a_page_with_no_join_panel_is_refused_by_name_too() {
        // A lobby nobody can be shown how to join is not a lobby to composite
        // silently — it is a bundle that is not what the operator thinks.
        let page = "<body><div id=\"lobby-panel\"><div id=\"station-grid\"></div></div></body>";
        assert_eq!(
            build_host_lobby_document(page),
            Err(HostLobbyDocumentError::NoJoinPanel)
        );
    }

    #[test]
    fn the_cog_keep_out_is_left_undefined_because_there_is_no_cog_on_this_window() {
        // `.lobby-panel-wrap` floors its left padding at the host PAGE's
        // settings-cog corner, naming the token with a `0px` fallback. Leaving
        // the property undefined here is what selects that fallback; defining it
        // to the page's own value would be an unexplained indent on a window
        // that has no cog.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(!html.contains("--settings-cog-keepout"));
    }

    #[test]
    fn the_body_is_painted_because_its_pixels_are_copied_into_an_opaque_texture() {
        // Before the first lobby push the page has nothing to show, and the
        // host still copies its frame. An unpainted body would composite as a
        // white sheet over the viewscreen for those frames.
        let html = build_host_lobby_document(HOST_PAGE).unwrap();
        assert!(html.contains("background: #000"));
    }

    #[test]
    fn a_page_with_no_lobby_is_refused_by_name_rather_than_shown_empty() {
        assert_eq!(
            build_host_lobby_document("<html><body><canvas></canvas></body></html>"),
            Err(HostLobbyDocumentError::NoLobbyPanel)
        );
    }

    #[test]
    fn a_lobby_panel_that_never_closes_is_refused_rather_than_truncated() {
        let unbalanced = "<body><div id=\"lobby-panel\"><div class=\"x\"></div></body>";
        assert_eq!(
            build_host_lobby_document(unbalanced),
            Err(HostLobbyDocumentError::UnbalancedLobbyPanel)
        );
    }

    #[test]
    fn a_comment_mentioning_a_div_does_not_unbalance_the_count() {
        // The real page's lobby markup is full of comments; one of them saying
        // `</div>` would truncate the panel at a place that still looked like
        // valid markup.
        let page = "<div id=\"lobby-panel\"><!-- a </div> in prose --><div>x</div></div>\
                    <div id=\"qr-panel\"></div>\
                    <canvas id=\"trailing\"></canvas>";
        let html = build_host_lobby_document(page).unwrap();
        assert!(html.contains("<div>x</div>"));
        assert!(!html.contains("trailing"));
    }

    #[test]
    fn the_document_path_sits_at_the_host_pages_own_depth() {
        // This is what makes `gui/host-lobby-render.js` and
        // `../assets/strings/strings.csv` resolve as they do for index.html.
        // A pane's `/client/` depth is the same rule applied to the other page.
        assert_eq!(host_lobby_document_path("abcd"), "/host-lobby-abcd.html");
        assert_eq!(
            host_lobby_url("127.0.0.1:8080", "abcd"),
            "http://127.0.0.1:8080/host-lobby-abcd.html"
        );
    }

    #[test]
    fn the_lobby_url_carries_no_fragment_because_there_is_no_identity_to_carry() {
        // A pane's URL puts its session token in the fragment. This surface is
        // not a participant: no token, no Identify, no station.
        let url = host_lobby_url("127.0.0.1:8080", "abcd");
        assert!(!url.contains('#'));
        assert!(!url.contains("token"));
    }

    #[test]
    fn a_pushed_payload_is_escaped_into_the_apply_call() {
        assert_eq!(
            host_lobby_apply_script(r#"{"phase":"Lobby","scenario_title":"O'Neil"}"#),
            r#"window.__phoenixHostLobbyApply('{"phase":"Lobby","scenario_title":"O\'Neil"}')"#
        );
    }

    #[test]
    fn the_reveal_flag_crosses_as_a_string_because_that_is_the_bridges_one_shape() {
        assert_eq!(
            host_lobby_reveal_script(true),
            "window.__phoenixHostLobbyReveal('true')"
        );
        assert_eq!(
            host_lobby_reveal_script(false),
            "window.__phoenixHostLobbyReveal('false')"
        );
    }

    #[test]
    fn a_lobby_document_assembles_from_the_repositorys_own_host_page() {
        // Every other test here runs against a stub, so nothing checked the
        // assembly against the page it actually operates on. The repository's
        // `server.html` is checked in, so this needs no build step — and
        // `trunk build` copies its lobby markup into `dist/index.html`
        // untouched (it rewrites only the `data-trunk` links and appends the
        // wasm bootstrap).
        let page = std::fs::read_to_string("server.html")
            .expect("the repository's own host page is checked in");
        let html =
            build_host_lobby_document(&page).expect("the real page becomes a lobby document");

        // The ids `gui/host-lobby-render.js` writes into. If the web lobby ever
        // renames one, this fails here rather than rendering nothing on the
        // viewscreen with a clean log.
        for id in [
            "lobby-panel",
            "lobby-title",
            "lobby-subtitle",
            "lobby-crew-count",
            "lobby-crew-dots",
            "lobby-spectator-tag",
            "lobby-ready-badge",
            "lobby-countdown",
            "station-grid",
            "reserved-aggregate",
            "lobby-spectator-list",
            "lobby-status-hint",
        ] {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "the lobby document must carry #{id}, which the shared renderer writes into"
            );
        }

        // The join panel's own ids, which gui/host-qr.js writes into (issue
        // #1329) — taken out of the same page, by the same rule.
        for id in ["qr-panel", "qr", "qr-url", "join-code", "join-code-row"] {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "the lobby document must carry #{id}, which the shared join panel writes into"
            );
        }

        // …and stops at each panel. `#hud-overlay` is the next full-screen
        // surface in the page and a runaway count would swallow it; the fleet
        // panel is #overlay's other child and is not this surface's business.
        assert!(!html.contains("id=\"hud-overlay\""));
        assert!(!html.contains("id=\"canvas\""));
        assert!(!html.contains("id=\"fleet-panel\""));
        assert!(!html.contains("<button"));
    }

    #[test]
    fn the_repositorys_own_host_page_links_the_stylesheet_this_document_links() {
        // The other half of the extraction: if server.html went back to an
        // inline lobby block, this document would be styling itself from a file
        // nothing else used — the duplication, restored quietly.
        let page = std::fs::read_to_string("server.html").unwrap();
        assert!(
            page.contains("href=\"gui/host-lobby.css\""),
            "server.html must link the shared lobby stylesheet this document also links"
        );
        assert!(
            !page.contains(".lobby-panel-wrap {"),
            "the lobby's rules belong in gui/host-lobby.css, not back in server.html"
        );
        assert!(
            page.contains("href=\"gui/host-qr.css\""),
            "server.html must link the shared join-panel stylesheet this document also links"
        );
        assert!(
            !page.contains("#qr-panel {"),
            "the join panel's rules belong in gui/host-qr.css, not back in server.html"
        );
        assert!(
            page.contains("src=\"gui/vendor/qrcode.js\""),
            "both surfaces load the vendored encoder from the host's own server (issue #1329)"
        );
    }
}
