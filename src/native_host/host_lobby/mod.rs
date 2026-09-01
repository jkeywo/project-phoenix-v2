//! The native host's own lobby surface (issue #1325).
//!
//! Launch `phoenix-host --client-dir dist --world <w>` and the viewscreen
//! window shows the crew lobby a browser host shows: the scenario title, the
//! crew counter, the station grid filling in as phones claim seats, the ready
//! badge, the countdown. It is an embedded web view composited onto the
//! viewscreen, fed by the bridge below.
//!
//! # The shape, in one pass
//!
//! ```text
//! server::viewscreen_border::push_lobby_state          the SAME system the browser host runs
//!   │  Messages<LobbyStateChanged> { json }            the SAME codec::encode_lobby_state bytes
//!   ▼
//! feed_lobby_state                                     [this module]
//!   ▼
//! HostLobbyBridge::push_lobby_state                    [bridge] latest-wins, deduped
//!   ▼
//! pump_host_lobby → PaneSurface::push                  [bridge, over panes::surface]
//!   ▼
//! window.__phoenixHostLobbyApply(json)                 [host_lobby_boot.js]
//!   ▼
//! localiseHostPayload → hostLobbyViewModel → renderHostLobby
//!        gui/host-channel.js   gui/host-lobby-view.js   gui/host-lobby-render.js
//! ```
//!
//! **Every box under the bridge is the web host's own.** The last line is the
//! point of the whole issue: `server.html`'s `__updateLobby` calls those three
//! modules in that order, and so does this surface's document. There is one
//! lobby renderer, not two.
//!
//! # What is here, and why each piece is where it is
//!
//! | piece | what it decides |
//! |---|---|
//! | [`document`] | what the surface loads: the host page's own `#lobby-panel` and `#qr-panel`, assembled in memory and served at the host page's own depth |
//! | [`bridge`] | what crosses, in both directions, and what a failed push costs |
//! | [`join`] | what the join panel says, and where its QR points |
//! | [`reveal`] | when the surface is on screen, and when it has yielded |
//! | this file | the Bevy wiring, and [`LocalHostLobby`], which the binary assembles after its listener has bound |
//!
//! Every one of them compiles and is tested with the `ultralight` feature
//! **off**, which is how every CI job in this repository builds — the same
//! arrangement [`panes`](super::panes) makes, and for the same reason: a claim
//! that could only be checked on a Windows machine with a GPU is a claim nobody
//! checks. Only the compositing and the input translation are behind the SDK,
//! in [`panes::ultralight`](super::panes::ultralight), which is where the one
//! Ultralight runtime a process may have already lives.
//!
//! # The surface is permanent
//!
//! It is created once and never torn down. On mission start the *chrome*
//! yields — see [`reveal`] — and one host key ([`HOST_LOBBY_REVEAL_KEY`])
//! brings it back. The join QR arrived on this same surface in issue #1329, and
//! the settings and layout rows follow, so nothing downstream should learn to
//! rebuild it.
//!
//! One consequence is worth stating plainly, because it is the shape of the
//! native answer rather than an omission: **in play, the QR is visible only
//! while the surface is revealed.** The view is composited into an opaque
//! texture (its body is painted; see [`document`]), so there is no way to float
//! a QR alone over a running viewscreen the way a browser host does — showing
//! the code means showing the surface. F9 is therefore the in-play "show the
//! join code" gesture, and a phone's toggle sets what the operator sees when
//! they press it.
//!
//! # It is under the panes, so a tiled host never sees it
//!
//! The surface takes the **whole** primary window and draws beneath the panes
//! (`ZIndex(-1)`), and tiled `--pane` consoles divide that same window between
//! them and cover it completely — so on `phoenix-host --pane … --pane …` the
//! lobby is composited and pushed to but never visible. It is on screen for the
//! two arrangements that leave the primary window free: no `--pane` at all, and
//! a `--profile` that seats every pane on a Station window. Nothing here fails
//! in the tiled case; it is simply occluded, which is worth knowing before
//! debugging a lobby that "does not appear".

