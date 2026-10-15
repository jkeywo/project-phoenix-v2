use super::*;
use bevy::ecs::system::RunSystemOnce;
fn id() -> SystemId {
    SystemId("concealment".into())
}
pub(crate) fn config() -> crate::entities::config::EntityConfig {
    crate::entities::include_resolve::load_entity_config("assets/entities/concealment_cruiser.toml")
        .unwrap()
}
#[test]
fn optional_cloak_hull_keeps_every_base_science_rating_assignment_and_real_equipment() {
    let base = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let equipped = crate::entities::include_resolve::load_entity_config(
        "assets/entities/concealment_cruiser.toml",
    )
    .unwrap();
    let base_ship = base.ship_config.as_ref().unwrap();
    let equipped_ship = equipped.ship_config.as_ref().unwrap();
    assert_eq!(base_ship.stations.len(), equipped_ship.stations.len());
    let science = |ship: &crate::ship::config::ShipConfig| {
        ship.stations
            .iter()
            .find(|row| row.id.0 == "science")
            .unwrap()
            .ratings
            .clone()
    };
    for old in science(base_ship) {
        let next = science(equipped_ship)
            .into_iter()
            .find(|row| row.name == old.name)
            .unwrap();
        assert!(old
            .automated_systems
            .iter()
            .all(|system| next.automated_systems.contains(system)));
        assert!(old
            .detailed_systems
            .as_ref()
            .unwrap()
            .iter()
            .all(|system| next.detailed_systems.as_ref().unwrap().contains(system)));
        if old.name == "Std" {
            assert!(next.detailed_systems.unwrap().contains(&id()));
        } else {
            assert!(next.automated_systems.contains(&id()));
        }
    }
    for system in &base_ship.systems {
        assert!(equipped_ship.systems.iter().any(|next| next == system));
    }
    assert_eq!(base.shield_arcs, equipped.shield_arcs);
}
pub(crate) fn fixture() -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(100));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    let config = config();
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::new(20.0, 0.0, 0.0),
        "concealment-source".into(),
        None,
    );
    world.flush();
    (world, entity)
}
fn reserve(world: &World, entity: Entity) -> f32 {
    world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&id()]
        .energy
}
pub(crate) fn enable(world: &mut World, entity: Entity, value: bool) {
    world
        .get_mut::<ShipConcealmentEmitters>(entity)
        .unwrap()
        .0
        .get_mut(&id())
        .unwrap()
        .set("enabled", CommandChannelValue::Boolean(value))
        .unwrap();
}
fn factor(world: &World, entity: Entity) -> f32 {
    perceptibility(
        world.get::<ConcealmentFields>(entity),
        world.get::<ShipPowerSystem>(entity),
        world.get::<ShipConfigComponent>(entity),
        world.resource::<crate::sim_tick::SimTick>().0,
        world.resource::<Time<Fixed>>().timestep().as_secs_f32(),
    )
}
#[test]
fn field_pays_once_partial_grants_scale_and_prior_work_cannot_survive_a_new_tick() {
    let (mut world, entity) = fixture();
    let initial = reserve(&world, entity);
    world.run_system_once(tick).unwrap();
    assert_eq!(reserve(&world, entity), initial);
    assert_eq!(factor(&world, entity), 1.0);
    enable(&mut world, entity, true);
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 101;
    world.run_system_once(tick).unwrap();
    let full = factor(&world, entity);
    assert!(full > 0.0 && full < 1.0);
    let after = reserve(&world, entity);
    assert!(after < initial);
    world.run_system_once(tick).unwrap();
    assert_eq!(reserve(&world, entity), after);
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 102;
    assert_eq!(
        factor(&world, entity),
        1.0,
        "last interval cannot provide prepaid field work"
    );
    world
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .spend_continuous(&id(), after - 0.001)
        .unwrap();
    world.run_system_once(tick).unwrap();
    let field = world
        .get::<ConcealmentFields>(entity)
        .unwrap()
        .entries
        .get(&id())
        .unwrap();
    assert!(field.paid_fraction > 0.0 && field.paid_fraction < 1.0);
    assert!(factor(&world, entity) > full && factor(&world, entity) < 1.0);
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 103;
    world.run_system_once(tick).unwrap();
    assert_eq!(factor(&world, entity), 1.0);
}
#[test]
fn actual_emission_weakens_a_still_enabled_field_without_changing_body_or_truth() {
    let (mut world, entity) = fixture();
    enable(&mut world, entity, true);
    let name = world
        .get::<crate::entities::spawner::EntityName>(entity)
        .map(|name| name.0.clone());
    let pose = *world
        .get::<crate::ship::state::ShipPhysics>(entity)
        .unwrap();
    world
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .set_generation_enabled(false);
    world.run_system_once(tick).unwrap();
    let quiet = factor(&world, entity);
    let dt = world.resource::<Time<Fixed>>().timestep().as_secs_f32();
    let grant = world
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .work_continuous(&SystemId("phaser-fore".into()), dt, 1000.0)
        .unwrap();
    assert!(grant.granted > 0.0);
    assert!(factor(&world, entity) > quiet);
    let config = world.get::<ShipConfigComponent>(entity).unwrap();
    assert!(world
        .get::<ShipConcealmentEmitters>(entity)
        .unwrap()
        .enabled(&config.0, &id()));
    assert_eq!(
        operating_signature(world.get::<ShipPowerSystem>(entity), Some(config), dt)
            .unwrap()
            .1,
        phoenix_model::sensing::PassiveSignature::Weapon
    );
    assert_eq!(
        world
            .get::<crate::entities::spawner::EntityName>(entity)
            .map(|name| name.0.clone()),
        name
    );
    let after = world
        .get::<crate::ship::state::ShipPhysics>(entity)
        .unwrap();
    assert_eq!(
        (after.x, after.y, after.z, after.yaw),
        (pose.x, pose.y, pose.z, pose.yaw)
    );
}
#[test]
fn invalid_unavailable_and_unfunded_hardware_has_no_field_or_free_work() {
    for fault in 0..5 {
        let (mut world, entity) = fixture();
        enable(&mut world, entity, true);
        match fault {
            0 => world
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap()
                .0
                .set_offline(id(), true),
            1 => {
                world.entity_mut(entity).remove::<ShipDamage>();
            }
            2 => {
                let energy = reserve(&world, entity);
                world
                    .get_mut::<ShipPowerSystem>(entity)
                    .unwrap()
                    .0
                    .spend_continuous(&id(), energy)
                    .unwrap();
            }
            3 => {
                world
                    .get_mut::<ShipConcealmentEmitters>(entity)
                    .unwrap()
                    .0
                    .get_mut(&id())
                    .unwrap()
                    .0
                    .insert("enabled".into(), CommandChannelValue::EntityTarget(None));
            }
            _ => {
                world
                    .entity_mut(entity)
                    .insert(crate::resources::fitting_server::ShipFitting {
                        equipment: Default::default(),
                        requested: Default::default(),
                        work: Default::default(),
                        next_request: 1,
                        last_tick: None,
                    });
            }
        }
        let before = reserve(&world, entity);
        world.run_system_once(tick).unwrap();
        assert_eq!(reserve(&world, entity), before);
        assert_eq!(factor(&world, entity), 1.0);
    }
}
#[test]
fn canonical_intent_round_trips_and_rejects_wrong_owner_channel_or_type() {
    let (mut world, entity) = fixture();
    enable(&mut world, entity, true);
    let state = world.get::<ShipConcealmentEmitters>(entity).unwrap();
    let config = world.get::<ShipConfigComponent>(entity).unwrap();
    let restored: ShipConcealmentEmitters =
        serde_json::from_str(&serde_json::to_string(state).unwrap()).unwrap();
    assert_eq!(&restored, state);
    restored.validate(&config.0).unwrap();
    for fault in 0..3 {
        let mut bad = restored.clone();
        match fault {
            0 => {
                bad.0.remove(&id());
            }
            1 => {
                bad.0.insert(SystemId("foreign".into()), Default::default());
            }
            _ => {
                bad.0
                    .get_mut(&id())
                    .unwrap()
                    .0
                    .insert("enabled".into(), CommandChannelValue::EntityTarget(None));
            }
        }
        assert!(bad.validate(&config.0).is_err());
        assert_eq!(world.get::<ShipConcealmentEmitters>(entity).unwrap(), state);
    }
    assert_eq!(ConcealmentFields::default().perceptibility(100, 0.0), 1.0);
}
