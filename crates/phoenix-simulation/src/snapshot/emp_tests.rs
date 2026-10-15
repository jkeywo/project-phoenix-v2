use super::*;
use crate::ship::compartment_damage::ShipDamage;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    fixture_with(|_| {})
}
fn fixture_with(
    adjust: impl FnOnce(&mut crate::entities::config::EntityConfig),
) -> (World, Entity, PhoenixSnapshot) {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let origin_policy = config
        .damage
        .as_ref()
        .unwrap()
        .emp
        .as_ref()
        .unwrap()
        .clone();
    adjust(&mut config);
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
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
    let effect = phoenix_model::emp::ComponentEmpState::apply(
        None,
        &origin_policy,
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
    assert!(
        snapshot.entities[0].weapons.is_some(),
        "the actual machinery row must be captured"
    );
    (world, entity, snapshot)
}

#[test]
fn custom_or_absent_current_policy_cannot_rewrite_saved_installed_or_stowed_receipts() {
    for absent in [false, true] {
        let (mut world, entity, snapshot) = fixture_with(|config| {
            if absent {
                config.damage.as_mut().unwrap().emp = None;
            } else {
                let policy = config.damage.as_mut().unwrap().emp.as_mut().unwrap();
                policy.max_stacks = 1;
                policy.decay_secs = 1.0;
                policy.bindings = vec![phoenix_model::damage::DamageBinding::Cooling];
            }
        });
        let decoded: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        validate_roster(&world, &decoded).unwrap();
        super::super::application::apply_entity_state(
            &mut world,
            entity,
            &decoded.entities[0],
            decoded.tick,
        );
        assert_eq!(capture(&world), decoded);
        let damage = world.get::<ShipDamage>(entity).unwrap();
        damage
            .state
            .validate_emp_at(&damage.definition, damage.emp_clock.unwrap())
            .unwrap();
        assert_eq!(
            damage.factor(
                &SystemId("phaser-fore".into()),
                phoenix_model::damage::DamageBinding::Operation
            ),
            0.0
        );
    }
}

#[test]
fn json_restores_component_expiries_and_derives_current_clock_before_readiness() {
    let (mut world, entity, snapshot) = fixture();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    validate_roster(&world, &decoded).unwrap();
    super::super::preflight::validate_entity_resources(&world, entity, &decoded.entities[0])
        .unwrap();
    super::super::validate_entity_repair_jobs(&world, entity, &decoded.entities[0]).unwrap();
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
    for variant in 0..10 {
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
            5 => {
                world.entity_mut(entity).remove::<ShipDamage>();
            }
            6 => installed.policy.decay_secs = 0.0,
            7 => installed.policy.max_stacks = 0,
            8 => installed.policy.bindings = vec![phoenix_model::damage::DamageBinding::Cooling],
            _ => {
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
                    .policy
                    .resistance_per_stack = 0.0
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
