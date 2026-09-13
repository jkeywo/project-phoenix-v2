use super::*;
use crate::audio_config::{ComputerMessageAudio, ComputerMessageCue, ShipAudioConfig};
use crate::core::narrative::{NarrativeEvent, NarrativeKind};
use std::time::Duration;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .insert_state(GamePhase::InProgress)
        .init_resource::<Time>()
        .add_message::<crate::lobby::OutboundMessage>()
        .add_message::<NarrativeEvent>()
        .add_message::<HudStateChanged>()
        .insert_resource(NativeRoomAudio::with_stores(None, false, None, None))
        .add_plugins(NativeRoomAudioPlugin);
    app.world_mut().spawn((
        LocalShip,
        ShipAudioSection(ShipAudioConfig {
            computer_message: Some(ComputerMessageAudio {
                advisory: Some(ComputerMessageCue {
                    file: "assets/sounds/ui_click.ogg".into(),
                    volume: 0.6,
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    ));
    crate::server::audio_lifecycle::publish_audio_lifecycle(app.world_mut());
    app.update();
    app
}
fn post(app: &mut App, severity: &str) {
    app.world_mut().write_message(
        NarrativeEvent::new(NarrativeKind::ComputerMessagePosted, "fixture")
            .text("text", "Maintain current course.")
            .text("severity", severity),
    );
    app.update();
}
fn player(app: &App) -> RoomPlayer {
    let audio = app.world().resource::<NativeRoomAudio>();
    let mut player = RoomPlayer::default();
    player.mixer = audio.mixer.clone();
    // As the production worker does, prepare authored files before a live cue.
    player.prepare(&audio.control.lock().unwrap().input);
    player
}
fn deliver(
    audio: &NativeRoomAudio,
    player: &mut RoomPlayer,
    ready: bool,
    now: Instant,
) -> Vec<f32> {
    let mut current = audio.control.lock().unwrap();
    player.prepare(&current.input);
    if let Some(severity) = current.take_computer(now) {
        if let Some(prepared) = player.prepare_computer(&current.input, &severity) {
            player.play_computer(&current.input, ready, prepared);
        }
    }
    // This real authored click is longer than a second. Render its complete
    // decoded duration before asserting a second delivery cannot replay it.
    let fixture = decoder::read("assets/sounds/ui_click.ogg").unwrap();
    let frames = (fixture.frames() as f64 * 44100.0 / f64::from(fixture.rate)).ceil() as usize + 1;
    let mut samples = vec![0.0; frames * 2];
    player.mixer.lock().unwrap().render(&mut samples, 44100, 2);
    assert!(samples.iter().all(|sample| sample.is_finite()));
    samples
}
fn sounded(samples: &[f32]) -> bool {
    samples.iter().any(|sample| sample.abs() > 0.0001)
}

#[test]
fn posted_computer_message_reaches_actual_native_samples_each_time_and_omissions_stay_silent() {
    let mut app = app();
    let mut player = player(&app);
    for _ in 0..2 {
        post(&mut app, "advisory");
        let audio = app.world().resource::<NativeRoomAudio>();
        assert!(sounded(&deliver(audio, &mut player, true, Instant::now())));
        assert!(!sounded(&deliver(audio, &mut player, true, Instant::now())));
    }
    for severity in ["info", "warning", "critical", "unknown"] {
        post(&mut app, severity);
        let audio = app.world().resource::<NativeRoomAudio>();
        assert!(audio.control.lock().unwrap().computer.is_none());
        assert!(!sounded(&deliver(audio, &mut player, true, Instant::now())));
    }
    let mut query = app.world_mut().query::<&mut ShipAudioSection>();
    query
        .single_mut(app.world_mut())
        .unwrap()
        .0
        .computer_message = None;
    post(&mut app, "advisory");
    assert!(!sounded(&deliver(
        app.world().resource::<NativeRoomAudio>(),
        &mut player,
        true,
        Instant::now()
    )));
}

#[test]
fn computer_tones_obey_live_mute_loss_deadline_and_restore_without_replay() {
    let mut app = app();
    let mut player = player(&app);
    for bus in ["alerts", "master"] {
        // Start an actual configured voice, then mute it live.
        post(&mut app, "advisory");
        {
            let audio = app.world().resource::<NativeRoomAudio>();
            let mut current = audio.control.lock().unwrap();
            let severity = current.take_computer(Instant::now()).unwrap();
            let tone = player.prepare_computer(&current.input, &severity).unwrap();
            player.play_computer(&current.input, true, tone);
            assert!(player.mixer.lock().unwrap().active("computer"));
        }
        post(&mut app, "advisory");
        app.world_mut()
            .resource_mut::<NativeRoomAudio>()
            .command(&HostLobbyRecord::SetAudioBus {
                bus: bus.into(),
                level_percent: 100,
                muted: true,
            });
        assert!(!player.mixer.lock().unwrap().active("computer"));
        post(&mut app, "advisory");
        app.world_mut()
            .resource_mut::<NativeRoomAudio>()
            .command(&HostLobbyRecord::SetAudioBus {
                bus: bus.into(),
                level_percent: 100,
                muted: false,
            });
        assert!(!sounded(&deliver(
            app.world().resource::<NativeRoomAudio>(),
            &mut player,
            true,
            Instant::now()
        )));
    }
    post(&mut app, "advisory");
    let audio = app.world().resource::<NativeRoomAudio>();
    assert!(!sounded(&deliver(
        audio,
        &mut player,
        false,
        Instant::now()
    )));
    assert!(!sounded(&deliver(audio, &mut player, true, Instant::now())));
    post(&mut app, "advisory");
    assert!(!sounded(&deliver(
        app.world().resource::<NativeRoomAudio>(),
        &mut player,
        true,
        Instant::now() + Duration::from_secs(1)
    )));
    post(&mut app, "advisory");
    let audio = app.world().resource::<NativeRoomAudio>();
    let mut input = audio.control.lock().unwrap().input.clone();
    input.lifecycle.generation += 1;
    input.lifecycle.suspended = true;
    audio.apply_input(input.clone());
    assert!(audio.control.lock().unwrap().computer.is_none());
    input.lifecycle.suspended = false;
    audio.apply_input(input);
    assert!(!sounded(&deliver(audio, &mut player, true, Instant::now())));
}
