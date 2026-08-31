//! The bridge layout law, exhaustively (issue #1327).
//!
//! No display, no window, no lobby — the whole law is a data transform, so all
//! of it runs in the ordinary `cargo test` CI job. The rules under test are the
//! three the PRD names (one viewscreen, no console over it, two per screen), and
//! the two things the surrounding slices depend on: that a station is keyed by
//! its **station id** all the way through the profile round-trip, and that the
//! model answers *why* a monitor is greyed so no UI has to work it out again.

use super::*;

use crate::native_host::bridge_profile::{identify, RawMonitor, TouchMapping};

// ── fixtures ────────────────────────────────────────────────────────────────

const TV: &str = "BRAVIA@3840x2160";
const LEFT: &str = "BenQ EX@1920x1080";
const RIGHT: &str = "Acer VG@1280x1024";

fn m(id: &str) -> MonitorIdentity {
    MonitorIdentity::new(id)
}

fn s(id: &str) -> StationId {
    StationId(id.to_string())
}

fn roster() -> Vec<StationId> {
    ["helm", "weapons", "comms", "engineering"]
        .into_iter()
        .map(s)
        .collect()
}

/// The TV is the viewscreen; two other monitors are free.
fn bridge() -> BridgeLayout {
    BridgeLayout::new([m(TV), m(LEFT), m(RIGHT)], roster(), &m(TV)).expect("the TV is a monitor")
}

fn assign(layout: &BridgeLayout, station: &str, monitor: &str) -> BridgeLayout {
    layout
        .apply(&LayoutAction::AssignStation {
            station: s(station),
            monitor: m(monitor),
        })
        .unwrap_or_else(|e| panic!("assigning {station} to {monitor} was refused: {e}"))
}

fn assign_err(layout: &BridgeLayout, station: &str, monitor: &str) -> LayoutRefusal {
    layout
        .apply(&LayoutAction::AssignStation {
            station: s(station),
            monitor: m(monitor),
        })
        .expect_err("expected a refusal")
}

// ── construction ────────────────────────────────────────────────────────────

#[test]
fn a_new_bridge_has_its_viewscreen_and_nothing_else_placed() {
    let layout = bridge();
    assert_eq!(layout.viewscreen(), &m(TV));
    assert_eq!(layout.monitors().len(), 3);
    assert_eq!(layout.roster().len(), 4);
    assert!(layout.stations_on(&m(LEFT)).is_empty());
    assert!(layout
        .roster()
        .iter()
        .all(|st| layout.monitor_of(st).is_none()));
}

#[test]
fn a_viewscreen_that_is_not_one_of_the_monitors_is_refused() {
    let err = BridgeLayout::new([m(LEFT)], roster(), &m(TV)).unwrap_err();
    assert_eq!(err, LayoutRefusal::UnknownMonitor { monitor: m(TV) });
    // Including the degenerate case: no monitors is not a bridge.
    assert!(BridgeLayout::new([], roster(), &m(TV)).is_err());
}

#[test]
fn a_bridge_from_discovered_monitors_puts_the_viewscreen_on_the_primary() {
    let discovered = identify(&[
        RawMonitor {
            name: Some("BenQ EX".to_string()),
            physical_width: 1920,
            physical_height: 1080,
            position_x: 0,
            position_y: 0,
            scale_factor: 1.0,
            primary: false,
        },
        RawMonitor {
            name: Some("BRAVIA".to_string()),
            physical_width: 3840,
            physical_height: 2160,
            position_x: 1920,
            position_y: 0,
            scale_factor: 1.0,
            primary: true,
        },
    ]);
    let layout = BridgeLayout::from_discovered(&discovered, roster()).expect("two monitors");
    assert_eq!(
        layout.viewscreen(),
        &m(TV),
        "the primary monitor is where the lobby is already drawn"
    );
    assert!(BridgeLayout::from_discovered(&[], roster()).is_none());
}

// ── the three rules ─────────────────────────────────────────────────────────

