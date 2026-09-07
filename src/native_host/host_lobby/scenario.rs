//! What the viewscreen's scenario picker is shown, and what it says back
//! (issue #1328).
//!
//! # The picker is the host page's, rendered from the host process's own answer
//!
//! A browser host chooses its scenario in JavaScript, before Bevy exists:
//! `server.html` holds the catalogue, `gui/scenario-arbiter.js` decides what a
//! pick does, and `renderScenarioLockState()` draws the result. A native host
//! has none of that — its lobby is a running `App` and its arbiter is
//! [`crate::lobby::scenario_arbiter`] — but it renders through the *same* two
//! modules the browser does (`gui/host-scenarios.js` for the stage decision,
//! `gui/host-scenario-render.js` for the writes), because #1328 extracted them
//! for exactly this.
//!
//! So this module is the adapter between those two facts, and it is deliberately
//! thin:
//!
//! * [`ScenarioPanelPayload`] is precisely `scenarioCatalogView`'s three
//!   arguments — the catalogue, the arbiter's lock state, and whether the picker
//!   is locked out — carried as one snapshot. Nothing is computed here that the
//!   shared view model computes there.
//! * [`HostLobbyRecord`] is what the surface says back — **all** of it, not
//!   only the picker's half: the monitor row's press (issue #1330), a
//!   station's two screen-row presses (issue #1331) and the landing menu's two
//!   (issue #1361) are variants here too,
//!   because the bridge's record queue is a drain with one reader. A
//!   closed vocabulary, because the surface is **not a participant**: it holds
//!   no session token, and a `ClientMessage` arriving on this bridge would be a
//!   category error (see [`super::document`]'s note on why the two page→host
//!   namespaces are distinct). The enum therefore outgrew this module's name;
//!   it is here because this is where the surface first had anything to say.
//!
//! # The catalogue snapshot is the phone's own
//!
//! The surface flattens the crew wire's typed ScenarioCatalogPayload, including
//! source provenance and active packs. Native world load projects it once, so
//! the viewscreen and every phone in the room cannot be looking at two different
//! catalogues. `locked_ship` in particular reports a pinned `--ship` ahead of an
//! arbitrated one, because that is the hull the host will actually fly — a
//! picker offering a choice the host has already overruled is a lie whoever is
//! standing in front of the viewscreen would act on.

use serde::{Deserialize, Serialize};

use crate::core::messages::ScenarioCatalogPayload;

/// The scenario picker's whole state, as one snapshot the surface renders.
///
/// Serialised straight into `scenarioCatalogView(scenarios, {scenario_id,
/// template_path}, locked)` by `host_lobby_link.js`, which is why the two lock
/// fields carry the wire message's names rather than the view model's: the
/// payload is the host's answer, and the mapping into the view model's argument
/// shape is one line on the page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScenarioPanelPayload {
    /// The crew wire's complete snapshot, including provenance and active packs.
    #[serde(flatten)]
    pub catalog: ScenarioCatalogPayload,
    /// The picker is closed for good: a world has been ingested.
    ///
    /// `scenarioCatalogView`'s third argument, and the native twin of
    /// `server.html`'s `_worldLoadStarted`. It is taken as an explicit input
    /// rather than derived from the two locks above for the reason
    /// `gui/host-scenarios.js` documents: a host can reach "the world is
    /// loading" without ever completing a selection (a pinned `--ship`
    /// completes the pick on the scenario lock alone, issue #1326), and a view
    /// deriving lockedness from completeness alone would be wrong on exactly
    /// that host.
    pub locked: bool,
}

impl ScenarioPanelPayload {
    /// Encode for the bridge. Infallible in practice — every field is a plain
    /// string, bool or list — and an encode that somehow failed is answered with
    /// a payload that renders a closed picker rather than with a panic on the
    /// simulation's own thread.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"scenarios":[],"locked":true}"#.into())
    }
}

