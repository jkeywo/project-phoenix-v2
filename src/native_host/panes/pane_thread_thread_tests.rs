use super::doubles::{RecordingRuntime, RecordingView};
use super::*;

const PANE: PaneId = PaneId(7);
const TIMEOUT: Duration = Duration::from_secs(3);

struct TrackedRuntime {
    inner: RecordingRuntime, // Rc makes this !Send, deliberately.
    owner: thread::ThreadId,
    trace: Sender<(thread::ThreadId, String)>,
}
struct TrackedView {
    inner: RecordingView,
    owner: thread::ThreadId,
    trace: Sender<(thread::ThreadId, String)>,
}
fn record(owner: thread::ThreadId, trace: &Sender<(thread::ThreadId, String)>, event: String) {
    assert_eq!(owner, thread::current().id(), "SDK call crossed threads");
    let _ = trace.send((thread::current().id(), event));
}
impl Drop for TrackedRuntime {
    fn drop(&mut self) {
        record(self.owner, &self.trace, "drop-runtime".into());
    }
}
impl Drop for TrackedView {
    fn drop(&mut self) {
        record(self.owner, &self.trace, "drop-view".into());
    }
}
impl PaneRuntime for TrackedRuntime {
    type View = TrackedView;
    fn update(&mut self) {
        record(self.owner, &self.trace, "update".into());
        self.inner.update();
    }
    fn render(&mut self) {
        record(self.owner, &self.trace, "render".into());
        self.inner.render();
    }
    fn create(
        &mut self,
        id: PaneId,
        kind: PaneKind,
        spec: &PaneSpecOwned,
        url: &str,
    ) -> Result<Self::View, PaneSurfaceError> {
        record(self.owner, &self.trace, "create".into());
        let mut inner = self.inner.create(id, kind, spec, url)?;
        inner.paint = Some(FrameRect::full(spec.width, spec.height));
        Ok(TrackedView {
            inner,
            owner: self.owner,
            trace: self.trace.clone(),
        })
    }
}
impl PaneSurface for TrackedView {
    fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
        self.inner.load(url)
    }
    fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }
    fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
        record(self.owner, &self.trace, format!("push:{script}"));
        self.inner.push(script)
    }
    fn drain(&mut self) -> Vec<String> {
        self.inner.drain()
    }
}
impl PaneView for TrackedView {
    fn set_visible(&mut self, visible: bool) {
        self.inner.set_visible(visible);
    }
    fn refresh_loaded(&mut self) -> bool {
        self.inner.refresh_loaded()
    }
    fn resize(&mut self, width: u32, height: u32) {
        record(self.owner, &self.trace, "resize".into());
        self.inner.resize(width, height);
        self.inner.paint = Some(FrameRect::full(width, height));
    }
    fn input(&mut self, input: &PaneInput) {
        record(self.owner, &self.trace, format!("input:{input:?}"));
        assert!(
            !matches!(input, PaneInput::KeyChar(text) if text == "panic"),
            "view panic"
        );
        self.inner.input(input);
    }
    fn copy_frame(
        &mut self,
        dst: &mut [u8],
        force: bool,
    ) -> Result<Option<FrameRect>, PaneSurfaceError> {
        record(self.owner, &self.trace, "copy".into());
        self.inner.copy_frame(dst, force)
    }
}
fn spawn(period: Duration) -> (PaneThreadHandle, Receiver<(thread::ThreadId, String)>) {
    let (trace, observed) = mpsc::channel();
    let handle = spawn_pane_thread(
        PaneThreadConfig {
            period,
            measure: true,
            ..Default::default()
        },
        move || {
            let owner = thread::current().id();
            assert_eq!(thread::current().name(), Some("phoenix-panes"));
            record(owner, &trace, "start".into());
            Ok(TrackedRuntime {
                inner: RecordingRuntime::default(),
                owner,
                trace,
            })
        },
    )
    .unwrap();
    (handle, observed)
}
fn event(handle: &PaneThreadHandle) -> PaneEvent {
    handle
        .events
        .lock()
        .unwrap()
        .recv_timeout(TIMEOUT)
        .expect("pane thread event")
}
fn until(handle: &PaneThreadHandle, predicate: impl Fn(&PaneEvent) -> bool) -> PaneEvent {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = handle
            .events
            .lock()
            .unwrap()
            .recv_timeout(remaining)
            .expect("expected pane event");
        if predicate(&event) {
            return event;
        }
    }
}
fn create() -> PaneCommand {
    PaneCommand::Create {
        id: PANE,
        kind: PaneKind::Console,
        spec: PaneSpecOwned {
            width: 2,
            height: 2,
            device_scale: 1.0,
        },
        url: "http://localhost/console".into(),
        epoch: 0,
        visible: true,
    }
}

