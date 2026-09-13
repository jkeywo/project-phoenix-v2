//! Current room state to authored samples. The embedded pages never play audio.
use super::{
    decoder::{self, Pcm},
    engine::Mixer,
};
use crate::{audio_config::AudioConfigPayload, console_bridge::AudioLifecycleState};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};

pub const MENU: &str = "assets/sounds/exploration.mp3";
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoomInput {
    pub lifecycle: AudioLifecycleState,
    pub config: AudioConfigPayload,
    pub red_alert: bool,
    pub thrust: f32,
    pub menu: bool,
    pub phaser: bool,
    pub forcefield: f32,
}
pub struct RoomPlayer {
    pub mixer: Arc<Mutex<Mixer>>,
    cache: BTreeMap<String, Result<Arc<Pcm>, String>>,
    previous: Option<RoomInput>,
    was_ready: bool,
    pub failures: Vec<String>,
}
pub struct PreparedBlaster {
    pcm: Arc<Pcm>,
    gain: f32,
    matrix: super::spatial::StereoMatrix,
}
pub struct PreparedComputer {
    pcm: Arc<Pcm>,
    gain: f32,
}
impl Default for RoomPlayer {
    fn default() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::default())),
            cache: BTreeMap::new(),
            previous: None,
            was_ready: false,
            failures: Vec::new(),
        }
    }
}
impl RoomPlayer {
    pub fn prepare(&mut self, input: &RoomInput) {
        let mut paths: Vec<_> = [
            Some(MENU),
            input.config.ambient.as_ref().map(|s| s.file.as_str()),
            input.config.engine.as_ref().map(|s| s.file.as_str()),
            input.config.phaser_loop.as_ref().map(|s| s.file.as_str()),
            input.config.forcefield.as_ref().map(|s| s.file.as_str()),
            input.config.blaster.as_ref().map(|s| s.file.as_str()),
            input
                .config
                .red_alert
                .as_ref()
                .map(|s| s.music_file.as_str()),
            input
                .config
                .red_alert
                .as_ref()
                .map(|s| s.siren_file.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if let Some(config) = &input.config.computer_message {
            paths.extend(
                ["info", "advisory", "warning", "critical"]
                    .into_iter()
                    .filter_map(|severity| {
                        config.for_severity(severity).map(|cue| cue.file.as_str())
                    }),
            );
        }
        self.cache.retain(|path, _| paths.contains(&path.as_str()));
        self.failures.clear();
        for path in &paths {
            self.asset(path);
        }
    }
    fn asset(&mut self, file: &str) -> Option<Arc<Pcm>> {
        match self
            .cache
            .entry(file.into())
            .or_insert_with(|| decoder::read(file))
        {
            Ok(pcm) => Some(pcm.clone()),
            Err(error) => {
                let message = format!("{file}: {error}");
                if !self.failures.contains(&message) {
                    self.failures.push(message);
                }
                None
            }
        }
    }
    pub fn retry(&mut self) {
        self.cache.retain(|_, result| result.is_ok());
        self.failures.clear();
    }
    pub fn reset_output(&mut self) {
        self.mixer.lock().unwrap().stop_all();
        self.was_ready = false;
    }
    pub fn suppress_edges(&mut self) {
        self.was_ready = false;
    }
    pub fn apply(&mut self, input: &RoomInput, ready: bool) {
        let boundary = self.previous.as_ref().is_none_or(|previous| {
            previous.lifecycle.generation != input.lifecycle.generation
                || previous.config != input.config
        });
        if boundary || !ready {
            self.mixer.lock().unwrap().stop_all();
        }
        let live = input.lifecycle.running && !input.lifecycle.suspended && ready;
        let mut desired = Vec::new();
        if input.menu && !input.lifecycle.running && !input.lifecycle.suspended && ready {
            desired.push(("menu", MENU, "music", 0.5));
        }
        if live {
            if let Some(spec) = &input.config.ambient {
                desired.push(("ambient", spec.file.as_str(), "ambience", spec.volume));
            }
            if let Some(spec) = &input.config.engine {
                desired.push((
                    "engine",
                    spec.file.as_str(),
                    "ambience",
                    spec.idle_volume + input.thrust.clamp(0.0, 1.0) * spec.volume_at_full_thrust,
                ));
            }
            if input.red_alert {
                if let Some(spec) = &input.config.red_alert {
                    desired.push((
                        "music",
                        spec.music_file.as_str(),
                        "music",
                        spec.music_volume,
                    ));
                }
            }
            if input.phaser {
                if let Some(spec) = &input.config.phaser_loop {
                    desired.push(("phaser", spec.file.as_str(), "effects", spec.volume));
                }
            }
            if let Some(spec) = &input.config.forcefield {
                desired.push((
                    "forcefield",
                    spec.file.as_str(),
                    "effects",
                    input.forcefield,
                ));
            }
        }
        let desired_ids: Vec<_> = desired.iter().map(|(id, _, _, _)| *id).collect();
        for id in ["menu", "ambient", "engine", "music", "phaser", "forcefield"] {
            if !desired_ids.contains(&id) {
                self.mixer.lock().unwrap().stop(id);
            }
        }
        for (id, file, category, gain) in desired {
            if let Some(pcm) = self.asset(file) {
                self.mixer.lock().unwrap().set_loop(id, pcm, category, gain);
            }
        }
        if live
            && self.was_ready
            && !boundary
            && input.red_alert
            && self
                .previous
                .as_ref()
                .is_some_and(|previous| !previous.red_alert)
        {
            if let Some(spec) = &input.config.red_alert {
                if let Some(pcm) = self.asset(&spec.siren_file) {
                    self.mixer
                        .lock()
                        .unwrap()
                        .cue("siren", pcm, "alerts", spec.siren_volume, None);
                }
            }
        } else if let Some(spec) = &input.config.red_alert {
            // Prepare the configured alert before its live edge. Loading a file
            // never queues an occurrence for later catch-up.
            self.asset(&spec.siren_file);
        }
        self.previous = Some(input.clone());
        self.was_ready = ready;
    }
    pub fn test_output(&mut self, ready: bool) {
        if !ready {
            return;
        }
        if let Some(pcm) = self.asset(MENU) {
            self.mixer
                .lock()
                .unwrap()
                .cue("test", pcm, "music", 0.5, Some(2.0));
        }
    }
    pub fn prepare_blaster(
        &mut self,
        input: &RoomInput,
        position: [f32; 3],
        deadline: Option<Instant>,
    ) -> Option<PreparedBlaster> {
        if !input.lifecycle.running || input.lifecycle.suspended {
            return None;
        }
        let spec = input.config.blaster.as_ref()?;
        if position.iter().any(|value| !value.is_finite())
            || position.into_iter().map(|v| v * v).sum::<f32>()
                > spec.max_distance * spec.max_distance
        {
            return None;
        }
        let pcm = self.asset(&spec.file)?;
        let gain = spec.volume * super::spatial::distance_gain(spec, position);
        let (pcm, matrix) = match spec.panning_model {
            crate::audio_config::PanningModel::EqualPower => {
                let matrix = super::spatial::equal_power(position, pcm.channels);
                (pcm, matrix)
            }
            crate::audio_config::PanningModel::Hrtf => (
                super::hrtf::render(&pcm, position, deadline)?,
                super::spatial::StereoMatrix([[1.0, 0.0], [0.0, 1.0]]),
            ),
        };
        Some(PreparedBlaster { pcm, gain, matrix })
    }
    pub fn play_blaster(&mut self, input: &RoomInput, ready: bool, prepared: PreparedBlaster) {
        if ready && input.lifecycle.running && !input.lifecycle.suspended {
            self.mixer.lock().unwrap().cue_spatial(
                "blaster",
                prepared.pcm,
                prepared.gain,
                prepared.matrix,
            );
        }
    }
    pub fn prepare_computer(
        &mut self,
        input: &RoomInput,
        severity: &str,
    ) -> Option<PreparedComputer> {
        let spec = input
            .config
            .computer_message
            .as_ref()?
            .for_severity(severity)?;
        Some(PreparedComputer {
            pcm: self.asset(&spec.file)?,
            gain: spec.volume,
        })
    }
    pub fn play_computer(&mut self, input: &RoomInput, ready: bool, prepared: PreparedComputer) {
        if ready && input.lifecycle.running && !input.lifecycle.suspended {
            self.mixer
                .lock()
                .unwrap()
                .cue("computer", prepared.pcm, "alerts", prepared.gain, None);
        }
    }
}
