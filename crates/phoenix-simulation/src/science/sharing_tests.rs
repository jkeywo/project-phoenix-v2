use super::*;
use crate::core::messages::AdmittedCommand;
use crate::science::broad;
use crate::ship::power::ShipPowerSystem;

fn sid() -> SystemId {
    SystemId("sensor-radar".into())
}
fn rig() -> (App, Entity, Entity) {
    rig_with_precision(None)
}
fn rig_with_precision(receiver_error: Option<f32>) -> (App, Entity, Entity) {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    config
        .sensing
        .as_mut()
        .unwrap()
        .broad
        .iter_mut()
        .find(|profile| profile.system == sid())
        .unwrap()
        .sample_interval_secs = 1.0;
    assert!(config.faction.is_some());
    let mut app = App::new();
    app.insert_resource(Time::<Fixed>::from_seconds(0.5))
        .insert_resource(crate::sim_tick::SimTick(0));
    app.add_systems(Update, (apply_commands, broad::tick, deliver).chain());
    let mut ships = Vec::new();
    for (id, x) in [
        ("00000000-0000-0000-0000-000000000151", 0.0),
        ("00000000-0000-0000-0000-000000000152", 20.0),
    ] {
        let mut config = config.clone();
        if x > 0.0 {
            if let Some(error) = receiver_error {
                config
                    .sensing
                    .as_mut()
                    .unwrap()
                    .broad
                    .iter_mut()
                    .find(|profile| profile.system == sid())
                    .unwrap()
                    .position_error = error;
            }
        }
        ships.push(crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::new(x, 0.0, 0.0),
            id.into(),
            None,
        ));
    }
    app.world_mut().flush();
    app.world_mut().spawn((
        EntityUuid("measured-target".into()),
        Transform::from_xyz(8.0, 0.0, 8.0),
        crate::entities::spawner::EntityTagsSection(vec!["station".into()]),
        crate::entities::spawner::EntityTarget(crate::entities::target::TargetSection {
            tags: vec!["station".into()],
            threat_level: Default::default(),
            description: None,
        }),
    ));
    (app, ships[0], ships[1])
}
fn order(app: &mut App, actor: Entity, share: bool, receive: bool) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(actor)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: sid(),
            payload: SystemControlPayload::SetRadarSharing {
                share_own_faction: share,
                receive,
            },
            response_token: None,
            feedback_correlation: None,
        });
}
fn step(app: &mut App, sender: Entity) {
    // The real broad receiver spends once in its ordinary power owner; the
    // sharing pass must neither charge that work again nor mint observations.
    app.world_mut()
        .get_mut::<ShipPowerSystem>(sender)
        .unwrap()
        .0
        .work_continuous(&sid(), 0.5, 1.0)
        .unwrap();
    app.update();
    let mut admitted = app.world_mut().query::<&mut AdmittedCommands>();
    for mut admitted in admitted.iter_mut(app.world_mut()) {
        admitted.0.clear();
    }
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
}
fn received(app: &App, actor: Entity) -> usize {
    app.world().get::<BroadTracks>(actor).unwrap().0[&sid()]
        .tracks
        .values()
        .filter(|track| track.received.is_some())
        .count()
}

