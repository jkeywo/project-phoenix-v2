//! Pure layout/profile tests for [`super`] (issue #1123).
//!
//! No display, no window, no GPU — these run in the ordinary `cargo test` CI
//! job. They cover the acceptance criteria that have logic in them: stable
//! identity across a simulated OS-settings rearrange, the one/two-pane
//! geometry math, the more-than-two refusal, the serialisation round-trip, and
//! the naming of a monitor that is gone or unassigned.

use super::*;

fn raw(name: &str, w: u32, h: u32, x: i32, y: i32) -> RawMonitor {
    RawMonitor {
        name: Some(name.to_string()),
        physical_width: w,
        physical_height: h,
        position_x: x,
        position_y: y,
        scale_factor: 1.0,
        primary: false,
    }
}

// ── stable identity ─────────────────────────────────────────────────────────

#[test]
fn distinct_monitors_keep_a_short_position_free_identity() {
    let monitors = [
        raw("DELL U2720Q", 3840, 2160, 0, 0),
        raw("BenQ EX", 1920, 1080, 3840, 0),
    ];
    let discovered = identify(&monitors);
    assert_eq!(discovered[0].identity.as_str(), "DELL U2720Q@3840x2160");
    assert_eq!(discovered[1].identity.as_str(), "BenQ EX@1920x1080");
}

#[test]
fn identity_survives_reordering_and_repositioning_the_displays() {
    // The whole point of a stable identity: rearranging displays in the OS —
    // which changes both enumeration order and position — must not change which
    // monitor a profile refers to.
    let before = [
        raw("DELL U2720Q", 3840, 2160, 0, 0),
        raw("BenQ EX", 1920, 1080, 3840, 0),
    ];
    // Same two monitors, enumerated in the other order and repositioned (the
    // BenQ is now on the left, the Dell to its right).
    let after = [
        raw("BenQ EX", 1920, 1080, 0, 0),
        raw("DELL U2720Q", 3840, 2160, 1920, 0),
    ];

    let a: std::collections::HashSet<String> = identify(&before)
        .into_iter()
        .map(|d| d.identity.as_str().to_string())
        .collect();
    let b: std::collections::HashSet<String> = identify(&after)
        .into_iter()
        .map(|d| d.identity.as_str().to_string())
        .collect();
    assert_eq!(
        a, b,
        "the same monitors keep the same identities across a rearrange"
    );
}

#[test]
fn identical_monitors_are_disambiguated_by_position() {
    // Two of the same model in the same mode are indistinguishable to winit, so
    // they fall back to a position suffix — the documented limit.
    let monitors = [
        raw("ACME 1080", 1920, 1080, 0, 0),
        raw("ACME 1080", 1920, 1080, 1920, 0),
    ];
    let discovered = identify(&monitors);
    assert_ne!(discovered[0].identity, discovered[1].identity);
    assert_eq!(discovered[0].identity.as_str(), "ACME 1080@1920x1080#0,0");
    assert_eq!(
        discovered[1].identity.as_str(),
        "ACME 1080@1920x1080#1920,0"
    );
}

#[test]
fn a_nameless_monitor_gets_a_stable_placeholder_name() {
    let mut m = raw("ignored", 1280, 720, 0, 0);
    m.name = None;
    let discovered = identify(std::slice::from_ref(&m));
    assert_eq!(discovered[0].identity.as_str(), "unnamed-display@1280x720");
}

// ── density rule ────────────────────────────────────────────────────────────

fn station_entry(id: &str, panes: &[&str]) -> DisplayEntry {
    DisplayEntry {
        id: id.to_string(),
        role: ROLE_STATION.to_string(),
        split: None,
        panes: panes
            .iter()
            .copied()
            .map(PaneSlot::for_participant)
            .collect(),
    }
}

/// A `viewscreen` entry for `id`. A profile that assigns monitors must name one
/// (issue #1327's [`ProfileError::MissingViewscreen`]), so a fixture that is
/// really about something else — density, labels, identity — still carries one.
fn viewscreen_entry(id: &str) -> DisplayEntry {
    DisplayEntry {
        id: id.to_string(),
        role: ROLE_VIEWSCREEN.to_string(),
        split: None,
        panes: Vec::new(),
    }
}

fn profile_with(entries: Vec<DisplayEntry>) -> BridgeProfile {
    BridgeProfile {
        version: PROFILE_VERSION,
        displays: entries,
        touch: Vec::new(),
        media: Vec::new(),
    }
}

#[test]
fn a_station_of_one_or_two_panes_validates() {
    let one = profile_with(vec![viewscreen_entry("vs"), station_entry("m1", &["Ada"])]);
    let two = profile_with(vec![
        viewscreen_entry("vs"),
        station_entry("m2", &["Ada", "Grace"]),
    ]);
    assert!(one.validate().is_ok());
    assert!(two.validate().is_ok());
}

#[test]
fn a_station_of_three_panes_is_refused_with_a_clear_explanation() {
    let three = profile_with(vec![station_entry("m3", &["Ada", "Grace", "Kay"])]);
    let err = three.validate().unwrap_err();
    assert_eq!(
        err,
        ProfileError::Density {
            id: "m3".to_string(),
            count: 3
        }
    );
    // The message names the monitor, the count, and the rule.
    let msg = err.to_string();
    assert!(msg.contains("m3"), "{msg}");
    assert!(msg.contains('3'), "{msg}");
    assert!(msg.contains("Station"), "{msg}");
}

#[test]
fn a_station_with_no_panes_is_refused() {
    let none = profile_with(vec![station_entry("m0", &[])]);
    assert_eq!(
        none.validate().unwrap_err(),
        ProfileError::Density {
            id: "m0".to_string(),
            count: 0
        }
    );
}

