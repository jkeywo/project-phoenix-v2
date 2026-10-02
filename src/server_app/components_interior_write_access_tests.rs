use super::*;
use bevy::ecs::system::{IntoSystem, System};

fn assert_writes<R: Resource, Mint: Resource, M>(
    system: impl IntoSystem<(), (), M>,
    rng: bool,
    mint: bool,
) {
    let mut world = World::new();
    crate::sim_rng::install(
        &mut world,
        crate::sim_rng::SimRng::new(1400, crate::sim_rng::SeedSource::Cli),
    );
    crate::world_id::install(&mut world, crate::world_id::WorldIdMint::default());
    let rng_id = world.components().resource_id::<R>().unwrap();
    let mint_id = world.components().resource_id::<Mint>().unwrap();
    let mut system = IntoSystem::into_system(system);
    let access = system.initialize(&mut world);
    assert_eq!(
        access.combined_access().has_resource_write(rng_id),
        rng,
        "{} RNG",
        system.name()
    );
    assert_eq!(
        access.combined_access().has_resource_write(mint_id),
        mint,
        "{} mint",
        system.name()
    );
}

#[test]
fn actual_damage_systems_expose_rng_without_spurious_mint_writes() {
    assert_writes::<crate::sim_rng::TorpedoRng, crate::world_id::ProjectileMint, _>(
        crate::console::weapons::torpedo::tick_torpedo_lifecycle,
        true,
        true,
    );
    assert_writes::<crate::sim_rng::BlasterRng, crate::world_id::ProjectileMint, _>(
        crate::console::weapons::blaster::handle_blaster_hits,
        true,
        false,
    );
    assert_writes::<crate::sim_rng::CollisionRng, crate::world_id::MessageMint, _>(
        crate::server_app::collision::handle_collisions,
        true,
        false,
    );
}

#[test]
fn actual_gm_reducer_exposes_comms_mint_and_event_local_rng_stays_read_only() {
    assert_writes::<crate::sim_rng::CollisionRng, crate::world_id::MessageMint, _>(
        crate::gm_action::apply_due_actions,
        false,
        true,
    );
    assert_writes::<crate::sim_rng::CollisionRng, crate::world_id::MessageMint, _>(
        crate::gm_effect::apply_gm_direct_effects,
        false,
        false,
    );
}
