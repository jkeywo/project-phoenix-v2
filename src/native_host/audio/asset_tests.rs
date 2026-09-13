use super::*;
use crate::audio_config::{AmbientAudio, ComputerMessageAudio, ComputerMessageCue, RedAlertAudio};
use crate::entities::config_cache::{
    mod_pack_revision, push_mod_pack, remove_mod_pack, reorder_mod_packs, ActivePack,
};

pub(super) const FILE: &str = "assets/sounds/audio_epoch_fixture.wav";
pub(super) fn wav(value: i16) -> Arc<[u8]> {
    let bytes = 1024u32 * 2;
    let mut result = b"RIFF".to_vec();
    result.extend((36 + bytes).to_le_bytes());
    result.extend(b"WAVEfmt ");
    result.extend(16u32.to_le_bytes());
    result.extend(1u16.to_le_bytes());
    result.extend(1u16.to_le_bytes());
    result.extend(44100u32.to_le_bytes());
    result.extend(88200u32.to_le_bytes());
    result.extend(2u16.to_le_bytes());
    result.extend(16u16.to_le_bytes());
    result.extend(b"data");
    result.extend(bytes.to_le_bytes());
    for _ in 0..1024 {
        result.extend(value.to_le_bytes());
    }
    Arc::from(result)
}
pub(super) fn install(id: &str, value: i16) {
    push_mod_pack(ActivePack {
        id: id.into(),
        assets: [(FILE.into(), wav(value))].into(),
        ..Default::default()
    });
}
pub(super) struct Cleanup(pub &'static [&'static str]);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for id in self.0 {
            remove_mod_pack(id);
        }
    }
}
fn input() -> RoomInput {
    RoomInput {
        lifecycle: crate::console_bridge::AudioLifecycleState {
            generation: 1,
            running: true,
            suspended: false,
        },
        config: crate::audio_config::AudioConfigPayload {
            ambient: Some(AmbientAudio {
                file: FILE.into(),
                volume: 1.0,
            }),
            computer_message: Some(ComputerMessageAudio {
                critical: Some(ComputerMessageCue {
                    file: FILE.into(),
                    volume: 1.0,
                }),
                ..Default::default()
            }),
            red_alert: Some(RedAlertAudio {
                music_file: FILE.into(),
                music_volume: 0.0,
                siren_file: FILE.into(),
                siren_volume: 1.0,
            }),
            ..Default::default()
        },
        red_alert: true,
        authored: vec![crate::sound_cues::SoundDefinition {
            id: "epoch".into(),
            label: "[Background tone]".into(),
            file: FILE.into(),
            category: "music".into(),
            audience: "viewscreen".into(),
            volume: 1.0,
            equivalent: None,
        }],
        ..Default::default()
    }
}
fn samples(player: &RoomPlayer) -> [f32; 32] {
    let mut output = [0.0; 32];
    player.mixer.lock().unwrap().render(&mut output, 44100, 2);
    assert!(output.iter().all(|value| value.is_finite()));
    output
}

#[test]
fn current_room_loop_reloads_exact_accepted_pcm_on_replace_reorder_and_remove() {
    let _lock = crate::entities::config_cache::overlay_test_guard();
    let _cleanup = Cleanup(&["audio-epoch-low", "audio-epoch-high"]);
    install("audio-epoch-low", 8192);
    let mut player = RoomPlayer::default();
    let current = input();
    player.refresh_assets(mod_pack_revision());
    player.prepare(&current);
    player.apply(&current, true);
    assert!(samples(&player)
        .iter()
        .all(|value| (*value - 0.25).abs() < 0.00001));
    let old = player.prepare_computer(&current, "critical").unwrap();
    let authored = player
        .prepare_authored(
            &current,
            &current.authored[0],
            Instant::now() + std::time::Duration::from_secs(1),
        )
        .unwrap();
    install("audio-epoch-high", 16384);
    player.refresh_assets(mod_pack_revision());
    player.play_computer(&current, true, old);
    player.play_authored(&current, true, authored);
    assert!(samples(&player).iter().all(|value| *value == 0.0));
    player.prepare(&current);
    player.apply(&current, true);
    assert!(!player.mixer.lock().unwrap().active("siren"));
    assert!(samples(&player)
        .iter()
        .all(|value| (*value - 0.5).abs() < 0.00001));
    reorder_mod_packs(&["audio-epoch-high".into(), "audio-epoch-low".into()]);
    player.refresh_assets(mod_pack_revision());
    player.prepare(&current);
    player.apply(&current, true);
    assert!(samples(&player)
        .iter()
        .all(|value| (*value - 0.25).abs() < 0.00001));
    remove_mod_pack("audio-epoch-low");
    remove_mod_pack("audio-epoch-high");
    player.refresh_assets(mod_pack_revision());
    player.prepare(&current);
    player.apply(&current, true);
    assert!(samples(&player).iter().all(|value| *value == 0.0));
    assert!(player.failures.iter().any(|error| error.starts_with(FILE)));
    // Re-admission must clear the missing-file cache, without inventing a siren.
    install("audio-epoch-low", 8192);
    player.refresh_assets(mod_pack_revision());
    player.prepare(&current);
    player.apply(&current, true);
    assert!(samples(&player)
        .iter()
        .all(|value| (*value - 0.25).abs() < 0.00001));
    assert!(player.failures.is_empty());
    assert!(!player.mixer.lock().unwrap().active("computer"));
    assert!(!player.mixer.lock().unwrap().active("siren"));
}

#[test]
fn room_owner_asset_boundary_stops_audio_and_consumes_pending_occurrences_before_commit() {
    let audio = NativeRoomAudio::with_stores(None, false, None, None);
    audio.apply_input(input());
    let cue = crate::gm_presentation::sound::LiveSoundCue {
        kind: "authored".into(),
        generation: 1,
        occurrence: 99,
        definition: input().authored.remove(0),
    };
    audio.authored_sound(cue.clone());
    audio.computer_message("critical");
    audio.blaster([1.0, 0.0, 0.0]);
    {
        let mut control = audio.control.lock().unwrap();
        control.test_at = Some(Instant::now());
        control.alert_at = Some(Instant::now());
    }
    let old = audio.control.lock().unwrap().clone();
    audio.mixer.lock().unwrap().cue(
        "old",
        Arc::new(decoder::Pcm {
            samples: vec![0.25; 1024],
            rate: 44100,
            channels: 1,
        }),
        "alerts",
        1.0,
        None,
    );
    audio.refresh_assets(old.asset_revision + 1);
    let current = audio.control.lock().unwrap();
    assert_ne!(current.asset_revision, old.asset_revision);
    assert_eq!(current.input, old.input);
    assert!(current.test_at.is_none() && current.alert_at.is_none());
    assert!(current.blaster.is_none() && current.computer.is_none() && current.authored.is_none());
    assert_eq!(current.authored_serial, 99);
    assert!(!audio.mixer.lock().unwrap().active("old"));
    drop(current);
    audio.authored_sound(cue);
    assert!(audio.control.lock().unwrap().authored.is_none());
}
