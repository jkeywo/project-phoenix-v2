//! What one iteration of [`PaneLoop`] does, and in what order (issue #1404,
//! slice 3).
//!
//! Every claim here used to be a claim about `drive_pane_host`, provable only by
//! a human on a Windows machine with an SDK and a GPU watching four
//! consoles. They are the claims that decide whether a console draws at all:
//! that a push reaches a page before the rasterise that would show it, that
//! a page pushed to is copied WHOLE (Ultralight does not flag every
//! repaint), that a run of failed copies is counted rather than smoothed
//! over, and that a closed pane stops being driven.

use super::doubles::{RecordingRuntime, RecordingSink};
use super::*;
use crate::core::messages::{DeliveryClass, ServerMessage};
use crate::lobby::handler::Target;
use crate::native_host::panes::identity::PaneIdentity;
use crate::native_host::transport::{NativeTransport, TransportDispatch};

const CONSOLE: PaneId = PaneId(1);
const LOBBY: PaneId = PaneId(900);
const HUD: PaneId = PaneId(901);
const WIDTH: u32 = 4;
const HEIGHT: u32 = 2;

fn create(id: PaneId, kind: PaneKind) -> PaneCommand {
    PaneCommand::Create {
        id,
        kind,
        spec: PaneSpecOwned {
            width: WIDTH,
            height: HEIGHT,
            device_scale: 1.0,
        },
        url: "http://127.0.0.1/pane".to_string(),
        epoch: 0,
        visible: true,
    }
}

/// A loop driving one pane of `kind`, whose page repaints its whole surface
/// whenever it is copied.
fn one_pane(id: PaneId, kind: PaneKind) -> PaneLoop<RecordingRuntime> {
    let mut driver = PaneLoop::new(RecordingRuntime::default());
    let mut out = Vec::new();
    assert_eq!(
        driver.apply(create(id, kind), &mut NoFrameSink, &mut out),
        LoopControl::Continue
    );
    assert!(matches!(
        out.as_slice(),
        [PaneEvent::Created { result: Ok(()), .. }]
    ));
    driver.view_mut(id).expect("it was created").paint = Some(FrameRect::full(WIDTH, HEIGHT));
    driver
}

fn bus_with_pane() -> (PaneBus, PaneId) {
    let bus = PaneBus::default();
    let id = bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
    (bus, id)
}

fn broadcast(bus: &PaneBus, msg: ServerMessage) {
    bus.transport().dispatch(TransportDispatch {
        target: &Target::All,
        msg: &msg,
        delivery: DeliveryClass::Reliable,
    });
}

fn stats(out: &[PaneEvent]) -> PaneThreadSample {
    match out.last() {
        Some(PaneEvent::Stats(sample)) => *sample,
        other => panic!("every iteration ends with its own cost, not {other:?}"),
    }
}

#[test]
fn disabled_attribution_attaches_no_trace_or_phase_clock_samples() {
    let mut driver = one_pane(HUD, PaneKind::Hud);
    let mut sink = PooledFrames::new(PANE_STAGING_BUFFERS);
    sink.configure(HUD, PaneKind::Hud, (WIDTH * HEIGHT * 4) as usize);
    let mut out = Vec::new();
    driver.iterate(&mut sink, &mut out);
    let PaneEvent::Frame(frame) = &sink.frames[0] else {
        panic!("a real frame")
    };
    assert!(frame.bytes.trace().is_none());
    assert!(
        !out.iter()
            .any(|event| matches!(event, PaneEvent::CopyObserved { .. })),
        "the ordinary unmeasured path sends no per-pane log events"
    );
    let sample = stats(&out);
    assert_eq!(
        [
            sample.update_ms,
            sample.pump_ms,
            sample.render_ms,
            sample.copy_ms,
            sample.publish_ms,
            sample.iteration_ms
        ],
        [0.0; 6]
    );
}