#[test]
fn real_thread_keeps_a_non_send_runtime_and_views_on_one_thread_and_drops_views_first() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PaneThreadHandle>();
    let main = thread::current().id();
    let (mut handle, trace) = spawn(Duration::from_millis(5));
    assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
    assert!(handle.is_running());
    handle.send(create()).unwrap();
    until(&handle, |event| matches!(event, PaneEvent::Frame(_)));
    handle
        .send(PaneCommand::Input {
            id: PANE,
            input: PaneInput::Focus,
        })
        .unwrap();
    handle
        .send(PaneCommand::Input {
            id: PANE,
            input: PaneInput::KeyChar("a".into()),
        })
        .unwrap();
    handle.stop();
    handle.stop();
    assert!(!handle.is_running());
    let observed: Vec<_> = trace.try_iter().collect();
    assert!(observed.iter().all(|(owner, _)| *owner != main));
    let names: Vec<_> = observed.iter().map(|(_, event)| event.as_str()).collect();
    let focus = names
        .iter()
        .position(|name| *name == "input:Focus")
        .unwrap();
    let key = names
        .iter()
        .position(|name| *name == "input:KeyChar(\"a\")")
        .unwrap();
    assert!(focus < key);
    assert_eq!(&names[names.len() - 2..], &["drop-view", "drop-runtime"]);
}

#[test]
fn a_command_arriving_during_the_wait_is_applied_before_the_next_iteration() {
    let (mut handle, _) = spawn(Duration::from_secs(1));
    assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
    until(&handle, |event| matches!(event, PaneEvent::Stats(_)));
    handle.send(create()).unwrap();
    let created = handle
        .events
        .lock()
        .unwrap()
        .recv_timeout(Duration::from_millis(500))
        .unwrap();
    assert!(matches!(created, PaneEvent::Created { result: Ok(()), .. }));
    handle.stop();
}

#[test]
fn startup_failure_and_startup_panic_both_begin_with_a_failed_handshake() {
    for panic in [false, true] {
        let mut handle = spawn_pane_thread(PaneThreadConfig::default(), move || {
            assert!(!panic, "factory panic");
            Err::<RecordingRuntime, _>("SDK unavailable".into())
        })
        .unwrap();
        assert!(matches!(event(&handle), PaneEvent::Started(Err(_))));
        handle.stop();
        assert!(!handle.is_running());
        assert!(matches!(handle.try_recv(), Err(TryRecvError::Disconnected)));
    }
}

#[test]
fn an_unwinding_view_failure_reports_terminal_death_and_finishes() {
    let (mut handle, trace) = spawn(Duration::from_millis(5));
    assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
    handle.send(create()).unwrap();
    until(&handle, |event| matches!(event, PaneEvent::Created { .. }));
    handle
        .send(PaneCommand::Input {
            id: PANE,
            input: PaneInput::KeyChar("panic".into()),
        })
        .unwrap();
    until(
        &handle,
        |event| matches!(event, PaneEvent::ThreadFailed { reason } if reason == "view panic"),
    );
    handle.stop();
    let names: Vec<_> = trace.try_iter().map(|(_, name)| name).collect();
    assert_eq!(&names[names.len() - 2..], &["drop-view", "drop-runtime"]);
}

#[test]
fn a_failed_create_is_reported_without_stopping_other_seats() {
    let mut handle = spawn_pane_thread(PaneThreadConfig::default(), || {
        Ok(RecordingRuntime {
            fail_create: HashMap::from([(PANE, "refused".into())]),
            ..Default::default()
        })
    })
    .unwrap();
    assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
    handle.send(create()).unwrap();
    until(
        &handle,
        |event| matches!(event, PaneEvent::Created { id, result: Err(_) } if *id == PANE),
    );
    let mut other = create();
    if let PaneCommand::Create { id, .. } = &mut other {
        *id = PaneId(8);
    }
    handle.send(other).unwrap();
    until(
        &handle,
        |event| matches!(event, PaneEvent::Created { id, result: Ok(()) } if *id == PaneId(8)),
    );
    assert!(handle.is_running());
    handle.stop();
}

#[test]
fn dropping_the_handle_stops_the_owning_thread() {
    let (handle, trace) = spawn(Duration::from_millis(5));
    assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
    drop(handle);
    assert!(trace.try_iter().any(|(_, name)| name == "drop-runtime"));
}

