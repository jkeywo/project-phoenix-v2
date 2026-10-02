use super::*;
fn surface() -> SurfaceIdentity {
    SurfaceIdentity {
        id: 7,
        epoch: 2,
        kind: "console",
        width: 10,
        height: 20,
        device_scale: 1.5,
        visible: true,
    }
}

#[test]
fn rectangle_log_and_capture_keep_dirty_knowledge_separate_from_copied_bounds() {
    let rect = FrameRect {
        left: 2,
        top: 3,
        right: 7,
        bottom: 9,
    };
    let partial = CopyObservation::from_result(&Ok(Some(rect)), false, FullCopyReasons::default());
    let json = serde_json::to_value(partial.operation(42)).unwrap();
    assert_eq!(
        json["dirty_rect"],
        serde_json::json!({ "left": 2, "top": 3, "right": 7, "bottom": 9 })
    );
    assert_eq!(json["copied_rect"], json["dirty_rect"]);
    assert_eq!(json["dirty_pixels"], 30);
    assert_eq!(json["copied_pixels"], 30);
    assert_eq!(json["duration_ns"], 42);
    assert!(partial.log_line(surface()).contains("id=7 epoch=2 kind=console raster=10x20 device_scale=1.5 visible=true outcome=copied dirty_rect=[2,3,7,9] copied_rect=[2,3,7,9] forced=false"));

    let forced = CopyObservation::from_result(
        &Ok(Some(FrameRect::full(10, 20))),
        true,
        FullCopyReasons {
            reveal: true,
            ..Default::default()
        },
    );
    let json = serde_json::to_value(forced.operation(0)).unwrap();
    assert!(json["dirty_rect"].is_null() && json["dirty_pixels"].is_null());
    assert_eq!(json["copied_pixels"], 200);
    assert!(forced
        .log_line(surface())
        .contains("dirty_rect=unknown copied_rect=[0,0,10,20] forced=true"));
    assert_eq!(json["reasons"]["reveal"], true);

    let clean = CopyObservation::from_result(&Ok(None), false, FullCopyReasons::default());
    assert_eq!(clean.dirty_rect, Some(FrameRect::default()));
    assert!(clean
        .log_line(surface())
        .contains("outcome=clean dirty_rect=[0,0,0,0] copied_rect=none"));
    let failed = CopyObservation::from_result(
        &Err(PaneSurfaceError::Frame("lock".into())),
        false,
        FullCopyReasons::default(),
    );
    assert!(failed
        .log_line(surface())
        .contains("outcome=failed dirty_rect=unknown copied_rect=none"));
    let json = serde_json::to_value(failed.operation(1)).unwrap();
    assert!(json["dirty_pixels"].is_null());
    assert_eq!(json["copied_pixels"], 0);
}

#[test]
fn buffer_lifetime_records_exactly_one_terminal_outcome_and_original_identity() {
    let observer = SurfaceObserver::new(Instant::now(), 32);
    let mut frame = observer.produced(surface(), 200, true, FullCopyReasons::default(), None);
    frame.drained();
    frame.extracted();
    frame.deferred(1);
    frame.uploaded(200, true, 42);
    frame.discarded(DiscardReason::StaleEpoch);
    drop(frame);
    let events = observer.events();
    assert_eq!(events.len(), 5);
    assert!(events
        .iter()
        .all(|e| e.surface == Some(surface()) && e.frame == Some(0)));
    assert!(matches!(
        events.last().unwrap().operation,
        Operation::Uploaded {
            write_texture_ns: 42,
            ..
        }
    ));
}

#[test]
fn bounded_recording_keeps_exact_totals_and_discloses_missing_raw_events() {
    let observer = SurfaceObserver::new(Instant::now(), 1);
    let frame = observer.produced(surface(), 1, false, FullCopyReasons::default(), None);
    drop(frame);
    let recording = observer.0.recording.lock().unwrap();
    assert_eq!(recording.events.len(), 1);
    assert_eq!(recording.omitted_events, 1);
    assert_eq!(recording.totals["produced"], 1);
    assert_eq!(recording.totals["discarded"], 1);
}

#[test]
fn closure_discloses_in_flight_frames_and_refuses_late_worker_events() {
    let observer = SurfaceObserver::new(Instant::now(), 32);
    let frame = observer.produced(surface(), 200, true, FullCopyReasons::default(), None);
    let capture = SurfaceCapture {
        observer: observer.clone(),
        path: PathBuf::new(),
        started_unix_ms: 0,
    };
    let artifact = capture.close(&AppExit::Success).unwrap();
    assert!(artifact.successful_exit);
    assert_eq!(artifact.in_flight_at_close, 1);
    assert_eq!(artifact.totals["produced"], 1);
    assert_eq!(artifact.omitted_events, 0);
    drop(frame);
    observer.record(None, Operation::MainFrame);
    let recording = observer.0.recording.lock().unwrap();
    assert!(recording.closed);
    assert!(recording.events.is_empty());
    assert!(
        recording.totals.is_empty(),
        "the finished artifact has one immutable boundary"
    );
}
