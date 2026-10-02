use super::*;
use crate::core::messages::ScenarioCatalogWire;

fn wire(id: &str) -> ScenarioCatalogWire {
    ScenarioCatalogWire {
        id: id.to_string(),
        world: format!("assets/worlds/{id}.toml"),
        label: Some(format!("{id} label")),
        description: None,
        ships: Vec::new(),
        slots: Vec::new(),
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
        ship_required: true,
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
        HostLobbyRecord::SetLocale {
            locale: "de".into(),
        },
        HostLobbyRecord::SetPresentation {
            text_scale_percent: Some(150),
            contrast: Some(true),
            shake_percent: Some(30),
            flash_percent: Some(0),
            decorative_motion_percent: Some(40),
        },
        HostLobbyRecord::SetPresentation {
            text_scale_percent: None,
            contrast: None,
            shake_percent: None,
            flash_percent: None,
            decorative_motion_percent: None,
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
        HostLobbyRecord::decode(r#"{"kind":"select_slot","slot_id":"escort"}"#),
        Some(HostLobbyRecord::SelectSlot {
            slot_id: "escort".into()
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
    // Issue #1427. `gui/viewscreen-presentation.js` builds this by hand too,
    // so the tag and both field names are pinned here. An ABSENT field is
    // "follow this machine", which is what a per-setting reset and Reset all
    // both send — so the empty-bodied form has to decode rather than being a
    // record the host cannot read.
    assert_eq!(
        HostLobbyRecord::decode(
            r#"{"kind":"set_presentation","text_scale_percent":150,"contrast":true,
                    "shake_percent":30,"flash_percent":0,"decorative_motion_percent":40}"#
        ),
        Some(HostLobbyRecord::SetPresentation {
            text_scale_percent: Some(150),
            contrast: Some(true),
            shake_percent: Some(30),
            flash_percent: Some(0),
            decorative_motion_percent: Some(40),
        })
    );
    assert_eq!(
        HostLobbyRecord::decode(
            r#"{"kind":"set_presentation","text_scale_percent":null,"contrast":null,
                    "shake_percent":null,"flash_percent":null,
                    "decorative_motion_percent":null}"#
        ),
        Some(HostLobbyRecord::SetPresentation {
            text_scale_percent: None,
            contrast: None,
            shake_percent: None,
            flash_percent: None,
            decorative_motion_percent: None,
        })
    );
    assert_eq!(
        HostLobbyRecord::decode(r#"{"kind":"set_presentation"}"#),
        Some(HostLobbyRecord::SetPresentation {
            text_scale_percent: None,
            contrast: None,
            shake_percent: None,
            flash_percent: None,
            decorative_motion_percent: None,
        })
    );
    // A BUNDLE OLDER than this host still speaks: the three effect fields
    // are `#[serde(default)]`, so a page that only knows about text size and
    // contrast leaves the effects following this machine rather than failing
    // to be read at all (issue #1428).
    assert_eq!(
        HostLobbyRecord::decode(
            r#"{"kind":"set_presentation","text_scale_percent":125,"contrast":false}"#
        ),
        Some(HostLobbyRecord::SetPresentation {
            text_scale_percent: Some(125),
            contrast: Some(false),
            shake_percent: None,
            flash_percent: None,
            decorative_motion_percent: None,
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
