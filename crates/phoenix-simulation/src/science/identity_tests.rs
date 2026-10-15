use super::*;
use bevy::ecs::system::RunSystemOnce;
const OWNER: &str = "identity-emitter";
fn id() -> SystemId {
    SystemId(OWNER.into())
}
pub(crate) fn config() -> crate::entities::config::EntityConfig {
    crate::entities::include_resolve::load_entity_config("assets/entities/identity_cruiser.toml")
        .unwrap()
}
pub(crate) fn fixture() -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(100));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config(),
        Vec3::new(20.0, 0.0, 0.0),
        "identity-source".into(),
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
        .get_mut::<ShipIdentityEmitters>(entity)
        .unwrap()
        .0
        .get_mut(&id())
        .unwrap()
        .set("enabled", CommandChannelValue::Boolean(value))
        .unwrap();
}
#[test]
fn actual_emission_pays_once_and_never_changes_entity_truth_or_reuses_prior_work() {
    let (mut world, entity) = fixture();
    let name = world
        .get::<crate::entities::spawner::EntityName>(entity)
        .map(|value| value.0.clone());
    let faction = world
        .get::<crate::entities::spawner::FactionComponent>(entity)
        .map(|value| value.0);
    let pose = *world
        .get::<crate::ship::state::ShipPhysics>(entity)
        .unwrap();
    let initial = reserve(&world, entity);
    world.run_system_once(tick).unwrap();
    assert_eq!(reserve(&world, entity), initial);
    assert!(world
        .get::<IdentityEmissions>(entity)
        .unwrap()
        .claim(100)
        .is_none());
    enable(&mut world, entity, true);
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 101;
    world.run_system_once(tick).unwrap();
    assert!(reserve(&world, entity) < initial);
    let claim = world
        .get::<IdentityEmissions>(entity)
        .unwrap()
        .claim(101)
        .unwrap();
    assert_eq!(claim.callsign, "Transit courier");
    assert_eq!(claim.faction, None);
    assert_eq!(
        world
            .get::<crate::entities::spawner::EntityName>(entity)
            .map(|value| value.0.clone()),
        name
    );
    assert_eq!(
        world
            .get::<crate::entities::spawner::FactionComponent>(entity)
            .map(|value| value.0),
        faction
    );
    let after = world
        .get::<crate::ship::state::ShipPhysics>(entity)
        .unwrap();
    assert_eq!(
        (after.x, after.y, after.z, after.yaw),
        (pose.x, pose.y, pose.z, pose.yaw)
    );
    let paid = reserve(&world, entity);
    world.run_system_once(tick).unwrap();
    assert_eq!(reserve(&world, entity), paid);
    assert!(world
        .get::<IdentityEmissions>(entity)
        .unwrap()
        .claim(102)
        .is_none());
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 102;
    world
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .spend_continuous(&id(), paid - 0.001)
        .unwrap();
    world.run_system_once(tick).unwrap();
    assert!(
        world
            .get::<IdentityEmissions>(entity)
            .unwrap()
            .claim(102)
            .is_none(),
        "partial payment cannot buy a full identity claim"
    );
    assert!(reserve(&world, entity) < 0.001);
}
#[test]
fn offline_missing_or_unfunded_actual_equipment_has_no_claim_or_bootstrap_credit() {
    for kind in 0..3 {
        let (mut world, entity) = fixture();
        enable(&mut world, entity, true);
        if kind == 0 {
            world
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap()
                .0
                .set_offline(id(), true);
        }
        if kind == 1 {
            world.entity_mut(entity).remove::<ShipDamage>();
        }
        if kind == 2 {
            let current = reserve(&world, entity);
            world
                .get_mut::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .spend_continuous(&id(), current)
                .unwrap();
        }
        let before = reserve(&world, entity);
        world.run_system_once(tick).unwrap();
        assert_eq!(reserve(&world, entity), before);
        assert!(world
            .get::<IdentityEmissions>(entity)
            .unwrap()
            .claim(100)
            .is_none());
    }
}

#[test]
fn malformed_intent_and_half_present_fitting_refuse_before_payment() {
    for fault in 0..2 {
        let (mut world, entity) = fixture();
        enable(&mut world, entity, true);
        if fault == 0 {
            world
                .get_mut::<ShipIdentityEmitters>(entity)
                .unwrap()
                .0
                .get_mut(&id())
                .unwrap()
                .0
                .insert("enabled".into(), CommandChannelValue::EntityTarget(None));
        } else {
            // Missing compiled hardware with finite fitted state must fail
            // closed; it cannot be mistaken for the valid absent-pair case.
            world
                .entity_mut(entity)
                .remove::<crate::resources::fitting_server::CompiledShipFitting>();
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
        let before = reserve(&world, entity);
        world.run_system_once(tick).unwrap();
        assert_eq!(reserve(&world, entity), before);
        assert!(world
            .get::<IdentityEmissions>(entity)
            .unwrap()
            .claim(100)
            .is_none());
    }
}