#[test]
fn assigning_a_station_seats_it_and_leaves_the_previous_layout_untouched() {
    let before = bridge();
    let after = assign(&before, "helm", LEFT);
    assert_eq!(after.monitor_of(&s("helm")), Some(&m(LEFT)));
    assert_eq!(after.stations_on(&m(LEFT)), &[s("helm")]);
    assert!(
        before.monitor_of(&s("helm")).is_none(),
        "apply answers a NEW layout; the one it was given is not mutated"
    );
}

#[test]
fn a_station_may_not_open_on_the_viewscreens_monitor() {
    // Rule 2, the direct form: a console never covers the shared view.
    let err = assign_err(&bridge(), "helm", TV);
    assert_eq!(
        err,
        LayoutRefusal::StationOnViewscreenMonitor {
            station: s("helm"),
            monitor: m(TV),
        }
    );
    let msg = err.to_string();
    assert!(msg.contains(TV), "{msg}");
    assert!(msg.contains("helm"), "{msg}");
    assert!(msg.contains("viewscreen"), "{msg}");
}

#[test]
fn two_consoles_share_a_monitor_but_a_third_is_refused() {
    // Rule 3: the legibility bound, as a runtime precondition.
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    assert_eq!(layout.stations_on(&m(LEFT)), &[s("helm"), s("weapons")]);

    let err = assign_err(&layout, "comms", LEFT);
    assert_eq!(
        err,
        LayoutRefusal::MonitorFull {
            monitor: m(LEFT),
            occupants: vec![s("helm"), s("weapons")],
        }
    );
    let msg = err.to_string();
    assert!(msg.contains(LEFT), "{msg}");
    assert!(msg.contains("helm"), "{msg}");
    assert!(msg.contains("weapons"), "{msg}");
}

#[test]
fn moving_the_viewscreen_onto_a_monitor_holding_consoles_is_refused_not_forced() {
    // Rule 2's mirror, and the ruling that matters most: NO SILENT EVICTION.
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let err = layout
        .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
        .expect_err("the viewscreen may not land on an occupied monitor");
    assert_eq!(
        err,
        LayoutRefusal::ViewscreenMonitorHoldsStations {
            monitor: m(LEFT),
            stations: vec![s("helm"), s("weapons")],
        }
    );
    assert!(err.to_string().contains("unassign them first"));
    assert_eq!(
        layout.viewscreen(),
        &m(TV),
        "the refused move changed nothing"
    );

    // Unassign both and the same move is accepted — the operator's own two
    // steps, never one silent one.
    let emptied = layout
        .apply(&LayoutAction::UnassignStation { station: s("helm") })
        .unwrap()
        .apply(&LayoutAction::UnassignStation {
            station: s("weapons"),
        })
        .unwrap();
    let moved = emptied
        .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
        .expect("an empty monitor takes the viewscreen");
    assert_eq!(moved.viewscreen(), &m(LEFT));
}

#[test]
fn moving_the_viewscreen_frees_its_old_monitor_for_consoles() {
    let moved = bridge()
        .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
        .unwrap();
    // The TV is now an ordinary screen, and the monitor now showing the
    // viewscreen is the one that is off limits.
    let seated = assign(&moved, "helm", TV);
    assert_eq!(seated.monitor_of(&s("helm")), Some(&m(TV)));
    assert_eq!(
        assign_err(&seated, "weapons", LEFT),
        LayoutRefusal::StationOnViewscreenMonitor {
            station: s("weapons"),
            monitor: m(LEFT),
        }
    );
}

// ── the remaining refusals ──────────────────────────────────────────────────

#[test]
fn an_action_naming_a_monitor_this_bridge_does_not_have_is_refused() {
    let gone = m("Unplugged@1024x768");
    assert_eq!(
        assign_err(&bridge(), "helm", "Unplugged@1024x768"),
        LayoutRefusal::UnknownMonitor {
            monitor: gone.clone()
        }
    );
    assert_eq!(
        bridge()
            .apply(&LayoutAction::SetViewscreen {
                monitor: gone.clone()
            })
            .unwrap_err(),
        LayoutRefusal::UnknownMonitor { monitor: gone }
    );
}

