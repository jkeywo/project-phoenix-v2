use super::*;
use crate::audio_config::{ForcefieldAudio, ShipAudioConfig};
use crate::core::messages::DeliveryClass;
use crate::lobby::Target;

#[test]
fn an_empty_boundary_clears_old_config_without_latching_out_a_late_ship() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.init_resource::<Messages<AudioConfigChanged>>();
    world.init_resource::<AudioConfigSent>();
    world.insert_resource(State::new(GamePhase::InProgress));
    rebase_audio_presentation(&mut world);
    assert!(!world.resource::<AudioConfigSent>().0);
    let clear: Vec<_> = world
        .resource_mut::<Messages<AudioConfigChanged>>()
        .drain()
        .collect();
    assert_eq!(clear.len(), 1);
    assert_eq!(
        clear[0].json,
        codec::encode_audio_config(&build_audio_payload(None, None)).unwrap()
    );
    let configured = ShipAudioConfig {
        forcefield: Some(forcefield_cfg()),
        ..Default::default()
    };
    let expected =
        codec::encode_audio_config(&build_audio_payload(Some(&configured), None)).unwrap();
    world.spawn((LocalShip, ShipAudioSection(configured)));
    world.run_system_once(push_audio_config).unwrap();
    let published: Vec<_> = world
        .resource_mut::<Messages<AudioConfigChanged>>()
        .drain()
        .collect();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].json, expected);
    assert!(world.resource::<AudioConfigSent>().0);
}

fn forcefield_cfg() -> ForcefieldAudio {
    ForcefieldAudio {
        file: "assets/sounds/ForcefieldHit.mp3".into(),
        base_volume: 0.06,
        spike_volume: 0.8,
        damage_threshold: 1.0,
        damage_full_spike: 30.0,
        decay_rate_per_sec: 1.5,
        source: ForcefieldSource::Shield,
    }
}

/// Minimal app: the systems under test plus a LocalShip carrying an
/// audio section. No rendering, no full sim.
fn test_app() -> App {
    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .add_message::<AudioCueEvent>()
        .init_resource::<ForcefieldAudioState>()
        .init_resource::<Time>();
    app.add_systems(
        Update,
        (
            process_forcefield_damage,
            drive_forcefield_level.after(process_forcefield_damage),
            push_blaster_cues,
        ),
    );
    app.world_mut().spawn((
        LocalShip,
        ShipPhysics::default(),
        ShipAudioSection(ShipAudioConfig {
            forcefield: Some(forcefield_cfg()),
            ..Default::default()
        }),
    ));
    app
}

/// Advance the clock, then run one update.
///
/// A bare `App` with `init_resource::<Time>()` never advances on its own,
/// so `delta_secs()` would stay 0 and the decay step would be a silent
/// no-op — the test would pass while testing nothing.
fn tick(app: &mut App, dt_secs: f32) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(dt_secs));
    app.update();
}

fn send_damage(app: &mut App, hull: f32, shield: f32) {
    app.world_mut()
        .resource_mut::<Messages<OutboundMessage>>()
        .write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::DamageTaken { hull, shield },
            // Matches `drain_lobby_outbox`, which marks every drained
            // message Reliable.
            delivery: DeliveryClass::Reliable,
        });
}

fn intensity(app: &App) -> f32 {
    app.world().resource::<ForcefieldAudioState>().intensity
}

#[test]
fn full_shield_hit_spikes_intensity_to_one() {
    let mut app = test_app();
    send_damage(&mut app, 0.0, 30.0);
    // A tiny dt so the same update's decay step barely bites.
    tick(&mut app, 0.001);
    assert!(intensity(&app) > 0.99, "got {}", intensity(&app));
}

#[test]
fn damage_below_threshold_leaves_intensity_untouched() {
    let mut app = test_app();
    send_damage(&mut app, 0.0, 0.5);
    tick(&mut app, 0.016);
    assert_eq!(intensity(&app), 0.0);
}

#[test]
fn hull_damage_ignored_when_source_is_shield() {
    let mut app = test_app();
    send_damage(&mut app, 50.0, 0.0);
    tick(&mut app, 0.016);
    assert_eq!(intensity(&app), 0.0);
}

