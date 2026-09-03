//! Remembering the bridge, against a real running host (issue #1334).
//!
//! Every test here drives the **actual plugins** —
//! [`BridgeDisplayPlugin`](crate::native_host::bridge_display::BridgeDisplayPlugin)
//! seeding the law from injected `Monitor` entities, and
//! [`BridgeLayoutStorePlugin`] beside it — against a store rooted in a scratch
//! directory. So the ordering, the run conditions and the hull-known moment are
//! covered by the ordinary `cargo test` runs rather than only by a machine with
//! three screens, and nothing here can touch a developer's real
//! `%APPDATA%\ProjectPhoenix\bridge-layouts`.

use super::*;

use bevy::window::{Monitor, PrimaryMonitor, PrimaryWindow, Window};

use crate::core::messages::StationId;
use crate::native_host::bridge_display::BridgeDisplayPlugin;
use crate::native_host::bridge_layout::LayoutAction;
use crate::native_host::bridge_profile::{
    BridgeProfile, DisplayEntry, MonitorIdentity, PaneSlot, ValidatedProfile, PROFILE_VERSION,
    ROLE_STATION, ROLE_VIEWSCREEN,
};

const DELL: &str = "DELL U2720Q@3840x2160";
const BENQ: &str = "BenQ EX@1920x1080";
const ACME: &str = "ACME 1080@1920x1080";

const DESTROYER: &str = "assets/entities/alliance_destroyer.toml";
const CRUISER: &str = "assets/entities/alliance_cruiser.toml";

// ── fixtures ────────────────────────────────────────────────────────────────

