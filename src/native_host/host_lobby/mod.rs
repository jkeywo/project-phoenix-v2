//! The native host's own lobby surface (issue #1325).
//!
//! Launch `phoenix-host --client-dir dist --world <w>` and the viewscreen
//! window shows the crew lobby a browser host shows: the scenario title, the
//! crew counter, the station grid filling in as phones claim seats, the ready
//! badge, the countdown. It is an embedded web view composited onto the
//! viewscreen, fed by the bridge below.
//!
//! Launch it `--lobby` instead and the same surface opens on the scenario
//! picker first (issue #1328) — the host page's own two-stage world-then-hull
//! flow — and moves on to the crew lobby once the world it chose has landed.
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
//! # …and ONE path back, for everything the surface says
//!
//! ```text
//! #scenario-panel click        [gui/host-scenario-render.js, shared with server.html]
//! a monitor or a station <button> is pressed           [gui/host-lobby-render.js built it]
//!   │  HostLobbyRecord         [scenario] a closed vocabulary, NOT a ClientMessage
//!   ▼  phoenixHostLobbyOut.send(json)                  [host_lobby_link.js, delegated]
//! HostLobbyBridge::take_records                        [bridge] DRAINED once a frame
//!   ▼
//! drain_surface_records                                [this file, PreUpdate]
//!   ├─ a pick      → InboundMessage { LOCAL_CONSOLE_TOKEN } → lobby::scenario_arbiter
//!   ├─ an AI launch→ PendingForceStart  → server::bridge::apply_force_start
//!   └─ a layout press (a monitor, or a station's screen)
//!                  → BridgeLayout::apply, over the layout law — ONE arm for all three
//!        ▼  accepted: BridgeLayoutResource moves   refused: a LayoutNotice for the row
//!        ▼
//!      publish_bridge_layout [Update, same frame] → follow_layout_viewscreen
//!                                                 → follow_layout_stations
//! ```
//!
//! **One vocabulary, one drain, one reader**, and that is a correctness
//! constraint rather than tidiness: `take_records` empties what it returns, so a
//! second consumer would swallow the first's records and warn about a language
//! it does not speak while the first saw an empty queue forever — with a clean
//! log at both ends. Issues #1328, #1330 and #1331 each grew this surface a
//! control, and all of them ride the same queue and the same enum.
//!
//! The surface therefore has controls now: which scenario and hull the mission
//! flies, whether to launch it AI-crewed, which display shows the shared view,
//! and which display each station's console opens on. It is still **not a
//! participant** — nothing it sends is a
//! `ClientMessage` and nothing crosses command admission. A pick borrows
//! `LOCAL_CONSOLE_TOKEN`, which is not an identity but a statement that the host
//! operator is acting on the host's own window (the same statement `server.html`
//! makes through `__localConsoleSend`); a layout press asks the host to
//! rearrange **its own screens**, and the layout law is what judges it. Which
//! station's console a screen shows is a layout question and never a question
//! of who may sit at it — that stays the ordinary claim flow a phone goes
//! through.
//!
//! # What is here, and why each piece is where it is
//!
//! | piece | what it decides |
//! |---|---|
//! | [`document`] | what the surface loads: the host page's own `#lobby-panel`, `#qr-panel`, `#scenario-panel` and `#landing-panel`, assembled in memory and served at the host page's own depth |
//! | [`bridge`] | what crosses, in both directions, and what a failed push costs |
//! | [`join`] | what the join panel says, and where its QR points |
//! | [`landing`] | what the front door is told about the process it is embedded in: which build, and whether a World has taken it away |
//! | [`layout`] | the monitor row and the per-station screen rows: the roster and the law's eligibility out, and the layout action a press asks for |
//! | [`packs`] | the mod-pack shelf the landing offers (issue #1366): the panel's whole contents out, and the adapter that fills [`crate::world::mod_pack`]'s injected seams on a host that READS its content rather than fetching it |
//! | [`reveal`] | when the surface is on screen, and when it has yielded |
//! | [`scenario`] | what the picker is shown, and [`HostLobbyRecord`] — everything the surface may say back, the three layout presses included |
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
//! # It talks back, since issue #1328
//!
//! The surface was read-only through #1325 and #1329: it rendered what the host
//! was already broadcasting and sent nothing. It now carries the scenario and
//! hull pick and the AI launch, so `phoenix-host --client-dir dist --lobby`
//! boots to a picker an operator can actually use — and, since #1330, the
//! monitor row beside it, and since #1331 a screen row per station. The path
//! all of them take is the one drawn above.
//!
//! # The surface is permanent
//!
//! It is created once and never torn down. On mission start the *chrome*
//! yields — see [`reveal`] — and one host key ([`HOST_LOBBY_REVEAL_KEY`])
//! brings it back. The join QR arrived on this same surface in issue #1329 and
//! the monitor row in issue #1330; the settings rows follow, so nothing
//! downstream should learn to rebuild it.
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
pub mod fullscreen;
pub mod join;
pub mod landing;
pub mod layout;
pub mod packs;
pub mod reveal;
pub mod scenario;

use bevy::prelude::*;

use crate::console_bridge::{LobbyStateChanged, LOCAL_CONSOLE_TOKEN};
use crate::core::messages::{ClientMessage, GamePhase};
use crate::delivery::serve::HostedDocuments;
use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::bridge_display::BridgeLayoutResource;
use crate::native_host::bridge_layout::LayoutAction;
use crate::native_host::panes::document::{connectable_host_addr, mint_document_nonce};
use crate::native_host::panes::PaneId;
use crate::native_host::world_load::{
    published_catalog, LobbyBootSettings, LobbyScenarioCatalog, LobbySelection,
};

pub use bridge::{pump_host_lobby, HostLobbyBridge, HostLobbyPumpReport};
pub use document::{build_host_lobby_document, host_lobby_drain_script, HostLobbyDocumentError};
pub use fullscreen::{monitor_to_restore, next_window_mode, WindowModeToggle};
pub use join::{
    join_addr_reach, phone_rendezvous, JoinAddrReach, JoinInvite, PhoneRendezvous,
    CLIENT_DEFAULT_RENDEZVOUS,
};
pub use landing::{LandingPanelPayload, BUILD_ID};
pub use layout::{
    assign_station_action, bridge_layout_payload, set_viewscreen_action, unassign_station_action,
    BridgeLayoutPayload, LayoutNotice, StationRowPayload, StationScreenPayload,
};
pub use packs::{
    InstallOutcome, InstalledPack, ModPackPanelPayload, ModPackShelfResource, OfferedPack,
    PackConflict, PackFinding,
};
pub use reveal::{RevealState, SurfacePresence};
pub use scenario::{HostLobbyRecord, ScenarioPanelPayload};

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
    /// The `--rendezvous` base, verbatim, or `None` for a host whose only
    /// ingress is its own port.
    pub rendezvous: Option<String>,
    /// The code this host minted for itself, when it accepts LAN joins directly
    /// (issue #1353) — and, when it is `Some`, the only code the viewscreen's QR
    /// will ever show.
    ///
    /// A host can run BOTH legs, and then two services have each issued it a
    /// code: this process's own, and the cloud worker's. They are not
    /// interchangeable, because the QR does not only carry a code — it carries
    /// the PAGE, and the page a LAN phone loads is served by this host, so the
    /// service it dials is this host (see `gui/join-url.js`'s origin rule).
    /// Putting the worker's code on that QR would send every phone in
    /// the room to a service that has never heard of them. The cloud code is
    /// still printed to the operator's terminal, which is where somebody
    /// arranging internet play reads it from.
    pub direct: Option<crate::core::rendezvous::JoinCode>,
}

impl HostLobbyJoinResource {
    /// Read the surface's own answer to "where should a phone be sent".
    pub fn from_lobby(lobby: &LocalHostLobby, rendezvous: Option<&str>) -> Self {
        Self {
            join_base: lobby.join_base.clone(),
            rendezvous: rendezvous.map(str::to_string),
            direct: None,
        }
    }

    /// …for a host that also accepts LAN joins itself (issue #1353).
    pub fn with_direct_code(mut self, code: crate::core::rendezvous::JoinCode) -> Self {
        self.direct = Some(code);
        self
    }

    /// The invitation an issued code makes.
    ///
    /// The `rendezvous` a directly-joinable host puts in the URL is `None`
    /// deliberately, whatever `--rendezvous` says: the client's rule is that a
    /// page not served from a known browser-game web origin dials the origin it
    /// was served from, so the QR needs no parameter at all — which is also a
    /// shorter QR and one fewer injection surface.
    pub fn invite(&self, code: &crate::core::rendezvous::JoinCode) -> JoinInvite {
        let rendezvous = if self.direct.is_some() {
            None
        } else {
            self.rendezvous.as_deref()
        };
        JoinInvite::from_code(code, &self.join_base, rendezvous)
    }