#[test]
fn intensity_decays_to_zero_and_stops() {
    let mut app = test_app();
    send_damage(&mut app, 0.0, 30.0);
    tick(&mut app, 0.001);
    let peak = intensity(&app);

    // decay_rate is 1.5/sec, so ~0.67s of silence should reach zero.
    let mut prev = peak;
    for _ in 0..10 {
        tick(&mut app, 0.1);
        let now = intensity(&app);
        assert!(
            now <= prev,
            "intensity rose without damage: {prev} -> {now}"
        );
        assert!(now >= 0.0, "intensity went negative: {now}");
        prev = now;
    }
    assert_eq!(prev, 0.0, "should have fully decayed to the bed");
    assert!(peak > prev, "never decayed at all");
}

#[test]
fn a_bigger_hit_overrides_a_decaying_tail() {
    let mut app = test_app();
    send_damage(&mut app, 0.0, 5.0);
    tick(&mut app, 0.001);
    let small = intensity(&app);

    send_damage(&mut app, 0.0, 30.0);
    tick(&mut app, 0.001);
    let big = intensity(&app);
    assert!(
        big > small,
        "big hit did not override tail: {small} -> {big}"
    );
}

#[test]
fn a_glancing_hit_does_not_quieten_a_loud_tail() {
    // `.max()` rather than assignment: a 2 HP scratch mid-decay must not
    // cut the tail of a 30 HP hit short.
    let mut app = test_app();
    send_damage(&mut app, 0.0, 30.0);
    tick(&mut app, 0.001);
    let loud = intensity(&app);

    send_damage(&mut app, 0.0, 2.0);
    tick(&mut app, 0.001);
    let after = intensity(&app);
    assert!(
        after > loud - 0.01,
        "glancing hit cut the tail: {loud} -> {after}"
    );
}

#[test]
fn ship_without_forcefield_config_never_spikes() {
    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .init_resource::<ForcefieldAudioState>()
        .init_resource::<Time>()
        .add_systems(Update, process_forcefield_damage);
    app.world_mut().spawn((
        LocalShip,
        ShipAudioSection(ShipAudioConfig::default()), // no [audio.forcefield]
    ));
    send_damage(&mut app, 0.0, 30.0);
    app.update();
    assert_eq!(
        app.world().resource::<ForcefieldAudioState>().intensity,
        0.0
    );
}

#[test]
fn blaster_cue_carries_listener_relative_position() {
    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .add_message::<AudioCueEvent>()
        .init_resource::<Time>()
        .add_systems(Update, push_blaster_cues);
    app.world_mut().spawn((
        LocalShip,
        ShipPhysics::default(), // at origin, yaw 0 (facing -Z)
        ShipAudioSection(ShipAudioConfig {
            blaster: Some(crate::audio_config::BlasterAudio {
                file: "assets/sounds/Blaster.mp3".into(),
                volume: 0.9,
                ref_distance: 30.0,
                max_distance: 800.0,
                rolloff_factor: 1.2,
                distance_model: crate::audio_config::DistanceModel::Inverse,
                panning_model: crate::audio_config::PanningModel::EqualPower,
            }),
            ..Default::default()
        }),
    ));
    // A shot 10 units due East of a north-facing ship is off the
    // starboard beam: +X in the listener's frame.
    app.world_mut()
        .resource_mut::<Messages<OutboundMessage>>()
        .write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BlasterFired {
                bank: "fore".into(),
                source_uuid: "npc-1".into(),
                projectile_id: "p1".into(),
                x: 10.0,
                z: 0.0,
                heading: 0.0,
                visual_scale: 1.0,
            },
            delivery: DeliveryClass::Reliable,
        });
    app.update();

    let cues = app.world().resource::<Messages<AudioCueEvent>>();
    let mut cursor = cues.get_cursor();
    let sent: Vec<_> = cursor.read(cues).collect();
    assert_eq!(sent.len(), 1, "expected exactly one blaster cue");
    let cue: AudioCue = serde_json::from_str(&sent[0].json).expect("valid cue JSON");
    assert_eq!(cue.kind, "blaster");
    assert!((cue.x - 10.0).abs() < 1e-4, "got x={}", cue.x);
    assert!(cue.z.abs() < 1e-4, "got z={}", cue.z);
}

