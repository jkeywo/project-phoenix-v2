//! Private, host-bound one-shot endpoints. No room subscription or playback log.
use super::{
    decoder::{self, Pcm},
    engine::Mixer,
    mix::{AudioMix, Bus},
};
use crate::native_host::{
    bridge_media::validate_media, bridge_profile::BridgeProfile, panes::PaneId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FRESH: Duration = Duration::from_millis(250);
#[path = "private_audition.rs"]
mod audition;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Endpoint {
    Console(PaneId),
    Gm(PaneId),
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub audition: bool,
    pub preview: &'static str,
    pub preview_id: u64,
    pub generation: u64,
    pub revision: u64,
    pub status: &'static str,
    pub test: &'static str,
    pub detail: String,
    pub outputs: Vec<String>,
    pub surface: String,
    pub categories: [&'static str; 2],
}
struct Entry {
    preview: Option<audition::Pending>,
    status: Status,
    routing_error: String,
    retry: u64,
    pending: Option<(Instant, String, bool)>,
    mix: AudioMix,
    mono: bool,
    reduced_range: bool,
    mixers: Vec<Arc<Mutex<Mixer>>>,
}
impl Entry {
    fn stop(&mut self) {
        audition::stop(self);
        self.pending = None;
        self.status.test = "idle";
        for mixer in &self.mixers {
            mixer.lock().unwrap().stop_all();
        }
    }
}
struct State {
    asset_revision: u64,
    entries: BTreeMap<Endpoint, Entry>,
    profile: BridgeProfile,
    generation: u64,
    continuation: Option<u64>,
    room_output: Option<String>,
    quit: bool,
    available: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            asset_revision: 0,
            entries: BTreeMap::new(),
            profile: BridgeProfile::empty(),
            generation: 0,
            continuation: None,
            room_output: None,
            quit: false,
            available: true,
        }
    }
}
#[derive(Clone, Default)]
pub struct PrivateAudio(Arc<Mutex<State>>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    #[serde(rename = "type")]
    kind: String,
    generation: u64,
    mix: Option<PrivateMix>,
    mono: Option<bool>,
    #[serde(rename = "reducedRange")]
    reduced_range: Option<bool>,
    #[serde(default)]
    stop: bool,
    #[serde(default)]
    retry: bool,
    cue: Option<Cue>,
    preview: Option<audition::Request>,
    #[serde(default)]
    stop_preview: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateMix {
    master: Bus,
    alerts: Bus,
    interface: Bus,
    #[serde(default)]
    music: Bus,
    #[serde(default)]
    ambience: Bus,
    #[serde(default)]
    effects: Bus,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cue {
    id: String,
    at_ms: u64,
    #[serde(default)]
    test: bool,
}
#[derive(Clone, Deserialize)]
struct Sound {
    file: String,
    category: String,
    volume: f32,
}
#[derive(Deserialize)]
pub(crate) struct Manifest {
    sounds: BTreeMap<String, Sound>,
}

impl PrivateAudio {
    pub fn new(profile: BridgeProfile) -> Self {
        Self(Arc::new(Mutex::new(State {
            profile,
            ..Default::default()
        })))
    }
    /// Identity and media key come from the host registry, never a page record.
    pub fn bind(&self, key: Endpoint, surface: &str) {
        let mut state = self.0.lock().unwrap();
        if state.entries.contains_key(&key) {
            return;
        }
        state.generation += 1;
        let generation = state.generation;
        state.entries.insert(
            key,
            Entry {
                preview: None,
                status: Status {
                    audition: matches!(key, Endpoint::Gm(_)),
                    preview: "idle",
                    preview_id: 0,
                    generation,
                    revision: 0,
                    status: "loading",
                    test: "idle",
                    detail: String::new(),
                    outputs: vec![],
                    surface: surface.into(),
                    categories: ["alerts", "interface"],
                },
                routing_error: String::new(),
                retry: 0,
                pending: None,
                mix: AudioMix::default(),
                mono: false,
                reduced_range: false,
                mixers: vec![],
            },
        );
        refresh_routes(&mut state);
    }
    pub fn close(&self, key: Endpoint) {
        let mut state = self.0.lock().unwrap();
        if let Some(mut entry) = state.entries.remove(&key) {
            entry.stop();
        }
        refresh_routes(&mut state);
    }
    pub fn profile(&self, profile: BridgeProfile) {
        let mut state = self.0.lock().unwrap();
        state.profile = profile;
        refresh_routes(&mut state);
    }
    pub fn continuation(&self, generation: u64) {
        let mut state = self.0.lock().unwrap();
        if state.continuation == Some(generation) {
            return;
        }
        state.continuation = Some(generation);
        retire_entries(&mut state);
    }
    pub fn refresh_assets(&self, revision: u64) {
        refresh_assets(&mut self.0.lock().unwrap(), revision);
    }
    /// The room worker reports its actually opened output. This is only a
    /// collision check, never a private routing choice or fallback.
    pub fn room_output(&self, output: Option<String>) {
        let mut state = self.0.lock().unwrap();
        if state.room_output == output {
            return;
        }
        state.room_output = output;
        refresh_routes(&mut state);
    }
    pub fn status(&self, key: Endpoint) -> Option<Status> {
        self.0
            .lock()
            .unwrap()
            .entries
            .get(&key)
            .map(|entry| entry.status.clone())
    }
    pub fn script(&self, key: Endpoint) -> Option<String> {
        let json =
            crate::core::codec::encode_native_private_audio_state(&self.status(key)?).ok()?;
        Some(vellum_ultralight::bridge::push_call(
            "window.__phoenixPrivateAudioApply",
            &json,
        ))
    }
    /// Recognised private envelopes are consumed even when stale or malformed.
    pub fn submit(&self, key: Endpoint, json: &str) -> bool {
        if !json.contains("\"NativePrivateAudio\"") {
            return false;
        }
        // Includes bounded authored equivalent text for one current audition.
        if json.len() > 16_384 {
            return true;
        }
        let Ok(request) = crate::core::codec::decode_native_private_audio_request(json) else {
            return true;
        };
        if request.kind != "NativePrivateAudio" {
            return false;
        }
        let mut state = self.0.lock().unwrap();
        let Some(entry) = state.entries.get_mut(&key) else {
            return true;
        };
        if request.generation != entry.status.generation {
            return true;
        }
        entry.status.revision += 1;
        if let Some(enabled) = request.mono {
            entry.mono = enabled;
            for mixer in &entry.mixers {
                mixer.lock().unwrap().set_mono(enabled);
            }
        }
        if let Some(enabled) = request.reduced_range {
            entry.reduced_range = enabled;
            for mixer in &entry.mixers {
                mixer.lock().unwrap().set_reduced_range(enabled);
            }
        }
        if let Some(mix) = request.mix {
            entry.mix = AudioMix {
                master: mix.master,
                alerts: mix.alerts,
                interface: mix.interface,
                music: mix.music,
                ambience: mix.ambience,
                effects: mix.effects,
            }
            .sanitised();
            for mixer in &entry.mixers {
                mixer.lock().unwrap().set_mix(entry.mix);
            }
            audition::mute(entry);
            if entry.pending.as_ref().is_some_and(|(_, id, _)| {
                private_category(id).is_none_or(|category| entry.mix.gain(category, 1.0) == 0.0)
            }) {
                entry.pending = None;
            }
        }
        if request.stop || request.retry {
            entry.stop();
        }
        if request.retry {
            entry.retry += 1;
        }
        if request.stop_preview {
            audition::stop(entry);
        }
        if let Some(preview) = request.preview {
            audition::request(entry, preview);
        }
        if let Some(cue) = request.cue {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            if entry.status.status == "playing"
                && now >= u128::from(cue.at_ms)
                && now - u128::from(cue.at_ms) <= FRESH.as_millis()
                && (!cue.test || cue.id == "test")
                && private_category(&cue.id)
                    .is_some_and(|category| entry.mix.gain(category, 1.0) > 0.0)
            {
                entry.pending = Some((
                    Instant::now() - Duration::from_millis((now - u128::from(cue.at_ms)) as u64),
                    cue.id,
                    cue.test,
                ));
            }
        }
        true
    }
    pub fn shutdown(&self) {
        let mut state = self.0.lock().unwrap();
        state.quit = true;
        for entry in state.entries.values_mut() {
            entry.stop();
        }
    }
    pub fn unavailable(&self) {
        let mut state = self.0.lock().unwrap();
        state.available = false;
        refresh_routes(&mut state);
    }
}

fn retire_entries(state: &mut State) {
    let mut next = state.generation;
    state.generation += state.entries.len() as u64;
    for entry in state.entries.values_mut() {
        next += 1;
        entry.stop();
        entry.status.generation = next;
        entry.status.revision += 1;
    }
}

fn refresh_assets(state: &mut State, revision: u64) {
    if state.asset_revision != revision {
        state.asset_revision = revision;
        retire_entries(state);
        for entry in state.entries.values_mut() {
            if entry.status.detail == "settings.audio.asset_failed" {
                entry.status.detail.clear();
            }
        }
    }
}

fn refresh_routes(state: &mut State) {
    let mut effective = state.profile.clone();
    let room_named = effective
        .media
        .iter()
        .find(|entry| entry.surface == "viewscreen")
        .is_some_and(|entry| !entry.outputs.is_empty());
    if !room_named {
        if let Some(id) = &state.room_output {
            if let Some(entry) = effective
                .media
                .iter_mut()
                .find(|entry| entry.surface == "viewscreen")
            {
                entry.outputs = vec![id.clone()];
            } else {
                effective
                    .media
                    .push(crate::native_host::bridge_media::MediaSurfaceEntry {
                        surface: "viewscreen".into(),
                        outputs: vec![id.clone()],
                        camera: None,
                        microphones: vec![],
                        allow_shared: vec![],
                    });
            }
        }
    }
    let unresolved_room = !room_named && state.room_output.is_none();
    let valid = validate_media(&effective.media).is_ok();
    let mut counts = BTreeMap::new();
    for entry in state.entries.values() {
        *counts.entry(entry.status.surface.clone()).or_insert(0) += 1;
    }
    for (key, entry) in &mut state.entries {
        let surface = &entry.status.surface;
        let reserved =
            surface == "viewscreen" || (surface == "native-gm" && !matches!(key, Endpoint::Gm(_)));
        let outputs = state
            .profile
            .media
            .iter()
            .find(|media| media.surface == *surface)
            .map(|media| media.outputs.clone())
            .unwrap_or_default();
        let error = if !state.available {
            "settings.audio.private_backend_unavailable"
        } else if !valid {
            "settings.audio.private_invalid_assignment"
        } else if reserved || counts[surface] > 1 {
            "settings.audio.private_surface_collision"
        } else if outputs.is_empty() {
            "settings.audio.private_unassigned"
        } else if unresolved_room {
            "settings.audio.private_room_unresolved"
        } else {
            ""
        };
        if entry.status.outputs != outputs || entry.routing_error != error {
            entry.stop();
            entry.status.outputs = outputs;
            entry.status.detail = error.into();
            entry.routing_error = error.into();
            entry.status.status = if error.is_empty() {
                "loading"
            } else {
                "failed"
            };
            entry.status.revision += 1;
            entry.retry += 1;
        }
    }
}

fn private_category(id: &str) -> Option<&'static str> {
    match id {
        "actionable" => Some("alerts"),
        "clicks" | "pending" | "applied" | "refused" | "timedOut" | "test" => Some("interface"),
        _ => None,
    }
}

/// The production worker and deterministic fake-device tests share this path.
pub trait Backend {
    type Stream;
    fn scan(&mut self) -> Result<Vec<(String, bool)>, String>;
    fn open(&mut self, id: &str, mixer: Arc<Mutex<Mixer>>) -> Result<Self::Stream, String>;
    fn failed(&self, stream: &Self::Stream) -> bool;
}
struct Output<S> {
    stream: S,
    mixer: Arc<Mutex<Mixer>>,
}
struct Live<S> {
    retry: u64,
    generation: u64,
    outputs: Vec<Output<S>>,
}
pub struct Worker<B: Backend> {
    asset_revision: u64,
    backend: B,
    live: BTreeMap<Endpoint, Live<B::Stream>>,
    manifest: BTreeMap<String, Sound>,
    pcm: BTreeMap<String, Result<Arc<Pcm>, String>>,
}
impl<B: Backend> Worker<B> {
    pub fn new(backend: B) -> Self {
        let manifest = crate::core::codec::decode_native_private_audio_manifest(include_str!(
            "../../../assets/audio/private-feedback.json"
        ))
        .expect("private sound manifest");
        Self {
            asset_revision: 0,
            backend,
            live: BTreeMap::new(),
            manifest: manifest.sounds,
            pcm: BTreeMap::new(),
        }
    }
    /// Prewarm outside any callback/control lock; a slow decoder loses a cue.
    pub fn prepare(&mut self) {
        for sound in self.manifest.values() {
            self.pcm
                .entry(sound.file.clone())
                .or_insert_with(|| decoder::read(&sound.file));
        }
    }
    pub fn step(&mut self, hub: &PrivateAudio) -> bool {
        // Deterministic fake-device entry point: the caller owns the epoch.
        self.step_with_revision(hub, None)
    }
    fn step_with_revision(&mut self, hub: &PrivateAudio, source: Option<fn() -> u64>) -> bool {
        let revision = {
            let mut state = hub.0.lock().unwrap();
            if let Some(read) = source {
                refresh_assets(&mut state, read());
            }
            state.asset_revision
        };
        if self.asset_revision != revision {
            self.asset_revision = revision;
            self.pcm.clear();
        }
        self.prepare();
        audition::prepare_with_revision(hub, source);
        let devices = self.backend.scan();
        let mut state = hub.0.lock().unwrap();
        if let Some(read) = source {
            refresh_assets(&mut state, read());
        }
        if state.quit {
            self.live.clear();
            return false;
        }
        // A revision accepted while decoding invalidates all prepared PCM.
        // Current owner retirement already stopped outputs and consumed slots.
        if state.asset_revision != self.asset_revision {
            return true;
        }
        self.live.retain(|key, _| state.entries.contains_key(key));
        for (key, entry) in &mut state.entries {
            let live = self.live.entry(*key).or_insert_with(|| Live {
                retry: u64::MAX,
                generation: 0,
                outputs: vec![],
            });
            let retry = live.retry != entry.retry;
            if retry {
                live.outputs.clear();
                entry.stop();
                entry.mixers.clear();
                live.retry = entry.retry;
            }
            if live.generation != entry.status.generation {
                entry.stop();
                live.generation = entry.status.generation;
            }
            let error = if !entry.routing_error.is_empty() {
                Some(entry.routing_error.clone())
            } else if let Err(error) = &devices {
                Some(error.clone())
            } else {
                let devices = devices.as_ref().unwrap();
                entry.status.outputs.iter().find_map(|id| {
                    match devices.iter().find(|(name, _)| name == id) {
                        None => Some("settings.audio.selected_missing".into()),
                        Some((_, true)) => Some("settings.audio.selected_ambiguous".into()),
                        _ => None,
                    }
                })
            };
            let error = error.or_else(|| {
                live.outputs
                    .iter()
                    .any(|output| self.backend.failed(&output.stream))
                    .then(|| "settings.audio.device_stopped".into())
            });
            if let Some(error) = error {
                entry.stop();
                live.outputs.clear();
                entry.mixers.clear();
                entry.status.status = "failed";
                entry.status.detail = error;
            } else if retry {
                entry.status.detail.clear();
                for id in &entry.status.outputs {
                    let mut mixer = Mixer::default();
                    mixer.set_mix(entry.mix);
                    mixer.set_mono(entry.mono);
                    mixer.set_reduced_range(entry.reduced_range);
                    let mixer = Arc::new(Mutex::new(mixer));
                    match self.backend.open(id, mixer.clone()) {
                        Ok(stream) => live.outputs.push(Output { stream, mixer }),
                        Err(error) => {
                            entry.status.detail = error;
                            break;
                        }
                    }
                }
                if entry.status.detail.is_empty() && !live.outputs.is_empty() {
                    entry.mixers = live
                        .outputs
                        .iter()
                        .map(|output| output.mixer.clone())
                        .collect();
                    entry.status.status = "playing";
                } else {
                    live.outputs.clear();
                    entry.status.status = "failed";
                }
            }
            // Commit while holding current owner/control. Close, mute, restore,
            // or route changes cannot race a prepared old cue back into playback.
            if let Some((at, id, test)) = entry.pending.take() {
                if entry.status.status == "playing" && at.elapsed() <= FRESH {
                    if let Some(sound) = self.manifest.get(&id) {
                        match self.pcm.get(&sound.file) {
                            Some(Ok(pcm)) => {
                                let category = if sound.category == "alerts" {
                                    "alerts"
                                } else {
                                    "interface"
                                };
                                for output in &live.outputs {
                                    output.mixer.lock().unwrap().cue(
                                        "private",
                                        pcm.clone(),
                                        category,
                                        sound.volume,
                                        Some(1.0),
                                    );
                                }
                                if test {
                                    entry.status.test = "playing";
                                }
                            }
                            _ => {
                                entry.status.detail = "settings.audio.asset_failed".into();
                                entry.status.test = "idle";
                            }
                        }
                    }
                }
            }
            if !live
                .outputs
                .iter()
                .any(|output| output.mixer.lock().unwrap().active("private"))
            {
                entry.status.test = "idle";
            }
            audition::commit(entry);
        }
        true
    }
}

#[cfg(feature = "host")]
pub fn spawn(hub: PrivateAudio) -> Option<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("phoenix-private-audio".into())
        .spawn(move || {
            let mut worker = Worker::new(cpal_backend::Cpal::default());
            while worker
                .step_with_revision(&hub, Some(crate::entities::config_cache::mod_pack_revision))
            {
                std::thread::sleep(Duration::from_millis(10));
            }
        })
        .ok()
}