#[test]
fn measured_rectangle_events_follow_real_copy_retry_hidden_and_resize_decisions() {
    fn step(
        driver: &mut PaneLoop<RecordingRuntime>,
        sink: &mut RecordingSink,
    ) -> (SurfaceIdentity, CopyObservation, PaneThreadSample) {
        let mut out = Vec::new();
        driver.iterate(sink, &mut out);
        let observations: Vec<_> = out
            .iter()
            .filter_map(|event| match event {
                PaneEvent::CopyObserved {
                    surface,
                    observation,
                } => Some((*surface, *observation)),
                _ => None,
            })
            .collect();
        assert_eq!(
            observations.len(),
            1,
            "one decision per measured pane iteration"
        );
        (observations[0].0, observations[0].1, stats(&out))
    }
    let observer = SurfaceObserver::new(Instant::now(), 128);
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    driver.set_measure(true);
    driver.set_observer(Some(observer.clone()));
    let mut sink = RecordingSink::new();
    let (old_identity, initial, _) = step(&mut driver, &mut sink);
    assert!(initial.forced && initial.reasons.initial);
    assert_eq!(initial.dirty_rect, None);
    assert_eq!(initial.copied_rect, Some(FrameRect::full(WIDTH, HEIGHT)));

    let partial = FrameRect {
        left: 1,
        top: 0,
        right: 3,
        bottom: 1,
    };
    driver.view_mut(CONSOLE).unwrap().paint = Some(partial);
    let (_, copied, sample) = step(&mut driver, &mut sink);
    assert_eq!(
        (copied.dirty_rect, copied.copied_rect),
        (Some(partial), Some(partial))
    );
    assert_eq!(sample.copied, 1);
    assert!(!copied.forced);
    driver.view_mut(CONSOLE).unwrap().paint = None;
    let (_, clean, sample) = step(&mut driver, &mut sink);
    assert_eq!(clean.dirty_rect, Some(FrameRect::default()));
    assert_eq!(sample.copied, 0);

    // A failed unforced copy does not create a new full-copy obligation.
    // A real reveal does; exercise its preservation through both failures.
    for visible in [false, true] {
        driver.apply(
            PaneCommand::SetVisible {
                id: CONSOLE,
                visible,
            },
            &mut sink,
            &mut Vec::new(),
        );
    }
    driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
    driver.view_mut(CONSOLE).unwrap().fail_copies = 1;
    let (_, failed, _) = step(&mut driver, &mut sink);
    assert!(failed.forced && failed.reasons.reveal);
    assert_eq!(
        (failed.outcome, failed.dirty_rect, failed.copied_rect),
        ("failed", None, None)
    );
    sink.starve = true;
    let (_, starved, _) = step(&mut driver, &mut sink);
    assert_eq!(
        (starved.outcome, starved.dirty_rect, starved.copied_rect),
        ("buffer_starved", None, None)
    );
    assert!(starved.forced && starved.reasons.copy_retry);
    sink.starve = false;
    let (_, retried, _) = step(&mut driver, &mut sink);
    assert!(retried.forced && retried.reasons.copy_retry && retried.reasons.buffer_retry);
    assert_eq!(retried.dirty_rect, None);

    driver.apply(
        PaneCommand::SetVisible {
            id: CONSOLE,
            visible: false,
        },
        &mut sink,
        &mut Vec::new(),
    );
    let before = sink.published.len();
    let (hidden_identity, hidden, sample) = step(&mut driver, &mut sink);
    assert!(!hidden_identity.visible);
    assert_eq!(
        driver.view_mut(CONSOLE).unwrap().visibility.last(),
        Some(&false)
    );
    assert_eq!(
        (hidden.outcome, hidden.dirty_rect, hidden.copied_rect),
        ("hidden", None, None)
    );
    assert_eq!(sample.copied, 0);
    assert_eq!(sink.published.len(), before);
    driver.apply(
        PaneCommand::Resize {
            id: CONSOLE,
            width: 8,
            height: 6,
            epoch: 3,
        },
        &mut sink,
        &mut Vec::new(),
    );
    driver.apply(
        PaneCommand::SetVisible {
            id: CONSOLE,
            visible: true,
        },
        &mut sink,
        &mut Vec::new(),
    );
    driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(8, 6));
    let (resized, reveal, _) = step(&mut driver, &mut sink);
    assert_eq!(
        driver.view_mut(CONSOLE).unwrap().visibility.last(),
        Some(&true)
    );
    assert_eq!(
        (
            resized.epoch,
            resized.width,
            resized.height,
            resized.visible
        ),
        (3, 8, 6, true)
    );
    assert!(reveal.forced && reveal.reasons.resize && reveal.reasons.reveal);
    assert_eq!(
        (reveal.dirty_rect, reveal.copied_rect),
        (None, Some(FrameRect::full(8, 6)))
    );
    assert_eq!(driver.view_mut(CONSOLE).unwrap().resizes, [(8, 6)]);
    // Already queued observations keep the old raster/epoch after resize.
    assert_eq!(
        (old_identity.epoch, old_identity.width, old_identity.height),
        (0, WIDTH, HEIGHT)
    );
    let copies: Vec<_> = observer
        .events()
        .into_iter()
        .filter(|event| matches!(event.operation, Operation::Copy { .. }))
        .collect();
    assert_eq!(
        copies.len(),
        7,
        "a hidden view never calls or records a copy"
    );
    assert!(
        matches!(copies[1].operation, Operation::Copy { dirty_rect: Some(rect), copied_rect: Some(copied), dirty_pixels: Some(2), copied_pixels: 2, .. } if rect == partial && copied == partial)
    );
}

#[test]
fn attribution_distinguishes_quiet_hud_revisions_hidden_failure_and_reveal() {
    let observer = SurfaceObserver::new(Instant::now(), 128);
    let mut driver = one_pane(HUD, PaneKind::Hud);
    driver.set_observer(Some(observer.clone()));
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.apply(
        PaneCommand::SetHudScript(Some("first".into())),
        &mut sink,
        &mut out,
    );
    driver.iterate(&mut sink, &mut out);
    driver.iterate(&mut sink, &mut out);
    driver.apply(
        PaneCommand::SetVisible {
            id: HUD,
            visible: false,
        },
        &mut sink,
        &mut out,
    );
    driver.iterate(&mut sink, &mut out);
    driver.apply(
        PaneCommand::SetHudScript(Some("second".into())),
        &mut sink,
        &mut out,
    );
    driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
    driver.iterate(&mut sink, &mut out);
    driver.apply(
        PaneCommand::SetVisible {
            id: HUD,
            visible: true,
        },
        &mut sink,
        &mut out,
    );
    driver.iterate(&mut sink, &mut out);

    let events = observer.events();
    let applications: Vec<_> = events
        .iter()
        .filter_map(|event| match event.operation {
            Operation::Push {
                revision: Some(revision),
                applied,
                failed,
                ..
            } => Some((revision, applied, failed, event.surface.unwrap().visible)),
            _ => None,
        })
        .collect();
    assert_eq!(
        applications,
        vec![
            (1, 1, 0, true),
            (1, 0, 0, true),
            (1, 0, 0, false),
            (2, 0, 1, false),
            (2, 1, 0, true)
        ]
    );
    let produced: Vec<_> = events
        .iter()
        .filter_map(|event| match event.operation {
            Operation::Produced {
                hud_revision,
                reasons,
                ..
            } => Some((hud_revision, reasons)),
            _ => None,
        })
        .collect();
    assert_eq!(
        produced.len(),
        3,
        "hidden views still pump but publish no pixels"
    );
    assert!(produced[0].1.initial && produced[0].1.hud_push);
    assert_eq!(produced[1].0, Some(1));
    assert!(!produced[1].1.hud_push, "unchanged HUD is not reapplied");
    assert!(produced[2].1.reveal && produced[2].1.hud_push);
    assert_eq!(produced[2].0, Some(2));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.operation, Operation::Iteration { .. }))
            .count(),
        5
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.operation, Operation::HudSlot { .. }))
            .count(),
        2
    );
}