/// A scratch directory that removes itself. See `layout_store_tests` for why
/// the location is never the operator's own.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "phoenix-layout-systems-{}-{}-{label}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn store(&self) -> LayoutStore {
        LayoutStore::at(&self.0)
    }

    fn file(&self, class: &str) -> std::path::PathBuf {
        self.0.join(format!("{class}.toml"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn monitor(name: &str, w: u32, h: u32, x: i32) -> Monitor {
    Monitor {
        name: Some(name.to_string()),
        physical_width: w,
        physical_height: h,
        physical_position: IVec2::new(x, 0),
        refresh_rate_millihertz: Some(60_000),
        scale_factor: 1.0,
        video_modes: Vec::new(),
    }
}

fn station(id: &str) -> StationId {
    StationId(id.to_string())
}

fn m(id: &str) -> MonitorIdentity {
    MonitorIdentity::new(id)
}

/// A three-station hull, as `install_world_selection` would have parsed it.
fn hull() -> crate::ship::components::PendingShipConfig {
    crate::ship::components::PendingShipConfig(
        toml::from_str(
            r#"
            [[station]]
            id = "helm"
            name = "Helm"
            description = "-"
            rank = "Crew"

            [[station]]
            id = "weapons"
            name = "Tactical"
            description = "-"
            rank = "Crew"

            [[station]]
            id = "comms"
            name = "Comms"
            description = "-"
            rank = "Crew"
            "#,
        )
        .expect("a three-station hull parses"),
    )
}

/// A host with three screens and the two plugins, **before** its first frame.
///
/// `profile` is an operator's `--profile`; `None` is the ordinary
/// `phoenix-host --world …` this issue is about. The store resource is inserted
/// only when the run is the lobby's to remember, which is exactly what
/// `app::build_native_host_app` does — so an authored run here is authored the
/// same way the shipped one is.
fn host(scratch: &Scratch, profile: Option<ValidatedProfile>) -> App {
    let mut app = App::new();
    app.add_plugins(BridgeDisplayPlugin);
    app.add_plugins(BridgeLayoutStorePlugin);
    match profile {
        Some(profile) => {
            app.insert_resource(crate::native_host::bridge_display::BridgeDisplayConfig {
                profile,
                authored: true,
            });
        }
        None => {
            app.insert_resource(BridgeLayoutStore {
                store: scratch.store(),
                remembered: None,
            });
        }
    }
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.world_mut()
        .spawn((monitor("DELL U2720Q", 3840, 2160, 0), PrimaryMonitor));
    app.world_mut().spawn(monitor("BenQ EX", 1920, 1080, 3840));
    app.world_mut()
        .spawn(monitor("ACME 1080", 1920, 1080, 5760));
    app
}

/// Tell the host which hull it is flying — the `install_world_selection`
/// moment, which is boot on a `--world` host and the pick on a `--lobby` one.
fn fly(app: &mut App, template_path: &str) {
    app.insert_resource(crate::lobby::SelectedShipResource(
        template_path.to_string(),
    ));
    app.insert_resource(hull());
}

/// A `--world` host: the hull is known before the first frame.
fn booted(scratch: &Scratch, template_path: &str) -> App {
    let mut app = host(scratch, None);
    fly(&mut app, template_path);
    app.update();
    app
}

/// Move the layout as an accepted lobby press does — through the law, onto the
/// live resource, exactly as `host_lobby::drain_surface_records` does it.
fn press(app: &mut App, action: LayoutAction) {
    let next = app
        .world()
        .resource::<BridgeLayoutResource>()
        .layout
        .apply(&action)
        .unwrap_or_else(|e| panic!("the press was refused: {e}"));
    app.world_mut()
        .resource_mut::<BridgeLayoutResource>()
        .layout = next;
    app.update();
}

fn live(app: &App) -> &BridgeLayout {
    &app.world().resource::<BridgeLayoutResource>().layout
}

/// Pull a display's cable, as `bevy_winit` does — despawn its `Monitor` entity
/// — and run out the display watcher's own debounce so the law believes it.
fn unplug(app: &mut App, name: &str) {
    let mut found = None;
    let mut query = app.world_mut().query::<(bevy::prelude::Entity, &Monitor)>();
    for (entity, monitor) in query.iter(app.world()) {
        if monitor.name.as_deref() == Some(name) {
            found = Some(entity);
        }
    }
    app.world_mut()
        .entity_mut(found.expect("the monitor this test named is there"))
        .despawn();
    // Comfortably past `bridge_display`'s `DISPLAY_LOSS_DEBOUNCE_FRAMES`.
    for _ in 0..16 {
        app.update();
    }
}

/// The saved layout on disk, adopted onto a fresh three-screen bridge — the
/// next session's boot, without the next session.
fn saved(scratch: &Scratch, class: &str) -> BridgeLayout {
    let key = ShipClassKey::from_template_path(class).unwrap();
    let profile = scratch
        .store()
        .load(&key)
        .expect("the file reads")
        .expect("and there is one");
    let bridge = BridgeLayout::new(
        [m(DELL), m(BENQ), m(ACME)],
        ["helm", "weapons", "comms"].map(station),
        &m(DELL),
    )
    .unwrap();
    bridge.adopt_profile(&profile).0
}

// ── the round trip, through a running host ──────────────────────────────────

#[test]
fn a_class_nobody_has_arranged_writes_nothing_until_the_first_press() {
    // "Created on first write" — the boot layout is not itself a change the
    // operator made, and a host that wrote one at every launch would have no
    // way of telling a bridge somebody arranged from a bridge nobody has
    // touched.
    let scratch = Scratch::new("first-write");
    let mut app = booted(&scratch, DESTROYER);
    app.update();
    app.update();
    assert!(!scratch.file("alliance_destroyer").exists());

    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );
    assert!(scratch.file("alliance_destroyer").exists());
}

#[test]
fn arranging_the_bridge_and_relaunching_puts_it_back() {
    // Acceptance criterion one, end to end: arrange, quit, relaunch, pick the
    // same class — the viewscreen is on the remembered monitor and the assigned
    // consoles reopen on their remembered screens.
    let scratch = Scratch::new("relaunch");
    {
        let mut app = booted(&scratch, DESTROYER);
        press(&mut app, LayoutAction::SetViewscreen { monitor: m(ACME) });
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("helm"),
                monitor: m(DELL),
            },
        );
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("weapons"),
                monitor: m(BENQ),
            },
        );
    }

    // A new process, on the same three screens.
    let app = booted(&scratch, DESTROYER);
    let layout = live(&app);
    assert_eq!(
        layout.viewscreen(),
        &m(ACME),
        "the viewscreen is back on the monitor the operator chose, not the primary"
    );
    assert_eq!(layout.monitor_of(&station("helm")), Some(&m(DELL)));
    assert_eq!(layout.monitor_of(&station("weapons")), Some(&m(BENQ)));
    assert_eq!(
        layout.monitor_of(&station("comms")),
        None,
        "and a station nobody placed is still unplaced"
    );
}

