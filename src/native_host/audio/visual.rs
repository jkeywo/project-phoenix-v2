//! Live visual equivalents cross into the one HUD independently of audio-device
//! success. Two expiring current family slots; no list, history or catch-up.
use super::player::RoomInput;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisualCue {
    Lifecycle {
        active: bool,
    },
    Beam {
        active: bool,
    },
    Blaster {
        x: f32,
        y: f32,
        z: f32,
    },
    Impact,
    Authored {
        equivalent: Option<crate::sound_cues::Equivalent>,
    },
}
#[derive(Clone)]
struct CurrentCue {
    serial: u64,
    at: Instant,
    cue: VisualCue,
}
#[derive(Default)]
struct Current {
    generation: u64,
    active: bool,
    beam: bool,
    serial: u64,
    level: Option<f32>,
    forcefield: Option<crate::audio_config::ForcefieldWire>,
    shot: Option<CurrentCue>,
    impact: Option<CurrentCue>,
    authored: Option<CurrentCue>,
}
#[derive(Clone, Default)]
pub struct NativeAudioVisual(Arc<Mutex<Current>>);
impl NativeAudioVisual {
    pub fn update(&self, input: &RoomInput) {
        let mut current = self.0.lock().unwrap();
        let active = input.lifecycle.running && !input.lifecycle.suspended;
        if current.generation != input.lifecycle.generation || current.active != active {
            current.generation = input.lifecycle.generation;
            current.active = active;
            current.shot = None;
            current.impact = None;
            current.authored = None;
            current.level = None;
        }
        if current.forcefield != input.config.forcefield {
            // Replacing the configured bed is not a new impact (the browser
            // also seeds its envelope comparison again when config changes).
            current.forcefield = input.config.forcefield.clone();
            current.level = None;
            current.impact = None;
        }
        if active
            && input.config.forcefield.is_some()
            && current
                .level
                .is_some_and(|before| input.forcefield > before)
        {
            current.serial = current.serial.wrapping_add(1);
            current.impact = Some(CurrentCue {
                serial: current.serial,
                at: Instant::now(),
                cue: VisualCue::Impact,
            });
        }
        current.level = Some(input.forcefield);
        current.beam = active && input.phaser;
    }
    pub fn blaster(&self, [x, y, z]: [f32; 3]) {
        if [x, y, z].iter().any(|v| !v.is_finite()) {
            return;
        }
        let mut current = self.0.lock().unwrap();
        if !current.active {
            return;
        }
        current.serial = current.serial.wrapping_add(1);
        current.shot = Some(CurrentCue {
            serial: current.serial,
            at: Instant::now(),
            cue: VisualCue::Blaster { x, y, z },
        });
    }
    pub fn authored(&self, equivalent: Option<crate::sound_cues::Equivalent>) {
        let mut current = self.0.lock().unwrap();
        current.authored = None;
        if !current.active {
            return;
        }
        current.serial = current.serial.wrapping_add(1);
        current.authored = Some(CurrentCue {
            serial: current.serial,
            at: Instant::now(),
            cue: VisualCue::Authored { equivalent },
        });
    }
}

/// Each document seeds its first/hidden/reloaded observation silently. A failed
/// push consumes the occurrence: playback/presentation never retries old cues.
#[derive(Default)]
pub struct VisualReader {
    boundary: Option<(u64, bool)>,
    visible: bool,
    serial: u64,
    beam: Option<bool>,
}
impl VisualReader {
    /// Current lifecycle/beam state can retry; transient occurrence serials stay
    /// consumed and the retry's fresh baseline cannot replay them.
    pub fn retry_current(&mut self) {
        self.boundary = None;
        self.beam = None;
    }
    pub fn read(
        &mut self,
        shared: &NativeAudioVisual,
        ready: bool,
        visible: bool,
        now: Instant,
    ) -> Vec<VisualCue> {
        let mut current = shared.0.lock().unwrap();
        if current
            .shot
            .as_ref()
            .is_some_and(|cue| now.saturating_duration_since(cue.at) > Duration::from_millis(250))
        {
            current.shot = None;
        }
        if current
            .impact
            .as_ref()
            .is_some_and(|cue| now.saturating_duration_since(cue.at) > Duration::from_millis(250))
        {
            current.impact = None;
        }
        let visible = ready && visible;
        if current
            .authored
            .as_ref()
            .is_some_and(|cue| now.saturating_duration_since(cue.at) > Duration::from_millis(250))
        {
            current.authored = None;
        }
        let boundary = (current.generation, current.active);
        let reset = self.boundary != Some(boundary) || self.visible != visible;
        self.boundary = Some(boundary);
        self.visible = visible;
        let mut cues = Vec::new();
        if reset && ready {
            cues.push(VisualCue::Lifecycle {
                active: visible && current.active,
            });
        }
        let beam = visible && current.beam;
        if ready && (reset || self.beam != Some(beam)) {
            cues.push(VisualCue::Beam { active: beam });
        }
        self.beam = Some(beam);
        if visible && current.active && !reset {
            for cue in [&current.impact, &current.shot, &current.authored]
                .into_iter()
                .flatten()
            {
                if cue.serial > self.serial {
                    cues.push(cue.cue.clone());
                }
            }
        }
        self.serial = current.serial;
        cues
    }
}