#[test]
fn attribution_distinguishes_failed_copy_and_starvation_from_produced_frames() {
    let observer = SurfaceObserver::new(Instant::now(), 64);
    let mut driver = one_pane(HUD, PaneKind::Hud);
    driver.set_observer(Some(observer.clone()));
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.view_mut(HUD).unwrap().fail_copies = 1;
    driver.iterate(&mut sink, &mut out);
    sink.starve = true;
    driver.iterate(&mut sink, &mut out);
    sink.starve = false;
    driver.iterate(&mut sink, &mut out);
    let events = observer.events();
    let outcomes: Vec<_> = events
        .iter()
        .filter_map(|event| match event.operation {
            Operation::Copy { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(outcomes, ["failed", "buffer_starved", "copied"]);
    let produced: Vec<_> = events
        .iter()
        .filter_map(|event| match event.operation {
            Operation::Produced { reasons, .. } => Some(reasons),
            _ => None,
        })
        .collect();
    assert_eq!(produced.len(), 1);
    assert!(produced[0].initial && produced[0].copy_retry && produced[0].buffer_retry);
}

#[test]
fn observed_pool_preserves_old_frame_identity_across_resize_and_recycles_once() {
    let observer = SurfaceObserver::new(Instant::now(), 128);
    let mut driver = one_pane(HUD, PaneKind::Hud);
    driver.set_observer(Some(observer.clone()));
    let mut sink = PooledFrames::new(PANE_STAGING_BUFFERS);
    sink.configure(HUD, PaneKind::Hud, (WIDTH * HEIGHT * 4) as usize);
    let mut out = Vec::new();
    for _ in 0..PANE_STAGING_BUFFERS + 1 {
        driver.iterate(&mut sink, &mut out);
    }
    assert_eq!(sink.frames.len(), PANE_STAGING_BUFFERS);
    assert_eq!(sink.starved, 1);
    let old_frames = std::mem::take(&mut sink.frames);
    driver.apply(
        PaneCommand::Resize {
            id: HUD,
            width: WIDTH * 2,
            height: HEIGHT,
            epoch: 7,
        },
        &mut sink,
        &mut out,
    );
    sink.configure(HUD, PaneKind::Hud, (WIDTH * 2 * HEIGHT * 4) as usize);
    driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH * 2, HEIGHT));
    driver.iterate(&mut sink, &mut out);
    drop(old_frames);
    sink.frames.clear();
    let events = observer.events();
    let terminal: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.operation, Operation::Discarded { .. }))
        .collect();
    assert_eq!(terminal.len(), PANE_STAGING_BUFFERS + 1);
    assert!(terminal[..PANE_STAGING_BUFFERS].iter().all(|event| {
        let identity = event.surface.unwrap();
        identity.epoch == 0 && identity.width == WIDTH && identity.device_scale == 1.0
    }));
    assert_eq!(terminal.last().unwrap().surface.unwrap().epoch, 7);
    // The old generation returns to its retired channel, never the new pool.
    assert!(sink.stage(HUD, (WIDTH * 2 * HEIGHT * 4) as usize).is_some());
    sink.return_staged();
    assert_eq!(sink.pools[&HUD].free.len(), PANE_STAGING_BUFFERS);
    assert_eq!(
        observer.events().len(),
        events.len(),
        "recycling does not duplicate disposal"
    );
}

#[test]
fn an_iteration_pushes_pads_then_updates_then_pumps_then_renders_then_copies() {
    // The order is the whole design. A push made after the rasterise shows a
    // frame late, every time; a copy made before it copies the frame before
    // the one the pump just caused.
    //
    // The gamepad snapshot is the one push that goes in AHEAD of `update`:
    // a console polls the pads from its own `requestAnimationFrame`, which
    // `update` is what services, so a snapshot pushed after it would be read
    // an iteration late — a whole iteration of stick latency.
    let (runtime, trace) = RecordingRuntime::traced();
    let mut driver = PaneLoop::new(runtime);
    let mut out = Vec::new();
    driver.apply(create(HUD, PaneKind::Hud), &mut NoFrameSink, &mut out);
    driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
    driver.apply(
        create(CONSOLE, PaneKind::Console),
        &mut NoFrameSink,
        &mut out,
    );
    driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
    driver.apply(
        PaneCommand::SetHudScript(Some("window.__updateHud({})".to_string())),
        &mut NoFrameSink,
        &mut out,
    );
    driver.apply(
        PaneCommand::SetGamepadScript(Some("window.__phoenixSetGamepads([])".to_string())),
        &mut NoFrameSink,
        &mut out,
    );

    let mut sink = RecordingSink::new();
    // One iteration to settle the load edges: a push, of either kind, goes
    // only into a document that has finished loading.
    driver.iterate(&mut sink, &mut out);
    driver.apply(
        PaneCommand::SetHudScript(Some("window.__updateHud({heading:1})".to_string())),
        &mut NoFrameSink,
        &mut out,
    );
    sink.published.clear();
    out.clear();
    trace.borrow_mut().clear();
    driver.iterate(&mut sink, &mut out);

    assert_eq!(
        *trace.borrow(),
        vec![
            format!("push:{CONSOLE}"),
            "update".to_string(),
            format!("push:{HUD}"),
            "render".to_string(),
            format!("copy:{HUD}"),
            format!("copy:{CONSOLE}"),
        ]
    );
    let pad_push = trace
        .borrow()
        .iter()
        .position(|t| t == &format!("push:{CONSOLE}"))
        .expect("the pad snapshot is pushed");
    let update = trace
        .borrow()
        .iter()
        .position(|t| t == "update")
        .expect("the library is updated");
    assert!(
        pad_push < update,
        "the pad snapshot must reach the page before the update that lets it read them"
    );
    // The first iteration cleared both panes' opening `needs_full`, so what
    // forces a copy here is only this iteration's pushing. The HUD's push
    // does; the console's pad snapshot does NOT — a snapshot the page polls
    // on its own schedule is not by itself a repaint, and the inline
    // `push_gamepads_to_panes` never counted one either.
    let hud = sink
        .published
        .iter()
        .find(|f| f.id == HUD)
        .expect("the HUD publishes");
    assert!(hud.full, "a HUD push forces a whole copy");
    let console = sink
        .published
        .iter()
        .find(|f| f.id == CONSOLE)
        .expect("the console publishes");
    assert!(!console.full, "a gamepad push does not force a copy");
}

