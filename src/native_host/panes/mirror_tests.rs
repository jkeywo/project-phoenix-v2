use super::*;

const CONSOLE: PaneId = PaneId(1);
const OTHER: PaneId = PaneId(2);
const LOBBY: PaneId = PaneId(900);
const HUD: PaneId = PaneId(901);

fn mirror() -> PaneMirror<()> {
    let mut mirror = PaneMirror::new();
    mirror.insert(CONSOLE, PaneKind::Console, true, ());
    mirror.insert(LOBBY, PaneKind::Lobby, true, ());
    mirror.insert(HUD, PaneKind::Hud, false, ());
    mirror
}

#[test]
fn a_frame_from_the_generation_before_a_resize_is_not_accepted() {
    // The whole reason an epoch exists: a resize mints a NEW texture, and a
    // frame still in flight against the old one is the right pixels for the
    // wrong surface. Uploading it would either be refused by the layout
    // check or, worse, land a partial rectangle in a texture that has never
    // held anything but its fill.
    let mut mirror = mirror();
    assert!(mirror.accepts_frame(CONSOLE, 0));

    assert_eq!(mirror.bump_epoch(CONSOLE), Some(1));
    assert!(!mirror.accepts_frame(CONSOLE, 0), "the frame is stale");
    assert!(mirror.accepts_frame(CONSOLE, 1));

    // And the bump is per pane: the surfaces that did not resize are
    // untouched.
    assert!(mirror.accepts_frame(LOBBY, 0));
    assert_eq!(mirror.bump_epoch(OTHER), None, "no such pane");
}

#[test]
fn a_closed_pane_accepts_nothing_that_arrives_after_it() {
    // A pane closes while the side that owns its view is mid-copy. The
    // frame is honest about the surface it came from and completely wrong
    // about what to do with it: nothing draws that texture now.
    let mut mirror = mirror();
    assert!(mirror.accepts_frame(CONSOLE, 0));
    let removed = mirror.remove(CONSOLE).expect("it was open");
    assert_eq!(removed.id, CONSOLE);
    assert!(!mirror.accepts_frame(CONSOLE, 0));
    assert_eq!(
        mirror.ids(),
        vec![LOBBY, HUD],
        "and the rest keep their order"
    );
    assert!(mirror.remove(CONSOLE).is_none(), "closed once");
}

#[test]
fn a_console_faults_at_the_threshold_and_a_permanent_surface_never_does() {
    let mut mirror = mirror();
    for n in 1..VIEW_CRASH_COPY_FAILURES {
        assert_eq!(
            mirror.record_copy_failure(CONSOLE, n),
            None,
            "a run shorter than the threshold is a transient, not a crash"
        );
    }
    assert_eq!(
        mirror.record_copy_failure(CONSOLE, VIEW_CRASH_COPY_FAILURES),
        Some(PaneFault::ViewCrashed)
    );
    assert_eq!(mirror.get(CONSOLE).map(|p| p.copy_failures), Some(30));

    // The lobby and the HUD hold no station, so there is no Backfill to
    // fall back to: a dead permanent view is a blank rectangle, not a fault
    // aimed at nothing.
    for surface in [LOBBY, HUD] {
        assert_eq!(
            mirror.record_copy_failure(surface, VIEW_CRASH_COPY_FAILURES * 10),
            None
        );
    }
    assert_eq!(
        mirror.record_copy_failure(OTHER, 1000),
        None,
        "no such pane"
    );
}

#[test]
fn one_success_ends_a_run_of_failures() {
    let mut mirror = mirror();
    assert_eq!(
        mirror.record_copy_failure(CONSOLE, VIEW_CRASH_COPY_FAILURES - 1),
        None
    );
    mirror.record_copy_ok(CONSOLE);
    assert_eq!(mirror.get(CONSOLE).map(|p| p.copy_failures), Some(0));
    // The count the producer sends is the producer's own run, so a fresh
    // run starts at one rather than resuming where the old one stopped.
    assert_eq!(mirror.record_copy_failure(CONSOLE, 1), None);
}

#[test]
fn a_dead_renderer_faults_the_consoles_and_merely_drops_the_permanent_surfaces() {
    let mut mirror = mirror();
    mirror.insert(OTHER, PaneKind::Console, true, ());
    let death = mirror.thread_death();
    assert_eq!(death.fault, vec![CONSOLE, OTHER]);
    assert_eq!(death.drop_permanent, vec![LOBBY, HUD]);
    assert_eq!(
        mirror.len(),
        4,
        "the partition is a question, not the teardown itself"
    );
}

#[test]
fn the_payload_is_the_owners_and_the_mirror_only_carries_it() {
    // Generic over the payload for one reason: everything above is checked
    // by the ordinary `cargo test`, with no Bevy and no SDK in scope.
    let mut mirror: PaneMirror<String> = PaneMirror::new();
    mirror.insert(CONSOLE, PaneKind::Console, true, "helm".to_string());
    assert_eq!(
        mirror.get(CONSOLE).map(|p| p.payload.as_str()),
        Some("helm")
    );
    mirror
        .get_mut(CONSOLE)
        .unwrap()
        .payload
        .push_str(" console");
    assert_eq!(
        mirror.remove(CONSOLE).map(|p| p.payload),
        Some("helm console".to_string())
    );
    assert!(mirror.is_empty());
}