#[test]
fn two_real_paid_instruments_refine_only_their_independent_measured_constraints() {
    let (mut app, sender, receiver) = rig_with_precision(Some(3.0));
    // These real positions yield intersecting error-2/error-3 measurement
    // discs whose lens is narrower than either input. Merely receiving two
    // compatible measurements need not provide any extra precision. Keep the
    // target discs clear of both real observing hull returns as well.
    let mut targets = app.world_mut().query::<(&EntityUuid, &mut Transform)>();
    for (id, mut transform) in targets.iter_mut(app.world_mut()) {
        if id.0 == "measured-target" {
            transform.translation = Vec3::new(4.8, 0.0, 20.0);
        }
    }
    order(&mut app, sender, true, true);
    app.world_mut()
        .get_mut::<ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .work_continuous(&sid(), 0.5, 1.0)
        .unwrap();
    step(&mut app, sender);
    let state = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    let before = state.clone();
    let profile = &app.world().get::<BroadProfile>(receiver).unwrap().0[&sid()].profile;
    let raw = state.projection(profile, 0, 0.5).unwrap();
    let fused = projection(
        &app.world().get::<EntityUuid>(receiver).unwrap().0,
        state,
        profile,
        0,
        0.5,
    )
    .unwrap();
    assert!(raw.iter().all(|record| record.samples.len() <= 1));
    assert!(fused
        .iter()
        .any(|record| record.samples.len() == 2 && record.position_error < 2.0));
    let sender_profile = &app.world().get::<BroadProfile>(sender).unwrap().0[&sid()].profile;
    let sender_state = &app.world().get::<BroadTracks>(sender).unwrap().0[&sid()];
    assert!(export_samples(
        &app.world().get::<EntityUuid>(sender).unwrap().0,
        sender_state,
        sender_profile,
        0,
        0.5
    )
    .unwrap()
    .iter()
    .all(|sample| sample.record.samples.len() == 1));
    assert_eq!(
        state, &before,
        "derived refinement does not mutate the canonical owner"
    );
}

#[test]
fn gameplay_link_range_uses_canonical_physics_instead_of_a_render_pose() {
    let (mut app, sender, receiver) = rig();
    order(&mut app, sender, true, true);
    app.world_mut()
        .get_mut::<Transform>(receiver)
        .unwrap()
        .translation
        .x = 1_000_000.0;
    step(&mut app, sender);
    assert!(
        received(&app, receiver) > 0,
        "render pose cannot move the physical Comms endpoint"
    );
    app.world_mut()
        .get_mut::<Transform>(receiver)
        .unwrap()
        .translation
        .x = 20.0;
    app.world_mut()
        .get_mut::<crate::ship::state::ShipPhysics>(receiver)
        .unwrap()
        .x = 1_000_000.0;
    step(&mut app, sender);
    let state = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(state.tracks.values().any(|track| track.received.is_some()));
    assert!(
        state
            .tracks
            .keys()
            .all(|id| state.resolve_current(id).is_none()),
        "physical separation stops delivery and routes while keeping the finding historical"
    );
}

#[test]
fn fitted_radio_shutdown_and_endpoint_jamming_stop_feeds_without_erasing_history() {
    let (mut app, sender, receiver) = rig();
    order(&mut app, sender, true, true);
    step(&mut app, sender);
    assert!(received(&app, receiver) > 0);
    let radio = app
        .world()
        .get::<ShipConfigComponent>(sender)
        .unwrap()
        .0
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::COMMS_KIND)
        .unwrap()
        .id
        .clone();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(sender)
        .unwrap()
        .0
        .set_offline(radio.clone(), true);
    step(&mut app, sender);
    let held = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(held.tracks.values().any(|track| track.received.is_some()));
    assert!(held
        .tracks
        .keys()
        .all(|id| held.resolve_current(id).is_none()));
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(sender)
        .unwrap()
        .0
        .set_offline(radio, false);
    step(&mut app, sender);
    let live = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(live
        .tracks
        .keys()
        .any(|id| live.resolve_current(id).is_some()));
    let source = crate::core::messages::ModifierSource::RegionEffect {
        uuid: uuid::Uuid::from_u128(77),
    };
    app.world_mut()
        .get_mut::<crate::modifiers::ShipModifiers>(receiver)
        .unwrap()
        .add_flag(source.clone(), crate::core::messages::FlagKind::CommsJammed);
    step(&mut app, sender);
    let jammed = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(jammed.tracks.values().any(|track| track.received.is_some()));
    assert!(jammed
        .tracks
        .keys()
        .all(|id| jammed.resolve_current(id).is_none()));
    app.world_mut()
        .get_mut::<crate::modifiers::ShipModifiers>(receiver)
        .unwrap()
        .remove_flag(source, crate::core::messages::FlagKind::CommsJammed);
    step(&mut app, sender);
    let restored = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(restored
        .tracks
        .keys()
        .any(|id| restored.resolve_current(id).is_some()));
}

