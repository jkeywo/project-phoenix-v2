use super::*;
use crate::core::messages::{AdmittedCommand, SystemId};
use crate::world_id::InstallWorldIdMint;
use bevy::ecs::system::RunSystemOnce;

fn first_difference(
    left: &serde_json::Value,
    right: &serde_json::Value,
    path: &str,
) -> Option<String> {
    use serde_json::Value;
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let location = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => {
                        if let Some(difference) = first_difference(a, b, &location) {
                            return Some(difference);
                        }
                    }
                    _ => return Some(format!("{location}: field presence differs")),
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: lengths {} / {}", a.len(), b.len()));
            }
            a.iter()
                .zip(b)
                .enumerate()
                .find_map(|(i, (a, b))| first_difference(a, b, &format!("{path}[{i}]")))
        }
        _ => (left != right).then(|| format!("{path}: {left} / {right}")),
    }
}

fn fixture(distance: f32) -> (App, Entity, Entity) {
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(10))
        .insert_resource(Time::<Fixed>::from_hz(30.0))
        .init_resource::<CommsInboxRes>()
        .init_resource::<CommsRuntime>()
        .init_resource::<crate::server_app::SimOutbox>();
    app.world_mut()
        .insert_world_id_mint(crate::world_id::WorldIdMint::default());
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let source = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "source".into(),
        None,
    );
    let receiver = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::new(distance, 0.0, 0.0),
        "receiver".into(),
        None,
    );
    app.world_mut().flush();
    if let Some(mut pose) = app.world_mut().get_mut::<ShipPhysics>(receiver) {
        pose.x = distance;
        pose.z = 0.0;
    }
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId("comms".into()),
            payload: SystemControlPayload::SendCommsText {
                channel: "public".into(),
                audience: CommsAudience::All,
                body: "Need assistance\nStanding by.".into(),
            },
            response_token: None,
            feedback_correlation: None,
        });
    (app, source, receiver)
}

fn messages(app: &App, recipient: &str) -> Vec<CommsMessage> {
    app.world()
        .resource::<CommsInboxRes>()
        .0
        .iter()
        .filter(|message| message.is_for_ship(Some(recipient)))
        .cloned()
        .collect()
}

#[test]
fn actual_paid_radio_emission_has_the_same_local_fact_whether_any_receiver_hears() {
    let mut local_facts = Vec::new();
    for distance in [10.0, 10000.0] {
        let (mut app, source, _) = fixture(distance);
        let before = app
            .world()
            .get::<ShipPowerSystem>(source)
            .unwrap()
            .0
            .network()
            .consumers[&SystemId("comms".into())]
            .energy;
        app.world_mut().run_system_once(tick_radio).unwrap();
        let sent = messages(&app, "source");
        assert_eq!(sent.len(), 1);
        assert!(sent[0]
            .transmission
            .as_ref()
            .unwrap()
            .received_tick
            .is_none());
        let after = app
            .world()
            .get::<ShipPowerSystem>(source)
            .unwrap()
            .0
            .network()
            .consumers[&SystemId("comms".into())]
            .energy;
        assert!(
            after < before,
            "actual continuous listening and discrete emission spend local reserve"
        );
        let heard = messages(&app, "receiver");
        assert_eq!(heard.len(), usize::from(distance < 200.0));
        if let Some(message) = heard.first() {
            assert_eq!(
                message.transmission.as_ref().unwrap().origin_id,
                sent[0].transmission.as_ref().unwrap().origin_id
            );
            assert_eq!(message.transmission.as_ref().unwrap().emitted_tick, 10);
            assert_eq!(
                message.transmission.as_ref().unwrap().received_tick,
                Some(10)
            );
        }
        // A later visible emission must not reveal conditional receiver count
        // through the shared mint's next sequence number either.
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
        app.world_mut().run_system_once(tick_radio).unwrap();
        local_facts.push((
            messages(&app, "source"),
            app.world()
                .get::<ShipPowerSystem>(source)
                .unwrap()
                .0
                .network()
                .consumers[&SystemId("comms".into())]
                .energy,
        ));
    }
    assert_eq!(
        local_facts[0], local_facts[1],
        "local send cannot reveal a hidden receiver outcome"
    );
}

#[test]
fn installed_protection_and_actual_operation_bound_reception_without_refusing_the_sender() {
    for unavailable in [false, true] {
        let (mut app, source, receiver) = fixture(10.0);
        if unavailable {
            let mut controls = app
                .world_mut()
                .get_mut::<ShipSystemControlSources>(receiver)
                .unwrap();
            controls.0.set_gm_disabled(SystemId("comms".into()), true);
        } else {
            // Different accepted equipment key, with a valid matching own key.
            let mut radio = app.world_mut().get_mut::<RadioProfile>(receiver).unwrap();
            radio.0.keys = vec!["other-key".into()];
            radio
                .0
                .channels
                .iter_mut()
                .find(|channel| channel.id == "fleet")
                .unwrap()
                .key = Some("other-key".into());
        }
        app.world_mut()
            .get_mut::<AdmittedCommands>(source)
            .unwrap()
            .0[0]
            .payload = SystemControlPayload::SendCommsText {
            channel: "fleet".into(),
            audience: CommsAudience::All,
            body: "Protected text".into(),
        };
        app.world_mut().run_system_once(tick_radio).unwrap();
        assert_eq!(messages(&app, "source").len(), 1);
        assert!(messages(&app, "receiver").is_empty());
    }
}

