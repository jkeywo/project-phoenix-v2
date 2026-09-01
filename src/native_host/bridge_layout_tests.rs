//! The bridge layout law, exhaustively (issue #1327).
//!
//! No display, no window, no lobby — the whole law is a data transform, so all
//! of it runs in the ordinary `cargo test` CI job. The rules under test are the
//! three the PRD names (one viewscreen, no console over it, two per screen), and
//! the two things the surrounding slices depend on: that a station is keyed by
//! its **station id** all the way through the profile round-trip, and that the
//! model answers *why* a monitor is greyed so no UI has to work it out again.

use super::*;

use crate::native_host::bridge_profile::{identify, RawMonitor, TouchMapping, ValidatedDisplay};

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
            panes: Vec::new(),
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
            panes: Vec::new(),
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
    // The boundary of the no-op doctrine: closing an unseated console is a
    // no-op, but closing one for a station this ship does not have is a stale
    // button, and is still refused as that.
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
fn closing_a_console_that_is_not_open_is_a_no_op_not_a_refusal() {
    // The third no-op. It is not only a double-press: a lobby's off button gets
    // pressed by two clients at once, and refusing the second would make an
    // ordinary race look like a fault.
    let layout = bridge();
    let same = layout
        .apply(&LayoutAction::UnassignStation { station: s("helm") })
        .expect("closing a console that is not open changes nothing");
    assert_eq!(same, layout);

    // Twice over from a seated start: the first closes it, the second is the
    // no-op, and the two answers are the same layout.
    let seated = assign(&bridge(), "helm", LEFT);
    let closed = seated
        .apply(&LayoutAction::UnassignStation { station: s("helm") })
        .unwrap();
    assert!(closed.monitor_of(&s("helm")).is_none());
    assert_eq!(
        closed
            .apply(&LayoutAction::UnassignStation { station: s("helm") })
            .unwrap(),
        closed
    );
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
        Some(MAX_STATIONS_PER_MONITOR)
    );
}

#[test]
fn moving_a_console_off_a_shared_screen_and_back_swaps_the_two_halves() {
    // Seat order is the order consoles were assigned, and it is part of the
    // layout's identity AND of what is drawn: the console that stayed put is now
    // the earlier of the two, so the halves swap. Pinned as a contract rather
    // than left as an accident, because it is what the operator sees.
    let both = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    assert_eq!(both.stations_on(&m(LEFT)), &[s("helm"), s("weapons")]);

    let away = both
        .apply(&LayoutAction::UnassignStation { station: s("helm") })
        .unwrap();
    let back = assign(&away, "helm", LEFT);

    assert_eq!(
        back.stations_on(&m(LEFT)),
        &[s("weapons"), s("helm")],
        "the console that stayed is now the left half"
    );
    assert_ne!(
        back, both,
        "seat order is part of PartialEq — these are not the same arrangement"
    );

    // And it is visible, not just structural: the rendered halves swap with it.
    let rects = back.station_rects(&m(LEFT), &geometry(1920, 1080));
    assert_eq!(rects[0].0, s("weapons"));
    assert_eq!(rects[0].1.x, 0);
    assert_eq!(rects[1].0, s("helm"));
    assert_eq!(rects[1].1.x, 960);

    // The file records the order too, so the swap survives a reload rather than
    // being undone by one.
    let reloaded = BridgeProfile::from_toml(&back.to_profile().to_toml().unwrap())
        .unwrap()
        .validate()
        .unwrap();
    let (adopted, notes) = back.adopt_profile(&reloaded);
    assert_eq!(notes, Vec::new());
    assert_eq!(adopted, back);
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
    assert_eq!(
        tv.free_slots, None,
        "the viewscreen takes no console — None, never Some(0), which would read \
         as a full screen somebody could free a slot on"
    );

    let left = &occupancy[1];
    assert!(!left.is_viewscreen);
    assert_eq!(left.stations, vec![s("helm"), s("weapons")]);
    assert_eq!(
        left.free_slots,
        Some(0),
        "genuinely full, and that is a number"
    );

    let right = &occupancy[2];
    assert!(right.stations.is_empty());
    assert_eq!(right.free_slots, Some(MAX_STATIONS_PER_MONITOR));

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

#[test]
fn moving_one_console_out_of_a_shared_screen_gives_the_other_the_whole_of_it() {
    // The 2-up boundary a move crosses (issue #1332). Pressing another screen's
    // button for a station already seated IS the move — the law has no move
    // action — and what it leaves behind is the interesting half: the screen it
    // left goes back to one console at full width, and un-greys for every OTHER
    // station's row at the same moment.
    let two = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    assert_eq!(
        two.eligibility_of(&s("comms"))
            .unwrap()
            .choice_for(&m(LEFT)),
        Some(MonitorChoice::Excluded(ExclusionReason::Full)),
        "while it holds two, it is greyed on the third station's row"
    );

    let moved = assign(&two, "weapons", RIGHT);
    assert_eq!(moved.stations_on(&m(LEFT)), &[s("helm")]);
    assert_eq!(moved.stations_on(&m(RIGHT)), &[s("weapons")]);
    assert_eq!(
        moved.station_rects(&m(LEFT), &geometry(1920, 1080)),
        vec![(
            s("helm"),
            PaneRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        )],
        "the console that stayed grows back to the whole screen"
    );
    assert_eq!(
        moved.station_rects(&m(RIGHT), &geometry(1280, 1024))[0].1,
        PaneRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 1024
        },
        "and the one that moved has the whole of its new screen"
    );

    // The un-greying, on EVERY other station's row rather than only the one
    // that moved — the acceptance criterion, asked of the law.
    for station in ["comms", "engineering"] {
        assert_eq!(
            moved
                .eligibility_of(&s(station))
                .unwrap()
                .choice_for(&m(LEFT)),
            Some(MonitorChoice::Eligible),
            "{station}'s row offers the vacated slot"
        );
    }
    assert_eq!(
        moved.occupancy_of(&m(LEFT)).unwrap().free_slots,
        Some(1),
        "one slot back"
    );
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
        adopted.reserved_on(&m(RIGHT)),
        &["Ada".to_string(), "Grace".to_string()],
        "nothing was SEATED, but the screen is not free either: the profile opens a \
         console on it, and rule 2's mirror has to know"
    );
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

