use super::*;
use crate::audio_config::{
    AmbientAudio, EngineAudio, RedAlertAudio, ShipAudioConfig, WorldAudioConfig,
};

#[test]
fn room_ducking_reaches_samples_only_on_beds_and_overlap_has_a_finite_release() {
    let spec = ducking::DuckingSpec::default();
    let pcm = Arc::new(decoder::Pcm {
        samples: vec![0.1; 48_000 * 2 * 6],
        rate: 48_000,
        channels: 2,
    });
    let mut mixer = engine::Mixer::default();
    for category in ["music", "ambience", "effects", "interface"] {
        mixer.set_loop(category, pcm.clone(), category, 1.0);
    }
    let render = |mixer: &mut engine::Mixer, seconds: f64| {
        let mut output = vec![0.0; (seconds * 48_000.0) as usize * 2];
        mixer.render(&mut output, 48_000, 2);
        output
    };
    mixer.important_cue("authored_alert", pcm.clone(), 1.0);
    assert!(render(&mut mixer, 0.1)
        .iter()
        .all(|v| (*v - 0.5).abs() < 0.00001));
    mixer.stop("authored_alert");
    mixer.set_ducking(true);
    mixer.important_cue("authored_alert", pcm.clone(), 1.0);
    let attack = render(&mut mixer, spec.attack_seconds);
    assert!((attack[0] - 0.5).abs() < 0.00001);
    assert!((*attack.last().unwrap() - (0.3 + 0.2 * spec.gain as f32)).abs() < 0.0001);
    for _ in 0..7 {
        mixer.important_cue("authored_alert", pcm.clone(), 1.0);
        let hold = render(&mut mixer, 0.5);
        assert!(hold
            .iter()
            .all(|v| (*v - (0.3 + 0.2 * spec.gain as f32)).abs() < 0.0001));
    }
    // Keep posting through the burst cap: release still completes.
    for _ in 0..3 {
        mixer.important_cue("authored_alert", pcm.clone(), 1.0);
        render(&mut mixer, 0.5);
    }
    assert!(render(&mut mixer, 0.1)
        .iter()
        .all(|v| (*v - 0.5).abs() < 0.0001));
    mixer.mix.music.muted = true;
    mixer.mix.ambience.muted = true;
    assert!(render(&mut mixer, 0.1)
        .iter()
        .all(|v| (*v - 0.3).abs() < 0.0001));
    mixer.mix.master.muted = true;
    assert!(render(&mut mixer, 0.1).iter().all(|v| *v == 0.0));
}

#[test]
fn real_authored_red_alert_decoding_ducks_native_beds_and_restore_resets_the_window() {
    let mut dry = RoomPlayer::default();
    let mut wet = RoomPlayer::default();
    wet.mixer.lock().unwrap().set_ducking(true);
    let mut live = input();
    for player in [&mut dry, &mut wet] {
        player.prepare(&live);
        player.apply(&live, true);
    }
    live.red_alert = true;
    for player in [&mut dry, &mut wet] {
        player.apply(&live, true);
        assert!(player.mixer.lock().unwrap().active("siren"));
        // Isolate decoded beds in the sink after their real siren edge.
        player.mixer.lock().unwrap().stop("siren");
    }
    let dry_samples = sink(&dry, 0.25);
    let wet_samples = sink(&wet, 0.25);
    assert_sound(&dry_samples);
    assert_sound(&wet_samples);
    let gain = ducking::DuckingSpec::default().gain as f32;
    for (a, b) in dry_samples[10_000..].iter().zip(&wet_samples[10_000..]) {
        assert!((*b - *a * gain).abs() < 0.00001);
    }
    live.lifecycle.generation += 1;
    for player in [&mut dry, &mut wet] {
        player.apply(&live, true);
    }
    assert_eq!(sink(&dry, 0.1), sink(&wet, 0.1));
    assert!(!wet.mixer.lock().unwrap().active("siren"));
}