#[test]
fn a_document_that_finishes_loading_says_so_exactly_once() {
    let mut driver = PaneLoop::new(RecordingRuntime::default());
    let mut out = Vec::new();
    driver.apply(
        create(CONSOLE, PaneKind::Console),
        &mut NoFrameSink,
        &mut out,
    );
    // Two iterations of "still loading" before the edge.
    driver.view_mut(CONSOLE).unwrap().finishes_loading_after = 2;
    let mut sink = RecordingSink::new();

    let mut edges = 0;
    for _ in 0..5 {
        out.clear();
        driver.iterate(&mut sink, &mut out);
        edges += out
            .iter()
            .filter(|e| matches!(e, PaneEvent::Loaded(id) if *id == CONSOLE))
            .count();
    }
    assert_eq!(edges, 1, "the rising edge, not the state");
}

#[test]
fn a_page_that_was_pushed_to_is_copied_whole_and_a_quiet_one_is_not() {
    // Ultralight's dirty-bounds tracking does not flag every repaint — a
    // plain attribute write is real DOM state that changed and is not in
    // them — so a push is trusted on its own. Without this a console that
    // updates one readout shows the update only when something else forces
    // a whole copy.
    let mut driver = one_pane(HUD, PaneKind::Hud);
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.apply(
        PaneCommand::SetHudScript(Some("window.__updateHud({})".to_string())),
        &mut NoFrameSink,
        &mut out,
    );

    driver.iterate(&mut sink, &mut out);
    driver.iterate(&mut sink, &mut out);
    // The slot is dropped: nothing is pushed, and nothing is forced.
    driver.apply(PaneCommand::SetHudScript(None), &mut NoFrameSink, &mut out);
    out.clear();
    driver.iterate(&mut sink, &mut out);

    let frames = sink.frames_for(HUD);
    assert_eq!(
        frames.iter().map(|f| f.full).collect::<Vec<_>>(),
        vec![true, false, false],
        "the first update is whole; unchanged and cleared HUD slots do not force copying"
    );
    assert!(
        frames.iter().all(|f| f.first_byte == 0xAB),
        "every published buffer carries what the copy wrote"
    );
    assert_eq!(stats(&out).copied, 1);
    assert_eq!(stats(&out).forced, 0);
}

#[test]
fn quiet_hud_keeps_animation_dirty_copies_and_retries_without_reapplying() {
    let observer = SurfaceObserver::new(Instant::now(), 256);
    let mut driver = one_pane(HUD, PaneKind::Hud);
    driver.set_observer(Some(observer.clone()));
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.apply(
        PaneCommand::SetHudScript(Some("first".into())),
        &mut sink,
        &mut out,
    );
    driver.iterate(&mut sink, &mut out);
    driver.view_mut(HUD).unwrap().paint = None;
    for _ in 0..3 {
        driver.iterate(&mut sink, &mut out);
    }
    assert_eq!(driver.view_mut(HUD).unwrap().surface.pushed, ["first"]);
    assert_eq!(
        sink.published.len(),
        1,
        "a static HUD produces no more frames"
    );
    assert_eq!(driver.view_mut(HUD).unwrap().forced_copies, 1);

    // A CSS animation can repaint without a new HUD revision. The ordinary
    // render/copy pass still sees that rectangle and does not force it full.
    let animated = FrameRect {
        left: 1,
        top: 0,
        right: 3,
        bottom: 1,
    };
    driver.view_mut(HUD).unwrap().paint = Some(animated);
    driver.iterate(&mut sink, &mut out);
    assert_eq!(sink.published[1].rect, animated);
    assert!(!sink.published[1].full);

    driver.apply(
        PaneCommand::SetHudScript(Some("second".into())),
        &mut sink,
        &mut out,
    );
    driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
    driver.view_mut(HUD).unwrap().paint = None;
    driver.iterate(&mut sink, &mut out);
    assert_eq!(driver.pane_mut(HUD).unwrap().applied_hud_revision, Some(1));

    // A successful retry owes a whole copy even if no staging buffer is
    // free, and then the copy itself fails. No further HUD push is needed
    // for that same obligation to reach the eventual successful frame.
    driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
    sink.starve = true;
    driver.iterate(&mut sink, &mut out);
    assert_eq!(driver.pane_mut(HUD).unwrap().applied_hud_revision, Some(2));
    sink.starve = false;
    driver.view_mut(HUD).unwrap().fail_copies = 1;
    driver.iterate(&mut sink, &mut out);
    driver.iterate(&mut sink, &mut out);
    assert_eq!(
        driver.view_mut(HUD).unwrap().surface.pushed,
        ["first", "second"]
    );
    assert_eq!(sink.published.len(), 3);
    assert!(sink.published[2].full);
    let events = observer.events();
    let (revision, reasons) = events
        .iter()
        .rev()
        .find_map(|event| match event.operation {
            Operation::Produced {
                hud_revision,
                reasons,
                ..
            } => Some((hud_revision, reasons)),
            _ => None,
        })
        .expect("the retry produced a frame");
    assert_eq!(revision, Some(2));
    assert!(reasons.hud_push && reasons.buffer_retry && reasons.copy_retry);
}

