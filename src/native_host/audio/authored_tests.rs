use super::*;
use crate::{
    command_admission::{log::ShipKey, HostSlot},
    entities::spawner::EntityUuid,
    gm_action::*,
    gm_presentation::{self, sound::LiveSoundCue, PresentationCue},
    lockstep::FleetSlotOf,
    ship::state::ShipViewMode,
    sim_tick::SimTick,
    world::server::WorldContentRuntime,
};
use bevy::ecs::system::RunSystemOnce;
use std::time::Duration;

fn app() -> (App, RoomPlayer) {
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .insert_state(GamePhase::InProgress)
        .init_resource::<Time>()
        .insert_resource(SimTick(42))
        .init_resource::<SimulationPaused>()
        .init_resource::<GmActionJournal>()
        .init_resource::<GmActionLog>()
        .init_resource::<WorldContentRuntime>()
        .add_message::<crate::lobby::OutboundMessage>()
        .add_message::<crate::core::narrative::NarrativeEvent>()
        .add_message::<HudStateChanged>()
        .insert_resource(NativeRoomAudio::with_stores(None, false, None, None))
        .add_plugins(NativeRoomAudioPlugin);
    app.world_mut().spawn((
        LocalShip,
        EntityUuid("alpha".into()),
        crate::server_app::Ship,
        FleetSlotOf(HostSlot(1)),
        ShipViewMode::default(),
    ));
    app.world_mut().spawn((
        EntityUuid("bravo".into()),
        crate::server_app::Ship,
        FleetSlotOf(HostSlot(2)),
        ShipViewMode::default(),
    ));
    app.update();
    let audio = app.world().resource::<NativeRoomAudio>();
    let mut player = RoomPlayer::default();
    player.mixer = audio.mixer.clone();
    player.prepare(&audio.control.lock().unwrap().input);
    (app, player)
}
fn command(ship: &str, id: &str, source: Option<&str>) -> PresentationCue {
    let raw = format!(
        r#"{{"operator_id":"gm","correlation":"live","action":"presentation","ship":"{ship}","cue":{{"sound":{{"id":"{id}","source":{}}}}}}}"#,
        source.map_or("null".into(), |s| format!("\"{s}\""))
    );
    let request = codec::decode_gm_action_request(&raw).unwrap();
    let GmAction::Presentation { cue, .. } = request.action else {
        panic!("authored-action");
    };
    cue
}
fn gm(app: &mut App, sequence: u64, ship: &str, cue: PresentationCue) {
    let grant = GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm".into(),
        correlation: GmActionId::new(format!("sound-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: 42,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::Presentation {
            ship: ShipKey(ship.into()),
            cue,
        },
    };
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant)
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    app.update();
}
fn deliver(
    audio: &NativeRoomAudio,
    player: &mut RoomPlayer,
    ready: bool,
    now: Instant,
) -> Vec<f32> {
    let request = audio.control.lock().unwrap().clone();
    let prepared = request.authored.as_ref().and_then(|(at, cue)| {
        player.prepare_authored(
            &request.input,
            &cue.definition,
            *at + Duration::from_millis(250),
        )
    });
    let mut current = audio.control.lock().unwrap();
    assert_eq!(current.authored, request.authored);
    let played = current.take_authored(now);
    if played.is_some() {
        if let Some(prepared) = prepared {
            player.play_authored(&current.input, ready, prepared);
        }
    }
    let frames = played
        .as_ref()
        .map(|cue| decoder::read(&cue.definition.file).unwrap())
        .map_or(2048, |pcm| {
            (pcm.frames() as f64 * 44100.0 / f64::from(pcm.rate)).ceil() as usize + 8192
        });
    let mut samples = vec![0.0; frames * 2];
    player.mixer.lock().unwrap().render(&mut samples, 44100, 2);
    samples
}
fn sounded(samples: &[f32]) -> bool {
    samples.iter().any(|value| value.abs() > 0.00001)
}

#[test]
fn gm_and_scenario_authored_cues_reach_real_native_pcm_once_without_canonical_presentation_state() {
    let (mut app, mut player) = app();
    let audio = app.world().resource::<NativeRoomAudio>();
    let mut reader = visual::VisualReader::default();
    reader.read(&audio.visuals, true, true, Instant::now());
    gm(
        &mut app,
        1,
        "alpha",
        command("alpha", "weapons", Some("bravo")),
    );
    let audio = app.world().resource::<NativeRoomAudio>();
    assert!(reader
        .read(&audio.visuals, true, true, Instant::now())
        .iter()
        .any(|cue| matches!(
            cue,
            visual::VisualCue::Authored {
                equivalent: Some(_)
            }
        )));
    let captured = audio
        .control
        .lock()
        .unwrap()
        .authored
        .as_ref()
        .unwrap()
        .1
        .clone();
    assert!(sounded(&deliver(audio, &mut player, true, Instant::now())));
    audio.authored_sound(captured);
    assert!(!sounded(&deliver(audio, &mut player, true, Instant::now())));
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .presentation
        .is_empty());
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0].outcome,
        GmActionOutcome::Applied
    );
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0].operator_id,
        "gm"
    );
    // Re-running the canonical drain cannot reapply a consumed grant.
    app.world_mut().run_system_once(apply_due_actions).unwrap();
    app.update();
    assert!(app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .authored
        .is_none());
    app.world_mut()
        .run_system_once_with(
            gm_presentation::apply_scenario_command,
            ("alpha".into(), command("alpha", "weapons", Some("bravo"))),
        )
        .unwrap();
    app.update();
    assert!(sounded(&deliver(
        app.world().resource::<NativeRoomAudio>(),
        &mut player,
        true,
        Instant::now()
    )));
    // The cue is presentation output only. The normal GM journal fact is the
    // canonical change, and draining sound never adds another digest field.
    let digest = crate::sim_digest::world_digest(app.world());
    app.update();
    assert_eq!(crate::sim_digest::world_digest(app.world()), digest);
}

