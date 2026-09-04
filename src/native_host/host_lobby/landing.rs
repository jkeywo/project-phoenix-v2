//! What the viewscreen's landing screen is shown (issue #1361, PRD #1355).
//!
//! # The landing is the host page's, rendered from the host process's own answer
//!
//! The same arrangement [`super::scenario`] makes for the picker, one slice
//! later and for the same reason. `gui/host-landing-view.js` decides what the
//! menu offers and which route is open; `gui/host-landing-render.js` writes it
//! into whichever document it is handed. Both were written as shared modules in
//! issue #1360 *because of this surface* — so nothing here re-decides anything
//! they decide, and there is exactly one landing.
//!
//! What crosses is therefore very little: the two facts the page cannot know
//! about the process it is embedded in.
//!
//! * **which build this is.** A browser host reads it from a `<meta>` tag
//!   (`server.html`'s `phoenix-build-id`), because on that surface the landing
//!   is the first paint and there is no WASM instance to ask. A native host has
//!   the opposite problem — the page is a fragment assembled out of somebody
//!   else's bundle, and the bundle's build id is the CLIENT's, not this
//!   binary's. So the host tells it, from [`BUILD_ID`].
//! * **whether the landing is past.** A World has been committed, so the front
//!   door has nothing left to offer and the crew lobby underneath it (`z-index`
//!   180, against the landing's 205) has to be visible. On the host page that is
//!   page lifecycle — `hideLanding()` — and on this surface the host is the only
//!   thing that knows, which is why it is pushed rather than inferred.
//!
//! Which entry is OPEN is deliberately not here. That is `nextOpenEntry`'s
//! answer over the page's own memory of it, exactly as `_landingOpenEntry` is
//! the host page's; a copy of the rule in Rust would be a second authority on
//! the one decision #1360 made pure so it could be tested without a document.
//! What the operator did still reaches the host — as
//! [`HostLobbyRecord::LandingOpen`](super::HostLobbyRecord::LandingOpen) and
//! [`LandingClose`](super::HostLobbyRecord::LandingClose) — so the process can
//! say in its log what its own viewscreen is showing, and so the entries that
//! need the HOST to answer them (#1365's Exit to Desktop, #1366's mod packs,
//! #1367's settings) extend one vocabulary rather than opening a second.

use serde::{Deserialize, Serialize};

/// Which build this host binary is, for the landing's status bar.
///
/// The crate version, not a deploy stamp: a report from a room full of people
/// carries the build they were on, and the honest thing a native binary knows
/// about itself is the version it was compiled at. The web host's `dev` default
/// is the same claim made where no deploy has rewritten the meta tag.
pub const BUILD_ID: &str = env!("CARGO_PKG_VERSION");

/// The landing screen's whole state, as one snapshot the surface renders.
///
/// Deserialised by `host_lobby_link.js` straight into `landingViewModel`'s
/// `build` and `dismissed` inputs. It carries no `platform`: that is a fact
/// about the document rather than about the run, and the link — which lives in
/// `src/native_host/` and is only ever loaded by this surface — says `'native'`
/// where it says `ownPanelVisibility: true`, beside the other facts about the
/// document it is wiring.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandingPanelPayload {
    /// [`BUILD_ID`], as the status bar's `{build}` parameter.
    pub build: String,
    /// A World has been committed: take the landing off screen.
    ///
    /// `false` is what a world-less host pushes on its first frame, and is what
    /// puts the landing on the viewscreen at all — the document assembles
    /// `#landing-panel` at `display: none` for the same reason it assembles the
    /// picker that way, so a `--world` host, which never pushes, never flashes a
    /// front door it has already walked through.
    pub dismissed: bool,
}

impl LandingPanelPayload {
    /// The landing a host in this state shows.
    pub fn new(dismissed: bool) -> Self {
        Self {
            build: BUILD_ID.to_string(),
            dismissed,
        }
    }

    /// Encode for the bridge. Infallible in practice — one string and one bool —
    /// and an encode that somehow failed answers with a landing that is *gone*
    /// rather than one covering a running mission, because the surface it would
    /// cover is the viewscreen a room full of people are watching.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"build":"","dismissed":true}"#.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_carries_the_two_things_the_page_cannot_know_for_itself() {
        // Everything else the landing shows is decided by the shared view model
        // from the shared entry table. These two are facts about the PROCESS,
        // which is the whole reason there is a push at all.
        let json = LandingPanelPayload::new(false).to_json();
        assert!(json.contains(r#""dismissed":false"#));
        assert!(json.contains(&format!(r#""build":"{BUILD_ID}""#)));
        // …and nothing else. A `platform` or an `open_entry` here would be the
        // host holding an opinion the page already holds.
        assert!(!json.contains("platform"));
        assert!(!json.contains("open_entry"));
    }

    #[test]
    fn a_committed_world_is_pushed_as_dismissed() {
        let json = LandingPanelPayload::new(true).to_json();
        assert!(json.contains(r#""dismissed":true"#));
    }

    #[test]
    fn the_build_id_is_this_binarys_own_and_never_the_bundles() {
        // The page is assembled out of somebody else's `--client-dir` bundle,
        // whose build id belongs to the CLIENT. A viewscreen naming that in a
        // bug report would send whoever reads it to the wrong commit.
        assert_eq!(LandingPanelPayload::new(false).build, BUILD_ID);
        assert!(!BUILD_ID.is_empty());
    }
}