    /// The invitation for a code the relay legs have just issued — or `None`
    /// when this code is not the one the viewscreen's QR belongs to.
    pub fn viewscreen_invite(
        &self,
        code: &crate::core::rendezvous::JoinCode,
    ) -> Option<JoinInvite> {
        match &self.direct {
            Some(direct) if direct.full != code.full => None,
            _ => Some(self.invite(code)),
        }
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
    pub gm_bridge: super::native_gm::bridge::NativeGmBridge,
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
            gm_bridge: super::native_gm::bridge::NativeGmBridge::default(),
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

    /// The URL the viewscreen HUD-overlay surface navigates to (issue #422's
    /// `#hud-overlay`, ported to the native path). Served from the same host at
    /// `/gui/viewscreen-hud.html`, dialled at the same connectable address the
    /// lobby surface uses.
    pub fn viewscreen_hud_url(&self) -> String {
        document::viewscreen_hud_url(&self.host_addr)
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
        documents.publish(
            super::native_gm::document::document_path(&self.nonce),
            super::native_gm::document::build_document(host_index_html),
        );
        Ok(())
    }

    /// Stop publishing the document.
    ///
    /// Called at shutdown, before the delivery thread is joined, so the window
    /// in which this process is still serving a bridge surface nobody is
    /// driving is empty rather than merely short.
    pub fn withdraw(&self, documents: &HostedDocuments) {
        documents.withdraw(&self.path());
        documents.withdraw(&super::native_gm::document::document_path(&self.nonce));
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
        app.init_resource::<HostLobbyRevealResource>()
            // The fullscreen control's latch (issue #1367). Initialised here
            // rather than by whatever opens a window, so a press has somewhere
            // to land on every composition that carries a lobby surface —
            // `apply_window_mode_toggle` is what decides a host with no window
            // cannot answer it, in one place, rather than the dispatcher
            // guessing from the absence of a resource.
            .init_resource::<fullscreen::WindowModeToggle>()
            .add_systems(
                Update,
                (
                    feed_lobby_state,
                    feed_landing_panel,
                    // The window mode a press in `PreUpdate` asked for
                    // (issue #1367). Early in the chain with the other
                    // answers, and before the feeds, for the reason
                    // `apply_mod_pack_choice` is: the operator's press is
                    // answered in the frame it arrived rather than the next.
                    fullscreen::apply_window_mode_toggle,
                    // The mod-pack install an operator asked for in `PreUpdate`
                    // (issue #1366), answered BEFORE the two feeds below it —
                    // deliberately, and it is the same one-frame edge the
                    // monitor row's note at the bottom of this chain describes.
                    // An accepted pack widens `LobbyScenarioCatalog`, which
                    // `feed_scenario_panel` publishes, and moves the shelf,
                    // which `feed_mod_pack_shelf` publishes; run after them and
                    // the operator would look at the old catalogue for a frame
                    // and at a shelf that had not heard about their press.
                    apply_mod_pack_choice,
                    feed_mod_pack_shelf,
                    feed_scenario_panel,
                    observe_phase,
                    // `ButtonInput<KeyCode>` exists only where `InputPlugin`
                    // does, and `NativeRenderSurface::Contract` stands one up
                    // nowhere. Bevy validates a system's parameters when it
                    // RUNS, so a bare `Res<ButtonInput<_>>` there is a panic
                    // rather than an inert system — the same trap
                    // `PaneDisplayPlugin` gates against.
                    toggle_reveal_key.run_if(resource_exists::<ButtonInput<KeyCode>>),
                    drain_client_qr_toggle,
                    publish_reveal,
                    // The monitor row's other half (issue #1330). The layout a
                    // press moved was moved in `PreUpdate` below, so this reads
                    // the result in the SAME frame — moved, or unmoved with a
                    // refusal to show for it — and publishes it back. That
                    // one-frame edge is what makes a refusal something the
                    // operator sees rather than something the log knows.
                    publish_bridge_layout,
                )
                    .chain(),
            )
            // The surface's own presses (issues #1328/#1330), in `PreUpdate` —
            // where every other participant's input enters the app
            // (`transport::drain_native_inbound`, and `drain_inbound` on wasm),
            // and therefore before the fixed loop that arbitrates them runs.
            //
            // ONE system, because the queue it reads is a drain: see
            // [`drain_surface_records`].
            .add_systems(PreUpdate, drain_surface_records);
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

/// Carry the scenario picker's state to the surface (issue #1328).
///
/// The viewscreen's picker renders from `gui/host-scenarios.js` +
/// `gui/host-scenario-render.js` — the browser host's own two modules — so what
/// has to cross is exactly `scenarioCatalogView`'s three arguments. They are
/// built by [`published_catalog`], which is the same call that builds the
/// `ScenarioCatalog` message every phone in the room folds: one derivation, two
/// audiences, and no way for the viewscreen and the phones to be shown different
/// catalogues.
///
/// # When it pushes, and why not every frame
///
/// Three moments, and nothing between them:
///
///  * the first frame, so a `--lobby` host puts its picker on screen without
///    waiting for anybody to touch it (the surface sends no `Identify` and so
///    gets no greeting the way a phone does);
///  * whenever [`LobbySelection`] moved — a lock, or the reset
///    `world_load::unwind_failed_load` performs after a refused world;
///  * whenever [`LobbyScenarioCatalog`] moved, which since issue #1366 it can:
///    an accepted mod pack widens it, and a picker still offering the catalogue
///    the host booted with would be the one screen in the room that had not
///    heard about the pack the operator just installed;
///  * the frame a [`WorldConfig`](crate::world::config::WorldConfig) lands,
///    which is what closes the picker for good.
///
/// A **refused** pick moves nothing, so it pushes nothing, and the picker stays
/// exactly where it was — which is precisely what the host page does with one
/// (`arbiterSelectScenario` returns before its render on any non-accepted
/// outcome). The refusal itself is not silent: `drain_scenario_selection` logs
/// it at warn level, on both hosts, through the same `log_outcome`.
///
/// A `--world` host has no [`LobbyScenarioCatalog`] and therefore never pushes
/// at all, which is what leaves its `#scenario-panel` at the `display: none`
/// [`document`] assembled it with — the flag that decides the world skips the
/// stage it decides.
fn feed_scenario_panel(
    bridge: Option<Res<HostLobbyBridgeResource>>,
    catalog: Option<Res<LobbyScenarioCatalog>>,
    selection: Option<Res<LobbySelection>>,
    settings: Option<Res<LobbyBootSettings>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut published: Local<bool>,
) {
    let (Some(bridge), Some(catalog), Some(selection)) = (bridge, catalog, selection) else {
        return;
    };
    let moved = !*published
        || selection.is_changed()
        || catalog.is_changed()
        || world_config.as_ref().is_some_and(|w| w.is_added());
    if !moved {
        return;
    }
    *published = true;
    let pinned = settings.as_ref().and_then(|s| s.ship_path.as_deref());
    let payload =
        published_catalog(&catalog.0, &selection.0, pinned).surface(world_config.is_some());
    bridge.0.push_scenario(payload.to_json());
}

/// Carry the landing screen's state to the surface (issue #1361).
///
/// The landing renders from `gui/host-landing-view.js` +
/// `gui/host-landing-render.js` — the browser host's own two modules, written
/// as shared ones in #1360 for exactly this — so what has to cross is only what
/// the page cannot know for itself: which build this binary is, and whether a
/// World has been committed. See [`landing`] for why those two and nothing
/// else.
///
/// # When it pushes, and why that decides the whole feature
///
/// The same trigger set as [`feed_scenario_panel`], deliberately, because the
/// landing and the picker answer the same question at two depths:
///
///  * the first frame, which is what puts the front door on the viewscreen at
///    all — the document assembles `#landing-panel` at `display: none`;
///  * the frame a [`WorldConfig`](crate::world::config::WorldConfig) lands,
///    which dismisses it for good.
///
/// And, exactly as the picker's feed does, it requires a
/// [`LobbyScenarioCatalog`] — a resource only a **world-less** host carries. So
/// a `phoenix-host --world …` never pushes, never leaves `display: none`, and
/// never shows a menu asking a question it was answered at the prompt. That is
/// the acceptance criterion "the native Viewscreen shows the landing on a host
/// started with no World", and it is the absence of a push rather than a check.
///
/// Nothing here has an opinion about which ENTRY is open. That is
/// `nextOpenEntry`'s answer over the page's own memory, on both surfaces; a
/// copy of it in Rust would be a second authority on a decision #1360 made pure
/// so it could be tested without a document.
fn feed_landing_panel(
    bridge: Option<Res<HostLobbyBridgeResource>>,
    catalog: Option<Res<LobbyScenarioCatalog>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut published: Local<bool>,
) {
    let (Some(bridge), Some(_catalog)) = (bridge, catalog) else {
        return;
    };
    let dismissed = world_config.is_some();
    let moved = !*published || world_config.as_ref().is_some_and(|w| w.is_added());
    if !moved {
        return;
    }
    *published = true;
    bridge
        .0
        .push_landing(landing::LandingPanelPayload::new(dismissed).to_json());
}

/// Carry the mod-pack shelf to the surface (issue #1366).
///
/// The landing's Load-mod-pack stage renders from `gui/host-landing-view.js` +
/// `gui/host-landing-render.js` — the same shared pair the menu around it comes
/// from — so what crosses is one snapshot of the whole panel:
/// [`packs::ModPackPanelPayload`].
///
/// # It pushes exactly when the shelf moved, and that is the whole rule
///
/// The first frame, so a host given `--mod-pack-dir` puts its shelf on the
/// landing without waiting for anybody to touch it, and then whenever
/// [`packs::ModPackShelfResource`] changed — which is precisely when an install
/// attempt finished, because nothing else writes it.
///
/// A host given no `--mod-pack-dir` carries no such resource and therefore never
/// pushes, which leaves the landing's Load-mod-pack row exactly as inert as
/// issue #1360 shipped it. That is not an omission: the row's stage opens only
/// on a surface that says it can answer the row (`provides: ['packs']` in
/// `host_lobby_link.js`, against the `needs: 'packs'` on the row itself), and a
/// surface with no shelf never says so. One rule — a control exists exactly when
/// something behind it answers it — said in data on both sides of the bridge.
fn feed_mod_pack_shelf(
    bridge: Option<Res<HostLobbyBridgeResource>>,
    shelf: Option<Res<packs::ModPackShelfResource>>,
    mut published: Local<bool>,
) {
    let (Some(bridge), Some(shelf)) = (bridge, shelf) else {
        return;
    };
    if *published && !shelf.is_changed() {
        return;
    }
    *published = true;
    bridge.0.push_packs(shelf.payload().to_json());
}

/// Answer the pack an operator chose off the shelf (issue #1366).
///
/// The other half of [`drain_surface_records`]'s latch, and the only place in
/// this crate that turns a name off the bridge into an open file. Four steps,
/// and only the first two are this slice's:
///
///  1. **the shelf lookup**, which is the gate. The name is looked up in the
///     shelf THIS host produced ([`crate::native_host::mod_packs::offered`]) and
///     never joined onto the scanned directory, so "never offered", "invented by
///     a page newer than this binary" and "deleted since the scan" are one
///     refusal — and a traversal is unrepresentable rather than merely rejected.
///  2. **the read**, whose failure is reported in exactly the shape a validator
///     finding takes, because from where the operator is standing a pack that
///     could not be opened and a pack that would not validate are the same
///     event: they chose it and it did not go in.
///  3. **the existing validation and overlay, unchanged.**
///     [`packs::install_pack`] is a seam-filling wrapper over
///     [`crate::world::mod_pack::validate_mod_pack`] and
///     [`crate::entities::config_cache::push_mod_pack`] — the same two calls
///     `bridge::wasm_add_mod_pack` makes for a browser upload, with the same
///     atomic acceptance: on any error nothing is installed and the findings say
///     what is wrong.
///  4. **the catalogue**, rebuilt through
///     [`ManifestSource::merged_catalog`](crate::delivery::serve::ManifestSource::merged_catalog),
///     which is the same `build_merged_catalog` call `wasm_get_scenario_catalog`
///     makes and which that method's own comment named as "the seam a native
///     mod-pack path would land on". This is that path landing on it. The result
///     goes to [`LobbyScenarioCatalog`] — which [`feed_scenario_panel`] then
///     publishes to the viewscreen — and one message goes to every phone in the
///     room, so a pack cannot widen only the screen the operator is looking at.
///
/// The shelf is **rescanned on every attempt**, accepted or not: a bridge
/// machine's mod folder is exactly the sort of place somebody drops a file into
/// while the host is already up, and a list that only ever reflected boot would
/// make that invisible.
///
/// `Update`, ahead of [`feed_scenario_panel`] and [`feed_mod_pack_shelf`] in the
/// plugin's chain, so an accepted pack's widened catalogue and the panel's
/// answer both reach the surface in the frame the operator pressed — the same
/// one-frame edge that makes a layout refusal something they see rather than
/// something only the log knows.
fn apply_mod_pack_choice(
    shelf: Option<ResMut<packs::ModPackShelfResource>>,
    catalog: Option<ResMut<LobbyScenarioCatalog>>,
    selection: Option<Res<LobbySelection>>,
    settings: Option<Res<LobbyBootSettings>>,
    outbox: Option<ResMut<crate::lobby::LobbyOutbox>>,
    log: Option<Res<LogFilterConfig>>,
) {
    let Some(mut shelf) = shelf else {
        return;
    };
    // Read through the immutable deref FIRST: touching `ResMut` marks the
    // resource changed, and `feed_mod_pack_shelf` pushes on exactly that signal.
    // A frame with nothing to install must cost the surface nothing.
    if shelf.pending.is_none() {
        return;
    }
    let Some(file) = shelf.pending.take() else {
        return;
    };
    shelf.attempted = Some(file.clone());

    let outcome = match crate::native_host::mod_packs::offered(&shelf.shelf, &file) {
        None => packs::InstallOutcome {
            accepted: false,
            findings: vec![packs::PackFinding::error(
                "unknown-pack",
                &file,
                format!(
                    "{file} is not on this host's mod-pack shelf — it may have been removed \
                     since the folder was scanned"
                ),
            )],
        },
        Some(pack) => {
            let path = shelf.dir.join(&pack.file);
            match std::fs::read(&path) {
                Err(e) => packs::InstallOutcome {
                    accepted: false,
                    findings: vec![packs::PackFinding::error(
                        "unreadable-archive",
                        &pack.file,
                        format!("{} could not be read: {e}", path.display()),
                    )],
                },
                Ok(bytes) => {
                    let root = std::path::PathBuf::from(&shelf.content_dir);
                    let manifest_toml =
                        std::fs::read_to_string(root.join(&shelf.manifest_rel)).unwrap_or_default();
                    packs::install_pack(
                        &bytes,
                        &manifest_toml,
                        // The native `resolve_base` seam. Base content here is
                        // READ, not fetched, so the browser's preload cache
                        // (`cached_base_world_source`) is a `None` stub off
                        // wasm by construction. Rooted at `--content-dir`, which
                        // is the one root `ManifestSource` resolves a world
                        // against, so a pack is judged against the very content
                        // this host serves and not against a second reading of
                        // it.
                        |authored| std::fs::read_to_string(root.join(authored)).ok(),
                        // Authoritative, and safe to pass: `validate_mod_pack`
                        // wraps it in `PackTemplates`, which serves the
                        // candidate's own hulls in front of it (issue #973's
                        // review, F3). A native host IS authoritative about its
                        // content tree, so a pack naming a hull nothing carries
                        // is caught here rather than at spawn.
                        &crate::entities::loader::FsTemplateLoader,
                    )
                }
            }
        }
    };

    for finding in &outcome.findings {
        crate::pwarn!(
            log,
            LogCat::Lobby,
            "host lobby: mod pack {file} [{}] {}: {}",
            finding.severity,
            finding.category,
            finding.message
        );
    }
    shelf.accepted = outcome.accepted;
    shelf.findings = outcome.findings;
    // Whatever happened, the folder is read again: the operator is about to look
    // at this list, and it should be the folder as it is now.
    shelf.rescan();

    if !shelf.accepted {
        crate::pwarn!(log, LogCat::Lobby, "host lobby: mod pack {file} refused");
        return;
    }
    crate::pinfo!(
        log,
        LogCat::Lobby,
        "host lobby: mod pack {file} installed; rebuilding the lobby catalogue"
    );

    let Some(mut catalog) = catalog else {
        return;
    };
    match crate::delivery::serve::ManifestSource::read(&shelf.content_dir, &shelf.manifest_rel) {
        Err(e) => crate::pwarn!(
            log,
            LogCat::Lobby,
            "host lobby: mod pack {file} is installed, but this host's own manifest could not \
             be re-read to widen the catalogue: {e}"
        ),
        Ok(source) => {
            let merged = source.merged_catalog();
            for finding in &merged.findings {
                crate::pwarn!(
                    log,
                    LogCat::Lobby,
                    "host lobby: merged catalogue [{}] {}: {}",
                    finding.category,
                    finding.source.reference,
                    finding.message
                );
            }
            catalog.0 = merged.catalog;
            // …and every phone in the room hears it too. The viewscreen gets the
            // same catalogue from `feed_scenario_panel`, out of this same
            // resource, in this same frame — one derivation, two audiences,
            // which is the invariant that would otherwise break the moment a
            // pack widened only the screen the operator is standing in front of.
            if let (Some(mut outbox), Some(selection)) = (outbox, selection) {
                let pinned = settings.as_ref().and_then(|s| s.ship_path.as_deref());
                outbox.0.push((
                    crate::lobby::handler::Target::All,
                    published_catalog(&catalog.0, &selection.0, pinned).wire(),
                ));
            }
        }
    }
}

/// Carry what the operator pressed on the viewscreen back into the host.
///
/// The other half of [`feed_scenario_panel`] and of [`publish_bridge_layout`],
/// and the reason this surface stopped being read-only. Records arrive as
/// [`HostLobbyRecord`] — a closed vocabulary, not `ClientMessage`s, because the
/// surface holds no session token — and leave as five different things:
///
///  * a pick becomes an `InboundMessage` under
///    [`LOCAL_CONSOLE_TOKEN`](crate::console_bridge::LOCAL_CONSOLE_TOKEN), which
///    is exactly what `server.html` submits its OWN picker's picks under
///    (`__localConsoleSend`, issue #822). From
///    [`crate::lobby::scenario_arbiter`]'s point of view the host page's picker
///    and this one are the same sender — the host operator, acting on the host's
///    own window — and the arbiter's first-valid-wins rule treats them as one
///    participant among the phones, with no priority. The token is reserved
///    (`lobby::handler::is_reserved_token`) so no network peer can impersonate
///    it, and the two selection variants are explicit no-ops in the lobby
///    handler, so nothing but the arbiter acts on them.
///  * an AI launch sets
///    [`PendingForceStart`](crate::server::bridge::PendingForceStart), which
///    `server::bridge::apply_force_start` reads on the next fixed step. Not a
///    message, because it is not a participant's command: it is the same
///    host-side latch the browser button sets, and the rule that answers it is
///    the same rule.
///  * a layout press — a monitor for the viewscreen (issue #1330), a screen for
///    a station's console or the "off" that closes it (issue #1331) — goes
///    straight through
///    [`BridgeLayout::apply`](crate::native_host::bridge_layout::BridgeLayout::apply)
///    onto the **live** [`BridgeLayoutResource`], so the lobby has no rule of its
///    own: a monitor unplugged between the click and this frame is a
///    `LayoutRefusal::UnknownMonitor` here exactly as a hand-authored profile
///    naming it would be at boot. All three verbs share ONE arm below, because
///    all that differs between them is the [`LayoutAction`] they name. A refusal
///    is **shown**, not only logged — "the lobby never silently ignores me" is
///    the user story, and a press that changed nothing with a clean log is
///    indistinguishable from a broken button — so it goes back as a
///    [`LayoutNotice`] and `publish_bridge_layout` carries it in this same
///    frame.
///
///  * the confirmed Exit to Desktop (issue #1365) leaves as `AppExit::Success`
///    — the ordinary application-exit message, written from a system exactly as
///    `bridge_display::setup_enumerate` writes it to end `--setup`. Nothing
///    here withdraws a document or stops the delivery service: the runner
///    returns, and the tail of `phoenix_host::main` does all of that unchanged,
///    which is the whole point of ending the run through the door that already
///    exists rather than opening a second one beside it.
///
///  * a mod pack chosen off the shelf (issue #1366) leaves as a LATCH on
///    [`packs::ModPackShelfResource`], answered by [`apply_mod_pack_choice`] in
///    `Update` — for the reason the AI launch sets one rather than acting here:
///    this system is a dispatcher over a drained queue, and an arm that read a
///    directory, opened an archive and rebuilt the scenario catalogue would make
///    every other record in the batch wait behind a disk read. On a host started
///    without `--mod-pack-dir` there is no such resource and the record is a
///    line in the log, exactly as an unknown one is.
///
///  * a landing-menu press (issue #1361) leaves as a LINE IN THE LOG, and that
///    is the whole of it in this slice. It is worth saying why rather than
///    leaving it to look unfinished: the one route the menu opens today is New
///    Game, which reveals the `#scenario-panel` this surface already carries —
///    so its picks reach [`crate::lobby::scenario_arbiter`] and
///    `world_load::apply_pending_world_load` down exactly the path they took
///    before the landing existed, and a host arm that "started a game" would be
///    a second world-load path beside the one that works. What the record buys
///    is that the process can say what its own viewscreen is showing, on a
///    surface nobody can attach a console to, and that the entries which DO
///    need a host answer — #1365's Exit to Desktop, #1366's mod packs, #1367's
///    settings — extend one vocabulary instead of opening a second queue.
///
/// **Nothing here opens a window or a pane.** An accepted press only moves the
/// layout; `bridge_display::follow_layout_stations` is what notices the new
/// arrangement and opens or closes the console (issue #1331), for the same
/// reason `follow_layout_viewscreen` owns the window: a reconcile after an
/// unplug produces the same transitions with nobody pressing anything, and two
/// places that opened consoles would disagree the first time one of them ran.
///
/// `pub(crate)` for one reason: it is one of the writers of
/// [`BridgeLayoutResource::notices`](crate::native_host::bridge_display::BridgeLayoutResource::notices),
/// and the test that pins the two of them landing in the SAME frame has to
/// register both in one schedule in a known order — see `bridge_display`'s
/// `a_press_and_a_seat_surrender_in_one_frame_both_reach_the_row`. Nothing
/// outside a test calls it; `HostLobbyPlugin` is how it runs.
///
/// # One system, because the queue is a drain
///
/// `HostLobbyBridge::take_records` empties what it returns, so every kind of
/// record the surface can send is dispatched HERE rather than by a system of its
/// own per kind. Two readers would not fail loudly: the first to run would
/// swallow the other's records and warn about a vocabulary it does not speak,
/// and the second would find an empty queue every frame for the rest of the run.
///
/// # `PreUpdate`, and what that buys the row
///
/// The picks have to be in before the fixed loop that arbitrates them. The
/// layout half gets something else out of it for free: the applied layout is
/// visible to `publish_bridge_layout` in `Update` of the **same** frame, so a
/// press and the row that answers it are one repaint rather than two.
///
/// `PendingForceStart` and `BridgeLayoutResource` are `Option` because a
/// headless-shaped composition, and a host whose winit has not enumerated its
/// displays yet, may not carry them. The picks are written unconditionally,
/// because a host with a lobby surface always has the message bus. Records are
/// drained either way, so a surface talking to a host that cannot answer it does
/// not sit on a queue for the whole mission.
pub(crate) fn drain_surface_records(
    native_gm: Option<Res<super::native_gm::NativeGmLifecycle>>,
    phase: Option<Res<State<GamePhase>>>,
    bridge: Option<Res<HostLobbyBridgeResource>>,
    mut inbound: MessageWriter<crate::lobby::InboundMessage>,
    force_start: Option<ResMut<crate::server::bridge::PendingForceStart>>,
    layout: Option<ResMut<BridgeLayoutResource>>,
    shelf: Option<ResMut<packs::ModPackShelfResource>>,
    window_mode: Option<ResMut<fullscreen::WindowModeToggle>>,
    mut exit: MessageWriter<AppExit>,
    log: Option<Res<LogFilterConfig>>,
    bus: Option<Res<super::panes::PaneBusResource>>,
    mut sessions: Option<ResMut<crate::lobby::Sessions>>,
    mut assignments: Option<ResMut<super::console_assignment::ConsoleAssignments>>,
    mut pending_save: Option<ResMut<super::layout_store_systems::PendingLayoutSave>>,
) {
    let Some(bridge) = bridge else {
        return;
    };
    let records = bridge.0.take_records();
    if records.is_empty() {
        return;
    }
    let mut force_start = force_start;
    let mut layout = layout;
    let mut shelf = shelf;
    let mut window_mode = window_mode;
    // Within this batch it replaces rather than accumulates: the row shows what
    // happened to the last thing the operator did, so an accepted press clears
    // the refusal the one before it earned. What lands on the RESOURCE is
    // appended (see the write at the end). `None` until a layout press is
    // actually seen, so a frame carrying only picks does not touch the layout
    // resource and mark it changed.
    let mut layout_notices: Option<Vec<LayoutNotice>> = None;
    for raw in records {
        let Some(record) = HostLobbyRecord::decode(&raw) else {
            // The only sender is a document this process assembled and serves,
            // so this is a bug in that document rather than input to validate —
            // and a bug on a surface nobody can attach a console to is a bug
            // that has to reach the operator log. Deliberately NOT a refusal to
            // show on the row either: a record this build cannot parse is a
            // page/host mismatch, which is an operator's problem and not
            // something to render as an answer to a press.
            crate::pwarn!(
                log,
                LogCat::Lobby,
                "host lobby: the surface sent something this bridge does not speak: {raw}"
            );
            continue;
        };
        // The three layout verbs fall through to ONE arm, below. What separates
        // them is only which [`LayoutAction`] they name; everything after that
        // — the law that judges it, the notice a refusal earns, the line the
        // operator log gets — is one path, so a station press cannot be judged
        // by a different rule from a monitor press (issue #1331).
        let action = match record {
            HostLobbyRecord::SelectScenario { scenario_id } => {
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "host lobby: scenario picked: {scenario_id}"
                );
                inbound.write(crate::lobby::InboundMessage {
                    token: LOCAL_CONSOLE_TOKEN.to_string(),
                    msg: ClientMessage::SelectScenario { scenario_id },
                });
                continue;
            }
            HostLobbyRecord::SelectShip { template_path } => {
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "host lobby: hull picked: {template_path}"
                );
                inbound.write(crate::lobby::InboundMessage {
                    token: LOCAL_CONSOLE_TOKEN.to_string(),
                    msg: ClientMessage::SelectPlayerShip { template_path },
                });
                continue;
            }
            HostLobbyRecord::ForceStart => {
                crate::pinfo!(log, LogCat::Lobby, "host lobby: AI launch requested");
                if let Some(pending) = force_start.as_mut() {
                    pending.0 = true;
                }
                continue;
            }
            HostLobbyRecord::LandingOpen { entry } => {
                crate::pinfo!(log, LogCat::Lobby, "host lobby: landing opened: {entry}");
                continue;
            }
            HostLobbyRecord::LandingClose => {
                crate::pinfo!(log, LogCat::Lobby, "host lobby: landing closed");
                continue;
            }
            HostLobbyRecord::ExitDesktop => {
                // Issue #1365, and it is the ORDINARY application exit and
                // nothing else: `AppExit::Success` is what
                // `bridge_display::setup_enumerate` writes to end `--setup`,
                // and it ends the run through the same door — the runner
                // returns, `native_host::run` returns, and the tail of
                // `phoenix_host::main` withdraws the hosted documents, stops
                // the delivery service and joins its thread exactly as it does
                // when the operator closes the window. Nothing here withdraws
                // or stops anything itself; a second teardown path would be a
                // second thing to keep in step with that one.
                //
                // No re-confirmation. The surface already asked, on a screen
                // the operator is looking at, and a host that asked again would
                // be second-guessing an answer it can see was given.
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "host lobby: exit to desktop confirmed; shutting the host down"
                );
                exit.write(AppExit::Success);
                continue;
            }
            HostLobbyRecord::InstallModPack { pack } => {
                // Issue #1366, and a LATCH rather than the work itself, for the
                // reason `ForceStart` above sets one: this system is a
                // dispatcher over a drained queue, and an arm that read a
                // directory, opened an archive, ran the whole composition
                // validation and rebuilt the scenario catalogue would make every
                // other record in the same batch wait behind a disk read.
                // `apply_mod_pack_choice` answers it in `Update`, in time for
                // `feed_scenario_panel` to publish the widened catalogue in the
                // SAME frame.
                //
                // Nothing here judges the name. That is the shelf lookup's job
                // (`native_host::mod_packs::offered`), and it is the one gate
                // between a string off a bridge and a file this process opens.
                match shelf.as_mut() {
                    Some(shelf) => {
                        crate::pinfo!(
                            log,
                            LogCat::Lobby,
                            "host lobby: mod pack chosen from the shelf: {pack}"
                        );
                        shelf.pending = Some(pack);
                    }
                    None => crate::pwarn!(
                        log,
                        LogCat::Lobby,
                        "host lobby: a mod pack ({pack}) was chosen on a host started without                          --mod-pack-dir, so there is no shelf to take it from; it is dropped"
                    ),
                }
                continue;
            }
            HostLobbyRecord::ToggleFullscreen => {
                // Issue #1367, and a LATCH rather than the work itself, for the
                // reason `ForceStart` above sets one: this system is a
                // dispatcher over a drained queue and most compositions that run
                // it carry no window at all. `fullscreen::apply_window_mode_toggle`
                // answers it in `Update` of the same frame, where the primary
                // window is legible and its absence is one warning rather than a
                // system that cannot run.
                match window_mode.as_mut() {
                    Some(toggle) => {
                        crate::pinfo!(
                            log,
                            LogCat::Lobby,
                            "host lobby: fullscreen toggle requested from the landing"
                        );
                        toggle.pending = true;
                    }
                    None => crate::pwarn!(
                        log,
                        LogCat::Lobby,
                        "host lobby: the fullscreen control was pressed on a host that carries \
                         no window-mode latch; it is dropped"
                    ),
                }
                continue;
            }
            HostLobbyRecord::SetGameMaster { monitor } => {
                crate::native_host::bridge_layout::LayoutAction::SetGameMaster {
                    monitor: monitor.map(crate::native_host::bridge_profile::MonitorIdentity::new),
                }
            }
            HostLobbyRecord::SetViewscreen { monitor } => layout::set_viewscreen_action(monitor),
            HostLobbyRecord::AssignStation { station, monitor } => {
                layout::assign_station_action(station, monitor)
            }
            HostLobbyRecord::UnassignStation { station } => {
                layout::unassign_station_action(station)
            }
        };
        let Some(layout) = layout.as_mut() else {
            crate::pwarn!(
                log,
                LogCat::Lobby,
                "host lobby: a layout action ({action:?}) arrived before this host had a bridge \
                 layout to apply it to; it is dropped"
            );
            continue;
        };
        let notices = layout_notices.get_or_insert_with(Vec::new);
        if let LayoutAction::AssignStation { station, .. } = &action {
            if let Some(holder) = sessions.as_ref().and_then(|s| {
                s.0.players().iter().find(|p| {
                    p.connected
                        && p.station.as_ref() == Some(station)
                        && s.0.native_station_for_token(&p.token) != Some(station)
                })
            }) {
                notices.push(LayoutNotice::StationHeld {
                    station: station.clone(),
                    holder: holder.name.clone(),
                });
                continue;
            }
        }
        let result = if matches!(&action, crate::native_host::bridge_layout::LayoutAction::SetGameMaster { monitor } if monitor.is_some() != native_gm.as_ref().map_or(layout.layout.game_master_monitor().is_some(), |gm| gm.enabled))
            && phase
                .as_ref()
                .is_some_and(|phase| phase.get() != &GamePhase::Lobby)
        {
            Err(crate::native_host::bridge_layout::LayoutRefusal::GameMasterRoleFrozen)
        } else {
            layout.layout.apply(&action)
        };
        match result {
            Ok(next) => {
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "host lobby: {}",
                    applied(&action, &next)
                );
                layout.layout = next;
                notices.clear();
                match &action {
                    LayoutAction::AssignStation { station, monitor } => {
                        if let Some(assignments) = assignments.as_mut() {
                            assignments.0.insert(station.clone(), monitor.clone());
                        }
                        if let Some(bus) = &bus {
                            bus.0.reserve_console(&station.0);
                        }
                    }
                    LayoutAction::UnassignStation { station } => {
                        let released = assignments
                            .as_mut()
                            .is_some_and(|assignments| assignments.0.remove(station).is_some());
                        if let Some(pending_save) = pending_save.as_mut() {
                            pending_save.0 |= released;
                        }
                        if let Some(bus) = &bus {
                            bus.0.release_console(&station.0);
                        }
                    }
                    _ => {}
                }
                if let (Some(sessions), Some(bus)) = (sessions.as_mut(), &bus) {
                    sessions
                        .0
                        .set_native_station_assignments(bus.0.console_assignments());
                }
            }
            Err(refusal) => {
                crate::pwarn!(log, LogCat::Lobby, "host lobby: {refusal}");
                notices.push(LayoutNotice::Refused(refusal));
            }
        }
    }
    if let (Some(layout), Some(notices)) = (layout.as_mut(), layout_notices) {
        // APPENDED, not assigned. This system and `bridge_display`'s reconcilers
        // both write here — in two chains that are unordered relative to each
        // other — so an assignment made this press the last writer some frames
        // and the display reconcilers the last writer on others. The frame where
        // that decides something is the frame worth reporting: a press racing a
        // `ConsoleCouldNotOpen` surrender erased the one notice
        // `reconcile_seated_consoles` exists to deliver. `publish_bridge_layout`
        // drains what it has pushed, which is what keeps this list "owed" rather
        // than "everything that ever happened". See `BridgeLayoutResource::notices`.
        layout.notices.extend(notices);
    }
}

