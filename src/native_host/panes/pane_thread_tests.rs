use super::doubles::{RecordingRuntime, RecordingView, RuntimePhase};
use super::*;
use vellum_ultralight::surface::DirtyRect;

const PANE: PaneId = PaneId(1);

#[test]
fn only_the_hud_is_transparent_and_only_it_and_the_lobby_are_permanent() {
    // The copy and the texture format follow `transparent`, and the fault
    // path follows `permanent`: a console that reported itself permanent
    // would never flip its station to Backfill when its view died.
    assert!(PaneKind::Hud.transparent());
    assert!(!PaneKind::Console.transparent());
    assert!(!PaneKind::Lobby.transparent());

    assert!(PaneKind::Hud.permanent());
    assert!(PaneKind::Lobby.permanent());
    assert!(!PaneKind::Console.permanent());
}

#[test]
fn a_full_rect_covers_the_surface_and_an_inverted_one_covers_nothing() {
    let full = FrameRect::full(1920, 1080);
    assert!(!full.is_empty());
    assert_eq!(full.pixel_count(), 1920 * 1080);

    assert!(FrameRect::default().is_empty());
    assert_eq!(FrameRect::default().pixel_count(), 0);
    let inverted = FrameRect {
        left: 10,
        top: 10,
        right: 4,
        bottom: 4,
    };
    assert!(inverted.is_empty());
    assert_eq!(inverted.pixel_count(), 0, "an empty rect counts no pixels");
}

#[test]
fn a_rect_survives_the_round_trip_through_vellums_own() {
    // The render half (`super::upload`) speaks `DirtyRect` because it is
    // talking to the copy that produced one; the protocol speaks
    // `FrameRect`. The conversion is where they meet, so it is the one
    // place a transposed edge would be silent.
    let rect = FrameRect {
        left: 3,
        top: 7,
        right: 40,
        bottom: 90,
    };
    let dirty: DirtyRect = rect.into();
    assert_eq!(dirty.left, 3);
    assert_eq!(dirty.top, 7);
    assert_eq!(dirty.right, 40);
    assert_eq!(dirty.bottom, 90);
    assert_eq!(FrameRect::from(dirty), rect);
    assert_eq!(dirty.pixel_count(), rect.pixel_count());
    assert_eq!(
        FrameRect::from(DirtyRect::full(64, 32)),
        FrameRect::full(64, 32)
    );
}

#[test]
fn a_dropped_frame_buffer_goes_back_to_its_own_pane() {
    let (tx, rx) = std::sync::mpsc::channel();
    drop(PaneFrameBuffer::new(PANE, vec![7; 12], Some(tx)));
    let (pane, bytes) = rx.try_recv().expect("the allocation came back");
    assert_eq!(pane, PANE);
    assert_eq!(bytes.len(), 12);
}

#[test]
fn the_doubles_satisfy_the_seams_the_loop_is_written_against() {
    // A smoke test with a purpose: slice 3's loop is generic over these
    // traits, so a double that no longer satisfies one of them is a
    // compile error here rather than in the policy tests that matter.
    fn drive<R: PaneRuntime>(runtime: &mut R, id: PaneId, url: &str) -> R::View {
        runtime.update();
        let view = runtime
            .create(
                id,
                PaneKind::Console,
                &PaneSpecOwned {
                    width: 8,
                    height: 4,
                    device_scale: 1.0,
                },
                url,
            )
            .expect("the double creates");
        runtime.render();
        view
    }

    let mut runtime = RecordingRuntime::default();
    let mut view = drive(&mut runtime, PANE, "http://127.0.0.1/pane");
    assert_eq!(
        runtime.phases,
        vec![
            RuntimePhase::Update,
            RuntimePhase::Create(PANE),
            RuntimePhase::Render
        ]
    );
    assert_eq!(view.surface.loaded, vec!["http://127.0.0.1/pane"]);
    assert!(view.refresh_loaded());

    view.resize(16, 8);
    view.input(&PaneInput::MouseMove { x: 2, y: 3 });
    view.input(&PaneInput::MouseDown { x: 2, y: 3 });
    view.input(&PaneInput::Key(PaneKeyCode::Return));
    assert_eq!(view.resizes, vec![(16, 8)]);
    assert_eq!(
        view.inputs,
        vec![
            PaneInput::MouseMove { x: 2, y: 3 },
            PaneInput::MouseDown { x: 2, y: 3 },
            PaneInput::Key(PaneKeyCode::Return),
        ],
        "the order a pane's inputs arrive in is the order it sees them"
    );

    let mut dst = vec![0u8; 8 * 4 * 4];
    assert_eq!(view.copy_frame(&mut dst, false), Ok(None), "a still page");
    view.paint = Some(FrameRect::full(8, 4));
    assert_eq!(
        view.copy_frame(&mut dst, true),
        Ok(Some(FrameRect::full(8, 4)))
    );
    view.fail_copies = 1;
    assert!(view.copy_frame(&mut dst, false).is_err());
    assert!(view.copy_frame(&mut dst, false).is_ok(), "one failure only");
}

#[test]
fn a_runtime_that_cannot_create_says_so_rather_than_handing_back_a_view() {
    const OTHER: PaneId = PaneId(2);
    let mut runtime = RecordingRuntime {
        fail_create: std::collections::HashMap::from([(PANE, "no renderer".to_string())]),
        ..Default::default()
    };
    let result = runtime.create(
        PANE,
        PaneKind::Lobby,
        &PaneSpecOwned {
            width: 4,
            height: 4,
            device_scale: 2.0,
        },
        "http://127.0.0.1/lobby",
    );
    assert!(matches!(result, Err(PaneSurfaceError::Load(_))));

    // Failure is per-pane: a seat that was not told to fail still creates
    // fine, even though another seat's create call just failed.
    let other = runtime.create(
        OTHER,
        PaneKind::Console,
        &PaneSpecOwned {
            width: 4,
            height: 4,
            device_scale: 1.0,
        },
        "http://127.0.0.1/console",
    );
    assert!(other.is_ok());

    assert_eq!(
        runtime.phases,
        vec![RuntimePhase::Create(PANE), RuntimePhase::Create(OTHER)]
    );
}

#[test]
fn refresh_loaded_reports_the_rising_edge_exactly_once() {
    // Mirrors `UltralightPaneSurface::refresh_loaded`: `is_ready` (its
    // sticky `loaded`) only becomes true through `refresh_loaded`, and the
    // loop's "finished loading" log line fires on `refresh_loaded() &&
    // !was_loaded` — so the double must actually produce a false-then-true
    // transition, not just echo `is_ready` back at itself.
    let mut view = RecordingView {
        finishes_loading_after: 2,
        ..RecordingView::ready()
    };
    assert!(!view.is_ready(), "not loaded until refresh says so");

    // Two calls before the document reports finished loading.
    assert!(!view.refresh_loaded());
    assert!(!view.is_ready());
    assert!(!view.refresh_loaded());
    assert!(!view.is_ready());

    // Third call: the rising edge, observed the way the loop observes it.
    let was_loaded = view.is_ready();
    assert!(
        view.refresh_loaded() && !was_loaded,
        "the edge fires exactly once, on this call"
    );
    assert!(view.is_ready());

    // Every call after stays true — there is no second edge.
    for _ in 0..3 {
        let was_loaded = view.is_ready();
        assert!(view.refresh_loaded());
        assert!(was_loaded, "already loaded, so this is not an edge");
    }
}