#[test]
fn audio_hud_delivers_current_beam_and_discards_missed_transients() {
    use crate::native_host::audio::{player::RoomInput, visual::NativeAudioVisual};

    let shared = NativeAudioVisual::default();
    let mut input = RoomInput {
        lifecycle: crate::console_bridge::AudioLifecycleState {
            generation: 1,
            running: true,
            suspended: false,
        },
        phaser: true,
        ..Default::default()
    };
    shared.update(&input);
    shared.blaster([1.0, 0.0, 0.0]);
    let mut driver = one_pane(HUD, PaneKind::Hud);
    driver.audio_visuals = Some(shared.clone());
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    let take = |driver: &mut PaneLoop<RecordingRuntime>| {
        std::mem::take(&mut driver.view_mut(HUD).unwrap().surface.pushed)
    };
    driver.iterate(&mut sink, &mut out);
    let initial = take(&mut driver);
    assert_eq!(initial.len(), 2);
    assert!(initial[0].contains("lifecycle"));
    assert!(initial[1].contains("beam") && initial[1].contains("true"));
    assert!(!initial.iter().any(|script| script.contains("blaster")));

    shared.blaster([2.0, 0.0, 0.0]);
    driver.iterate(&mut sink, &mut out);
    assert!(matches!(take(&mut driver).as_slice(), [shot] if shot.contains("blaster")));
    driver.iterate(&mut sink, &mut out);
    assert!(take(&mut driver).is_empty());

    shared.blaster([3.0, 0.0, 0.0]);
    driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
    driver.iterate(&mut sink, &mut out);
    driver.iterate(&mut sink, &mut out);
    assert!(
        take(&mut driver).is_empty(),
        "failed transients never retry"
    );

    for visible in [false, true] {
        driver.apply(
            PaneCommand::SetVisible { id: HUD, visible },
            &mut sink,
            &mut out,
        );
        shared.blaster([4.0, 0.0, 0.0]);
        driver.iterate(&mut sink, &mut out);
        assert!(!take(&mut driver)
            .iter()
            .any(|script| script.contains("blaster")));
    }

    input.phaser = false;
    shared.update(&input);
    driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
    driver.iterate(&mut sink, &mut out);
    assert!(take(&mut driver).is_empty());
    driver.iterate(&mut sink, &mut out);
    let retried = take(&mut driver);
    assert_eq!(retried.len(), 2, "current lifecycle and beam state retry");
    assert!(retried[1].contains("beam") && retried[1].contains("false"));

    shared.blaster([5.0, 0.0, 0.0]);
    input.lifecycle.generation += 1;
    input.lifecycle.suspended = true;
    shared.update(&input);
    driver.iterate(&mut sink, &mut out);
    let held = take(&mut driver);
    assert_eq!(held.len(), 2);
    assert!(held[0].contains("lifecycle") && held[0].contains("false"));
    assert!(!held.iter().any(|script| script.contains("blaster")));
}

#[test]
fn latest_hud_survives_loading_recreation_reveal_and_resize() {
    let mut driver = one_pane(HUD, PaneKind::Hud);
    driver.view_mut(HUD).unwrap().finishes_loading_after = 1;
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.apply(
        PaneCommand::SetHudScript(Some("before load".into())),
        &mut sink,
        &mut out,
    );
    driver.iterate(&mut sink, &mut out);
    assert!(driver.view_mut(HUD).unwrap().surface.pushed.is_empty());
    driver.apply(
        PaneCommand::SetHudScript(Some("latest".into())),
        &mut sink,
        &mut out,
    );
    driver.iterate(&mut sink, &mut out);
    assert_eq!(driver.view_mut(HUD).unwrap().surface.pushed, ["latest"]);

    driver.apply(PaneCommand::Close(HUD), &mut sink, &mut out);
    driver.apply(create(HUD, PaneKind::Hud), &mut sink, &mut out);
    driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
    driver.iterate(&mut sink, &mut out);
    assert_eq!(driver.view_mut(HUD).unwrap().surface.pushed, ["latest"]);
    for visible in [false, true] {
        driver.apply(
            PaneCommand::SetVisible { id: HUD, visible },
            &mut sink,
            &mut out,
        );
        driver.iterate(&mut sink, &mut out);
    }
    assert_eq!(
        driver.view_mut(HUD).unwrap().surface.pushed,
        ["latest", "latest"]
    );
    driver.apply(
        PaneCommand::Resize {
            id: HUD,
            width: WIDTH * 2,
            height: HEIGHT,
            epoch: 7,
        },
        &mut sink,
        &mut out,
    );
    driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH * 2, HEIGHT));
    driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
    driver.iterate(&mut sink, &mut out);
    assert_eq!(driver.pane_mut(HUD).unwrap().applied_hud_revision, Some(2));
    assert!(driver.pane_mut(HUD).unwrap().hud_apply_owed);
    driver.iterate(&mut sink, &mut out);
    assert!(!driver.pane_mut(HUD).unwrap().hud_apply_owed);
    assert_eq!(
        driver.view_mut(HUD).unwrap().surface.pushed,
        ["latest", "latest", "latest"]
    );
    let frame = sink.published.last().unwrap();
    assert_eq!(frame.epoch, 7);
    assert_eq!(frame.rect, FrameRect::full(WIDTH * 2, HEIGHT));
    assert!(
        frame.full,
        "the retried resize application is painted whole"
    );
}

