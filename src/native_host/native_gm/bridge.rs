//! Private GM presentation mailbox. It never joins the crew pane bus.
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::native_host::panes::{PaneId, PaneSurface};

const MAX_RECORDS: usize = 256;
const MAX_RECORD_BYTES: usize = 128 * 1024;
const MAX_QUEUED_BYTES: usize = 512 * 1024;

#[derive(Default)]
struct Inner {
    active: Option<PaneId>,
    live: bool,
    failed: bool,
    failure_pending: bool,
    latest: BTreeMap<String, String>,
    pending: BTreeMap<String, String>,
    records: VecDeque<String>,
}

#[derive(Clone, Default)]
pub struct NativeGmBridge(Arc<Mutex<Inner>>);

impl NativeGmBridge {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn publish(&self, channel: &str, json: String) {
        let mut state = self.lock();
        if state.latest.get(channel) == Some(&json) {
            return;
        }
        state.latest.insert(channel.into(), json.clone());
        state.pending.insert(channel.into(), json);
    }

    /// A new view must receive every projection, including unchanged paused state.
    pub fn activate(&self, id: PaneId) {
        let mut state = self.lock();
        state.active = Some(id);
        state.live = false;
        state.failed = false;
        state.records.clear();
        state.pending = state.latest.clone();
    }

    pub fn close(&self) {
        let mut state = self.lock();
        state.active = None;
        state.live = false;
        state.records.clear();
    }

    pub fn fault(&self) {
        let mut state = self.lock();
        state.failed = true;
        state.failure_pending = true;
        state.live = false;
        state.records.clear();
    }
    pub fn take_failure(&self) -> bool {
        std::mem::take(&mut self.lock().failure_pending)
    }
    pub fn failed(&self) -> bool {
        self.lock().failed
    }
    pub fn live(&self) -> bool {
        self.lock().live
    }
    pub fn mark_live(&self) {
        self.lock().live = true;
    }
    pub fn take_records(&self) -> Vec<String> {
        self.lock().records.drain(..).collect()
    }

    pub fn pump(&self, id: PaneId, surface: &mut dyn PaneSurface) -> usize {
        if !surface.is_ready() {
            return 0;
        }
        let pending = {
            let mut state = self.lock();
            if state.active != Some(id) || state.failed {
                surface.drain();
                return 0;
            }
            std::mem::take(&mut state.pending)
        };
        let mut pushed = 0;
        for (channel, json) in pending {
            let script = vellum_ultralight::bridge::push_call(
                &format!("window.__phoenixNativeGmChannels.{channel}"),
                &json,
            );
            if surface.push(&script).is_ok() {
                pushed += 1;
            } else {
                self.lock().pending.entry(channel).or_insert(json);
            }
        }
        let records = surface.drain();
        let mut state = self.lock();
        if state.active == Some(id) && !state.failed {
            // Reliable actions are one ordered batch. An overflowing surface
            // is faulted rather than applying a suffix after losing its prefix.
            let bytes: usize = state.records.iter().map(String::len).sum();
            let incoming_bytes = records.iter().try_fold(0usize, |sum, record| {
                (record.len() <= MAX_RECORD_BYTES)
                    .then(|| sum.checked_add(record.len()))
                    .flatten()
            });
            if state.records.len().saturating_add(records.len()) > MAX_RECORDS
                || incoming_bytes
                    .is_none_or(|incoming| bytes.saturating_add(incoming) > MAX_QUEUED_BYTES)
            {
                state.failed = true;
                state.failure_pending = true;
                state.live = false;
                state.records.clear();
            } else {
                state.records.extend(records);
            }
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_host::panes::PaneSurfaceError;
    #[derive(Default)]
    struct Surface {
        scripts: Vec<String>,
        records: Vec<String>,
        refuse: bool,
    }
    impl PaneSurface for Surface {
        fn load(&mut self, _: &str) -> Result<(), PaneSurfaceError> {
            Ok(())
        }
        fn is_ready(&self) -> bool {
            true
        }
        fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
            if self.refuse {
                return Err(PaneSurfaceError::Script("loading".into()));
            }
            self.scripts.push(script.into());
            Ok(())
        }
        fn drain(&mut self) -> Vec<String> {
            std::mem::take(&mut self.records)
        }
    }
    #[test]
    fn replacement_surface_replays_latest_projection_and_rejects_old_surface_records() {
        let bridge = NativeGmBridge::default();
        bridge.publish("gm_session", "{\"paused\":true}".into());
        bridge.activate(PaneId(10));
        let mut surface = Surface::default();
        assert_eq!(bridge.pump(PaneId(10), &mut surface), 1);
        assert_eq!(bridge.pump(PaneId(10), &mut surface), 0);
        bridge.activate(PaneId(11));
        surface.records.push("stale action".into());
        assert_eq!(bridge.pump(PaneId(10), &mut surface), 0);
        assert!(bridge.take_records().is_empty());
        assert_eq!(bridge.pump(PaneId(11), &mut surface), 1);
        assert!(surface.scripts.last().unwrap().contains("paused"));
    }
    #[test]
    fn load_retry_preserves_projection_and_failure_edge_survives_rebuild() {
        let bridge = NativeGmBridge::default();
        bridge.activate(PaneId(1));
        bridge.publish("metadata", "{}".into());
        let mut surface = Surface {
            refuse: true,
            ..Default::default()
        };
        assert_eq!(bridge.pump(PaneId(1), &mut surface), 0);
        surface.refuse = false;
        assert_eq!(bridge.pump(PaneId(1), &mut surface), 1);
        bridge.fault();
        bridge.activate(PaneId(2));
        assert!(bridge.take_failure());
        assert!(!bridge.take_failure());
        assert!(!bridge.live());
    }

    #[test]
    fn reliable_overflow_faults_the_whole_batch_and_accepts_no_suffix() {
        for records in [
            vec!["action".into(); MAX_RECORDS + 1],
            vec!["x".repeat(MAX_RECORD_BYTES + 1)],
            vec!["x".repeat(MAX_RECORD_BYTES); 5],
        ] {
            let bridge = NativeGmBridge::default();
            bridge.activate(PaneId(1));
            bridge.mark_live();
            let mut surface = Surface {
                records,
                ..Default::default()
            };
            bridge.pump(PaneId(1), &mut surface);
            assert!(bridge.failed());
            assert!(bridge.take_failure());
            assert!(!bridge.live());
            assert!(bridge.take_records().is_empty());
            surface.records.push("suffix action".into());
            bridge.pump(PaneId(1), &mut surface);
            assert!(bridge.take_records().is_empty());
        }
    }
}
