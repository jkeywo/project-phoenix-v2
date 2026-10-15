use super::*;
use bevy::ecs::system::RunSystemOnce;
const OWNER: &str = "signal-protection";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
pub(crate) fn config() -> crate::entities::config::EntityConfig {
    crate::entities::include_resolve::load_entity_config("assets/entities/protection_cruiser.toml")
        .unwrap()
}
pub(crate) fn fixture() -> (World, Entity) {
    let config = config();
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(100));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "protection-source".into(),
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
        .consumers[&id(OWNER)]
        .energy
}
fn suppression(world: &World, entity: Entity, receiver: &str) -> f64 {
    world
        .get::<ProtectionGrants>(entity)
        .unwrap()
        .suppression_for(
            &id(receiver),
            world.resource::<crate::sim_tick::SimTick>().0,
        )
}
fn set(world: &mut World, entity: Entity, value: CommandChannelValue) {
    world
        .get_mut::<ShipSignalProtection>(entity)
        .unwrap()
        .0
        .get_mut(&id(OWNER))
        .unwrap()
        .set("priorities", value)
        .unwrap();
}
#[test]
fn enabled_actual_hardware_pays_without_exposure_and_allocates_only_whole_declared_priorities() {
    let (mut world, entity) = fixture();
    let initial = reserve(&world, entity);
    assert_eq!(suppression(&world, entity, "comms"), 0.0);
    world.run_system_once(tick).unwrap();
    assert!(reserve(&world, entity) < initial);
    assert_eq!(suppression(&world, entity, "comms"), 4.0);
    assert_eq!(suppression(&world, entity, "tactical-radar"), 0.0);
    assert_eq!(suppression(&world, entity, "sensors"), 4.0);
    let paid = reserve(&world, entity);
    world.run_system_once(tick).unwrap();
    assert_eq!(
        reserve(&world, entity),
        paid,
        "duplicate boundary cannot pay twice"
    );
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    assert_eq!(
        suppression(&world, entity, "comms"),
        0.0,
        "no prior-step credit"
    );
    set(
        &mut world,
        entity,
        CommandChannelValue::SystemPriority(vec![id("tactical-radar"), id("comms"), id("sensors")]),
    );
    world.run_system_once(tick).unwrap();
    assert_eq!(suppression(&world, entity, "tactical-radar"), 4.0);
    assert_eq!(suppression(&world, entity, "comms"), 0.0);
    assert_eq!(suppression(&world, entity, "sensors"), 4.0);
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    set(
        &mut world,
        entity,
        CommandChannelValue::SystemPriority(vec![]),
    );
    let before = reserve(&world, entity);
    world.run_system_once(tick).unwrap();
    assert!(
        reserve(&world, entity) < before,
        "enabled empty allocation still operates"
    );
    assert_eq!(suppression(&world, entity, "comms"), 0.0);
}
#[test]
fn explicit_offline_missing_power_and_destroyed_operation_cannot_pay_or_protect() {
    for fault in 0..4 {
        let (mut world, entity) = fixture();
        match fault {
            0 => world
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap()
                .0
                .set(
                    id(OWNER),
                    crate::ship::control_source::ControlSource::Offline,
                ),
            1 => {
                world.entity_mut(entity).remove::<ShipPowerSystem>();
            }
            2 => {
                world
                    .get_mut::<ShipSystemControlSources>(entity)
                    .unwrap()
                    .0
                    .set_destroyed(true);
            }
            _ => {
                world.entity_mut(entity).remove::<ShipDamage>();
            }
        }
        let power = world
            .get::<ShipPowerSystem>(entity)
            .map(|power| power.0.capture_continuation());
        world.run_system_once(tick).unwrap();
        assert_eq!(suppression(&world, entity, "comms"), 0.0);
        assert_eq!(
            world
                .get::<ShipPowerSystem>(entity)
                .map(|power| power.0.capture_continuation()),
            power
        );
        assert!(!world
            .get::<ProtectionGrants>(entity)
            .unwrap()
            .is_unprocessed());
    }
}
#[test]
fn stationary_owner_and_partial_actual_work_keep_same_finite_allocation_without_free_resume() {
    let (mut world, entity) = fixture();
    world
        .entity_mut(entity)
        .remove::<(crate::ship::state::ShipPhysics, Transform)>();
    let dt = world.resource::<Time<Fixed>>().timestep().as_secs_f32();
    let cost = world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .config()
        .consumers
        .iter()
        .find(|consumer| consumer.system == id(OWNER))
        .unwrap()
        .continuous_cost_per_sec;
    world
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&id(OWNER))
        .unwrap()
        .energy = cost * dt * 0.25;
    world.run_system_once(tick).unwrap();
    let partial = suppression(&world, entity, "comms");
    assert!(partial > 0.0 && partial < 4.0);
    assert_eq!(suppression(&world, entity, "sensors"), partial);
    assert_eq!(suppression(&world, entity, "tactical-radar"), 0.0);
    assert_eq!(reserve(&world, entity), 0.0);
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    world.run_system_once(tick).unwrap();
    assert_eq!(suppression(&world, entity, "comms"), 0.0);
}