#[test]
fn a_stalled_thread_reports_timeout_once_and_drops_its_runtime_on_the_owner() {
    static REPORTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let (blocked, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (trace, observed) = mpsc::channel();
    let main = thread::current().id();
    let mut handle = spawn_pane_thread(
        PaneThreadConfig {
            on_shutdown_timeout: Some(|| {
                REPORTS.fetch_add(1, Ordering::Relaxed);
            }),
            ..Default::default()
        },
        move || {
            let runtime = TrackedRuntime {
                inner: RecordingRuntime::default(),
                owner: thread::current().id(),
                trace,
            };
            blocked.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(runtime)
        },
    )
    .unwrap();
    waiting.recv_timeout(TIMEOUT).unwrap();

    let start = Instant::now();
    handle.stop();
    assert!(start.elapsed() < TIMEOUT, "shutdown must stay bounded");
    assert!(
        handle.is_running(),
        "the stalled runtime remains on its owner"
    );
    assert_eq!(REPORTS.load(Ordering::Relaxed), 1);
    handle.stop();
    drop(handle);
    assert_eq!(REPORTS.load(Ordering::Relaxed), 1);

    release.send(()).unwrap();
    let (owner, event) = observed.recv_timeout(TIMEOUT).unwrap();
    assert_ne!(owner, main);
    assert_eq!(event, "drop-runtime");
}

#[test]
fn resize_changes_the_frame_generation_and_close_stops_publication() {
    let (mut handle, _) = spawn(Duration::from_millis(5));
    assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
    handle.send(create()).unwrap();
    until(&handle, |event| matches!(event, PaneEvent::Frame(_)));
    handle
        .send(PaneCommand::Resize {
            id: PANE,
            width: 3,
            height: 2,
            epoch: 1,
        })
        .unwrap();
    let PaneEvent::Frame(frame) = until(
        &handle,
        |event| matches!(event, PaneEvent::Frame(frame) if frame.epoch == 1),
    ) else {
        unreachable!()
    };
    assert_eq!(frame.bytes.len(), 24);
    assert_eq!(frame.rect, FrameRect::full(3, 2));
    assert!(frame.full);
    drop(frame);
    handle.send(PaneCommand::Close(PANE)).unwrap();
    until(
        &handle,
        |event| matches!(event, PaneEvent::Stats(sample) if sample.panes == 0),
    );
    assert!(matches!(event(&handle), PaneEvent::Stats(sample) if sample.panes == 0));
    handle.stop();
}

#[test]
fn retained_empty_gamepad_slot_reaches_js_even_when_main_frames_outrun_iterations() {
    let (mut handle, trace) = spawn(Duration::from_millis(5));
    assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
    handle.send(create()).unwrap();
    until(&handle, |event| matches!(event, PaneEvent::Frame(_)));
    handle
        .send(PaneCommand::SetGamepadScript(Some("populated".into())))
        .unwrap();
    handle
        .send(PaneCommand::SetGamepadScript(Some("empty".into())))
        .unwrap();
    // Quiet main frames send no None, so the final state cannot be coalesced away.
    for _ in 0..3 {
        until(&handle, |event| matches!(event, PaneEvent::Stats(_)));
    }
    handle.stop();
    let names: Vec<_> = trace.try_iter().map(|(_, name)| name).collect();
    let first = names.iter().position(|name| name == "push:empty").unwrap();
    assert_eq!(names[first + 1], "update");
    assert!(names.iter().filter(|name| *name == "push:empty").count() >= 2);
}

#[test]
fn pooled_frames_are_bounded_recycled_and_isolated_across_resize_and_close() {
    let mut sink = PooledFrames::new(PANE_STAGING_BUFFERS);
    sink.configure(PANE, PaneKind::Console, 16);
    for _ in 0..3 {
        assert_eq!(sink.stage(PANE, 16).unwrap()[3], 255);
        sink.publish(PANE, 0, FrameRect::full(2, 2), true);
    }
    assert!(sink.stage(PANE, 16).is_none());
    let held = sink.frames.pop().unwrap();
    drop(held);
    assert!(
        sink.stage(PANE, 16).is_some(),
        "drop recycled one allocation"
    );
    sink.publish(PANE, 0, FrameRect::full(2, 2), true);
    let old_frames = std::mem::take(&mut sink.frames);
    sink.configure(PANE, PaneKind::Hud, 16); // Equal length, different generation.
    drop(old_frames);
    for _ in 0..3 {
        assert_eq!(sink.stage(PANE, 16).unwrap()[3], 0);
        sink.publish(PANE, 1, FrameRect::full(2, 2), true);
    }
    assert!(
        sink.stage(PANE, 16).is_none(),
        "old buffers never inflated this pool"
    );
    sink.drop_pane(PANE);
    sink.frames.clear();
    assert!(
        sink.stage(PANE, 16).is_none(),
        "late returns never resurrect a closed pane"
    );
}
