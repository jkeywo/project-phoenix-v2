use super::*;
use bevy::{ecs::system::RunSystemOnce, prelude::World};
fn entity(mint: LiveMint<'_, { IdNamespace::Entity as usize }>) -> String {
    mint_live_id_with(mint.as_deref(), IdNamespace::Entity)
}

fn projectile(mint: LiveMint<'_, { IdNamespace::Projectile as usize }>) -> String {
    mint_live_id_with(mint.as_deref(), IdNamespace::Projectile)
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