#[test]
fn radio_inbox_and_paid_reserves_continue_after_json_and_malformed_copies_do_not_write() {
    fn host() -> (App, Entity) {
        let (mut app, source, _) = fixture(10.0);
        // Production registers optional capture columns and run-scope owners.
        // Register the same empty columns/owners in this narrow handler fixture.
        app.world_mut()
            .register_component::<crate::entities::spawner::EntityShipArcHull>();
        app.world_mut()
            .register_component::<crate::console::repair::server::RepairRequestQueue>();
        app.world_mut()
            .register_component::<crate::ship_plugin::RepairHumanAlerted>();
        app.world_mut()
            .register_component::<crate::ai::server::ObjectiveCursors>();
        app.world_mut()
            .register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
        app.world_mut()
            .register_component::<crate::server_app::AsteroidUuid>();
        app.init_resource::<crate::gm::actions::action::SimulationPaused>()
            .init_resource::<crate::core::report::MissionReport>()
            .init_resource::<crate::gm::actions::action::GmActionJournal>()
            .init_resource::<crate::gm::operations::puppet::StationPuppets>()
            .init_resource::<crate::gm::operations::puppet::PendingGmStationCommands>();
        // A real host publishes these derived views before a save. Restore
        // rebuilds the same views and then reinstates held publications. Seed
        // them here too instead of comparing an un-published bootstrap to a
        // post-restore host that has already run its ordinary projection pass.
        let _ = app
            .world_mut()
            .run_system_once(crate::science::server::publish_scan_blackboard);
        let _ = app
            .world_mut()
            .run_system_once(crate::science::passive::publish);
        let _ = app
            .world_mut()
            .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard);
        let _ = app
            .world_mut()
            .run_system_once(crate::console::helm::server::publish_helm_blackboard);
        let _ = app
            .world_mut()
            .run_system_once(crate::console::navigation::server::publish_navigation_blackboard);
        let _ = app.world_mut().run_system_once(
            crate::console::weapons::blackboard::publish_tactical_radar_blackboard,
        );
        (app, source)
    }
    fn advance(app: &mut App) {
        app.world_mut().run_system_once(tick_radio).unwrap();
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    }
    let (mut live, _) = host();
    advance(&mut live);
    let saved = crate::snapshot::capture(live.world());
    assert_eq!(
        saved.entities.len(),
        2,
        "both actual installed ships participate"
    );
    assert_eq!(
        saved.comms.as_ref().unwrap().inbox.len(),
        2,
        "real paid send and reception precede save"
    );
    let saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let (mut resumed, _) = host();
    let report = crate::snapshot::restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    for step in 0..360 {
        let live_state = crate::snapshot::capture(live.world());
        let resumed_state = crate::snapshot::capture(resumed.world());
        assert!(
            live_state == resumed_state,
            "step {step}: {:?}",
            first_difference(
                &serde_json::to_value(&live_state).unwrap(),
                &serde_json::to_value(&resumed_state).unwrap(),
                "snapshot"
            )
        );
        assert_eq!(
            crate::sim_digest::world_digest(live.world()),
            crate::sim_digest::world_digest(resumed.world()),
            "step {step}"
        );
        advance(&mut live);
        advance(&mut resumed);
    }
    let before = crate::snapshot::capture(live.world());
    let digest = crate::sim_digest::world_digest(live.world());
    for fault in 0..3 {
        let mut bad = before.clone();
        bad.tick += 10;
        let inbox = &mut bad.comms.as_mut().unwrap().inbox;
        match fault {
            0 => inbox[0].transmission.as_mut().unwrap().received_tick = Some(bad.tick + 1),
            1 => inbox[0].recipient_ship = None,
            _ => {
                let mut duplicate = inbox[0].clone();
                duplicate.id = "forged-repeat".into();
                inbox.push(duplicate);
            }
        }
        assert!(!crate::snapshot::restore(live.world_mut(), &bad).is_complete());
        assert_eq!(crate::snapshot::capture(live.world()), before);
        assert_eq!(crate::sim_digest::world_digest(live.world()), digest);
    }
}

#[path = "request_tests.rs"]
mod request_tests;

