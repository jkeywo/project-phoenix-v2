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
            std::env::temp_dir().join(format!("phoenix-workshop-bridge-{}", uuid::Uuid::new_v4())),
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
        WorkshopWorker::spawn_with_operators(provider, operators, None, None, None).unwrap()
    }
    fn hosted_worker(&self, documents: crate::delivery::serve::HostedDocuments) -> WorkshopWorker {
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
