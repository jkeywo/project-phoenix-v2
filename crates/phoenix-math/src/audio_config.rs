// Pure-Rust server audio configuration and envelope math.
//
// Audio playback lives in the host page's JS (`server.html`) — Bevy audio was
// tried and reverted in-browser. What Rust owns is everything that benefits
// from being typed and testable:
//
// - **Config parsing.** Filenames and every tuning parameter come from TOML.
//   Red-alert music/siren live on the world (`[audio.red_alert]` in
//   `assets/worlds/*.toml`); every other sound lives on the ship entity
//   (`[audio.*]` in `assets/entities/*.toml`) and is read from the LocalShip.
// - **The forcefield envelope.** Spike-on-damage then decay, computed here so
//   the five tuning numbers never have to cross the bridge.
// - **Listener-relative geometry.** The blaster is positional; Rust rotates
//   world coordinates into the ship's frame so JS can drop them straight into
//   a Web Audio `PannerNode` with the listener parked at the origin.
//
// `AudioConfigPayload` is the wire shape sent to JS once, on game start.
//
// This module has no Bevy dependency — it is fully unit-testable on native.

use serde::{Deserialize, Serialize};

// ── Sound sections (ship entity TOML) ─────────────────────────────────────

/// Looping ambient bed. From `[audio.ambient]` on the ship entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmbientAudio {
    /// Asset path, relative to the page root (e.g. `assets/sounds/Ambient.mp3`).
    pub file: String,
    /// Starting volume fraction, 0.0–1.0.
    pub volume: f32,
}

/// Looping engine bed whose volume tracks helm thrust.
/// From `[audio.engine]` on the ship entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineAudio {
    pub file: String,
    /// Volume contribution at full thrust: `volume = idle_volume + thrust * this`.
    pub volume_at_full_thrust: f32,
    /// Volume at zero thrust.
    pub idle_volume: f32,
}

/// Looping phaser sound, played while a beam is active.
/// From `[audio.phaser_loop]` on the ship entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaserLoopAudio {
    pub file: String,
    pub volume: f32,
}

/// Web Audio `PannerNode.distanceModel`. Serialises to the exact strings the
/// Web Audio API expects, so JS can assign the value without a lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistanceModel {
    Inverse,
    Linear,
    Exponential,
}

/// Web Audio `PannerNode.panningModel`. Serialises to the exact strings the
/// Web Audio API expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanningModel {
    #[serde(rename = "equalpower")]
    EqualPower,
    #[serde(rename = "HRTF")]
    Hrtf,
}

/// Positional one-shot fired on every blaster shot (player *and* NPC).
/// From `[audio.blaster]` on the ship entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlasterAudio {
    pub file: String,
    pub volume: f32,
    /// `PannerNode.refDistance` — world units at which volume is unattenuated.
    pub ref_distance: f32,
    /// `PannerNode.maxDistance`.
    pub max_distance: f32,
    /// `PannerNode.rolloffFactor` — how sharply volume falls with distance.
    pub rolloff_factor: f32,
    pub distance_model: DistanceModel,
    pub panning_model: PanningModel,
}

/// Which field of `DamageTaken` drives the forcefield spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForcefieldSource {
    Shield,
    Hull,
    Total,
}

/// Continuous forcefield bed whose volume spikes on damage then decays.
/// From `[audio.forcefield]` on the ship entity.
///
/// The file crosses the bridge to JS; every other field stays server-side and
/// feeds [`forcefield_spike`] / [`forcefield_decay`] / [`forcefield_volume`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForcefieldAudio {
    pub file: String,
    /// Level of the bed at intensity 0 — the idle hum.
    pub base_volume: f32,
    /// Level at intensity 1 — a full-strength hit.
    pub spike_volume: f32,
    /// Damage below this many HP produces no spike at all.
    pub damage_threshold: f32,
    /// Damage at or above this many HP produces a full (intensity 1.0) spike.
    pub damage_full_spike: f32,
    /// Intensity units shed per second, decaying back toward the bed.
    pub decay_rate_per_sec: f32,
    /// Which `DamageTaken` field to read.
    pub source: ForcefieldSource,
}

/// One severity's ship's-computer tone: file plus a flat playback volume. Not
/// positional — see [`AudioCue::computer_message`] — so no `PannerNode`
/// parameters, unlike [`BlasterAudio`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerMessageCue {
    pub file: String,
    pub volume: f32,
}

