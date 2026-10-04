use super::*;
use crate::core::messages::StationId;
use crate::native_host::bridge_profile::{identify, RawMonitor};

fn raw(name: &str, w: u32, h: u32, x: i32, primary: bool) -> RawMonitor {
    RawMonitor {
        name: Some(name.to_string()),
        physical_width: w,
        physical_height: h,
        position_x: x,
        position_y: 0,
        scale_factor: 1.0,
        primary,
    }
}

/// A bridge of two monitors: the TV is primary and the viewscreen.
fn two_monitors() -> Vec<DiscoveredMonitor> {
    identify(&[
        raw("BRAVIA", 3840, 2160, 0, true),
        raw("BenQ EX", 1920, 1080, 3840, false),
    ])
}

fn roster() -> Vec<StationId> {
    vec![StationId("helm".to_string())]
}

fn layout() -> BridgeLayout {
    BridgeLayout::from_discovered(&two_monitors(), roster()).expect("two monitors are a bridge")
}

#[test]
fn a_button_carries_the_identity_a_press_names_back() {
    // The whole round trip in one claim: what the row shows is what the
    // press says, so the host resolves the button to the display the
    // operator was looking at rather than to an index into a list that
    // moved.
    let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
    let pressed = &payload.monitors[1];

    // What the page sends back carries that identity verbatim, in the
    // surface's one vocabulary…
    assert_eq!(
        crate::core::codec::from_json::<super::super::HostLobbyRecord>(&format!(
            r#"{{"kind":"set-viewscreen","monitor":"{}"}}"#,
            pressed.identity
        ))
        .unwrap(),
        super::super::HostLobbyRecord::SetViewscreen {
            monitor: pressed.identity.clone(),
        }
    );
    // …and it resolves to the display the operator was looking at.
    assert_eq!(
        set_viewscreen_action(pressed.identity.as_str()),
        LayoutAction::SetViewscreen {
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        }
    );
}

#[test]
fn the_row_names_each_monitor_by_what_the_os_reported_and_marks_the_viewscreen() {
    let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
    assert_eq!(payload.monitors.len(), 2);
    let tv = &payload.monitors[0];
    assert_eq!(tv.name.as_deref(), Some("BRAVIA"));
    assert_eq!((tv.width, tv.height), (3840, 2160));
    assert!(tv.primary);
    assert!(tv.viewscreen, "from_discovered seats it on the primary");
    let benq = &payload.monitors[1];
    assert!(!benq.primary);
    assert!(!benq.viewscreen);
    assert!(benq.stations.is_empty());
}

#[test]
fn moving_the_viewscreen_moves_the_mark_and_nothing_else() {
    let moved = layout()
        .apply(&LayoutAction::SetViewscreen {
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        })
        .expect("a free monitor takes the viewscreen");
    let payload = bridge_layout_payload(&moved, &two_monitors(), &[]);
    assert!(!payload.monitors[0].viewscreen);
    assert!(payload.monitors[1].viewscreen);
    assert!(
        payload.monitors[0].primary,
        "the OS primary does not move because the viewscreen did"
    );
}

#[test]
fn a_single_monitor_bridge_still_shows_its_one_monitor_marked() {
    // The inert row (issue #1330 AC4): one button, already the viewscreen,
    // and pressing it is the no-op the law defines.
    let one = identify(&[raw("BRAVIA", 3840, 2160, 0, true)]);
    let layout = BridgeLayout::from_discovered(&one, roster()).unwrap();
    let payload = bridge_layout_payload(&layout, &one, &[]);
    assert_eq!(payload.monitors.len(), 1);
    assert!(payload.monitors[0].viewscreen);
}

#[test]
fn a_monitor_the_layout_has_but_nothing_reported_is_left_out_of_the_row() {
    // The two only diverge while a reconcile is pending. Half a button —
    // named, sized 0x0 — is worse than no button.
    let payload = bridge_layout_payload(&layout(), &two_monitors()[..1], &[]);
    assert_eq!(payload.monitors.len(), 1);
    assert_eq!(payload.monitors[0].identity, "BRAVIA@3840x2160");
}

