//! Routed GM Comms through real simulation, ordinary consumers and continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::command_admission::{log::ShipKey, HostSlot};
use phoenix::comms::server::{CommsInboxRes, CommsRuntime};
use phoenix::core::messages::{CommsMessage, StationId, SystemControlPayload};
use phoenix::entities::spawner::{EntityName, EntityUuid};
use phoenix::gm_action::*;
use phoenix::gm_comms::*;
use phoenix::lockstep::{FleetRoster, FleetShip, FleetSlotOf};
use project_phoenix as phoenix;

const TEXT: &str = "  server.gm.comms.heading\n<b>🌒</b> {sender}\t  ";
fn args() -> phoenix::headless::HeadlessArgs {
    phoenix::headless::HeadlessArgs {
        world_path: "assets/worlds/probe_gm_comms.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1317),
        deterministic: true,
        max_ticks: 240,
        ..Default::default()
    }
}
fn seeded(local: u32) -> App {
    let mut app = phoenix::headless::build_headless_app(&args()).unwrap();
    app.insert_resource(FleetRoster::new(
        (1..=3).map(|slot| FleetShip::new(HostSlot(slot))).collect(),
        HostSlot(local),
    ));
    app.finish();
    app.cleanup();
    advance(&mut app, 90);
    app
}
fn advance(app: &mut App, ticks: usize) {
    for _ in 0..ticks {
        app.update();
    }
}
fn ships(app: &mut App) -> Vec<(Entity, String)> {
    let mut query = app
        .world_mut()
        .query::<(Entity, &EntityUuid, &FleetSlotOf)>();
    let mut rows: Vec<_> = query
        .iter(app.world())
        .map(|(entity, uuid, slot)| (slot.0, entity, uuid.0.clone()))
        .collect();
    rows.sort_by_key(|row| row.0);
    rows.into_iter()
        .map(|(_, entity, uuid)| (entity, uuid))
        .collect()
}
fn speaker(app: &mut App) -> String {
    let mut query = app.world_mut().query::<(&EntityUuid, &EntityName)>();
    query
        .iter(app.world())
        .find(|(_, name)| name.0 == "world.probe_gm_comms.speaker")
        .unwrap()
        .0
         .0
        .clone()
}
fn messages(app: &App) -> Vec<CommsMessage> {
    app.world().resource::<CommsInboxRes>().0.messages()
}
fn intent(app: &mut App, recipients: &[String], hail: bool) -> GmCommsTransmission {
    let mut recipients: Vec<_> = recipients.iter().cloned().map(ShipKey).collect();
    recipients.sort_by(|a, b| a.0.cmp(&b.0));
    GmCommsTransmission {
        sender: speaker(app),
        route: "selected".into(),
        recipients,
        content: if hail {
            GmCommsContent::ScriptedHail {
                hail: "offer".into(),
            }
        } else {
            GmCommsContent::Literal { text: TEXT.into() }
        },
    }
}
fn grant(sequence: u64, tick: u64, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(4),
        sequenced_by: HostSlot(1),
        operator_id: "gm-comms".into(),
        correlation: GmActionId::new(format!("comms-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(4), sequence),
        action,
    }
}
fn enqueue(app: &mut App, action: GmAction) -> GmActionGrant {
    let tick = app.world().resource::<phoenix::sim_tick::SimTick>().0;
    let sequence = app.world().resource::<GmActionJournal>().next_sequence();
    let grant = grant(sequence, tick, action);
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant.clone())
        .unwrap();
    grant
}
fn send(app: &mut App, transmission: GmCommsTransmission) -> GmActionGrant {
    let grant = enqueue(app, GmAction::TransmitComms { transmission });
    advance(app, 4);
    grant
}
fn puppet(app: &mut App, ship: &str) {
    let station = comms_station(app, ship);
    enqueue(
        app,
        GmAction::SetStationPuppet {
            ship: ShipKey(ship.into()),
            station,
            active: true,
        },
    );
    advance(app, 3);
}
fn command(app: &mut App, ship: &str, payload: SystemControlPayload) {
    let station = comms_station(app, ship);
    enqueue(
        app,
        GmAction::IssueStationCommand {
            ship: ShipKey(ship.into()),
            station,
            target: phoenix::ship::system_registry::comms_system_id(),
            payload: phoenix::core::codec::canonical_system_command(&payload).unwrap(),
        },
    );
    advance(app, 4);
}
fn comms_station(app: &mut App, ship: &str) -> StationId {
    let mut query = app.world_mut().query::<(
        &EntityUuid,
        &phoenix::ship::components::ShipConfigComponent,
        Option<&phoenix::ship_plugin::HumanSeekingHosts>,
    )>();
    let (_, config, hosts) = query
        .iter(app.world())
        .find(|(uuid, _, _)| uuid.0 == ship)
        .unwrap();
    phoenix::command_admission::station_for_system(
        &config.0,
        hosts,
        &phoenix::ship::system_registry::comms_system_id(),
    )
    .unwrap()
}
fn last_reason(app: &App) -> Option<GmActionRefusalReason> {
    app.world()
        .resource::<GmActionLog>()
        .entries()
        .last()
        .unwrap()
        .reason
}
fn digest(app: &App) -> u64 {
    phoenix::sim_digest::world_digest(app.world())
}