#[test]
fn actual_ship_feed_requires_deliberate_intent_and_preserves_receiver_control_authority() {
    for source in [
        crate::ship::control_source::ControlSource::Human,
        crate::ship::control_source::ControlSource::Ai,
    ] {
        let (mut app, sender, receiver) = rig();
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(sender)
            .unwrap()
            .0
            .set(sid(), source);
        step(&mut app, sender);
        assert_eq!(received(&app, receiver), 0);
        let receiver_selection = app
            .world()
            .get::<crate::ship::sensors::SensorRadarSelection>(receiver)
            .map(|selection| selection.0.clone());
        order(&mut app, sender, true, true);
        let initial = app
            .world()
            .get::<ShipPowerSystem>(receiver)
            .unwrap()
            .0
            .capture_continuation();
        step(&mut app, sender);
        assert!(received(&app, receiver) > 0);
        assert_eq!(
            app.world()
                .get::<ShipPowerSystem>(receiver)
                .unwrap()
                .0
                .capture_continuation(),
            initial,
            "receiving a copy is not another sensor observation or a new resource charge"
        );
        assert_eq!(
            app.world()
                .get::<crate::ship::sensors::SensorRadarSelection>(receiver)
                .map(|selection| selection.0.clone()),
            receiver_selection
        );
        let states = &app.world().get::<BroadTracks>(receiver).unwrap().0;
        let state = &states[&sid()];
        let profile = &app.world().get::<BroadProfile>(receiver).unwrap().0[&sid()].profile;
        let records = projection(
            &app.world().get::<EntityUuid>(receiver).unwrap().0,
            state,
            profile,
            1,
            0.5,
        )
        .unwrap();
        assert!(records
            .iter()
            .any(|record| record.samples[0].received_tick == Some(1)));
        assert!(records
            .iter()
            .all(|record| !record.contact_id.contains("measured-target")));
        assert!(app.world().get::<BroadTracks>(sender).unwrap().0[&sid()].share_own_faction);
    }
}

#[test]
fn recipient_conceal_blocks_shared_acquisition_and_current_route_refresh() {
    let (mut app, sender, receiver) = rig();
    let receiver_id = app.world().get::<EntityUuid>(receiver).unwrap().0.clone();
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    crate::gm::operations::contact::set(
        &mut runtime.contact_overrides,
        &receiver_id,
        "measured-target",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    app.insert_resource(runtime);
    order(&mut app, sender, true, true);
    step(&mut app, sender);
    assert_eq!(received(&app, receiver), 0);
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        &receiver_id,
        "measured-target",
        crate::gm::operations::contact::ContactMode::Reveal,
    );
    step(&mut app, sender);
    let state = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    let id = state
        .contact_for_source("measured-target")
        .unwrap()
        .to_owned();
    let before = state.tracks[&id].clone();
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        &receiver_id,
        "measured-target",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    step(&mut app, sender);
    let state = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert_eq!(
        state.tracks[&id], before,
        "conceal cannot refresh previously held evidence"
    );
    assert_eq!(state.resolve_current(&id), None);
}

