use super::*;
use crate::science::concealment::{ConcealmentFields, ShipConcealmentEmitters};
use bevy::ecs::system::RunSystemOnce;
use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let (mut world, entity) = crate::science::concealment::tests::fixture();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    crate::science::concealment::tests::enable(&mut world, entity, true);
    world
        .run_system_once(crate::science::concealment::tick)
        .unwrap();
    let saved = capture(&world);
    assert!(saved
        .entities
        .iter()
        .find(|row| row.uuid == "concealment-source")
        .unwrap()
        .concealment_emitters
        .is_some());
    (world, entity, saved)
}
#[test]
fn json_saved_intent_overrides_authored_initial_and_restore_has_no_paid_credit() {
    let (mut world, entity, saved) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let row = decoded
        .entities
        .iter()
        .find(|row| row.uuid == "concealment-source")
        .unwrap();
    validate_entity(&world, entity, row).unwrap();
    let initial = ShipConcealmentEmitters::from_ship(
        &crate::science::concealment::tests::config()
            .ship_config
            .unwrap(),
    )
    .unwrap();
    assert_ne!(Some(&initial), row.concealment_emitters.as_ref());
    world.entity_mut(entity).insert(initial);
    super::super::application::apply_entity_state(&mut world, entity, row, decoded.tick);
    assert_eq!(
        world.get::<ShipConcealmentEmitters>(entity),
        row.concealment_emitters.as_ref()
    );
    assert_eq!(
        world
            .get::<ConcealmentFields>(entity)
            .unwrap()
            .perceptibility(decoded.tick, 0.0),
        1.0
    );
    let first = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..5 {
        let mut bad = row.clone();
        if fault == 0 {
            bad.concealment_emitters = None;
        } else {
            let channels = bad
                .concealment_emitters
                .as_mut()
                .unwrap()
                .0
                .get_mut(&SystemId("concealment".into()))
                .unwrap();
            match fault {
                1 => {
                    channels
                        .0
                        .insert("enabled".into(), CommandChannelValue::EntityTarget(None));
                }
                2 => {
                    channels.0.clear();
                }
                3 => {
                    channels
                        .0
                        .insert("extra".into(), CommandChannelValue::Boolean(true));
                }
                _ => {
                    bad.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
                        template_path: "assets/entities/alliance_cruiser.toml".into(),
                        name: "concealment-source".into(),
                        ..Default::default()
                    });
                }
            }
        }
        assert!(
            validate_entity(&world, entity, &bad).is_err(),
            "fault{fault}"
        );
        super::super::application::apply_entity_state(&mut world, entity, &bad, decoded.tick + 1);
        assert_eq!(capture(&world), first);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
#[test]
fn absent_actor_exact_recipe_validates_before_materialization_and_bad_intent_never_writes() {
    let (mut world, entity, mut saved) = fixture();
    saved
        .entities
        .iter_mut()
        .find(|row| row.uuid == "concealment-source")
        .unwrap()
        .spawn = Some(crate::world::spawn_origin::SpawnOrigin {
        template_path: "assets/entities/concealment_cruiser.toml".into(),
        name: "concealment-source".into(),
        ..Default::default()
    });
    world.despawn(entity);
    validate_roster(&world, &saved).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..3 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == "concealment-source")
            .unwrap();
        match fault {
            0 => row.concealment_emitters = None,
            1 => {
                row.concealment_emitters
                    .as_mut()
                    .unwrap()
                    .0
                    .get_mut(&SystemId("concealment".into()))
                    .unwrap()
                    .0
                    .insert("foreign".into(), CommandChannelValue::Boolean(true));
            }
            _ => {
                row.spawn.as_mut().unwrap().template_path =
                    "assets/entities/no-such-transponder.toml".into()
            }
        }
        assert!(validate_roster(&world, &bad).is_err());
        let report = restore(&mut world, &bad);
        assert!(!report.gaps.is_empty());
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
