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
//! fragment cannot carry: a `<head>` naming the five stylesheets the web host
//! links (`gui/tokens.css`, `gui/host-lobby.css`, `gui/host-qr.css`,
//! `gui/host-scenarios.css`, `gui/host-landing.css` — the last four exist
//! *because* of this surface, the landing's written that way one slice ahead of
//! arriving here) plus the one it does not (`gui/native-settings.css`, issue
//! #1367: the host page paints its settings overlay from an inline `<style>`
//! block there is no way to slice, so the surface that mounts the shared kit
//! brings a token-only sheet), a ground colour, the vendored QR encoder, the
//! bridge
//! scripts, and the module island that wires the shared view models to the
//! shared renderers.
//!
//! It carries no `<title>`, deliberately: an embedded view has no tab bar, so a
//! title would be player-visible English nothing ever shows — and every string a
//! player CAN see on this surface comes through `gui/strings.js` from
//! `assets/strings/strings.csv`, like the web lobby's (AGENTS.md rule 11).
//!
//! It also carries no Google Fonts `<link>`, which `server.html` does have. A
//! bridge machine is not assumed to have internet, and a render-blocking
//! stylesheet on a host that does not would stall the surface's first paint for
//! however long the DNS lookup takes. The shared sheet names its faces through
//! the vocabulary's `--font-display` / `--font-mono`, each of which is declared
//! in `gui/tokens.css` with a full fallback stack, so the cost is the
//! substitute face rather than an unstyled lobby.
//!
//! # What the document adds and takes away
//!
//! | edit | why |
//! |---|---|
//! | the page's `#qr-panel` carried too, in an `#overlay` of this document's own (issue #1329) | the crew have to be shown how to JOIN the lobby they are looking at. The panel is the page's own markup for the same reason the lobby is; the overlay around it is not, because the page's also carries the fleet panel and the diagnostics readout, and neither has anything to say on a viewscreen |
//! | `#host-lobby-qr-toggle` added (issue #1329) | a host page toggles the QR from its settings cog; this window had no cog, so the one decision that surface genuinely needs got the one control it needs. #1367 gave the window a cog and put the same decision on its Gameplay tab as well — two triggers, one `gui/host-qr.js` toggle — and this control stays, because a join code is what a room needs fastest and one press from the screen the crew are looking at is the whole point of it |
//! | the page's `#scenario-panel` carried too (issue #1328) | an operator has to be able to pick the scenario and the hull from the viewscreen. Same rule as the lobby and the join panel: the page's own markup, so `gui/host-scenario-render.js` writes into the ids it expects |
//! | that panel's `#mod-pack-upload` and `#snapshot-import` removed (issue #1328) | host TOOLING — file inputs with page-lifetime handlers this document does not carry, and which a demo build removes outright. A control that silently does nothing is worse than no control |
//! | the lobby rail's `#gm-start-controls` buttons removed (issue #1300's Game Master, merged onto #1325) | the same rule: the GM Ready/Force Start buttons are wired by `server.html`'s GM session script, which this surface does not run, so on the viewscreen they would be dead controls. Their `<div class="gm-start-actions">` is stripped; the section's aria-hidden status regions carry no control and stay |
//! | that panel starts `display: none` (issue #1328) | the page opens ON the picker, because a browser host always chooses at the prompt; a native host may have been given `--world`, and a picker covering the lobby of a host that has nothing to pick would be a viewscreen that never moves. It is shown by the first scenario push, which only a world-less host makes |
//! | the page's `#landing-panel` carried too, and starting `display: none` (issue #1361) | the front door, by the same rule again: the page's own markup, so `gui/host-landing-render.js` writes into the ids it expects, and hidden until the first landing push — which, like the picker's, only a world-less host makes. A `--world` host was told at the prompt what it is flying and must not be shown a menu asking |
//! | that panel's `#landing-ship` hull column REMOVED (issue #1362) | the landing's third column, and the one part of it this surface cannot reach: the staged rung it belongs to is entered by passing `deepStage` (which `host_lobby_link.js` does not) and left by a Back control that needs a release verb `HostLobbyRecord` does not have, while `is-deep` makes `#landing-menu` inert. Carried, it would be a permanently invisible column with the ONLY hull picker mounted inside it — a multi-hull World unpickable on this screen. Removed, `gui/host-scenario-render.js` takes its documented single-column branch and draws the hulls in the World column, as this surface always did |
//! | that panel's `#landing-fullscreen-btn` KEPT (issue #1367; removed by #1361) | #1361 stripped it because a browser host's control forwards to `gui/page-chrome.js`'s one `initFullscreen`, which asks a BROWSER to fill a screen, and this window has no browser chrome. #1367 put something behind it instead: the press crosses the page->host queue as `HostLobbyRecord::ToggleFullscreen` and `fullscreen::apply_window_mode_toggle` sets the primary window's mode the way the display-assignment law already does |
//! | `gui/native-settings.css` linked (issue #1367) | the native settings overlay's chrome. The host PAGE's settings CSS lives inline in `server.html` and cannot be borrowed the way its markup can, so the surface that mounts the shared overlay kit brings a token-only sheet of its own. The kit, the tab list and every control's behaviour are the shared ones; this is only where the panel is painted |
//!
//! The AI-launch `<button>` is **kept**, and was not always: #1325 stripped it,
//! because a read-only surface with a control that silently does nothing is
//! worse than one with no control. Since #1328 it does something — the
//! force-start path is no longer wasm-only (`server::bridge::apply_force_start`)
//! — so the reason to remove it has gone with it. It is the one way a crew who
//! are all on phones can be launched from the viewscreen they are standing in
//! front of.
//!
//! That is the rule the surface has kept throughout, rather than a policy about
//! controls in general: **a control exists here exactly when something behind it
//! answers it.** #1329's QR toggle is added because the host owns the flip;
//! #1330's monitor row is not in this markup at all, because the shared renderer
//! builds those buttons from a row the host pushes, so they exist only when
//! there is a layout to move; the picker's file inputs are removed because
//! nothing here handles them; #1361's landing arrived with its fullscreen
//! control stripped and its Connect-to-Host entry absent — the second not by an
//! edit here at all, but because that entry's row in
//! `gui/host-landing-view.js` is marked `platforms: ['web']`, a native host
//! being always a host with no join leg to offer; and #1367 gave the fullscreen
//! control a host verb to reach, so it comes back by the same rule that took it
//! away. A test pins the resulting set — see
//! `the_ai_launch_button_is_kept_because_the_surface_can_now_answer_it`.
//!
//! Until #1367 the document also acted by *omission*: it left
//! `--settings-cog-keepout` undefined, which selects the `0px` fallback
//! `gui/host-lobby.css` and `gui/host-scenarios.css` both name, because there
//! was no settings cog on the viewscreen window to reserve a corner for. There
//! is one now, so the ground supplies the token and the two floors hold. The
//! rail itself still reserves nothing, for the reason `gui/host-landing.css`
//! gives: it keeps its stamp at the far end, and padding above content already
//! pushed to the other end holds nothing.
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