#[cfg(feature = "host")]
mod cpal_backend {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[derive(Default)]
    pub struct Cpal {
        devices: Option<crate::native_host::media_output::OutputDevices>,
        scanned: Option<Instant>,
    }
    pub struct Stream {
        _stream: cpal::Stream,
        failed: Arc<AtomicBool>,
    }
    impl Backend for Cpal {
        type Stream = Stream;
        fn scan(&mut self) -> Result<Vec<(String, bool)>, String> {
            if self
                .scanned
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
            {
                self.scanned = Some(Instant::now());
                self.devices = None;
                self.devices = Some(crate::native_host::media_output::OutputDevices::scan()?);
            }
            let devices = self
                .devices
                .as_ref()
                .ok_or("settings.audio.device_stopped")?;
            Ok(devices
                .discovered
                .iter()
                .enumerate()
                .map(|(index, device)| (device.identity.to_string(), devices.ambiguous[index]))
                .collect())
        }
        fn open(&mut self, id: &str, mixer: Arc<Mutex<Mixer>>) -> Result<Stream, String> {
            let devices = self
                .devices
                .as_ref()
                .ok_or("settings.audio.selected_missing")?;
            let index = devices
                .discovered
                .iter()
                .position(|device| device.identity.as_str() == id)
                .ok_or("settings.audio.selected_missing")?;
            if devices.ambiguous[index] {
                return Err("settings.audio.selected_ambiguous".into());
            }
            let failed = Arc::new(AtomicBool::new(false));
            let stream =
                super::super::device::open(&devices.handles[index], mixer, failed.clone())?;
            Ok(Stream {
                _stream: stream,
                failed,
            })
        }
        fn failed(&self, stream: &Stream) -> bool {
            stream.failed.load(Ordering::Acquire)
        }
    }
}

#[cfg(test)]
#[path = "private_tests.rs"]
mod tests;