#[test]
fn an_action_naming_a_station_off_the_roster_is_refused() {
    // A destroyer's saved layout opened against a cruiser, or a stale button.
    assert_eq!(
        assign_err(&bridge(), "flight-deck", LEFT),
        LayoutRefusal::UnknownStation {
            station: s("flight-deck"),
        }
    );
    assert_eq!(
        bridge()
            .apply(&LayoutAction::UnassignStation {
                station: s("flight-deck"),
            })
            .unwrap_err(),
        LayoutRefusal::UnknownStation {
            station: s("flight-deck"),
        }
    );
    // The station is judged before the monitor, so an action wrong in both ways
    // is refused as the wrong station rather than ambiguously.
    assert_eq!(
        assign_err(&bridge(), "flight-deck", "Unplugged@1024x768"),
        LayoutRefusal::UnknownStation {
            station: s("flight-deck"),
        }
    );
}

#[test]
fn closing_a_console_that_is_not_open_is_refused_rather_than_ignored() {
    let err = bridge()
        .apply(&LayoutAction::UnassignStation { station: s("helm") })
        .unwrap_err();
    assert_eq!(
        err,
        LayoutRefusal::StationNotAssigned { station: s("helm") }
    );
    assert!(err.to_string().contains("helm"));
}

// ── move, and the no-ops that are not refusals ──────────────────────────────

#[test]
fn assigning_a_seated_station_to_another_screen_moves_it_leaving_no_ghost() {
    // The PRD's "move a station's console by pressing a different screen
    // button": one action, and the old monitor's slot is genuinely freed.
    let layout = assign(&assign(&bridge(), "helm", LEFT), "helm", RIGHT);
    assert_eq!(layout.monitor_of(&s("helm")), Some(&m(RIGHT)));
    assert!(layout.stations_on(&m(LEFT)).is_empty());
    assert_eq!(
        layout.occupancy_of(&m(LEFT)).unwrap().free_slots,
        MAX_STATIONS_PER_MONITOR
    );
}

#[test]
fn re_pressing_the_screen_a_console_is_already_on_is_accepted_not_refused() {
    // Even when that screen is full — the station is one of the two occupants,
    // so counting it against the cap would refuse an action that changes
    // nothing.
    let full = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let again = assign(&full, "helm", LEFT);
    assert_eq!(
        again, full,
        "pressing the button under your finger is a no-op"
    );
}

#[test]
fn naming_the_current_viewscreen_again_is_accepted_not_refused() {
    let layout = bridge();
    let same = layout
        .apply(&LayoutAction::SetViewscreen { monitor: m(TV) })
        .expect("the viewscreen is already there");
    assert_eq!(same, layout);
}

// ── occupancy and eligibility: everything a greyed button row needs ─────────

#[test]
fn occupancy_names_each_monitors_consoles_and_its_remaining_room() {
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let occupancy = layout.occupancy();
    assert_eq!(occupancy.len(), 3);

    let tv = &occupancy[0];
    assert_eq!(tv.monitor, m(TV));
    assert!(tv.is_viewscreen);
    assert!(tv.stations.is_empty());
    assert_eq!(tv.free_slots, 0, "the viewscreen takes no console");

    let left = &occupancy[1];
    assert!(!left.is_viewscreen);
    assert_eq!(left.stations, vec![s("helm"), s("weapons")]);
    assert_eq!(left.free_slots, 0);

    let right = &occupancy[2];
    assert!(right.stations.is_empty());
    assert_eq!(right.free_slots, MAX_STATIONS_PER_MONITOR);

    assert!(layout.occupancy_of(&m("Unplugged@1024x768")).is_none());
}

