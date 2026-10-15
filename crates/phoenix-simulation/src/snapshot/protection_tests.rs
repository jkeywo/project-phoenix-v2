use super::*;
use crate::science::protection::{ProtectionGrants, ShipSignalProtection};
use bevy::ecs::system::RunSystemOnce;
use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let (mut world, entity) = crate::science::protection::tests::fixture();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world
        .get_mut::<ShipSignalProtection>(entity)
        .unwrap()
        .0
        .get_mut(&SystemId("signal-protection".into()))
        .unwrap()
        .set(
            "priorities",
            CommandChannelValue::SystemPriority(vec![
                SystemId("tactical-radar".into()),
                SystemId("sensors".into()),
            ]),
        )
        .unwrap();
    world
        .run_system_once(crate::science::protection::tick)
        .unwrap();
    let saved = capture(&world);
    assert!(saved
        .entities
        .iter()
        .find(|row| row.uuid == "protection-source")
        .unwrap()
        .signal_protection
        .is_some());
    (world, entity, saved)
}
#[test]
fn json_saved_priorities_override_authored_initial_and_restore_has_no_paid_credit() {
    let (mut world, entity, saved) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let row = decoded
        .entities
        .iter()
        .find(|row| row.uuid == "protection-source")
        .unwrap();
    validate_entity(&world, entity, row).unwrap();
    let initial = ShipSignalProtection::from_ship(
        &crate::science::protection::tests::config()
            .ship_config
            .unwrap(),
    )
    .unwrap();
    assert_ne!(Some(&initial), row.signal_protection.as_ref());
    world.entity_mut(entity).insert(initial);
    super::super::application::apply_entity_state(&mut world, entity, row, decoded.tick);
    assert_eq!(
        world.get::<ShipSignalProtection>(entity),
        row.signal_protection.as_ref()
    );
    assert!(world
        .get::<ProtectionGrants>(entity)
        .unwrap()
        .is_unprocessed());
    assert_eq!(
        world
            .get::<ProtectionGrants>(entity)
            .unwrap()
            .suppression_for(&SystemId("tactical-radar".into()), decoded.tick),
        0.0
    );
    let first = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..6 {
        let mut bad = row.clone();
        if fault == 0 {
            bad.signal_protection = None;
        } else {
            let channels = bad
                .signal_protection
                .as_mut()
                .unwrap()
                .0
                .get_mut(&SystemId("signal-protection".into()))
                .unwrap();
            match fault {
                1 => {
                    channels
                        .0
                        .insert("priorities".into(), CommandChannelValue::Boolean(true));
                }
                2 => {
                    channels.0.insert(
                        "priorities".into(),
                        CommandChannelValue::SystemPriority(vec![SystemId("phaser-fore".into())]),
                    );
                }
                3 => {
                    channels.0.insert(
                        "priorities".into(),
                        CommandChannelValue::SystemPriority(vec![SystemId("sensors".into()); 2]),
                    );
                }
                4 => {
                    channels
                        .0
                        .insert("extra".into(), CommandChannelValue::Boolean(true));
                }
                _ => {
                    bad.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
                        template_path: "assets/entities/alliance_cruiser.toml".into(),
                        name: "protection-source".into(),
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
        .find(|row| row.uuid == "protection-source")
        .unwrap()
        .spawn = Some(crate::world::spawn_origin::SpawnOrigin {
        template_path: "assets/entities/protection_cruiser.toml".into(),
        name: "protection-source".into(),
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
            .find(|row| row.uuid == "protection-source")
            .unwrap();
        match fault {
            0 => row.signal_protection = None,
            1 => {
                row.signal_protection
                    .as_mut()
                    .unwrap()
                    .0
                    .get_mut(&SystemId("signal-protection".into()))
                    .unwrap()
                    .0
                    .insert(
                        "priorities".into(),
                        CommandChannelValue::SystemPriority(vec![SystemId("foreign".into())]),
                    );
            }
            _ => {
                row.spawn.as_mut().unwrap().template_path =
                    "assets/entities/no-such-protector.toml".into()
            }
        }
        assert!(validate_roster(&world, &bad).is_err());
        let report = restore(&mut world, &bad);
        assert!(!report.gaps.is_empty());
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