// ── an authored console is an occupant (issue #1330) ────────────────────────

/// `bridge()` with a hand-authored `--pane` profile adopted: the viewscreen
/// stays on the TV, and Ada's participant console is opened on `LEFT`.
fn with_an_authored_console() -> BridgeLayout {
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
            panes: vec![PaneSlot::for_participant("Ada")],
        },
    ];
    bridge().adopt_profile(&profile.validate().unwrap()).0
}

#[test]
fn the_viewscreen_may_not_move_onto_a_console_the_layout_does_not_own() {
    // The gap this closes: `adopt_profile` seats only panes that name a
    // station, so a `--pane`-shaped profile seated NOTHING and the law read
    // that screen as free — while the display adapter had opened a real
    // borderless-fullscreen console on it. Rule 2's mirror applies to any
    // console, not only to the ones this law is able to move.
    let layout = with_an_authored_console();
    let err = layout
        .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
        .expect_err("Ada's console is on it");
    assert_eq!(
        err,
        LayoutRefusal::ViewscreenMonitorHoldsStations {
            monitor: m(LEFT),
            stations: Vec::new(),
            panes: vec!["Ada".to_string()],
        }
    );
    assert!(err.to_string().contains("\"Ada\""));
    let params = err.params();
    let stations = &params.iter().find(|(k, _)| *k == "stations").unwrap().1;
    assert_eq!(
        stations, "Ada",
        "and the player-visible parameter names who is on it, unquoted"
    );
}

#[test]
fn a_refusal_names_the_seated_consoles_and_the_authored_ones_together() {
    // One monitor, both kinds of occupant. The operator is looking at one
    // screen and needs one list of what is on it.
    let layout = assign(&with_an_authored_console(), "helm", LEFT);
    let err = layout
        .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
        .expect_err("both are on it");
    let params = err.params();
    let stations = &params.iter().find(|(k, _)| *k == "stations").unwrap().1;
    assert_eq!(stations, "helm, Ada", "seats first, then what was authored");
}

#[test]
fn an_authored_console_is_an_occupant_but_never_a_station() {
    // Which is exactly why it is not a faked `StationId`: it would otherwise
    // show up on the roster-driven rows as a station no ship has, with an
    // unassign button that could not work.
    let layout = with_an_authored_console();
    assert!(
        layout.stations_on(&m(LEFT)).is_empty(),
        "there is no station seated there"
    );
    assert!(
        !layout.roster().iter().any(|s| s.0 == "Ada"),
        "and Ada is not on the roster"
    );
    assert_eq!(
        layout.occupants_on(&m(LEFT)),
        vec!["Ada".to_string()],
        "but the row must still draw the screen as taken"
    );
    let occupancy = layout.occupancy_of(&m(LEFT)).unwrap();
    assert_eq!(occupancy.reserved, vec!["Ada".to_string()]);
    assert_eq!(
        occupancy.free_slots,
        Some(MAX_STATIONS_PER_MONITOR - 1),
        "and it fills one of that screen's two slots (issue #1332) — it is a console on the \
         glass whoever opened it, so the screen has room for exactly one more"
    );
    assert_eq!(
        layout
            .eligibility()
            .iter()
            .map(|e| e.station.0.clone())
            .collect::<Vec<_>>(),
        vec!["helm", "weapons", "comms", "engineering"],
        "and no screen row appeared for it"
    );
}