#[test]
fn room_ducking_persists_with_mix_preserves_display_and_resets_only_audio() {
    let directory = scratch();
    let store = ViewscreenPresentationStore::at(&directory);
    let display = super::super::viewscreen_presentation::ViewscreenPresentation {
        text_scale_percent: Some(175),
        ..Default::default()
    };
    store.save(&display).unwrap();
    let mut audio = NativeRoomAudio::with_stores(None, false, Some(store.clone()), None);
    assert!(!audio.snapshot().ducking);
    audio.command(&HostLobbyRecord::SetAudioDucking { enabled: true });
    audio.command(&HostLobbyRecord::SetAudioBus {
        bus: "music".into(),
        level_percent: 35,
        muted: true,
    });
    let mut reloaded = NativeRoomAudio::with_stores(None, false, Some(store.clone()), None);
    assert!(reloaded.snapshot().ducking);
    assert!(reloaded.snapshot().mix.music.muted);
    assert_eq!(store.load(), display);
    reloaded.command(&HostLobbyRecord::ResetAudioMix);
    assert!(!store.load_audio_ducking());
    assert_eq!(store.load(), display);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn browser_and_native_share_gain_policy_fixtures() {
    #[derive(serde::Deserialize)]
    struct Policy {
        mix: AudioMix,
        category: String,
        authored: f32,
        expected: f32,
    }
    let cases: Vec<Policy> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/audio-gain-policy.json"
    ))
    .unwrap();
    for case in cases {
        assert!((case.mix.gain(&case.category, case.authored) - case.expected).abs() < 0.00001);
    }
}

fn input() -> RoomInput {
    RoomInput {
        lifecycle: crate::console_bridge::AudioLifecycleState {
            generation: 1,
            running: true,
            suspended: false,
        },
        config: build_audio_payload(Some(&ship()), Some(&world())),
        red_alert: false,
        thrust: 0.0,
        menu: false,
        phaser: false,
        forcefield: 0.0,
    }
}
fn ship() -> ShipAudioConfig {
    ShipAudioConfig {
        ambient: Some(AmbientAudio {
            file: "assets/sounds/Ambient.mp3".into(),
            volume: 0.35,
        }),
        engine: Some(EngineAudio {
            file: "assets/sounds/Engine.mp3".into(),
            idle_volume: 0.1,
            volume_at_full_thrust: 0.6,
        }),
        ..Default::default()
    }
}
fn world() -> WorldAudioConfig {
    WorldAudioConfig {
        red_alert: Some(RedAlertAudio {
            siren_file: "assets/sounds/red_alert_siren.ogg".into(),
            siren_volume: 0.7,
            music_file: "assets/sounds/last_stand_in_space_looped.ogg".into(),
            music_volume: 0.4,
        }),
    }
}
/// A real application sink: the same Mixer::render call as the CPAL callback,
/// with an ordinary sample buffer in place of an OS device.
fn sink(player: &RoomPlayer, seconds: f32) -> Vec<f32> {
    let mut samples = vec![0.0; (44_100.0 * seconds) as usize * 2];
    player.mixer.lock().unwrap().render(&mut samples, 44_100, 2);
    samples
}
fn assert_sound(samples: &[f32]) {
    assert!(samples.iter().all(|sample| sample.is_finite()));
    assert!(
        samples.iter().any(|sample| sample.abs() > 0.0001),
        "actual decoded PCM must reach the sink"
    );
}