#[test]
fn a_lobby_host_pre_applies_at_the_pick_rather_than_at_boot() {
    // Acceptance criterion two's other path (issue #1326's `--lobby` host): the
    // hull is not known at boot, so there is no class to load until the operator
    // picks one. Before the pick the bridge is the displays as found and the
    // roster is empty; the pick is what brings BOTH the roster and the
    // remembered arrangement.
    let scratch = Scratch::new("lobby-pick");
    {
        let mut app = booted(&scratch, DESTROYER);
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("comms"),
                monitor: m(ACME),
            },
        );
    }

    let mut app = host(&scratch, None);
    app.update();
    app.update();
    assert!(
        live(&app).roster().is_empty(),
        "a world-less host has no hull yet, so it has no stations to seat"
    );
    assert_eq!(live(&app).viewscreen(), &m(DELL), "the primary, as found");

    fly(&mut app, DESTROYER);
    app.update();

    assert_eq!(
        live(&app).roster().len(),
        3,
        "the pick brings the hull's roster, which is what the station rows are a map over"
    );
    assert_eq!(
        live(&app).monitor_of(&station("comms")),
        Some(&m(ACME)),
        "and the arrangement this class was left in"
    );
}

#[test]
fn two_classes_hold_independent_layouts() {
    // Acceptance criterion three: arranging the destroyer never touches the
    // cruiser's.
    let scratch = Scratch::new("two-classes");
    {
        let mut app = booted(&scratch, DESTROYER);
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("helm"),
                monitor: m(BENQ),
            },
        );
    }
    {
        let mut app = booted(&scratch, CRUISER);
        assert_eq!(
            live(&app).monitor_of(&station("helm")),
            None,
            "a class nobody has arranged starts from the displays as found — it does not \
             inherit the destroyer's"
        );
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("helm"),
                monitor: m(ACME),
            },
        );
    }

    assert_eq!(
        saved(&scratch, DESTROYER).monitor_of(&station("helm")),
        Some(&m(BENQ))
    );
    assert_eq!(
        saved(&scratch, CRUISER).monitor_of(&station("helm")),
        Some(&m(ACME))
    );
}

#[test]
fn every_accepted_change_is_filed_including_closing_a_console_again() {
    // The write trigger is every accepted change, not just the ones that add
    // something — an operator who takes a console OFF a screen and quits must
    // not find it back next session.
    let scratch = Scratch::new("every-change");
    let mut app = booted(&scratch, DESTROYER);
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );
    assert_eq!(
        saved(&scratch, DESTROYER).monitor_of(&station("helm")),
        Some(&m(BENQ))
    );

    press(
        &mut app,
        LayoutAction::UnassignStation {
            station: station("helm"),
        },
    );
    assert_eq!(
        saved(&scratch, DESTROYER).monitor_of(&station("helm")),
        None,
        "the close was filed too"
    );
}

#[test]
fn a_cable_coming_out_mid_session_leaves_the_saved_layout_alone() {
    // The saved layout is the operator's INTENT. A screen blipping through a
    // dock degrades the live arrangement — that is the law, and it is right —
    // but writing that degradation would permanently forget where two consoles
    // were, on a bridge nobody touched. So the unplug re-baselines and writes
    // nothing, and the FULL arrangement is still in the file.
    let scratch = Scratch::new("unplug");
    let mut app = booted(&scratch, DESTROYER);
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("weapons"),
            monitor: m(ACME),
        },
    );

    unplug(&mut app, "ACME 1080");
    assert_eq!(
        live(&app).monitor_of(&station("weapons")),
        None,
        "the LIVE bridge degrades, because the screen really is gone"
    );
    let on_disk = saved(&scratch, DESTROYER);
    assert_eq!(
        on_disk.monitor_of(&station("weapons")),
        Some(&m(ACME)),
        "and the FILE still knows where that console lives when the screen is back"
    );
    assert_eq!(on_disk.monitor_of(&station("helm")), Some(&m(BENQ)));
}