#[test]
fn a_monitor_holding_consoles_says_which_ones() {
    let seated = layout()
        .apply(&LayoutAction::AssignStation {
            station: StationId("helm".to_string()),
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        })
        .expect("a free non-viewscreen monitor takes a console");
    let payload = bridge_layout_payload(&seated, &two_monitors(), &[]);
    assert_eq!(payload.monitors[1].stations, vec!["helm".to_string()]);
}

#[test]
fn a_monitor_holding_a_console_the_layout_does_not_own_says_so_too() {
    // The `--pane`-shaped profile: its panes name a person rather than a
    // station, so the layout seats nothing for them — but the display
    // adapter opens a real console on that screen, and a press to move the
    // viewscreen there is refused. The button has to say so BEFORE the
    // press, not only in the sentence that comes back.
    use crate::native_host::bridge_profile::{BridgeProfile, DisplayEntry, PaneSlot};
    use crate::native_host::bridge_profile::{ROLE_STATION, ROLE_VIEWSCREEN};

    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: "BRAVIA@3840x2160".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: "BenQ EX@1920x1080".to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot::for_participant("Ada")],
        },
    ];
    let (adopted, _) = layout().adopt_profile(&profile.validate().unwrap());
    let payload = bridge_layout_payload(&adopted, &two_monitors(), &[]);
    assert_eq!(payload.monitors[1].stations, vec!["Ada".to_string()]);
    assert!(payload.monitors[0].stations.is_empty());
}

#[test]
fn a_refusal_crosses_as_an_id_and_its_parameters_rather_than_a_sentence() {
    let refusal = LayoutRefusal::UnknownMonitor {
        monitor: MonitorIdentity::new("Gone@1920x1080"),
    };
    let payload = bridge_layout_payload(
        &layout(),
        &two_monitors(),
        &[LayoutNotice::Refused(refusal)],
    );
    assert_eq!(payload.notices.len(), 1);
    assert_eq!(
        payload.notices[0].id,
        "server.bridge_layout.unknown_monitor"
    );
    assert_eq!(
        payload.notices[0].params.get("monitor").map(String::as_str),
        Some("Gone@1920x1080")
    );
}

#[test]
fn an_adoption_note_with_a_cause_renders_as_two_lines() {
    let note = LayoutAdoption::SeatRefused {
        station: StationId("helm".to_string()),
        monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        refusal: LayoutRefusal::MonitorFull {
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            occupants: vec![StationId("weapons".to_string())],
            panes: Vec::new(),
        },
    };
    let payload = bridge_layout_payload(&layout(), &two_monitors(), &[LayoutNotice::Adopted(note)]);
    assert_eq!(
        payload
            .notices
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "server.bridge_layout.adopt_seat_refused",
            "server.bridge_layout.monitor_full",
        ]
    );
}