#[test]
fn eligibility_greys_the_viewscreen_and_the_full_screen_and_says_which_is_which() {
    // The whole acceptance criterion: the UI does no logic of its own. For each
    // station, every monitor is Selected, Eligible, or Excluded WITH a reason.
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let comms = layout.eligibility_of(&s("comms")).expect("on the roster");

    assert_eq!(comms.assigned_to, None);
    assert_eq!(
        comms.choice_for(&m(TV)),
        Some(MonitorChoice::Excluded(ExclusionReason::IsViewscreen))
    );
    assert_eq!(
        comms.choice_for(&m(LEFT)),
        Some(MonitorChoice::Excluded(ExclusionReason::Full))
    );
    assert_eq!(comms.choice_for(&m(RIGHT)), Some(MonitorChoice::Eligible));
    assert_eq!(comms.eligible().collect::<Vec<_>>(), vec![&m(RIGHT)]);

    // The two reasons are distinguishable downstream without re-deriving them.
    assert_eq!(ExclusionReason::IsViewscreen.as_str(), "is-viewscreen");
    assert_eq!(ExclusionReason::Full.as_str(), "full");
    assert_eq!(ExclusionReason::Full.to_string(), "full");
    assert!(!MonitorChoice::Excluded(ExclusionReason::Full).is_offered());
    assert!(MonitorChoice::Eligible.is_offered());
    assert!(MonitorChoice::Selected.is_offered());
    assert_eq!(
        MonitorChoice::Excluded(ExclusionReason::IsViewscreen).exclusion(),
        Some(ExclusionReason::IsViewscreen)
    );
    assert_eq!(MonitorChoice::Eligible.exclusion(), None);
}

#[test]
fn a_seated_stations_own_screen_reads_selected_not_full() {
    // Its own monitor is where its console IS; greying it as "full" would tell
    // the operator the opposite of the truth.
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let helm = layout.eligibility_of(&s("helm")).unwrap();
    assert_eq!(helm.assigned_to, Some(m(LEFT)));
    assert_eq!(helm.choice_for(&m(LEFT)), Some(MonitorChoice::Selected));
    assert_eq!(helm.choice_for(&m(RIGHT)), Some(MonitorChoice::Eligible));
    assert_eq!(
        helm.eligible().collect::<Vec<_>>(),
        vec![&m(RIGHT)],
        "the screen it is already on is not somewhere to move it"
    );
}

#[test]
fn eligibility_covers_the_whole_roster_and_refuses_a_station_off_it() {
    let layout = bridge();
    let rows = layout.eligibility();
    assert_eq!(rows.len(), layout.roster().len());
    assert_eq!(rows[0].station, s("helm"));
    assert!(rows
        .iter()
        .all(|row| row.monitors.len() == layout.monitors().len()));
    assert_eq!(
        layout.eligibility_of(&s("flight-deck")).unwrap_err(),
        LayoutRefusal::UnknownStation {
            station: s("flight-deck"),
        }
    );
}

#[test]
fn a_single_monitor_bridge_offers_no_screen_at_all() {
    // The one-display case: the only monitor is the viewscreen, so every station
    // is excluded from it for that reason, and no console opens natively.
    let layout = BridgeLayout::new([m(TV)], roster(), &m(TV)).unwrap();
    for row in layout.eligibility() {
        assert_eq!(row.eligible().count(), 0);
        assert_eq!(
            row.choice_for(&m(TV)),
            Some(MonitorChoice::Excluded(ExclusionReason::IsViewscreen))
        );
    }
}

// ── the deterministic split ─────────────────────────────────────────────────

fn geometry(w: u32, h: u32) -> MonitorGeometry {
    MonitorGeometry {
        physical_width: w,
        physical_height: h,
        position_x: 0,
        position_y: 0,
        scale_factor: 1.0,
    }
}