#[test]
fn a_viewscreen_may_not_carry_panes() {
    let entry = DisplayEntry {
        id: "vs".to_string(),
        role: ROLE_VIEWSCREEN.to_string(),
        split: None,
        panes: vec![PaneSlot::for_participant("Ada")],
    };
    assert_eq!(
        profile_with(vec![entry]).validate().unwrap_err(),
        ProfileError::ViewscreenHasPanes {
            id: "vs".to_string()
        }
    );
}

#[test]
fn an_unknown_role_word_is_refused() {
    let entry = DisplayEntry {
        id: "m".to_string(),
        role: "cupholder".to_string(),
        split: None,
        panes: Vec::new(),
    };
    assert_eq!(
        profile_with(vec![entry]).validate().unwrap_err(),
        ProfileError::UnknownRole {
            id: "m".to_string(),
            role: "cupholder".to_string()
        }
    );
}

#[test]
fn two_displays_claiming_one_monitor_is_refused() {
    let dup = profile_with(vec![
        DisplayEntry {
            id: "m".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        station_entry("m", &["Ada"]),
    ]);
    assert_eq!(
        dup.validate().unwrap_err(),
        ProfileError::DuplicateId {
            id: "m".to_string()
        }
    );
}

#[test]
fn two_viewscreens_is_refused() {
    // AC2 and the module's "unsupported config is refused visibly" philosophy:
    // a bridge has exactly one shared viewscreen, so `validate` must catch this
    // rather than let `apply_bridge_profile` silently keep only the last one
    // and leave the other configured monitor black with no diagnostic.
    let two = profile_with(vec![
        DisplayEntry {
            id: "m1".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        DisplayEntry {
            id: "m2".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
    ]);
    let err = two.validate().unwrap_err();
    assert_eq!(
        err,
        ProfileError::MultipleViewscreens {
            ids: vec!["m1".to_string(), "m2".to_string()]
        }
    );
    // The message names both monitors.
    let msg = err.to_string();
    assert!(msg.contains("m1"), "{msg}");
    assert!(msg.contains("m2"), "{msg}");
}

#[test]
fn a_single_viewscreen_profile_still_validates() {
    let one = profile_with(vec![
        DisplayEntry {
            id: "m1".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        station_entry("m2", &["Ada"]),
    ]);
    assert!(one.validate().is_ok());
}

// ── the viewscreen must be named (issue #1327) ──────────────────────────────

#[test]
fn a_profile_of_stations_with_no_viewscreen_is_refused() {
    // The one station-over-viewscreen path a hand-authored profile still had:
    // `apply_bridge_profile` places the viewscreen on the PRIMARY window, so a
    // profile that names no viewscreen never places it — it stays on whatever
    // monitor the OS opened it on, which one of these Stations may then cover
    // with a borderless-fullscreen console.
    let stations_only = profile_with(vec![
        station_entry("m1", &["Ada"]),
        station_entry("m2", &["Grace"]),
    ]);
    let err = stations_only.validate().unwrap_err();
    assert_eq!(
        err,
        ProfileError::MissingViewscreen {
            stations: vec!["m1".to_string(), "m2".to_string()],
        }
    );
    // The message names the monitors, the missing role, and the harm.
    let msg = err.to_string();
    assert!(msg.contains("m1"), "{msg}");
    assert!(msg.contains("m2"), "{msg}");
    assert!(msg.contains("viewscreen"), "{msg}");
    assert!(msg.contains("primary window"), "{msg}");
}

#[test]
fn a_single_monitor_profile_naming_only_a_station_is_refused_too() {
    // The most harmful shape of all, and the one an operator with one display
    // reaches for: the single monitor is both where the primary window is and
    // where the console would open.
    let one = profile_with(vec![station_entry("only", &["Ada", "Grace"])]);
    let err = one.validate().unwrap_err();
    assert_eq!(
        err,
        ProfileError::MissingViewscreen {
            stations: vec!["only".to_string()],
        }
    );
    // The singular wording, pinned: the #1124 acceptance kit quotes this exact
    // phrase back to the operator (docs/acceptance/1124-input.md), and a quote
    // nothing asserts is a quote that silently goes stale.
    let msg = err.to_string();
    assert!(msg.contains("one monitor"), "{msg}");
    assert!(!msg.contains("1 monitors"), "{msg}");
}

#[test]
fn a_profile_that_assigns_no_monitor_at_all_still_validates() {
    // Gated on there being displays: a `[[touch]]`/`[[media]]`-only profile (the
    // shape the #1126 media kit ships) opens no Station window, so it has nothing
    // to cover the viewscreen with. Refusing it would break `--pane`'s own
    // profile-less tiling path for no gain.
    let media_only = BridgeProfile {
        version: PROFILE_VERSION,
        displays: Vec::new(),
        touch: vec![TouchMapping {
            device: "ELAN Touchscreen".to_string(),
            monitor: "BenQ EX@1920x1080".to_string(),
        }],
        media: Vec::new(),
    };
    assert!(media_only.validate().is_ok());
    assert!(BridgeProfile::empty().validate().is_ok());
}

// ── a pane slot's station id (issue #1327) ──────────────────────────────────

#[test]
fn a_pane_slot_carries_its_station_id_through_the_toml_round_trip() {
    // The layout keys stations by station id, and this is how that id rides in a
    // `[[display.pane]]` slot: an optional `station = "…"` beside the label.
    let p = profile_with(vec![
        viewscreen_entry("vs"),
        DisplayEntry {
            id: "m1".to_string(),
            role: ROLE_STATION.to_string(),
            split: Some(PaneSplit::SideBySide),
            panes: vec![
                PaneSlot::for_station("helm"),
                PaneSlot::for_station("weapons"),
            ],
        },
    ]);
    let text = p.to_toml().expect("serialises");
    assert!(text.contains("station = \"helm\""), "{text}");
    assert!(text.contains("station = \"weapons\""), "{text}");
    let reparsed = BridgeProfile::from_toml(&text).expect("parses");
    assert_eq!(reparsed, p);
    assert_eq!(
        reparsed.displays[1].panes[0].station.as_deref(),
        Some("helm")
    );
}

#[test]
fn a_participant_pane_names_no_station_and_omits_the_field_entirely() {
    // A `--pane <NAME>` pane belongs to no station: the crew member at it claims
    // one from inside their own console. Every profile authored before #1327 is
    // this shape, so the field must be absent from the file, not `station = ""`.
    let slot = PaneSlot::for_participant("Ada");
    assert_eq!(slot.label, "Ada");
    assert_eq!(slot.station, None);
    let text = profile_with(vec![viewscreen_entry("vs"), station_entry("m1", &["Ada"])])
        .to_toml()
        .unwrap();
    assert!(text.contains("label = \"Ada\""), "{text}");
    assert!(!text.contains("station ="), "{text}");
}

#[test]
fn a_profile_written_before_the_station_field_existed_still_parses() {
    // Backwards compatibility, stated as a test rather than as a promise: the
    // exact `[[display.pane]]` shape the #1124 kit ships.
    let text = r#"
version = 1

[[display]]
id = "DELL@3840x2160"
role = "viewscreen"

[[display]]
id = "BenQ@1920x1080"
role = "station"
split = "side_by_side"
[[display.pane]]
label = "Ada"
[[display.pane]]
label = "Grace"
"#;
    let parsed = BridgeProfile::from_toml(text).expect("parses");
    let validated = parsed.validate().expect("validates");
    let DisplayRole::Station { panes, .. } = &validated.displays[1].role else {
        panic!("the second display is a Station");
    };
    assert_eq!(panes[0], PaneSlot::for_participant("Ada"));
    assert_eq!(panes[1], PaneSlot::for_participant("Grace"));
}

#[test]
fn two_panes_sharing_a_participant_label_across_the_profile_is_refused() {
    // A lost monitor's panes are resolved to disconnect by participant NAME, with
    // no monitor anchor, so the same label on two Station monitors could not be
    // resolved to the right one. `validate` refuses it at author time (composing
    // with #1123's other refusals) rather than mis-resolving it at runtime.
    let dup = profile_with(vec![
        station_entry("m1", &["Ada"]),
        station_entry("m2", &["Ada"]),
    ]);
    let err = dup.validate().unwrap_err();
    assert_eq!(
        err,
        ProfileError::DuplicatePaneLabel {
            label: "Ada".to_string()
        }
    );
    // The message names the offending label and the rule.
    let msg = err.to_string();
    assert!(msg.contains("Ada"), "{msg}");
    assert!(msg.contains("unique"), "{msg}");
}

#[test]
fn a_label_repeated_within_one_two_pane_station_is_also_refused() {
    // The same hole, inside a single Station: two panes named for the same
    // participant are two panes one name would resolve to.
    let dup = profile_with(vec![station_entry("m1", &["Ada", "Ada"])]);
    assert_eq!(
        dup.validate().unwrap_err(),
        ProfileError::DuplicatePaneLabel {
            label: "Ada".to_string()
        }
    );
}

#[test]
fn two_panes_naming_the_same_station_are_refused() {
    // A station has exactly one console. A file that seats it on two screens
    // leaves nothing to say which one it opens on, so it is refused at the prompt
    // — the same shape the layout law reports as `LayoutAdoption::StationNamedTwice`
    // when it is handed one, so the file law and the transition law agree.
    // The labels differ, so this is not `DuplicatePaneLabel` wearing a hat.
    let dup = profile_with(vec![
        viewscreen_entry("vs"),
        DisplayEntry {
            id: "m1".to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot {
                label: "Ada".to_string(),
                station: Some("helm".to_string()),
            }],
        },
        DisplayEntry {
            id: "m2".to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot {
                label: "Grace".to_string(),
                station: Some("helm".to_string()),
            }],
        },
    ]);
    let err = dup.validate().unwrap_err();
    assert_eq!(
        err,
        ProfileError::DuplicatePaneStation {
            station: "helm".to_string(),
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("helm"), "{msg}");
    assert!(msg.contains("one console"), "{msg}");

    // The same hole inside a single two-pane Station.
    let within = profile_with(vec![
        viewscreen_entry("vs"),
        DisplayEntry {
            id: "m1".to_string(),
            role: ROLE_STATION.to_string(),
            split: Some(PaneSplit::SideBySide),
            panes: vec![
                PaneSlot {
                    label: "Ada".to_string(),
                    station: Some("helm".to_string()),
                },
                PaneSlot {
                    label: "Grace".to_string(),
                    station: Some("helm".to_string()),
                },
            ],
        },
    ]);
    assert_eq!(
        within.validate().unwrap_err(),
        ProfileError::DuplicatePaneStation {
            station: "helm".to_string(),
        }
    );
}

#[test]
fn a_profile_the_layout_wrote_is_untouched_by_the_duplicate_station_rule() {
    // A layout seats each station once and names each pane for its station, so
    // `label` and `station` are the same unique id. The refusal cannot fire on
    // anything the layout writes — which is what makes
    // `BridgeLayout::to_validated_profile` infallible.
    let ok = profile_with(vec![
        viewscreen_entry("vs"),
        DisplayEntry {
            id: "m1".to_string(),
            role: ROLE_STATION.to_string(),
            split: Some(PaneSplit::SideBySide),
            panes: vec![
                PaneSlot::for_station("helm"),
                PaneSlot::for_station("weapons"),
            ],
        },
        DisplayEntry {
            id: "m2".to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![PaneSlot::for_station("comms")],
        },
    ]);
    assert!(ok.validate().is_ok());

    // And a profile of hand-authored participant panes names no station at all,
    // so several of them never collide either.
    let participants = profile_with(vec![
        viewscreen_entry("vs"),
        station_entry("m1", &["Ada", "Grace"]),
        station_entry("m2", &["Kay"]),
    ]);
    assert!(participants.validate().is_ok());
}

#[test]
fn distinct_labels_across_stations_still_validate() {
    // The refusal is only for a genuine collision — a bridge of many stations
    // with distinct crew names is the ordinary case and must pass.
    let ok = profile_with(vec![
        viewscreen_entry("vs"),
        station_entry("m1", &["Ada", "Grace"]),
        station_entry("m2", &["Kay"]),
    ]);
    assert!(ok.validate().is_ok());
}

#[test]
fn a_profile_from_another_schema_version_is_refused_rather_than_reinterpreted() {
    let mut p = profile_with(vec![]);
    p.version = PROFILE_VERSION + 1;
    assert_eq!(
        p.validate().unwrap_err(),
        ProfileError::Version {
            found: PROFILE_VERSION + 1
        }
    );
}

// ── pane geometry math ──────────────────────────────────────────────────────

fn geom(w: u32, h: u32) -> MonitorGeometry {
    MonitorGeometry {
        physical_width: w,
        physical_height: h,
        position_x: 0,
        position_y: 0,
        scale_factor: 1.0,
    }
}

#[test]
fn one_pane_is_the_whole_monitor() {
    let rects = pane_rects(&geom(1920, 1080), PaneSplit::SideBySide, 1);
    assert_eq!(
        rects,
        vec![PaneRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080
        }]
    );
}

#[test]
fn two_side_by_side_panes_tile_the_width_exactly() {
    let rects = pane_rects(&geom(1920, 1080), PaneSplit::SideBySide, 2);
    assert_eq!(
        rects,
        vec![
            PaneRect {
                x: 0,
                y: 0,
                width: 960,
                height: 1080
            },
            PaneRect {
                x: 960,
                y: 0,
                width: 960,
                height: 1080
            },
        ]
    );
}

#[test]
fn an_odd_width_is_absorbed_by_the_second_pane_with_no_gap() {
    let rects = pane_rects(&geom(1921, 1080), PaneSplit::SideBySide, 2);
    assert_eq!(rects[0].width, 960);
    assert_eq!(rects[1].x, 960);
    assert_eq!(rects[1].width, 961);
    // No gap, no overlap: the two together cover the whole width.
    assert_eq!(rects[0].width + rects[1].width, 1921);
}

#[test]
fn two_stacked_panes_tile_the_height_exactly() {
    let rects = pane_rects(&geom(1920, 1081), PaneSplit::Stacked, 2);
    assert_eq!(
        rects[0],
        PaneRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 540
        }
    );
    assert_eq!(
        rects[1],
        PaneRect {
            x: 0,
            y: 540,
            width: 1920,
            height: 541
        }
    );
}

// ── TOML round-trip ─────────────────────────────────────────────────────────

fn round_trip_fixture() -> BridgeProfile {
    BridgeProfile {
        version: PROFILE_VERSION,
        displays: vec![
            DisplayEntry {
                id: "DELL U2720Q@3840x2160".to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            DisplayEntry {
                id: "BenQ EX@1920x1080".to_string(),
                role: ROLE_STATION.to_string(),
                split: Some(PaneSplit::SideBySide),
                panes: vec![
                    PaneSlot::for_participant("Ada"),
                    PaneSlot::for_participant("Grace"),
                ],
            },
            station_entry("Acer@1280x1024", &["Kay"]),
        ],
        touch: vec![TouchMapping {
            device: "ELAN Touchscreen".to_string(),
            monitor: "BenQ EX@1920x1080".to_string(),
        }],
        media: vec![
            super::super::bridge_media::MediaSurfaceEntry {
                surface: "viewscreen".to_string(),
                camera: Some("camera:Logitech BRIO".to_string()),
                microphones: vec!["mic:Blue Yeti".to_string()],
                outputs: vec!["output:Bridge Speakers".to_string()],
                allow_shared: Vec::new(),
            },
            super::super::bridge_media::MediaSurfaceEntry {
                surface: "comms".to_string(),
                camera: None,
                microphones: vec!["mic:Comms Headset".to_string()],
                outputs: vec!["output:Comms Headset".to_string()],
                allow_shared: Vec::new(),
            },
        ],
    }
}

#[test]
fn a_profile_round_trips_through_toml_unchanged() {
    let original = round_trip_fixture();
    let text = original.to_toml().expect("serialises");
    let reparsed = BridgeProfile::from_toml(&text).expect("parses");
    assert_eq!(original, reparsed);
    // And the reload validates to the same roles.
    assert_eq!(
        original.validate().unwrap(),
        reparsed.validate().unwrap(),
        "roles, pane geometry, assignments and touch mapping all survive the round-trip"
    );
}

#[test]
fn the_serialised_form_is_human_readable_toml() {
    let text = round_trip_fixture().to_toml().unwrap();
    // Operator-facing sanity: the shape is what the docs describe.
    assert!(text.contains("version = 1"), "{text}");
    assert!(text.contains("[[display]]"), "{text}");
    assert!(text.contains("role = \"viewscreen\""), "{text}");
    assert!(text.contains("role = \"station\""), "{text}");
    assert!(text.contains("[[touch]]"), "{text}");
    assert!(text.contains("ELAN Touchscreen"), "{text}");
    // The media half (issue #1126) rides in the same file.
    assert!(text.contains("[[media]]"), "{text}");
    assert!(text.contains("surface = \"viewscreen\""), "{text}");
    assert!(text.contains("camera = \"camera:Logitech BRIO\""), "{text}");
    // The array's exact spacing is the pretty-printer's; assert on the field name
    // and the value rather than the bracket layout.
    assert!(text.contains("microphone = ["), "{text}");
    assert!(text.contains("mic:Blue Yeti"), "{text}");
}

#[test]
fn an_empty_profile_round_trips() {
    let text = BridgeProfile::empty().to_toml().unwrap();
    assert_eq!(
        BridgeProfile::from_toml(&text).unwrap(),
        BridgeProfile::empty()
    );
}

#[test]
fn a_windows_device_name_identity_round_trips_through_toml() {
    // winit reports a GDI device name like `\\.\DISPLAY5` on Windows (confirmed
    // by the integration test on the dev machine), so a real identity contains
    // backslashes. The TOML round-trip must survive the escaping.
    let id = r"\\.\DISPLAY5@1920x1080";
    let p = profile_with(vec![DisplayEntry {
        id: id.to_string(),
        role: ROLE_VIEWSCREEN.to_string(),
        split: None,
        panes: Vec::new(),
    }]);
    let reparsed = BridgeProfile::from_toml(&p.to_toml().unwrap()).unwrap();
    assert_eq!(reparsed.displays[0].id, id);
    assert_eq!(reparsed, p);
}

#[test]
fn a_position_suffixed_identical_monitor_id_round_trips_through_toml() {
    // `identify`'s fallback for two identical monitors appends a `#x,y`
    // suffix (see `identical_monitors_are_disambiguated_by_position` above).
    // The `#` sits inside a quoted TOML string, so it is ordinary text to the
    // parser rather than a comment starter — this pins that a profile
    // referencing such an id survives the round-trip unchanged.
    let id = "ACME 1080@1920x1080#1920,0";
    let p = profile_with(vec![DisplayEntry {
        id: id.to_string(),
        role: ROLE_VIEWSCREEN.to_string(),
        split: None,
        panes: Vec::new(),
    }]);
    let text = p.to_toml().expect("serialises");
    let reparsed = BridgeProfile::from_toml(&text).expect("parses");
    assert_eq!(reparsed.displays[0].id, id);
    assert_eq!(reparsed, p);
}

// ── resolution against real monitors ────────────────────────────────────────

fn discovered_fixture() -> Vec<DiscoveredMonitor> {
    identify(&[
        {
            let mut m = raw("DELL U2720Q", 3840, 2160, 0, 0);
            m.primary = true;
            m
        },
        raw("BenQ EX", 1920, 1080, 3840, 0),
    ])
}

fn matching_profile() -> ValidatedProfile {
    profile_with(vec![
        DisplayEntry {
            id: "DELL U2720Q@3840x2160".to_string(),
            role: ROLE_VIEWSCREEN.to_string(),
            split: None,
            panes: Vec::new(),
        },
        station_entry("BenQ EX@1920x1080", &["Ada"]),
    ])
    .validate()
    .unwrap()
}

#[test]
fn a_matching_profile_resolves_to_surfaces_with_live_geometry() {
    let resolved = resolve(&matching_profile(), &discovered_fixture());
    assert!(!resolved.has_problems());
    assert_eq!(resolved.surfaces.len(), 2);

    let vs = resolved.viewscreen().expect("a viewscreen");
    assert_eq!(vs.geometry.physical_width, 3840);
    assert!(vs.primary);

    let station = resolved.stations().next().expect("a station");
    assert!(matches!(station.role, DisplayRole::Station { .. }));
    assert_eq!(station.geometry.position_x, 3840);
}

#[test]
fn a_repositioned_but_same_monitor_is_honoured_not_flagged() {
    // Move the BenQ from the right of the Dell to the left. Its identity is
    // unchanged (position is not part of identity), so it still resolves — and
    // the surface carries the NEW position.
    let moved = identify(&[raw("BenQ EX", 1920, 1080, -1920, 0), {
        let mut m = raw("DELL U2720Q", 3840, 2160, 0, 0);
        m.primary = true;
        m
    }]);
    let resolved = resolve(&matching_profile(), &moved);
    assert!(!resolved.has_problems(), "a rearrange is not a problem");
    let station = resolved.stations().next().unwrap();
    assert_eq!(
        station.geometry.position_x, -1920,
        "the live position is used"
    );
}

#[test]
fn a_missing_monitor_is_named_and_its_role_is_not_re_homed() {
    // Only the Dell is connected; the profile still assigns the BenQ.
    let only_dell = identify(&[{
        let mut m = raw("DELL U2720Q", 3840, 2160, 0, 0);
        m.primary = true;
        m
    }]);
    let resolved = resolve(&matching_profile(), &only_dell);

    // The viewscreen still resolves; the Station is reported missing, NOT moved
    // onto the Dell.
    assert_eq!(resolved.surfaces.len(), 1);
    assert_eq!(resolved.viewscreen().unwrap().geometry.physical_width, 3840);
    assert_eq!(
        resolved.problems,
        vec![ProfileProblem::MonitorMissing {
            id: MonitorIdentity::new("BenQ EX@1920x1080"),
            role: DisplayRole::Station {
                split: PaneSplit::SideBySide,
                panes: vec![PaneSlot::for_participant("Ada")],
            }
            .summary(),
        }]
    );
    assert!(resolved.problems[0].to_string().contains("BenQ EX"));
}

#[test]
fn a_present_but_unassigned_monitor_is_reported() {
    // A third monitor appeared that the profile says nothing about.
    let mut discovered = discovered_fixture();
    discovered.extend(identify(&[raw("Acer VG", 1280, 1024, 5760, 0)]));
    let resolved = resolve(&matching_profile(), &discovered);
    assert_eq!(
        resolved.problems,
        vec![ProfileProblem::MonitorUnassigned {
            id: MonitorIdentity::new("Acer VG@1280x1024"),
            name: Some("Acer VG".to_string()),
        }]
    );
}

#[test]
fn a_changed_resolution_surfaces_as_missing_plus_unassigned_nothing_moved() {
    // The viewscreen monitor was switched to a different resolution. Its identity
    // changes with it, so the profile's old id is missing and the new id is
    // unassigned — both named, the viewscreen role NOT silently moved.
    let changed = identify(&[
        {
            let mut m = raw("DELL U2720Q", 2560, 1440, 0, 0); // was 3840x2160
            m.primary = true;
            m
        },
        raw("BenQ EX", 1920, 1080, 2560, 0),
    ]);
    let resolved = resolve(&matching_profile(), &changed);
    assert!(
        resolved.viewscreen().is_none(),
        "the old viewscreen id is gone"
    );
    assert!(resolved.problems.contains(&ProfileProblem::MonitorMissing {
        id: MonitorIdentity::new("DELL U2720Q@3840x2160"),
        role: "viewscreen".to_string(),
    }));
    assert!(resolved
        .problems
        .contains(&ProfileProblem::MonitorUnassigned {
            id: MonitorIdentity::new("DELL U2720Q@2560x1440"),
            name: Some("DELL U2720Q".to_string()),
        }));
}

// ── setup report ────────────────────────────────────────────────────────────

#[test]
fn the_setup_report_lists_monitors_and_their_assignment() {
    let discovered = discovered_fixture();
    let profile = {
        let mut p = BridgeProfile::empty();
        p.displays = vec![
            DisplayEntry {
                id: "DELL U2720Q@3840x2160".to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            station_entry("BenQ EX@1920x1080", &["Ada"]),
        ];
        p
    };
    let report = render_setup_report(&discovered, Some(&profile));
    assert!(report.contains("Discovered 2 monitor(s)"), "{report}");
    assert!(report.contains("DELL U2720Q@3840x2160"), "{report}");
    assert!(report.contains("(primary)"), "{report}");
    assert!(report.contains("viewscreen"), "{report}");
    assert!(report.contains("station"), "{report}");
    assert!(
        report.contains("Profile matches the connected displays."),
        "{report}"
    );
}

#[test]
fn the_setup_report_without_a_profile_says_everything_is_unassigned() {
    let report = render_setup_report(&discovered_fixture(), None);
    assert!(report.contains("every monitor is unassigned"), "{report}");
    assert!(report.contains("unassigned"), "{report}");
}

#[test]
fn the_setup_report_surfaces_a_missing_monitor() {
    let only_dell = identify(&[{
        let mut m = raw("DELL U2720Q", 3840, 2160, 0, 0);
        m.primary = true;
        m
    }]);
    let profile = {
        let mut p = BridgeProfile::empty();
        p.displays = vec![
            DisplayEntry {
                id: "DELL U2720Q@3840x2160".to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            station_entry("BenQ EX@1920x1080", &["Ada"]),
        ];
        p
    };
    let report = render_setup_report(&only_dell, Some(&profile));
    assert!(report.contains("Profile problems:"), "{report}");
    assert!(report.contains("not connected"), "{report}");
}

#[test]
fn the_setup_report_surfaces_an_invalid_profile() {
    let profile = profile_with(vec![station_entry("m", &["A", "B", "C"])]);
    let report = render_setup_report(&[], Some(&profile));
    assert!(report.contains("Profile is invalid"), "{report}");
}

// ── runtime display loss (issue #1125) ──────────────────────────────────────

/// A present-set of monitor identities, as the runtime watcher tracks.
fn present(ids: &[&str]) -> std::collections::HashSet<MonitorIdentity> {
    ids.iter().map(|id| MonitorIdentity::new(*id)).collect()
}

const DELL: &str = "DELL U2720Q@3840x2160";
const BENQ: &str = "BenQ EX@1920x1080";

#[test]
fn assigned_surfaces_carry_each_stations_pane_labels_and_the_viewscreen_none() {
    let assigned = matching_profile().assigned_surfaces();
    let dell = assigned
        .iter()
        .find(|a| a.identity.as_str() == DELL)
        .unwrap();
    let benq = assigned
        .iter()
        .find(|a| a.identity.as_str() == BENQ)
        .unwrap();
    assert!(
        dell.pane_labels.is_empty(),
        "the viewscreen carries no panes"
    );
    assert_eq!(benq.pane_labels, vec!["Ada".to_string()]);
}

#[test]
fn a_station_bearing_slot_contributes_no_pane_label_to_the_watcher() {
    // Issue #1331. `PaneSlot::for_station` sets `label == station_id`, so
    // copying every label would put a STATION ID into the list the #1125
    // watcher resolves against the live pane bus — and that list is the BOOT
    // profile's, which never moves while the console does. An operator who
    // seats `helm` in a `--profile` and then moves it from the lobby would have
    // an unplug of the OLD screen close a console that is not on it.
    //
    // A station's console is the LAW's: `reconcile` unseats it and
    // `follow_layout_stations` closes it. So the watcher's list is participants
    // only, which is exactly what its own documentation has always claimed.
    let profile = profile_with(vec![
        viewscreen_entry(DELL),
        DisplayEntry {
            id: BENQ.to_string(),
            role: ROLE_STATION.to_string(),
            split: None,
            panes: vec![
                PaneSlot::for_station("helm"),
                PaneSlot::for_participant("Ada"),
            ],
        },
    ])
    .validate()
    .expect("a station and a participant on one screen is a lawful two-pane profile");

    let assigned = profile.assigned_surfaces();
    let benq = assigned
        .iter()
        .find(|a| a.identity.as_str() == BENQ)
        .expect("the Station monitor is assigned");
    assert_eq!(
        benq.pane_labels,
        vec!["Ada".to_string()],
        "the participant rides on the watcher's list; the station's console does not"
    );

    // And therefore an unplug of that screen names Ada's pane and nothing else.
    let losses = runtime_display_losses(&assigned, &present(&[DELL, BENQ]), &present(&[DELL]));
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].pane_labels, vec!["Ada".to_string()]);
    // The role SUMMARY still names both slots — it describes the profile — but
    // the list of panes that disconnect names only the one that does.
    assert!(
        losses[0].to_string().contains("the panes it carried (Ada)"),
        "the report cannot list a console it does not close: {}",
        losses[0]
    );
}

#[test]
fn losing_a_station_monitor_names_it_and_the_panes_that_must_disconnect() {
    // The runtime extension of the #1123 missing-display report: a monitor that
    // WAS present and driving a pane is unplugged mid-mission. It is named
    // exactly, and its pane is named so its token can disconnect → Backfill.
    let assigned = matching_profile().assigned_surfaces();
    let losses = runtime_display_losses(&assigned, &present(&[DELL, BENQ]), &present(&[DELL]));
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].identity.as_str(), BENQ);
    assert_eq!(losses[0].pane_labels, vec!["Ada".to_string()]);
    // The report names the monitor and the fall-back, and promises no re-home.
    let text = losses[0].to_string();
    assert!(text.contains(BENQ), "{text}");
    assert!(text.contains("Ada"), "{text}");
    assert!(text.contains("AI control"), "{text}");
    assert!(
        text.contains("not moved to another display") || text.contains("rather than moved"),
        "{text}"
    );
}

