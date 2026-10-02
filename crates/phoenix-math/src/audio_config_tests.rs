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
    assert_eq!(
        cfg.ambient.as_ref().unwrap().file,
        "assets/sounds/Ambient.mp3"
    );
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

// ── Ship's-computer message audio (issue #1342) ────────────────────

fn computer_message_cfg() -> ComputerMessageAudio {
    ComputerMessageAudio {
        info: Some(ComputerMessageCue {
            file: "assets/sounds/ComputerInfo.mp3".into(),
            volume: 0.3,
        }),
        advisory: None,
        warning: Some(ComputerMessageCue {
            file: "assets/sounds/ComputerWarning.mp3".into(),
            volume: 0.6,
        }),
        critical: None,
    }
}

#[test]
fn computer_message_audio_parses_and_omits_absent_severities() {
    let cfg: ShipAudioConfig = toml::from_str(
        r#"
[computer_message.info]
file   = "assets/sounds/ComputerInfo.mp3"
volume = 0.3
"#,
    )
    .expect("parses");
    let cm = cfg.computer_message.as_ref().unwrap();
    assert_eq!(cm.info.as_ref().unwrap().volume, 0.3);
    assert!(cm.advisory.is_none());
    assert!(cm.warning.is_none());
    assert!(cm.critical.is_none());
}

#[test]
fn for_severity_returns_only_the_configured_ones() {
    let cm = computer_message_cfg();
    assert_eq!(
        cm.for_severity("info").unwrap().file,
        "assets/sounds/ComputerInfo.mp3"
    );
    assert!(cm.for_severity("advisory").is_none());
    assert_eq!(
        cm.for_severity("warning").unwrap().file,
        "assets/sounds/ComputerWarning.mp3"
    );
    assert!(cm.for_severity("critical").is_none());
    assert!(cm.for_severity("not-a-severity").is_none());
}

#[test]
fn build_payload_carries_computer_message_audio_through() {
    let ship = ShipAudioConfig {
        computer_message: Some(computer_message_cfg()),
        ..Default::default()
    };
    let p = build_audio_payload(Some(&ship), None);
    assert_eq!(
        p.computer_message.unwrap().info.unwrap().file,
        "assets/sounds/ComputerInfo.mp3"
    );
}

#[test]
fn build_payload_without_computer_message_config_is_absent_from_json() {
    let p = build_audio_payload(None, None);
    assert!(p.computer_message.is_none());
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains("computer_message"));
}

#[test]
fn audio_cue_computer_message_is_not_positional() {
    let cue = AudioCue::computer_message("critical");
    assert_eq!(cue.kind, "computer_message");
    assert_eq!(cue.x, 0.0);
    assert_eq!(cue.y, 0.0);
    assert_eq!(cue.z, 0.0);
    assert_eq!(cue.severity.as_deref(), Some("critical"));
}

#[test]
fn audio_cue_blaster_carries_no_severity() {
    let cue = AudioCue::blaster([1.0, 2.0, 3.0]);
    assert_eq!(cue.kind, "blaster");
    assert!(cue.severity.is_none());
    let json = serde_json::to_string(&cue).unwrap();
    assert!(
        !json.contains("severity"),
        "absent severity must not appear in the wire JSON: {json}"
    );
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
    assert!(
        !json.contains("decay_rate_per_sec"),
        "envelope leaked: {json}"
    );
    assert!(
        !json.contains("damage_threshold"),
        "envelope leaked: {json}"
    );
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
    approx(
        listener_relative(0.0, 0.0, 0.0, 10.0, 0.0),
        [10.0, 0.0, 0.0],
    );
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
    approx(
        listener_relative(0.0, 0.0, 0.0, 0.0, 10.0),
        [0.0, 0.0, 10.0],
    );
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
