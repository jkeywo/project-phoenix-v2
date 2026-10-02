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
#[path = "bridge_tests.rs"]
mod tests;