// ── an authored console fills a slot too (issue #1332) ──────────────────────
//
// #1331's carried defect, and the whole of it: `assign` and `Excluded(Full)`
// counted only `seats`. So a screen already holding a hand-authored `--pane`
// console offered a station a slot, the law took the press, and the display
// adapter laid the new console across the whole monitor on top of the old one.
// Counting is half the fix (here); the other half is that the adapter lays every
// pane on a screen out together (`surface_rects`, below).

#[test]
fn a_station_may_share_a_screen_with_an_authored_console_but_not_crowd_it() {
    // One slot left, and exactly one. The first press is lawful — two consoles
    // on a screen is what the screen is for — and the second is the density
    // refusal, reached by the same rule two seated stations reach it by.
    let layout = with_an_authored_console();
    assert_eq!(
        layout
            .eligibility_of(&s("helm"))
            .unwrap()
            .choice_for(&m(LEFT)),
        Some(MonitorChoice::Eligible),
        "one authored console leaves room for one more"
    );

    let shared = assign(&layout, "helm", LEFT);
    assert_eq!(
        shared.occupancy_of(&m(LEFT)).unwrap().free_slots,
        Some(0),
        "and then the screen is full"
    );
    assert_eq!(
        shared
            .eligibility_of(&s("weapons"))
            .unwrap()
            .choice_for(&m(LEFT)),
        Some(MonitorChoice::Excluded(ExclusionReason::Full)),
        "so every OTHER station's row greys it, on the law's own answer"
    );
    assert_eq!(
        shared
            .eligibility_of(&s("helm"))
            .unwrap()
            .choice_for(&m(LEFT)),
        Some(MonitorChoice::Selected),
        "while the station that is on it still reads as selected — capacity is judged \
         after the no-op case, so re-pressing its own button is not a refusal"
    );
}

#[test]
fn the_third_console_on_a_screen_is_refused_whoever_opened_the_first_two() {
    // The refusal itself, and the sentence it carries: it counts and names BOTH
    // kinds, because an operator looking at that screen can see two consoles on
    // it and a message saying "already holds 1" would read as a bug in the host.
    let shared = assign(&with_an_authored_console(), "helm", LEFT);
    let err = assign_err(&shared, "weapons", LEFT);
    assert_eq!(
        err,
        LayoutRefusal::MonitorFull {
            monitor: m(LEFT),
            occupants: vec![s("helm")],
            panes: vec!["Ada".to_string()],
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("2 console(s)"), "{msg}");
    assert!(msg.contains("\"helm\", \"Ada\""), "{msg}");
    let params = err.params();
    let stations = &params.iter().find(|(k, _)| *k == "stations").unwrap().1;
    assert_eq!(
        stations, "helm, Ada",
        "and the player-visible parameter names both, unquoted"
    );
}

#[test]
fn a_screen_with_two_authored_consoles_offers_no_slot_at_all() {
    // The other end of the count: `MAX_PANES_PER_STATION` authored panes fill
    // the screen on their own, so no station is ever offered it.
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
            split: Some(LAYOUT_SPLIT),
            panes: vec![
                PaneSlot::for_participant("Ada"),
                PaneSlot::for_participant("Grace"),
            ],
        },
    ];
    let layout = bridge().adopt_profile(&profile.validate().unwrap()).0;
    assert_eq!(layout.occupancy_of(&m(LEFT)).unwrap().free_slots, Some(0));
    assert_eq!(
        layout
            .eligibility_of(&s("helm"))
            .unwrap()
            .choice_for(&m(LEFT)),
        Some(MonitorChoice::Excluded(ExclusionReason::Full))
    );
    assert_eq!(
        assign_err(&layout, "helm", LEFT),
        LayoutRefusal::MonitorFull {
            monitor: m(LEFT),
            occupants: Vec::new(),
            panes: vec!["Ada".to_string(), "Grace".to_string()],
        }
    );
}

