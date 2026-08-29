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
            .map(|l| PaneSlot {
                label: l.to_string(),
            })
            .collect(),
    }
}

fn profile_with(entries: Vec<DisplayEntry>) -> BridgeProfile {
    BridgeProfile {
        version: PROFILE_VERSION,
        displays: entries,
        touch: Vec::new(),
    }
}

#[test]
fn a_station_of_one_or_two_panes_validates() {
    let one = profile_with(vec![station_entry("m1", &["Ada"])]);
    let two = profile_with(vec![station_entry("m2", &["Ada", "Grace"])]);
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
        panes: vec![PaneSlot {
            label: "Ada".to_string(),
        }],
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
                    PaneSlot {
                        label: "Ada".to_string(),
                    },
                    PaneSlot {
                        label: "Grace".to_string(),
                    },
                ],
            },
            station_entry("Acer@1280x1024", &["Kay"]),
        ],
        touch: vec![TouchMapping {
            device: "ELAN Touchscreen".to_string(),
            monitor: "BenQ EX@1920x1080".to_string(),
        }],
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
                panes: vec![PaneSlot {
                    label: "Ada".to_string()
                }],
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
    let dell = assigned.iter().find(|a| a.identity.as_str() == DELL).unwrap();
    let benq = assigned.iter().find(|a| a.identity.as_str() == BENQ).unwrap();
    assert!(dell.pane_labels.is_empty(), "the viewscreen carries no panes");
    assert_eq!(benq.pane_labels, vec!["Ada".to_string()]);
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
    assert!(text.contains("not moved to another display") || text.contains("rather than moved"), "{text}");
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
    assert!(runtime_display_losses(&assigned, &present(&[DELL, BENQ, other]), &present(&[DELL, BENQ])).is_empty());
    assert!(runtime_display_returns(&assigned, &present(&[DELL, BENQ]), &present(&[DELL, BENQ, other])).is_empty());
}