#[test]
fn a_record_the_page_sends_is_tagged_by_the_verb_it_asks_for() {
    // The wire shape, pinned: `gui/`'s side builds this object by hand, so
    // a rename here is a button that silently does nothing. The tag stays
    // KEBAB (`set-viewscreen`) even though its siblings in
    // `HostLobbyRecord` are snake_case, because the page-side JS that
    // writes it shipped before the two vocabularies were folded into one
    // and there is no reason to make it a wire break.
    let json = r#"{"kind":"set-viewscreen","monitor":"BenQ EX@1920x1080"}"#;
    assert_eq!(
        crate::core::codec::from_json::<super::super::HostLobbyRecord>(json).unwrap(),
        super::super::HostLobbyRecord::SetViewscreen {
            monitor: "BenQ EX@1920x1080".to_string(),
        }
    );
    assert!(
        crate::core::codec::from_json::<super::super::HostLobbyRecord>(r#"{"monitor":"x"}"#)
            .is_err()
    );
}

#[test]
fn the_encoded_payload_is_byte_stable_for_an_unchanged_layout() {
    // The bridge drops a push identical to the last one accepted, which is
    // what keeps a quiet lobby free of `evaluate_script` calls on the
    // simulation's own thread. That only works if encoding the same layout
    // twice yields the same bytes — hence the ordered parameter map.
    let first = crate::core::codec::to_json(&bridge_layout_payload(
        &layout(),
        &two_monitors(),
        &[LayoutNotice::Refused(LayoutRefusal::MonitorFull {
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            occupants: vec![StationId("helm".to_string())],
            panes: Vec::new(),
        })],
    ))
    .unwrap();
    let second = crate::core::codec::to_json(&bridge_layout_payload(
        &layout(),
        &two_monitors(),
        &[LayoutNotice::Refused(LayoutRefusal::MonitorFull {
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            occupants: vec![StationId("helm".to_string())],
            panes: Vec::new(),
        })],
    ))
    .unwrap();
    assert_eq!(first, second);
    assert!(first.contains("\"identity\":\"BRAVIA@3840x2160\""));
}

// ── the per-station screen rows (issue #1331) ───────────────────────────

/// A three-monitor bridge, so a row can hold a full screen beside a free
/// one and still exclude the viewscreen's.
fn three_monitors() -> Vec<DiscoveredMonitor> {
    identify(&[
        raw("BRAVIA", 3840, 2160, 0, true),
        raw("BenQ EX", 1920, 1080, 3840, false),
        raw("Acer VG", 1280, 1024, 5760, false),
    ])
}

fn two_station_roster() -> Vec<StationId> {
    vec![
        StationId("helm".to_string()),
        StationId("weapons".to_string()),
    ]
}

fn row_for<'a>(payload: &'a BridgeLayoutPayload, station: &str) -> &'a StationRowPayload {
    payload
        .stations
        .iter()
        .find(|r| r.station == station)
        .expect("every roster station has a row")
}

fn screen_for<'a>(row: &'a StationRowPayload, identity: &str) -> &'a StationScreenPayload {
    row.monitors
        .iter()
        .find(|m| m.identity == identity)
        .expect("every drawn monitor has an entry")
}

#[test]
fn every_station_on_the_roster_gets_a_row_whether_or_not_its_console_is_open() {
    // The row is how a console is OPENED, so a station without one is a
    // station the operator could never seat.
    let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
    assert_eq!(
        payload
            .stations
            .iter()
            .map(|r| r.station.as_str())
            .collect::<Vec<_>>(),
        vec!["helm"]
    );
    assert!(payload.stations[0].assigned_to.is_none(), "nothing is open");
}

#[test]
fn a_rows_entries_carry_the_laws_own_answer_for_every_monitor() {
    // Including the viewscreen's, marked with its reason: the surface drops
    // that entry rather than the host omitting it, so the row acts on the
    // law's answer instead of re-deriving which screen is the shared view.
    let payload = bridge_layout_payload(&layout(), &two_monitors(), &[]);
    let helm = row_for(&payload, "helm");
    assert_eq!(
        screen_for(helm, "BRAVIA@3840x2160").choice,
        "excluded",
        "the viewscreen's own display is never offered a console"
    );
    assert_eq!(
        screen_for(helm, "BRAVIA@3840x2160").excluded.as_deref(),
        Some("is-viewscreen")
    );
    assert_eq!(screen_for(helm, "BenQ EX@1920x1080").choice, "eligible");
    assert!(screen_for(helm, "BenQ EX@1920x1080").excluded.is_none());
}

#[test]
fn a_seated_console_marks_its_own_screen_and_says_where_it_is() {
    let seated = layout()
        .apply(&LayoutAction::AssignStation {
            station: StationId("helm".to_string()),
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        })
        .expect("a free non-viewscreen monitor takes a console");
    let payload = bridge_layout_payload(&seated, &two_monitors(), &[]);
    let helm = row_for(&payload, "helm");
    assert_eq!(helm.assigned_to.as_deref(), Some("BenQ EX@1920x1080"));
    assert_eq!(screen_for(helm, "BenQ EX@1920x1080").choice, "selected");
}