/// Everything the lobby surface may say to its host.
///
/// A closed vocabulary, tagged by `kind`, and deliberately **not**
/// `ClientMessage`: the surface has no session token and is not a participant,
/// so a record here is the host operator acting on the host's own window. Two of
/// them are turned into `ClientMessage`s under
/// [`crate::console_bridge::LOCAL_CONSOLE_TOKEN`] — the same token
/// `server.html`'s own picker submits its picks under (`__localConsoleSend`,
/// issue #822) — because from the arbiter's point of view the host page's picker
/// and this one are the same sender.
///
/// # Why the layout presses are in here and not in a type of their own
///
/// [`SetViewscreen`](Self::SetViewscreen) is issue #1330's and the two station
/// verbs are #1331's, and all three live beside #1328's picks because
/// [`HostLobbyBridge::take_records`] is a **drain**. Two record types would want
/// two readers; whichever ran first would swallow the other's records and warn
/// about a vocabulary it does not speak, and the second would find an empty
/// queue every frame for the rest of the run — both with a clean log. So the
/// surface gets one vocabulary and
/// [`drain_surface_records`](super::drain_surface_records) is its one reader,
/// dispatching on the variant. What each verb *does* still belongs to the module
/// that owns it: the layout half of these three is
/// [`layout::set_viewscreen_action`](super::layout::set_viewscreen_action),
/// [`layout::assign_station_action`](super::layout::assign_station_action) and
/// [`layout::unassign_station_action`](super::layout::unassign_station_action).
///
/// The rule for the slices that follow: a new page->host control is a variant
/// here, never a second record type and never a second `take_records` caller.
///
/// [`HostLobbyBridge::take_records`]: super::HostLobbyBridge::take_records
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostLobbyRecord {
    /// The operator picked a scenario.
    SelectScenario { scenario_id: String },
    /// The operator picked a hull (or the single-hull auto-resolve did).
    SelectShip { template_path: String },
    /// The operator pressed the lobby's AI-launch control.
    ///
    /// Answered by `server::bridge::apply_force_start`, which is the browser
    /// host's own rule de-wasm-gated rather than a second one — so a fully
    /// AI-crewed launch obeys the same Lobby/preload/world checks on both hosts.
    ForceStart,
    /// The operator pressed a button in the monitor row: show the shared
    /// viewscreen on this display (issue #1330).
    ///
    /// **Kebab-tagged, against the `rename_all` its siblings take.** The tag is
    /// what `gui/host-lobby-view.js` writes by hand, it shipped that way, and
    /// nothing about folding the two vocabularies together is worth a wire break
    /// on a bridge whose page comes out of a bundle that may be older than the
    /// host. The explicit rename is the whole cost of keeping it.
    #[serde(rename = "set-viewscreen")]
    SetViewscreen { monitor: String },
    /// The operator pressed a screen button in a station's row: open — or
    /// re-seat — that station's console on this display (issue #1331).
    ///
    /// Kebab-tagged, like the `set-viewscreen` it was written to match: the
    /// three layout verbs are one row of controls on the page and share one
    /// spelling convention there, whatever the picks beside them do.
    ///
    /// There is no `move-station` sibling, because the law has no move action:
    /// naming a different screen for a station that is already seated *is* the
    /// move, so the row presses one button either way and cannot pick the wrong
    /// verb.
    #[serde(rename = "assign-station")]
    AssignStation { station: String, monitor: String },
    /// The operator pressed a station row's "off" button: close that station's
    /// console (issue #1331). Kebab-tagged, for the reason above.
    #[serde(rename = "unassign-station")]
    UnassignStation { station: String },
    /// The operator opened a route on the landing screen (issue #1361).
    ///
    /// `entry` is a row's `id` from `gui/host-landing-view.js`'s
    /// `LANDING_ENTRIES` — `new_game` today, and whatever #1365-#1367 add
    /// beside it. Carried as a **string**, not as a Rust enum mirroring that
    /// table: the table is the one place an entry is declared, and a second
    /// copy here would make "add an entry" mean editing two files in two
    /// languages, which is exactly what #1360 made the entries data to avoid.
    ///
    /// WHICH entry is open is not the host's answer — `nextOpenEntry` decides
    /// it over the page's own memory, on both surfaces. What this carries is
    /// what the operator *did*, so the process can say in its log what its own
    /// viewscreen is showing, and so an entry that needs the host to answer it
    /// has somewhere to land. `new_game` needs nothing: it reveals the
    /// `#scenario-panel` this surface already carries, whose picks reach
    /// `crate::lobby::scenario_arbiter` and `native_host::world_load` down
    /// exactly the path they always did.
    ///
    /// Snake_case, like the picks and the AI launch it sits with — the kebab
    /// spellings are the layout row's alone, and are historical (see
    /// [`SetViewscreen`](Self::SetViewscreen)) rather than a convention to
    /// extend.
    LandingOpen { entry: String },
    /// The operator closed the open route — a second press on the entry that
    /// opened it (issue #1361).
    ///
    /// A verb of its own rather than a `LandingOpen { entry: "" }`, for the
    /// reason [`UnassignStation`](Self::UnassignStation) is not an
    /// `AssignStation` with an empty monitor at this layer: "nothing is open"
    /// is a state the host can state, and a sentinel string would be a state
    /// it can only be read to mean.
    LandingClose,
    /// The operator confirmed Exit to Desktop (issue #1365).
    ///
    /// The first landing verb the HOST has to answer, and the reason
    /// [`LandingOpen`](Self::LandingOpen) was written to be extended rather
    /// than to be the whole of the menu's vocabulary. It is the confirmation's
    /// press, never the entry's: the row carries a `confirm` block
    /// (`gui/host-landing-view.js`), so opening the route only draws the panel
    /// that asks, and this arrives only once an operator has answered it. There
    /// is no second confirmation on this side — a host that re-asked would be
    /// asking a question the operator already answered on a surface they are
    /// looking at.
    ///
    /// Snake_case, with the picks: the kebab spellings are the layout row's
    /// alone and are historical (see [`SetViewscreen`](Self::SetViewscreen)).
    /// The tag is deliberately the same token as the row's `confirm.action`, so
    /// the page forwards the verb it was given instead of keeping a mapping
    /// table that would be a second place to edit.
    ///
    /// **Web hosts never send it**, and cannot: the row is `platforms:
    /// ['native']`, because a browser tab has no application to quit.
    ExitDesktop,
    /// The operator chose a mod pack off the shelf (issue #1366).
    ///
    /// `pack` is a FILE NAME this host itself offered — one of the `file` values
    /// in [`packs::ModPackPanelPayload::offered`](super::packs::ModPackPanelPayload)
    /// — carried verbatim. It is never a path, and whatever answers it never
    /// joins it onto the scanned directory: the host looks it up in the shelf it
    /// produced ([`crate::native_host::mod_packs::offered`]), so a name it never
    /// offered and a name deleted since the scan are one refusal. That lookup
    /// gate is why this can safely be a bare string off a bridge.
    ///
    /// The SECOND landing verb the host has to answer, and the second one
    /// [`LandingOpen`](Self::LandingOpen) was written to make room for. Unlike
    /// [`ExitDesktop`](Self::ExitDesktop) it carries no confirmation: a pack that
    /// is wrong is refused whole and reports why, and one that is merely unwanted
    /// can be taken back out of the overlay stack — so there is nothing here to
    /// ask twice about, and a host that asked would be asking about the one
    /// landing route that IS reversible.
    ///
    /// Snake_case, with the picks and the landing's own two; the kebab spellings
    /// are the layout row's alone and are historical (see
    /// [`SetViewscreen`](Self::SetViewscreen)).
    ///
    /// **Web hosts never send it**, and are not offered the row: the shelf is a
    /// scanned FOLDER and a browser has none, while the mod-pack door a browser
    /// host does have — the `#mod-pack-upload` file input inside
    /// `#scenario-panel`, which rides into the landing's middle column when New
    /// Game docks that panel — is a working control one column away. So the row
    /// is `platforms: ['native']`, the same doctrine
    /// [`ExitDesktop`](Self::ExitDesktop) states pointing the other way; a
    /// second, permanently dashed door beside a live one would be the thing that
    /// doctrine forbids. `needs: 'packs'` stays on the row beside it and says
    /// the other half: a native host started without `--mod-pack-dir` has
    /// nothing to offer either, so the row is inert on that run too.
    InstallModPack { pack: String },
    /// The operator pressed the landing's fullscreen control (issue #1367).
    ///
    /// The THIRD landing verb the host has to answer, and the only one that is
    /// not a route at all: it is the corner control beside the menu rather than
    /// a row in it, so it carries no entry id and opens no stage.
    ///
    /// A browser host never sends it and does not need to — `gui/page-chrome.js`
    /// implements fullscreen once, in the page, and the landing's control
    /// forwards to that. There is no such implementation to forward to here:
    /// this window has no browser chrome, and what fullscreen MEANS on it is the
    /// primary window's `WindowMode`, which is the host process's to set. So the
    /// press crosses the bridge, and
    /// [`fullscreen::apply_window_mode_toggle`](super::fullscreen::apply_window_mode_toggle)
    /// answers it the way the display-assignment law already does.
    ///
    /// A toggle rather than a `SetWindowMode { fullscreen: bool }`, because the
    /// page cannot see the answer: the surface is an embedded view with no
    /// `document.fullscreenElement` and no window manager, so a record naming
    /// the state it wanted would be the page asserting something only the host
    /// knows. What the operator did is "press the control"; what that means is
    /// decided where the current mode is legible.
    ///
    /// Snake_case, with the picks and the landing's own verbs; the kebab
    /// spellings are the layout row's alone and are historical (see
    /// [`SetViewscreen`](Self::SetViewscreen)).
    ToggleFullscreen,
}