#[test]
fn a_seated_station_tiles_beside_an_authored_console_rather_than_over_it() {
    // The geometry half. `surface_rects` is ONE `pane_rects` call over the whole
    // occupancy, so the two consoles cover the screen exactly once — which is
    // the bug this closes: two tilings each handed the full rectangle.
    let shared = assign(&with_an_authored_console(), "helm", LEFT);
    let rects = shared.surface_rects(&m(LEFT), &geometry(1920, 1080));
    assert_eq!(
        rects
            .iter()
            .map(|(o, _)| o.name().to_string())
            .collect::<Vec<_>>(),
        vec!["Ada".to_string(), "helm".to_string()],
        "the authored console was there from boot, so it keeps the left half"
    );
    assert_eq!((rects[0].1.x, rects[0].1.width), (0, 960));
    assert_eq!((rects[1].1.x, rects[1].1.width), (960, 960));
    assert_eq!(rects[0].1.height, 1080);
    assert_eq!(rects[1].1.height, 1080);

    // And which kind each is survives the trip, because the adapter treats them
    // differently: one is the layout's to close, the other only to place.
    assert_eq!(rects[0].0.station(), None);
    assert_eq!(rects[1].0.station(), Some(&s("helm")));

    // `station_rects` is that same tiling filtered, NOT a tiling of its own —
    // so a caller that only wants the stations still gets the honest half.
    assert_eq!(
        shared.station_rects(&m(LEFT), &geometry(1920, 1080)),
        vec![(s("helm"), rects[1].1)]
    );
}

#[test]
fn an_authored_console_alone_still_covers_its_whole_screen() {
    // The one-console case, so the count that changed cannot have quietly
    // shrunk an authored pane that has the screen to itself.
    let layout = with_an_authored_console();
    let rects = layout.surface_rects(&m(LEFT), &geometry(1920, 1080));
    assert_eq!(rects.len(), 1);
    assert_eq!((rects[0].1.x, rects[0].1.width), (0, 1920));
    assert!(layout
        .station_rects(&m(LEFT), &geometry(1920, 1080))
        .is_empty());
}

#[test]
fn a_profile_the_layout_itself_wrote_reserves_nothing() {
    // The other direction, unchanged: every pane a layout writes names a
    // station, so a saved arrangement coming back is seats and nothing else.
    let saved = assign(&bridge(), "helm", LEFT).to_validated_profile();
    let (adopted, notes) = bridge().adopt_profile(&saved);
    assert_eq!(adopted.stations_on(&m(LEFT)), &[s("helm")]);
    assert!(adopted.reserved_on(&m(LEFT)).is_empty());
    assert!(notes.is_empty());
    let err = adopted
        .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
        .expect_err("helm's console is on it");
    assert_eq!(
        err,
        LayoutRefusal::ViewscreenMonitorHoldsStations {
            monitor: m(LEFT),
            stations: vec![s("helm")],
            panes: Vec::new(),
        },
        "the refusal an operator has always got, unchanged"
    );
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
    // `validate` now refuses this shape at the prompt
    // (`ProfileError::DuplicatePaneStation` — see
    // `two_panes_naming_the_same_station_are_refused` in the profile tests), so
    // the profile is assembled here by hand. The transition law keeps its own
    // answer regardless: `ValidatedProfile` is a public struct, adoption is the
    // one door every seat comes through, and a law that trusted its input to have
    // been checked elsewhere is a law with a hole in it.
    let station_on = |monitor: &str, label: &str| ValidatedDisplay {
        identity: m(monitor),
        role: DisplayRole::Station {
            split: LAYOUT_SPLIT,
            panes: vec![PaneSlot {
                label: label.to_string(),
                station: Some("helm".to_string()),
            }],
        },
    };
    let profile = ValidatedProfile {
        displays: vec![
            ValidatedDisplay {
                identity: m(TV),
                role: DisplayRole::Viewscreen,
            },
            station_on(LEFT, "Ada"),
            station_on(RIGHT, "Grace"),
        ],
        touch: Vec::new(),
        media: Default::default(),
    };
    let (adopted, notes) = bridge().adopt_profile(&profile);
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

#[test]
fn a_layouts_validated_profile_is_the_same_arrangement() {
    // The infallible conversion consumers take: no `Result` to launder, because a
    // lawful layout cannot write a profile `validate` rejects. What it produces
    // is the arrangement itself — adopting it back is the identity.
    let layout = assign(
        &assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT),
        "comms",
        RIGHT,
    );
    let validated = layout.to_validated_profile();
    let (adopted, notes) = layout.adopt_profile(&validated);
    assert_eq!(notes, Vec::new());
    assert_eq!(adopted, layout);

    // Including the empty bridge, which is what the lobby opens on.
    let empty = bridge();
    assert_eq!(empty.to_validated_profile().displays.len(), 1);
    assert_eq!(
        empty.to_validated_profile().displays[0].role,
        DisplayRole::Viewscreen
    );
    // And it agrees with the fallible form, so the two cannot drift.
    assert_eq!(
        layout.to_validated_profile(),
        layout.to_profile().validate().unwrap()
    );
}

// ── reconciling a layout with a bridge that changed under it ────────────────

