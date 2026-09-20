//! Ordered private request bridge. Source IO runs on its own worker so a large
//! validated save cannot freeze the UI renderer. Replies stay with their view.
use crate::{
    native_host::panes::{PaneId, PaneSurface},
    workshop::provider::NativeWorkshopProvider,
};
use std::{
    collections::VecDeque,
    sync::{mpsc, Arc, Mutex},
    thread,
};

const MAX_RECORD_BYTES: usize = 32 * 1024 * 1024;
enum Job {
    Request {
        epoch: u64,
        pane: PaneId,
        record: String,
    },
    Stop,
    #[cfg(test)]
    InstallTest {
        epoch: u64,
        process: Box<super::test_process::TestProcess>,
        ready: mpsc::SyncSender<()>,
    },
    #[cfg(test)]
    InstallPreview {
        epoch: u64,
        snapshot: crate::workshop::provider::preview_snapshot::PreviewSnapshot,
        ready: mpsc::SyncSender<()>,
    },
}
#[derive(Default)]
struct Inner {
    active: Option<PaneId>,
    epoch: u64,
    failed: bool,
    live: bool,
    replies: VecDeque<String>,
    reply_bytes: usize,
}
#[derive(Clone)]
pub struct WorkshopBridge {
    state: Arc<Mutex<Inner>>,
    requests: mpsc::SyncSender<Job>,
}
impl WorkshopBridge {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn activate(&self, pane: PaneId) {
        let mut state = self.lock();
        state.epoch += 1;
        state.active = Some(pane);
        state.failed = false;
        state.live = false;
        state.replies.clear();
        state.reply_bytes = 0;
    }
    pub fn fault(&self) {
        let mut state = self.lock();
        state.epoch += 1;
        state.failed = true;
        state.live = false;
    }
    pub fn failed(&self) -> bool {
        self.lock().failed
    }
    pub fn live(&self) -> bool {
        self.lock().live
    }
    pub fn pump(&self, pane: PaneId, surface: &mut dyn PaneSurface) -> usize {
        if !surface.is_ready() {
            return 0;
        }
        let mut state = self.lock();
        if state.active != Some(pane) || state.failed {
            drop(state);
            surface.drain();
            return 0;
        }
        let epoch = state.epoch;
        let mut count = 0;
        while let Some(script) = state.replies.front() {
            if surface.push(script).is_err() {
                break;
            }
            let length = script.len();
            state.reply_bytes -= length;
            state.replies.pop_front();
            count += 1;
        }
        drop(state);
        for record in surface.drain() {
            if record == "NativeWorkshopReady" {
                let mut state = self.lock();
                if state.epoch == epoch && state.active == Some(pane) && !state.failed {
                    state.live = true;
                }
                continue;
            }
            if record.len() > MAX_RECORD_BYTES
                || self
                    .requests
                    .try_send(Job::Request {
                        epoch,
                        pane,
                        record,
                    })
                    .is_err()
            {
                self.fault();
                break;
            }
        }
        count
    }
}

pub struct WorkshopWorker {
    bridge: WorkshopBridge,
    thread: Option<thread::JoinHandle<()>>,
}
impl WorkshopWorker {
    pub fn spawn(provider: NativeWorkshopProvider) -> Result<Self, String> {
        let mut operators = crate::native_host::panes::operator::NativeOperators::default();
        operators.configure("workshop");
        Self::spawn_with_operators(provider, operators, None, None)
    }

    pub fn spawn_hosted(
        provider: NativeWorkshopProvider,
        documents: crate::delivery::serve::HostedDocuments,
        origin: String,
    ) -> Result<Self, String> {
        let mut operators = crate::native_host::panes::operator::NativeOperators::default();
        operators.configure("workshop");
        let (tool_root, capture_directory) = provider.capture_paths();
        let capture = super::billboard_capture::BillboardCapture::new(
            documents.clone(),
            origin.clone(),
            tool_root,
            capture_directory,
        )?;
        Self::spawn_with_operators(
            provider,
            operators,
            Some(super::preview::PreviewRoutes::new(documents, origin)),
            Some(capture),
        )
    }