pub mod bridge;
pub mod document;
pub mod join;
pub mod reveal;

use bevy::prelude::*;

use crate::console_bridge::LobbyStateChanged;
use crate::core::messages::GamePhase;
use crate::delivery::serve::HostedDocuments;
use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::panes::document::{connectable_host_addr, mint_document_nonce};
use crate::native_host::panes::PaneId;

pub use bridge::{pump_host_lobby, HostLobbyBridge, HostLobbyPumpReport};
pub use document::{build_host_lobby_document, HostLobbyDocumentError};
pub use join::JoinInvite;
pub use reveal::{RevealState, SurfacePresence};

/// The host key that reveals and hides the lobby surface during play.
///
/// **F9**, and named here so the binary's help text, the operator log and the
/// input system all read the same declaration rather than three copies of a
/// keycode.
///
/// Chosen against the two things already bound in a native host: `Ctrl+Tab` is
/// inter-pane focus traversal (`panes::ultralight::traverse_focus_keys`), and
/// every printable key, the arrows, Home/End, Tab, Enter, Backspace and Delete
/// are forwarded verbatim into whichever pane holds focus
/// (`forward_keyboard_text`). A function key is in neither set, so revealing the
/// lobby cannot be mistaken for typing into a console — which is exactly the
/// collision that would make this control unusable on a crewed bridge.
pub const HOST_LOBBY_REVEAL_KEY: KeyCode = KeyCode::F9;

/// The lobby surface's handle in the pane input router (issue #1124).
///
/// The router, the focus ring and the touch-capture map are all keyed by
/// [`PaneId`], and the lobby surface has to appear in them — the acceptance
/// criterion is that a mouse and a keyboard operate it exactly as they operate
/// a pane. It is **not** a pane, though: it has no identity, no session and no
/// entry in [`PaneRegistry`](super::panes::PaneRegistry).
///
/// So it takes a reserved handle rather than a minted one. `PaneRegistry` mints
/// ids from `0` upward and never reuses one, so `u32::MAX` is unreachable in
/// any process that has not opened four billion panes — and unlike "the next
/// free id", it cannot collide with a pane *recreated* later in the run
/// (issue #1125), which is the case a high-water-mark scheme would get wrong.
///
/// Everything that treats a `PaneId` as a participant — the pane bus, the fault
/// path, the close-and-retire sweep — checks for this id and skips it. See
/// `panes::ultralight`.
pub const HOST_LOBBY_SURFACE_ID: PaneId = PaneId(u32::MAX);

/// The bridge, as a Bevy resource.
///
/// A newtype rather than `impl Resource for HostLobbyBridge`, so [`bridge`]
/// stays a plain module a unit test can build without an `App` — the same
/// arrangement `PaneBusResource` makes.
#[derive(Resource, Clone)]
pub struct HostLobbyBridgeResource(pub HostLobbyBridge);

/// The reveal state machine, as a Bevy resource.
#[derive(Resource, Clone, Default)]
pub struct HostLobbyRevealResource(pub RevealState);

/// Everything needed to turn an issued join code into an invitation
/// (issue #1329).
///
/// A resource rather than a field on [`LocalHostLobby`], because the second
/// half of it — which rendezvous service this host registered with — is not
/// known when the surface is opened. The listener has to be bound before the
/// document can be published, and the relay socket is dialled after that.
///
/// Installed only by a host that has a lobby surface. A delivery-only or
/// headless host has nothing to put an invitation on.
#[derive(Resource, Clone)]
pub struct HostLobbyJoinResource {
    /// See [`LocalHostLobby::join_base`].
    pub join_base: String,
    /// The `--rendezvous` base, verbatim, or `None` for a host nobody can join.
    pub rendezvous: Option<String>,
}