#[test]
fn losing_the_viewscreen_monitor_names_it_but_fails_no_pane() {
    // The viewscreen carries no participant, so losing it leaves the shared 3-D
    // view nowhere to draw and touches no station — the mission carries on.
    let assigned = matching_profile().assigned_surfaces();
    let losses = runtime_display_losses(&assigned, &present(&[DELL, BENQ]), &present(&[BENQ]));
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].identity.as_str(), DELL);
    assert!(losses[0].pane_labels.is_empty());
    assert!(losses[0].to_string().contains("nowhere to draw"));
}

#[test]
fn a_monitor_that_was_never_present_is_not_a_runtime_loss() {
    // A profile naming a monitor that never connected is the SETUP-time report's
    // job (`resolve` → `MonitorMissing`), not a runtime loss: nothing was lost.
    let assigned = matching_profile().assigned_surfaces();
    let losses = runtime_display_losses(&assigned, &present(&[DELL]), &present(&[DELL]));
    assert!(losses.is_empty(), "no change means no loss");
    let never = runtime_display_losses(&assigned, &present(&[]), &present(&[DELL]));
    assert!(never.is_empty(), "a monitor appearing is not a loss");
}

#[test]
fn a_returned_monitor_is_reported_for_explicit_repair_and_not_rehomed() {
    // AC5: a display coming back is reported, never silently used. Bringing it
    // back into service is an explicit re-apply of the profile.
    let assigned = matching_profile().assigned_surfaces();
    let returns = runtime_display_returns(&assigned, &present(&[DELL]), &present(&[DELL, BENQ]));
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].identity.as_str(), BENQ);
    let text = returns[0].to_string();
    assert!(text.contains("re-apply"), "{text}");
    assert!(text.contains("explicit repair"), "{text}");
}

