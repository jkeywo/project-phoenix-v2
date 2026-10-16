//! Owner-level retained observation proof, not a full lifecycle-save journey.
use super::*;
use bevy::ecs::system::RunSystemOnce;

fn held_publication() -> (World, Entity, PhoenixSnapshot) {
    use crate::core::messages::{AdmittedCommand, AdmittedCommands, SystemControlPayload};
    let mut world = World::new();
    world.insert_resource(SimTick(10));
    world.init_resource::<crate::server_app::CaptainPriorityBoost>();
    let accepted = crate::entities::include_resolve::load_entity_config(
        "tests/fixtures/listening-cruiser.toml",
    )
    .unwrap();
    let ship = accepted.ship_config.as_ref().unwrap().clone();
    let (sources, ratings) = crate::ship::rating::seed_boot_ratings(&ship, |_| {
        crate::ship::rating::BACKFILL_RATING.into()
    });
    let actor = crate::entities::spawner::spawn_entity_with_ship_seed(
        &mut world.commands(),
        &accepted,
        Vec3::ZERO,
        "held-alert".into(),
        None,
        Some(crate::entities::ship_spawn::ShipSpawnSeed {
            ship_config: crate::ship_plugin::ShipConfigComponent(ship),
            control_sources: crate::ship_plugin::ShipSystemControlSources(sources),
            active_ratings: crate::ship_plugin::ActiveStationRatings(ratings),
            crew_presence: Default::default(),
            initial_yaw: 0.0,
        }),
    );
    world.flush();
    world
        .run_system_once(crate::console::captain::server::publish_captain_blackboard)
        .unwrap();
    // This is the real Input-owner -> Publish frontier: use the actual admitted
    // Alert handler, leaving the previous genuine publication held meanwhile.
    // No metadata/value is manufactured and no lifecycle save is claimed here.
    world
        .get_mut::<AdmittedCommands>(actor)
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: SystemId("red-alert".into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
            response_token: None,
            feedback_correlation: None,
        });
    world
        .run_system_once(crate::console::captain::server::handle_set_red_alert)
        .unwrap();
    // Sparse owner fixtures register the unchanged capture schema, never work.
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::weapons::beam::ActiveBeam>();
    world.register_component::<crate::console::weapons::beam::PhaserCooldown>();
    world.register_component::<crate::console::weapons::TorpedoSystemResource>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(&world)).unwrap()).unwrap();
    (world, actor, saved)
}

#[test]
fn genuine_held_alert_survives_canonical_change_without_refresh_or_private_value_rewrite() {
    let (mut world, actor, saved) = held_publication();
    let row = saved
        .entities
        .iter()
        .find(|row| row.uuid == "held-alert")
        .unwrap();
    assert_eq!(row.red_alert, Some(true));
    let (_, SystemBlackboard::Captain(board)) = row
        .blackboards
        .iter()
        .find(|(_, board)| matches!(board, SystemBlackboard::Captain(_)))
        .unwrap()
    else {
        unreachable!()
    };
    assert!(
        !board.red_alert,
        "genuine earlier knowledge is not current machinery truth"
    );
    assert_eq!(
        board.alert_observation.as_ref().unwrap().observed_tick,
        Some(10)
    );
    validate_roster(&world, &saved).unwrap();
    // Change the live actuator through its actual handler again, so the
    // positive apply assertion proves this restore door actually wrote state.
    world
        .get_mut::<crate::core::messages::AdmittedCommands>(actor)
        .unwrap()
        .0 = vec![crate::core::messages::AdmittedCommand {
        target: SystemId("red-alert".into()),
        payload: crate::core::messages::SystemControlPayload::SetRedAlert { active: false },
        response_token: None,
        feedback_correlation: None,
    }];
    world
        .run_system_once(crate::console::captain::server::handle_set_red_alert)
        .unwrap();
    assert!(
        !world
            .get::<crate::ship::state::ShipRedAlert>(actor)
            .unwrap()
            .0
    );
    super::super::application::apply_entity_state(&mut world, actor, row, saved.tick);
    let restored = world
        .get::<crate::server_app::ShipSystemBlackboards>(actor)
        .unwrap();
    assert_eq!(
        restored.0.get(&SystemId("captain".into())),
        Some(&SystemBlackboard::Captain(board.clone()))
    );
    assert!(
        world
            .get::<crate::ship::state::ShipRedAlert>(actor)
            .unwrap()
            .0
    );
    let config = world
        .get::<crate::ship_plugin::ShipConfigComponent>(actor)
        .unwrap();
    let radio = &world
        .get::<crate::comms::radio::RadioProfile>(actor)
        .unwrap()
        .0;
    let sample = crate::comms::traffic::sample(config, restored, radio, 10);
    assert!(sample.is_some());
    let profile = radio.traffic.as_ref().unwrap();
    let mut trial = row.comms_traffic.clone().unwrap();
    let prepared = trial
        .prepare_report(profile, &crate::comms::traffic::channels(radio), sample, 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared.body(),
        profile.alert.as_ref().unwrap().routine_text,
        "permitted historical false stays false despite current private Alert=true"
    );
    assert!(
        crate::comms::traffic::sample(config, restored, radio, 12).is_none(),
        "stale knowledge stays unknown instead of refreshing from the private Alert"
    );
    // An older held frontier is structurally valid even when unusable by Radio.
    validate_entity(&world, actor, row, 12).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..4 {
        let mut bad = row.clone();
        let (_, SystemBlackboard::Captain(board)) = bad
            .blackboards
            .iter_mut()
            .find(|(_, board)| matches!(board, SystemBlackboard::Captain(_)))
            .unwrap()
        else {
            unreachable!()
        };
        let observation = board.alert_observation.as_mut().unwrap();
        match fault {
            0 => observation.observed_tick = Some(13),
            1 => observation.provenance.system = Some(SystemId("foreign".into())),
            2 => observation.received_tick = Some(10),
            _ => board.red_alert_system_id = SystemId("foreign".into()),
        }
        assert!(validate_entity(&world, actor, &bad, 12).is_err());
        super::super::application::apply_entity_state(&mut world, actor, &bad, 12);
        assert_eq!(capture(&world), before, "fault {fault} writes nothing");
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
