use super::*;
use crate::ship::compartment_damage::ShipDamage;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.insert_resource(SimTick(20));
    world.insert_resource(Time::<Fixed>::default());
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "emp-ship".into(),
        None,
    );
    world.flush();
    world
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntitySpawnOrigin(
            crate::world::spawn_origin::SpawnOrigin {
                template_path: "assets/entities/alliance_cruiser.toml".into(),
                name: "emp-ship".into(),
                ..Default::default()
            },
        ));
    let clock = crate::ship::emp::clock(&world, 20).unwrap();
    let policy = config.damage.as_ref().unwrap().emp.as_ref().unwrap();
    let effect = phoenix_model::emp::ComponentEmpState::apply(
        None,
        policy,
        phoenix_model::emp::EmpPayload { duration_secs: 6.0 },
        clock,
    )
    .unwrap();
    {
        let mut damage = world.get_mut::<ShipDamage>(entity).unwrap();
        damage.emp_clock = Some(clock);
        damage
            .state
            .components
            .get_mut(&SystemId("phaser-fore".into()))
            .unwrap()
            .get_mut("operation")
            .unwrap()
            .emp = Some(effect.clone());
    }
    world
        .get_mut::<crate::resources::fitting_server::ShipFitting>(entity)
        .unwrap()
        .equipment
        .assemblies
        .values_mut()
        .find_map(|item| item.condition.as_mut())
        .unwrap()
        .emp = Some(effect);
    let snapshot = capture(&world);
    (world, entity, snapshot)
}

#[test]
fn json_restores_component_expiries_and_derives_current_clock_before_readiness() {
    let (mut world, entity, snapshot) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    validate_roster(&world, &decoded).unwrap();
    world.get_mut::<ShipDamage>(entity).unwrap().emp_clock = None;
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &decoded.entities[0],
        decoded.tick,
    );
    let damage = world.get::<ShipDamage>(entity).unwrap();
    assert_eq!(damage.emp_clock.unwrap().tick, decoded.tick);
    assert_eq!(
        damage.factor(
            &SystemId("phaser-fore".into()),
            phoenix_model::damage::DamageBinding::Operation
        ),
        0.0
    );
    assert_eq!(capture(&world), decoded);
    world.despawn(entity);
    validate_roster(&world, &decoded).unwrap();
}

#[test]
fn installed_and_portable_tampering_refuses_before_any_global_or_entity_write() {
    for variant in 0..6 {
        let (mut world, entity, mut snapshot) = fixture();
        let installed = snapshot.entities[0]
            .physical_damage
            .as_mut()
            .unwrap()
            .components
            .get_mut(&SystemId("phaser-fore".into()))
            .unwrap()
            .get_mut("operation")
            .unwrap()
            .emp
            .as_mut()
            .unwrap();
        match variant {
            0 => installed.applied_tick = 21,
            1 => installed.decay_ticks += 1,
            2 => installed.resistance_stacks = 9,
            3 => {
                snapshot.entities[0]
                    .fitting
                    .as_mut()
                    .unwrap()
                    .equipment
                    .assemblies
                    .values_mut()
                    .find_map(|item| item.condition.as_mut())
                    .unwrap()
                    .emp
                    .as_mut()
                    .unwrap()
                    .applied_tick = 21
            }
            4 => {
                world.remove_resource::<Time<Fixed>>();
            }
            _ => {
                world.entity_mut(entity).remove::<ShipDamage>();
            }
        }
        world.resource_mut::<SimTick>().0 = 7;
        world.spawn((EntityUuid("untouched-surplus".into()), ShipRedAlert(false)));
        snapshot.entities[0].red_alert = Some(true);
        let before = capture(&world);
        let report = restore(&mut world, &snapshot);
        assert!(!report.is_complete(), "variant {variant}");
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(report.despawned, 0);
        assert_eq!(capture(&world), before, "variant {variant}");
    }
}