#[test]
fn blaster_cue_culled_beyond_max_distance() {
    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .add_message::<AudioCueEvent>()
        .init_resource::<Time>()
        .add_systems(Update, push_blaster_cues);
    app.world_mut().spawn((
        LocalShip,
        ShipPhysics::default(),
        ShipAudioSection(ShipAudioConfig {
            blaster: Some(crate::audio_config::BlasterAudio {
                file: "assets/sounds/Blaster.mp3".into(),
                volume: 0.9,
                ref_distance: 30.0,
                max_distance: 800.0,
                rolloff_factor: 1.2,
                distance_model: crate::audio_config::DistanceModel::Inverse,
                panning_model: crate::audio_config::PanningModel::EqualPower,
            }),
            ..Default::default()
        }),
    ));
    app.world_mut()
        .resource_mut::<Messages<OutboundMessage>>()
        .write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::BlasterFired {
                bank: "fore".into(),
                source_uuid: "npc-far".into(),
                projectile_id: "p2".into(),
                x: 5000.0, // well beyond max_distance
                z: 0.0,
                heading: 0.0,
                visual_scale: 1.0,
            },
            delivery: DeliveryClass::Reliable,
        });
    app.update();

    let cues = app.world().resource::<Messages<AudioCueEvent>>();
    let mut cursor = cues.get_cursor();
    assert_eq!(
        cursor.read(cues).count(),
        0,
        "inaudible shot should not allocate a JS audio node"
    );
}

// ── Ship's-computer tone (issue #1342) ─────────────────────────────

#[test]
fn blaster_audience_uses_current_canonical_forced_view_without_replaying_suppressed_shots() {
    use crate::gm_presentation::{PresentationView, ShipPresentation, TimedView};
    let mut app = App::new();
    app.add_message::<OutboundMessage>()
        .add_message::<AudioCueEvent>()
        .insert_resource(crate::sim_tick::SimTick(1))
        .add_systems(Update, push_blaster_cues);
    let mut content = crate::world::server::WorldContentRuntime::default();
    crate::gm_contact::set(
        &mut content.contact_overrides,
        "observer",
        "source",
        crate::gm_contact::ContactMode::Conceal,
    );
    content.presentation.insert(
        "observer".into(),
        ShipPresentation {
            forced_view: Some(TimedView {
                view: PresentationView::SensorsRadar,
                until_tick: 10,
            }),
            card: None,
        },
    );
    app.insert_resource(content);
    app.world_mut().spawn((
        LocalShip,
        crate::entities::spawner::EntityUuid("observer".into()),
        crate::ship::state::ShipViewMode::default(),
        ShipPhysics::default(),
        ShipAudioSection(ShipAudioConfig {
            blaster: Some(crate::audio_config::BlasterAudio {
                file: "assets/sounds/Blaster.mp3".into(),
                volume: 0.9,
                ref_distance: 30.0,
                max_distance: 800.0,
                rolloff_factor: 1.2,
                distance_model: crate::audio_config::DistanceModel::Inverse,
                panning_model: crate::audio_config::PanningModel::EqualPower,
            }),
            ..Default::default()
        }),
    ));
    let shot = || OutboundMessage {
        target: Target::All,
        delivery: DeliveryClass::Reliable,
        msg: ServerMessage::BlasterFired {
            bank: "fore".into(),
            source_uuid: "source".into(),
            projectile_id: "shot".into(),
            x: 30.0,
            z: 0.0,
            heading: 0.0,
            visual_scale: 1.0,
        },
    };
    app.world_mut().write_message(shot());
    app.update();
    assert!(app.world().resource::<Messages<AudioCueEvent>>().is_empty());
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 10;
    app.update();
    assert!(
        app.world().resource::<Messages<AudioCueEvent>>().is_empty(),
        "expired force cannot replay the suppressed occurrence"
    );
    app.world_mut().write_message(shot());
    app.update();
    let cues: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<AudioCueEvent>>()
        .drain()
        .collect();
    assert_eq!(
        cues.len(),
        1,
        "ordinary camera returns to its public combat geometry"
    );
    assert!(!cues[0].json.contains("source"));
}

fn app_with_computer_message_audio(cfg: crate::audio_config::ComputerMessageAudio) -> App {
    let mut app = App::new();
    app.add_message::<NarrativeEvent>()
        .add_message::<AudioCueEvent>()
        .add_systems(Update, push_computer_message_cue);
    app.world_mut().spawn((
        LocalShip,
        ShipAudioSection(ShipAudioConfig {
            computer_message: Some(cfg),
            ..Default::default()
        }),
    ));
    app
}