#[test]
fn an_unassigned_monitor_appearing_is_neither_a_loss_nor_a_return() {
    // A monitor the profile assigns no role is not part of the bridge; its
    // coming or going is not a runtime loss/return of a configured surface.
    let assigned = matching_profile().assigned_surfaces();
    let other = "Some Other@1280x1024";
    assert!(runtime_display_losses(
        &assigned,
        &present(&[DELL, BENQ, other]),
        &present(&[DELL, BENQ])
    )
    .is_empty());
    assert!(runtime_display_returns(
        &assigned,
        &present(&[DELL, BENQ]),
        &present(&[DELL, BENQ, other])
    )
    .is_empty());
}

// ── stable present-set for identical monitors (issue #1125) ──────────────────

/// A profile of two identical Station monitors, disambiguated by position, each
/// carrying one participant pane — plus the viewscreen every assigning profile
/// must name (issue #1327). The viewscreen is a third, unrelated monitor, so the
/// twins are still the only thing this fixture is about.
fn two_identical_stations() -> ValidatedProfile {
    profile_with(vec![
        viewscreen_entry(DELL),
        station_entry("ACME 1080@1920x1080#0,0", &["Ada"]),
        station_entry("ACME 1080@1920x1080#1920,0", &["Grace"]),
    ])
    .validate()
    .unwrap()
}