#[test]
fn one_console_is_the_whole_screen_and_two_divide_it_side_by_side() {
    let one = assign(&bridge(), "helm", LEFT);
    assert_eq!(
        one.station_rects(&m(LEFT), &geometry(1920, 1080)),
        vec![(
            s("helm"),
            PaneRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        )]
    );

    let two = assign(&one, "weapons", LEFT);
    let rects = two.station_rects(&m(LEFT), &geometry(1920, 1080));
    assert_eq!(rects[0].0, s("helm"));
    assert_eq!(rects[1].0, s("weapons"));
    assert_eq!(rects[0].1.x, 0);
    assert_eq!(rects[1].1.x, 960);
    assert_eq!(rects[0].1.width + rects[1].1.width, 1920);
    assert_eq!(rects[0].1.height, 1080);

    // The viewscreen carries none, and neither does a monitor this bridge lacks.
    assert!(two.station_rects(&m(TV), &geometry(3840, 2160)).is_empty());
    assert!(two
        .station_rects(&m("Unplugged@1024x768"), &geometry(1024, 768))
        .is_empty());
}

// ── the profile round-trip, keyed by station id ─────────────────────────────

#[test]
fn a_two_consoles_on_one_screen_layout_reloads_unchanged() {
    // The acceptance criterion the whole station-id decision exists for: an
    // arrangement written to a profile, taken through the TOML the operator's
    // settings hold, and read back is the SAME layout — same viewscreen, same
    // screens, same order.
    let layout = assign(
        &assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT),
        "comms",
        RIGHT,
    );

    let text = layout.to_profile().to_toml().expect("serialises");
    let reloaded = BridgeProfile::from_toml(&text)
        .expect("parses")
        .validate()
        .expect("a layout writes a valid profile");
    let (adopted, notes) = layout.adopt_profile(&reloaded);

    assert_eq!(notes, Vec::new(), "nothing about it failed to fit");
    assert_eq!(adopted, layout, "the arrangement survives the round-trip");
    assert_eq!(adopted.viewscreen(), &m(TV));
    assert_eq!(adopted.stations_on(&m(LEFT)), &[s("helm"), s("weapons")]);
    assert_eq!(adopted.stations_on(&m(RIGHT)), &[s("comms")]);
    // And writing the adopted layout out again is byte-identical.
    assert_eq!(adopted.to_profile().to_toml().unwrap(), text);
}

#[test]
fn the_written_profile_keys_each_pane_by_station_id() {
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let text = layout.to_profile().to_toml().unwrap();

    assert!(text.contains(&format!("id = \"{TV}\"")), "{text}");
    assert!(text.contains("role = \"viewscreen\""), "{text}");
    assert!(text.contains("role = \"station\""), "{text}");
    assert!(text.contains("split = \"side_by_side\""), "{text}");
    assert!(text.contains("station = \"helm\""), "{text}");
    assert!(text.contains("station = \"weapons\""), "{text}");

    // A one-console screen leaves `split` out — it is ignored for a pane that is
    // the whole monitor, and an operator should not have to read past it.
    let single = assign(&bridge(), "helm", LEFT)
        .to_profile()
        .to_toml()
        .unwrap();
    assert!(!single.contains("split ="), "{single}");
}

#[test]
fn a_layouts_profile_always_names_the_viewscreen() {
    // The tie to `ProfileError::MissingViewscreen`: a layout cannot write the
    // shape that refusal exists to catch, because it always has a viewscreen.
    let empty = bridge().to_profile();
    assert!(empty.validate().is_ok());
    assert_eq!(empty.displays.len(), 1);
    assert_eq!(empty.displays[0].role, ROLE_VIEWSCREEN);
    assert!(assign(&bridge(), "helm", LEFT)
        .to_profile()
        .validate()
        .is_ok());
}

#[test]
fn saving_a_layout_over_a_profile_keeps_its_touch_and_media_assignments() {
    // A bridge profile is one file with more than displays in it. Writing the
    // arrangement back must not drop the room's touchscreen or its microphones.
    let mut existing = BridgeProfile::empty();
    existing.touch = vec![TouchMapping {
        device: "ELAN Touchscreen".to_string(),
        monitor: LEFT.to_string(),
    }];
    existing.media = vec![crate::native_host::bridge_media::MediaSurfaceEntry {
        surface: "viewscreen".to_string(),
        camera: Some("camera:Logitech BRIO".to_string()),
        microphones: vec!["mic:Blue Yeti".to_string()],
        outputs: vec!["output:Bridge Speakers".to_string()],
        allow_shared: Vec::new(),
    }];

    let mut saved = existing.clone();
    assign(&bridge(), "helm", LEFT).write_displays_into(&mut saved);

    assert_eq!(saved.touch, existing.touch);
    assert_eq!(saved.media, existing.media);
    assert_eq!(saved.displays.len(), 2);
    assert!(saved.validate().is_ok());
}