fn write_posted(app: &mut App, severity: &str) {
    app.world_mut()
        .resource_mut::<Messages<NarrativeEvent>>()
        .write(
            NarrativeEvent::new(NarrativeKind::ComputerMessagePosted, "hail_debris")
                .text("text", "world.probe.computer_message.text")
                .text("severity", severity),
        );
}

fn sent_cues(app: &App) -> Vec<AudioCue> {
    let cues = app.world().resource::<Messages<AudioCueEvent>>();
    let mut cursor = cues.get_cursor();
    cursor
        .read(cues)
        .map(|c| serde_json::from_str(&c.json).expect("valid cue JSON"))
        .collect()
}

#[test]
fn a_posted_message_with_a_configured_severity_fires_a_cue() {
    let mut app = app_with_computer_message_audio(crate::audio_config::ComputerMessageAudio {
        critical: Some(crate::audio_config::ComputerMessageCue {
            file: "assets/sounds/ComputerCritical.mp3".into(),
            volume: 0.8,
        }),
        ..Default::default()
    });
    write_posted(&mut app, "critical");
    app.update();

    let cues = sent_cues(&app);
    assert_eq!(cues.len(), 1, "{cues:?}");
    assert_eq!(cues[0].kind, "computer_message");
    assert_eq!(cues[0].severity.as_deref(), Some("critical"));
    // Not positional.
    assert_eq!(cues[0].x, 0.0);
    assert_eq!(cues[0].y, 0.0);
    assert_eq!(cues[0].z, 0.0);
}

#[test]
fn missing_configuration_for_the_severity_is_silent() {
    // AC4: a severity with no configured tone plays nothing — not a
    // fallback to some other severity's file.
    let mut app = app_with_computer_message_audio(crate::audio_config::ComputerMessageAudio {
        critical: Some(crate::audio_config::ComputerMessageCue {
            file: "assets/sounds/ComputerCritical.mp3".into(),
            volume: 0.8,
        }),
        ..Default::default()
    });
    write_posted(&mut app, "advisory");
    app.update();
    assert!(sent_cues(&app).is_empty());
}

#[test]
fn a_ship_with_no_computer_message_config_is_entirely_silent() {
    let mut app = App::new();
    app.add_message::<NarrativeEvent>()
        .add_message::<AudioCueEvent>()
        .add_systems(Update, push_computer_message_cue);
    app.world_mut().spawn((
        LocalShip,
        ShipAudioSection(ShipAudioConfig::default()), // no [audio.computer_message]
    ));
    write_posted(&mut app, "critical");
    app.update();
    assert!(sent_cues(&app).is_empty());
}

#[test]
fn a_replacement_at_the_same_severity_still_fires_a_fresh_cue() {
    // "Replacement plays the new tone even at the same severity" — the
    // issue's own wording. Two `ComputerMessagePosted` events at the same
    // severity in the same batch must yield two cues, not one suppressed
    // by "nothing changed".
    let mut app = app_with_computer_message_audio(crate::audio_config::ComputerMessageAudio {
        advisory: Some(crate::audio_config::ComputerMessageCue {
            file: "assets/sounds/ComputerAdvisory.mp3".into(),
            volume: 0.5,
        }),
        ..Default::default()
    });
    write_posted(&mut app, "advisory");
    write_posted(&mut app, "advisory");
    app.update();
    assert_eq!(sent_cues(&app).len(), 2);
}

#[test]
fn other_narrative_kinds_do_not_fire_a_cue() {
    let mut app = app_with_computer_message_audio(crate::audio_config::ComputerMessageAudio {
        info: Some(crate::audio_config::ComputerMessageCue {
            file: "assets/sounds/ComputerInfo.mp3".into(),
            volume: 0.3,
        }),
        ..Default::default()
    });
    app.world_mut()
        .resource_mut::<Messages<NarrativeEvent>>()
        .write(NarrativeEvent::new(
            NarrativeKind::ComputerMessageCleared,
            "hail_debris",
        ));
    app.update();
    assert!(sent_cues(&app).is_empty());
}