/// A discovered monitor with a chosen identity — the shape a plug event brings.
/// The geometry is nominal: reconcile keys on identity, and only reads `primary`.
fn discovered(id: &str, primary: bool) -> DiscoveredMonitor {
    DiscoveredMonitor {
        identity: m(id),
        geometry: geometry(1920, 1080),
        name: Some(id.to_string()),
        primary,
    }
}

/// This bridge's own three monitors, TV primary — the "nothing changed" report.
fn all_three() -> Vec<DiscoveredMonitor> {
    vec![
        discovered(TV, true),
        discovered(LEFT, false),
        discovered(RIGHT, false),
    ]
}

#[test]
fn an_authored_console_follows_its_screen_through_a_reconcile() {
    // A monitor that is still plugged in is still carrying whatever the
    // profile opened on it — and a monitor that has gone took its console with
    // it, so the screen that replaced it is genuinely free.
    let layout = with_an_authored_console();
    let (kept, _) = layout.reconcile(&all_three(), roster());
    assert_eq!(kept.reserved_on(&m(LEFT)), &["Ada".to_string()]);
    assert!(
        kept.apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
            .is_err(),
        "so the viewscreen still may not move onto it"
    );

    let (gone, _) = layout.reconcile(&[discovered(TV, true), discovered(RIGHT, false)], roster());
    assert!(
        gone.reserved_on(&m(LEFT)).is_empty(),
        "the screen it was on is not part of this bridge any more"
    );
    assert!(gone
        .apply(&LayoutAction::SetViewscreen { monitor: m(RIGHT) })
        .is_ok());
}

#[test]
fn reconciling_against_the_same_bridge_changes_nothing_and_says_nothing() {
    let layout = assign(
        &assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT),
        "comms",
        RIGHT,
    );
    let (same, notes) = layout.reconcile(&all_three(), roster());
    assert_eq!(
        notes,
        Vec::new(),
        "nothing changed, so there is nothing to say"
    );
    assert_eq!(
        same, layout,
        "identical — including the seat order on the shared screen"
    );
}

#[test]
fn unplugging_the_viewscreens_monitor_falls_back_to_the_primary_and_says_so() {
    // Never silently: the operator chose that screen, and a fallback that left no
    // trace would look exactly like nothing having happened.
    let layout = assign(&bridge(), "helm", LEFT);
    let (next, notes) = layout.reconcile(
        &[discovered(LEFT, false), discovered(RIGHT, true)],
        roster(),
    );

    assert_eq!(
        next.viewscreen(),
        &m(RIGHT),
        "primary, since the TV is gone"
    );
    assert_eq!(
        notes,
        vec![LayoutAdoption::ViewscreenMonitorGone {
            monitor: m(TV),
            replacement: m(RIGHT),
        }]
    );
    let msg = notes[0].to_string();
    assert!(msg.contains(TV), "{msg}");
    assert!(msg.contains(RIGHT), "{msg}");
    assert!(
        next.stations_on(&m(LEFT)) == [s("helm")],
        "the rest of the arrangement is untouched"
    );

    // Primary-ELSE-FIRST: with nothing flagged primary the first reported
    // monitor takes it, matching `from_discovered`'s own fallback.
    let (first, _) = bridge().reconcile(
        &[discovered(RIGHT, false), discovered(LEFT, false)],
        roster(),
    );
    assert_eq!(first.viewscreen(), &m(RIGHT));
}

#[test]
fn reconciling_does_not_reset_a_surviving_viewscreen_to_the_primary() {
    // The plug event this whole method exists for: somebody moved the viewscreen
    // off the primary deliberately, then a DIFFERENT screen was re-plugged. The
    // monitor set is re-reported, and the operator's choice must survive it.
    let moved = bridge()
        .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
        .unwrap();
    let seated = assign(&moved, "helm", TV);

    let (next, notes) = seated.reconcile(&all_three(), roster());
    assert_eq!(
        next.viewscreen(),
        &m(LEFT),
        "the TV is primary, but the operator chose the LEFT screen"
    );
    assert_eq!(notes, Vec::new());
    assert_eq!(next.stations_on(&m(TV)), &[s("helm")]);
    assert_eq!(next, seated);
}

#[test]
fn unplugging_a_stations_monitor_leaves_it_unassigned_and_names_it() {
    // Not re-homed onto a screen that is still there: that is a rearrangement
    // nobody asked for, and it is refused one slice up for the same reason.
    let layout = assign(&assign(&bridge(), "helm", LEFT), "comms", RIGHT);
    let (next, notes) =
        layout.reconcile(&[discovered(TV, true), discovered(LEFT, false)], roster());

    assert_eq!(next.viewscreen(), &m(TV));
    assert_eq!(
        next.stations_on(&m(LEFT)),
        &[s("helm")],
        "this one survived"
    );
    assert!(next.monitor_of(&s("comms")).is_none());
    assert_eq!(
        notes,
        vec![LayoutAdoption::StationMonitorGone {
            station: s("comms"),
            monitor: m(RIGHT),
        }]
    );
    let msg = notes[0].to_string();
    assert!(msg.contains("comms"), "{msg}");
    assert!(msg.contains("left unassigned"), "{msg}");
}