#[test]
fn and_re_assigning_after_the_unplug_writes_the_updated_layout() {
    // The other half of #1334's changed-monitor criterion, and what keeps the
    // rule above from being "the file is frozen after any unplug": the press
    // that puts the console somewhere real files the new arrangement, because
    // by then the bridge and the baseline agree again.
    let scratch = Scratch::new("unplug-then-press");
    let mut app = booted(&scratch, DESTROYER);
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("weapons"),
            monitor: m(ACME),
        },
    );
    unplug(&mut app, "ACME 1080");

    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("weapons"),
            monitor: m(BENQ),
        },
    );

    let key = ShipClassKey::from_template_path(DESTROYER).unwrap();
    let profile = scratch.store().load(&key).unwrap().unwrap();
    let two_screens = BridgeLayout::new(
        [m(DELL), m(BENQ)],
        ["helm", "weapons", "comms"].map(station),
        &m(DELL),
    )
    .unwrap();
    assert_eq!(
        two_screens
            .adopt_profile(&profile)
            .0
            .monitor_of(&station("weapons")),
        Some(&m(BENQ)),
        "the re-assignment reached the file"
    );
}

#[test]
fn a_bridge_nobody_touched_is_not_rewritten_frame_after_frame() {
    // The `saved` value compare, not change detection alone: a press the law's
    // no-op doctrine accepts without changing anything still marks the
    // resource, and a writer keyed only on the change tick would rewrite the
    // file (and log a line) every time.
    let scratch = Scratch::new("no-op");
    let mut app = booted(&scratch, DESTROYER);
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );
    let stamp = std::fs::metadata(scratch.file("alliance_destroyer"))
        .unwrap()
        .modified()
        .unwrap();

    // The same press again, which `apply` answers with an identical layout.
    for _ in 0..8 {
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("helm"),
                monitor: m(BENQ),
            },
        );
    }
    assert_eq!(
        std::fs::metadata(scratch.file("alliance_destroyer"))
            .unwrap()
            .modified()
            .unwrap(),
        stamp,
        "nothing changed, so nothing was written"
    );
}

// ── a bridge whose screens changed (acceptance criterion four) ──────────────

#[test]
fn a_station_whose_remembered_monitor_is_missing_comes_back_unassigned_and_re_assigning_saves() {
    let scratch = Scratch::new("changed-screens");
    {
        let mut app = booted(&scratch, DESTROYER);
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("helm"),
                monitor: m(BENQ),
            },
        );
        press(
            &mut app,
            LayoutAction::AssignStation {
                station: station("weapons"),
                monitor: m(ACME),
            },
        );
    }

    // Next session, on a bridge that has lost the ACME.
    let mut app = App::new();
    app.add_plugins(BridgeDisplayPlugin);
    app.add_plugins(BridgeLayoutStorePlugin);
    app.insert_resource(BridgeLayoutStore {
        store: scratch.store(),
        remembered: None,
    });
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.world_mut()
        .spawn((monitor("DELL U2720Q", 3840, 2160, 0), PrimaryMonitor));
    app.world_mut().spawn(monitor("BenQ EX", 1920, 1080, 3840));
    fly(&mut app, DESTROYER);
    app.update();

    assert_eq!(
        live(&app).monitor_of(&station("helm")),
        Some(&m(BENQ)),
        "everything whose screen is still here applies"
    );
    assert_eq!(
        live(&app).monitor_of(&station("weapons")),
        None,
        "and the one whose screen is gone is unassigned — never an error"
    );

    // Re-assigning it writes the updated layout.
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("weapons"),
            monitor: m(BENQ),
        },
    );
    let key = ShipClassKey::from_template_path(DESTROYER).unwrap();
    let profile = scratch.store().load(&key).unwrap().unwrap();
    let two_screens = BridgeLayout::new(
        [m(DELL), m(BENQ)],
        ["helm", "weapons", "comms"].map(station),
        &m(DELL),
    )
    .unwrap();
    assert_eq!(
        two_screens
            .adopt_profile(&profile)
            .0
            .monitor_of(&station("weapons")),
        Some(&m(BENQ))
    );
}

// ── a file that no longer works (acceptance criterion five) ─────────────────

#[test]
fn a_saved_layout_that_fails_validation_is_ignored_and_the_host_still_runs() {
    let scratch = Scratch::new("invalid");
    std::fs::create_dir_all(scratch.store().root()).unwrap();
    std::fs::write(
        scratch.file("alliance_destroyer"),
        "this is not a bridge profile at all",
    )
    .unwrap();

    let app = booted(&scratch, DESTROYER);
    assert_eq!(
        live(&app).viewscreen(),
        &m(DELL),
        "the host booted, on the displays as found"
    );
    assert_eq!(live(&app).roster().len(), 3, "with the hull's roster");
    assert!(live(&app)
        .roster()
        .iter()
        .all(|s| live(&app).monitor_of(s).is_none()));
    assert!(
        scratch.file("alliance_destroyer").exists(),
        "and the file the operator would go and look at is left where it is"
    );
}