#[test]
fn a_full_screen_is_excluded_with_its_reason_rather_than_left_out() {
    // Two consoles is the per-screen maximum, so a THIRD station's row
    // greys that screen — and says `full`, which is a different sentence
    // from `is-viewscreen` and a different button state.
    let mut roster = two_station_roster();
    roster.push(StationId("comms".to_string()));
    let base = BridgeLayout::from_discovered(&three_monitors(), roster)
        .expect("three monitors are a bridge");
    let seated = base
        .apply(&LayoutAction::AssignStation {
            station: StationId("helm".to_string()),
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        })
        .and_then(|l| {
            l.apply(&LayoutAction::AssignStation {
                station: StationId("weapons".to_string()),
                monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
            })
        })
        .expect("two consoles fit one screen");
    let payload = bridge_layout_payload(&seated, &three_monitors(), &[]);
    let comms = row_for(&payload, "comms");
    let benq = screen_for(comms, "BenQ EX@1920x1080");
    assert_eq!(benq.choice, "excluded");
    assert_eq!(benq.excluded.as_deref(), Some("full"));
    assert_eq!(
        screen_for(comms, "Acer VG@1280x1024").choice,
        "eligible",
        "and the free screen is still offered"
    );
    // A station already ON the full screen still reads as selected there —
    // capacity is judged after the no-op case, so re-pressing its own
    // button is not a refusal.
    assert_eq!(
        screen_for(row_for(&payload, "helm"), "BenQ EX@1920x1080").choice,
        "selected"
    );
}

#[test]
fn a_screen_an_authored_console_shares_offers_one_slot_and_then_none() {
    // The carried #1331 defect at the WIRE (issue #1332). The screen row is
    // built from the law's eligibility, so counting an authored `--pane`
    // console against the two-per-screen cap has to reach the page as a
    // greyed button — and the button has to be able to say who is on it,
    // which is why the monitor entry lists the authored label beside the
    // seats. A participant has no station row of their own, so without that
    // list the screen would grey for no visible reason at all.
    use crate::native_host::bridge_profile::{BridgeProfile, DisplayEntry, PaneSlot};
    use crate::native_host::bridge_profile::{ROLE_STATION, ROLE_VIEWSCREEN};

    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: "BRAVIA@3840x2160".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: "BenQ EX@1920x1080".to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot::for_participant("Ada")],
        },
    ];
    let base = BridgeLayout::from_discovered(&three_monitors(), two_station_roster())
        .expect("three monitors are a bridge");
    let (adopted, _) = base.adopt_profile(&profile.validate().unwrap());

    // One authored console: the screen still has a slot, so it is offered.
    let payload = bridge_layout_payload(&adopted, &three_monitors(), &[]);
    assert_eq!(
        screen_for(row_for(&payload, "helm"), "BenQ EX@1920x1080").choice,
        "eligible"
    );

    // Take it, and the screen is full for everybody else — the state that
    // used to read `eligible`, and used to let the two consoles overlap.
    let shared = adopted
        .apply(&LayoutAction::AssignStation {
            station: StationId("helm".to_string()),
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        })
        .expect("a screen with one authored console takes one station");
    let payload = bridge_layout_payload(&shared, &three_monitors(), &[]);
    let weapons = screen_for(row_for(&payload, "weapons"), "BenQ EX@1920x1080");
    assert_eq!(weapons.choice, "excluded");
    assert_eq!(weapons.excluded.as_deref(), Some("full"));
    assert_eq!(
        payload.monitors[1].stations,
        vec!["Ada".to_string(), "helm".to_string()],
        "and the button names both, so the greying has a visible reason — in the order they \
             are DRAWN on that screen (issue #1332's fix round), so a person looking at the glass \
             reads the button left to right and finds them where it says they are. Ada was \
             authored into the first pane slot, so she is the left half and the station seated \
             beside her is the right"
    );
    assert_eq!(
        payload.monitors[1].reserved,
        vec!["Ada".to_string()],
        "and the one of them the lobby cannot free is named as such: there is no unassign \
             button for an authored console, and a row that only said `full` would send the \
             operator hunting for one"
    );
}