#[test]
fn existing_environmental_jam_keeps_exact_radio_denial_payment_and_recovery() {
    use crate::core::messages::{FlagKind, ModifierSource};
    for jam_source in [false, true] {
        let (mut app, source, receiver) = fixture(10.0);
        let jammed = if jam_source { source } else { receiver };
        let owner = SystemId("comms".into());
        let energy = |world: &World| {
            world
                .get::<ShipPowerSystem>(jammed)
                .unwrap()
                .0
                .network()
                .consumers[&owner]
                .energy
        };
        let before = energy(app.world());
        let natural = ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(17001),
        };
        app.world_mut()
            .get_mut::<crate::modifiers::ShipModifiers>(jammed)
            .unwrap()
            .add_flag(natural.clone(), FlagKind::CommsJammed);
        app.world_mut().run_system_once(tick_radio).unwrap();
        assert_eq!(
            energy(app.world()),
            before,
            "legacy denied endpoint does no listening or transmission work"
        );
        assert_eq!(messages(&app, "source").len(), usize::from(!jam_source));
        assert!(messages(&app, "receiver").is_empty());
        app.world_mut()
            .get_mut::<crate::modifiers::ShipModifiers>(jammed)
            .unwrap()
            .remove_flag(natural, FlagKind::CommsJammed);
        app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
        app.world_mut().run_system_once(tick_radio).unwrap();
        assert_eq!(messages(&app, "receiver").len(), 1);
        assert!(
            energy(app.world()) < before,
            "recovery resumes ordinary paid operation"
        );
    }
}

#[test]
fn actual_paid_relay_restores_blocked_text_route_and_loss_does_not_change_sender_facts() {
    let mut sender_facts = Vec::new();
    for fault in 0..3 {
        let (mut app, source, _) = fixture(350.0);
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/relay_cruiser.toml",
        )
        .unwrap();
        let relay = crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::new(175.0, 0.0, 0.0),
            "relay".into(),
            None,
        );
        app.world_mut().flush();
        if let Some(mut pose) = app.world_mut().get_mut::<ShipPhysics>(relay) {
            pose.x = 175.0;
            pose.z = 0.0;
        }
        let owner = SystemId("comms".into());
        if fault == 1 {
            app.world_mut()
                .get_mut::<ShipSystemControlSources>(relay)
                .unwrap()
                .0
                .set_gm_disabled(owner.clone(), true);
        }
        if fault == 2 {
            app.world_mut()
                .get_mut::<ShipPowerSystem>(relay)
                .unwrap()
                .0
                .network_mut()
                .consumers
                .get_mut(&owner)
                .unwrap()
                .energy = 0.0;
        }
        let before = app
            .world()
            .get::<ShipPowerSystem>(relay)
            .unwrap()
            .0
            .network()
            .consumers[&owner]
            .energy;
        app.world_mut().run_system_once(tick_radio).unwrap();
        let sent = messages(&app, "source");
        assert_eq!(sent.len(), 1);
        let received = messages(&app, "receiver");
        assert_eq!(received.len(), usize::from(fault == 0), "fault{fault}");
        if let Some(heard) = received.first() {
            assert_eq!(heard.sender_uuid, "source");
            assert_eq!(heard.body, sent[0].body);
            assert_eq!(
                heard.transmission.as_ref().unwrap().origin_id,
                sent[0].transmission.as_ref().unwrap().origin_id
            );
            assert_eq!(heard.transmission.as_ref().unwrap().emitted_tick, 10);
            assert!(
                app.world()
                    .get::<ShipPowerSystem>(relay)
                    .unwrap()
                    .0
                    .network()
                    .consumers[&owner]
                    .energy
                    < before
            );
        }
        sender_facts.push((
            sent,
            app.world()
                .get::<ShipPowerSystem>(source)
                .unwrap()
                .0
                .network()
                .consumers[&owner]
                .energy,
        ));
    }
    assert_eq!(sender_facts[0], sender_facts[1]);
    assert_eq!(sender_facts[0], sender_facts[2]);
}

#[test]
fn protected_relay_requires_actual_matching_channel_and_held_key() {
    for compatible in [false, true] {
        let (mut app, source, _) = fixture(2100.0);
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/relay_cruiser.toml",
        )
        .unwrap();
        let relay = crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::new(1050.0, 0.0, 0.0),
            "relay".into(),
            None,
        );
        app.world_mut().flush();
        if let Some(mut pose) = app.world_mut().get_mut::<ShipPhysics>(relay) {
            pose.x = 1050.0;
            pose.z = 0.0;
        }
        if !compatible {
            let mut profile = app.world_mut().get_mut::<RadioProfile>(relay).unwrap();
            profile.0.keys = vec!["different-key".into()];
            profile
                .0
                .channels
                .iter_mut()
                .find(|row| row.id == "fleet")
                .unwrap()
                .key = Some("different-key".into());
        }
        app.world_mut()
            .get_mut::<AdmittedCommands>(source)
            .unwrap()
            .0[0]
            .payload = SystemControlPayload::SendCommsText {
            channel: "fleet".into(),
            audience: CommsAudience::All,
            body: "Protected source message".into(),
        };
        app.world_mut().run_system_once(tick_radio).unwrap();
        assert_eq!(messages(&app, "source").len(), 1);
        assert_eq!(messages(&app, "receiver").len(), usize::from(compatible));
    }
}