#[test]
fn a_station_that_leaves_the_roster_has_its_console_closed_and_named() {
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let shrunk = ["helm", "comms", "engineering"].map(s).to_vec();
    let (next, notes) = layout.reconcile(&all_three(), shrunk);

    assert_eq!(next.roster().len(), 3);
    assert_eq!(
        next.stations_on(&m(LEFT)),
        &[s("helm")],
        "helm keeps its seat"
    );
    assert!(next.monitor_of(&s("weapons")).is_none());
    assert_eq!(
        notes,
        vec![LayoutAdoption::StationOffRoster {
            station: s("weapons"),
            monitor: m(LEFT),
        }]
    );
    assert!(notes[0].to_string().contains("weapons"));
}

#[test]
fn a_station_new_to_the_roster_arrives_unassigned_with_nothing_to_report() {
    // An unplaced console is the ordinary state of a station nobody has put on a
    // screen yet, not a degradation — so there is no note to make.
    let layout = assign(&bridge(), "helm", LEFT);
    let mut grown = roster();
    grown.push(s("flight-deck"));
    let (next, notes) = layout.reconcile(&all_three(), grown);

    assert_eq!(notes, Vec::new());
    assert_eq!(next.roster().len(), 5);
    assert!(next.monitor_of(&s("flight-deck")).is_none());
    assert_eq!(next.stations_on(&m(LEFT)), &[s("helm")]);
    // And it is a full citizen of the new bridge: it has a screen row.
    let row = next
        .eligibility_of(&s("flight-deck"))
        .expect("on the new roster");
    assert_eq!(
        row.eligible().collect::<Vec<_>>(),
        vec![&m(LEFT), &m(RIGHT)]
    );
}

#[test]
fn the_viewscreens_fallback_prefers_a_screen_that_is_holding_nothing() {
    // The fallback is a MOVE, and rule 2 governs moves. Chosen primary-else-first
    // without asking what is on the screen, unplugging the TV on this bridge
    // would have dropped the shared view straight on top of Ada's authored
    // console — silently, because a reserved pane earns no `SeatRefused` and
    // cannot be unassigned afterwards — while RIGHT sat there empty.
    let layout = with_an_authored_console();
    let (next, notes) = layout.reconcile(
        &[discovered(LEFT, true), discovered(RIGHT, false)],
        roster(),
    );

    assert_eq!(
        next.viewscreen(),
        &m(RIGHT),
        "the free screen, even though the occupied one is the primary"
    );
    assert_eq!(
        notes,
        vec![LayoutAdoption::ViewscreenMonitorGone {
            monitor: m(TV),
            replacement: m(RIGHT),
        }],
        "and nothing was covered, so there is nothing more to say"
    );
    assert_eq!(
        next.reserved_on(&m(LEFT)),
        &["Ada".to_string()],
        "Ada's console is still on the screen it was on"
    );
    assert!(
        next.apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
            .is_err(),
        "and still protected by rule 2's mirror"
    );
}

#[test]
fn a_fallback_with_nowhere_free_lands_on_an_occupied_screen_and_says_what_it_covered() {
    // The other side of the same rule: preferring a free screen is not the same
    // as refusing to land, because a bridge is at least one screen and the shared
    // view has to be somewhere. What it may not do is land in silence — this is
    // the case that used to produce NO note at all, because a reserved pane is
    // never unseated and so never earns a `SeatRefused`.
    let layout = with_an_authored_console();
    let (next, notes) = layout.reconcile(&[discovered(LEFT, true)], roster());

    assert_eq!(next.viewscreen(), &m(LEFT), "there was nowhere else");
    assert_eq!(
        notes,
        vec![
            LayoutAdoption::ViewscreenMonitorGone {
                monitor: m(TV),
                replacement: m(LEFT),
            },
            LayoutAdoption::ViewscreenCoversOccupants {
                monitor: m(LEFT),
                occupants: vec!["Ada".to_string()],
            },
        ]
    );
    let msg = notes[1].to_string();
    assert!(msg.contains("\"Ada\""), "{msg}");
    assert!(msg.contains(LEFT), "{msg}");
    assert_eq!(
        notes[1]
            .params()
            .iter()
            .find(|(k, _)| *k == "occupants")
            .map(|(_, v)| v.as_str()),
        Some("Ada"),
        "and the player-visible parameter names who was covered, unquoted"
    );
}

