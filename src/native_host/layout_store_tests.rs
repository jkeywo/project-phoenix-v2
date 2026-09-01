//! The saved bridge layouts (issue #1334), against an **injected** location.
//!
//! Every test here builds its own scratch directory under the OS temp dir and
//! removes it again, so the suite never reads or writes a developer's real
//! `%APPDATA%\ProjectPhoenix\bridge-layouts`. That is not tidiness: the whole
//! point of [`LayoutStore::at`] is that the one function which consults the
//! environment ([`LayoutStore::user`]) is never on a test's path, and a suite
//! that quietly clobbered a real bridge would be proof it was not.

use super::*;

use crate::core::messages::StationId;
use crate::native_host::bridge_layout::{BridgeLayout, LayoutAction, LAYOUT_SPLIT};
use crate::native_host::bridge_profile::{
    DisplayEntry, MonitorIdentity, PaneSlot, PROFILE_VERSION, ROLE_STATION, ROLE_VIEWSCREEN,
};

// ── fixtures ────────────────────────────────────────────────────────────────

const TV: &str = "BRAVIA@3840x2160";
const LEFT: &str = "BenQ EX@1920x1080";
const RIGHT: &str = "Acer VG@1280x1024";

/// A scratch directory that removes itself, so a failing assertion does not
/// leave a trail of half-written bridges in the temp dir.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        // Process id and a per-process counter, so two tests in one binary (and
        // two `cargo test` runs at once) never share a directory. `cargo test`
        // runs this file's tests on several threads by default, which is
        // exactly the collision this avoids.
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "phoenix-layout-store-{}-{}-{label}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn store(&self) -> LayoutStore {
        LayoutStore::at(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn m(id: &str) -> MonitorIdentity {
    MonitorIdentity::new(id)
}

fn s(id: &str) -> StationId {
    StationId(id.to_string())
}

fn roster() -> Vec<StationId> {
    ["helm", "weapons", "comms"].into_iter().map(s).collect()
}

/// The three-screen bridge every test starts from: the TV is the viewscreen.
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

fn destroyer() -> ShipClassKey {
    ShipClassKey::from_template_path("assets/entities/alliance_destroyer.toml")
        .expect("a hull path is a class key")
}

fn cruiser() -> ShipClassKey {
    ShipClassKey::from_template_path("assets/entities/alliance_cruiser.toml")
        .expect("a hull path is a class key")
}

// ── the class key ───────────────────────────────────────────────────────────

#[test]
fn a_class_is_keyed_by_its_hull_templates_file_stem() {
    assert_eq!(destroyer().as_str(), "alliance_destroyer");
    assert_eq!(cruiser().as_str(), "alliance_cruiser");
}

#[test]
fn the_key_is_the_same_whichever_separator_the_path_was_spelled_with() {
    // `install_world_selection` stores the CANONICAL path, so this should never
    // arrive with backslashes — but a key that changed with the spelling would
    // file one hull under two names the first time something forgot, and the
    // operator would find their layout had silently reset.
    let forward = ShipClassKey::from_template_path("assets/entities/alliance_destroyer.toml");
    let back = ShipClassKey::from_template_path("assets\\entities\\alliance_destroyer.toml");
    let mixed = ShipClassKey::from_template_path("./assets/entities/Alliance_Destroyer.TOML");
    assert_eq!(forward, back);
    assert_eq!(forward, mixed, "and case is not part of a file name here");
}

#[test]
fn a_key_can_never_name_a_file_outside_the_store() {
    // The reason `ShipClassKey` is a newtype rather than a `&str` parameter:
    // there is no way to hand the store a class name that has not been through
    // this. Separators, drive letters and dots all reduce.
    let store = LayoutStore::at("C:/bridge-layouts");
    let key = ShipClassKey::from_template_path("../../secrets/passwd.toml")
        .expect("it still names a stem");
    assert_eq!(key.as_str(), "passwd");
    assert_eq!(
        store.path_for(&key),
        std::path::Path::new("C:/bridge-layouts").join("passwd.toml")
    );

    // And a stem that is nothing but separators names no class at all.
    assert_eq!(ShipClassKey::from_template_path(".."), None);
    assert_eq!(ShipClassKey::from_template_path("assets/entities/"), None);
    assert_eq!(ShipClassKey::from_template_path(""), None);
    assert_eq!(ShipClassKey::from_template_path("/"), None);
}

#[test]
fn the_operators_own_store_sits_under_the_apps_data_directory() {
    // Reads no file and creates no directory — it resolves a path and checks
    // its shape, which is the half of `user()` that can be wrong. On Windows
    // this is `%APPDATA%\ProjectPhoenix\bridge-layouts`.
    let Some(store) = LayoutStore::user() else {
        // A machine with no home directory: the caller runs without saved
        // layouts, and so does this assertion.
        return;
    };
    assert!(store.root().ends_with(std::path::Path::new(LAYOUTS_DIR)));
    assert!(store
        .root()
        .parent()
        .expect("the layouts directory has a parent")
        .ends_with(std::path::Path::new(APP_DIR)));
}

// ── the round trip ──────────────────────────────────────────────────────────

#[test]
fn a_saved_layout_reloads_as_the_same_arrangement() {
    // Acceptance criterion one, at this altitude: arrange the bridge, save,
    // and read it back onto the same bridge. The viewscreen is on the monitor
    // the operator chose and each console is on the screen they put it on.
    let scratch = Scratch::new("round-trip");
    let store = scratch.store();

    let arranged = {
        let l = bridge()
            .apply(&LayoutAction::SetViewscreen { monitor: m(LEFT) })
            .expect("nothing is on the BenQ");
        let l = assign(&l, "helm", TV);
        assign(&l, "weapons", RIGHT)
    };

    let path = store.save(&destroyer(), &arranged).expect("the save lands");
    assert!(path.exists(), "created on first write");

    let profile = store
        .load(&destroyer())
        .expect("it reads back")
        .expect("and there is one");
    // Adopted onto a FRESH bridge, exactly as a later session's boot layout is:
    // this is the reload, not a comparison of two in-memory values.
    let (reloaded, notes) = bridge().adopt_profile(&profile);
    assert_eq!(notes, Vec::new(), "nothing degraded");
    assert_eq!(reloaded, arranged);
    assert_eq!(reloaded.viewscreen(), &m(LEFT));
    assert_eq!(reloaded.monitor_of(&s("helm")), Some(&m(TV)));
    assert_eq!(reloaded.monitor_of(&s("weapons")), Some(&m(RIGHT)));
    assert_eq!(reloaded.monitor_of(&s("comms")), None);
}

#[test]
fn a_class_nobody_has_arranged_yet_has_no_layout_and_that_is_not_an_error() {
    let scratch = Scratch::new("absent");
    let store = scratch.store();
    assert_eq!(store.load(&destroyer()), Ok(None));
    assert!(
        !store.root().exists(),
        "and reading does not create the directory — the FIRST WRITE does"
    );
}

#[test]
fn two_ship_classes_hold_independent_layouts() {
    // Acceptance criterion two. Arranging the destroyer never touches the
    // cruiser's, and the two files sit side by side under names an operator can
    // read.
    let scratch = Scratch::new("per-class");
    let store = scratch.store();

    let destroyer_bridge = assign(&bridge(), "helm", LEFT);
    let cruiser_bridge = assign(&bridge(), "helm", RIGHT);
    store.save(&destroyer(), &destroyer_bridge).unwrap();
    store.save(&cruiser(), &cruiser_bridge).unwrap();

    let back = |class: &ShipClassKey| {
        let profile = store.load(class).unwrap().unwrap();
        bridge().adopt_profile(&profile).0
    };
    assert_eq!(back(&destroyer()).monitor_of(&s("helm")), Some(&m(LEFT)));
    assert_eq!(back(&cruiser()).monitor_of(&s("helm")), Some(&m(RIGHT)));

    // Re-arranging one leaves the other exactly as it was.
    store.save(&destroyer(), &bridge()).unwrap();
    assert_eq!(
        back(&destroyer()).monitor_of(&s("helm")),
        None,
        "the destroyer's console was closed"
    );
    assert_eq!(
        back(&cruiser()).monitor_of(&s("helm")),
        Some(&m(RIGHT)),
        "and the cruiser's is exactly where it was"
    );
    assert_eq!(
        store.path_for(&cruiser()),
        store.root().join("alliance_cruiser.toml")
    );
}

// ── a bridge whose screens changed ──────────────────────────────────────────

#[test]
fn a_saved_station_whose_monitor_is_gone_comes_back_unassigned_and_the_rest_applies() {
    // Acceptance criterion three, at the store's own altitude: the degradation
    // is the LAW's (`adopt_profile` puts every seat through `assign`), and what
    // this pins is that a saved file reaches it — a missing screen is a
    // `SeatRefused` note and an unassigned station, never a failed load.
    let scratch = Scratch::new("missing-monitor");
    let store = scratch.store();

    let arranged = {
        let l = assign(&bridge(), "helm", LEFT);
        assign(&l, "weapons", RIGHT)
    };
    store.save(&destroyer(), &arranged).unwrap();

    // Next session, the Acer is unplugged.
    let smaller = BridgeLayout::new([m(TV), m(LEFT)], roster(), &m(TV)).unwrap();
    let profile = store.load(&destroyer()).unwrap().unwrap();
    let (adopted, notes) = smaller.adopt_profile(&profile);

    assert_eq!(
        adopted.monitor_of(&s("helm")),
        Some(&m(LEFT)),
        "everything else applies"
    );
    assert_eq!(
        adopted.monitor_of(&s("weapons")),
        None,
        "and the station whose screen is gone is simply unassigned"
    );
    assert_eq!(notes.len(), 1, "reported, not silent: {notes:?}");
}

#[test]
fn a_saved_viewscreen_whose_monitor_is_gone_leaves_the_bridge_on_the_one_it_booted_with() {
    let scratch = Scratch::new("missing-viewscreen");
    let store = scratch.store();
    store
        .save(
            &destroyer(),
            &bridge()
                .apply(&LayoutAction::SetViewscreen { monitor: m(RIGHT) })
                .unwrap(),
        )
        .unwrap();

    let smaller = BridgeLayout::new([m(TV), m(LEFT)], roster(), &m(TV)).unwrap();
    let profile = store.load(&destroyer()).unwrap().unwrap();
    let (adopted, notes) = smaller.adopt_profile(&profile);
    assert_eq!(
        adopted.viewscreen(),
        &m(TV),
        "the boot layout's choice stands rather than the load failing"
    );
    assert_eq!(notes.len(), 1, "and it is reported: {notes:?}");
}

// ── a file that is no longer usable ─────────────────────────────────────────

#[test]
fn a_saved_layout_that_no_longer_validates_is_refused_rather_than_adopted() {
    // Acceptance criterion four's other half — the caller's answer is a warning
    // (see `layout_store_systems`), and what THIS pins is that the failure is
    // typed and names the file rather than being swallowed into an `Ok(None)`
    // that would look like "you never saved one".
    let scratch = Scratch::new("invalid");
    let store = scratch.store();
    std::fs::create_dir_all(store.root()).unwrap();

    // Three panes on one Station: past the density rule, refused by `validate`.
    let profile = BridgeProfile {
        version: PROFILE_VERSION,
        displays: vec![
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
                    PaneSlot::for_station("helm"),
                    PaneSlot::for_station("weapons"),
                    PaneSlot::for_station("comms"),
                ],
            },
        ],
        touch: Vec::new(),
        media: Vec::new(),
    };
    std::fs::write(store.path_for(&destroyer()), profile.to_toml().unwrap()).unwrap();

    let err = store.load(&destroyer()).expect_err("it does not validate");
    assert!(
        matches!(
            err,
            LayoutStoreError::Profile {
                source: ProfileError::Density { .. },
                ..
            }
        ),
        "the file's own refusal is carried, not flattened: {err}"
    );
    assert!(
        err.to_string().contains("alliance_destroyer.toml"),
        "and it names the file the operator would go and look at: {err}"
    );
}