#[test]
fn a_present_monitor_keeps_its_suffixed_identity_after_its_twin_leaves() {
    // The bug fix at the pure seam. `identify` would give the lone survivor of a
    // pair of identical monitors the SHORT key (no `#x,y` suffix), because it
    // disambiguates against the live set. Matching assignments against the raw
    // monitors directly keeps each present monitor's suffixed identity stable
    // regardless of its twin — so the survivor is still recognised as present.
    let assigned = two_identical_stations().assigned_surfaces();
    let both = [
        raw("ACME 1080", 1920, 1080, 0, 0),
        raw("ACME 1080", 1920, 1080, 1920, 0),
    ];
    let present_both = present_assigned_identities(&assigned, &both);
    assert_eq!(
        present_both,
        present(&["ACME 1080@1920x1080#0,0", "ACME 1080@1920x1080#1920,0"]),
        "both twins are present under their suffixed identities"
    );

    // The monitor at (0,0) is unplugged; only the (1920,0) one remains.
    let survivor = [raw("ACME 1080", 1920, 1080, 1920, 0)];
    let present_survivor = present_assigned_identities(&assigned, &survivor);
    assert_eq!(
        present_survivor,
        present(&["ACME 1080@1920x1080#1920,0"]),
        "the survivor keeps its OWN suffixed identity — it is not misread as gone"
    );

    // So the diff names exactly the removed monitor's pane, and never the
    // survivor's — the whole point of the fix.
    let losses = runtime_display_losses(&assigned, &present_both, &present_survivor);
    assert_eq!(losses.len(), 1, "exactly one monitor was lost");
    assert_eq!(losses[0].identity.as_str(), "ACME 1080@1920x1080#0,0");
    assert_eq!(losses[0].pane_labels, vec!["Ada".to_string()]);
}