/// The first script in a lobby document. See `host_lobby_boot.js`.
pub const HOST_LOBBY_BOOT_JS: &str = include_str!("host_lobby_boot.js");

/// The last script in a lobby document. See `host_lobby_link.js`.
pub const HOST_LOBBY_LINK_JS: &str = include_str!("host_lobby_link.js");

/// The JavaScript namespace the page→host queue lives under.
///
/// Installed by issue #1325 with nothing riding it, so the bridge would have
/// ONE shape from the start rather than growing a second channel beside it the
/// first time something needed to send. It carries the operator's scenario and
/// hull picks and their AI-launch press (issue #1328) and their monitor presses
/// (issue #1330), all as [`super::HostLobbyRecord`]s — one namespace, one
/// vocabulary, one drain. The settings rows that land on this same permanent
/// surface go here too.
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

/// The script that hands the surface one encoded
/// [`BridgeLayoutPayload`](super::layout::BridgeLayoutPayload) — the bridge's
/// monitor row (issue #1330).
///
/// A push of its own rather than a field folded into the lobby payload above:
/// that payload is `LobbyStatePayload`, the shared Rust wire type **the browser
/// host also consumes**, and a browser host has no monitors. Growing it a
/// monitor list would put a native-only concern on the one shape whose whole
/// point is that both surfaces read the same bytes.
pub fn host_lobby_layout_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostLobbyLayout", json)
}