impl HostLobbyJoinResource {
    /// Read the surface's own answer to "where should a phone be sent".
    pub fn from_lobby(lobby: &LocalHostLobby, rendezvous: Option<&str>) -> Self {
        Self {
            join_base: lobby.join_base.clone(),
            rendezvous: rendezvous.map(str::to_string),
        }
    }

    /// The invitation an issued code makes.
    pub fn invite(&self, code: &crate::core::rendezvous::JoinCode) -> JoinInvite {
        JoinInvite::from_code(code, &self.join_base, self.rendezvous.as_deref())
    }
}

/// The lobby surface one host process owns.
///
/// Assembled by `phoenix-host` **after** the delivery listener has bound,
/// because the document is published at that listener's own address and a `:0`
/// bind does not know its port until then — the same ordering constraint
/// [`LocalPanes`](super::panes::LocalPanes) has, for the same reason.
#[derive(Clone)]
pub struct LocalHostLobby {
    /// The bridge the surface and the simulation both hold.
    pub bridge: HostLobbyBridge,
    /// This surface's document path segment, minted per run.
    pub nonce: String,
    /// `host:port` the surface's view should **connect** to.
    ///
    /// Normalised from the listener's bind address by
    /// [`connectable_host_addr`], because the documented default bind is
    /// `0.0.0.0:8080` and nothing can dial that.
    pub host_addr: String,
    /// The base URL a **phone** should be sent to (issue #1329).
    ///
    /// Deliberately NOT [`Self::host_addr`], and this is the difference the
    /// join QR turns on: the surface's own address is loopback, because that is
    /// what an embedded view on this machine has to dial, and it is the one
    /// address in the building no phone can open. See [`join`] for how the
    /// shareable one is chosen.
    pub join_base: String,
}

impl LocalHostLobby {
    /// Open the lobby surface's side of the bridge and mint its document path.
    ///
    /// `host_addr` is the listener's **bind** address, and is normalised twice
    /// here — once for the view that has to dial it, and once for the phones
    /// that have to reach it.
    pub fn open(host_addr: impl AsRef<str>) -> Self {
        let bound = host_addr.as_ref();
        Self {
            bridge: HostLobbyBridge::new(),
            nonce: mint_document_nonce(),
            host_addr: connectable_host_addr(bound),
            join_base: join::join_page_base(&join::shareable_host_addr(
                bound,
                join::discover_lan_addr(),
            )),
        }
    }

    /// Tell the surface what to put in its join panel.
    pub fn publish_join(&self, invite: &JoinInvite) {
        self.bridge.push_join(invite.to_json());
    }

    /// The URL the surface's view navigates to.
    pub fn url(&self) -> String {
        document::host_lobby_url(&self.host_addr, &self.nonce)
    }

    /// Where the document is published, relative to the served root.
    pub fn path(&self) -> String {
        document::host_lobby_document_path(&self.nonce)
    }

    /// Publish the lobby document on the host's own HTTP surface.
    ///
    /// `host_index_html` is the served bundle's own `index.html` — the host
    /// page — read once: the document is that page's `#lobby-panel` markup with
    /// a head and two scripts around it, so the lobby the viewscreen shows is
    /// built from the same elements a browser host's is.
    pub fn publish(
        &self,
        host_index_html: &str,
        documents: &HostedDocuments,
    ) -> Result<(), HostLobbyDocumentError> {
        // The same machine-wide OS accessibility read every pane document gets
        // (issue #1127), by the same call: an Ultralight view has no OS-backed
        // `matchMedia`, so a `gui/` module that reads the machine's preferences
        // would otherwise initialise from nothing at all. Best-effort, read
        // once, and never across the transport seam.
        //
        // Stated honestly, because the claim is easy to overstate: the lobby
        // chrome reads NOTHING from this layer today, exactly as the web lobby
        // reads nothing from it — `gui/accessibility-profile.js` is the phone's,
        // and the host lobby's own reduced-motion answer is a CSS media query.
        // It is seeded because a document assembled for an Ultralight view is
        // assembled the same way whichever surface it is, and because the slices
        // that put profile-reading UI on this permanent surface should find the
        // layer already there rather than have to remember to add it.
        let prefs = super::panes::os_prefs::query_os_accessibility_prefs();
        let body = super::panes::document::inject_os_accessibility_defaults(
            &build_host_lobby_document(host_index_html)?,
            &prefs,
        );
        documents.publish(self.path(), body);
        Ok(())
    }