// ── an identity that survives a roster change (issue #1330) ──────────────────
//
// `present_assigned_identities` above answers "is this ASSIGNMENT still on
// screen". `identify_stable` answers the same question about a whole live
// roster, which is what the lobby's monitor row and the bridge layout are
// rebuilt from — and the three cases below are the ones where re-deriving with
// `identify` would have thrown away the display the operator chose.

/// The identity strings `identify_stable` gives `raws`, given what was known.
fn stable(raws: &[RawMonitor], known: &[DiscoveredMonitor]) -> Vec<String> {
    identify_stable(raws, known)
        .iter()
        .map(|d| d.identity.as_str().to_string())
        .collect()
}

#[test]
fn a_roster_nothing_is_known_about_is_identified_exactly_as_before() {
    // The boot path, and the property that keeps every #1123 identity claim
    // true: with nothing to carry forward this IS `identify`.
    let raws = [
        raw("DELL U2720Q", 3840, 2160, 0, 0),
        raw("ACME 1080", 1920, 1080, 3840, 0),
        raw("ACME 1080", 1920, 1080, 5760, 0),
    ];
    assert_eq!(identify_stable(&raws, &[]), identify(&raws));
}

#[test]
fn a_known_display_keeps_its_short_key_when_its_twin_arrives() {
    // `identify` alone suffixes BOTH twins the moment the second one appears,
    // so the display an operator has been looking at changes its name behind
    // their back. Only the newcomer pays the disambiguator here.
    let alone = [raw("ACME 1080", 1920, 1080, 0, 0)];
    let known = identify(&alone);
    let with_twin = [
        raw("ACME 1080", 1920, 1080, 0, 0),
        raw("ACME 1080", 1920, 1080, 1920, 0),
    ];
    assert_eq!(
        stable(&with_twin, &known),
        vec!["ACME 1080@1920x1080", "ACME 1080@1920x1080#1920,0"]
    );
}

