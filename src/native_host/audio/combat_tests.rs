use super::*;
use crate::audio_config::{
    BlasterAudio, DistanceModel, ForcefieldAudio, ForcefieldSource, PanningModel, PhaserLoopAudio,
    ShipAudioConfig,
};
use crate::core::messages::{DeliveryClass, ServerMessage};
use crate::lobby::{OutboundMessage, Target};
use std::time::Duration;

fn config() -> ShipAudioConfig {
    ShipAudioConfig {
        phaser_loop: Some(PhaserLoopAudio {
            file: "assets/sounds/PhaserLoop.mp3".into(),
            volume: 0.5,
        }),
        forcefield: Some(ForcefieldAudio {
            file: "assets/sounds/ForcefieldHit.mp3".into(),
            base_volume: 0.06,
            spike_volume: 0.8,
            damage_threshold: 1.0,
            damage_full_spike: 30.0,
            decay_rate_per_sec: 1.5,
            source: ForcefieldSource::Shield,
        }),
        blaster: Some(BlasterAudio {
            file: "assets/sounds/Blaster.mp3".into(),
            volume: 0.9,
            ref_distance: 30.0,
            max_distance: 800.0,
            rolloff_factor: 1.2,
            distance_model: DistanceModel::Inverse,
            panning_model: PanningModel::EqualPower,
        }),
        ..Default::default()
    }
}

#[test]
fn browser_and_native_share_actual_spatial_sample_policy() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        position: [f32; 3],
        samples: Vec<f32>,
        spec: BlasterAudio,
        expected: [f32; 2],
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/audio-spatial-policy.json"
    ))
    .unwrap();
    for case in cases {
        let matrix = spatial::equal_power(case.position, case.samples.len());
        let pcm = std::sync::Arc::new(decoder::Pcm {
            samples: case.samples.clone(),
            rate: 44100,
            channels: case.samples.len(),
        });
        let mut mixer = engine::Mixer::default();
        mixer.cue_spatial(
            "fixture",
            pcm,
            spatial::distance_gain(&case.spec, case.position),
            matrix,
        );
        let mut out = [0.0; 2];
        mixer.render(&mut out, 44100, 2);
        for (actual, expected) in out.into_iter().zip(case.expected) {
            assert!(
                (actual - expected).abs() < 0.00001,
                "{}: {actual} != {expected}",
                case.name
            );
        }
    }
}
fn render(player: &RoomPlayer) -> Vec<f32> {
    let mut out = vec![0.0; 44100 / 5 * 2];
    player.mixer.lock().unwrap().render(&mut out, 44100, 2);
    assert!(out.iter().all(|sample| sample.is_finite()));
    out
}
fn energy(samples: &[f32], channel: usize) -> f32 {
    samples.iter().skip(channel).step_by(2).map(|v| v * v).sum()
}
fn live(app: &App) -> RoomInput {
    app.world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .input
        .clone()
}
fn advance(app: &mut App, seconds: f32) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(seconds));
    app.update();
}