#[test]
fn failed_copies_count_up_in_a_run_and_one_success_ends_it() {
    // A single failure is a transient — a repaint mid-flight, a buffer not
    // ready. A view that has genuinely died fails every frame, so the run is
    // the signal, and the count is what the mirror's threshold reads.
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    driver.view_mut(CONSOLE).unwrap().fail_copies = 3;
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();

    let mut runs = Vec::new();
    for _ in 0..4 {
        out.clear();
        driver.iterate(&mut sink, &mut out);
        runs.extend(out.iter().filter_map(|e| match e {
            PaneEvent::CopyFailed { consecutive, .. } => Some(*consecutive),
            _ => None,
        }));
    }
    assert_eq!(runs, vec![1, 2, 3], "consecutive, then a success");
    assert_eq!(sink.published.len(), 1, "the fourth iteration published");
    assert!(
        sink.frames_for(CONSOLE)[0].full,
        "the force the failed copies could not honour was carried, not dropped"
    );

    // And the next failure starts a fresh run rather than resuming the old.
    driver.view_mut(CONSOLE).unwrap().fail_copies = 1;
    out.clear();
    driver.iterate(&mut sink, &mut out);
    assert!(out
        .iter()
        .any(|e| matches!(e, PaneEvent::CopyFailed { consecutive: 1, .. })));
}

#[test]
fn a_closed_pane_is_dropped_and_stops_being_driven() {
    // A view left behind is not inert: it would still be pumped, and its
    // page's records would still be drained into a registry that refuses
    // them, once per iteration for the rest of the run.
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.iterate(&mut sink, &mut out);
    assert_eq!(sink.published.len(), 1);

    driver.apply(PaneCommand::Close(CONSOLE), &mut sink, &mut out);
    assert!(!driver.contains(CONSOLE));
    assert_eq!(sink.dropped, vec![CONSOLE], "and its pool went with it");

    out.clear();
    driver.iterate(&mut sink, &mut out);
    assert_eq!(sink.published.len(), 1, "nothing more was copied");
    assert_eq!(stats(&out).panes, 0);
}

#[test]
fn a_view_that_cannot_be_created_is_reported_and_leaves_no_pane_behind() {
    // A per-seat failure fails THIS pane only — its station simply stays on
    // Backfill — rather than the whole host.
    let mut driver = PaneLoop::new(RecordingRuntime {
        fail_create: std::collections::HashMap::from([(CONSOLE, "no renderer".to_string())]),
        ..Default::default()
    });
    let mut out = Vec::new();
    driver.apply(
        create(CONSOLE, PaneKind::Console),
        &mut NoFrameSink,
        &mut out,
    );
    match out.as_slice() {
        [PaneEvent::Created {
            id,
            result: Err(reason),
        }] => {
            assert_eq!(*id, CONSOLE);
            assert_eq!(reason, "load failed: no renderer");
        }
        other => panic!("expected one refusal, got {other:?}"),
    }
    assert!(driver.is_empty());

    // The other seat still builds, and the loop drives it.
    out.clear();
    driver.apply(create(LOBBY, PaneKind::Lobby), &mut NoFrameSink, &mut out);
    assert!(matches!(
        out.as_slice(),
        [PaneEvent::Created { result: Ok(()), .. }]
    ));
    assert_eq!(driver.len(), 1);
}

#[test]
fn a_panes_inputs_reach_its_view_in_the_order_they_were_sent() {
    // Ultralight decides what is under the pointer from the MOVE, and drops
    // input into an unfocused view — so a `MouseDown` ahead of its
    // `MouseMove` lands wherever the pointer last was, and keys ahead of
    // their `Focus` type into nothing.
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    let mut out = Vec::new();
    let sent = [
        PaneInput::Focus,
        PaneInput::MouseMove { x: 3, y: 4 },
        PaneInput::MouseDown { x: 3, y: 4 },
        PaneInput::KeyChar("a".to_string()),
        PaneInput::Key(PaneKeyCode::Return),
        PaneInput::MouseUp { x: 3, y: 4 },
        PaneInput::Unfocus,
    ];
    for input in sent.iter().cloned() {
        driver.apply(
            PaneCommand::Input { id: CONSOLE, input },
            &mut NoFrameSink,
            &mut out,
        );
    }
    assert_eq!(driver.view_mut(CONSOLE).unwrap().inputs, sent.to_vec());
    assert!(out.is_empty(), "input is fire-and-forget");

    // An input for a pane that has gone is dropped, not a panic.
    driver.apply(PaneCommand::Close(CONSOLE), &mut NoFrameSink, &mut out);
    driver.apply(
        PaneCommand::Input {
            id: CONSOLE,
            input: PaneInput::Focus,
        },
        &mut NoFrameSink,
        &mut out,
    );
}

#[test]
fn commands_have_landed_before_the_iteration_that_follows_them() {
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.iterate(&mut sink, &mut out);

    driver.apply(
        PaneCommand::Input {
            id: CONSOLE,
            input: PaneInput::MouseMove { x: 1, y: 1 },
        },
        &mut NoFrameSink,
        &mut out,
    );
    out.clear();
    driver.iterate(&mut sink, &mut out);
    assert_eq!(
        driver.view_mut(CONSOLE).unwrap().inputs.len(),
        1,
        "delivered once, before this iteration rather than during it"
    );
    assert_eq!(stats(&out).copied, 1);
}

