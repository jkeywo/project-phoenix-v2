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
}

/// A one-shot positional audio cue. Encoded by `codec::encode_audio_cue` and
/// pushed via the `AudioCueEvent` bridge message.
///
/// `x`/`y`/`z` are **listener-relative** (see [`listener_relative`]), so JS
/// leaves the Web Audio listener at the origin facing −Z and assigns these
/// straight to `PannerNode.positionX/Y/Z`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioCue {
    /// Cue discriminator. Currently only `"blaster"`.
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl AudioCue {
    /// A blaster report at the given listener-relative position.
    pub fn blaster(pos: [f32; 3]) -> Self {
        Self {
            kind: "blaster".to_string(),
            x: pos[0],
            y: pos[1],
            z: pos[2],
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
mod tests {
    use super::*;

    const SHIP_TOML: &str = r#"
[ambient]
file   = "assets/sounds/Ambient.mp3"
volume = 0.25

[engine]
file                  = "assets/sounds/Engine.mp3"
volume_at_full_thrust = 0.15
idle_volume           = 0.0

[phaser_loop]
file   = "assets/sounds/PhaserLoop.mp3"
volume = 0.5

[blaster]
file           = "assets/sounds/Blaster.mp3"
volume         = 0.9
ref_distance   = 30.0
max_distance   = 800.0
rolloff_factor = 1.2
distance_model = "inverse"
panning_model  = "equalpower"

[forcefield]
file               = "assets/sounds/ForcefieldHit.mp3"
base_volume        = 0.06
spike_volume       = 0.80
damage_threshold   = 1.0
damage_full_spike  = 30.0
decay_rate_per_sec = 1.5
source             = "shield"
"#;

    // ── Config parsing ────────────────────────────────────────────────

    #[test]
    fn ship_audio_config_parses_every_section() {
        let cfg: ShipAudioConfig = toml::from_str(SHIP_TOML).expect("parses");
        assert_eq!(cfg.ambient.as_ref().unwrap().file, "assets/sounds/Ambient.mp3");
        assert_eq!(cfg.ambient.as_ref().unwrap().volume, 0.25);
        assert_eq!(cfg.engine.as_ref().unwrap().volume_at_full_thrust, 0.15);
        assert_eq!(cfg.phaser_loop.as_ref().unwrap().volume, 0.5);

        let blaster = cfg.blaster.as_ref().unwrap();
        assert_eq!(blaster.ref_distance, 30.0);
        assert_eq!(blaster.distance_model, DistanceModel::Inverse);
        assert_eq!(blaster.panning_model, PanningModel::EqualPower);

        let ff = cfg.forcefield.as_ref().unwrap();
        assert_eq!(ff.damage_threshold, 1.0);
        assert_eq!(ff.source, ForcefieldSource::Shield);
    }

    #[test]
    fn ship_audio_config_round_trips_via_toml() {
        let cfg: ShipAudioConfig = toml::from_str(SHIP_TOML).expect("parses");
        let encoded = toml::to_string(&cfg).expect("serialises");
        let decoded: ShipAudioConfig = toml::from_str(&encoded).expect("re-parses");
        assert_eq!(cfg, decoded);
    }

    #[test]
    fn ship_audio_omitted_sections_are_none() {
        let cfg: ShipAudioConfig = toml::from_str(
            r#"
[ambient]
file   = "assets/sounds/Ambient.mp3"
volume = 0.25
"#,
        )
        .expect("partial config is legal");
        assert!(cfg.ambient.is_some());
        assert!(cfg.engine.is_none());
        assert!(cfg.blaster.is_none());
        assert!(cfg.forcefield.is_none());
    }

    #[test]
    fn ship_audio_empty_table_is_all_none() {
        let cfg: ShipAudioConfig = toml::from_str("").expect("empty is legal");
        assert_eq!(cfg, ShipAudioConfig::default());
    }

    #[test]
    fn ship_audio_config_rejects_unknown_field() {
        let err = toml::from_str::<ShipAudioConfig>(
            r#"
[ambient]
file   = "a.mp3"
volume = 0.25
loudness = 3.0
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("loudness"), "got: {err}");
    }

    #[test]
    fn ship_audio_config_rejects_unknown_section() {
        assert!(toml::from_str::<ShipAudioConfig>(
            r#"
[tractor_beam]
file = "a.mp3"
"#,
        )
        .is_err());
    }

    #[test]
    fn present_section_requires_all_fields() {
        // No hidden defaults: a half-specified section is an error, not a
        // silent fallback to a hardcoded volume.
        let err = toml::from_str::<ShipAudioConfig>(
            r#"
[ambient]
file = "assets/sounds/Ambient.mp3"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("volume"), "got: {err}");
    }

    #[test]
    fn world_audio_config_parses_red_alert() {
        let cfg: WorldAudioConfig = toml::from_str(
            r#"
[red_alert]
siren_file   = "assets/sounds/red_alert_siren.ogg"
siren_volume = 0.7
music_file   = "assets/sounds/last_stand_in_space_looped.ogg"
music_volume = 0.35
"#,
        )
        .expect("parses");
        let ra = cfg.red_alert.as_ref().unwrap();
        assert_eq!(ra.siren_file, "assets/sounds/red_alert_siren.ogg");
        assert_eq!(ra.music_volume, 0.35);
    }

    #[test]
    fn panning_and_distance_models_use_web_audio_spelling() {
        // These serialise straight into PannerNode properties, so the exact
        // strings matter.
        assert_eq!(
            serde_json::to_string(&PanningModel::Hrtf).unwrap(),
            "\"HRTF\""
        );
        assert_eq!(
            serde_json::to_string(&PanningModel::EqualPower).unwrap(),
            "\"equalpower\""
        );
        assert_eq!(
            serde_json::to_string(&DistanceModel::Exponential).unwrap(),
            "\"exponential\""
        );
    }

    // ── Payload merge ─────────────────────────────────────────────────

    #[test]
    fn build_payload_merges_ship_and_world() {
        let ship: ShipAudioConfig = toml::from_str(SHIP_TOML).unwrap();
        let world = WorldAudioConfig {
            red_alert: Some(RedAlertAudio {
                siren_file: "s.ogg".into(),
                siren_volume: 0.7,
                music_file: "m.ogg".into(),
                music_volume: 0.35,
            }),
        };
        let p = build_audio_payload(Some(&ship), Some(&world));
        assert_eq!(p.ambient.unwrap().file, "assets/sounds/Ambient.mp3");
        assert_eq!(p.red_alert.unwrap().music_file, "m.ogg");
    }

    #[test]
    fn build_payload_sends_only_the_forcefield_file() {
        // The envelope parameters must not cross the bridge — Rust owns them.
        let ship: ShipAudioConfig = toml::from_str(SHIP_TOML).unwrap();
        let p = build_audio_payload(Some(&ship), None);
        assert_eq!(
            p.forcefield,
            Some(ForcefieldWire {
                file: "assets/sounds/ForcefieldHit.mp3".into()
            })
        );
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("decay_rate_per_sec"), "envelope leaked: {json}");
        assert!(!json.contains("damage_threshold"), "envelope leaked: {json}");
    }

    #[test]
    fn build_payload_tolerates_both_sides_absent() {
        assert_eq!(
            build_audio_payload(None, None),
            AudioConfigPayload::default()
        );
    }

    #[test]
    fn payload_omits_absent_sounds_from_json() {
        let json = serde_json::to_string(&AudioConfigPayload::default()).unwrap();
        assert_eq!(json, "{}");
    }

    // ── Forcefield envelope ───────────────────────────────────────────

    #[test]
    fn forcefield_spike_below_threshold_is_none() {
        assert_eq!(forcefield_spike(0.5, 1.0, 30.0), None);
    }

    #[test]
    fn forcefield_spike_at_full_spike_is_one() {
        assert_eq!(forcefield_spike(30.0, 1.0, 30.0), Some(1.0));
    }

    #[test]
    fn forcefield_spike_scales_linearly_between_threshold_and_full() {
        // Midpoint of the 1.0..30.0 ramp.
        let mid = forcefield_spike(15.5, 1.0, 30.0).unwrap();
        assert!((mid - 0.5).abs() < 1e-5, "got {mid}");
    }

    #[test]
    fn forcefield_spike_clamps_above_full() {
        assert_eq!(forcefield_spike(500.0, 1.0, 30.0), Some(1.0));
    }

    #[test]
    fn forcefield_spike_at_threshold_is_zero_not_none() {
        assert_eq!(forcefield_spike(1.0, 1.0, 30.0), Some(0.0));
    }

    #[test]
    fn forcefield_spike_survives_degenerate_config() {
        // full_spike <= threshold would divide by zero.
        assert_eq!(forcefield_spike(10.0, 5.0, 5.0), Some(1.0));
        assert_eq!(forcefield_spike(10.0, 5.0, 1.0), Some(1.0));
        assert_eq!(forcefield_spike(1.0, 5.0, 1.0), None);
    }

    #[test]
    fn forcefield_decay_reaches_zero_and_stops() {
        let mut i = 1.0_f32;
        for _ in 0..100 {
            i = forcefield_decay(i, 0.1, 1.5);
            assert!(i >= 0.0, "intensity went negative: {i}");
        }
        assert_eq!(i, 0.0);
    }

    #[test]
    fn forcefield_decay_is_linear_in_dt() {
        let after = forcefield_decay(1.0, 0.2, 1.5);
        assert!((after - 0.7).abs() < 1e-5, "got {after}");
    }

    #[test]
    fn forcefield_volume_lerps_base_to_spike() {
        assert!((forcefield_volume(0.0, 0.06, 0.8) - 0.06).abs() < 1e-6);
        assert!((forcefield_volume(1.0, 0.06, 0.8) - 0.8).abs() < 1e-6);
        assert!((forcefield_volume(0.5, 0.06, 0.8) - 0.43).abs() < 1e-6);
    }

    #[test]
    fn forcefield_volume_clamps_to_unit_range() {
        // Designer-edited TOML can specify out-of-range volumes; the JS
        // setter throws IndexSizeError if we pass them through.
        assert_eq!(forcefield_volume(1.0, 0.0, 5.0), 1.0);
        assert_eq!(forcefield_volume(0.0, -3.0, 1.0), 0.0);
    }

    // ── Listener-relative geometry ────────────────────────────────────

    fn approx(a: [f32; 3], b: [f32; 3]) {
        for i in 0..3 {
            assert!(
                (a[i] - b[i]).abs() < 1e-4,
                "component {i}: got {a:?}, want {b:?}"
            );
        }
    }

    #[test]
    fn listener_relative_sound_dead_ahead_is_negative_z() {
        // yaw 0 faces world −Z (North). A sound 10 units North is straight
        // ahead, which in Web Audio's frame is −Z.
        approx(
            listener_relative(0.0, 0.0, 0.0, 0.0, -10.0),
            [0.0, 0.0, -10.0],
        );
    }

    #[test]
    fn listener_relative_sound_due_east_is_positive_x() {
        // yaw 0, sound 10 units East (+X world) is off the starboard beam.
        approx(listener_relative(0.0, 0.0, 0.0, 10.0, 0.0), [10.0, 0.0, 0.0]);
    }

    #[test]
    fn listener_relative_rotates_with_yaw() {
        // The test that catches a yaw-sign inversion: turn 90° to starboard
        // and the same East-side sound must now be dead ahead.
        use std::f32::consts::FRAC_PI_2;
        approx(
            listener_relative(0.0, 0.0, FRAC_PI_2, 10.0, 0.0),
            [0.0, 0.0, -10.0],
        );
    }

    #[test]
    fn listener_relative_sound_astern_is_positive_z() {
        approx(listener_relative(0.0, 0.0, 0.0, 0.0, 10.0), [0.0, 0.0, 10.0]);
    }

    #[test]
    fn listener_relative_sound_to_port_is_negative_x() {
        approx(
            listener_relative(0.0, 0.0, 0.0, -10.0, 0.0),
            [-10.0, 0.0, 0.0],
        );
    }

    #[test]
    fn listener_relative_preserves_distance_under_rotation() {
        let expected = (3.0_f32 * 3.0 + 4.0 * 4.0).sqrt();
        for steps in 0..16 {
            let yaw = steps as f32 * std::f32::consts::TAU / 16.0;
            let p = listener_relative(1.0, 2.0, yaw, 4.0, 6.0);
            let d = (p[0] * p[0] + p[2] * p[2]).sqrt();
            assert!((d - expected).abs() < 1e-4, "yaw {yaw}: {d} != {expected}");
        }
    }

    #[test]
    fn listener_relative_is_translation_invariant() {
        approx(
            listener_relative(100.0, -50.0, 0.0, 100.0, -60.0),
            [0.0, 0.0, -10.0],
        );
    }
}
