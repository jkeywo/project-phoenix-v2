use super::{
    decoder,
    engine::Mixer,
    mix::AudioMix,
    player::{RoomInput, RoomPlayer},
    range::RangeSpec,
};
use std::sync::Arc;

const RATE: u32 = 24000;
fn signal() -> Arc<decoder::Pcm> {
    Arc::new(decoder::Pcm {
        rate: RATE,
        channels: 2,
        samples: (0..RATE * 5)
            .flat_map(|i| {
                let amplitude = if (RATE..RATE * 2).contains(&i) {
                    0.8
                } else {
                    0.025
                };
                let sample =
                    amplitude * (i as f32 * std::f32::consts::TAU * 440.0 / RATE as f32).sin();
                [sample, sample * 0.4]
            })
            .collect(),
    })
}
fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    mixer.render(&mut output, RATE, 2);
    assert!(output.iter().all(|sample| sample.is_finite()));
    output
}
fn rms(samples: &[f32], start: f32, end: f32) -> f32 {
    let from = (start * RATE as f32) as usize;
    let to = (end * RATE as f32) as usize;
    (samples
        .chunks_exact(2)
        .skip(from)
        .take(to - from)
        .map(|p| p[0] * p[0])
        .sum::<f32>()
        / (to - from) as f32)
        .sqrt()
}
fn mixer(enabled: bool, pcm: Arc<decoder::Pcm>) -> Mixer {
    let mut mixer = Mixer::default();
    mixer.set_reduced_range(enabled);
    mixer.set_loop("current-bed", pcm, "ambience", 1.0);
    mixer
}

#[test]
fn full_dynamics_preserves_samples_and_reduced_range_narrows_quiet_loud_difference() {
    let pcm = signal();
    let full = render(&mut mixer(false, pcm.clone()), RATE as usize * 5);
    assert_eq!(full, pcm.samples);
    let reduced = render(&mut mixer(true, pcm), RATE as usize * 5);
    let quiet = rms(&full, 0.4, 0.8);
    let loud = rms(&full, 1.4, 1.8);
    let small = rms(&reduced, 0.4, 0.8);
    let large = rms(&reduced, 1.4, 1.8);
    assert!(small > quiet * 1.1 && small < quiet * 2.0);
    assert!(large < loud * 0.6 && large > 0.0);
    assert!(large / small < loud / quiet * 0.4);
    assert!((rms(&reduced, 4.2, 4.8) / small - 1.0).abs() < 0.02);
    assert!(reduced
        .iter()
        .all(|sample| sample.abs() <= RangeSpec::default().ceiling + 0.00001));
    assert!(reduced
        .chunks_exact(2)
        .all(|frame| (frame[1] - frame[0] * 0.4).abs() < 0.00001));
}

#[test]
fn reduced_range_keeps_master_linear_muted_categories_out_and_live_cursors_unchanged() {
    let pcm = signal();
    let mut reference = mixer(true, pcm.clone());
    let mut actual = mixer(false, pcm.clone());
    let _ = render(&mut actual, 333);
    let _ = render(&mut reference, 333);
    actual.set_reduced_range(true);
    // Match only processing state after the option changes; neither source cursor moves.
    reference.set_reduced_range(false);
    reference.set_reduced_range(true);
    assert_eq!(render(&mut actual, 1024), render(&mut reference, 1024));
    let mut mix = AudioMix::default();
    mix.master.level = 0.25;
    actual.set_mix(mix);
    let scaled = render(&mut actual, 1024);
    let unscaled = render(&mut reference, 1024);
    assert!(scaled.iter().zip(&unscaled).all(|(a, b)| *a == *b * 0.25));
    // An inaudible loud category cannot pump an otherwise identical quiet bed.
    mix.effects.muted = true;
    actual.set_mix(mix);
    actual.set_loop(
        "muted-loud",
        Arc::new(decoder::Pcm {
            samples: vec![8.0, 8.0],
            rate: RATE,
            channels: 2,
        }),
        "effects",
        1.0,
    );
    let scaled = render(&mut actual, 1024);
    let unscaled = render(&mut reference, 1024);
    assert!(scaled.iter().zip(&unscaled).all(|(a, b)| *a == *b * 0.25));
    mix.master.muted = true;
    actual.set_mix(mix);
    assert!(render(&mut actual, 1024)
        .iter()
        .all(|sample| *sample == 0.0));
    actual.stop_all();
    mix.master.muted = false;
    actual.set_mix(mix);
    assert!(render(&mut actual, 1024)
        .iter()
        .all(|sample| *sample == 0.0));
}

#[test]
fn reduced_range_bounds_summed_peaks_and_stereo_to_mono_composition() {
    let pcm = Arc::new(decoder::Pcm {
        samples: vec![0.9, 0.3],
        rate: RATE,
        channels: 2,
    });
    let mut actual = mixer(true, pcm.clone());
    actual.set_loop("another-bed", pcm, "music", 1.0);
    actual.set_mono(true);
    let output = render(&mut actual, 4096);
    assert!(output.chunks_exact(2).all(|p| p[0] == p[1]));
    assert!(output
        .iter()
        .all(|sample| sample.abs() <= RangeSpec::default().ceiling));
    assert!(output.iter().any(|sample| *sample > 0.01));
}

#[test]
fn authored_room_cue_uses_reduced_range_and_remains_silent_under_master_or_alerts_mute() {
    use crate::audio_config::{ComputerMessageAudio, ComputerMessageCue, ShipAudioConfig};
    let input = RoomInput {
        lifecycle: crate::console_bridge::AudioLifecycleState {
            generation: 1,
            running: true,
            suspended: false,
        },
        config: crate::audio_config::build_audio_payload(
            Some(&ShipAudioConfig {
                computer_message: Some(ComputerMessageAudio {
                    critical: Some(ComputerMessageCue {
                        file: "assets/sounds/ui_click.ogg".into(),
                        volume: 1.0,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            None,
        ),
        ..Default::default()
    };
    let mut player = RoomPlayer::default();
    player.mixer.lock().unwrap().set_reduced_range(true);
    let prepared = player.prepare_computer(&input, "critical").unwrap();
    player.play_computer(&input, true, prepared);
    let output = render(&mut player.mixer.lock().unwrap(), RATE as usize);
    assert!(output.iter().any(|sample| sample.abs() > 0.0001));
    assert!(output
        .iter()
        .all(|sample| sample.abs() <= RangeSpec::default().ceiling));
    for bus in ["master", "alerts"] {
        let mut mix = AudioMix::default();
        mix.set(
            bus,
            super::mix::Bus {
                level: 1.0,
                muted: true,
            },
        );
        player.mixer.lock().unwrap().set_mix(mix);
        let prepared = player.prepare_computer(&input, "critical").unwrap();
        player.play_computer(&input, true, prepared);
        assert!(render(&mut player.mixer.lock().unwrap(), 1024)
            .iter()
            .all(|sample| *sample == 0.0));
    }
}