// ── a --profile copied into the store (#1334 fix round) ────────────────────

#[test]
fn a_profile_copied_into_the_store_is_ignored_and_seats_no_phantom_console() {
    // The store's directory is one an operator browses, and what it holds is a
    // bridge profile they may hand back to `--profile` — so copying a
    // `--profile` INTO it is an invitation this feature extends, and the file
    // that arrives is perfectly valid. It must still be refused, because
    // adopting it would populate `reserved` on a run NOBODY AUTHORED:
    //
    //  * the BenQ would be reported full at two seats nobody can see, its
    //    Station window opened with a permanently dead half;
    //  * every screen-row press onto it would be refused, naming consoles the
    //    operator cannot find; and
    //  * the first press anywhere would then hit `save`'s
    //    `WouldDropReservations` — for ever, because the file that caused it is
    //    never rewritten. One ship class, silently un-saveable, with the only
    //    line in the log being the wrong sentence.
    let scratch = Scratch::new("copied-profile");
    std::fs::create_dir_all(scratch.store().root()).unwrap();
    let copied = BridgeProfile {
        version: PROFILE_VERSION,
        displays: vec![
            DisplayEntry {
                id: ACME.to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            DisplayEntry {
                id: BENQ.to_string(),
                role: ROLE_STATION.to_string(),
                split: None,
                panes: vec![
                    PaneSlot::for_participant("Ada"),
                    PaneSlot::for_participant("Grace"),
                ],
            },
        ],
        touch: Vec::new(),
        media: Vec::new(),
    };
    assert!(
        copied.clone().validate().is_ok(),
        "the fixture is a profile the flag itself would take — that is the trap"
    );
    let text = copied.to_toml().unwrap();
    std::fs::write(scratch.file("alliance_destroyer"), &text).unwrap();

    let mut app = booted(&scratch, DESTROYER);

    assert_eq!(
        live(&app).viewscreen(),
        &m(DELL),
        "the host runs on the bridge it BOOTED with — the copied file's viewscreen is not adopted"
    );
    assert_eq!(live(&app).roster().len(), 3, "with the hull's roster");
    assert!(
        live(&app).reserved_on(&m(BENQ)).is_empty(),
        "and no phantom occupant: nothing but an authored --profile may populate `reserved`"
    );
    assert!(
        live(&app).occupants_on(&m(BENQ)).is_empty(),
        "the screen really is free, not merely free of SEATS"
    );
    let row = live(&app)
        .eligibility_of(&station("weapons"))
        .expect("weapons is on the roster");
    assert_eq!(
        row.choice_for(&m(BENQ)),
        Some(crate::native_host::bridge_layout::MonitorChoice::Eligible),
        "so the operator's own screen row offers it, rather than greying it for two \
         consoles that are nowhere on screen"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.file("alliance_destroyer")).unwrap(),
        text,
        "and the file is left exactly where it is — an operator who put it there wants to \
         see what they put"
    );

    // And the class is NOT un-saveable. The refused file is a file like any
    // other unusable one: this session runs on the bridge it has, and the first
    // arrangement made from the lobby files a real layout over it.
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("weapons"),
            monitor: m(BENQ),
        },
    );
    assert_eq!(
        saved(&scratch, DESTROYER).monitor_of(&station("weapons")),
        Some(&m(BENQ)),
        "the press saved normally, and what is on disk now IS a lobby layout"
    );
}

// ── a disk that will not take it (#1334 fix round) ─────────────────────────