/// Ship's-computer message tones, keyed by severity (issue #1342). From
/// `[audio.computer_message]` on the ship entity.
///
/// Every severity is optional, exactly like every section of
/// [`ShipAudioConfig`] — a severity with no cue configured plays no sound at
/// all when a message of that severity shows, rather than falling back to
/// some other severity's tone or a hardcoded default.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerMessageAudio {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<ComputerMessageCue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisory: Option<ComputerMessageCue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<ComputerMessageCue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical: Option<ComputerMessageCue>,
}

impl ComputerMessageAudio {
    /// The configured cue for `severity`, or `None` if that severity's
    /// section was never authored — "missing configuration is silent" applied
    /// per-severity rather than to the section as a whole.
    pub fn for_severity(&self, severity: &str) -> Option<&ComputerMessageCue> {
        match severity {
            "info" => self.info.as_ref(),
            "advisory" => self.advisory.as_ref(),
            "warning" => self.warning.as_ref(),
            "critical" => self.critical.as_ref(),
            _ => None,
        }
    }
}

/// All ship-borne audio. From `[audio]` on the ship entity TOML.
///
/// Every section is optional — an absent section means that sound is silent.
/// But a section that *is* present must specify all of its fields: there are
/// no hidden defaults, because the whole point is that designers tune this
/// file rather than recompile.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipAudioConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<AmbientAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phaser_loop: Option<PhaserLoopAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blaster: Option<BlasterAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forcefield: Option<ForcefieldAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computer_message: Option<ComputerMessageAudio>,
}

// ── World audio (world TOML) ──────────────────────────────────────────────

/// Red-alert audio. The siren is a one-shot fired on the false→true edge; the
/// music loops underneath for as long as the alert is active.
/// From `[audio.red_alert]` in the world TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedAlertAudio {
    pub siren_file: String,
    pub siren_volume: f32,
    pub music_file: String,
    pub music_volume: f32,
}

/// World-level audio. From `[audio]` in `assets/worlds/*.toml`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldAudioConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red_alert: Option<RedAlertAudio>,
}

// ── Wire payload ──────────────────────────────────────────────────────────

/// The forcefield's JS-visible half: just the file. The envelope parameters
/// stay server-side — Rust computes the level and pushes it as a bare float.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForcefieldWire {
    pub file: String,
}

/// Everything JS needs to build the audio graph, merged from the local ship's
/// config and the world's. Encoded by `codec::encode_audio_config` and pushed
/// once on game start via the `AudioConfigChanged` bridge message.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AudioConfigPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambient: Option<AmbientAudio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineAudio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phaser_loop: Option<PhaserLoopAudio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blaster: Option<BlasterAudio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forcefield: Option<ForcefieldWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub red_alert: Option<RedAlertAudio>,
    /// Ship's-computer tones by severity (issue #1342). Unlike forcefield,
    /// there is no envelope to hide — every field here is exactly what JS
    /// needs — so the ship config crosses the bridge unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer_message: Option<ComputerMessageAudio>,
}

/// A one-shot audio cue. Encoded by `codec::encode_audio_cue` and pushed via
/// the `AudioCueEvent` bridge message.
///
/// Two shapes share one struct, discriminated by `kind`:
///
/// * `"blaster"` is **positional** — `x`/`y`/`z` are listener-relative (see
///   [`listener_relative`]), so JS leaves the Web Audio listener at the
///   origin facing −Z and assigns these straight to
///   `PannerNode.positionX/Y/Z`. `severity` is unused (`None`).
/// * `"computer_message"` (issue #1342) is **not** positional — a ship's-
///   computer tone plays the same everywhere on the bridge, so `x`/`y`/`z`
///   are zeroed and JS skips the panner node entirely; `severity` names which
///   of `AudioConfigPayload::computer_message`'s already-pushed cues to play.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioCue {
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Severity word (`"info"`/`"advisory"`/`"warning"`/`"critical"`) for a
    /// `"computer_message"` cue. `None` for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

impl AudioCue {
    /// A blaster report at the given listener-relative position.
    pub fn blaster(pos: [f32; 3]) -> Self {
        Self {
            kind: "blaster".to_string(),
            x: pos[0],
            y: pos[1],
            z: pos[2],
            severity: None,
        }
    }