// ── adoption: a profile that does not fit this bridge ───────────────────────

/// A profile seating `station` on `monitor`, with the TV as the viewscreen.
fn profile_seating(monitor: &str, stations: &[&str]) -> ValidatedProfile {
    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: TV.to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: monitor.to_string(),
            role: ROLE_STATION.to_string(),
            split: Some(LAYOUT_SPLIT),
            panes: stations
                .iter()
                .copied()
                .map(PaneSlot::for_station)
                .collect(),
        },
    ];
    profile.validate().expect("the fixture is a valid profile")
}

#[test]
fn a_console_whose_screen_is_gone_is_left_unassigned_and_named() {
    // The LAN-party case: the saved layout was drawn on a bridge with a monitor
    // this one does not have. The rest of the arrangement still applies.
    let saved = profile_seating("Missing@1600x900", &["helm"]);
    let (adopted, notes) = bridge().adopt_profile(&saved);

    assert!(adopted.monitor_of(&s("helm")).is_none());
    assert_eq!(
        notes,
        vec![LayoutAdoption::SeatRefused {
            station: s("helm"),
            monitor: m("Missing@1600x900"),
            refusal: LayoutRefusal::UnknownMonitor {
                monitor: m("Missing@1600x900"),
            },
        }]
    );
    let msg = notes[0].to_string();
    assert!(msg.contains("helm"), "{msg}");
    assert!(msg.contains("left unassigned"), "{msg}");
}

#[test]
fn a_console_for_a_station_this_ship_does_not_have_is_named_not_seated() {
    // A cruiser's layout opened on a destroyer.
    let saved = profile_seating(LEFT, &["flight-deck"]);
    let (adopted, notes) = bridge().adopt_profile(&saved);
    assert!(adopted.stations_on(&m(LEFT)).is_empty());
    assert_eq!(
        notes,
        vec![LayoutAdoption::SeatRefused {
            station: s("flight-deck"),
            monitor: m(LEFT),
            refusal: LayoutRefusal::UnknownStation {
                station: s("flight-deck"),
            },
        }]
    );
}

#[test]
fn a_hand_authored_participant_profile_adopts_its_viewscreen_and_names_its_panes() {
    // `--profile` takes precedence over a saved layout, and a hand-authored one
    // seats participants rather than stations. There is nothing to key on, so
    // the panes are reported rather than minted into stations named for people.
    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: LEFT.to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: RIGHT.to_string(),
            role: ROLE_STATION.to_string(),
            split: Some(LAYOUT_SPLIT),
            panes: vec![
                PaneSlot::for_participant("Ada"),
                PaneSlot::for_participant("Grace"),
            ],
        },
    ];
    let validated = profile.validate().unwrap();
    let (adopted, notes) = bridge().adopt_profile(&validated);

    assert_eq!(
        adopted.viewscreen(),
        &m(LEFT),
        "the profile's viewscreen is honoured"
    );
    assert!(adopted.stations_on(&m(RIGHT)).is_empty());
    assert_eq!(
        notes,
        vec![
            LayoutAdoption::PaneNamesNoStation {
                monitor: m(RIGHT),
                label: "Ada".to_string(),
            },
            LayoutAdoption::PaneNamesNoStation {
                monitor: m(RIGHT),
                label: "Grace".to_string(),
            },
        ]
    );
    assert!(notes[0].to_string().contains("Ada"));
}