#[test]
fn an_endpoint_clear_comms_obstruction_stops_receipt_then_restores_held_sample_age() {
    let (mut app, sender, receiver) = rig();
    order(&mut app, sender, true, true);
    step(&mut app, sender);
    let barrier = app
        .world_mut()
        .spawn((
            EntityUuid("authored-obstruction".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            crate::entities::spawner::RegionShapeSection(
                crate::regions::shape::RegionShape::Sphere { radius: 3.0 },
            ),
            crate::regions::signal_path::SignalMaterialConfig {
                sensor_absorption_per_unit: 0.0,
                comms_absorption_per_unit: 0.0,
                sensor_opaque: false,
                comms_opaque: true,
            },
        ))
        .id();
    let first = app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()].clone();
    step(&mut app, sender);
    let blocked = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    for (id, track) in &first.tracks {
        if track.received.is_some() {
            assert_eq!(&blocked.tracks[id], track);
            assert!(blocked.resolve_current(id).is_none());
        }
    }
    app.world_mut().despawn(barrier);
    step(&mut app, sender);
    let caught_up = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(caught_up
        .tracks
        .values()
        .any(|track| track
            .received
            .as_ref()
            .is_some_and(|receipt| receipt.last_delivery_tick == 2
                && receipt.provenance.received_tick == Some(2)
                && track.observed_tick == 1)));
}

#[test]
fn wrong_faction_and_real_comms_range_loss_leave_received_findings_historical() {
    let (mut app, sender, receiver) = rig();
    order(&mut app, sender, true, true);
    step(&mut app, sender);
    let held = app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()].clone();
    assert!(held.tracks.values().any(|track| track.received.is_some()));
    let faction = app.world().get::<FactionComponent>(receiver).unwrap().0;
    app.world_mut()
        .entity_mut(receiver)
        .insert(FactionComponent(uuid::Uuid::from_u128(99)));
    step(&mut app, sender);
    let after = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    for (id, track) in &held.tracks {
        if track.received.is_some() {
            assert_eq!(&after.tracks[id], track);
            assert!(after.resolve_current(id).is_none());
        }
    }
    app.world_mut()
        .entity_mut(receiver)
        .insert(FactionComponent(faction));
    app.world_mut()
        .get_mut::<Transform>(receiver)
        .unwrap()
        .translation
        .x = 1_000_000.0;
    app.world_mut()
        .get_mut::<crate::ship::state::ShipPhysics>(receiver)
        .unwrap()
        .x = 1_000_000.0;
    step(&mut app, sender);
    let state = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(state
        .tracks
        .iter()
        .all(|(id, _)| state.resolve_current(id).is_none()));
    app.world_mut()
        .get_mut::<Transform>(receiver)
        .unwrap()
        .translation
        .x = 20.0;
    app.world_mut()
        .get_mut::<crate::ship::state::ShipPhysics>(receiver)
        .unwrap()
        .x = 20.0;
    step(&mut app, sender);
    let restored = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sid()];
    assert!(restored
        .tracks
        .iter()
        .any(|(id, _)| restored.resolve_current(id).is_some()));
    let json = serde_json::to_string(restored).unwrap();
    let decoded: phoenix_sim_gameplay::science::tracks::BroadTrackState =
        serde_json::from_str(&json).unwrap();
    assert_eq!(&decoded, restored);
}

#[test]
fn no_sender_delivery_at_processed_frontier_preserves_tracks_and_paid_work() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, first, second) = rig();
    step(&mut app, first);
    // Repeat only delivery at the already-completed Broad frontier. No extra
    // physical work, age, route refresh, share toggle or receipt is admitted.
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 -= 1;
    let before: Vec<_> = [first, second]
        .into_iter()
        .map(|entity| {
            (
                entity,
                app.world().get::<BroadTracks>(entity).unwrap().0.clone(),
                app.world()
                    .get::<ShipPowerSystem>(entity)
                    .unwrap()
                    .0
                    .clone(),
            )
        })
        .collect();
    app.world_mut().run_system_once(deliver).unwrap();
    for (entity, tracks, power) in before {
        assert_eq!(app.world().get::<BroadTracks>(entity).unwrap().0, tracks);
        assert_eq!(app.world().get::<ShipPowerSystem>(entity).unwrap().0, power);
    }
}
