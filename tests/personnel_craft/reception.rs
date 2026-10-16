//! Normal Human source work and ordinary static receiver AI, one actual host.
//! No injected custody, controllers, observations, energy or completion facts.
use super::*;
use phoenix_model::{
    comms_channel::CommsAudience,
    comms_request::{
        CommsRadioContent, CommsRequestDecision, CommsRequestReference, GenericCommsRequest,
    },
    craft::{CraftLocation, CraftReceptionOrder, CraftReceptionReceipt},
    personnel::PersonRef,
};
use project_phoenix::{
    comms::server::CommsInboxRes,
    entities::craft::{ProbeCustody, ShipBayJobs},
};
fn boot_reception() -> (App, Entity, Entity) {
    let (app, parent, child) = boot_with_world(
        "tests/fixtures/personnel-reception-world.toml",
        "assets/entities/reception_carrier.toml",
    );
    let receiver = receiver(&app).expect("the authored receiving facility must actually exist");
    assert!(
        app.world()
            .get::<project_phoenix::comms::CommsHailable>(receiver)
            .is_some(),
        "the actual receiver opts into the normal contact roster, not just radio range"
    );
    assert!(
        app.world()
            .get::<project_phoenix::comms::radio::RadioProfile>(receiver)
            .is_some(),
        "the actual receiver has its installed paid radio"
    );
    ordinary_receiver(&app);
    (app, parent, child)
}
fn receiver(app: &App) -> Option<Entity> {
    app.world()
        .try_query::<(Entity, &EntityId)>()?
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == "receiver").then_some(entity))
}
fn received_order(app: &App, receiver: &str, craft: &str) -> Option<CraftReceptionOrder> {
    app.world().resource::<CommsInboxRes>().0.iter().find_map(|message|{
        let reference=CommsRequestReference::from_received(message)?;
        if reference.recipient_ship!=receiver || !matches!(&reference.request,GenericCommsRequest::ReceiveCraft{craft:proposed} if proposed==craft){return None;}
        let decision=message.request_progress.as_ref()?.facts.decision.as_ref()?;
        (decision.choice==CommsRequestDecision::Accept).then_some(CraftReceptionOrder{request:reference,decision_tick:decision.tick})
    })
}
fn receipt(app: &App, order: &CraftReceptionOrder) -> Option<CraftReceptionReceipt> {
    app.world()
        .resource::<CommsInboxRes>()
        .0
        .iter()
        .find_map(|message| {
            message
                .request_progress
                .as_ref()?
                .facts
                .reception
                .as_ref()
                .filter(|receipt| &receipt.order == order)
                .cloned()
        })
}
fn ordinary_receiver(app: &App) {
    if let Some(entity) = receiver(app) {
        let sources = &app
            .world()
            .get::<ShipSystemControlSources>(entity)
            .unwrap()
            .0;
        for owner in ["comms", "reception-bay"] {
            assert!(
                sources.policy_for(&id(owner)).operate_ai,
                "actual receiver {owner} must remain ordinary AI"
            );
        }
        assert!(
            app.world()
                .get::<project_phoenix::server_app::Ship>(entity)
                .is_none(),
            "fixed receiver must not gain a Ship marker/propulsion to work"
        );
        assert!(app
            .world()
            .get::<project_phoenix::ship::state::ShipPhysics>(entity)
            .is_none());
    }
}
fn continue_reception(live: &mut App) {
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, _, _) = boot_reception();
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    for step in 0..360 {
        ordinary_receiver(live);
        ordinary_receiver(&resumed);
        same(live, &resumed, step);
        live.update();
        resumed.update();
    }
    ordinary_receiver(live);
    ordinary_receiver(&resumed);
    same(live, &resumed, 360);
}
fn board_chief(app: &mut App, parent: Entity, child: Entity) -> String {
    let craft = prepare_escort(app, parent, child);
    dispatch(app, "repair-chief", 0);
    wait(app, |app| {
        matches!(
            officer(app, parent, "repair-chief"),
            OfficerLocation::AboardCraft { .. }
        )
    });
    wait(app, |app| {
        !app.world()
            .get::<ShipRepairTeams>(parent)
            .unwrap()
            .0
            .claims_personnel_bay(&id("probe-bay"))
    });
    craft
}
fn negotiate(app: &mut App, parent: Entity, craft: &str) -> CraftReceptionOrder {
    let receiver = receiver(app).unwrap();
    let receiver_uuid = app.world().get::<EntityUuid>(receiver).unwrap().0.clone();
    wait(app, |app| {
        matches!(&app.world().get::<ShipSystemBlackboards>(parent).unwrap().0[&id("comms")],
        SystemBlackboard::Comms(board) if board.contacts.iter().any(|row|row.uuid==receiver_uuid) && board.reception_craft.iter().any(|row|row.id==craft))
    });
    control(
        app,
        "comms",
        "comms",
        Control::SendCommsRequest {
            channel: "public".into(),
            audience: CommsAudience::Contact {
                target_uuid: receiver_uuid.clone(),
            },
            request: GenericCommsRequest::ReceiveCraft {
                craft: craft.into(),
            },
            planned_request: None,
        },
    );
    wait(app, |app| {
        received_order(app, &receiver_uuid, craft).is_some()
    });
    let order = received_order(app, &receiver_uuid, craft).unwrap();
    assert!(receipt(app, &order).is_none());
    ordinary_receiver(app);
    assert!(
        app.world().get::<ShipBayJobs>(receiver).unwrap().0[&id("reception-bay")]
            .job
            .as_ref()
            .is_none_or(|job| job.outcome.is_some()),
        "no own contact exists for a still stored pod"
    );
    order
}
fn launch(app: &mut App, child: Entity, craft: &str) {
    control(
        app,
        "tactical",
        "probe-bay",
        Control::LaunchCraft {
            craft: craft.into(),
        },
    );
    wait(app, |app| !stored(app, child));
}
fn returned(app: &App, order: &CraftReceptionOrder) -> Option<CraftReceptionReceipt> {
    app.world()
        .resource::<CommsInboxRes>()
        .0
        .iter()
        .find_map(|message| {
            let wire = message.transmission.as_ref()?;
            if wire.received_tick.is_none() {
                return None;
            }
            match &wire.content {
                CommsRadioContent::Outcome { outcome } if outcome.request == order.request => {
                    outcome.facts.reception.clone()
                }
                _ => None,
            }
        })
}
fn refused(live: &mut App, bad: &PhoenixSnapshot, label: &str, expected_reason: &str) {
    let before = capture(live.world());
    let digest = world_digest(live.world());
    let report = restore(live.world_mut(), bad);
    assert!(!report.is_complete(), "{label}");
    assert!(
        format!("{:?}", report.gaps).contains(expected_reason),
        "{label}: {:?}",
        report.gaps
    );
    assert_eq!(
        capture(live.world()),
        before,
        "{label}: full capture unchanged"
    );
    assert_eq!(
        world_digest(live.world()),
        digest,
        "{label}: digest unchanged"
    );
}
#[test]
fn actual_human_request_and_paid_static_ai_receive_people_then_report_and_continue_json360() {
    let (mut live, parent, child) = boot_reception();
    let receiver = receiver(&live).unwrap();
    let craft = board_chief(&mut live, parent, child);
    let origin = live.world().get::<EntityUuid>(parent).unwrap().0.clone();
    let receiver_uuid = live.world().get::<EntityUuid>(receiver).unwrap().0.clone();
    let order = negotiate(&mut live, parent, &craft);
    continue_reception(&mut live);
    assert!(receipt(&live, &order).is_none());
    assert!(stored(&live, child));
    launch(&mut live, child, &craft);
    wait(&mut live, |app| {
        app.world().get::<ShipBayJobs>(receiver).unwrap().0[&id("reception-bay")]
            .job
            .as_ref()
            .is_some_and(|job| {
                job.activity.reception.as_ref() == Some(&order) && job.outcome.is_none()
            })
    });
    let started = live.world().get::<ShipBayJobs>(receiver).unwrap().0[&id("reception-bay")]
        .job
        .as_ref()
        .unwrap()
        .clone();
    assert!(receipt(&live, &order).is_none());
    let energy = reserve(&live, receiver, "reception-bay");
    continue_reception(&mut live);
    wait(&mut live, |app| receipt(app, &order).is_some());
    let received = receipt(&live, &order).unwrap();
    assert_eq!(received.scope.job, started.id);
    assert_eq!(received.scope.actor, receiver_uuid);
    assert_eq!(
        received.people,
        vec![PersonRef::new(&origin, "repair-chief")]
    );
    assert!(received.handoff_tick > started.execution_tick);
    assert!(reserve(&live, receiver, "reception-bay") < energy);
    assert_eq!(
        live.world().get::<ProbeCustody>(child).unwrap().0,
        CraftLocation::Carried {
            parent_uuid: receiver_uuid.clone(),
            bay: id("reception-bay")
        }
    );
    assert_eq!(
        officer(&live, parent, "repair-chief"),
        OfficerLocation::AboardCraft {
            craft: craft.clone(),
            berth: "passenger".into()
        }
    );
    assert!(
        people(&live, parent)
            .officers
            .iter()
            .all(|row| row.person.officer != "repair-chief"),
        "original Console cannot see foreign-current personnel telemetry"
    );
    wait(&mut live, |app| returned(app, &order).is_some());
    assert_eq!(returned(&live, &order), Some(received.clone()));
    let board = &live
        .world()
        .get::<ShipSystemBlackboards>(receiver)
        .unwrap()
        .0[&id("reception-bay")];
    assert!(
        matches!(board,SystemBlackboard::Bay(view) if view.stored.iter().any(|row|row.id==craft && row.people==received.people))
    );
    let saved = capture(live.world());
    for fault in 0..7 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == receiver_uuid)
            .unwrap();
        let job = row
            .bay_jobs
            .as_mut()
            .unwrap()
            .0
            .get_mut(&id("reception-bay"))
            .unwrap()
            .job
            .as_mut()
            .unwrap();
        match fault {
            0 => job.activity.reception.as_mut().unwrap().decision_tick += 1,
            1 => job.activity.receipt.as_mut().unwrap().scope.actor = origin.clone(),
            2 => {
                job.activity.receipt.as_mut().unwrap().scope.job =
                    phoenix_model::job::JobId(job.id.0 + 1)
            }
            3 => job.activity.receipt.as_mut().unwrap().people.clear(),
            4 => job.activity.receipt.as_mut().unwrap().handoff_tick = job.execution_tick,
            5 => {
                job.activity.receipt.as_mut().unwrap().people[0].officer = "invented-person".into()
            }
            _ => {
                job.activity
                    .receipt
                    .as_mut()
                    .unwrap()
                    .order
                    .request
                    .origin_id = "invented-order".into();
            }
        }
        refused(&mut live, &bad, &format!("receiving forgery {fault}"), "");
    }
    continue_reception(&mut live);
    // A second real recovery retires the first Bay Job, keeping its genuine
    // receipt historical before the conflicting-old-scope continuation probe.
    let empty = entity(&mut live, "empty-probe");
    let empty_uuid = live.world().get::<EntityUuid>(empty).unwrap().0.clone();
    let second = negotiate(&mut live, parent, &empty_uuid);
    launch(&mut live, empty, &empty_uuid);
    wait(&mut live, |app| receipt(app, &second).is_some());
    let actual_second = receipt(&live, &second).unwrap();
    assert!(actual_second.scope.job > received.scope.job);
    let SystemBlackboard::Bay(full) = &live
        .world()
        .get::<ShipSystemBlackboards>(receiver)
        .unwrap()
        .0[&id("reception-bay")]
    else {
        panic!("actual receiving Bay publication");
    };
    assert_eq!(full.occupied_space, full.capacity_space);
    assert!(full.occupied_space > 0);
    assert!(
        !full.can_recover,
        "two actual received craft exhaust the authored finite Bay space"
    );
    let mut bad = capture(live.world());
    let inbox = &mut bad.comms.as_mut().unwrap().inbox;
    // Clone the original issued/heard lineage with a distinct claimed order.
    // Its old valid Job mint is insufficient: two orders cannot own one Job.
    let mut copies: Vec<_> = inbox
        .iter()
        .filter(|message| {
            message.transmission.as_ref().is_some_and(|wire| {
                wire.origin_id == order.request.origin_id
                    && matches!(wire.content, CommsRadioContent::Request { .. })
            })
        })
        .cloned()
        .collect();
    assert_eq!(copies.len(), 2);
    for (index, message) in copies.iter_mut().enumerate() {
        message.id = format!("conflicting-copy-{index}");
        message.transmission.as_mut().unwrap().origin_id = "conflicting-old-order".into();
        if let Some(progress) = &mut message.request_progress {
            progress.last_returned = None;
            if let Some(receipt) = &mut progress.facts.reception {
                receipt.order.request.origin_id = "conflicting-old-order".into();
            }
        }
    }
    inbox.extend(copies);
    phoenix_model::comms_channel::validate_transmissions(inbox, bad.tick)
        .expect("conflicting receipt keeps otherwise coherent closed Comms lineage");
    refused(
        &mut live,
        &bad,
        "conflicting historical receiving scope",
        "One actual Bay Job cannot claim different receiving receipts",
    );
    continue_reception(&mut live);
}
#[test]
fn actual_origin_loss_preserves_deployed_people_reception_and_receiver_loss_resolves_only_actual_occupants(
) {
    let (mut live, parent, child) = boot_reception();
    let receiver = receiver(&live).unwrap();
    let craft = board_chief(&mut live, parent, child);
    let origin = live.world().get::<EntityUuid>(parent).unwrap().0.clone();
    let order = negotiate(&mut live, parent, &craft);
    launch(&mut live, child, &craft);
    // This is the supported trusted World removal input, not a GM-admission claim.
    live.world_mut()
        .resource_mut::<project_phoenix::world::server::WorldContentRuntime>()
        .pending_gm_despawns
        .push(origin.clone());
    live.update();
    assert!(live.world().get_entity(parent).is_err());
    wait(&mut live, |app| receipt(app, &order).is_some());
    let received = receipt(&live, &order).unwrap();
    assert_eq!(
        received.people,
        vec![PersonRef::new(&origin, "repair-chief")]
    );
    let state = &capture(live.world()).personnel.origins[&origin];
    assert!(state.removed_tick.is_some());
    assert_eq!(
        state.state.officers["repair-chief"].location,
        OfficerLocation::AboardCraft {
            craft: craft.clone(),
            berth: "passenger".into()
        }
    );
    continue_reception(&mut live);
    let receiver_uuid = live.world().get::<EntityUuid>(receiver).unwrap().0.clone();
    live.world_mut()
        .resource_mut::<project_phoenix::world::server::WorldContentRuntime>()
        .pending_gm_despawns
        .push(receiver_uuid);
    live.update();
    assert!(live.world().get_entity(receiver).is_err());
    assert!(live.world().get_entity(child).is_err());
    assert_eq!(
        capture(live.world()).personnel.origins[&origin]
            .state
            .officers["repair-chief"]
            .location,
        OfficerLocation::Lost { actor: craft }
    );
    assert_eq!(
        receipt(&live, &order),
        Some(received),
        "factual handoff history survives the later lethal loss"
    );
    continue_reception(&mut live);
}