    /// Stop publishing the document.
    ///
    /// Called at shutdown, before the delivery thread is joined, so the window
    /// in which this process is still serving a bridge surface nobody is
    /// driving is empty rather than merely short.
    pub fn withdraw(&self, documents: &HostedDocuments) {
        documents.withdraw(&self.path());
    }
}

/// Feeds the lobby surface and owns its reveal state.
///
/// Installed only when a [`LocalHostLobby`] was opened, so a delivery-only or
/// headless host is byte-for-byte unchanged. Adding it without a
/// [`HostLobbyBridgeResource`] is a no-op.
pub struct HostLobbyPlugin;

impl Plugin for HostLobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HostLobbyRevealResource>().add_systems(
            Update,
            (
                feed_lobby_state,
                observe_phase,
                // `ButtonInput<KeyCode>` exists only where `InputPlugin` does,
                // and `NativeRenderSurface::Contract` stands one up nowhere.
                // Bevy validates a system's parameters when it RUNS, so a bare
                // `Res<ButtonInput<_>>` there is a panic rather than an inert
                // system — the same trap `PaneDisplayPlugin` gates against.
                toggle_reveal_key.run_if(resource_exists::<ButtonInput<KeyCode>>),
                publish_reveal,
            )
                .chain(),
        );
    }
}

/// Carry every lobby-state push the browser host would put on its `lobby`
/// channel to the native surface instead.
///
/// Reads the SAME `LobbyStateChanged` message `server::bridge`'s
/// `flush_host_channels` reads on wasm, so the two surfaces cannot be looking at
/// different lobbies. Only the newest of a frame's messages matters — the
/// payload is a snapshot — and the bridge drops one identical to the last, which
/// is what keeps an idle lobby free.
fn feed_lobby_state(
    bridge: Option<Res<HostLobbyBridgeResource>>,
    mut lobby: MessageReader<LobbyStateChanged>,
) {
    // Read first, unconditionally: an uninstalled surface must still advance
    // the reader rather than leave it trailing a buffer it never catches up on.
    let latest = lobby.read().last().map(|m| m.json.clone());
    let (Some(bridge), Some(json)) = (bridge, latest) else {
        return;
    };
    bridge.0.push_lobby_state(json);
}

/// Follow the simulation's phase, so the chrome yields at mission start and
/// comes back on a return to the lobby.
fn observe_phase(phase: Res<State<GamePhase>>, mut reveal: ResMut<HostLobbyRevealResource>) {
    // `ResMut` change detection would fire every frame if this wrote
    // unconditionally, and `publish_reveal` below is driven by it.
    if reveal.0.phase() != phase.get() {
        reveal.0.observe_phase(phase.get());
    }
}

/// The host key.
fn toggle_reveal_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut reveal: ResMut<HostLobbyRevealResource>,
    log: Option<Res<LogFilterConfig>>,
) {
    if !keys.just_pressed(HOST_LOBBY_REVEAL_KEY) {
        return;
    }
    let presence = reveal.0.toggle();
    crate::pinfo!(
        log,
        LogCat::Lobby,
        "host lobby: {} (F9)",
        if presence.composited {
            "revealed"
        } else {
            "hidden"
        }
    );
}