pub fn host_lobby_audio_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostLobbyAudio", json)
}

pub fn host_lobby_fleet_config_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostFleetConfigure", json)
}

pub fn host_lobby_fleet_update_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostFleetUpdate", json)
}

pub fn host_lobby_fleet_wire_script(frame: &str) -> String {
    let json = serde_json::to_string(frame).unwrap_or_else(|_| "\"\"".to_string());
    format!("window.__phoenixHostFleetWire?.({json});")
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

/// The script that hands the surface the scenario picker's state (issue #1328).
///
/// One encoded [`ScenarioPanelPayload`]: the catalogue this host publishes, what
/// the arbiter has locked, and whether a world has landed and closed the picker.
/// Those are precisely `scenarioCatalogView`'s three arguments, so the
/// viewscreen's picker and the host page's are one decision rendered twice
/// rather than two decisions that agree today.
///
/// [`ScenarioPanelPayload`]: super::scenario::ScenarioPanelPayload
pub fn host_lobby_scenario_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostLobbyScenario", json)
}

/// The script that hands the surface the landing screen's state (issue #1361).
///
/// One encoded [`LandingPanelPayload`]: which build this binary is, and whether
/// a World has been committed and taken the front door away. Those are the only
/// two things `landingViewModel` needs that the page cannot know for itself --
/// everything else the landing shows comes from the shared entry table, which
/// is why this payload is two fields rather than a menu.
///
/// A push of its own rather than a field folded into the scenario payload
/// beside it: that payload is precisely `scenarioCatalogView`'s three
/// arguments, and #1328 carried three and nothing else on purpose. Growing it a
/// fourth for a different screen's renderer would put two decisions back into
/// one shape that the next slice would have to split again.
///
/// [`LandingPanelPayload`]: super::landing::LandingPanelPayload
pub fn host_lobby_landing_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostLobbyLanding", json)
}