#[test]
fn literal_multi_recipient_delivery_is_exact_and_duplicate_safe_in_real_blackboards() {
    let mut app = seeded(1);
    let fleet = ships(&mut app);
    assert_eq!(fleet.len(), 3);
    let recipients = vec![fleet[0].1.clone(), fleet[1].1.clone()];
    let transmission = intent(&mut app, &recipients, false);
    let request = send(&mut app, transmission.clone());
    assert_eq!(last_reason(&app), None);
    let delivered = messages(&app);
    assert_eq!(delivered.len(), 2);
    assert_ne!(delivered[0].id, delivered[1].id);
    for (entity, uuid) in &fleet {
        let bb = app
            .world()
            .get::<phoenix::server_app::ShipSystemBlackboards>(*entity)
            .unwrap();
        let phoenix::core::messages::SystemBlackboard::Comms(bb) =
            bb.0.get(&phoenix::ship::system_registry::comms_system_id())
                .unwrap()
        else {
            panic!()
        };
        assert_eq!(bb.messages.len(), usize::from(recipients.contains(uuid)));
        for message in &bb.messages {
            assert_eq!(message.body, TEXT);
            assert!(message.literal_body);
            assert_eq!(message.recipient_ship.as_ref().unwrap().0, *uuid);
        }
    }
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(request)
        .unwrap();
    advance(&mut app, 4);
    assert_eq!(messages(&app), delivered);
    app.world_mut()
        .run_system_once(publish_comms_projection)
        .unwrap();
    let results = &app
        .world()
        .resource::<LastGmCommsProjection>()
        .0
        .as_ref()
        .unwrap()
        .results;
    assert_eq!(
        results.last().unwrap().transmission.as_ref(),
        Some(&transmission)
    );
    assert_eq!(results.last().unwrap().result.operator_id, "gm-comms");
    let json = serde_json::to_string(&app.world().resource::<GmActionJournal>()).unwrap();
    let decoded: GmActionJournal = serde_json::from_str(&json).unwrap();
    assert_eq!(
        decoded.grants(),
        app.world().resource::<GmActionJournal>().grants()
    );
}

#[test]
fn scripted_hail_reply_and_clear_respect_immutable_recipient() {
    let mut app = seeded(1);
    let fleet = ships(&mut app);
    let request = intent(&mut app, &[fleet[0].1.clone(), fleet[1].1.clone()], true);
    send(&mut app, request);
    assert_eq!(last_reason(&app), None);
    let initial = messages(&app);
    assert_eq!(initial.len(), 2);
    let first = initial
        .iter()
        .find(|m| m.is_for_ship(Some(&fleet[0].1)))
        .unwrap()
        .clone();
    let second = initial
        .iter()
        .find(|m| m.is_for_ship(Some(&fleet[1].1)))
        .unwrap()
        .clone();
    assert_eq!(
        app.world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .len(),
        2
    );
    puppet(&mut app, &fleet[2].1);
    command(
        &mut app,
        &fleet[2].1,
        SystemControlPayload::RespondToMessage {
            message_id: first.id.clone(),
            response_index: 0,
        },
    );
    assert_eq!(
        last_reason(&app),
        Some(GmActionRefusalReason::SystemRefused)
    );
    assert_eq!(messages(&app), initial);
    command(&mut app, &fleet[2].1, SystemControlPayload::ClearComms);
    assert_eq!(
        app.world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .len(),
        2
    );
    puppet(&mut app, &fleet[0].1);
    command(
        &mut app,
        &fleet[0].1,
        SystemControlPayload::RespondToMessage {
            message_id: first.id.clone(),
            response_index: 0,
        },
    );
    assert_eq!(last_reason(&app), None);
    let follow_up = messages(&app)
        .into_iter()
        .find(|m| m.body == "world.probe_gm_comms.follow_up")
        .unwrap();
    assert_eq!(follow_up.recipient_ship, first.recipient_ship);
    assert_eq!(follow_up.thread_id, first.thread_id);
    assert!(app
        .world()
        .resource::<CommsRuntime>()
        .active_dialogues
        .contains_key(&second.id));
    command(&mut app, &fleet[0].1, SystemControlPayload::ClearComms);
    assert!(!app
        .world()
        .resource::<CommsRuntime>()
        .active_dialogues
        .contains_key(&follow_up.id));
    assert!(app
        .world()
        .resource::<CommsRuntime>()
        .active_dialogues
        .contains_key(&second.id));
}

