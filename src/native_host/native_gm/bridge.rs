//! Private GM presentation mailbox. It never joins the crew pane bus.
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::native_host::panes::{PaneId, PaneSurface};

const MAX_RECORDS: usize = 256;
const MAX_RECORD_BYTES: usize = 128 * 1024;
const MAX_QUEUED_BYTES: usize = 512 * 1024;

#[derive(Default)]
struct Inner {
    audio: Option<crate::native_host::audio::private::PrivateAudio>,
    audio_sent: Option<String>,
    operators: crate::native_host::panes::operator::NativeOperators,
    active: Option<PaneId>,
    live: bool,
    failed: bool,
    failure_pending: bool,
    latest: BTreeMap<String, String>,
    pending: BTreeMap<String, String>,
    records: VecDeque<String>,
    save_outcomes: VecDeque<super::saves::SaveOutcome>,
}

#[derive(Clone, Default)]
pub struct NativeGmBridge(Arc<Mutex<Inner>>);

impl NativeGmBridge {
    /// Retain bounded completion receipts even after the CLI logger drains
    /// the store, or while an embedded frame has not yet pumped its mailbox.
    pub(super) fn retain_save_outcomes(&self, outcomes: Vec<super::saves::SaveOutcome>) {
        if outcomes.is_empty() {
            return;
        }
        let mut state = self.lock();
        for outcome in outcomes {
            state.save_outcomes.retain(|row| row.slot != outcome.slot);
            state.save_outcomes.push_back(outcome);
        }
        while state.save_outcomes.len() > 64 {
            state.save_outcomes.pop_front();
        }
        let outcomes: Vec<_> = state.save_outcomes.iter().cloned().collect();
        drop(state);
        if let Ok(json) = crate::core::codec::encode_native_gm_save_outcomes(&outcomes) {
            self.publish("save_outcomes", json);
        }
    }

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
        if let Some(previous) = state.active {
            state.operators.close(previous);
            if let Some(audio) = &state.audio {
                audio.close(crate::native_host::audio::private::Endpoint::Gm(previous));
            }
        }
        state.audio_sent = None;
        if let Some(audio) = &state.audio {
            audio.bind(
                crate::native_host::audio::private::Endpoint::Gm(id),
                "native-gm",
            );
        }
        state.active = Some(id);
        state.live = false;
        state.failed = false;
        state.records.clear();
        state.pending = state.latest.clone();
    }

    pub fn close(&self) {
        let mut state = self.lock();
        if let Some(previous) = state.active {
            state.operators.close(previous);
            if let Some(audio) = &state.audio {
                audio.close(crate::native_host::audio::private::Endpoint::Gm(previous));
            }
        }
        state.audio_sent = None;
        state.active = None;
        state.live = false;
        state.records.clear();
    }

    pub fn fault(&self) {
        let mut state = self.lock();
        if let Some(previous) = state.active {
            state.operators.close(previous);
            if let Some(audio) = &state.audio {
                audio.close(crate::native_host::audio::private::Endpoint::Gm(previous));
            }
        }
        state.audio_sent = None;
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
    pub fn attach_audio(&self, audio: crate::native_host::audio::private::PrivateAudio) {
        let mut state = self.lock();
        if state.audio.is_some() {
            return;
        }
        if let Some(id) = state.active {
            audio.bind(
                crate::native_host::audio::private::Endpoint::Gm(id),
                "native-gm",
            );
        }
        state.audio = Some(audio);
    }
    pub fn set_operator_scope(&self, hull: &str) {
        let mut state = self.lock();
        // Same store, a host-chosen GM scope. A crew label cannot select this
        // file and the private workspace never creates a crew session identity.
        if state.operators.configure(&format!("native-gm:{hull}")) {
            if let Some(id) = state.active {
                state
                    .operators
                    .replies
                    .insert(id, vec!["window.__phoenixOperatorReload()".into()]);
            }
        }
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
        let audio_script = {
            let state = self.lock();
            state
                .audio
                .as_ref()
                .and_then(|audio| {
                    audio.script(crate::native_host::audio::private::Endpoint::Gm(id))
                })
                .filter(|script| state.audio_sent.as_ref() != Some(script))
        };
        if let Some(script) = audio_script {
            if surface.push(&script).is_ok() {
                self.lock().audio_sent = Some(script);
                pushed += 1;
            }
        }
        let replies = self
            .lock()
            .operators
            .replies
            .remove(&id)
            .unwrap_or_default();
        for (index, reply) in replies.iter().enumerate() {
            if surface.push(reply).is_err() {
                self.lock()
                    .operators
                    .replies
                    .insert(id, replies[index..].to_vec());
                break;
            }
            pushed += 1;
        }
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
        let incoming = surface.drain();
        let mut state = self.lock();
        if state.active == Some(id) && !state.failed {
            let mut records = Vec::new();
            for record in incoming {
                if state.audio.as_ref().is_some_and(|audio| {
                    audio.submit(
                        crate::native_host::audio::private::Endpoint::Gm(id),
                        &record,
                    )
                }) {
                    continue;
                }
                if record.len() <= 1024 * 1024 && record.contains("\"NativeOperator\"") {
                    if let Some(allowed) = crate::core::codec::is_native_gm_profile_record(&record)
                    {
                        if allowed {
                            state.operators.handle(id, "native-gm", &record);
                        }
                        continue;
                    }
                }
                records.push(record);
            }
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
                if let Some(audio) = &state.audio {
                    audio.close(crate::native_host::audio::private::Endpoint::Gm(id));
                }
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
    #[test]
    #[allow(clippy::disallowed_methods)] // Unique test-only filesystem sandbox, never simulation identity.
    fn native_gm_audio_profile_reloads_on_its_host_scope_without_crew_identity() {
        let root = std::env::temp_dir().join(format!(
            "phoenix-gm-private-profile-{}",
            uuid::Uuid::new_v4()
        ));
        let bridge = NativeGmBridge::default();
        bridge.set_operator_scope("cruiser");
        bridge.lock().operators.root = Some(root.clone());
        bridge.activate(PaneId(1));
        let mut surface = Surface::default();
        surface
            .records
            .push(include_str!("../../../tests/fixtures/native-private-profile-save.json").into());
        bridge.pump(PaneId(1), &mut surface);
        assert!(bridge.take_records().is_empty());
        assert_eq!(
            bridge.lock().operators.scope.as_deref(),
            Some("native-gm:cruiser")
        );
        // A fresh process-style bridge uses the same established operator store.
        let relaunched = NativeGmBridge::default();
        relaunched.set_operator_scope("cruiser");
        relaunched.lock().operators.root = Some(root.clone());
        relaunched.activate(PaneId(2));
        surface
            .records
            .push(r#"{"type":"NativeOperator","operation":"load"}"#.into());
        relaunched.pump(PaneId(2), &mut surface);
        relaunched.pump(PaneId(2), &mut surface);
        let reply: serde_json::Value = surface
            .scripts
            .iter()
            .rev()
            .filter_map(|script| {
                let json = script
                    .strip_prefix("window.__phoenixOperatorReply(")?
                    .strip_suffix(')')?;
                serde_json::from_str::<serde_json::Value>(json).ok()
            })
            .find(|reply| reply["operation"] == "load")
            .expect("the real bridge returned the loaded operator profile");
        assert_eq!(reply["status"], "ok");
        let profile: serde_json::Value =
            serde_json::from_str(reply["profile"].as_str().unwrap()).unwrap();
        assert_eq!(profile["audio"]["mix"]["master"]["level"], 0.23);
        assert_eq!(profile["audio"]["mix"]["master"]["muted"], true);
        assert_eq!(profile["audio"]["cues"]["applied"], true);
        // A literal same-name crew operator files under its ordinary hull scope.
        let mut crew = crate::native_host::panes::operator::NativeOperators::default();
        crew.configure("cruiser");
        crew.root = Some(root.clone());
        crew.handle(
            PaneId(3),
            "native-gm",
            r#"{"type":"NativeOperator","operation":"load"}"#,
        );
        assert!(crew.replies[&PaneId(3)][0].contains(r#""profile":null"#));
        relaunched.set_operator_scope("courier");
        relaunched.lock().operators.root = Some(root.clone());
        surface.scripts.clear();
        surface
            .records
            .push(r#"{"type":"NativeOperator","operation":"load"}"#.into());
        relaunched.pump(PaneId(2), &mut surface);
        relaunched.pump(PaneId(2), &mut surface);
        assert!(surface
            .scripts
            .iter()
            .any(|script| script.contains(r#""profile":null"#)));
        assert!(!surface.scripts.iter().any(|script| script.contains("0.23")));
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }
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