/// The script that hands the surface its mod-pack shelf (issue #1366).
///
/// One encoded [`ModPackPanelPayload`]: which folder is being scanned, what is
/// on it, what is already installed, what the last attempt had to say, and which
/// pack wins each authored path two of them share.
///
/// A push of its own rather than a field folded into the landing payload beside
/// it, for exactly the reason that payload is not a field of the picker's: the
/// landing carries the two facts the page cannot know about the PROCESS, and
/// this carries a whole panel's contents. One shape holding both would put a
/// shelf rescan behind the landing's push rule — which fires on the first frame
/// and when a World lands, and never when an operator installs something.
///
/// [`ModPackPanelPayload`]: super::packs::ModPackPanelPayload
pub fn host_lobby_packs_script(json: &str) -> String {
    vellum_ultralight::bridge::push_call("window.__phoenixHostLobbyPacks", json)
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
    /// The page has no `#scenario-panel` element, so there is nothing to pick a
    /// world from (issue #1328).
    ///
    /// Refused for the same reason as its two siblings: a `--lobby` host whose
    /// viewscreen cannot show the picker is a host that can never start a
    /// mission, and being told so at the prompt beats discovering it in a room
    /// full of people.
    NoScenarioPanel,
    /// `#scenario-panel` opens and never closes.
    UnbalancedScenarioPanel,
    /// The page has no `#landing-panel` element, so there is no front door
    /// (issue #1361).
    ///
    /// Refused for the same reason as its three siblings: a world-less host
    /// whose viewscreen cannot show the landing opens on nothing, and being
    /// told so at the prompt beats discovering it in a room full of people.
    NoLandingPanel,
    /// `#landing-panel` opens and never closes.
    UnbalancedLandingPanel,
    /// The page has no shared landing join-code panel.
    NoLandingJoinPanel,
    /// `#landing-join-panel` opens and never closes.
    UnbalancedLandingJoinPanel,
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
            HostLobbyDocumentError::NoScenarioPanel => write!(
                f,
                "the host page has no #scenario-panel element, so the viewscreen would have no \
                 way to pick a scenario; check that --client-dir points at a bundle built from \
                 this checkout's server.html"
            ),
            HostLobbyDocumentError::UnbalancedScenarioPanel => write!(
                f,
                "the host page's #scenario-panel never closes — its <div> elements do not \
                 balance before the end of the document"
            ),
            HostLobbyDocumentError::NoLandingPanel => write!(
                f,
                "the host page has no #landing-panel element, so the viewscreen would open on \
                 nothing; check that --client-dir points at a bundle built from this \
                 checkout's server.html"
            ),
            HostLobbyDocumentError::UnbalancedLandingPanel => write!(
                f,
                "the host page's #landing-panel never closes — its <div> elements do not \
                 balance before the end of the document"
            ),
            HostLobbyDocumentError::NoLandingJoinPanel => write!(
                f,
                "the host page has no #landing-join-panel element, so Join as Peer has no \
                 code entry surface; check that --client-dir points at a current bundle"
            ),
            HostLobbyDocumentError::UnbalancedLandingJoinPanel => write!(
                f,
                "the host page's #landing-join-panel never closes — its <aside> elements do \
                 not balance before the end of the document"
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

/// Where the viewscreen HUD-overlay page is served, relative to the served root.
/// A static file under the bundle's `gui/` (issue #422's `#hud-overlay`, ported
/// to the native path), unlike the lobby document which is minted per run.
pub const VIEWSCREEN_HUD_PATH: &str = "/gui/viewscreen-hud.html";

/// The URL the viewscreen HUD-overlay surface's view navigates to — the static
/// `gui/viewscreen-hud.html` on the host's own HTTP surface, dialled at the same
/// connectable address the lobby surface uses.
pub fn viewscreen_hud_url(host_addr: &str) -> String {
    format!("http://{host_addr}{VIEWSCREEN_HUD_PATH}")
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

/// The element the scenario picker hangs off, in the host page (issue #1328).
const SCENARIO_PANEL_MARKER: &str = "<div id=\"scenario-panel\"";

/// The element the landing screen hangs off, in the host page (issue #1361).
///
/// The FOURTH extraction, by the rule the three above it established: the
/// landing is fifteen element ids in a particular nest and
/// `gui/host-landing-render.js` writes into them, so a hand-written copy here
/// would render nowhere the first time somebody added a row to the web landing.
/// The menu inside it is deliberately EMPTY in the page and in this document
/// alike -- its entries are data (`gui/host-landing-view.js`'s
/// `LANDING_ENTRIES`), built by the shared renderer, which is what lets the two
/// surfaces differ exactly where a row says they differ and nowhere else.
const LANDING_PANEL_MARKER: &str = "<div id=\"landing-panel\"";

/// The shared code-entry stage used by Join as Peer.
const LANDING_JOIN_PANEL_MARKER: &str = "<aside id=\"landing-join-panel\"";

/// The landing's THIRD column — the staged hull picker — which this surface
/// does not carry (issue #1362).
///
/// The one place the landing is not taken whole, and the reason is the same
/// rule the tooling removals below follow: this document must not carry a
/// control it cannot reach.
///
/// #1362 gave the web landing a second rung. Choosing a World slides the track
/// one column along, the hulls are mounted into `#ship-list` in their own
/// column, and a Back control in that column releases the locked World and
/// slides back. Three things make that rung reachable, and this surface has
/// exactly none of them:
///
///  * the caller has to say the picker is on its `ship-picker` stage
///    (`landingViewModel`'s `deepStage`), which is what puts `is-deep` on the
///    root. `server.html` derives it from `scenarioCatalogView` on every
///    picker render; `host_lobby_link.js` has no such call, so this landing is
///    only ever `is-open` and `gui/host-landing.css` keeps `.landing-col-ship`
///    at `opacity: 0; visibility: hidden` at every breakpoint;
///  * the Back control is drawn only for a surface that supplies a
///    `backToWorlds` hook, and this one has no arbiter to answer it: releasing
///    a locked World is a HOST move here, and `HostLobbyRecord` has no verb for
///    it;
///  * `is-deep` also makes `#landing-menu` `inert`, so a surface that went deep
///    with no Back would have taken away the front door and offered no way out
///    of the column it went into.
///
/// Carried anyway, the column would therefore be a permanently invisible one
/// with the ONLY hull picker mounted inside it — a multi-hull World would be
/// unpickable on the native viewscreen, silently. Removed, the shared renderer
/// takes its documented single-column branch (`gui/host-scenario-render.js`'s
/// `const host = shipList || worldList`) and the hulls are drawn in the World
/// column, which is what this surface showed before #1362 and what it shows
/// now. That branch is not a lesser rendering of the staged one; it is the only
/// honest one on a document with one column to draw in.
///
/// A slice that wants the staged layout here buys the three bullets above — a
/// `deepStage` derived the way `server.html` derives it, a release verb on
/// `HostLobbyRecord`, and a `backToWorlds` hook that sends it — and then
/// deletes this marker. Until then this line is the record that it did not.
///
/// A `<section>`, unlike every other marker in this module, which is why
/// [`extract_tagged`] takes a tag name.
const LANDING_HULL_COLUMN_MARKER: &str = "<section id=\"landing-ship\"";

/// The two host-tooling blocks inside `#scenario-panel` that this surface must
/// not carry (issue #1328).
///
/// Both are file pickers with page-lifetime handlers `server.html` installs and
/// this document does not, and both are removed outright from a demo build
/// (`applyPreScenarioRestrictions`). Removed rather than hidden, for the reason
/// that function gives: a `display: none` control is still in the DOM to be
/// reached.
const SCENARIO_TOOLING_MARKERS: [&str; 2] =
    ["<div id=\"mod-pack-upload\"", "<div id=\"snapshot-import\""];

/// The lobby rail's privileged GM start controls — the Ready/Unready and Force
/// Start `<button>`s inside `#gm-start-controls` (the host-mesh Game Master of
/// issue #1300) — which this document does not carry either, by #1325's rule.
/// Their handlers live in `server.html`'s GM session script and this surface
/// runs no GM session, so on the viewscreen they would be exactly the dead
/// buttons the allowlist test forbids. Removed rather than hidden, for the same
/// reason as the scenario tooling. The `<div>` holding the two buttons is the
/// marker, not the enclosing `<section>`: [`extract_element`] counts `<div>`
/// nesting only, and the section's aria-hidden status regions carry no control
/// and may stay.
const LOBBY_TOOLING_MARKERS: [&str; 1] = ["<div class=\"gm-start-actions\""];

/// Assemble the lobby document from the host page's own `index.html`.
///
/// Pure: bytes in, bytes out. Everything that makes this document different
/// from the lobby a browser shows is decided here, so it is decided somewhere a
/// unit test can read without an SDK, a GPU or an HTTP server.
pub fn build_host_lobby_document(host_index_html: &str) -> Result<String, HostLobbyDocumentError> {
    // The lobby, minus the GM start controls — see `LOBBY_TOOLING_MARKERS`.
    let mut panel = extract_element(
        host_index_html,
        LOBBY_PANEL_MARKER,
        HostLobbyDocumentError::NoLobbyPanel,
        HostLobbyDocumentError::UnbalancedLobbyPanel,
    )?
    .to_string();
    for marker in LOBBY_TOOLING_MARKERS {
        panel = remove_element(&panel, marker);
    }

    let join_panel = extract_element(
        host_index_html,
        JOIN_PANEL_MARKER,
        HostLobbyDocumentError::NoJoinPanel,
        HostLobbyDocumentError::UnbalancedJoinPanel,
    )?;

    // The picker (issue #1328), minus the two host-tooling blocks, and hidden
    // until a world-less host says there is something to pick. See the module
    // table for both edits.
    let mut scenario_panel = extract_element(
        host_index_html,
        SCENARIO_PANEL_MARKER,
        HostLobbyDocumentError::NoScenarioPanel,
        HostLobbyDocumentError::UnbalancedScenarioPanel,
    )?
    .to_string();
    for marker in SCENARIO_TOOLING_MARKERS {
        scenario_panel = remove_element(&scenario_panel, marker);
    }
    let scenario_panel = scenario_panel.replacen(
        SCENARIO_PANEL_MARKER,
        &format!("{SCENARIO_PANEL_MARKER} style=\"display:none\""),
        1,
    );

    // The landing (issue #1361), whole, and hidden until the first landing push
    // -- which, like the picker's, only a world-less host makes. Its fullscreen
    // control came back in #1367, when a host verb was put behind it; see the
    // module table.
    let landing_panel = extract_element(
        host_index_html,
        LANDING_PANEL_MARKER,
        HostLobbyDocumentError::NoLandingPanel,
        HostLobbyDocumentError::UnbalancedLandingPanel,
    )?;
    // ...minus the staged hull column, the one part of the landing this surface
    // cannot reach. See `LANDING_HULL_COLUMN_MARKER` — without it the shared
    // picker takes its single-column branch and the hulls are drawn in the
    // World column, which is what this viewscreen has always shown.
    let landing_panel = remove_tagged(landing_panel, LANDING_HULL_COLUMN_MARKER, "section");
    let landing_panel = landing_panel.replacen(
        LANDING_PANEL_MARKER,
        &format!("{LANDING_PANEL_MARKER} style=\"display:none\""),
        1,
    );
    let landing_join_panel = extract_tagged(
        host_index_html,
        LANDING_JOIN_PANEL_MARKER,
        "aside",
        HostLobbyDocumentError::NoLandingJoinPanel,
        HostLobbyDocumentError::UnbalancedLandingJoinPanel,
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
         <link rel=\"stylesheet\" href=\"gui/host-scenarios.css\" />\n\
         <link rel=\"stylesheet\" href=\"gui/host-landing.css\" />\n\
         <link rel=\"stylesheet\" href=\"gui/native-settings.css\" />\n\
         <link rel=\"stylesheet\" href=\"gui/audio-settings.css\" />\n\
         <style>\n{GROUND_CSS}</style>{head}\
         <script src=\"{QR_ENCODER_SRC}\"></script>\n\
         </head>\n\
         <body>\n\
         {panel}\n\
         <div id=\"overlay\">\n{join_panel}\n</div>\n\
         {QR_TOGGLE_MARKUP}\n\
         {scenario_panel}\n\
         {landing_join_panel}\n\
         {landing_panel}\n\
         <script type=\"module\">\n{HOST_LOBBY_LINK_JS}\n</script>\n\
         </body>\n\
         </html>\n"
    ))
}

/// `html` with the `<div>` element opening at `marker` removed entirely.
///
/// Depth-counted through [`extract_element`] rather than through
/// `panes::document::strip_elements_matching`, which finds the FIRST closing tag
/// after the opening one — correct for the flat `<audio>`/`<button>` elements it
/// was written for, and wrong for both of these blocks, whose several nested
/// `<div>`s would leave a truncated fragment behind.
///
/// A marker that is not present leaves `html` untouched: this removes host
/// tooling that a demo bundle has already removed for its own reasons, so
/// "already gone" is a normal outcome rather than an error.
fn remove_element(html: &str, marker: &str) -> String {
    remove_tagged(html, marker, "div")
}

/// [`remove_element`], for an element whose tag is not `<div>`.
///
/// Same contract as its sibling, including the "absent is not an error" one:
/// a marker this page does not carry leaves the html alone, because the
/// removals are about what this SURFACE must not show and not about what the
/// page happens to contain today.
fn remove_tagged(html: &str, marker: &str, tag: &str) -> String {
    match extract_tagged(
        html,
        marker,
        tag,
        HostLobbyDocumentError::NoScenarioPanel,
        HostLobbyDocumentError::UnbalancedScenarioPanel,
    ) {
        Ok(subtree) => html.replacen(subtree, "", 1),
        Err(_) => html.to_string(),
    }
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
/// window had no cog and no chrome of its own, so this document grew the one
/// control that decision needs — the smallest honest affordance, reachable
/// exactly when the surface is: in the lobby, and in play once F9 has revealed
/// it (`super::reveal`).
///
/// Issue #1367 gave the window a cog, and put the same decision on its Gameplay
/// tab (`gui/native-settings.js`'s `NATIVE_SETTINGS_CONTROLS`). This control
/// stays anyway, and that is not a duplicate implementation: both triggers call
/// the one `gui/host-qr.js` toggle, the way three separate callers already
/// reach it. A join code is what a room needs fastest, and one press from the
/// screen the crew are looking at is the whole reason this affordance exists.
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
/// Four things, and all of them are load-bearing:
///
/// * `html, body` reset — the shared lobby sheet positions `.lobby-panel` at
///   `inset: 0`, which needs a viewport-sized ground with no default margin.
/// * the black background — the host copies this view's pixels into an opaque
///   texture, so an unpainted body would composite as white over the viewscreen
///   for the frames before the first push arrives.
/// * the join panel's layer, above **both** full-screen panels this document
///   carries (issues #1328 and #1361). This is the surface's own answer to a
///   decision `server.html` makes in JavaScript. The shared sheets declare a
///   ladder — `#overlay` is `190` (`gui/host-qr.css`), `#scenario-panel` is
///   `200` (`gui/host-scenarios.css`), `#landing-panel` is `205`
///   (`gui/host-landing.css`) — so while either of the two is up the join code
///   is behind it, and the whole point of showing the QR *during* selection is
///   that the crew join while the operator is still choosing. The host page
///   reaches the same end by moving the node instead of lifting it:
///   `showJoinQrOverPanel()` docks the live `#overlay` INTO `#scenario-panel`
///   as `.pre-scenario`, which the landing's own sheet then lays out inside its
///   middle column (`#scenario-panel.landing-docked #overlay.pre-scenario`),
///   and `resetJoinQrLayer()` moves it back out, because that page has a HUD
///   and a canvas underneath whose stacking it has to return to. This document
///   does neither, and cannot: it never adds the class and never moves the
///   node, so an `#overlay` parked inside `#scenario-panel` would go dark with
///   the picker at world load and never come back for the F9 reveal, and there
///   is no page lifecycle here to move it back. So the lift stays, it is
///   unconditional, and since #1361 it clears `205` as well as `200`.
///
///   That last rung is now belt-and-braces rather than the thing that puts the
///   code on screen: because this document floats the panel instead of docking
///   it, the join panel's own law (`joinPanelAction` in
///   `gui/host-lobby-view.js`) keeps it OFF while the landing is up — there is
///   no World, no Session and nothing to join at the front door, and a floating
///   panel would stand over it rather than sit in its middle column the way the
///   host page's docked one does. The clearance stays anyway, because stacking
///   that cannot collide is worth more than stacking that depends on a law
///   being asked in the right order. `#host-lobby-qr-toggle` rides one rung
///   above the panel, so the control that hides it is never underneath it —
///   and that control, like every other way in, refuses while the landing is up
///   (`requestToggleQr` in `host_lobby_link.js`): the verb is guarded rather
///   than the button, because a phone's `ToggleQrCode` arrives as a message and
///   a hidden button would not have covered it.
/// * the join panel's bottom inset, because that lift buys a collision. A
///   floating `#overlay` sits at `bottom: 1rem; right: 1rem`, and the landing's
///   `.landing-statusbar` is `left: 64px; right: 0; bottom: 0` — the same
///   corner, and the row under it is the build stamp. So the panel is raised by
///   the bar's height plus the inset it would otherwise have had, at both of
///   the landing sheet's breakpoints (the bar is 56px, and 44px once the rail
///   becomes a top bar). Unconditional for the same reason the lift is: nothing
///   in this document knows whether the landing is up, and holding the join
///   panel a status bar's height off the floor of the crew lobby costs a
///   viewscreen nothing.
///
/// * `--settings-cog-keepout`, which until issue #1367 was deliberately absent.
///   `gui/host-lobby.css` floors `.lobby-panel-wrap`'s left padding at the
///   settings cog's corner and `gui/host-scenarios.css` floors `#world-list`'s
///   top padding at the same token, both naming it WITH a fallback — which is
///   those sheets saying the property may legitimately be missing, as it was
///   here while this window had no cog at all. It has one now
///   (`gui/native-settings.js`), pinned into the landing rail's top, so the
///   floor has something to hold.
///
///   ONE scalar answers TWO axes — a left padding on the lobby and a top
///   padding on the picker — so it is the cog's TALLEST extent at either of the
///   landing sheet's breakpoints rather than an average of the four. The wide
///   breakpoint pins the cog at `top: 34px`, level with
///   `#landing-fullscreen-btn` in the opposite corner because the design draws
///   the two as a pair, so 46px of button reaches 80px; 92px is that plus the
///   same 12px gap the host page leaves its own cog (`server.html` declares
///   56px for a 44px corner). The other three extents are shorter — 55px to the
///   right of the wide breakpoint's `left: 9px`, 55px and 62px at the narrow
///   one — so this token is sized by its VERTICAL consumer, and it is the same
///   92px `gui/native-settings.css` drops the panel to, by the same arithmetic.
///   `the_cog_keep_out_clears_the_corner_its_cog_occupies` derives all of it
///   from those two sheets rather than restating it. The picker does not pay
///   it twice — `#scenario-panel.landing-docked #world-list` overrides that top
///   padding back to 16px, because a picker docked in the landing's middle
///   column has no cog above it.
const GROUND_CSS: &str = "\
:root { --settings-cog-keepout: 92px; }\n\
html, body { margin: 0; padding: 0; width: 100%; height: 100%; background: #000; overflow: hidden; }\n\
/* The root every `rem` on this surface is relative to, multiplied by this\n\
DISPLAY's own text-size setting (issue #1427). gui/viewscreen-presentation.js\n\
stamps --a11y-text-scale on this document's :root, so one choice at the settings\n\
cog enlarges the landing, the world picker, the join panel and the lobby chrome\n\
together. --root-size-viewscreen (gui/tokens.css) is the 16px this document rode\n\
by default written down, so an unscaled surface renders exactly as before.\n\
server.html carries the identical rule. */\n\
html { font-size: calc(var(--root-size-viewscreen) * var(--a11y-text-scale, 1)); }\n\
body { font-family: monospace; }\n\
#overlay { z-index: 210; bottom: 72px; }\n\
#host-lobby-qr-toggle { z-index: 211; }\n\
@media (max-width: 999px), (orientation: portrait) {\n\
#overlay { bottom: 60px; }\n\
}\n";

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
    extract_tagged(html, marker, "div", missing, unbalanced)
}

/// [`extract_element`], counting whichever tag the caller names.
///
/// The four panels this document borrows are `<div>`s, and were the only reason
/// this scan existed — but the landing's own columns are `<section>`s, and one
/// of them has to come OUT (see [`LANDING_HULL_COLUMN_MARKER`]). Counting
/// `<div>` while removing a `<section>` would have matched the column's inner
/// panel and left a stray `</section>` behind, so the tag is a parameter rather
/// than a second hand-rolled scanner.
///
/// It counts ONE tag name and nothing else, which is what makes it safe on the
/// element it is pointed at rather than in general: `<section id="landing-ship">`
/// nests no further sections, so its own opening and closing tags are the only
/// two the count ever sees.
fn extract_tagged<'a>(
    html: &'a str,
    marker: &str,
    tag: &str,
    missing: HostLobbyDocumentError,
    unbalanced: HostLobbyDocumentError,
) -> Result<&'a str, HostLobbyDocumentError> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
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
        if rest[i..].starts_with(close.as_str()) {
            // `depth` cannot be 0 here — the scan starts on the panel's own
            // opening tag — but a saturating decrement keeps a future caller
            // that started it elsewhere out of an arithmetic panic.
            depth = depth.saturating_sub(1);
            i += close.len();
            if depth == 0 {
                return Ok(&rest[..i]);
            }
            continue;
        }
        if rest[i..].starts_with(open.as_str()) {
            depth += 1;
            i += open.len();
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
#[path = "document_tests.rs"]
mod tests;