#[test]
fn real_combat_producers_reach_native_samples_and_muted_live_equivalents() {
    let _assets = crate::entities::config_cache::overlay_test_guard();
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .insert_state(GamePhase::InProgress)
        .init_resource::<Time>()
        .add_message::<OutboundMessage>()
        .add_message::<crate::core::narrative::NarrativeEvent>()
        .insert_resource(NativeRoomAudio::with_stores(None, false, None, None))
        .add_plugins(NativeRoomAudioPlugin);
    app.world_mut().spawn((
        LocalShip,
        crate::ship::state::ShipPhysics::default(),
        ShipAudioSection(config()),
    ));
    // Establish the ordinary boundary before attaching this test's HUD source.
    crate::server::audio_lifecycle::publish_audio_lifecycle(app.world_mut());
    app.add_message::<HudStateChanged>();
    app.world_mut().write_message(HudStateChanged { json: r#"{"heading":0,"hull_pct":100,"condition":"NOMINAL","red_alert":false,"phaser_firing":true}"#.into() });
    advance(&mut app, 0.0);
    let mut player = RoomPlayer::default();
    player.mixer = app.world().resource::<NativeRoomAudio>().mixer.clone();
    let state = live(&app);
    assert!(state.phaser);
    player.prepare(&state);
    player.apply(&state, true);
    assert!(player.mixer.lock().unwrap().active("phaser"));
    assert!(energy(&render(&player), 0) > 0.01);
    let mut visual = visual::VisualReader::default();
    let shared = app.world().resource::<NativeRoomAudio>().visuals.clone();
    visual.read(&shared, true, true, Instant::now());

    app.world_mut().write_message(OutboundMessage {
        target: Target::All,
        delivery: DeliveryClass::Reliable,
        msg: ServerMessage::DamageTaken {
            hull: 0.0,
            shield: 30.0,
        },
    });
    advance(&mut app, 0.0);
    let spike = live(&app);
    assert!((spike.forcefield - 0.8).abs() < 0.0001);
    player.apply(&spike, true);
    player.mixer.lock().unwrap().stop("phaser");
    assert!(energy(&render(&player), 0) > 0.01);
    assert!(visual
        .read(&shared, true, true, Instant::now())
        .iter()
        .any(|cue| matches!(cue, visual::VisualCue::Impact)));
    advance(&mut app, 1.0);
    let settled = live(&app);
    assert!((settled.forcefield - 0.06).abs() < 0.0001);

    app.world_mut().write_message(HudStateChanged { json: r#"{"heading":0,"hull_pct":100,"condition":"NOMINAL","red_alert":false,"phaser_firing":false}"#.into() });
    app.world_mut().write_message(OutboundMessage {
        target: Target::All,
        delivery: DeliveryClass::Reliable,
        msg: ServerMessage::BlasterFired {
            bank: "fore".into(),
            source_uuid: "public-combat-source".into(),
            projectile_id: "shot".into(),
            x: 30.0,
            z: 0.0,
            heading: 0.0,
            visual_scale: 1.0,
        },
    });
    advance(&mut app, 0.0);
    let audio = app.world().resource::<NativeRoomAudio>();
    let state = live(&app);
    let (_, position) = audio.control.lock().unwrap().blaster.take().unwrap();
    player.apply(&state, true);
    assert!(!player.mixer.lock().unwrap().active("phaser"));
    player.mixer.lock().unwrap().stop("forcefield");
    let prepared = player.prepare_blaster(&state, position, None).unwrap();
    player.play_blaster(&state, true, prepared);
    let shot = render(&player);
    assert!(energy(&shot, 1) > energy(&shot, 0) * 4.0);
    assert!(energy(&shot, 1) > 0.01);
    for bus in ["effects", "master"] {
        let mut mix = AudioMix::default();
        mix.set(
            bus,
            Bus {
                level: 1.0,
                muted: true,
            },
        );
        player.mixer.lock().unwrap().set_mix(mix);
        assert!(render(&player).iter().all(|value| *value == 0.0));
        assert!(!player.mixer.lock().unwrap().active("blaster"));
        let prepared = player.prepare_blaster(&state, position, None).unwrap();
        player.play_blaster(&state, true, prepared);
        assert!(
            !player.mixer.lock().unwrap().active("blaster"),
            "future muted cues are discarded"
        );
        player.mixer.lock().unwrap().set_mix(AudioMix::default());
        assert!(
            render(&player).iter().all(|value| *value == 0.0),
            "unmuting never catches up"
        );
        let prepared = player.prepare_blaster(&state, position, None).unwrap();
        player.play_blaster(&state, true, prepared);
    }
    assert!(visual
        .read(&shared, true, true, Instant::now())
        .iter()
        .any(|cue| matches!(cue, visual::VisualCue::Blaster { .. })));
    advance(&mut app, 0.0);
    assert!(app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .blaster
        .is_none());
}

#[test]
fn measured_hrtf_impulse_and_authored_mp3_produce_directional_finite_samples() {
    let source = decoder::read("assets/sounds/Blaster.mp3").unwrap();
    assert_eq!(
        source.rate, 24000,
        "this asset exercises FIR downsampling regression"
    );
    let processed = hrtf::render(&source, [30.0, 0.0, 0.0], None).unwrap();
    assert_eq!(processed.rate, 44100);
    let duration = processed.frames() as f64 / f64::from(processed.rate);
    let source_duration = source.frames() as f64 / f64::from(source.rate);
    assert!((duration - source_duration - 255.0 / 44100.0).abs() < 1.0 / 24000.0);
    let right = hrtf::impulse([30.0, 0.0, 0.0], 44100);
    let left = hrtf::impulse([-30.0, 0.0, 0.0], 44100);
    assert_eq!(right.len(), 256);
    assert!(right.iter().flatten().all(|v| v.is_finite()));
    let power = |filter: &Vec<[f32; 2]>, channel| {
        filter
            .iter()
            .map(|sample| sample[channel] * sample[channel])
            .sum::<f32>()
    };
    assert!(power(&right, 1) > power(&right, 0));
    assert!(power(&left, 0) > power(&left, 1));
    let mut input = RoomInput {
        lifecycle: crate::console_bridge::AudioLifecycleState {
            generation: 1,
            running: true,
            suspended: false,
        },
        config: build_audio_payload(Some(&config()), None),
        ..Default::default()
    };
    input.config.blaster.as_mut().unwrap().panning_model = PanningModel::Hrtf;
    let mut player = RoomPlayer::default();
    player.prepare(&input);
    player.apply(&input, true);
    player.mixer.lock().unwrap().stop_all();
    let prepared = player
        .prepare_blaster(&input, [30.0, 0.0, 0.0], None)
        .unwrap();
    player.play_blaster(&input, true, prepared);
    let out = render(&player);
    assert!(energy(&out, 1) > energy(&out, 0));
    assert!(energy(&out, 1) > 0.01);
    assert!(player
        .prepare_blaster(
            &input,
            [30.0, 0.0, 0.0],
            Some(Instant::now() - Duration::from_secs(1))
        )
        .is_none());
    input.config.blaster = None;
    assert!(player
        .prepare_blaster(&input, [30.0, 0.0, 0.0], None)
        .is_none());
}

#[test]
fn native_visual_observer_discards_hidden_late_repeated_and_pre_restore_cues() {
    let shared = visual::NativeAudioVisual::default();
    let mut input = RoomInput {
        lifecycle: crate::console_bridge::AudioLifecycleState {
            generation: 1,
            running: true,
            suspended: false,
        },
        ..Default::default()
    };
    shared.update(&input);
    let mut reader = visual::VisualReader::default();
    shared.blaster([1.0, 0.0, 0.0]);
    assert!(reader
        .read(&shared, true, true, Instant::now())
        .iter()
        .all(|cue| matches!(
            cue,
            visual::VisualCue::Lifecycle { .. } | visual::VisualCue::Beam { .. }
        )));
    shared.blaster([1.0, 0.0, 0.0]);
    assert_eq!(reader.read(&shared, true, true, Instant::now()).len(), 1);
    assert!(reader.read(&shared, true, true, Instant::now()).is_empty());
    reader.read(&shared, true, false, Instant::now());
    shared.blaster([1.0, 0.0, 0.0]);
    assert!(reader
        .read(&shared, true, true, Instant::now())
        .iter()
        .all(|cue| matches!(
            cue,
            visual::VisualCue::Lifecycle { .. } | visual::VisualCue::Beam { .. }
        )));
    shared.blaster([1.0, 0.0, 0.0]);
    assert!(reader
        .read(&shared, true, true, Instant::now() + Duration::from_secs(1))
        .is_empty());
    input.config = build_audio_payload(Some(&config()), None);
    input.forcefield = 0.06;
    shared.update(&input);
    assert!(reader.read(&shared, true, true, Instant::now()).is_empty());
    input.config.forcefield.as_mut().unwrap().file = "assets/sounds/PhaserLoop.mp3".into();
    input.forcefield = 0.2;
    shared.update(&input);
    assert!(
        reader.read(&shared, true, true, Instant::now()).is_empty(),
        "replacing the configured bed is not an impact"
    );
    shared.blaster([1.0, 0.0, 0.0]);
    input.lifecycle.generation += 1;
    input.lifecycle.suspended = true;
    shared.update(&input);
    assert!(matches!(
        reader.read(&shared, true, true, Instant::now()).as_slice(),
        [
            visual::VisualCue::Lifecycle { active: false },
            visual::VisualCue::Beam { active: false }
        ]
    ));
}