/// Tell the page whether to force its chrome visible.
///
/// Only when the answer changed: every push is a synchronous `evaluate_script`
/// on the simulation's own thread, and the reveal is a state rather than an
/// edge, so re-asserting it every frame would be sixty repaints a second of a
/// decision nobody made.
fn publish_reveal(
    bridge: Option<Res<HostLobbyBridgeResource>>,
    reveal: Res<HostLobbyRevealResource>,
    mut last: Local<Option<bool>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    let force = reveal.0.presence().force_chrome;
    if *last != Some(force) {
        *last = Some(force);
        bridge.0.push_reveal(force);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare `App` with the plugin's own resources and messages, and nothing
    /// else. `NativeRenderSurface::Contract` is a real composition with no
    /// `InputPlugin`, so the key system must be absent here rather than
    /// panicking — which is the arrangement this fixture reproduces.
    fn app_with_lobby() -> (App, HostLobbyBridge) {
        let bridge = HostLobbyBridge::new();
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .add_message::<LobbyStateChanged>()
            .init_state::<GamePhase>()
            .insert_resource(HostLobbyBridgeResource(bridge.clone()))
            .add_plugins(HostLobbyPlugin);
        (app, bridge)
    }

    #[test]
    fn a_lobby_state_push_reaches_the_bridge_as_the_bytes_the_web_host_reads() {
        let (mut app, bridge) = app_with_lobby();
        app.world_mut().write_message(LobbyStateChanged {
            json: r#"{"phase":"Lobby","crew_count":2}"#.to_string(),
        });
        app.update();
        assert!(bridge.has_pending());
    }

    #[test]
    fn the_plugin_runs_without_an_input_plugin_because_a_contract_host_has_none() {
        // Bevy validates a system's parameters when it runs, so a bare
        // `Res<ButtonInput<KeyCode>>` in an app with no `InputPlugin` is a
        // panic in a host that would otherwise be fine.
        let (mut app, _) = app_with_lobby();
        app.update();
        app.update();
    }

    #[test]
    fn mission_start_reaches_the_reveal_state_from_the_simulations_own_phase() {
        let (mut app, _) = app_with_lobby();
        app.update();
        assert!(
            app.world()
                .resource::<HostLobbyRevealResource>()
                .0
                .presence()
                .composited
        );

        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();
        app.update();
        let reveal = app.world().resource::<HostLobbyRevealResource>().0.clone();
        assert_eq!(reveal.phase(), &GamePhase::InProgress);
        assert!(!reveal.presence().composited);
    }

    #[test]
    fn the_reveal_flag_is_pushed_only_when_it_changes() {
        // A state, not an edge: re-asserting it every frame would be a repaint
        // a frame of a decision nobody made.
        let (mut app, bridge) = app_with_lobby();
        app.update();
        assert!(bridge.has_pending(), "the first frame states the baseline");
        // Drain it the way the frame loop would.
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        assert!(!bridge.has_pending());

        app.update();
        app.update();
        assert!(!bridge.has_pending(), "an unchanged reveal says nothing");
    }

    #[test]
    fn the_surface_id_is_not_one_the_pane_registry_can_mint() {
        // The router, the focus ring and the touch-capture map are keyed by
        // PaneId, and the lobby surface has to appear in them without being a
        // participant. Ids are minted from 0 upward and never reused.
        use crate::native_host::panes::{PaneBus, PaneIdentity};
        let bus = PaneBus::default();
        for i in 0..8 {
            let id = bus.open(
                PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", format!("p{i}"))
                    .unwrap(),
            );
            assert_ne!(id, HOST_LOBBY_SURFACE_ID);
            assert!(id.0 < 1_000, "ids are minted from zero upward: {id}");
        }
    }

    #[test]
    fn the_document_path_and_url_agree_with_each_other() {
        let lobby = LocalHostLobby::open("0.0.0.0:8080");
        assert_eq!(
            lobby.host_addr, "127.0.0.1:8080",
            "nothing can dial the documented default bind"
        );
        assert!(lobby.url().ends_with(&lobby.path()));
        assert!(lobby.path().starts_with("/host-lobby-"));
    }

    #[test]
    fn the_address_a_phone_is_sent_to_is_not_the_one_the_view_dialled() {
        // Issue #1329, and the whole reason `join_base` exists beside
        // `host_addr`: the surface loads over loopback because that is what an
        // embedded view on this machine can dial, and a QR built from THAT
        // encodes the one address in the room no phone can open.
        //
        // Which address the discovery finds depends on the machine running the
        // test, so what is asserted is the shape and the port — the parts that
        // are decisions rather than environment.
        let lobby = LocalHostLobby::open("0.0.0.0:8080");
        assert!(lobby.join_base.starts_with("http://"));
        assert!(
            lobby.join_base.ends_with(":8080/"),
            "the port the listener bound, and the trailing slash the URL builder needs: {}",
            lobby.join_base
        );

        // A bind that names an address is the operator's own answer and is used
        // as given — which is also the one case with no environment in it.
        let pinned = LocalHostLobby::open("192.168.1.5:8080");
        assert_eq!(pinned.join_base, "http://192.168.1.5:8080/");
    }

    #[test]
    fn an_issued_join_code_reaches_the_surface_as_the_page_can_use_it() {
        // The bridge plumbing for issue #1329's AC1: the code the relay was
        // issued becomes a push the document's own `__phoenixHostLobbyJoin`
        // answers, carrying the letters, the structured code and the address a
        // phone should be sent to.
        let lobby = LocalHostLobby::open("192.168.1.5:8080");
        let code = crate::core::rendezvous::JoinCode {
            full: "PHX-1-ABCDE".to_string(),
            suffix: "ABCDE".to_string(),
            ..Default::default()
        };
        let join = HostLobbyJoinResource::from_lobby(&lobby, None);
        lobby.publish_join(&join.invite(&code));

        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&lobby.bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 1);
        let pushed = &surface.pushed[0];
        assert!(pushed.starts_with("window.__phoenixHostLobbyJoin("));
        assert!(pushed.contains("ABCDE"));
        assert!(pushed.contains("PHX-1-ABCDE"));
        assert!(pushed.contains("http://192.168.1.5:8080/"));
    }

    #[test]
    fn a_host_with_no_join_service_says_so_rather_than_showing_a_dead_qr() {
        // `--solo`, or no `--rendezvous` (issue #1329 AC2). The crew must not be
        // stood in front of the viewscreen scanning something that can never
        // work, and "no code yet" must not look like "no code ever".
        let lobby = LocalHostLobby::open("0.0.0.0:8080");
        lobby.publish_join(&JoinInvite::Off);

        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&lobby.bridge, &mut surface);
        assert_eq!(
            surface.pushed,
            vec![r#"window.__phoenixHostLobbyJoin('{"kind":"off"}')"#.to_string()]
        );
    }

    #[test]
    fn publishing_serves_the_lobby_at_that_path_and_withdrawing_stops() {
        let documents = HostedDocuments::default();
        let lobby = LocalHostLobby::open("127.0.0.1:8080");
        let page = std::fs::read_to_string("server.html").unwrap();
        lobby.publish(&page, &documents).unwrap();

        let body = documents
            .get(&lobby.path())
            .expect("the document is served");
        assert!(body.contains("id=\"station-grid\""));
        // The OS accessibility default layer every embedded surface gets
        // (issue #1127): an Ultralight view has no matchMedia to read.
        assert!(body.contains("window.PhoenixOsAccessibilityDefaults ="));

        lobby.withdraw(&documents);
        assert!(documents.get(&lobby.path()).is_none());
    }

    #[test]
    fn a_bundle_with_no_lobby_refuses_by_name_instead_of_publishing_a_blank_page() {
        let documents = HostedDocuments::default();
        let lobby = LocalHostLobby::open("127.0.0.1:8080");
        assert_eq!(
            lobby.publish("<html><body></body></html>", &documents),
            Err(HostLobbyDocumentError::NoLobbyPanel)
        );
        assert!(documents.is_empty());
    }
}