#[test]
fn a_viewscreen_falling_back_onto_an_occupied_screen_unseats_it_by_the_same_rule() {
    // The compound case, and the one that shows reconcile is not a second law:
    // the TV is gone, there is no free screen to prefer, the fallback lands on
    // the monitor holding both consoles — saying so — and rule 2 then refuses
    // them there exactly as a button press would be refused.
    let layout = assign(&assign(&bridge(), "helm", LEFT), "weapons", LEFT);
    let (next, notes) = layout.reconcile(&[discovered(LEFT, true)], roster());

    assert_eq!(next.viewscreen(), &m(LEFT));
    assert!(next.stations_on(&m(LEFT)).is_empty());
    assert_eq!(
        notes,
        vec![
            LayoutAdoption::ViewscreenMonitorGone {
                monitor: m(TV),
                replacement: m(LEFT),
            },
            LayoutAdoption::ViewscreenCoversOccupants {
                monitor: m(LEFT),
                occupants: vec!["helm".to_string(), "weapons".to_string()],
            },
            LayoutAdoption::SeatRefused {
                station: s("helm"),
                monitor: m(LEFT),
                refusal: LayoutRefusal::StationOnViewscreenMonitor {
                    station: s("helm"),
                    monitor: m(LEFT),
                },
            },
            LayoutAdoption::SeatRefused {
                station: s("weapons"),
                monitor: m(LEFT),
                refusal: LayoutRefusal::StationOnViewscreenMonitor {
                    station: s("weapons"),
                    monitor: m(LEFT),
                },
            },
        ]
    );
}

#[test]
fn reconciling_against_no_monitors_at_all_keeps_the_layout_and_says_so() {
    // A bridge is at least one screen — the lobby is drawn on it — so there is no
    // lawful layout to degrade to. An empty enumeration is a display driver
    // restarting far more often than it is a bridge that ceased to exist.
    let layout = assign(&bridge(), "helm", LEFT);
    let (next, notes) = layout.reconcile(&[], roster());

    assert_eq!(next, layout, "kept whole");
    assert_eq!(
        notes,
        vec![LayoutAdoption::NoMonitorsReported {
            kept: vec![m(TV), m(LEFT), m(RIGHT)],
        }]
    );
    assert!(notes[0].to_string().contains("3 monitors"));
}

// ── the player-visible rendering (issue #1330) ──────────────────────────────

#[test]
fn every_refusal_names_a_string_id_and_the_parameters_that_id_interpolates() {
    // The lobby's monitor row is a screen a player reads, so what crosses to it
    // is an id and values rather than the Rust-composed sentence `Display`
    // writes for the operator log (AGENTS.md rule 11). The pairing is checked
    // here so a new variant cannot reach a surface with no id at all.
    let cases: Vec<(LayoutRefusal, &str, Vec<&str>)> = vec![
        (
            LayoutRefusal::UnknownMonitor { monitor: m(TV) },
            "server.bridge_layout.unknown_monitor",
            vec!["monitor"],
        ),
        (
            LayoutRefusal::UnknownStation { station: s("helm") },
            "server.bridge_layout.unknown_station",
            vec!["station"],
        ),
        (
            LayoutRefusal::StationOnViewscreenMonitor {
                station: s("helm"),
                monitor: m(TV),
            },
            "server.bridge_layout.station_on_viewscreen",
            vec!["station", "monitor"],
        ),
        (
            LayoutRefusal::MonitorFull {
                monitor: m(LEFT),
                occupants: vec![s("helm"), s("weapons")],
                panes: Vec::new(),
            },
            "server.bridge_layout.monitor_full",
            vec!["monitor", "stations", "max"],
        ),
        (
            LayoutRefusal::ViewscreenMonitorHoldsStations {
                monitor: m(LEFT),
                stations: vec![s("helm")],
                panes: Vec::new(),
            },
            "server.bridge_layout.viewscreen_holds_stations",
            vec!["monitor", "stations"],
        ),
    ];
    for (refusal, id, keys) in cases {
        assert_eq!(refusal.string_id(), id, "{refusal:?}");
        let params = refusal.params();
        assert_eq!(
            params.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            keys,
            "{refusal:?}"
        );
        assert!(
            params.iter().all(|(_, v)| !v.is_empty()),
            "no parameter is blank: {params:?}"
        );
    }
}