#[test]
fn authored_refusals_recipient_and_information_gate_do_not_disclose_or_queue_sound() {
    let (mut app, _) = app();
    for (sequence, id) in [(1, "missing"), (2, "private-alert")] {
        gm(&mut app, sequence, "alpha", command("alpha", id, None));
        assert_eq!(
            app.world()
                .resource::<GmActionLog>()
                .entries()
                .last()
                .unwrap()
                .outcome,
            GmActionOutcome::Refused
        );
        assert!(app
            .world()
            .resource::<NativeRoomAudio>()
            .control
            .lock()
            .unwrap()
            .authored
            .is_none());
    }
    gm(&mut app, 3, "bravo", command("bravo", "weapons", None));
    assert!(app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .authored
        .is_none());
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .presentation
        .insert(
            "alpha".into(),
            gm_presentation::ShipPresentation {
                forced_view: Some(gm_presentation::TimedView {
                    view: gm_presentation::PresentationView::SensorsRadar,
                    until_tick: 100,
                }),
                ..Default::default()
            },
        );
    crate::gm_contact::set(
        &mut app
            .world_mut()
            .resource_mut::<WorldContentRuntime>()
            .contact_overrides,
        "alpha",
        "bravo",
        crate::gm_contact::ContactMode::Conceal,
    );
    gm(
        &mut app,
        4,
        "alpha",
        command("alpha", "weapons", Some("bravo")),
    );
    assert_eq!(
        app.world()
            .resource::<GmActionLog>()
            .entries()
            .last()
            .unwrap()
            .outcome,
        GmActionOutcome::Applied
    );
    assert!(app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .authored
        .is_none());
    // Switching to the public Camera is not permission to replay a missed cue.
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .presentation
        .clear();
    app.update();
    assert!(app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .authored
        .is_none());
}

#[test]
fn authored_mute_loss_expiry_and_restore_consume_current_native_request() {
    let (mut app, mut player) = app();
    gm(&mut app, 1, "alpha", command("alpha", "weapons", None));
    let audio = app.world().resource::<NativeRoomAudio>();
    let captured: LiveSoundCue = audio
        .control
        .lock()
        .unwrap()
        .authored
        .as_ref()
        .unwrap()
        .1
        .clone();
    assert!(!sounded(&deliver(
        audio,
        &mut player,
        false,
        Instant::now()
    )));
    audio.authored_sound(captured.clone());
    assert!(audio.control.lock().unwrap().authored.is_none());
    gm(&mut app, 2, "alpha", command("alpha", "weapons", None));
    assert!(!sounded(&deliver(
        app.world().resource::<NativeRoomAudio>(),
        &mut player,
        true,
        Instant::now() + Duration::from_secs(1)
    )));
    gm(&mut app, 3, "alpha", command("alpha", "weapons", None));
    app.world_mut()
        .resource_mut::<NativeRoomAudio>()
        .command(&HostLobbyRecord::SetAudioBus {
            bus: "effects".into(),
            level_percent: 100,
            muted: true,
        });
    assert!(app
        .world()
        .resource::<NativeRoomAudio>()
        .control
        .lock()
        .unwrap()
        .authored
        .is_none());
    app.world_mut()
        .resource_mut::<NativeRoomAudio>()
        .command(&HostLobbyRecord::SetAudioBus {
            bus: "effects".into(),
            level_percent: 100,
            muted: false,
        });
    assert!(!sounded(&deliver(
        app.world().resource::<NativeRoomAudio>(),
        &mut player,
        true,
        Instant::now()
    )));
    gm(&mut app, 4, "alpha", command("alpha", "weapons", None));
    let audio = app.world().resource::<NativeRoomAudio>();
    let mut input = audio.control.lock().unwrap().input.clone();
    input.lifecycle.generation += 1;
    input.lifecycle.suspended = true;
    audio.apply_input(input.clone());
    audio.authored_sound(captured);
    assert!(audio.control.lock().unwrap().authored.is_none());
    assert!(!sounded(&deliver(audio, &mut player, true, Instant::now())));
}