impl HostLobbyRecord {
    /// Decode one record the surface queued, or `None` if it is not one.
    ///
    /// `None` rather than an error type: the only sender is a document this
    /// process assembled and serves, so anything unrecognised here is a bug in
    /// that document rather than input to be validated — and the caller logs it.
    ///
    /// The parse itself is [`crate::core::codec`]'s, like every other JSON this
    /// repository reads (AGENTS.md Key Constraint 1); this is the caller-shaped
    /// wrapper around it.
    pub fn decode(json: &str) -> Option<Self> {
        crate::core::codec::decode_host_lobby_record(json).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::ScenarioCatalogWire;

    fn wire(id: &str) -> ScenarioCatalogWire {
        ScenarioCatalogWire {
            id: id.to_string(),
            world: format!("assets/worlds/{id}.toml"),
            label: Some(format!("{id} label")),
            description: None,
            ships: Vec::new(),
            source: "base".into(),
        }
    }

    #[test]
    fn the_payload_carries_the_three_arguments_the_shared_view_model_takes() {
        // The claim that makes "one picker" true: what crosses is exactly what
        // `scenarioCatalogView(catalog, preSelection, locked)` reads, so the
        // native surface cannot be deciding anything the browser does not.
        let payload = ScenarioPanelPayload {
            catalog: ScenarioCatalogPayload {
                scenarios: vec![wire("combat_test")],
                locked_scenario: Some("combat_test".into()),
                ..Default::default()
            },
            locked: false,
        };
        let json = payload.to_json();
        assert!(json.contains(r#""locked_scenario":"combat_test""#));
        assert!(json.contains(r#""locked_ship":null"#));
        assert!(json.contains(r#""locked":false"#));
        assert!(json.contains(r#""id":"combat_test""#));
    }

    #[test]
    fn an_empty_catalogue_still_encodes_a_pickable_panel() {
        // `scenario-empty` is a stage the shared view model has a name for; it
        // must not be reachable only by the payload failing to encode.
        let json = ScenarioPanelPayload::default().to_json();
        assert!(json.contains(r#""scenarios":[]"#));
        assert!(json.contains(r#""locked":false"#));
    }

    #[test]
    fn the_surfaces_eleven_records_round_trip() {
        for record in [
            HostLobbyRecord::SelectScenario {
                scenario_id: "combat_test".into(),
            },
            HostLobbyRecord::SelectShip {
                template_path: "assets/entities/alliance_destroyer.toml".into(),
            },
            HostLobbyRecord::ForceStart,
            HostLobbyRecord::SetViewscreen {
                monitor: "BenQ EX@1920x1080".into(),
            },
            HostLobbyRecord::AssignStation {
                station: "helm".into(),
                monitor: "BenQ EX@1920x1080".into(),
            },
            HostLobbyRecord::UnassignStation {
                station: "helm".into(),
            },
            HostLobbyRecord::LandingOpen {
                entry: "new_game".into(),
            },
            HostLobbyRecord::LandingClose,
            HostLobbyRecord::ExitDesktop,
            HostLobbyRecord::InstallModPack {
                pack: "thin-margin.zip".into(),
            },
            HostLobbyRecord::ToggleFullscreen,
        ] {
            let json = serde_json::to_string(&record).expect("a record encodes");
            assert_eq!(HostLobbyRecord::decode(&json), Some(record));
        }
    }

    #[test]
    fn the_wire_names_are_the_ones_the_document_writes() {
        // `host_lobby_link.js` and `gui/host-lobby-view.js` build these by hand
        // — neither has serde — so the tags are a contract between files and
        // are pinned here rather than left to be discovered on a viewscreen.
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"select_scenario","scenario_id":"patrol"}"#),
            Some(HostLobbyRecord::SelectScenario {
                scenario_id: "patrol".into()
            })
        );
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"select_ship","template_path":"a.toml"}"#),
            Some(HostLobbyRecord::SelectShip {
                template_path: "a.toml".into()
            })
        );
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"force_start"}"#),
            Some(HostLobbyRecord::ForceStart)
        );
        // KEBAB, all three of the layout verbs. `rename_all = "snake_case"`
        // would have made these `set_viewscreen`, `assign_station` and
        // `unassign_station`; the page has been sending `set-viewscreen` since
        // issue #1330 and its two station siblings were written to match it — so
        // the explicit `rename`s are load-bearing and the snake_case spellings
        // must NOT be accepted, or a bundle drifting to them would work here and
        // nowhere else.
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"set-viewscreen","monitor":"BenQ@1920x1080"}"#),
            Some(HostLobbyRecord::SetViewscreen {
                monitor: "BenQ@1920x1080".into()
            })
        );
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"set_viewscreen","monitor":"BenQ@1920x1080"}"#),
            None
        );
        assert_eq!(
            HostLobbyRecord::decode(
                r#"{"kind":"assign-station","station":"helm","monitor":"BenQ@1920x1080"}"#
            ),
            Some(HostLobbyRecord::AssignStation {
                station: "helm".into(),
                monitor: "BenQ@1920x1080".into()
            })
        );
        assert_eq!(
            HostLobbyRecord::decode(
                r#"{"kind":"assign_station","station":"helm","monitor":"BenQ@1920x1080"}"#
            ),
            None
        );
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"unassign-station","station":"helm"}"#),
            Some(HostLobbyRecord::UnassignStation {
                station: "helm".into()
            })
        );
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"unassign_station","station":"helm"}"#),
            None
        );
        // The landing's two (issue #1361) are snake_case, like the picks they
        // sit with rather than like the layout row above them.
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"landing_open","entry":"new_game"}"#),
            Some(HostLobbyRecord::LandingOpen {
                entry: "new_game".into()
            })
        );
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"landing_close"}"#),
            Some(HostLobbyRecord::LandingClose)
        );
        // Exit to Desktop (issue #1365) is snake_case with them, and its tag is
        // the same token the entry table gives the row as its `confirm.action`
        // — that identity is what lets `host_lobby_link.js` forward the verb it
        // was handed rather than translate it.
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"exit_desktop"}"#),
            Some(HostLobbyRecord::ExitDesktop)
        );
        assert_eq!(HostLobbyRecord::decode(r#"{"kind":"exit-desktop"}"#), None);
        // The mod-pack shelf's one verb (issue #1366), snake_case with them.
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"install_mod_pack","pack":"thin-margin.zip"}"#),
            Some(HostLobbyRecord::InstallModPack {
                pack: "thin-margin.zip".into()
            })
        );
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"install-mod-pack","pack":"a.zip"}"#),
            None
        );
        // A pack name this host never offered still DECODES — the refusal is the
        // host's shelf lookup, not the parser's. A parse failure here would read
        // as a broken bridge, and the honest answer is a finding on the panel.
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"install_mod_pack","pack":"../secrets.zip"}"#),
            Some(HostLobbyRecord::InstallModPack {
                pack: "../secrets.zip".into()
            })
        );
        // An entry id this build has never heard of still decodes: the entry
        // TABLE is the client's, a bundle may be newer than the host, and a
        // record the host cannot act on is a line in its log rather than a
        // parse failure that reads as a broken bridge.
        assert_eq!(
            HostLobbyRecord::decode(r#"{"kind":"landing_open","entry":"exit"}"#),
            Some(HostLobbyRecord::LandingOpen {
                entry: "exit".into()
            })
        );
    }

    #[test]
    fn a_record_this_bridge_does_not_speak_is_refused_rather_than_guessed_at() {
        // A `ClientMessage` envelope is the shape most likely to be sent by
        // mistake, and is exactly what must NOT be accepted here: the surface
        // holds no session token, so a participant's message on this bridge
        // would be a participant nobody admitted.
        assert_eq!(HostLobbyRecord::decode(r#"{"type":"SetReady"}"#), None);
        assert_eq!(HostLobbyRecord::decode(r#"{"kind":"launch"}"#), None);
        assert_eq!(HostLobbyRecord::decode("not json at all"), None);
    }
}