    fn spawn_with_operators(
        mut provider: NativeWorkshopProvider,
        mut operators: crate::native_host::panes::operator::NativeOperators,
        mut preview: Option<super::preview::PreviewRoutes>,
        mut billboard: Option<super::billboard_capture::BillboardCapture>,
    ) -> Result<Self, String> {
        super::test_process::retire_abandoned_stages(&provider.test_directory());
        let (requests, input) = mpsc::sync_channel(8);
        let state = Arc::new(Mutex::new(Inner::default()));
        let bridge = WorkshopBridge {
            state: state.clone(),
            requests,
        };
        let thread =
            thread::Builder::new()
                .name("phoenix-workshop-source".into())
                .spawn(move || {
                    let mut active_epoch = 0;
                    let mut test: Option<super::test_process::TestProcess> = None;
                    loop {
                        let job = match input.recv_timeout(std::time::Duration::from_millis(50)) {
                            Ok(job) => job,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                // A failed/replaced page may never send another
                                // request. Retire after the accepted FIFO drains,
                                // independently of a replacement document mounting.
                                let epoch = state.lock().unwrap_or_else(|e| e.into_inner()).epoch;
                                if epoch != active_epoch {
                                    provider.retire_view();
                                    if let Some(preview) = preview.as_mut() {
                                        preview.retire();
                                    }
                                    if let Some(capture) = billboard.as_mut() { capture.retire(); }
                                    test = None;
                                    active_epoch = epoch;
                                }
                                continue;
                            }
                        };
                        #[cfg(test)]
                        if let Job::InstallTest {
                            epoch,
                            process,
                            ready,
                        } = job
                        {
                            test = Some(*process);
                            active_epoch = epoch;
                            let _ = ready.send(());
                            continue;
                        }
                        #[cfg(test)]
                        if let Job::InstallPreview {
                            epoch,
                            snapshot,
                            ready,
                        } = job
                        {
                            preview.as_mut().unwrap().publish(snapshot);
                            active_epoch = epoch;
                            let _ = ready.send(());
                            continue;
                        }
                        let Job::Request {
                            epoch,
                            pane,
                            record,
                        } = job
                        else {
                            break;
                        };
                        if epoch != active_epoch {
                            provider.retire_view();
                            if let Some(preview) = preview.as_mut() {
                                preview.retire();
                            }
                            if let Some(capture) = billboard.as_mut() { capture.retire(); }
                            // A view crash/replacement retains source recovery, but
                            // cannot leave a detached disposable simulation alive.
                            test = None;
                            active_epoch = epoch;
                        }
                        let scripts =
                            if operators.handle(pane, "operator", &record) {
                                operators.replies.remove(&pane).unwrap_or_default()
                            } else {
                                let response =
                                    match crate::core::codec::decode_workshop_request(&record) {
                                        Ok(request) => {
                                            use crate::workshop::provider::{
                                                Operation, Response, WorkshopResponse,
                                            };
                                            let id = request.id;
                                            let result = match request.operation {
                                    Operation::TestStart { files, selection, breakpoint } => {
                                        if let Some(preview) = preview.as_mut() {
                                            preview.retire();
                                        }
                                        if let Some(capture) = billboard.as_mut() { capture.retire(); }
                                        match provider.prepare_test(files, selection, breakpoint) {
                                            Ok(snapshot) => {
                                                let started = std::env::current_exe()
                                                    .map_err(|e| e.to_string())
                                                    .and_then(|executable| {
                                                        super::test_process::TestProcess::start(
                                                            &executable,
                                                            &provider.test_directory(),
                                                            snapshot,
                                                        )
                                                    });
                                                match started {
                                                    Ok(mut started) => {
                                                        let run = started.status();
                                                        test = run.running.then_some(started);
                                                        Response::Test { run: Some(run.into()) }
                                                    }
                                                    Err(message) => Response::Refused {
                                                        message,
                                                        report: None,
                                                    },
                                                }
                                            }
                                            Err(refusal) => refusal,
                                        }
                                    }
                                    Operation::TestControl { control } => match test.as_mut() {
                                        Some(process) => match process.control(control) {
                                            Ok(run) => Response::Test { run: Some(run.into()) },
                                            Err(message) => Response::Refused {
                                                message,
                                                report: None,
                                            },
                                        },
                                        None => Response::Refused {
                                            message: "No disposable Test is running".into(),
                                            report: None,
                                        },
                                    },
                                    Operation::TestStatus => {
                                        let run = test.as_mut().map(|process| process.status());
                                        // A closed output pipe is a failed Test
                                        // even if the process has not exited.
                                        // Release both it and its stage now;
                                        // the UI cannot control a dead channel.
                                        if run.as_ref().is_some_and(|run| !run.running) {
                                            test = None;
                                        }
                                        Response::Test { run: run.map(Box::new) }
                                    }
                                    Operation::TestStop => {
                                        test = None;
                                        Response::Test { run: None }
                                    }
                                    Operation::PreviewStart { files, selection } => {
                                        match preview.as_mut() {
                                            Some(routes) => match provider.prepare_preview(files, selection) {
                                                Ok(snapshot) => routes.publish(snapshot),
                                                Err(refusal) => refusal,
                                            },
                                            None => Response::Refused {
                                                message: "Native Workshop preview delivery is unavailable".into(),
                                                report: None,
                                            },
                                        }
                                    }
                                    Operation::PreviewRelease { capture } => {
                                        if let Some(preview) = preview.as_mut() {
                                            preview.release(&capture);
                                        }
                                        Response::Done
                                    }
                                    Operation::PreviewStop => {
                                        if let Some(preview) = preview.as_mut() {
                                            preview.retire();
                                        }
                                        Response::Done
                                    }
                                    Operation::BillboardCaptureStart { files, sidecar, lod, source_revision } => {
                                        match billboard.as_mut() {
                                            Some(capture) => match provider.prepare_billboard_capture(files) {
                                                Ok(files) => capture.start(files, sidecar, lod, source_revision)
                                                    .unwrap_or_else(|message| Response::Refused { message, report: None }),
                                                Err(response) => response,
                                            },
                                            None => Response::Refused { message: "Native billboard capture is unavailable".into(), report: None },
                                        }
                                    }
                                    Operation::BillboardCaptureStatus => match billboard.as_mut() {
                                        Some(capture) => capture.status().unwrap_or_else(|message| Response::Refused { message, report: None }),
                                        None => Response::Refused { message: "Native billboard capture is unavailable".into(), report: None },
                                    },
                                    Operation::BillboardCaptureCancel => match billboard.as_mut() {
                                        Some(capture) => capture.cancel(),
                                        None => Response::Done,
                                    },
                                    operation => {
                                        provider
                                            .handle(crate::workshop::provider::WorkshopRequest {
                                                id,
                                                operation,
                                            })
                                            .result
                                    }
                                };
                                            crate::core::codec::encode_workshop_response(
                                                &WorkshopResponse { id, result },
                                            )
                                            .expect("finite private Workshop responses serialize")
                                        }
                                        Err(_) => provider.handle_json(&record),
                                    };
                                vec![vellum_ultralight::bridge::push_call(
                                    "window.__phoenixNativeWorkshopReply",
                                    &response,
                                )]
                            };
                        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                        if state.epoch != epoch || state.active != Some(pane) || state.failed {
                            continue;
                        }
                        let bytes: usize = scripts.iter().map(String::len).sum();
                        if state.reply_bytes.saturating_add(bytes) > 2 * MAX_RECORD_BYTES {
                            state.epoch += 1;
                            state.failed = true;
                            state.live = false;
                            continue;
                        }
                        state.reply_bytes += bytes;
                        state.replies.extend(scripts);
                    }
                })
                .map_err(|e| e.to_string())?;
        Ok(Self {
            bridge,
            thread: Some(thread),
        })
    }
    pub fn bridge(&self) -> WorkshopBridge {
        self.bridge.clone()
    }
}
impl Drop for WorkshopWorker {
    fn drop(&mut self) {
        // The worker completes an already accepted transactional write before
        // releasing its OS root claim. No source operation is torn in half.
        let _ = self.bridge.requests.send(Job::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_host::panes::{operator::NativeOperators, RecordingSurface};
    use crate::workshop::{provider::WorkspaceKind, WorkshopDependencies};
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        #[allow(clippy::disallowed_methods)] // Private test workspace, never simulation identity.
        fn new() -> Self {
            let fixture = Self(
                std::env::temp_dir()
                    .join(format!("phoenix-workshop-bridge-{}", uuid::Uuid::new_v4())),
            );
            fs::create_dir_all(fixture.0.join("project/assets/worlds")).unwrap();
            fs::write(
                fixture.0.join("project/assets/scenarios.toml"),
                "[content]\nid='phoenix-base'\nepoch=1\n",
            )
            .unwrap();
            fs::write(
                fixture.0.join("project/assets/worlds/test.toml"),
                "# Keep\r\n[global]\r\ntitle='Test'\r\n",
            )
            .unwrap();
            fixture
        }
        fn worker(&self) -> WorkshopWorker {
            let provider = NativeWorkshopProvider::open(
                WorkspaceKind::Project,
                self.0.join("project"),
                self.0.join("private"),
                WorkshopDependencies::default(),
            )
            .unwrap();
            let mut operators = NativeOperators::default();
            operators.configure("workshop");
            operators.root = Some(self.0.join("profiles"));
            WorkshopWorker::spawn_with_operators(provider, operators, None, None).unwrap()
        }
        fn hosted_worker(
            &self,
            documents: crate::delivery::serve::HostedDocuments,
        ) -> WorkshopWorker {
            let provider = NativeWorkshopProvider::open(
                WorkspaceKind::Project,
                self.0.join("project"),
                self.0.join("private"),
                WorkshopDependencies::default(),
            )
            .unwrap();
            let mut operators = NativeOperators::default();
            operators.configure("workshop");
            WorkshopWorker::spawn_with_operators(
                provider,
                operators,
                Some(super::super::preview::PreviewRoutes::new(
                    documents,
                    "http://127.0.0.1:7".into(),
                )),
                None,
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn wait_replies(bridge: &WorkshopBridge, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while bridge.lock().replies.len() < count {
            assert!(
                Instant::now() < deadline,
                "worker did not publish expected replies"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn source_replies_retry_in_order_and_only_a_mounted_workspace_is_live() {
        let fixture = Fixture::new();
        let worker = fixture.worker();
        let bridge = worker.bridge();
        let pane = PaneId(1);
        bridge.activate(pane);
        let mut surface = RecordingSurface::ready();
        surface.queue_record(r#"{"id":1,"op":"load-sources"}"#);
        surface.queue_record(r#"{"id":2,"op":"recovery-load"}"#);
        bridge.pump(pane, &mut surface);
        assert!(
            !bridge.live(),
            "loaded HTML is not proof its modules mounted"
        );
        wait_replies(&bridge, 2);
        surface.failing_pushes = 1;
        assert_eq!(bridge.pump(pane, &mut surface), 0);
        assert_eq!(bridge.pump(pane, &mut surface), 2);
        assert!(surface.pushed[0].contains("\"id\":1"));
        assert!(
            surface.pushed[0].contains("\\\\r\\\\n"),
            "{}",
            surface.pushed[0]
        );
        assert!(surface.pushed[1].contains("\"id\":2"));
        surface.queue_record("NativeWorkshopReady");
        bridge.pump(pane, &mut surface);
        assert!(bridge.live());
    }
    #[test]
    fn replaced_views_cannot_receive_old_replies_or_submit_source_requests() {
        let fixture = Fixture::new();
        let worker = fixture.worker();
        let bridge = worker.bridge();
        bridge.activate(PaneId(1));
        let mut old = RecordingSurface::ready();
        old.queue_record(r#"{"id":1,"op":"load-sources"}"#);
        bridge.pump(PaneId(1), &mut old);
        wait_replies(&bridge, 1);
        bridge.activate(PaneId(2));
        old.queue_record("NativeWorkshopReady");
        old.queue_record(r#"{"id":2,"op":"recovery-clear"}"#);
        bridge.pump(PaneId(1), &mut old);
        assert!(old.pushed.is_empty());
        assert!(old.queued_records.is_empty());
        assert!(!bridge.live());
        let mut replacement = RecordingSurface::ready();
        replacement.queue_record(r#"{"id":3,"op":"load-sources"}"#);
        bridge.pump(PaneId(2), &mut replacement);
        wait_replies(&bridge, 1);
        bridge.pump(PaneId(2), &mut replacement);
        assert_eq!(replacement.pushed.len(), 1);
        assert!(replacement.pushed[0].contains("\"id\":3"));
    }
    #[test]
    fn native_profile_survives_worker_and_provider_restart() {
        let fixture = Fixture::new();
        {
            let worker = fixture.worker();
            let bridge = worker.bridge();
            bridge.activate(PaneId(1));
            let mut surface = RecordingSurface::ready();
            surface.queue_record(r#"{"type":"NativeOperator","operation":"save","profile":"{\"kind\":\"project-phoenix/operator-profile\",\"version\":1,\"accessibility\":{\"presentation\":{\"textScale\":1.5}}}"}"#);
            bridge.pump(PaneId(1), &mut surface);
            wait_replies(&bridge, 1);
            bridge.pump(PaneId(1), &mut surface);
            assert!(surface.pushed[0].contains("\"status\":\"ok\""));
        }
        let worker = fixture.worker();
        let bridge = worker.bridge();
        bridge.activate(PaneId(2));
        let mut surface = RecordingSurface::ready();
        surface.queue_record(r#"{"type":"NativeOperator","operation":"load"}"#);
        bridge.pump(PaneId(2), &mut surface);
        wait_replies(&bridge, 1);
        bridge.pump(PaneId(2), &mut surface);
        assert!(
            surface.pushed[0].contains("textScale\\\": 1.5"),
            "{}",
            surface.pushed[0]
        );
    }

    #[test]
    fn replacement_view_can_import_after_the_old_view_abandoned_an_upload() {
        let fixture = Fixture::new();
        let worker = fixture.worker();
        let bridge = worker.bridge();
        bridge.activate(PaneId(1));
        let mut old = RecordingSurface::ready();
        old.queue_record(r#"{"id":1,"op":"asset-begin","length":150000}"#);
        bridge.pump(PaneId(1), &mut old);
        wait_replies(&bridge, 1);
        bridge.pump(PaneId(1), &mut old);
        assert!(old.pushed[0].contains("asset-upload"));
        bridge.activate(PaneId(2));
        let mut replacement = RecordingSurface::ready();
        replacement.queue_record(r#"{"id":2,"op":"asset-begin","length":3}"#);
        bridge.pump(PaneId(2), &mut replacement);
        wait_replies(&bridge, 1);
        bridge.pump(PaneId(2), &mut replacement);
        assert!(
            replacement.pushed[0].contains("asset-upload"),
            "{}",
            replacement.pushed[0]
        );
        assert!(!replacement.pushed[0].contains("refused"));
    }

    #[test]
    fn fault_or_replacement_without_another_document_request_retires_the_child() {
        for fault in [false, true] {
            let fixture = Fixture::new();
            let worker = fixture.worker();
            let bridge = worker.bridge();
            bridge.activate(PaneId(1));
            let (mut process, path) =
                super::super::test_process::pipe_probe(&fixture.0.join("test-probes"));
            assert!(
                process
                    .control(super::super::test_clock::TestControl::Pause {})
                    .unwrap()
                    .running
            );
            let (ready, installed) = mpsc::sync_channel(1);
            bridge
                .requests
                .send(Job::InstallTest {
                    epoch: bridge.lock().epoch,
                    process: Box::new(process),
                    ready,
                })
                .unwrap();
            installed.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(path.exists());
            if fault {
                bridge.fault();
            } else {
                bridge.activate(PaneId(2));
            }
            // No subsequent request is submitted. This exercises the actual
            // worker's idle retirement and a real inherited-pipe child.
            let deadline = Instant::now() + Duration::from_secs(5);
            while path.exists() {
                assert!(
                    Instant::now() < deadline,
                    "orphaned Test stage after document retirement"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    #[test]
    fn test_start_and_pane_retirement_withdraw_preview_capture_routes() {
        use crate::workshop::{
            provider::preview_snapshot::PreviewSnapshot, test_protocol::PreviewSelection,
        };
        for action in ["test", "fault", "replace"] {
            let fixture = Fixture::new();
            let documents = crate::delivery::serve::HostedDocuments::default();
            let worker = fixture.hosted_worker(documents.clone());
            let bridge = worker.bridge();
            bridge.activate(PaneId(1));
            let (ready, installed) = mpsc::sync_channel(1);
            bridge
                .requests
                .send(Job::InstallPreview {
                    epoch: bridge.lock().epoch,
                    snapshot: PreviewSnapshot {
                        files: std::collections::BTreeMap::from([(
                            "assets/models/draft.glb".into(),
                            vec![1, 2, 3],
                        )]),
                        selection: PreviewSelection {
                            model: Some("assets/models/draft.glb".into()),
                            ..Default::default()
                        },
                        revision: "capture".into(),
                    },
                    ready,
                })
                .unwrap();
            installed.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(documents.len(), 1);
            match action {
                "test" => {
                    let mut surface = RecordingSurface::ready();
                    surface.queue_record(r#"{"id":1,"op":"test-start","files":{},"selection":{"world":"missing","ship":"missing","seed":1}}"#);
                    bridge.pump(PaneId(1), &mut surface);
                }
                "fault" => bridge.fault(),
                "replace" => bridge.activate(PaneId(2)),
                _ => unreachable!(),
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while !documents.is_empty() {
                assert!(
                    Instant::now() < deadline,
                    "preview routes survived {action}"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