#[test]
fn unavailable_identity_route_recipient_and_hail_are_refused_atomically() {
    let mut app = seeded(1);
    let fleet = ships(&mut app);
    let base = intent(&mut app, &[fleet[0].1.clone(), fleet[1].1.clone()], false);
    for (request, reason) in [
        (
            GmCommsTransmission {
                sender: "missing".into(),
                ..base.clone()
            },
            GmActionRefusalReason::UnavailableCommsIdentity,
        ),
        (
            GmCommsTransmission {
                route: "missing".into(),
                ..base.clone()
            },
            GmActionRefusalReason::UnknownCommsRoute,
        ),
        (
            GmCommsTransmission {
                content: GmCommsContent::ScriptedHail {
                    hail: "not-authored".into(),
                },
                ..base.clone()
            },
            GmActionRefusalReason::UnavailableCommsHail,
        ),
        (
            GmCommsTransmission {
                route: "fleet".into(),
                ..base.clone()
            },
            GmActionRefusalReason::UnavailableCommsRecipient,
        ),
    ] {
        send(&mut app, request);
        assert_eq!(last_reason(&app), Some(reason));
        assert!(messages(&app).is_empty());
    }
    app.world_mut().despawn(fleet[1].0);
    send(&mut app, base);
    assert_eq!(
        last_reason(&app),
        Some(GmActionRefusalReason::UnavailableCommsRecipient)
    );
    assert!(messages(&app).is_empty());
}

#[test]
fn queued_hail_and_live_conversation_restore_and_continue_identically_across_local_ships() {
    let mut live = seeded(1);
    let fleet = ships(&mut live);
    let literal = intent(&mut live, &[fleet[0].1.clone()], false);
    send(&mut live, literal);
    let hail = intent(&mut live, &[fleet[1].1.clone()], true);
    enqueue(&mut live, GmAction::TransmitComms { transmission: hail });
    live.world_mut()
        .run_system_once(phoenix::gm_action::apply_due_actions)
        .unwrap();
    assert_eq!(
        live.world()
            .resource::<phoenix::world::server::WorldScriptRuntime>()
            .pending_comms_opens
            .len(),
        1
    );
    let snapshot = phoenix::snapshot::capture(live.world());
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let snapshot = serde_json::from_str(&encoded).unwrap();
    let mut restored = seeded(2);
    let report = phoenix::snapshot::restore(restored.world_mut(), &snapshot);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(digest(&live), digest(&restored));
    for _ in 0..6 {
        advance(&mut live, 1);
        advance(&mut restored, 1);
        assert_eq!(digest(&live), digest(&restored));
    }
    assert_eq!(messages(&live), messages(&restored));
    let message = messages(&live)
        .into_iter()
        .find(|m| m.body == "world.probe_gm_comms.offer")
        .unwrap();
    for app in [&mut live, &mut restored] {
        puppet(app, &fleet[1].1);
        command(
            app,
            &fleet[1].1,
            SystemControlPayload::RespondToMessage {
                message_id: message.id.clone(),
                response_index: 0,
            },
        );
    }
    assert_eq!(digest(&live), digest(&restored));
    assert!(messages(&live)
        .iter()
        .any(|m| m.body == "world.probe_gm_comms.follow_up"));
}

#[test]
fn literal_and_scripted_comms_replay_from_the_portable_artifact_with_recovery() {
    use phoenix::headless::replay::{drive_run, drive_run_with_gm_actions};
    let mut discovery = drive_run(&args(), &[], 0).unwrap();
    let fleet = ships(discovery.app_mut());
    let literal = intent(discovery.app_mut(), &[fleet[0].1.clone()], false);
    let hail = intent(discovery.app_mut(), &[fleet[0].1.clone()], true);
    let mut planned = GmActionJournal::default();
    planned.record_slot_recovery(HostSlot(4), 140).unwrap();
    planned
        .insert(grant(
            1,
            110,
            GmAction::TransmitComms {
                transmission: literal.clone(),
            },
        ))
        .unwrap();
    planned
        .insert(grant(
            2,
            150,
            GmAction::TransmitComms {
                transmission: literal.clone(),
            },
        ))
        .unwrap();
    let mut recovered = grant(3, 160, GmAction::TransmitComms { transmission: hail });
    recovered.recovery_generation = 1;
    planned.insert(recovered).unwrap();
    planned.restore_applied_frontier(planned.len()).unwrap();
    let mut recorded = drive_run_with_gm_actions(&args(), &[], &planned, 25).unwrap();
    let actions = recorded.recorded_gm_actions();
    assert_eq!(
        actions.applied_results()[1].reason,
        Some(GmActionRefusalReason::NotGameMaster)
    );
    assert_eq!(
        actions.applied_results()[2].outcome,
        GmActionOutcome::Applied
    );
    let artifact = phoenix::headless::ReplayArtifact::capture(
        &args(),
        recorded.recorded_log(),
        actions,
        recorded.tick(),
        recorded.seal(),
    )
    .unwrap();
    assert_eq!(phoenix::headless::verify_artifact(&artifact).unwrap(), None);
}