    /// A ship's-computer tone for `severity`. Not positional (issue #1342).
    pub fn computer_message(severity: &str) -> Self {
        Self {
            kind: "computer_message".to_string(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            severity: Some(severity.to_string()),
        }
    }
}

/// Merge the local ship's audio config and the world's into the JS payload.
///
/// Either side may be absent (a ship with no `[audio]` block, a world with no
/// `[audio.red_alert]`); the corresponding sounds are simply omitted and JS
/// skips them.
pub fn build_audio_payload(
    ship: Option<&ShipAudioConfig>,
    world: Option<&WorldAudioConfig>,
) -> AudioConfigPayload {
    AudioConfigPayload {
        ambient: ship.and_then(|s| s.ambient.clone()),
        engine: ship.and_then(|s| s.engine.clone()),
        phaser_loop: ship.and_then(|s| s.phaser_loop.clone()),
        blaster: ship.and_then(|s| s.blaster.clone()),
        forcefield: ship.and_then(|s| {
            s.forcefield.as_ref().map(|f| ForcefieldWire {
                file: f.file.clone(),
            })
        }),
        red_alert: world.and_then(|w| w.red_alert.clone()),
        computer_message: ship.and_then(|s| s.computer_message.clone()),
    }
}

// ── Forcefield envelope ───────────────────────────────────────────────────

/// Spike intensity (0.0–1.0) for a single damage event, or `None` when the
/// damage is below `threshold` and should not disturb the bed at all.
///
/// Intensity ramps linearly from 0.0 at `threshold` to 1.0 at `full_spike_hp`
/// and clamps above that. A degenerate config where `full_spike_hp <=
/// threshold` yields a full spike for any qualifying hit rather than dividing
/// by zero.
pub fn forcefield_spike(damage_hp: f32, threshold: f32, full_spike_hp: f32) -> Option<f32> {
    if damage_hp < threshold {
        return None;
    }
    if full_spike_hp <= threshold {
        return Some(1.0);
    }
    Some(((damage_hp - threshold) / (full_spike_hp - threshold)).clamp(0.0, 1.0))
}

/// One decay step. Intensity sheds `decay_rate` units per second and never
/// goes negative.
pub fn forcefield_decay(intensity: f32, dt: f32, decay_rate: f32) -> f32 {
    (intensity - dt * decay_rate).max(0.0)
}

/// Final element volume for a given intensity: lerp `base`→`spike`, clamped to
/// 0.0–1.0.
///
/// The clamp is load-bearing, not defensive: `HTMLMediaElement.volume` throws
/// `IndexSizeError` outside that range, and the bounds come from
/// designer-edited TOML.
pub fn forcefield_volume(intensity: f32, base: f32, spike: f32) -> f32 {
    (base + (spike - base) * intensity).clamp(0.0, 1.0)
}

// ── Listener-relative geometry ────────────────────────────────────────────

/// Rotate a world-space XZ position into the listener's frame.
///
/// The **ship** is the listener, not the camera — `cinematic_camera` can
/// detach the camera from the hull, and sounds should stay anchored to the
/// crew.
///
/// The sim's heading convention is fixed by the movement integration (see
/// `ai::server`): `x += speed * yaw.sin() * dt; z -= speed * yaw.cos() * dt`.
/// So world-forward is `(sin yaw, 0, −cos yaw)` and world-right is
/// `(cos yaw, 0, sin yaw)`. Web Audio's listener faces −Z with +X to the
/// right, hence the negated forward component in the returned vector.
///
/// Returns `[right, 0.0, -forward]` — ready for `PannerNode.positionX/Y/Z`.
// Presentation-only audio panning (JS-side playback): never feeds simulation
// state, so std transcendentals are fine (issue #908, simmath.rs).
#[allow(clippy::disallowed_methods)]
pub fn listener_relative(
    listener_x: f32,
    listener_z: f32,
    listener_yaw: f32,
    sound_x: f32,
    sound_z: f32,
) -> [f32; 3] {
    let dx = sound_x - listener_x;
    let dz = sound_z - listener_z;
    let (sin_yaw, cos_yaw) = listener_yaw.sin_cos();
    let right = dx * cos_yaw + dz * sin_yaw;
    let forward = dx * sin_yaw - dz * cos_yaw;
    [right, 0.0, -forward]
}

#[cfg(test)]
#[path = "audio_config_tests.rs"]
mod tests;