#[test]
fn a_single_monitor_bridge_offers_a_station_no_screen_at_all() {
    // Issue #1331's single-monitor acceptance criterion, at the model
    // boundary: the one display is the viewscreen, the law excludes it, and
    // what is left is nothing — which is what the surface renders its
    // "consoles need a second monitor" line from.
    let one = identify(&[raw("BRAVIA", 3840, 2160, 0, true)]);
    let layout = BridgeLayout::from_discovered(&one, roster()).unwrap();
    let payload = bridge_layout_payload(&layout, &one, &[]);
    let helm = row_for(&payload, "helm");
    assert_eq!(helm.monitors.len(), 1);
    assert_eq!(helm.monitors[0].excluded.as_deref(), Some("is-viewscreen"));
    assert!(
        !helm.monitors.iter().any(|m| m.choice == "eligible"),
        "no screen may hold a console on a one-screen bridge"
    );
}

#[test]
fn a_monitor_the_row_cannot_draw_is_not_offered_to_a_station_either() {
    // The monitor row skips a display the layout knows but nothing
    // reported (half a button is worse than no button). Offering a station
    // that same display would offer a press the operator cannot see.
    let payload = bridge_layout_payload(&layout(), &two_monitors()[..1], &[]);
    assert_eq!(payload.monitors.len(), 1);
    assert_eq!(row_for(&payload, "helm").monitors.len(), 1);
}

#[test]
fn a_host_with_no_hull_has_a_monitor_row_and_no_station_rows() {
    // A delivery host, or one still in a world-less lobby: a lawful bridge
    // with an empty roster. The viewscreen still moves; there is simply
    // nothing to seat.
    let bare = BridgeLayout::from_discovered(&two_monitors(), []).unwrap();
    let payload = bridge_layout_payload(&bare, &two_monitors(), &[]);
    assert_eq!(payload.monitors.len(), 2);
    assert!(payload.stations.is_empty());
}

#[test]
fn the_two_station_verbs_round_trip_from_the_page_as_the_law_reads_them() {
    // The wire shape, pinned: `gui/`'s side builds these objects by hand.
    // They arrive in the surface's ONE vocabulary, beside the picks, and
    // KEBAB like the `set-viewscreen` they were written to match.
    use super::super::HostLobbyRecord;
    assert_eq!(
        crate::core::codec::from_json::<super::super::HostLobbyRecord>(
            r#"{"kind":"assign-station","station":"helm","monitor":"BenQ EX@1920x1080"}"#
        )
        .unwrap(),
        HostLobbyRecord::AssignStation {
            station: "helm".to_string(),
            monitor: "BenQ EX@1920x1080".to_string(),
        }
    );
    assert_eq!(
        assign_station_action("helm", "BenQ EX@1920x1080"),
        LayoutAction::AssignStation {
            station: StationId("helm".to_string()),
            monitor: MonitorIdentity::new("BenQ EX@1920x1080"),
        }
    );
    assert_eq!(
        crate::core::codec::from_json::<super::super::HostLobbyRecord>(
            r#"{"kind":"unassign-station","station":"helm"}"#
        )
        .unwrap(),
        HostLobbyRecord::UnassignStation {
            station: "helm".to_string(),
        }
    );
    assert_eq!(
        unassign_station_action("helm"),
        LayoutAction::UnassignStation {
            station: StationId("helm".to_string()),
        }
    );
    // There is no `move-station` verb, because the law has no move action:
    // assigning a seated station elsewhere IS the move.
    assert!(
        crate::core::codec::from_json::<super::super::HostLobbyRecord>(
            r#"{"kind":"move-station","station":"helm","monitor":"x"}"#
        )
        .is_err()
    );
}
