//! Owner-level accepted-copy setup, not an end-to-end radio/session proof.
use super::*;
use phoenix_model::{
    comms_channel::{CommsAudience, CommsTransmission},
    comms_request::{
        CommsRadioContent, CommsRequestDecision, CommsRequestProgress, CommsRequestReference,
        GenericCommsRequest,
    },
    craft::CraftReceptionOrder,
};
fn bay() -> SystemId {
    SystemId("reception-bay".into())
}
fn fixture() -> (App, Entity, Entity, CraftReceptionOrder) {
    let config = super::super::super::include_resolve::load_entity_config(
        "assets/entities/reception_outpost.toml",
    )
    .unwrap();
    let mut app = App::new();
    register(&mut app);
    app.insert_resource(crate::sim_tick::SimTick(4));
    app.insert_resource(Time::<Fixed>::from_seconds(1.0 / 60.0));
    app.init_resource::<Time>();
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.init_resource::<phoenix_sim_gameplay::personnel::PersonnelRoster>();
    app.init_resource::<crate::comms::server::CommsInboxRes>();
    let parent = super::super::super::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "receiver".into(),
        None,
    );
    let probe = super::super::super::include_resolve::load_entity_config(
        "assets/entities/physical_probe.toml",
    )
    .unwrap();
    let child = super::super::super::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &probe,
        Vec3::new(0.0, 0.0, -10.0),
        "actual-pod".into(),
        None,
    );
    app.world_mut().flush();
    assert!(app.world().get::<crate::server_app::Ship>(parent).is_none());
    assert!(app.world().get::<ShipPhysics>(parent).is_none());
    assert!(app
        .world()
        .get::<crate::console::comms::server::CommsResponseAiPolicy>(parent)
        .is_some());
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(parent)
        .unwrap()
        .0
        .set(bay(), crate::ship::control_source::ControlSource::Human);
    let request = GenericCommsRequest::ReceiveCraft {
        craft: "actual-pod".into(),
    };
    let order = CraftReceptionOrder {
        request: CommsRequestReference {
            formation_sequence: None,
            origin_id: "heard-order".into(),
            source_ship: "original-carrier".into(),
            recipient_ship: "receiver".into(),
            emitted_tick: 1,
            heard_tick: 2,
            request: request.clone(),
        },
        decision_tick: 3,
    };
    let mut message = crate::core::messages::CommsMessage::injected(
        "copy".into(),
        "original-carrier".into(),
        "Carrier".into(),
        request.body().into(),
        Default::default(),
        vec![],
        "thread".into(),
        true,
        crate::core::messages::CommsPriority::Routine,
    );
    message.recipient_ship = Some(phoenix_model::wire::ShipKey("receiver".into()));
    message.transmission = Some(CommsTransmission {
        formation_sequence: None,
        sustained: false,
        origin_id: "heard-order".into(),
        channel: "public".into(),
        audience: CommsAudience::Contact {
            target_uuid: "receiver".into(),
        },
        emitted_tick: 1,
        received_tick: Some(2),
        content: CommsRadioContent::Request { request },
    });
    let mut progress = CommsRequestProgress::default();
    progress
        .decide(2, 3, CommsRequestDecision::Accept, None)
        .unwrap();
    message.responses = [
        phoenix_model::comms_request::RECEIVE_CRAFT_RESPONSE,
        phoenix_model::comms_request::DECLINE_RESPONSE,
    ]
    .into_iter()
    .map(|text| crate::core::messages::CommsResponseView {
        text: text.into(),
        available: true,
        important: false,
    })
    .collect();
    message.request_progress = Some(progress);
    message.is_read = true;
    message.selected_response = Some(0);
    app.world_mut()
        .resource_mut::<crate::comms::server::CommsInboxRes>()
        .0
        .inject(message);
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(parent)
        .unwrap()
        .0
        .insert(
            SystemId("comms".into()),
            SystemBlackboard::Comms(Default::default()),
        );
    (app, parent, child, order)
}
fn command(app: &mut App, parent: Entity, contact: String, reception: CraftReceptionOrder) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(parent)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: bay(),
            payload: Control::RecoverCraft {
                contact,
                reception: Some(reception),
            },
            response_token: None,
            feedback_correlation: None,
        });
    runtime::handle(app.world_mut());
    app.world_mut()
        .get_mut::<AdmittedCommands>(parent)
        .unwrap()
        .0
        .clear();
}
fn scan(app: &mut App, parent: Entity) {
    let dt = app
        .world()
        .resource::<Time<Fixed>>()
        .timestep()
        .as_secs_f32();
    app.world_mut()
        .get_mut::<ShipPowerSystem>(parent)
        .unwrap()
        .0
        .work_continuous(&SystemId("sensor-radar".into()), dt, 1.0)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::science::broad::tick)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
        .unwrap();
}
#[test]
fn fixed_receiver_needs_own_earned_alias_and_exact_consent_then_paid_actual_handoff() {
    let (mut app, parent, child, order) = fixture();
    assert!(runtime::accepted_receptions(app.world(), parent, &bay(), 4)
        .iter()
        .any(|(actual, _)| actual == &order));
    runtime::handle(app.world_mut());
    assert!(
        app.world().get::<ShipBayJobs>(parent).unwrap().0[&bay()]
            .job
            .is_none(),
        "Comms acceptance never operates a Human Bay"
    );
    let initial = app
        .world()
        .get::<ShipPowerSystem>(parent)
        .unwrap()
        .0
        .capture_continuation();
    command(&mut app, parent, "actual-pod".into(), order.clone());
    assert!(app.world().get::<ShipBayJobs>(parent).unwrap().0[&bay()]
        .job
        .is_none());
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(parent)
            .unwrap()
            .0
            .capture_continuation(),
        initial
    );
    let contact = (0..120)
        .find_map(|_| {
            next(&mut app);
            scan(&mut app, parent);
            runtime::alias(app.world(), parent, &bay(), "actual-pod")
        })
        .expect("actual fixed sensor publication must earn a local alias");
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    let mut stale = order.clone();
    stale.decision_tick = 4;
    command(&mut app, parent, contact.clone(), stale);
    assert!(app.world().get::<ShipBayJobs>(parent).unwrap().0[&bay()]
        .job
        .is_none());
    command(&mut app, parent, contact, order.clone());
    let job = app.world().get::<ShipBayJobs>(parent).unwrap().0[&bay()]
        .job
        .as_ref()
        .unwrap()
        .id;
    let before = app
        .world()
        .get::<ShipPowerSystem>(parent)
        .unwrap()
        .0
        .network()
        .consumers[&bay()]
        .energy;
    for _ in 0..240 {
        next(&mut app);
        scan(&mut app, parent);
        runtime::tick(app.world_mut());
        if app.world().get::<ShipBayJobs>(parent).unwrap().0[&bay()]
            .job
            .as_ref()
            .unwrap()
            .activity
            .ready()
        {
            break;
        }
    }
    assert!(app.world().get::<ShipBayJobs>(parent).unwrap().0[&bay()]
        .job
        .as_ref()
        .unwrap()
        .activity
        .ready());
    assert!(
        app.world()
            .get::<ShipPowerSystem>(parent)
            .unwrap()
            .0
            .network()
            .consumers[&bay()]
            .energy
            < before
    );
    assert!(app
        .world()
        .resource::<crate::comms::server::CommsInboxRes>()
        .0
        .iter()
        .all(|row| row
            .request_progress
            .as_ref()
            .is_none_or(|p| p.facts.reception.is_none())));
    next(&mut app);
    runtime::commit(app.world_mut());
    let state = &app.world().get::<ShipBayJobs>(parent).unwrap().0[&bay()];
    state
        .validate(&bay(), app.world().resource::<crate::sim_tick::SimTick>().0)
        .unwrap();
    let actual = state.job.as_ref().unwrap();
    assert_eq!(actual.id, job);
    assert_eq!(actual.outcome, Some(JobOutcome::Completed));
    let receipt = actual.activity.receipt.as_ref().unwrap();
    assert_eq!(receipt.order, order);
    assert!(receipt.handoff_tick > now);
    assert!(receipt.people.is_empty());
    assert_eq!(
        app.world().get::<ProbeCustody>(child).unwrap().0,
        CraftLocation::Carried {
            parent_uuid: "receiver".into(),
            bay: bay()
        }
    );
    assert!(app
        .world()
        .resource::<crate::comms::server::CommsInboxRes>()
        .0
        .iter()
        .any(|row| row
            .request_progress
            .as_ref()
            .is_some_and(|p| p.facts.reception.as_ref() == Some(receipt))));
}
