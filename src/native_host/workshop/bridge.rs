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
        Self::spawn_with_operators(provider, operators, None, None, None)
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
        let (tool_root, generation_directory) = provider.lod_generation_paths();
        let generation = super::lod_generation::LodGeneration::new(
            documents.clone(),
            origin.clone(),
            tool_root,
            generation_directory,
        )?;
        Self::spawn_with_operators(
            provider,
            operators,
            Some(super::preview::PreviewRoutes::new(documents, origin)),
            Some(capture),
            Some(generation),
        )
    }

    fn spawn_with_operators(
        mut provider: NativeWorkshopProvider,
        mut operators: crate::native_host::panes::operator::NativeOperators,
        mut preview: Option<super::preview::PreviewRoutes>,
        mut billboard: Option<super::billboard_capture::BillboardCapture>,
        mut lod_generation: Option<super::lod_generation::LodGeneration>,
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
                                    if let Some(capture) = billboard.as_mut() {
                                        capture.retire();
                                    }
                                    if let Some(run) = lod_generation.as_mut() {
                                        run.retire();
                                    }
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
                            if let Some(capture) = billboard.as_mut() {
                                capture.retire();
                            }
                            if let Some(run) = lod_generation.as_mut() {
                                run.retire();
                            }
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
                                        if let Some(capture) = billboard.as_mut() {
                                            capture.retire();
                                        }
                                        if let Some(run) = lod_generation.as_mut() {
                                            run.retire();
                                        }
                                        match provider.prepare_test(files, selection, breakpoint) {
                                            Ok(snapshot) => {
                                                let started = std::env::current_exe()
                                                    .map_err(|e| e.to_string())
                                                    .and_then(|executable| {
                                                        super::test_process::TestProcess::start_with_delivery(
                                                            &executable,
                                                            &provider.test_directory(),
                                                            snapshot,
                                                            preview.as_ref().map(|routes| routes.delivery()),
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
                                    Operation::LodGenerateStart { files, sidecar, source_revision, remesh } => {
                                        match lod_generation.as_mut() {
                                            Some(run) => match provider.prepare_lod_generation(files) {
                                                Ok(files) => run.start(files, sidecar, source_revision, remesh)
                                                    .unwrap_or_else(|message| Response::Refused { message, report: None }),
                                                Err(response) => response,
                                            },
                                            None => Response::Refused { message: "Native LOD generation is unavailable".into(), report: None },
                                        }
                                    }
                                    Operation::LodGenerateStatus => match lod_generation.as_mut() {
                                        Some(run) => run.status().unwrap_or_else(|message| Response::Refused { message, report: None }),
                                        None => Response::Refused { message: "Native LOD generation is unavailable".into(), report: None },
                                    },
                                    Operation::LodGenerateCancel => match lod_generation.as_mut() {
                                        Some(run) => run.cancel(), None => Response::Done,
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
                                            crate::core::codec::to_json(
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
#[path = "bridge_tests.rs"]
mod tests;
