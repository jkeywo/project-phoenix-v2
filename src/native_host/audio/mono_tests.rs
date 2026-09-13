//! Right-only authored-shaped PCM through the actual room player and live mixer.
use super::*;
use crate::audio_config::{
    AmbientAudio, ComputerMessageAudio, ComputerMessageCue, ShipAudioConfig,
};
use crate::native_host::{
    audio::NativeRoomAudio,
    host_lobby::HostLobbyRecord,
    viewscreen_presentation::{ViewscreenPresentation, ViewscreenPresentationStore},
};

fn pcm() -> Arc<Pcm> {
    decoder::decode(
        include_bytes!("../../../tests/fixtures/audio-mono-right.wav").to_vec(),
        "wav",
    )
    .unwrap()
}
fn input() -> RoomInput {
    RoomInput {
        lifecycle: AudioLifecycleState {
            generation: 1,
            running: true,
            suspended: false,
        },
        config: crate::audio_config::build_audio_payload(
            Some(&ShipAudioConfig {
                ambient: Some(AmbientAudio {
                    file: "mono-fixture.wav".into(),
                    volume: 0.5,
                }),
                computer_message: Some(ComputerMessageAudio {
                    critical: Some(ComputerMessageCue {
                        file: "mono-fixture.wav".into(),
                        volume: 0.6,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            None,
        ),
        ..Default::default()
    }
}
fn player(audio: &NativeRoomAudio) -> RoomPlayer {
    let mut player = RoomPlayer {
        mixer: audio.mixer.clone(),
        ..Default::default()
    };
    player.cache.insert("mono-fixture.wav".into(), Ok(pcm()));
    player
}
fn samples(player: &RoomPlayer) -> Vec<f32> {
    let mut samples = vec![0.0; 512];
    player.mixer.lock().unwrap().render(&mut samples, 44100, 2);
    assert!(samples.iter().all(|value| value.is_finite()));
    samples
}
fn mono(samples: &[f32]) {
    assert!(samples.iter().any(|value| value.abs() > 0.0001));
    assert!(samples.chunks_exact(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn native_mono_changes_live_room_loops_without_reset_and_converts_authored_alerts_before_mutes() {
    let mut audio = NativeRoomAudio::with_stores(None, false, None, None);
    let reference = NativeRoomAudio::with_stores(None, false, None, None);
    reference.mixer.lock().unwrap().set_mono(true);
    let mut actual = player(&audio);
    let mut expected = player(&reference);
    let input = input();
    actual.apply(&input, true);
    expected.apply(&input, true);
    let stereo = samples(&actual);
    let _ = samples(&expected);
    assert!(stereo.chunks_exact(2).all(|pair| pair[0] == 0.0));
    assert!(stereo.iter().any(|value| value.abs() > 0.0001));
    audio.command(&HostLobbyRecord::SetAudioMono { enabled: true });
    audio.command(&HostLobbyRecord::SetAudioDucking { enabled: true });
    let converted = samples(&actual);
    mono(&converted);
    assert_eq!(converted, samples(&expected));
    actual.mixer.lock().unwrap().stop_all();
    let cue = actual.prepare_computer(&input, "critical").unwrap();
    actual.play_computer(&input, true, cue);
    mono(&samples(&actual));
    audio.command(&HostLobbyRecord::SetAudioBus {
        bus: "alerts".into(),
        level_percent: 100,
        muted: true,
    });
    assert!(samples(&actual).iter().all(|value| *value == 0.0));
    audio.command(&HostLobbyRecord::SetAudioBus {
        bus: "alerts".into(),
        level_percent: 100,
        muted: false,
    });
    audio.command(&HostLobbyRecord::SetAudioBus {
        bus: "master".into(),
        level_percent: 0,
        muted: false,
    });
    let cue = actual.prepare_computer(&input, "critical").unwrap();
    actual.play_computer(&input, true, cue);
    assert!(samples(&actual).iter().all(|value| *value == 0.0));
}

#[test]
fn native_mono_uses_endpoint_storage_and_audio_reset_keeps_display_preferences() {
    #[allow(clippy::disallowed_methods)]
    let root = std::env::temp_dir().join(format!("phoenix-mono-{}", uuid::Uuid::new_v4()));
    let store = ViewscreenPresentationStore::at(&root);
    store
        .save(&ViewscreenPresentation {
            text_scale_percent: Some(175),
            ..Default::default()
        })
        .unwrap();
    let mut audio = NativeRoomAudio::with_stores(None, false, Some(store.clone()), None);
    assert!(!audio.snapshot().mono);
    audio.command(&HostLobbyRecord::SetAudioMono { enabled: true });
    audio.command(&HostLobbyRecord::SetAudioDucking { enabled: true });
    audio.command(&HostLobbyRecord::SetAudioReducedRange { enabled: true });
    audio.command(&HostLobbyRecord::SetAudioBus {
        bus: "master".into(),
        level_percent: 23,
        muted: true,
    });
    let mut next = NativeRoomAudio::with_stores(None, false, Some(store.clone()), None);
    assert!(next.snapshot().mono);
    assert!(next.snapshot().ducking);
    assert!(next.snapshot().reduced_range);
    assert!(next.snapshot().mix.master.muted);
    assert_eq!(next.snapshot().mix.master.level, 0.23);
    next.command(&HostLobbyRecord::ResetAudioMix);
    assert!(!store.load_audio_mono());
    assert!(!store.load_audio_ducking());
    assert!(!store.load_audio_reduced_range());
    assert_eq!(store.load().text_scale_percent, Some(175));
    std::fs::remove_dir_all(root).unwrap();
}