#[test]
fn a_store_that_cannot_be_written_retries_once_per_change_not_once_per_frame() {
    // A `LayoutStoreError::Write` is almost never transient — a read-only
    // `%APPDATA%`, a full disk, a scanner holding the file. `saved` is
    // deliberately left alone on a failure so the next press retries, which
    // taken alone means `saved != layout` stays true and the warning repeats at
    // the FRAME RATE for the rest of the run, burying `LogCat::Lobby` under one
    // sentence. The retry is keyed on the accepted change instead.
    let scratch = Scratch::new("unwritable");
    let (mut app, root) = host_with_an_unwritable_store(&scratch);

    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );

    let refused = live(&app).clone();
    let remembered = app
        .world()
        .resource::<BridgeLayoutStore>()
        .remembered
        .clone()
        .expect("the class was taken up even though its store is unusable");
    assert_eq!(
        remembered.unsaved.as_ref(),
        Some(&refused),
        "the disk refused THIS arrangement, and that is what is recorded"
    );
    assert_ne!(
        remembered.saved, refused,
        "`saved` still records what is on disk, which is nothing — a bridge the file never \
         received must not look filed"
    );

    // The operator fixes the directory. THIRTY IDLE FRAMES STILL WRITE NOTHING:
    // the retry waits for a change, not for a frame. Without the gate the very
    // next frame would find `saved != layout` and file it — and every frame
    // before this one would have warned.
    std::fs::remove_file(&root).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    for _ in 0..30 {
        app.update();
    }
    assert!(
        !root.join("alliance_destroyer.toml").exists(),
        "an idle bridge does not retry, however writable the directory has become"
    );

    // The next accepted change does, and recovers — the file appears, carrying
    // both the press that failed and the one that succeeded.
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("weapons"),
            monitor: m(ACME),
        },
    );
    let key = ShipClassKey::from_template_path(DESTROYER).unwrap();
    let profile = LayoutStore::at(&root)
        .load(&key)
        .expect("it reads")
        .expect("and there is one now");
    let bridge = BridgeLayout::new(
        [m(DELL), m(BENQ), m(ACME)],
        ["helm", "weapons", "comms"].map(station),
        &m(DELL),
    )
    .unwrap();
    let on_disk = bridge.adopt_profile(&profile).0;
    assert_eq!(on_disk.monitor_of(&station("helm")), Some(&m(BENQ)));
    assert_eq!(on_disk.monitor_of(&station("weapons")), Some(&m(ACME)));
    assert!(
        app.world()
            .resource::<BridgeLayoutStore>()
            .remembered
            .as_ref()
            .expect("still remembering")
            .unsaved
            .is_none(),
        "and the refusal is cleared, so a later failure is warned about again"
    );
}

/// A host whose store directory cannot be created: a **file** sits where the
/// directory belongs, which `create_dir_all` refuses on every platform. That is
/// a read-only `%APPDATA%` without needing one in CI.
///
/// Answers the app and the path of the blocking file, which a test removes to
/// let the writes through.
fn host_with_an_unwritable_store(scratch: &Scratch) -> (App, std::path::PathBuf) {
    std::fs::create_dir_all(&scratch.0).unwrap();
    let root = scratch.0.join("bridge-layouts");
    std::fs::write(&root, "not a directory").unwrap();

    let mut app = host(scratch, None);
    app.insert_resource(BridgeLayoutStore {
        store: LayoutStore::at(&root),
        remembered: None,
    });
    fly(&mut app, DESTROYER);
    app.update();
    (app, root)
}

fn unsaved_record(app: &App) -> Option<BridgeLayout> {
    app.world()
        .resource::<BridgeLayoutStore>()
        .remembered
        .as_ref()
        .expect("the class was taken up")
        .unsaved
        .clone()
}

#[test]
fn undoing_a_refused_change_and_making_it_again_files_it() {
    // THE OPERATOR'S OWN RECOVERY, and the one the warning promises them. A
    // press is refused by the disk (`%APPDATA%` locked down, a scanner holding
    // the file); they put the bridge back the way it was while they go and fix
    // it; then they make the same change again.
    //
    // That third press produces the arrangement the disk refused — NOTHING ELSE
    // CAN, because undo-then-redo is the only path back to it — so a refusal
    // record that only ever cleared on a successful write would suppress it, and
    // every press after it, for the rest of the run. The session would end with
    // one bridge on screen and another on disk, after a log line that had said
    // the next change would try again.
    let scratch = Scratch::new("undo-redo");
    let (mut app, root) = host_with_an_unwritable_store(&scratch);

    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );
    let refused = live(&app).clone();
    assert_eq!(
        unsaved_record(&app).as_ref(),
        Some(&refused),
        "the disk refused this arrangement, and that is what is recorded"
    );

    // They fix the directory, then put the bridge back — which is not a change
    // the file needs, because the file has never held anything else.
    std::fs::remove_file(&root).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    press(
        &mut app,
        LayoutAction::UnassignStation {
            station: station("helm"),
        },
    );
    assert!(
        !root.join("alliance_destroyer.toml").exists(),
        "the undo is not itself a change to file: what is on screen is what the disk holds"
    );
    assert_eq!(
        unsaved_record(&app),
        None,
        "and with the screen and the disk agreeing there is nothing outstanding, so the \
         refusal is cleared rather than left standing against an arrangement the operator \
         may well make again"
    );

    // The redo: the same press, the same arrangement, ONE new attempt — and it
    // lands, because the directory is writable now.
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );
    let key = ShipClassKey::from_template_path(DESTROYER).unwrap();
    let profile = LayoutStore::at(&root)
        .load(&key)
        .expect("it reads")
        .expect("and there is one now");
    let bridge = BridgeLayout::new(
        [m(DELL), m(BENQ), m(ACME)],
        ["helm", "weapons", "comms"].map(station),
        &m(DELL),
    )
    .unwrap();
    assert_eq!(
        bridge
            .adopt_profile(&profile)
            .0
            .monitor_of(&station("helm")),
        Some(&m(BENQ)),
        "the retry the warning promised actually happened"
    );
    assert_eq!(
        unsaved_record(&app),
        None,
        "and nothing is outstanding after a save that landed"
    );
}

