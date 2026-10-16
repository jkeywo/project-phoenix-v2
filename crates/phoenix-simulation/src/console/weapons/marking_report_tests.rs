//! Actual paid Radio/Broad/report owner pipeline. Attachment is this isolated
//! owner's initial condition; flight/contact and whole-host proof are separate.
use super::*;
use crate::ship::state::ShipPhysics;
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::{identity::SystemId, marking::*};

fn id(tick: u64, seq: u64) -> String {
    crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, tick, seq).render()
}
fn sensor() -> SystemId {
    SystemId("sensor-radar".into())
}
fn radio() -> SystemId {
    SystemId("comms".into())
}
fn fixture(monitored: bool) -> (App, Entity, Entity) {
    let mut receiver_config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    if monitored {
        receiver_config
            .comms
            .as_mut()
            .unwrap()
            .radio
            .as_mut()
            .unwrap()
            .traffic = Some(phoenix_model::comms_traffic::CommsTrafficConfig {
            listening: Some(phoenix_model::comms_traffic::CommsListeningConfig {
                simultaneous_channels: 1,
                priority: vec!["fleet".into(), "public".into()],
            }),
            alert: None,
        });
        receiver_config = crate::entities::config::EntityConfig::from_toml(
            &toml::to_string(&receiver_config.to_toml_value().unwrap()).unwrap(),
        )
        .unwrap();
    }
    let mut marker_config =
        crate::entities::include_resolve::load_entity_config("assets/entities/marking_beacon.toml")
            .unwrap();
    marker_config.faction = receiver_config.faction;
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(10))
        .insert_resource(Time::<Fixed>::from_seconds(0.5))
        .init_resource::<crate::comms::server::CommsInboxRes>()
        .init_resource::<crate::comms::server::CommsRuntime>()
        .init_resource::<crate::server_app::SimOutbox>();
    // Use the actual accepted parent's shape to establish this owner's initial
    // surface relation; no source pose/velocity comes from a sensor shortcut.
    let parent_pose = ShipPhysics {
        x: 8.0,
        ..Default::default()
    };
    let parent_point = Vec3::new(parent_pose.x, parent_pose.y, parent_pose.z);
    let point =
        crate::entities::spawner::physical_collider(receiver_config.collider.as_ref().unwrap())
            .project_point(parent_point, Quat::IDENTITY, Vec3::ZERO, false)
            .point;
    let offset = (point - parent_point).to_array();
    let target = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &receiver_config,
        parent_point,
        id(0, 0),
        None,
    );
    let source = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &marker_config,
        point,
        id(3, 0),
        None,
    );
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &receiver_config,
        Vec3::new(10_000.0, 0.0, 0.0),
        id(0, 1),
        None,
    );
    app.world_mut().flush();
    app.world_mut().entity_mut(target).insert(parent_pose);
    app.world_mut().entity_mut(receiver).insert(ShipPhysics {
        x: 10_000.0,
        ..Default::default()
    });
    app.world_mut()
        .entity_mut(source)
        .insert(ShipMarkingAttachment(MarkingAttachment {
            round: crate::world_id::WorldId::new(crate::world_id::IdNamespace::Projectile, 2, 0)
                .render(),
            source: id(0, 4),
            payload: FrozenMarkingPayload {
                recipe: MarkingPayloadConfig {
                    template: "assets/entities/marking_beacon.toml".into(),
                },
                faction: marker_config.faction.unwrap(),
            },
            contact_tick: 3,
            location: MarkingLocation::Attached {
                subject: id(0, 0),
                offset,
            },
            report: None,
        }));
    (app, source, receiver)
}
fn pay_and_listen(app: &mut App, source: Entity) {
    app.world_mut()
        .run_system_once(crate::ship::energy_runtime::begin_work_interval)
        .unwrap();
    let grant = app
        .world_mut()
        .get_mut::<ShipPowerSystem>(source)
        .unwrap()
        .0
        .work_continuous(&sensor(), 0.5, 1.0)
        .unwrap();
    assert_eq!(grant.fraction, 1.0);
    app.world_mut()
        .run_system_once(crate::comms::radio::tick_radio)
        .unwrap();
}
fn acquire_and_report(app: &mut App) {
    app.world_mut()
        .run_system_once(crate::science::broad::tick)
        .unwrap();
    app.world_mut().run_system_once(tick).unwrap();
}
fn energy(app: &App, source: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(source)
        .unwrap()
        .0
        .network()
        .consumers[&radio()]
        .energy
}
fn report(app: &App, source: Entity) -> Option<MarkingReportFrontier> {
    app.world()
        .get::<ShipMarkingAttachment>(source)
        .unwrap()
        .0
        .report
        .clone()
}
fn received(app: &App, receiver: Entity) -> Vec<crate::science::broad::BroadTrack> {
    app.world().get::<BroadTracks>(receiver).unwrap().0[&sensor()]
        .tracks
        .values()
        .filter(|track| track.received.is_some())
        .cloned()
        .collect()
}
#[test]
fn actual_late_paid_route_uses_current_pose_and_preserves_only_original_sensor_evidence() {
    let (mut app, source, receiver) = fixture(false);
    let before = energy(&app, source);
    pay_and_listen(&mut app, source);
    // A real late placement after the ordinary Radio tick: the stale Radio
    // endpoint was out of range, but this current physical route is valid.
    app.world_mut().get_mut::<ShipPhysics>(receiver).unwrap().x = 30.0;
    acquire_and_report(&mut app);
    assert_eq!(before - energy(&app, source), 1.5); // one 0.5s listen + one send
    let sent = report(&app, source).unwrap();
    assert_eq!(sent.observed_tick, 10);
    assert_eq!(sent.emitted_tick, 10);
    let held = received(&app, receiver);
    assert_eq!(held.len(), 1);
    let sample = held[0].received.as_ref().unwrap();
    assert_eq!(sample.provenance.sample_id, sent.sample);
    assert_eq!(sample.provenance.observed_tick, 10);
    assert_eq!(sample.provenance.received_tick, Some(10));
    assert_eq!(sample.route, None);
    assert!(
        held[0].observation.is_none()
            && held[0].identity.is_none()
            && held[0].classification.is_none()
            && held[0].assessment.is_none()
    );
    assert!(held[0].velocity.is_none()); // one actual sample cannot infer motion
    let state = &app.world().get::<BroadTracks>(receiver).unwrap().0[&sensor()];
    assert_eq!(state.resolve_current(&held[0].contact_id), None);
    let encoded = serde_json::to_string(&sample.provenance).unwrap();
    assert!(!encoded.contains(&id(0, 0)) && !encoded.contains(&id(3, 0)));
    // Re-running the consumer in this interval neither repays nor refreshes.
    let paid = energy(&app, source);
    app.world_mut().run_system_once(tick).unwrap();
    assert_eq!(energy(&app, source), paid);
    assert_eq!(report(&app, source), Some(sent));
    assert_eq!(received(&app, receiver), held);
}
#[test]
fn blocked_receiver_never_changes_source_payment_or_retries_an_old_sample() {
    let (mut app, source, receiver) = fixture(true);
    app.world_mut().get_mut::<ShipPhysics>(receiver).unwrap().x = 30.0;
    let before = energy(&app, source);
    pay_and_listen(&mut app, source);
    acquire_and_report(&mut app);
    assert_eq!(before - energy(&app, source), 1.5);
    assert!(report(&app, source).is_some());
    assert!(received(&app, receiver).is_empty()); // listens to fleet, not public
    let old = report(&app, source);
    // Existing admitted listening-order control, consumed by actual Radio.
    app.world_mut()
        .get_mut::<crate::core::messages::AdmittedCommands>(receiver)
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: radio(),
            payload: crate::core::messages::SystemControlPayload::SetCommsListeningOrder {
                priority: vec!["public".into(), "fleet".into()],
            },
            response_token: None,
            feedback_correlation: None,
        });
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 11;
    pay_and_listen(&mut app, source);
    // No fresh sensor sample yet: hearing the channel is not an observation.
    app.world_mut().run_system_once(tick).unwrap();
    assert!(received(&app, receiver).is_empty());
    assert_eq!(report(&app, source), old);
    acquire_and_report(&mut app);
    let heard = received(&app, receiver);
    assert_eq!(heard.len(), 1);
    assert_eq!(
        heard[0].received.as_ref().unwrap().provenance.observed_tick,
        11
    );
    assert!(heard[0].velocity.is_some()); // now two paid distinct samples
}
