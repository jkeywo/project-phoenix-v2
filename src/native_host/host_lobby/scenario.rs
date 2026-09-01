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
//!   only the picker's half: the monitor row's press (issue #1330) and a
//!   station's two screen-row presses (issue #1331) are variants here too,
//!   because the bridge's record queue is a drain with one reader. A
//!   closed vocabulary, because the surface is **not a participant**: it holds
//!   no session token, and a `ClientMessage` arriving on this bridge would be a
//!   category error (see [`super::document`]'s note on why the two page→host
//!   namespaces are distinct). The enum therefore outgrew this module's name;
//!   it is here because this is where the surface first had anything to say.
//!
//! # The three catalogue fields are the phone's own
//!
//! `scenarios`, `locked_scenario` and `locked_ship` are the fields of
//! [`ServerMessage::ScenarioCatalog`](crate::core::messages::ServerMessage::ScenarioCatalog),
//! and [`crate::native_host::world_load`] builds both from one value so the
//! viewscreen and every phone in the room cannot be looking at two different
//! catalogues. `locked_ship` in particular reports a pinned `--ship` ahead of an
//! arbitrated one, because that is the hull the host will actually fly — a
//! picker offering a choice the host has already overruled is a lie whoever is
//! standing in front of the viewscreen would act on.

use serde::{Deserialize, Serialize};

use crate::core::messages::ScenarioCatalogWire;

/// The scenario picker's whole state, as one snapshot the surface renders.
///
/// Serialised straight into `scenarioCatalogView(scenarios, {scenario_id,
/// template_path}, locked)` by `host_lobby_link.js`, which is why the two lock
/// fields carry the wire message's names rather than the view model's: the
/// payload is the host's answer, and the mapping into the view model's argument
/// shape is one line on the page.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioPanelPayload {
    /// Every scenario this host publishes, in manifest order.
    pub scenarios: Vec<ScenarioCatalogWire>,
    /// The arbiter's locked scenario id, or `None` while the pick is open.
    pub locked_scenario: Option<String>,
    /// The locked hull's `template_path` — a pinned `--ship` first, then the
    /// arbitrated pick.
    pub locked_ship: Option<String>,
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

    fn wire(id: &str) -> ScenarioCatalogWire {
        ScenarioCatalogWire {
            id: id.to_string(),
            world: format!("assets/worlds/{id}.toml"),
            label: Some(format!("{id} label")),
            description: None,
            ships: Vec::new(),
        }
    }

    #[test]
    fn the_payload_carries_the_three_arguments_the_shared_view_model_takes() {
        // The claim that makes "one picker" true: what crosses is exactly what
        // `scenarioCatalogView(catalog, preSelection, locked)` reads, so the
        // native surface cannot be deciding anything the browser does not.
        let payload = ScenarioPanelPayload {
            scenarios: vec![wire("combat_test")],
            locked_scenario: Some("combat_test".into()),
            locked_ship: None,
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
    fn the_surfaces_six_records_round_trip() {
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