#[test]
fn a_frame_carries_the_generation_of_the_resize_that_produced_it_and_is_whole() {
    // A resize mints a new texture, which holds only its fill until
    // something covers it — so the first frame after one must be the whole
    // surface, and must be recognisable as belonging to the new generation.
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.iterate(&mut sink, &mut out);
    assert_eq!(sink.frames_for(CONSOLE)[0].epoch, 0);

    driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH * 2, HEIGHT));
    driver.apply(
        PaneCommand::Resize {
            id: CONSOLE,
            width: WIDTH * 2,
            height: HEIGHT,
            epoch: 7,
        },
        &mut NoFrameSink,
        &mut out,
    );
    assert_eq!(
        driver.view_mut(CONSOLE).unwrap().resizes,
        vec![(WIDTH * 2, HEIGHT)]
    );

    out.clear();
    driver.iterate(&mut sink, &mut out);
    let frame = sink.frames_for(CONSOLE)[1];
    assert_eq!(frame.epoch, 7, "the generation the resize numbered");
    assert!(frame.full, "and the whole of the new texture");
    assert_eq!(frame.rect, FrameRect::full(WIDTH * 2, HEIGHT));
}

#[test]
fn a_hidden_surface_is_pumped_but_publishes_nothing_and_a_reveal_is_whole() {
    // The point of hiding rather than tearing down: the page stays live and
    // keeps taking state, so a reveal is a `display` flip rather than a page
    // load. What it stops paying is the copy.
    let mut driver = one_pane(HUD, PaneKind::Hud);
    let mut sink = RecordingSink::new();
    let mut out = Vec::new();
    driver.apply(
        PaneCommand::SetHudScript(Some("window.__updateHud({})".to_string())),
        &mut NoFrameSink,
        &mut out,
    );
    driver.apply(
        PaneCommand::SetVisible {
            id: HUD,
            visible: false,
        },
        &mut NoFrameSink,
        &mut out,
    );

    driver.iterate(&mut sink, &mut out);
    driver.apply(
        PaneCommand::SetHudScript(Some("window.__updateHud({heading:1})".to_string())),
        &mut NoFrameSink,
        &mut out,
    );
    driver.iterate(&mut sink, &mut out);
    assert_eq!(
        driver.view_mut(HUD).unwrap().surface.pushed.len(),
        2,
        "the page kept taking state while it was hidden"
    );
    assert!(sink.published.is_empty(), "and cost no copy at all");

    driver.apply(
        PaneCommand::SetVisible {
            id: HUD,
            visible: true,
        },
        &mut NoFrameSink,
        &mut out,
    );
    out.clear();
    driver.iterate(&mut sink, &mut out);
    assert_eq!(sink.published.len(), 1);
    assert!(
        sink.frames_for(HUD)[0].full,
        "the texture is however stale the hidden iterations left it"
    );
    assert_eq!(stats(&out).forced, 1);
}

#[test]
fn a_pane_with_no_buffer_free_is_skipped_and_keeps_what_it_owed() {
    // The render world runs a frame behind, so a pool can genuinely be
    // empty. Allocating a whole surface on the frame path instead would be
    // the wrong answer; Ultralight keeps unioning its dirty bounds until the
    // next successful copy, so the pixels are deferred rather than lost —
    // but only if the force is carried with them.
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    let mut sink = RecordingSink::new();
    sink.starve = true;
    let mut out = Vec::new();

    driver.iterate(&mut sink, &mut out);
    assert!(sink.published.is_empty());
    assert_eq!(sink.starved, 1);
    assert_eq!(
        driver.view_mut(CONSOLE).unwrap().forced_copies,
        0,
        "the copy was skipped entirely, not made into nothing"
    );

    sink.starve = false;
    out.clear();
    driver.iterate(&mut sink, &mut out);
    assert!(
        sink.frames_for(CONSOLE)[0].full,
        "the whole frame it owed survived the starved iteration"
    );
}