#[test]
fn the_station_id_is_what_is_seated_even_when_the_pane_is_labelled_otherwise() {
    // The keying decision, stated as a test: a pane whose label is a person's
    // name but which carries `station = "helm"` seats HELM. The layout never
    // reads a participant name as a station.
    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: TV.to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: LEFT.to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot {
                label: "Ada".to_string(),
                station: Some("helm".to_string()),
            }],
        },
    ];
    let (adopted, notes) = bridge().adopt_profile(&profile.validate().unwrap());
    assert_eq!(notes, Vec::new());
    assert_eq!(adopted.stations_on(&m(LEFT)), &[s("helm")]);
    assert!(adopted.monitor_of(&s("comms")).is_none());
}

#[test]
fn adoption_replaces_the_arrangement_rather_than_adding_to_it() {
    let current = assign(&bridge(), "engineering", RIGHT);
    let saved = profile_seating(LEFT, &["helm", "weapons"]);
    let (adopted, notes) = current.adopt_profile(&saved);

    assert_eq!(notes, Vec::new());
    assert_eq!(adopted.stations_on(&m(LEFT)), &[s("helm"), s("weapons")]);
    assert!(
        adopted.stations_on(&m(RIGHT)).is_empty(),
        "the arrangement being adopted is the whole arrangement"
    );
}

#[test]
fn a_profile_seating_one_station_on_two_screens_keeps_the_first_and_says_so() {
    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: TV.to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: LEFT.to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot {
                label: "Ada".to_string(),
                station: Some("helm".to_string()),
            }],
        },
        DisplayEntry {
            id: RIGHT.to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot {
                label: "Grace".to_string(),
                station: Some("helm".to_string()),
            }],
        },
    ];
    let (adopted, notes) = bridge().adopt_profile(&profile.validate().unwrap());
    assert_eq!(adopted.stations_on(&m(LEFT)), &[s("helm")]);
    assert!(adopted.stations_on(&m(RIGHT)).is_empty());
    assert_eq!(
        notes,
        vec![LayoutAdoption::StationNamedTwice {
            station: s("helm"),
            monitor: m(RIGHT),
        }]
    );
}

#[test]
fn adopting_a_profile_whose_viewscreen_monitor_is_gone_keeps_the_current_one() {
    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: "Missing@1600x900".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: LEFT.to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot::for_station("helm")],
        },
    ];
    let (adopted, notes) = bridge().adopt_profile(&profile.validate().unwrap());

    assert_eq!(
        adopted.viewscreen(),
        &m(TV),
        "the viewscreen stays where the lobby is being drawn"
    );
    assert_eq!(
        adopted.stations_on(&m(LEFT)),
        &[s("helm")],
        "the rest of the arrangement still applies"
    );
    assert_eq!(
        notes,
        vec![LayoutAdoption::ViewscreenRefused {
            monitor: m("Missing@1600x900"),
            refusal: LayoutRefusal::UnknownMonitor {
                monitor: m("Missing@1600x900"),
            },
        }]
    );
    assert!(notes[0].to_string().contains("stays where it is"));
}

#[test]
fn adopting_a_profile_that_seats_a_console_on_this_bridges_viewscreen_refuses_it() {
    // Structurally impossible to author INSIDE one profile — a monitor appears
    // once — but reachable when the profile's own viewscreen is missing, so the
    // bridge keeps a viewscreen the profile seats a console on. The same law
    // that refuses the button press refuses this.
    let mut profile = BridgeProfile::empty();
    profile.displays = vec![
        DisplayEntry {
            id: "Missing@1600x900".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: TV.to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot::for_station("helm")],
        },
    ];
    let (adopted, notes) = bridge().adopt_profile(&profile.validate().unwrap());

    assert!(adopted.stations_on(&m(TV)).is_empty());
    assert!(notes.contains(&LayoutAdoption::SeatRefused {
        station: s("helm"),
        monitor: m(TV),
        refusal: LayoutRefusal::StationOnViewscreenMonitor {
            station: s("helm"),
            monitor: m(TV),
        },
    }));
}