#[test]
fn a_saved_layout_that_is_not_toml_is_refused_as_a_parse_failure() {
    let scratch = Scratch::new("garbage");
    let store = scratch.store();
    std::fs::create_dir_all(store.root()).unwrap();
    // What half a file looks like — the shape a NON-atomic write would leave
    // behind after a crash, which is why `save` renames rather than truncating.
    std::fs::write(
        store.path_for(&destroyer()),
        "version = 1\n[[display]]\nid = \"BRAV",
    )
    .unwrap();

    let err = store.load(&destroyer()).expect_err("it does not parse");
    assert!(
        matches!(
            err,
            LayoutStoreError::Profile {
                source: ProfileError::Parse(_),
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn a_layout_written_by_a_different_schema_version_is_refused_rather_than_reinterpreted() {
    let scratch = Scratch::new("version");
    let store = scratch.store();
    std::fs::create_dir_all(store.root()).unwrap();
    let text = assign(&bridge(), "helm", LEFT)
        .to_profile()
        .to_toml()
        .unwrap()
        .replace(
            &format!("version = {PROFILE_VERSION}"),
            &format!("version = {}", PROFILE_VERSION + 1),
        );
    std::fs::write(store.path_for(&destroyer()), text).unwrap();

    assert!(matches!(
        store.load(&destroyer()),
        Err(LayoutStoreError::Profile {
            source: ProfileError::Version { .. },
            ..
        })
    ));
}

// ── the data-loss guard (issue #1334's acceptance criterion five) ───────────

#[test]
fn a_layout_carrying_an_authored_console_is_refused_at_the_door() {
    // The structural half of the never-write-a---profile-run rule. The POLICY
    // half is in `layout_store_systems` (an authored run's writer never runs at
    // all) and is tested there; this is the guard that makes the data loss
    // unreachable even from a caller that forgets it.
    //
    // What would be lost: `to_validated_profile` emits SEATS and only seats, so
    // a layout seeded from a `--profile` full of `--pane` participant slots
    // writes a file with those slots missing — and the next boot reads those
    // screens as free and moves the viewscreen on top of a live console.
    let scratch = Scratch::new("reserved");
    let store = scratch.store();

    let profile = BridgeProfile {
        version: PROFILE_VERSION,
        displays: vec![
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
        ],
        touch: Vec::new(),
        media: Vec::new(),
    }
    .validate()
    .unwrap();
    let seeded = bridge().adopt_profile(&profile).0;
    assert_eq!(
        seeded.reserved_on(&m(LEFT)),
        &["Ada".to_string(), "Grace".to_string()],
        "the fixture really is carrying two consoles the layout does not own"
    );
    // ... and the operator then rearranges something from the lobby, which is
    // the press that would otherwise trigger the save.
    let edited = assign(&seeded, "helm", RIGHT);

    let err = store
        .save(&destroyer(), &edited)
        .expect_err("a reservation-carrying layout is not savable");
    assert_eq!(
        err,
        LayoutStoreError::WouldDropReservations {
            class: "alliance_destroyer".to_string(),
            labels: vec!["Ada".to_string(), "Grace".to_string()],
        }
    );
    assert!(
        !store.path_for(&destroyer()).exists(),
        "and NOTHING was written — the refusal is before the disk, not a rollback"
    );
}

// ── the write itself ────────────────────────────────────────────────────────

#[test]
fn saving_twice_replaces_the_file_and_leaves_no_temporary_behind() {
    // The atomic write, from the outside: the second save replaces the first
    // (not appends, not fails on an existing destination — Windows'
    // `MoveFileExW` needs `MOVEFILE_REPLACE_EXISTING` for that, which is what
    // `std::fs::rename` passes), and the directory afterwards holds exactly one
    // file. A leftover `.tmp` would mean a crash had left debris an operator
    // has to clean up.
    let scratch = Scratch::new("atomic");
    let store = scratch.store();

    store
        .save(&destroyer(), &assign(&bridge(), "helm", LEFT))
        .unwrap();
    store
        .save(&destroyer(), &assign(&bridge(), "helm", RIGHT))
        .unwrap();

    let entries: Vec<String> = std::fs::read_dir(store.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(entries, vec!["alliance_destroyer.toml".to_string()]);

    let profile = store.load(&destroyer()).unwrap().unwrap();
    assert_eq!(
        bridge().adopt_profile(&profile).0.monitor_of(&s("helm")),
        Some(&m(RIGHT)),
        "the second arrangement is the one on disk"
    );
}

#[test]
fn the_saved_file_is_a_bridge_profile_an_operator_could_hand_back_to_the_flag() {
    // It is not an internal state dump: what is written is the same TOML
    // `--profile` reads, which is what makes "copy your layout to the other
    // machine" and "open it and see which monitor is the viewscreen" work.
    let scratch = Scratch::new("shape");
    let store = scratch.store();
    let path = store
        .save(&destroyer(), &assign(&bridge(), "helm", LEFT))
        .unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(&format!("version = {PROFILE_VERSION}")));
    assert!(text.contains(ROLE_VIEWSCREEN));
    assert!(text.contains(ROLE_STATION));
    assert!(text.contains("station = \"helm\""));
    // And it round-trips through the flag's own front door.
    BridgeProfile::from_toml(&text)
        .expect("it parses")
        .validate()
        .expect("and validates");
}