#[test]
fn a_known_display_keeps_its_suffixed_key_when_its_twin_leaves() {
    // The mirror. `identify` would collapse the lone survivor back to the short
    // key, and a layout comparing that against what it stored would conclude
    // its own viewscreen had been unplugged.
    let both = [
        raw("ACME 1080", 1920, 1080, 0, 0),
        raw("ACME 1080", 1920, 1080, 1920, 0),
    ];
    let known = identify(&both);
    let survivor = [raw("ACME 1080", 1920, 1080, 1920, 0)];
    assert_eq!(
        stable(&survivor, &known),
        vec!["ACME 1080@1920x1080#1920,0"]
    );
}

#[test]
fn a_display_that_renegotiated_its_mode_is_matched_by_name_and_place() {
    // A television waking rewrites the `WxH` half outright, so no form match
    // can find it — but it is the same screen in the same place, and its
    // geometry follows while its key does not.
    let before = [raw("BRAVIA", 3840, 2160, 0, 0)];
    let known = identify(&before);
    let after = [raw("BRAVIA", 1920, 1080, 0, 0)];
    let out = identify_stable(&after, &known);
    assert_eq!(out[0].identity.as_str(), "BRAVIA@3840x2160");
    assert_eq!(
        (
            out[0].geometry.physical_width,
            out[0].geometry.physical_height
        ),
        (1920, 1080),
        "the KEY is carried; the geometry is whatever it is running at now"
    );
}