#[test]
fn a_second_frame_carrying_the_same_refused_arrangement_offers_it_nothing() {
    // THE SUPPRESSION BRANCH ITSELF, which the run condition alone does not
    // reach: an idle bridge never gets here, but this module is not the law's
    // only writer, so `bridge_display`'s reconcilers holding a `ResMut` open the
    // gate on a frame that changed no arrangement at all. Without the refusal
    // record such a frame would re-offer the bridge the disk has already
    // declined and warn again — at the rate the other writers touch the
    // resource, for the rest of the run.
    //
    // The observable is the FILE: the directory is repaired first, so a frame
    // that reached the writer would leave a layout behind. Nothing appears.
    let scratch = Scratch::new("same-arrangement");
    let (mut app, root) = host_with_an_unwritable_store(&scratch);

    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(BENQ),
        },
    );
    let refused = live(&app).clone();
    assert_eq!(unsaved_record(&app).as_ref(), Some(&refused));

    std::fs::remove_file(&root).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    for _ in 0..3 {
        // Another writer's frame: the resource is marked changed, the layout is
        // not touched. This is what `bridge_display::reconcile_layout` does on a
        // frame that rebuilt an identical arrangement.
        app.world_mut()
            .resource_mut::<BridgeLayoutResource>()
            .set_changed();
        app.update();
    }

    assert!(
        !root.join("alliance_destroyer.toml").exists(),
        "the arrangement has already been offered to the disk and declined, so a frame \
         carrying that same one again has nothing to say"
    );
    assert_eq!(
        unsaved_record(&app).as_ref(),
        Some(&refused),
        "and the record still stands — it is cleared by the bridge changing, not by a frame \
         passing"
    );
}

// ── --profile precedence, and the data-loss guard (criterion six) ───────────

/// An authored profile: the Dell is the viewscreen, the BenQ carries two
/// hand-authored `--pane` participant slots — the shape whose loss this
/// criterion exists to prevent.
fn authored_with_two_panes() -> ValidatedProfile {
    BridgeProfile {
        version: PROFILE_VERSION,
        displays: vec![
            DisplayEntry {
                id: DELL.to_string(),
                role: ROLE_VIEWSCREEN.to_string(),
                split: None,
                panes: Vec::new(),
            },
            DisplayEntry {
                id: BENQ.to_string(),
                role: ROLE_STATION.to_string(),
                split: None,
                panes: vec![
                    PaneSlot::for_participant("Ada"),
                    PaneSlot::for_participant("Grace"),
                ],
            },
        ],
        touch: Vec::new(),
        media: Vec::new(),
    }
    .validate()
    .expect("the fixture profile is valid")
}