#[test]
fn native_application_adapter_decodes_authored_mp3_and_ogg_into_device_samples() {
    let mut app = App::new();
    app.insert_resource(State::new(GamePhase::InProgress))
        .insert_resource(RoomAudioLifecycle::default())
        .insert_resource(WorldConfig {
            audio: Some(world()),
            ..Default::default()
        })
        .insert_resource(NativeRoomAudio::with_stores(None, false, None, None))
        .add_message::<HudStateChanged>()
        .add_message::<AudioCueEvent>()
        .add_systems(Update, update_room);
    app.world_mut().spawn((LocalShip, ShipAudioSection(ship())));
    app.world_mut().resource_mut::<RoomAudioLifecycle>().state = input().lifecycle;
    app.world_mut().write_message(HudStateChanged { json: r#"{"heading":0,"hull_pct":100,"condition":"NOMINAL","red_alert":false,"engine_thrust":0.5}"#.into() });
    app.update();
    let live = app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .input
        .clone();
    assert_eq!(live.thrust, 0.5);
    let mut player = RoomPlayer::default();
    player.prepare(&live);
    player.apply(&live, true);
    assert!(player.failures.is_empty(), "{:?}", player.failures);
    assert_sound(&sink(&player, 0.2));
    app.world_mut().write_message(HudStateChanged { json: r#"{"heading":0,"hull_pct":100,"condition":"ALERT","red_alert":true,"engine_thrust":0.5}"#.into() });
    app.update();
    let alert = app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .input
        .clone();
    player.apply(&alert, true);
    assert!(player.mixer.lock().unwrap().active("siren"));
    let mut mix = AudioMix::default();
    mix.ambience.muted = true;
    mix.music.muted = true;
    player.mixer.lock().unwrap().set_mix(mix);
    assert_sound(&sink(&player, 0.2)); // OGG siren alone, not the MP3 bed.
    mix.master.muted = true;
    player.mixer.lock().unwrap().set_mix(mix);
    assert!(sink(&player, 0.2).iter().all(|sample| *sample == 0.0));
    assert!(!player.mixer.lock().unwrap().active("siren"));
}

#[test]
fn boundaries_lost_outputs_and_repeated_snapshots_never_catch_up_old_sirens_or_tests() {
    let mut player = RoomPlayer::default();
    let mut state = input();
    player.prepare(&state);
    player.apply(&state, true);
    state.red_alert = true;
    player.apply(&state, true);
    assert!(player.mixer.lock().unwrap().active("siren"));
    sink(&player, 8.0);
    player.apply(&state, true);
    assert!(!player.mixer.lock().unwrap().active("siren"));
    player.test_output(true);
    assert!(player.mixer.lock().unwrap().active("test"));
    state.lifecycle.generation += 1;
    state.lifecycle.suspended = true;
    player.apply(&state, true);
    assert!(!player.mixer.lock().unwrap().active("test"));
    assert!(sink(&player, 0.1).iter().all(|sample| *sample == 0.0));
    state.lifecycle.generation += 1;
    state.lifecycle.suspended = false;
    player.apply(&state, true);
    assert!(!player.mixer.lock().unwrap().active("siren"));
    assert!(player.mixer.lock().unwrap().active("music"));
    state.red_alert = false;
    player.apply(&state, false);
    state.red_alert = true;
    player.apply(&state, false);
    player.reset_output();
    player.apply(&state, true);
    assert!(!player.mixer.lock().unwrap().active("siren"));
    assert!(!player.mixer.lock().unwrap().active("test"));
}

#[test]
fn output_test_is_real_menu_samples_bounded_and_master_or_music_muted() {
    let mut player = RoomPlayer::default();
    player.test_output(true);
    assert_sound(&sink(&player, 0.4));
    sink(&player, 2.0);
    assert!(!player.mixer.lock().unwrap().active("test"));
    for bus in ["master", "music"] {
        let mut mix = AudioMix::default();
        mix.set(
            bus,
            Bus {
                level: 1.0,
                muted: true,
            },
        );
        player.mixer.lock().unwrap().set_mix(mix);
        player.test_output(true);
        assert!(!player.mixer.lock().unwrap().active("test"));
        assert!(sink(&player, 0.1).iter().all(|sample| *sample == 0.0));
    }
    player.test_output(false);
    assert!(!player.mixer.lock().unwrap().active("test"));
}

#[test]
fn decoder_refuses_invalid_content_and_mixer_resamples_stereo_with_authored_gain() {
    assert!(decoder::decode(vec![0, 1, 2], "mp3").is_err());
    assert!(decoder::read("../private.mp3").is_err());
    let pcm = Arc::new(decoder::Pcm {
        samples: vec![0.4, -0.2, 0.8, -0.4],
        rate: 2,
        channels: 2,
    });
    let mut mixer = engine::Mixer::default();
    mixer.mix.master.level = 0.5;
    mixer.mix.ambience.level = 0.25;
    mixer.set_loop("bed", pcm, "ambience", 0.8);
    let mut result = [0.0; 8];
    mixer.render(&mut result, 4, 2);
    for (actual, expected) in result
        .iter()
        .zip([0.04, -0.02, 0.06, -0.03, 0.08, -0.04, 0.06, -0.03])
    {
        assert!((actual - expected).abs() < 0.00001);
    }
}

fn scratch() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("phoenix-audio-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}
#[test]
fn endpoint_mix_survives_visual_writes_and_visual_reset_hardware_stays_separate() {
    let dir = scratch();
    let store = ViewscreenPresentationStore::at(&dir);
    let mut mix = AudioMix::default();
    mix.master.level = 0.23;
    mix.alerts.muted = true;
    store.save_audio(mix).unwrap();
    let visual = super::super::viewscreen_presentation::ViewscreenPresentation {
        text_scale_percent: Some(175),
        ..Default::default()
    };
    store.save(&visual).unwrap();
    assert_eq!(store.load_audio().0, mix);
    store.save(&Default::default()).unwrap();
    assert_eq!(store.load_audio().0, mix);
    store.save(&visual).unwrap();
    store.save_audio(AudioMix::default()).unwrap();
    assert_eq!(store.load(), visual);
    let hardware = store::HardwareStore::at(&dir);
    let profile = store::select_room(
        &BridgeProfile::empty(),
        Some("output:Bridge speakers".into()),
    )
    .unwrap();
    hardware.save(&profile).unwrap();
    assert_eq!(
        store::room_output(&hardware.load().unwrap()).unwrap(),
        Some("output:Bridge speakers".into())
    );
    assert!(!std::fs::read_to_string(store.path())
        .unwrap()
        .contains("Bridge speakers"));
    assert!(!std::fs::read_to_string(hardware.path())
        .unwrap()
        .contains("master"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn corrupt_hardware_cannot_silently_choose_default_and_private_assignments_are_preserved() {
    let dir = scratch();
    let hardware = store::HardwareStore::at(&dir);
    std::fs::write(hardware.path(), "invalid TOML!!!").unwrap();
    let audio = NativeRoomAudio::with_stores(None, false, None, Some(hardware));
    assert!(audio.control.lock().unwrap().routing_error.is_some());
    assert_eq!(audio.snapshot().hardware_persistence, "corrupt");
    let mut profile = BridgeProfile::empty();
    profile
        .media
        .push(super::super::bridge_media::MediaSurfaceEntry {
            surface: "comms".into(),
            outputs: vec!["output:Private headset".into()],
            camera: None,
            microphones: vec![],
            allow_shared: vec![],
        });
    assert!(store::select_room(&profile, Some("output:Private headset".into())).is_err());
    let selected = store::select_room(&profile, Some("output:Room".into())).unwrap();
    assert_eq!(selected.media[0], profile.media[0]);
    assert_eq!(store::room_output(&profile).unwrap(), None);
    drop(audio);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn thrust_updates_do_not_refresh_an_old_alert_edge_after_slow_decoding() {
    let audio = NativeRoomAudio::with_stores(None, false, None, None);
    let mut live = input();
    audio.apply_input(live.clone());
    live.red_alert = true;
    audio.apply_input(live.clone());
    let old = Instant::now() - std::time::Duration::from_secs(1);
    audio.control.lock().unwrap().alert_at = Some(old);
    live.thrust = 0.8;
    audio.apply_input(live.clone());
    assert_eq!(audio.control.lock().unwrap().alert_at, Some(old));
    live.lifecycle.generation += 1;
    audio.apply_input(live);
    assert!(audio.control.lock().unwrap().alert_at.is_none());
}

fn media(surface: &str, output: Option<&str>) -> super::super::bridge_media::MediaSurfaceEntry {
    super::super::bridge_media::MediaSurfaceEntry {
        surface: surface.into(),
        outputs: output.into_iter().map(str::to_owned).collect(),
        camera: None,
        microphones: vec![],
        allow_shared: vec![],
    }
}

#[test]
fn media_merge_policy_preserves_saved_surfaces_unless_explicitly_authored() {
    let dir = scratch();
    let hardware = store::HardwareStore::at(&dir);
    let mut saved = BridgeProfile::empty();
    saved.media = vec![
        media("viewscreen", Some("output:Room")),
        media("comms", Some("output:Private")),
    ];
    hardware.save(&saved).unwrap();

    // Relaunch with a display-only profile does not turn room audio into default.
    let mut launch = BridgeProfile::empty();
    let audio = NativeRoomAudio::with_stores(
        Some(&launch.validate().unwrap()),
        false,
        None,
        Some(hardware.clone()),
    );
    assert_eq!(audio.profile.media, saved.media);
    assert_eq!(audio.snapshot().output.as_deref(), Some("output:Room"));
    assert!(!audio.snapshot().profile_override);
    drop(audio);

    launch.media = vec![media("comms", Some("output:Replacement headset"))];
    let audio = NativeRoomAudio::with_stores(
        Some(&launch.validate().unwrap()),
        false,
        None,
        Some(hardware.clone()),
    );
    assert_eq!(audio.profile.media[0], saved.media[0]);
    assert_eq!(audio.profile.media[1], launch.media[0]);
    assert!(!audio.snapshot().profile_override);
    drop(audio);

    // An explicit empty output is a deliberate system-default choice, not omission.
    launch.media = vec![media("viewscreen", None)];
    let audio = NativeRoomAudio::with_stores(
        Some(&launch.validate().unwrap()),
        false,
        None,
        Some(hardware),
    );
    assert_eq!(audio.snapshot().output, None);
    assert!(audio.snapshot().profile_override);
    assert_eq!(audio.profile.media[1], saved.media[1]);
    assert!(audio.control.lock().unwrap().routing_error.is_none());
    drop(audio);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn media_merge_policy_refuses_new_sharing_between_saved_and_authored_assignments() {
    let dir = scratch();
    let hardware = store::HardwareStore::at(&dir);
    let mut saved = BridgeProfile::empty();
    saved.media = vec![media("comms", Some("output:Private"))];
    hardware.save(&saved).unwrap();
    let mut launch = BridgeProfile::empty();
    launch.media = vec![media("viewscreen", Some("output:Private"))];
    let audio = NativeRoomAudio::with_stores(
        Some(&launch.validate().unwrap()),
        false,
        None,
        Some(hardware),
    );
    assert!(audio.control.lock().unwrap().routing_error.is_some());
    assert_eq!(audio.profile.media.len(), 2);
    drop(audio);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn media_merge_policy_corrupt_saved_route_needs_an_explicit_room_assignment() {
    let dir = scratch();
    let hardware = store::HardwareStore::at(&dir);
    std::fs::write(hardware.path(), "invalid TOML!!!").unwrap();
    let mut launch = BridgeProfile::empty();
    for entries in [
        vec![],
        vec![media("comms", Some("output:Private"))],
        vec![media("viewscreen", Some("output:Room"))],
    ] {
        launch.media = entries;
        let explicit_room = launch
            .media
            .iter()
            .any(|entry| entry.surface == "viewscreen");
        let audio = NativeRoomAudio::with_stores(
            Some(&launch.validate().unwrap()),
            false,
            None,
            Some(hardware.clone()),
        );
        assert_eq!(audio.snapshot().hardware_persistence, "corrupt");
        assert_eq!(audio.snapshot().profile_override, explicit_room);
        assert_eq!(
            audio.control.lock().unwrap().routing_error.is_none(),
            explicit_room
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