#[test]
fn the_lobby_drains_its_own_bridge_and_a_console_drains_the_bus() {
    // The one thing the two surfaces must not share. Nothing the lobby says
    // is a participant's `ClientMessage` and nothing it hears is a
    // projection, so a lobby that drained the bus — or a console that
    // drained the lobby's queue — would be the whole separation undone.
    let (bus, console) = bus_with_pane();
    super::super::transport::identify_test_pane(&bus, console);
    let bridge = HostLobbyBridge::new();
    bridge.push_lobby_state("{}");
    broadcast(&bus, ServerMessage::GameStarted);

    let mut driver = PaneLoop::new(RecordingRuntime::default());
    let mut out = Vec::new();
    driver.apply(
        create(console, PaneKind::Console),
        &mut NoFrameSink,
        &mut out,
    );
    driver.apply(create(LOBBY, PaneKind::Lobby), &mut NoFrameSink, &mut out);
    driver.set_bus(Some(bus.clone()));
    driver.set_lobby(Some(bridge.clone()));
    // A page saying something it is not entitled to say is refused and
    // reported rather than swallowed.
    driver
        .view_mut(console)
        .unwrap()
        .surface
        .queue_record(r#"{"type":"Identify","data":{"token":"__local__","name":"impostor"}}"#);

    let mut sink = RecordingSink::new();
    out.clear();
    driver.iterate(&mut sink, &mut out);

    let console_pushed = driver.view_mut(console).unwrap().surface.pushed.clone();
    assert_eq!(console_pushed.len(), 1);
    assert!(console_pushed[0].contains("GameStarted"));
    let lobby_pushed = driver.view_mut(LOBBY).unwrap().surface.pushed.clone();
    assert_eq!(lobby_pushed.len(), 1);
    assert!(
        !lobby_pushed[0].contains("GameStarted"),
        "the bus's traffic never reaches the lobby surface"
    );
    assert!(!bridge.has_pending(), "and the bridge was drained");

    assert!(out
        .iter()
        .any(|e| matches!(e, PaneEvent::Refused { id, .. } if *id == console)));
}

/// Drain a queue of commands the way `drive_pane_host` does: FIFO, through
/// [`PaneLoop::apply`], before the iteration.
fn drain(
    driver: &mut PaneLoop<RecordingRuntime>,
    queue: &mut std::collections::VecDeque<PaneCommand>,
    out: &mut Vec<PaneEvent>,
) {
    while let Some(cmd) = queue.pop_front() {
        assert_eq!(
            driver.apply(cmd, &mut NoFrameSink, out),
            LoopControl::Continue,
            "nothing a Bevy system queues stops the loop"
        );
    }
}

#[test]
fn a_drained_queue_delivers_its_inputs_in_order_and_before_the_iteration() {
    // Slice 4's whole claim: a queue between the system and the view changes
    // WHERE the call is made, not when it lands nor in what order. The
    // systems that fill it are chained ahead of `drive_pane_host`, so every
    // command raised in a frame is applied in that frame, before the
    // `update` and the render that show it.
    let (runtime, trace) = RecordingRuntime::traced();
    let mut driver = PaneLoop::new(runtime);
    let mut out = Vec::new();
    driver.apply(
        create(CONSOLE, PaneKind::Console),
        &mut NoFrameSink,
        &mut out,
    );
    driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));

    // The order a click-and-type raises them in: focus, then the move that
    // decides what is under the pointer, then the press, then the keys.
    let sent = [
        PaneInput::Focus,
        PaneInput::MouseMove { x: 1, y: 1 },
        PaneInput::MouseDown { x: 1, y: 1 },
        PaneInput::KeyChar("a".to_string()),
    ];
    let mut queue: std::collections::VecDeque<PaneCommand> = sent
        .iter()
        .cloned()
        .map(|input| PaneCommand::Input { id: CONSOLE, input })
        .collect();

    trace.borrow_mut().clear();
    out.clear();
    let mut sink = RecordingSink::new();
    drain(&mut driver, &mut queue, &mut out);
    driver.iterate(&mut sink, &mut out);

    assert_eq!(
        driver.view_mut(CONSOLE).unwrap().inputs,
        sent.to_vec(),
        "the queue is a FIFO, and one pane's stream is its order"
    );
    assert_eq!(
        *trace.borrow(),
        vec![
            format!("input:{CONSOLE}:focus"),
            format!("input:{CONSOLE}:mousemove"),
            format!("input:{CONSOLE}:mousedown"),
            format!("input:{CONSOLE}:keychar"),
            "update".to_string(),
            "render".to_string(),
            format!("copy:{CONSOLE}"),
        ],
        "every input landed before the update and the render that show it"
    );
    assert_eq!(sink.published.len(), 1, "and this frame drew");
}

#[test]
fn a_queued_gamepad_snapshot_is_in_the_slot_before_the_update_that_reads_it() {
    // The pad slot is filled by a system chained ahead of `drive_pane_host` and
    // drained with everything else, so it is in place for the pre-update
    // push — the phase whose whole reason is that a console polls the pads
    // inside `Renderer::update`.
    let (runtime, trace) = RecordingRuntime::traced();
    let mut driver = PaneLoop::new(runtime);
    let mut out = Vec::new();
    driver.apply(
        create(CONSOLE, PaneKind::Console),
        &mut NoFrameSink,
        &mut out,
    );
    driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
    let mut sink = RecordingSink::new();
    // One iteration to settle the load edge: nothing is pushed into a
    // document that has not finished loading.
    driver.iterate(&mut sink, &mut out);

    let mut queue = std::collections::VecDeque::from([PaneCommand::SetGamepadScript(Some(
        "window.__phoenixSetGamepads([])".to_string(),
    ))]);
    trace.borrow_mut().clear();
    out.clear();
    drain(&mut driver, &mut queue, &mut out);
    driver.iterate(&mut sink, &mut out);

    assert_eq!(
        *trace.borrow(),
        vec![
            format!("push:{CONSOLE}"),
            "update".to_string(),
            "render".to_string(),
            format!("copy:{CONSOLE}"),
        ],
        "queued this frame, pushed this frame, and ahead of the update"
    );
}

#[test]
fn a_resize_queued_after_an_input_is_applied_after_it() {
    // The two are raised by different systems, and the resize's system runs
    // first — but what decides which the view sees first is the queue, not
    // which system pushed. A resize that overtook an input would deliver a
    // click at coordinates the view had already moved past.
    let (runtime, trace) = RecordingRuntime::traced();
    let mut driver = PaneLoop::new(runtime);
    let mut out = Vec::new();
    driver.apply(
        create(CONSOLE, PaneKind::Console),
        &mut NoFrameSink,
        &mut out,
    );

    let mut queue = std::collections::VecDeque::from([
        PaneCommand::Input {
            id: CONSOLE,
            input: PaneInput::MouseMove { x: 2, y: 2 },
        },
        PaneCommand::Resize {
            id: CONSOLE,
            width: WIDTH * 2,
            height: HEIGHT,
            epoch: 3,
        },
    ]);
    trace.borrow_mut().clear();
    out.clear();
    drain(&mut driver, &mut queue, &mut out);

    assert_eq!(
        *trace.borrow(),
        vec![
            format!("input:{CONSOLE}:mousemove"),
            format!("resize:{CONSOLE}"),
        ]
    );
    assert_eq!(
        driver.view_mut(CONSOLE).unwrap().resizes,
        vec![(WIDTH * 2, HEIGHT)]
    );
}

#[test]
fn shutdown_is_the_last_command_the_loop_takes() {
    let mut driver = one_pane(CONSOLE, PaneKind::Console);
    let mut out = Vec::new();
    assert_eq!(
        driver.apply(PaneCommand::Shutdown, &mut NoFrameSink, &mut out),
        LoopControl::Stop
    );
    assert!(out.is_empty());
}
