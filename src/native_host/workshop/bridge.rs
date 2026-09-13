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
        Self::spawn_with_operators(provider, operators)
    }

    fn spawn_with_operators(
        mut provider: NativeWorkshopProvider,
        mut operators: crate::native_host::panes::operator::NativeOperators,
    ) -> Result<Self, String> {
        let (requests, input) = mpsc::sync_channel(8);
        let state = Arc::new(Mutex::new(Inner::default()));
        let bridge = WorkshopBridge {
            state: state.clone(),
            requests,
        };
        let thread = thread::Builder::new()
            .name("phoenix-workshop-source".into())
            .spawn(move || {
                let mut active_epoch = 0;
                while let Ok(job) = input.recv() {
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
                        active_epoch = epoch;
                    }
                    let scripts = if operators.handle(pane, "operator", &record) {
                        operators.replies.remove(&pane).unwrap_or_default()
                    } else {
                        vec![vellum_ultralight::bridge::push_call(
                            "window.__phoenixNativeWorkshopReply",
                            &provider.handle_json(&record),
                        )]
                    };
                    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                    if state.epoch != epoch || state.active != Some(pane) || state.failed {
                        continue;
                    }
                    let bytes: usize = scripts.iter().map(String::len).sum();
                    if state.reply_bytes.saturating_add(bytes) > 2 * MAX_RECORD_BYTES {
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
            WorkshopWorker::spawn_with_operators(provider, operators).unwrap()
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
}
