//! One process-owned Viewscreen audio endpoint. State and preferences are local
//! presentation; no audio event enters a save, journal, replay or sim digest.
pub mod decoder;
#[cfg(feature = "host")]
mod device;
pub mod engine;
pub mod hrtf;
pub mod mix;
pub mod player;
pub mod spatial;
pub mod store;
pub mod visual;

use super::{
    bridge_profile::{BridgeProfile, ValidatedProfile},
    host_lobby::{HostLobbyBridgeResource, HostLobbyRecord},
    viewscreen_presentation::ViewscreenPresentationStore,
};
use crate::authoritative::{DeclareState, StateClass};
use crate::{
    audio_config::build_audio_payload,
    console_bridge::{AudioCueEvent, HudStateChanged},
    core::{codec, messages::GamePhase},
    entities::spawner::ShipAudioSection,
    server::audio_lifecycle::RoomAudioLifecycle,
    server_app::LocalShip,
    world::config::WorldConfig,
};
use bevy::prelude::*;
use mix::{AudioMix, Bus};
use player::{RoomInput, RoomPlayer};
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone, Debug, Serialize)]
pub struct OutputChoice {
    pub id: String,
    pub label: String,
    pub available: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct NativeAudioState {
    pub room: bool,
    pub mix: AudioMix,
    pub categories: Vec<&'static str>,
    pub status: &'static str,
    pub test: &'static str,
    pub persistence: &'static str,
    pub hardware_persistence: &'static str,
    pub profile_override: bool,
    pub output: Option<String>,
    pub devices: Vec<OutputChoice>,
    pub detail: String,
    pub asset_failures: Vec<String>,
}
impl Default for NativeAudioState {
    fn default() -> Self {
        Self {
            room: true,
            mix: AudioMix::default(),
            categories: vec!["music", "ambience", "effects", "alerts"],
            status: "loading",
            test: "idle",
            persistence: "unavailable",
            hardware_persistence: "unavailable",
            profile_override: false,
            output: None,
            devices: Vec::new(),
            detail: String::new(),
            asset_failures: Vec::new(),
        }
    }
}
#[derive(Clone)]
struct Control {
    input: RoomInput,
    output: Option<String>,
    routing_error: Option<String>,
    retry: u64,
    test: u64,
    test_at: Option<Instant>,
    alert_at: Option<Instant>,
    blaster: Option<(Instant, [f32; 3])>,
    computer: Option<(Instant, String)>,
    quit: bool,
}
impl Control {
    fn take_computer(&mut self, now: Instant) -> Option<String> {
        self.computer.take().and_then(|(at, severity)| {
            (now.saturating_duration_since(at) <= std::time::Duration::from_millis(250))
                .then_some(severity)
        })
    }
}
/// Requests contain only current settings/state and one expiring deliberate test.
/// They are overwritten, never an audio event queue.
#[derive(Resource)]
pub struct NativeRoomAudio {
    pub visuals: visual::NativeAudioVisual,
    control: Arc<Mutex<Control>>,
    state: Arc<Mutex<NativeAudioState>>,
    mixer: Arc<Mutex<engine::Mixer>>,
    profile: BridgeProfile,
    preferences: Option<ViewscreenPresentationStore>,
    hardware: Option<store::HardwareStore>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl NativeRoomAudio {
    pub fn new(authored: Option<&ValidatedProfile>, start_device: bool) -> Self {
        Self::with_stores(
            authored,
            start_device,
            ViewscreenPresentationStore::user(),
            store::HardwareStore::user(),
        )
    }
    pub fn with_stores(
        authored: Option<&ValidatedProfile>,
        start_device: bool,
        preferences: Option<ViewscreenPresentationStore>,
        hardware: Option<store::HardwareStore>,
    ) -> Self {
        let (mix, persistence) = preferences
            .as_ref()
            .map(|store| store.load_audio())
            .unwrap_or((AudioMix::default(), "unavailable"));
        let saved = hardware
            .as_ref()
            .map(|store| store.load())
            .unwrap_or_else(|| Ok(BridgeProfile::empty()));
        let hardware_persistence = if hardware.is_none() {
            "unavailable"
        } else if saved.is_err() {
            "corrupt"
        } else {
            "saved"
        };
        let profile_override = authored.is_some_and(|profile| {
            profile
                .media
                .surfaces
                .iter()
                .any(|entry| entry.surface == "viewscreen")
        });
        let profile = store::merge_media(
            &saved.clone().unwrap_or_else(|_| BridgeProfile::empty()),
            authored.map(|profile| &profile.media),
        );
        let route = store::room_output(&profile);
        let routing_error = if profile_override {
            route.clone().err()
        } else {
            saved.err().or_else(|| route.clone().err())
        };
        let output = route.unwrap_or_default();
        let state = Arc::new(Mutex::new(NativeAudioState {
            mix,
            persistence,
            hardware_persistence,
            profile_override,
            output: output.clone(),
            ..Default::default()
        }));
        let control = Arc::new(Mutex::new(Control {
            input: RoomInput {
                lifecycle: crate::console_bridge::AudioLifecycleState {
                    suspended: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            output,
            routing_error,
            retry: 0,
            test: 0,
            test_at: None,
            alert_at: None,
            blaster: None,
            computer: None,
            quit: false,
        }));
        let player = RoomPlayer::default();
        let mixer = player.mixer.clone();
        mixer.lock().unwrap().mix = mix;
        #[cfg(feature = "host")]
        let worker = if start_device {
            let control = control.clone();
            let state = state.clone();
            std::thread::Builder::new()
                .name("phoenix-room-audio".into())
                .spawn(move || device::run(control, state, player))
                .ok()
        } else {
            None
        };
        #[cfg(not(feature = "host"))]
        let worker = {
            let _ = (start_device, player);
            None
        };
        if worker.is_none() {
            state.lock().unwrap().status = "unavailable";
        }
        Self {
            visuals: visual::NativeAudioVisual::default(),
            control,
            state,
            mixer,
            profile,
            preferences,
            hardware,
            worker,
        }
    }
    pub fn snapshot(&self) -> NativeAudioState {
        self.state.lock().unwrap().clone()
    }
    pub fn apply_input(&self, input: RoomInput) {
        let mut control = self.control.lock().unwrap();
        if control.input == input {
            return;
        }
        self.visuals.update(&input);
        if control.input.lifecycle.generation != input.lifecycle.generation
            || control.input.config != input.config
            || input.lifecycle.suspended
        {
            self.mixer.lock().unwrap().stop_all();
            control.test_at = None;
            control.alert_at = None;
            control.blaster = None;
            control.computer = None;
        } else if control.input.red_alert != input.red_alert {
            control.alert_at = input.red_alert.then(Instant::now);
        }
        control.input = input;
    }
    pub fn blaster(&self, position: [f32; 3]) {
        let mut control = self.control.lock().unwrap();
        if control.input.lifecycle.running && !control.input.lifecycle.suspended {
            // One short-lived current request. Repetition coalesces; a blocked
            // decoder/device never accumulates a playback queue.
            control.blaster = Some((Instant::now(), position));
            self.visuals.blaster(position);
        }
    }
    pub fn computer_message(&self, severity: &str) {
        let mut control = self.control.lock().unwrap();
        if control.input.lifecycle.running && !control.input.lifecycle.suspended {
            if let Some(spec) = control
                .input
                .config
                .computer_message
                .as_ref()
                .and_then(|config| config.for_severity(severity))
            {
                if self.mixer.lock().unwrap().mix.gain("alerts", spec.volume) > 0.0 {
                    // One expiring occurrence, even for repeated equal severity.
                    // Current text travels independently in the normal HUD.
                    control.computer = Some((Instant::now(), severity.to_owned()));
                }
            }
        }
    }
    pub fn command(&mut self, record: &HostLobbyRecord) {
        match record {
            HostLobbyRecord::SetAudioBus {
                bus,
                level_percent,
                muted,
            } => {
                let mut state = self.state.lock().unwrap();
                if bus != "master" && !state.categories.contains(&bus.as_str()) {
                    return;
                }
                if !state.mix.set(
                    bus,
                    Bus {
                        level: (*level_percent).min(100) as f32 / 100.0,
                        muted: *muted,
                    },
                ) {
                    return;
                }
                self.mixer.lock().unwrap().set_mix(state.mix);
                if state.mix.gain("alerts", 1.0) == 0.0 {
                    self.control.lock().unwrap().computer = None;
                }
                state.persistence = if self
                    .preferences
                    .as_ref()
                    .is_some_and(|store| store.save_audio(state.mix).is_ok())
                {
                    "saved"
                } else {
                    "unavailable"
                };
            }
            HostLobbyRecord::ResetAudioMix => {
                let mut state = self.state.lock().unwrap();
                state.mix = AudioMix::default();
                self.mixer.lock().unwrap().set_mix(state.mix);
                state.persistence = if self
                    .preferences
                    .as_ref()
                    .is_some_and(|store| store.save_audio(state.mix).is_ok())
                {
                    "saved"
                } else {
                    "unavailable"
                };
            }
            HostLobbyRecord::SelectAudioOutput { output } => {
                let selected = store::select_room(&self.profile, output.clone());
                match selected {
                    Ok(profile) => {
                        self.profile = profile;
                        let mut state = self.state.lock().unwrap();
                        state.output = output.clone();
                        state.status = "loading";
                        state.hardware_persistence = if self
                            .hardware
                            .as_ref()
                            .is_some_and(|store| store.save(&self.profile).is_ok())
                        {
                            "saved"
                        } else {
                            "unavailable"
                        };
                        let mut control = self.control.lock().unwrap();
                        control.output = output.clone();
                        control.routing_error = None;
                        control.retry = control.retry.wrapping_add(1);
                        control.test_at = None;
                        control.alert_at = None;
                        control.blaster = None;
                        control.computer = None;
                        self.mixer.lock().unwrap().stop_all();
                    }
                    Err(error) => {
                        self.state.lock().unwrap().detail = error;
                    }
                }
            }
            HostLobbyRecord::RetryAudioOutput => {
                self.state.lock().unwrap().status = "loading";
                let mut control = self.control.lock().unwrap();
                control.retry = control.retry.wrapping_add(1);
                control.test_at = None;
                control.alert_at = None;
                control.blaster = None;
                control.computer = None;
                self.mixer.lock().unwrap().stop_all();
            }
            HostLobbyRecord::TestAudioOutput => {
                self.state.lock().unwrap().test = "loading";
                let mut control = self.control.lock().unwrap();
                control.test = control.test.wrapping_add(1);
                control.test_at = Some(Instant::now());
            }
            _ => {}
        }
    }
}
impl Drop for NativeRoomAudio {
    fn drop(&mut self) {
        self.control.lock().unwrap().quit = true;
        self.mixer.lock().unwrap().stop_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub struct NativeRoomAudioPlugin;
impl Plugin for NativeRoomAudioPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::server::audio::ServerAudioPlugin>() {
            app.add_plugins(crate::server::audio::ServerAudioPlugin);
        }
        app.add_systems(
            PostUpdate,
            update_room.after(crate::server::audio_lifecycle::publish_audio_lifecycle),
        );
        app.declare_state::<NativeRoomAudio>(StateClass::Presentation, "native-room-audio");
    }
}
fn update_room(
    audio: Option<Res<NativeRoomAudio>>,
    lifecycle: Res<RoomAudioLifecycle>,
    phase: Res<State<GamePhase>>,
    ship: Query<&ShipAudioSection, With<LocalShip>>,
    world: Option<Res<WorldConfig>>,
    mut hud: MessageReader<HudStateChanged>,
    mut cues: MessageReader<AudioCueEvent>,
    forcefield: Option<Res<crate::server::audio::ForcefieldAudioState>>,
    mut current: Local<Option<crate::core::messages::ViewscreenHudState>>,
    bridge: Option<Res<HostLobbyBridgeResource>>,
) {
    let Some(audio) = audio else {
        return;
    };
    for event in hud.read() {
        if let Ok(value) = codec::decode_native_audio_hud(&event.json) {
            *current = Some(value);
        }
    }
    let running = *phase.get() == GamePhase::InProgress;
    let config = if running {
        build_audio_payload(
            ship.single().ok().map(|ship| &ship.0),
            world.as_ref().and_then(|world| world.audio.as_ref()),
        )
    } else {
        Default::default()
    };
    audio.apply_input(RoomInput {
        lifecycle: lifecycle.state.clone(),
        config,
        menu: matches!(phase.get(), GamePhase::Lobby | GamePhase::Loading),
        red_alert: current.as_ref().is_some_and(|hud| hud.red_alert),
        thrust: current.as_ref().map_or(0.0, |hud| hud.engine_thrust),
        phaser: current.as_ref().is_some_and(|hud| hud.phaser_firing),
        forcefield: ship
            .single()
            .ok()
            .and_then(|ship| ship.0.forcefield.as_ref())
            .map_or(0.0, |spec| {
                crate::audio_config::forcefield_volume(
                    forcefield.map_or(0.0, |state| state.intensity),
                    spec.base_volume,
                    spec.spike_volume,
                )
            }),
    });
    for event in cues.read() {
        if let Ok(cue) = codec::decode_native_audio_cue(&event.json) {
            if cue.kind == "blaster" {
                audio.blaster([cue.x, cue.y, cue.z]);
            } else if cue.kind == "computer_message" {
                if let Some(severity) = cue.severity {
                    audio.computer_message(&severity);
                }
            }
        }
    }
    if let Some(bridge) = bridge {
        if let Ok(json) = codec::encode_native_audio_state(&audio.snapshot()) {
            bridge.0.push_audio(json);
        }
    }
}

#[cfg(test)]
mod combat_tests;
#[cfg(test)]
mod computer_tests;
#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Random UUIDs isolate temporary test directories.
mod tests;