#[test]
fn a_display_that_only_moved_is_still_matched_wherever_it_went() {
    // The OS-settings rearrange `identify`'s scheme was built to survive, held
    // through the stable pass as well.
    let before = [
        raw("DELL U2720Q", 3840, 2160, 0, 0),
        raw("BenQ EX", 1920, 1080, 3840, 0),
    ];
    let known = identify(&before);
    let after = [
        raw("BenQ EX", 1920, 1080, -1920, 0),
        raw("DELL U2720Q", 3840, 2160, 0, 0),
    ];
    assert_eq!(
        stable(&after, &known),
        vec!["BenQ EX@1920x1080", "DELL U2720Q@3840x2160"]
    );
}

#[test]
fn a_display_that_moved_and_changed_mode_at_once_is_treated_as_a_new_one() {
    // Nothing anchors that match, and guessing would re-home the viewscreen
    // onto a screen the operator did not choose. Conservative on purpose: it
    // reads as one display leaving and another arriving, which is the doctrine
    // #1123 already holds for a display it cannot account for.
    let before = [raw("BRAVIA", 3840, 2160, 0, 0)];
    let known = identify(&before);
    let after = [raw("BRAVIA", 1920, 1080, 1920, 0)];
    assert_eq!(stable(&after, &known), vec!["BRAVIA@1920x1080"]);
}

#[test]
fn a_newcomer_never_takes_a_key_a_carried_display_is_still_using() {
    // The one collision this scheme can produce: a display carries `@3840x2160`
    // forward after dropping to 1080p, and then a second display arrives in the
    // mode the first one left. Deduplicated rather than allowed to repeat — a
    // repeated identity is silently dropped by the layout, which is a display
    // with no button.
    let before = [raw("BRAVIA", 3840, 2160, 0, 0)];
    let known = identify(&before);
    let after = [
        raw("BRAVIA", 1920, 1080, 0, 0),
        raw("BRAVIA", 3840, 2160, 1920, 0),
    ];
    let out = stable(&after, &known);
    assert_eq!(out[0], "BRAVIA@3840x2160", "carried by name and place");
    assert_eq!(
        out[1], "BRAVIA@3840x2160#1920,0",
        "so the newcomer stands aside"
    );
    assert_ne!(out[0], out[1]);
}

#[test]
fn a_display_that_is_genuinely_gone_is_still_gone() {
    // The carry-forward must not resurrect anything: an unplug has to keep
    // reading as an unplug, or the row would go on offering a dead button.
    let before = [
        raw("DELL U2720Q", 3840, 2160, 0, 0),
        raw("BenQ EX", 1920, 1080, 3840, 0),
    ];
    let known = identify(&before);
    let after = [raw("DELL U2720Q", 3840, 2160, 0, 0)];
    assert_eq!(stable(&after, &known), vec!["DELL U2720Q@3840x2160"]);
}