#[test]
fn a_refusals_station_list_reaches_a_player_unquoted() {
    // `station_list` quotes, because an operator log line is naming machine
    // keys. The same list inside a sentence a player reads is just punctuation
    // noise, so the two renderings do not share a formatter.
    let refusal = LayoutRefusal::ViewscreenMonitorHoldsStations {
        monitor: m(LEFT),
        stations: vec![s("helm"), s("weapons")],
        panes: Vec::new(),
    };
    assert!(refusal.to_string().contains("\"helm\", \"weapons\""));
    let params = refusal.params();
    let stations = &params.iter().find(|(k, _)| *k == "stations").unwrap().1;
    assert_eq!(stations, "helm, weapons");
}

#[test]
fn every_adoption_note_names_a_string_id_and_the_parameters_that_id_interpolates() {
    let refusal = LayoutRefusal::UnknownMonitor { monitor: m(TV) };
    let cases: Vec<(LayoutAdoption, &str, Vec<&str>)> = vec![
        (
            LayoutAdoption::ViewscreenRefused {
                monitor: m(TV),
                refusal: refusal.clone(),
            },
            "server.bridge_layout.adopt_viewscreen_refused",
            vec!["monitor"],
        ),
        (
            LayoutAdoption::PaneNamesNoStation {
                monitor: m(LEFT),
                label: "Ada".to_string(),
            },
            "server.bridge_layout.adopt_pane_no_station",
            vec!["label", "monitor"],
        ),
        (
            LayoutAdoption::StationNamedTwice {
                station: s("helm"),
                monitor: m(LEFT),
            },
            "server.bridge_layout.adopt_station_twice",
            vec!["station", "monitor"],
        ),
        (
            LayoutAdoption::SeatRefused {
                station: s("helm"),
                monitor: m(LEFT),
                refusal: refusal.clone(),
            },
            "server.bridge_layout.adopt_seat_refused",
            vec!["station", "monitor"],
        ),
        (
            LayoutAdoption::ViewscreenMonitorGone {
                monitor: m(TV),
                replacement: m(LEFT),
            },
            "server.bridge_layout.adopt_viewscreen_gone",
            vec!["monitor", "replacement"],
        ),
        (
            LayoutAdoption::ViewscreenCoversOccupants {
                monitor: m(LEFT),
                occupants: vec!["helm".to_string(), "Ada".to_string()],
            },
            "server.bridge_layout.adopt_viewscreen_covers",
            vec!["monitor", "occupants"],
        ),
        (
            LayoutAdoption::StationMonitorGone {
                station: s("helm"),
                monitor: m(LEFT),
            },
            "server.bridge_layout.adopt_station_monitor_gone",
            vec!["station", "monitor"],
        ),
        (
            LayoutAdoption::StationOffRoster {
                station: s("helm"),
                monitor: m(LEFT),
            },
            "server.bridge_layout.adopt_station_off_roster",
            vec!["station", "monitor"],
        ),
        (
            LayoutAdoption::ConsoleCouldNotOpen {
                station: s("helm"),
                monitor: m(LEFT),
            },
            "server.bridge_layout.adopt_console_could_not_open",
            vec!["station", "monitor"],
        ),
        (
            // The one note whose subject is NOT a station id (issue #1332): an
            // authored `--pane` participant is re-tiled by the same rule, so the
            // parameter is `console` and carries either kind of name.
            LayoutAdoption::ConsoleRetiling {
                console: "Ada".to_string(),
                monitor: m(LEFT),
            },
            "server.bridge_layout.adopt_console_retiling",
            vec!["console", "monitor"],
        ),
        (
            LayoutAdoption::NoMonitorsReported {
                kept: vec![m(TV), m(LEFT)],
            },
            "server.bridge_layout.adopt_no_monitors",
            vec!["count"],
        ),
    ];
    for (note, id, keys) in cases {
        assert_eq!(note.string_id(), id, "{note:?}");
        assert_eq!(
            note.params().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            keys,
            "{note:?}"
        );
    }
}

#[test]
fn a_note_carrying_a_refusal_reports_it_beside_itself_rather_than_inside_itself() {
    // `t()` interpolates VALUES. A second localised sentence is not a value:
    // a translator handed `{reason}` cannot see what grammar lands in it, so
    // the cause is rendered as its own line.
    let refusal = LayoutRefusal::ViewscreenMonitorHoldsStations {
        monitor: m(LEFT),
        stations: vec![s("helm")],
        panes: Vec::new(),
    };
    let note = LayoutAdoption::ViewscreenRefused {
        monitor: m(LEFT),
        refusal: refusal.clone(),
    };
    assert_eq!(note.cause(), Some(&refusal));
    assert!(
        !note.params().iter().any(|(k, _)| *k == "reason"),
        "the cause is not smuggled in as a parameter"
    );
    assert_eq!(
        LayoutAdoption::StationOffRoster {
            station: s("helm"),
            monitor: m(LEFT),
        }
        .cause(),
        None
    );
}