#[test]
fn a_profile_run_pre_applies_nothing_and_writes_nothing_even_after_lobby_edits() {
    // ACCEPTANCE CRITERION SIX, AND THE DATA-LOSS GUARD IT DOUBLES AS.
    //
    // `to_validated_profile` emits SEATS and only seats, so a saved
    // `--profile`-seeded layout would silently drop the operator's `--pane`
    // participant slots — and the NEXT boot would read the BenQ as free and
    // move the viewscreen on top of Ada's and Grace's live consoles. The
    // never-write-on-authored rule is what makes that unreachable, so it is
    // asserted directly: an authored run with lobby edits writes NOTHING.
    let scratch = Scratch::new("authored");

    // A saved layout for this very class already exists, so "nothing was
    // written" is a statement about an EXISTING file rather than about an
    // absent one — the destructive case, not the harmless one.
    let key = ShipClassKey::from_template_path(DESTROYER).unwrap();
    let previously = BridgeLayout::new(
        [m(DELL), m(BENQ), m(ACME)],
        ["helm", "weapons", "comms"].map(station),
        &m(DELL),
    )
    .unwrap()
    .apply(&LayoutAction::AssignStation {
        station: station("comms"),
        monitor: m(ACME),
    })
    .unwrap();
    scratch.store().save(&key, &previously).unwrap();
    let before = std::fs::read_to_string(scratch.file("alliance_destroyer")).unwrap();

    let mut app = host(&scratch, Some(authored_with_two_panes()));
    fly(&mut app, DESTROYER);
    app.update();

    assert!(
        app.world().get_resource::<BridgeLayoutStore>().is_none(),
        "a --profile run is not given a store at all — the FIRST of the two guards"
    );
    assert_eq!(
        live(&app).reserved_on(&m(BENQ)),
        &["Ada".to_string(), "Grace".to_string()],
        "the authored panes are on the live layout, which is what a save would drop"
    );
    assert_eq!(
        live(&app).monitor_of(&station("comms")),
        None,
        "and the saved layout was NOT pre-applied over the operator's profile"
    );

    // Now the operator rearranges from the lobby, which on a remembering host
    // is exactly the press that writes.
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(ACME),
        },
    );
    app.update();

    assert_eq!(
        std::fs::read_to_string(scratch.file("alliance_destroyer")).unwrap(),
        before,
        "the saved layout is byte-for-byte what it was: an authored run never writes"
    );
}

#[test]
fn a_store_handed_to_an_authored_run_anyway_is_still_never_written() {
    // The run condition on its own, with the composition-site guard deliberately
    // bypassed: `app::build_native_host_app` does not give a `--profile` run a
    // store, and the test above pins that — this pins the SECOND layer, so a
    // future composition that inserted one unconditionally could not start
    // writing over the operator's saved layouts without a test going red.
    let scratch = Scratch::new("authored-run-condition");
    let key = ShipClassKey::from_template_path(DESTROYER).unwrap();
    let previously = BridgeLayout::new(
        [m(DELL), m(BENQ), m(ACME)],
        ["helm", "weapons", "comms"].map(station),
        &m(DELL),
    )
    .unwrap();
    scratch.store().save(&key, &previously).unwrap();
    let before = std::fs::read_to_string(scratch.file("alliance_destroyer")).unwrap();

    let mut app = host(&scratch, Some(authored_with_two_panes()));
    app.insert_resource(BridgeLayoutStore {
        store: scratch.store(),
        remembered: None,
    });
    fly(&mut app, DESTROYER);
    app.update();
    press(
        &mut app,
        LayoutAction::AssignStation {
            station: station("helm"),
            monitor: m(ACME),
        },
    );

    assert!(
        app.world()
            .resource::<BridgeLayoutStore>()
            .remembered
            .is_none(),
        "the pre-apply never ran, so no class was ever taken up"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.file("alliance_destroyer")).unwrap(),
        before
    );
}

#[test]
fn the_store_itself_refuses_a_layout_carrying_authored_consoles() {
    // The SECOND of the two guards, asserted against a real authored run's live
    // layout rather than against a hand-built one: even if a future caller
    // reached the writer with this arrangement, the file could not be written.
    // Belt and braces, because the failure is silent and permanent.
    let scratch = Scratch::new("authored-store");
    let mut app = host(&scratch, Some(authored_with_two_panes()));
    fly(&mut app, DESTROYER);
    app.update();

    let key = ShipClassKey::from_template_path(DESTROYER).unwrap();
    let err = scratch
        .store()
        .save(&key, live(&app))
        .expect_err("a reservation-carrying layout is not savable");
    assert_eq!(
        err,
        crate::native_host::layout_store::LayoutStoreError::WouldDropReservations {
            class: "alliance_destroyer".to_string(),
            labels: vec!["Ada".to_string(), "Grace".to_string()],
        }
    );
    assert!(!scratch.file("alliance_destroyer").exists());
}