/// Carry a phone's join-QR toggle to the surface (issue #1329).
///
/// The other end of a button a browser host answers in its own JavaScript. A
/// native host has no page in front of its simulation, so the frame decodes
/// into `ClientMessage::ToggleQrCode` and arrives here, on the same
/// `InboundMessage` stream `debug_overlay::drain_client_pause` reads and in the
/// same frame-driven way — this changes no simulation outcome a replay must
/// re-derive, so it stays out of command admission and out of the command log.
///
/// **Every press is carried, not "somebody asked"**: two taps are two flips,
/// and a phone with a slow thumb landing both in one frame means the operator
/// ends where they started, which is what the person pressing it expects.
///
/// No station, captaincy or `GamePhase` check, deliberately. The panel says how
/// to join this ship; anyone already admitted to it can show it to somebody
/// standing next to them, and gating that on holding a seat would mean a full
/// bridge cannot let a latecomer in.
///
/// What it does NOT do is reveal the surface. In play the lobby surface is
/// composited only while F9 says so (see the module note on why an opaque
/// texture leaves no third option), and a phone in a player's pocket must not
/// be able to drop a black sheet over a running viewscreen. The press sets what
/// the operator finds when they reveal it.
fn drain_client_qr_toggle(
    bridge: Option<Res<HostLobbyBridgeResource>>,
    mut inbound: MessageReader<crate::lobby::InboundMessage>,
    log: Option<Res<LogFilterConfig>>,
) {
    // Read first, unconditionally, for the reason `feed_lobby_state` gives.
    let presses = inbound
        .read()
        .filter(|ev| matches!(ev.msg, crate::core::messages::ClientMessage::ToggleQrCode))
        .count();
    let Some(bridge) = bridge else {
        return;
    };
    if presses == 0 {
        return;
    }
    for _ in 0..presses {
        bridge.0.push_qr_toggle();
    }
    crate::pinfo!(
        log,
        LogCat::Lobby,
        "host lobby: join QR toggled from a phone ({presses})"
    );
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

/// What an accepted press did, for the operator log.
///
/// Rust-composed English, and allowed to be: this is a file, a scrollback and a
/// screenshot for whoever is running the bridge, never text a player reads
/// (AGENTS.md rule 11's operator-log footing — the player-visible half of this
/// same surface goes over as a `strings.csv` id and its parameters).
///
/// It reads the layout that RESULTED rather than restating the action, because
/// the law's no-op doctrine means a lawful press can change nothing — closing a
/// console that was not open, or naming the monitor a station is already on —
/// and a line that recited the request would claim something happened.
fn applied(
    action: &crate::native_host::bridge_layout::LayoutAction,
    next: &crate::native_host::bridge_layout::BridgeLayout,
) -> String {
    use crate::native_host::bridge_layout::LayoutAction;
    match action {
        LayoutAction::SetGameMaster { .. } => {
            format!("GM display: {:?}", next.game_master_monitor())
        }
        LayoutAction::SetViewscreen { .. } => {
            format!("viewscreen now on monitor {}", next.viewscreen())
        }
        LayoutAction::AssignStation { station, .. } | LayoutAction::UnassignStation { station } => {
            match next.monitor_of(station) {
                Some(monitor) => {
                    format!("station {:?}'s console is on monitor {monitor}", station.0)
                }
                None => format!("station {:?}'s console is closed", station.0),
            }
        }
    }
}

/// Publish the live bridge layout to the surface as its monitor row and its
/// per-station screen rows, and take the notices it carried off the resource.
///
/// Only when the layout resource actually changed — a press, a reconcile, or
/// the seed. The bridge drops a byte-identical push on top of that, so a bridge
/// nobody rearranged costs the simulation nothing; both guards are here because
/// they answer different questions ("has anything touched it" and "does it
/// still say the same thing"), and a `ResMut` deref that wrote the same value
/// would pass the first.
///
/// In `Update`, after [`drain_surface_records`] has applied this frame's presses
/// in `PreUpdate`. That ordering is the whole reason a refusal is something the
/// operator sees: the press, the layout it moved (or the notice it earned), and
/// the row that reports it are one frame and one repaint.
///
/// # It is the drain, which is what lets every writer append
///
/// [`BridgeLayoutResource::notices`](crate::native_host::bridge_display::BridgeLayoutResource::notices)
/// is what the lobby is *owed*, written by three systems in two unordered
/// chains — [`drain_surface_records`]'s layout arm, and `bridge_display`'s two
/// reconcilers. They all append, so none of them can erase another's sentence
/// (issue #1331); this is the one reader, so this is where "owed" ends.
///
/// The drain goes through `bypass_change_detection` deliberately. Marking the
/// resource changed here would publish the very same row again next frame — with
/// the notices now gone — and `gui/host-lobby-render.js` rebuilds the notice
/// element from the payload, so the operator's sentence would appear and vanish
/// within two frames. Nothing else reads this list, so skipping the change tick
/// hides nothing from anyone.
fn publish_bridge_layout(
    native_gm: Option<Res<super::native_gm::NativeGmLifecycle>>,
    phase: Option<Res<State<GamePhase>>>,
    bridge: Option<Res<HostLobbyBridgeResource>>,
    layout: Option<ResMut<BridgeLayoutResource>>,
    log: Option<Res<LogFilterConfig>>,
    assignments: Option<Res<super::console_assignment::ConsoleAssignments>>,
) {
    let (Some(bridge), Some(mut layout)) = (bridge, layout) else {
        return;
    };
    if !layout.is_changed()
        && !phase.as_ref().is_some_and(|p| p.is_changed())
        && !assignments.as_ref().is_some_and(|a| a.is_changed())
    {
        return;
    }
    let mut payload = bridge_layout_payload(&layout.layout, &layout.monitors, &layout.notices);
    if let Some(gm) = &mut payload.gm {
        gm.role_mutable = phase
            .as_ref()
            .is_none_or(|phase| phase.get() == &GamePhase::Lobby);
        if !gm.role_mutable && gm.assigned_to.is_none() {
            gm.assigned_to = native_gm
                .as_ref()
                .and_then(|g| g.desired_monitor.as_ref())
                .map(|id| id.as_str().to_string());
        }
    }
    if let Some(assignments) = assignments {
        for row in &mut payload.stations {
            if row.assigned_to.is_none() {
                row.assigned_to = assignments
                    .0
                    .get(&crate::core::messages::StationId(row.station.clone()))
                    .map(ToString::to_string);
            }
        }
    }
    match crate::core::codec::encode_bridge_layout(&payload) {
        Ok(json) => {
            bridge.0.push_layout(json);
            // Only what was actually pushed is cleared: a payload that could not
            // be encoded leaves the notices still owed, to ride out on the next
            // push rather than being lost to a codec error.
            if !layout.notices.is_empty() {
                layout.bypass_change_detection().notices.clear();
            }
        }
        Err(e) => crate::pwarn!(
            log,
            LogCat::Lobby,
            "host lobby: the monitor row could not be encoded: {e}"
        ),
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
            .add_message::<crate::lobby::InboundMessage>()
            .init_state::<GamePhase>()
            .insert_resource(HostLobbyBridgeResource(bridge.clone()))
            .add_plugins(HostLobbyPlugin);
        (app, bridge)
    }

    /// Read every `InboundMessage` this frame wrote, without consuming the
    /// reader the systems under test share.
    fn inbound(app: &mut App) -> Vec<crate::lobby::InboundMessage> {
        let messages = app
            .world()
            .resource::<Messages<crate::lobby::InboundMessage>>();
        messages.iter_current_update_messages().cloned().collect()
    }

    #[test]
    fn a_pick_made_on_the_surface_arrives_as_the_host_pages_own_picker_would() {
        // Issue #1328. The surface is not a participant, so what it sends is a
        // `HostLobbyRecord` rather than a `ClientMessage` — but what reaches the
        // arbiter has to be indistinguishable from the host PAGE's own picker,
        // which submits under `LOCAL_CONSOLE_TOKEN` through
        // `__localConsoleSend`. Anything else would be a second sender the
        // first-valid-wins rule had never heard of.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"select_scenario","scenario_id":"combat_test"}"#);
        surface.queue_record(r#"{"kind":"select_ship","template_path":"assets/a.toml"}"#);
        pump_host_lobby(&bridge, &mut surface);

        app.update();
        let sent = inbound(&mut app);
        assert_eq!(sent.len(), 2);
        for message in &sent {
            assert_eq!(message.token, crate::console_bridge::LOCAL_CONSOLE_TOKEN);
        }
        assert!(matches!(
            &sent[0].msg,
            crate::core::messages::ClientMessage::SelectScenario { scenario_id }
                if scenario_id == "combat_test"
        ));
        assert!(matches!(
            &sent[1].msg,
            crate::core::messages::ClientMessage::SelectPlayerShip { template_path }
                if template_path == "assets/a.toml"
        ));
    }

    #[test]
    fn a_record_this_bridge_does_not_speak_reaches_no_message_bus() {
        // A `ClientMessage` envelope is the shape most likely to arrive here by
        // mistake, and is the one that must NOT be forwarded: the surface holds
        // no session token, so admitting one would be admitting a participant
        // nobody identified.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"type":"SetReady","data":{"ready":true}}"#);
        surface.queue_record("not json at all");
        pump_host_lobby(&bridge, &mut surface);

        app.update();
        assert!(inbound(&mut app).is_empty());
    }

    #[test]
    fn the_launch_control_sets_the_latch_the_browser_button_sets() {
        // One force-start rule, not two: the record only raises
        // `PendingForceStart`, and `server::bridge::apply_force_start` — the
        // browser host's own, de-wasm-gated — decides everything else.
        let (mut app, bridge) = app_with_lobby();
        app.init_resource::<crate::server::bridge::PendingForceStart>();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"force_start"}"#);
        pump_host_lobby(&bridge, &mut surface);

        app.update();
        assert!(
            app.world()
                .resource::<crate::server::bridge::PendingForceStart>()
                .0
        );
        assert!(
            inbound(&mut app).is_empty(),
            "a launch is a host-side latch, not a participant's command"
        );
    }

    /// Every `AppExit` this frame wrote, without consuming the reader the app
    /// itself uses to decide it is finished.
    fn exits(app: &mut App) -> Vec<AppExit> {
        app.world()
            .resource::<Messages<AppExit>>()
            .iter_current_update_messages()
            .cloned()
            .collect()
    }

    #[test]
    fn a_confirmed_exit_writes_the_ordinary_app_exit_and_nothing_else() {
        // Issue #1365. The whole mechanism: the surface's confirmed press
        // becomes `AppExit::Success` — the same message
        // `bridge_display::setup_enumerate` writes to end `--setup` — and the
        // binary's existing teardown then runs unchanged, because the run ends
        // through the door that was already there. Nothing here withdraws a
        // document or stops the delivery service, and this asserts that too:
        // a second teardown path would be a second thing to keep in step.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"exit_desktop"}"#);
        pump_host_lobby(&bridge, &mut surface);

        app.update();
        assert_eq!(exits(&mut app), vec![AppExit::Success]);
        assert!(
            inbound(&mut app).is_empty(),
            "quitting is the host's own act, not a participant's command"
        );
    }

    #[test]
    fn opening_the_exit_route_asks_rather_than_quitting() {
        // The confirmation is the point of the slice: the ENTRY only opens the
        // stage that asks (`landing_open`), and it is the stage's own control
        // that sends `exit_desktop`. A host that quit on the menu press would
        // be a host with no confirmation at all, whatever the page drew.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"landing_open","entry":"exit_desktop"}"#);
        surface.queue_record(r#"{"kind":"landing_close"}"#);
        pump_host_lobby(&bridge, &mut surface);

        app.update();
        assert!(exits(&mut app).is_empty());
    }

    /// A shelf pointed at a folder that cannot exist.
    ///
    /// No temp directory, and none needed: every claim below is about what
    /// happens to a NAME the surface sent, and the shelf that name is looked up
    /// in is empty either way. The rules about which files reach a shelf are
    /// `native_host::mod_packs`'s, tested there without a filesystem at all.
    fn empty_shelf() -> packs::ModPackShelfResource {
        packs::ModPackShelfResource::new(
            "no-such-mod-folder-for-issue-1366",
            ".",
            "assets/scenarios.toml",
        )
    }

    #[test]
    fn a_shelf_is_put_on_the_surface_on_the_first_frame_and_not_again() {
        // Issue #1366's push rule: the landing must show the folder without
        // waiting for anybody to touch it, and an idle host must then cost the
        // surface nothing — the same arrangement the picker and the landing
        // beside it make.
        let (mut app, bridge) = app_with_lobby();
        app.insert_resource(empty_shelf());
        let mut surface = crate::native_host::panes::RecordingSurface::ready();

        let shelves = |surface: &crate::native_host::panes::RecordingSurface| {
            surface
                .pushed
                .iter()
                .filter(|s| s.contains("__phoenixHostLobbyPacks("))
                .cloned()
                .collect::<Vec<_>>()
        };

        app.update();
        pump_host_lobby(&bridge, &mut surface);
        let first = shelves(&surface);
        assert_eq!(first.len(), 1);
        assert!(first[0].contains("no-such-mod-folder-for-issue-1366"));
        // The folder is not there, so the panel says WHICH emptiness this is
        // rather than showing an empty list that reads as "there are no packs".
        assert!(first[0].contains("scan_error"));

        app.update();
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(shelves(&surface).len(), 1, "an idle shelf pushes nothing");
    }

    #[test]
    fn a_host_with_no_shelf_never_offers_one() {
        // The whole of how the landing's Load-mod-pack row stays inert on a host
        // started without `--mod-pack-dir`: no resource, no push, and therefore
        // nothing that tells the surface it can answer the row. Not a flag, and
        // not a check on the page.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        app.update();
        pump_host_lobby(&bridge, &mut surface);
        assert!(surface
            .pushed
            .iter()
            .all(|s| !s.contains("__phoenixHostLobbyPacks(")));
    }

    #[test]
    fn a_pack_this_host_never_offered_is_refused_and_says_so_on_the_panel() {
        // The gate, end to end. A name off the bridge is looked up in the shelf
        // THIS host produced and never joined onto the scanned directory, so
        // "never offered", "invented by a newer page" and "deleted since the
        // scan" are one refusal — and it is a refusal the operator can read,
        // not a silent drop.
        let (mut app, bridge) = app_with_lobby();
        app.insert_resource(empty_shelf());
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"install_mod_pack","pack":"../../secrets.zip"}"#);
        pump_host_lobby(&bridge, &mut surface);

        app.update();
        let shelf = app.world().resource::<packs::ModPackShelfResource>();
        assert_eq!(shelf.attempted.as_deref(), Some("../../secrets.zip"));
        assert!(!shelf.accepted);
        assert_eq!(shelf.findings.len(), 1);
        assert_eq!(shelf.findings[0].category, "unknown-pack");
        assert_eq!(shelf.findings[0].severity, "error");
        assert!(
            shelf.pending.is_none(),
            "the latch is answered, not left set"
        );
        // …and the answer reaches the surface in the same frame the press did.
        pump_host_lobby(&bridge, &mut surface);
        let packs_push = surface
            .pushed
            .iter()
            .rev()
            .find(|s| s.contains("__phoenixHostLobbyPacks("))
            .expect("the panel is told what happened");
        assert!(packs_push.contains("unknown-pack"));
        assert!(packs_push.contains(r#""accepted":false"#));
    }

    #[test]
    fn a_pack_chosen_on_a_host_with_no_shelf_is_dropped_rather_than_panicking() {
        // The resource is `Option` in the drain for the reason every other one
        // is: a bundle can be newer than the binary serving it, and a record for
        // a feature this host was not started with must be a line in the log
        // rather than a missing-resource panic on the simulation's own thread.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"install_mod_pack","pack":"anything.zip"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
        assert!(inbound(&mut app).is_empty());
        assert!(exits(&mut app).is_empty());
    }

    #[test]
    fn choosing_a_pack_is_not_a_participants_command_and_never_becomes_one() {
        // The surface holds no session token. An install rearranges this host's
        // own content overlay, exactly as a layout press rearranges its own
        // screens — it is not something a participant may say, and nothing here
        // may quietly turn it into one.
        let (mut app, bridge) = app_with_lobby();
        app.insert_resource(empty_shelf());
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"install_mod_pack","pack":"thin-margin.zip"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
        assert!(inbound(&mut app).is_empty());
    }

    #[test]
    fn a_composition_with_no_force_start_latch_ignores_the_press_rather_than_panicking() {
        // `app_with_lobby` is deliberately bare: the resource is `Option` in the
        // drain because a headless-shaped composition need not carry it, and a
        // missing resource on a system that RUNS is a Bevy panic rather than an
        // inert system.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"force_start"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
    }

    /// The primary window's mode, read the way `bridge_display`'s own tests
    /// read it — by the entity the fixture spawned, so a second window arriving
    /// later could never make this ambiguous.
    fn window_mode(app: &App, window: Entity) -> bevy::window::WindowMode {
        app.world()
            .entity(window)
            .get::<bevy::window::Window>()
            .unwrap()
            .mode
    }

    #[test]
    fn the_fullscreen_control_moves_the_primary_window_and_moves_it_back() {
        // Issue #1367, and the half `fullscreen.rs`'s unit tests cannot reach:
        // those prove what the two pure functions DECIDE, and this proves the
        // wiring around them — the record latches in `PreUpdate`, and
        // `apply_window_mode_toggle` spends the latch on the primary window in
        // `Update` of the SAME frame. Without it, deleting the dispatcher's arm
        // or dropping the applier out of `HostLobbyPlugin`'s chain leaves the
        // whole suite green and the answer to "does the window actually move?"
        // written down nowhere.
        use bevy::window::{MonitorSelection, PrimaryWindow, Window, WindowMode};

        let (mut app, bridge) = app_with_lobby();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        assert_eq!(window_mode(&app, window), WindowMode::Windowed);

        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
        pump_host_lobby(&bridge, &mut surface);

        // ONE frame: the drain latches in `PreUpdate` and the applier spends it
        // in `Update`, so an operator's press is answered in the frame it
        // arrived rather than the next one.
        app.update();
        assert_eq!(
            window_mode(&app, window),
            // `Current` because nothing has assigned this window a display:
            // the pure functions' own tests say why that is the only honest
            // answer, and this says the applier asks them.
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        );
        assert!(
            !app.world()
                .resource::<fullscreen::WindowModeToggle>()
                .pending,
            "the frame that applies a press clears the latch, or the window would flip again"
        );
        assert!(
            inbound(&mut app).is_empty(),
            "a window mode is this host's own, never a participant's command"
        );

        // And back, which is the whole control: a press is a TOGGLE, and the
        // second one has to read the mode the first one wrote.
        surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
        assert_eq!(window_mode(&app, window), WindowMode::Windowed);
    }

    #[test]
    fn a_host_with_no_primary_window_drops_the_fullscreen_press_rather_than_panicking() {
        // The sibling of the force-start case above, and `app_with_lobby` is
        // bare in the same way: a delivery-only or headless composition carries
        // the lobby surface and no window at all, so the applier's window query
        // matches nothing every time it runs. That is one warning, not a panic
        // — and the latch is spent either way, because a press held until the
        // day a window appeared would be a control acting minutes after it was
        // pressed.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
        assert!(
            !app.world()
                .resource::<fullscreen::WindowModeToggle>()
                .pending
        );
    }

    #[test]
    fn a_composition_with_no_window_mode_latch_ignores_the_press_rather_than_panicking() {
        // The drain's OTHER guard: `window_mode` is `Option<ResMut<_>>` there
        // for the reason `force_start` is, and while `HostLobbyPlugin` always
        // inserts the latch today, the arm that answers its absence is reached
        // by any composition that runs `drain_surface_records` without it. A
        // dispatcher that panicked on a record it could not answer would take
        // every other record in the same batch down with it.
        let (mut app, bridge) = app_with_lobby();
        app.world_mut()
            .remove_resource::<fullscreen::WindowModeToggle>();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"toggle_fullscreen"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
    }

    #[test]
    fn a_host_with_no_catalogue_never_publishes_a_picker() {
        // A `--world` host holds no `LobbyScenarioCatalog`, which is what leaves
        // its `#scenario-panel` at the `display: none` the document assembled it
        // with — the flag that decides the world skipping the stage it decides.
        let (mut app, bridge) = app_with_lobby();
        app.update();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        assert!(
            !surface
                .pushed
                .iter()
                .any(|s| s.contains("__phoenixHostLobbyScenario(")),
            "no catalogue, no picker: {:?}",
            surface.pushed
        );
    }

    #[test]
    fn a_host_with_no_catalogue_never_publishes_a_landing_either() {
        // Issue #1361, and the same fact doing the same job one layer out: a
        // `--world` host holds no `LobbyScenarioCatalog`, so it never pushes a
        // landing and `#landing-panel` stays at the `display: none` the
        // document assembled it with. An operator who was told what they are
        // flying at the prompt is not shown a menu asking.
        let (mut app, bridge) = app_with_lobby();
        app.update();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        assert!(
            !surface
                .pushed
                .iter()
                .any(|s| s.contains("__phoenixHostLobbyLanding(")),
            "no catalogue, no front door: {:?}",
            surface.pushed
        );
    }

    #[test]
    fn a_world_less_host_puts_the_landing_on_screen_and_a_landed_world_takes_it_away() {
        // The acceptance criterion, as the two pushes that make it true. The
        // first frame reveals the front door; the frame a `WorldConfig` lands
        // dismisses it, because the crew lobby underneath sits at z-index 180
        // and a landing left up would cover the thing the room is watching.
        let (mut app, bridge) = app_with_lobby();
        app.insert_resource(LobbyScenarioCatalog::default());
        app.insert_resource(LobbySelection::default());
        app.update();

        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        let first = surface
            .pushed
            .iter()
            .find(|s| s.contains("__phoenixHostLobbyLanding("))
            .expect("a world-less host shows its front door")
            .clone();
        assert!(first.contains(r#""dismissed":false"#), "{first}");
        assert!(
            first.contains(landing::BUILD_ID),
            "the status bar names THIS binary's build: {first}"
        );

        // Nothing moved: no second push, because the surface already has the
        // answer and every push is main-thread time inside a browser engine.
        surface.pushed.clear();
        app.update();
        pump_host_lobby(&bridge, &mut surface);
        assert!(
            !surface
                .pushed
                .iter()
                .any(|s| s.contains("__phoenixHostLobbyLanding(")),
            "an idle landing costs the simulation nothing: {:?}",
            surface.pushed
        );

        // …and the world lands.
        app.insert_resource(crate::world::config::WorldConfig::default());
        app.update();
        pump_host_lobby(&bridge, &mut surface);
        let last = surface
            .pushed
            .iter()
            .rev()
            .find(|s| s.contains("__phoenixHostLobbyLanding("))
            .expect("a committed world dismisses the landing")
            .clone();
        assert!(last.contains(r#""dismissed":true"#), "{last}");
    }

    #[test]
    fn a_landing_press_is_drained_by_the_one_reader_and_reaches_no_message_bus() {
        // Issue #1361. The menu's two records ride the same queue as the picks
        // beside them — `take_records` is a drain with one reader, so a new
        // control is a variant and never a second queue. What they must NOT do
        // is reach the arbiter: opening a route on the front door is not a
        // participant's command, and New Game's actual effect is the
        // `#scenario-panel` this surface already carries, whose picks take the
        // world-load path they always took.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"landing_open","entry":"new_game"}"#);
        surface.queue_record(r#"{"kind":"landing_close"}"#);
        surface.queue_record(r#"{"kind":"select_scenario","scenario_id":"combat_test"}"#);
        pump_host_lobby(&bridge, &mut surface);

        app.update();
        let sent = inbound(&mut app);
        assert_eq!(
            sent.len(),
            1,
            "only the pick is a message; the two menu presses are not"
        );
        assert!(matches!(
            &sent[0].msg,
            crate::core::messages::ClientMessage::SelectScenario { scenario_id }
                if scenario_id == "combat_test"
        ));
        // Drained rather than left on the queue: a record the host does not act
        // on still has to be taken, or the surface sits on a growing backlog.
        assert!(bridge.take_records().is_empty());
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
    fn a_phones_qr_toggle_reaches_the_surface() {
        // Issue #1329's AC3, end to end on the Rust side: the same button on
        // the same phone that a browser host answers in JavaScript arrives here
        // as a decoded message and leaves as a push the document answers.
        let (mut app, bridge) = app_with_lobby();
        app.update();
        // Drain the baseline reveal the first frame states.
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        surface.pushed.clear();

        app.world_mut().write_message(crate::lobby::InboundMessage {
            token: "phone-1".to_string(),
            msg: crate::core::messages::ClientMessage::ToggleQrCode,
        });
        app.update();
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(
            surface.pushed,
            vec!["window.__phoenixHostLobbyQrToggle()".to_string()]
        );
    }

    #[test]
    fn two_phone_presses_in_one_frame_are_two_flips() {
        // Which is a no-op, and is exactly what the person pressing twice
        // expects. Collapsing them to "somebody asked" would turn a double-tap
        // into a single flip.
        let (mut app, bridge) = app_with_lobby();
        app.update();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        surface.pushed.clear();

        for _ in 0..2 {
            app.world_mut().write_message(crate::lobby::InboundMessage {
                token: "phone-1".to_string(),
                msg: crate::core::messages::ClientMessage::ToggleQrCode,
            });
        }
        app.update();
        pump_host_lobby(&bridge, &mut surface);
        assert_eq!(surface.pushed.len(), 2);
    }

    #[test]
    fn a_phones_qr_toggle_does_not_uncover_the_surface_mid_mission() {
        // A phone in a player's pocket must not be able to drop a black sheet
        // over a running viewscreen: the surface composites into an OPAQUE
        // texture, so revealing it in play covers the mission. F9 stays the
        // only thing that uncovers it; the press sets what the operator finds
        // when they do.
        let (mut app, _bridge) = app_with_lobby();
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();
        app.update();

        app.world_mut().write_message(crate::lobby::InboundMessage {
            token: "phone-1".to_string(),
            msg: crate::core::messages::ClientMessage::ToggleQrCode,
        });
        app.update();
        assert!(
            !app.world()
                .resource::<HostLobbyRevealResource>()
                .0
                .presence()
                .composited
        );
    }

    #[test]
    fn other_client_messages_are_not_mistaken_for_the_qr_toggle() {
        let (mut app, bridge) = app_with_lobby();
        app.update();
        let mut surface = crate::native_host::panes::RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        surface.pushed.clear();

        app.world_mut().write_message(crate::lobby::InboundMessage {
            token: "phone-1".to_string(),
            msg: crate::core::messages::ClientMessage::ReleaseStation,
        });
        app.update();
        pump_host_lobby(&bridge, &mut surface);
        assert!(surface.pushed.is_empty());
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
    fn a_directly_joinable_host_shows_its_own_code_and_not_the_clouds() {
        // Issue #1353. A host running BOTH legs holds two codes, and they are
        // not interchangeable: the QR carries the PAGE as well as the code, the
        // page a LAN phone loads is served by this host, and so the service it
        // dials is this host. The worker's letters on that QR would send every
        // phone in the room to a service that never heard of them.
        let lobby = LocalHostLobby::open("192.168.1.5:8080");
        let mine = crate::core::rendezvous::JoinCode {
            full: "PHX_1_ABCDE".to_string(),
            suffix: "ABCDE".to_string(),
            ..Default::default()
        };
        let cloud = crate::core::rendezvous::JoinCode {
            full: "PHX_1_ZZZZZ".to_string(),
            suffix: "ZZZZZ".to_string(),
            ..Default::default()
        };
        let join = HostLobbyJoinResource::from_lobby(&lobby, Some("https://worker.example"))
            .with_direct_code(mine.clone());
        assert_eq!(join.viewscreen_invite(&cloud), None);
        let invite = join.viewscreen_invite(&mine).expect("its own code shows");
        // And with no `?rendezvous=` in it: the page dials the origin that
        // served it, so naming a service would be both redundant and wrong.
        assert_eq!(
            invite,
            JoinInvite::Code {
                code: "ABCDE".to_string(),
                full: "PHX_1_ABCDE".to_string(),
                page_base: "http://192.168.1.5:8080/".to_string(),
                rendezvous: None,
            }
        );

        // A cloud-only host is unchanged: whatever the service issues goes up,
        // with the service named for `gui/join-url.js` to judge.
        let cloud_only = HostLobbyJoinResource::from_lobby(&lobby, Some("https://worker.example"));
        assert!(cloud_only.viewscreen_invite(&cloud).is_some());
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

    // ── the monitor row's two directions (issue #1330) ──────────────────────
    //
    // The surface half of the row is `gui/host-lobby-{view,render}.js` and its
    // vitest suites; the law is `bridge_layout`'s. What is left — and what is
    // here — is the loop between them: a record the page queued becomes a
    // transition on the live layout, and the layout that results becomes the
    // row the page is handed back.

    use crate::native_host::bridge_layout::BridgeLayout;
    use crate::native_host::bridge_profile::{identify, DiscoveredMonitor, RawMonitor};
    use crate::native_host::panes::RecordingSurface;

    const DELL: &str = "DELL U2720Q@3840x2160";
    const BENQ: &str = "BenQ EX@1920x1080";

    fn two_monitors() -> Vec<DiscoveredMonitor> {
        identify(&[
            RawMonitor {
                name: Some("DELL U2720Q".to_string()),
                physical_width: 3840,
                physical_height: 2160,
                position_x: 0,
                position_y: 0,
                scale_factor: 1.0,
                primary: true,
            },
            RawMonitor {
                name: Some("BenQ EX".to_string()),
                physical_width: 1920,
                physical_height: 1080,
                position_x: 3840,
                position_y: 0,
                scale_factor: 1.0,
                primary: false,
            },
        ])
    }

    /// A two-monitor bridge with the viewscreen on the primary, as
    /// `apply_bridge_profile` seeds one.
    fn seeded_layout() -> BridgeLayoutResource {
        let monitors = two_monitors();
        let layout = BridgeLayout::from_discovered(
            &monitors,
            [crate::core::messages::StationId("helm".to_string())],
        )
        .expect("two monitors are a bridge");
        BridgeLayoutResource {
            layout,
            monitors,
            notices: Vec::new(),
        }
    }

    /// An app with the lobby plugin and a seeded bridge layout, plus a surface
    /// that has already taken the opening row.
    fn app_with_layout() -> (App, HostLobbyBridge, RecordingSurface) {
        let (mut app, bridge) = app_with_lobby();
        app.insert_resource(seeded_layout());
        app.update();
        let mut surface = RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        surface.pushed.clear();
        (app, bridge, surface)
    }

    /// Queue one record on the surface and run **one** frame that answers it.
    ///
    /// Exactly one `app.update()`, which is what makes every test below an
    /// assertion about the schedule as well as about the rule: the drain is in
    /// `PreUpdate` and `publish_bridge_layout` in `Update`, so a press and the
    /// row that answers it land in the same frame. Move the drain after the
    /// publish and every one of these fails, because the row would be a frame
    /// late and this helper never runs that frame.
    fn press(app: &mut App, bridge: &HostLobbyBridge, surface: &mut RecordingSurface, json: &str) {
        surface.queue_record(json);
        pump_host_lobby(bridge, surface);
        app.update();
        pump_host_lobby(bridge, surface);
    }

    fn viewscreen(app: &App) -> String {
        app.world()
            .resource::<BridgeLayoutResource>()
            .layout
            .viewscreen()
            .as_str()
            .to_string()
    }

    #[test]
    fn the_opening_row_is_pushed_as_soon_as_a_layout_exists() {
        let (mut app, bridge) = app_with_lobby();
        app.insert_resource(seeded_layout());
        app.update();
        let mut surface = RecordingSurface::ready();
        pump_host_lobby(&bridge, &mut surface);
        let row = surface
            .pushed
            .iter()
            .find(|s| s.contains("__phoenixHostLobbyLayout"))
            .expect("the row reaches the surface");
        assert!(row.contains(DELL));
        assert!(row.contains(BENQ));
    }

    #[test]
    fn a_button_press_moves_the_live_viewscreen_and_the_row_says_so() {
        // The acceptance criterion, end to end on the host side: a record the
        // page queued becomes one lawful transition, and the row that comes
        // back marks the display the operator chose.
        let (mut app, bridge, mut surface) = app_with_layout();
        assert_eq!(viewscreen(&app), DELL);

        press(
            &mut app,
            &bridge,
            &mut surface,
            r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#,
        );

        assert_eq!(viewscreen(&app), BENQ);
        let row = surface
            .pushed
            .iter()
            .find(|s| s.contains("__phoenixHostLobbyLayout"))
            .expect("the moved row is pushed back");
        // The mark travels as data — `viewscreen: true` on the display that
        // was pressed and nowhere else — and the row's words are the page's.
        assert!(
            row.contains(r#"{"identity":"BenQ EX@1920x1080","name":"BenQ EX","width":1920,"height":1080,"primary":false,"viewscreen":true}"#),
            "the pressed display is marked: {row}"
        );
        assert!(
            row.contains(r#""identity":"DELL U2720Q@3840x2160","name":"DELL U2720Q","width":3840,"height":2160,"primary":true,"viewscreen":false"#),
            "and the one it left is not: {row}"
        );
        assert!(
            !row.contains("bridge_layout"),
            "an accepted press has nothing to say: {row}"
        );
    }

    #[test]
    fn a_press_for_a_monitor_that_is_gone_is_refused_with_something_to_read() {
        // The stale press. "The lobby never silently ignores me" is the user
        // story, and a boolean cannot satisfy it — so the refusal comes back as
        // the id of a sentence the row renders.
        let (mut app, bridge, mut surface) = app_with_layout();

        press(
            &mut app,
            &bridge,
            &mut surface,
            r#"{"kind":"set-viewscreen","monitor":"Unplugged@1920x1080"}"#,
        );

        assert_eq!(viewscreen(&app), DELL, "nothing moved");
        let row = surface
            .pushed
            .iter()
            .find(|s| s.contains("__phoenixHostLobbyLayout"))
            .expect("the refusal is pushed back");
        assert!(row.contains("server.bridge_layout.unknown_monitor"));
        assert!(row.contains("Unplugged@1920x1080"));
        // And having been pushed, it is no longer OWED. The notices list is
        // what the surface has not been told yet (issue #1331 made every writer
        // append to it, so it has to be emptied somewhere); this is where.
        assert!(
            app.world()
                .resource::<BridgeLayoutResource>()
                .notices
                .is_empty(),
            "the publisher drains what it has pushed"
        );
    }

    #[test]
    fn moving_the_viewscreen_onto_a_monitor_holding_a_console_is_refused_not_resolved() {
        // Rule 2's mirror, reaching a person: the consoles are not evicted to
        // make room, and the row says which ones are in the way.
        let (mut app, bridge, mut surface) = app_with_layout();
        let seated = app
            .world()
            .resource::<BridgeLayoutResource>()
            .layout
            .apply(
                &crate::native_host::bridge_layout::LayoutAction::AssignStation {
                    station: crate::core::messages::StationId("helm".to_string()),
                    monitor: crate::native_host::bridge_profile::MonitorIdentity::new(BENQ),
                },
            )
            .expect("a free non-viewscreen monitor takes a console");
        app.world_mut()
            .resource_mut::<BridgeLayoutResource>()
            .layout = seated;

        press(
            &mut app,
            &bridge,
            &mut surface,
            r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#,
        );

        assert_eq!(viewscreen(&app), DELL);
        let row = surface
            .pushed
            .iter()
            .find(|s| s.contains("__phoenixHostLobbyLayout"))
            .expect("the refusal is pushed back");
        assert!(row.contains("server.bridge_layout.viewscreen_holds_stations"));
        assert!(row.contains("helm"));
    }

    #[test]
    fn an_accepted_press_clears_the_refusal_the_one_before_it_earned() {
        // The row shows what happened to the LAST thing the operator did.
        //
        // Asserted on the ROWS rather than on the resource, which is where the
        // claim actually lives: since issue #1331 every writer of `notices`
        // appends (so a press cannot erase a console surrender it raced) and the
        // publisher drains what it pushed — so "the refusal is gone" is a fact
        // about the row the surface is handed next, not about a field.
        let (mut app, bridge, mut surface) = app_with_layout();
        press(
            &mut app,
            &bridge,
            &mut surface,
            r#"{"kind":"set-viewscreen","monitor":"Unplugged@1920x1080"}"#,
        );
        let refused = surface
            .pushed
            .iter()
            .rev()
            .find(|s| s.contains("__phoenixHostLobbyLayout"))
            .expect("the refusal is pushed back")
            .clone();
        assert!(refused.contains("server.bridge_layout.unknown_monitor"));

        press(
            &mut app,
            &bridge,
            &mut surface,
            r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#,
        );
        let accepted = surface
            .pushed
            .iter()
            .rev()
            .find(|s| s.contains("__phoenixHostLobbyLayout"))
            .expect("the moved row is pushed back");
        assert_ne!(&refused, accepted, "a new row, not the old one repeated");
        assert!(
            !accepted.contains("bridge_layout"),
            "and it carries no notice at all: {accepted}"
        );
        assert!(app
            .world()
            .resource::<BridgeLayoutResource>()
            .notices
            .is_empty());
    }

    #[test]
    fn a_record_this_build_cannot_read_moves_nothing_and_says_nothing_to_the_row() {
        // A page/host mismatch is an operator's problem, not an answer to a
        // press — rendering it as feedback would tell the crew their button is
        // broken when what is broken is the bundle.
        let (mut app, bridge, mut surface) = app_with_layout();
        press(
            &mut app,
            &bridge,
            &mut surface,
            r#"{"monitor":"BenQ EX@1920x1080"}"#,
        );

        assert_eq!(viewscreen(&app), DELL);
        assert!(app
            .world()
            .resource::<BridgeLayoutResource>()
            .notices
            .is_empty());
        assert!(
            !surface
                .pushed
                .iter()
                .any(|s| s.contains("__phoenixHostLobbyLayout")),
            "and nothing changed, so there is no row to push"
        );
    }

    #[test]
    fn a_bridge_nobody_rearranged_pushes_no_row_at_all() {
        // Every push is a synchronous evaluate_script on the thread the fixed
        // tick runs on, and a monitor row changes about once a session.
        let (mut app, bridge, _) = app_with_layout();
        for _ in 0..10 {
            app.update();
        }
        assert!(!bridge.has_pending());
    }

    #[test]
    fn a_press_that_arrives_before_a_layout_exists_is_dropped_rather_than_queued() {
        // A host whose winit has not enumerated its displays yet has no layout
        // to judge a press against. Draining anyway is what stops a surface
        // that started talking to it sitting on a queue for the whole mission.
        let (mut app, bridge) = app_with_lobby();
        let mut surface = RecordingSurface::ready();
        surface.queue_record(r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
        assert!(bridge.take_records().is_empty());
    }

    #[test]
    fn one_drain_serves_every_verb_the_surface_speaks_in_one_frame() {
        // The claim this whole arrangement exists for. `take_records` is a
        // DRAIN, so a second reader would swallow the first's records and warn
        // about a vocabulary it does not speak while the first saw an empty
        // queue for the rest of the run — with a clean log at both ends, which
        // is why nothing else here would catch it.
        //
        // A frame carrying a pick, a launch and a monitor press proves all
        // three arrive: the pick as an InboundMessage the arbiter reads, the
        // launch on the force-start latch, the press on the live layout — and
        // the row reporting it published in that SAME frame, which is the
        // PreUpdate-drain → Update-publish edge stated as a claim rather than
        // left to the other tests to imply.
        let (mut app, bridge, mut surface) = app_with_layout();
        app.insert_resource(crate::server::bridge::PendingForceStart(false));
        assert_eq!(viewscreen(&app), DELL);

        surface.queue_record(r#"{"kind":"select_scenario","scenario_id":"combat_test"}"#);
        surface.queue_record(r#"{"kind":"force_start"}"#);
        surface.queue_record(r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#);
        pump_host_lobby(&bridge, &mut surface);
        app.update();
        pump_host_lobby(&bridge, &mut surface);

        let picks = inbound(&mut app);
        assert_eq!(picks.len(), 1, "the pick reached the arbiter's bus");
        assert_eq!(picks[0].token, LOCAL_CONSOLE_TOKEN);
        assert!(matches!(
            picks[0].msg,
            ClientMessage::SelectScenario { ref scenario_id } if scenario_id == "combat_test"
        ));
        assert!(
            app.world()
                .resource::<crate::server::bridge::PendingForceStart>()
                .0,
            "the launch reached the latch in the same frame as the pick"
        );
        assert_eq!(
            viewscreen(&app),
            BENQ,
            "and the monitor press reached the live layout, rather than being \
             swallowed by whichever reader ran first"
        );
        assert!(
            surface
                .pushed
                .iter()
                .any(|s| s.contains("__phoenixHostLobbyLayout") && s.contains(BENQ)),
            "the row that answers the press is published in that same frame: {:?}",
            surface.pushed
        );
    }
}
