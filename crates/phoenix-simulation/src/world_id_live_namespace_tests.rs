use super::*;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::World;

fn entity(mint: LiveMint<'_, { IdNamespace::Entity as usize }>) -> String {
    mint_live_id_with(mint.as_deref(), IdNamespace::Entity)
}
fn projectile(mint: LiveMint<'_, { IdNamespace::Projectile as usize }>) -> String {
    mint_live_id_with(mint.as_deref(), IdNamespace::Projectile)
}

#[test]
fn live_namespaces_share_counters_and_follow_atomic_tick_reset() {
    let mut world = World::new();
    install(&mut world, WorldIdMint::default());
    let reference = WorldIdMint::default();
    for tick in [0, 0, 42, 42, 43] {
        world.resource::<WorldIdMint>().begin_tick(tick);
        reference.begin_tick(tick);
        assert_eq!(
            world.run_system_once(projectile).unwrap(),
            reference.mint(IdNamespace::Projectile).render()
        );
        assert_eq!(
            world.run_system_once(entity).unwrap(),
            reference.mint(IdNamespace::Entity).render()
        );
        assert_eq!(world.resource::<WorldIdMint>().state(), reference.state());
    }
}

#[test]
fn snapshot_restore_and_checkpoint_rollback_rebind_the_next_identity() {
    let mut world = World::new();
    install(&mut world, WorldIdMint::default());
    world.resource::<WorldIdMint>().begin_tick(42);
    world.run_system_once(entity).unwrap();
    let checkpoint = crate::snapshot::capture(&world);
    world.run_system_once(projectile).unwrap();
    world.run_system_once(entity).unwrap();
    let progressed = crate::snapshot::capture(&world);
    for snapshot in [&progressed, &checkpoint] {
        let expected = WorldIdMint::from_state(snapshot.mint.clone().unwrap());
        crate::snapshot::restore(&mut world, snapshot);
        assert_eq!(
            world.run_system_once(entity).unwrap(),
            expected.mint(IdNamespace::Entity).render()
        );
        assert_eq!(
            world.run_system_once(projectile).unwrap(),
            expected.mint(IdNamespace::Projectile).render()
        );
        assert_eq!(world.resource::<WorldIdMint>().state(), expected.state());
    }
}

#[test]
#[should_panic(expected = "incomplete namespace mint installation")]
fn aggregate_without_live_namespace_cannot_fall_back() {
    let mut world = World::new();
    world.insert_resource(WorldIdMint::default());
    world.run_system_once(entity).unwrap();
}

#[test]
#[should_panic(expected = "stale namespace mint")]
fn replacing_only_aggregate_cannot_mint_from_old_counter() {
    let mut world = World::new();
    install(&mut world, WorldIdMint::default());
    world.insert_resource(WorldIdMint::default());
    world.run_system_once(entity).unwrap();
}
