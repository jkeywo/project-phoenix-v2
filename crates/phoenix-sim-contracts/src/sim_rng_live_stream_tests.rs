use super::*;
use bevy::ecs::system::RunSystemOnce;

fn draw_beam(rng: LiveStream<{ SimStream::BeamDamage as usize }>) -> u32 {
    with_live_stream(rng.as_deref(), |stream| stream.next_u32())
}
fn draw_cycle(rng: LiveStream<{ SimStream::BeamCycleJitter as usize }>) -> u32 {
    with_live_stream(rng.as_deref(), |stream| stream.next_u32())
}

#[test]
fn live_streams_share_the_exact_aggregate_state_and_preserve_other_positions() {
    let mut world = World::new();
    install(&mut world, SimRng::new(1400, SeedSource::Cli));
    let reference = SimRng::new(1400, SeedSource::Cli);
    for _ in 0..4 {
        assert_eq!(
            world.run_system_once(draw_cycle).unwrap(),
            reference.stream(SimStream::BeamCycleJitter).next_u32()
        );
        assert_eq!(
            world.run_system_once(draw_beam).unwrap(),
            reference.stream(SimStream::BeamDamage).next_u32()
        );
    }
    assert_eq!(world.resource::<SimRng>().state(), reference.state());
    assert_eq!(
        ron::to_string(&world.resource::<SimRng>().state()).unwrap(),
        ron::to_string(&reference.state()).unwrap()
    );
    // The legacy fingerprint can advance the aggregate outside execution;
    // the next real typed draw must see that exact same cell, not a clone.
    world
        .resource::<SimRng>()
        .stream(SimStream::BeamDamage)
        .next_u32();
    reference.stream(SimStream::BeamDamage).next_u32();
    assert_eq!(
        world.run_system_once(draw_beam).unwrap(),
        reference.stream(SimStream::BeamDamage).next_u32()
    );
}

#[test]
fn reseed_restore_and_rollback_replace_live_cells_before_the_next_draw() {
    let mut world = World::new();
    install(&mut world, SimRng::new(1, SeedSource::Random));
    world.run_system_once(draw_beam).unwrap();
    install(&mut world, SimRng::new(1400, SeedSource::World));
    let checkpoint = world.resource::<SimRng>().state();
    let reference = SimRng::from_state(checkpoint.clone()).unwrap();
    assert_eq!(
        world.run_system_once(draw_beam).unwrap(),
        reference.stream(SimStream::BeamDamage).next_u32()
    );
    let resumed = world.resource::<SimRng>().state();
    // Successful restore of a progressed state, then rejected-transfer
    // rollback to the earlier checkpoint use the same replacement entry.
    for state in [resumed, checkpoint] {
        let reference = SimRng::from_state(state.clone()).unwrap();
        install(&mut world, SimRng::from_state(state).unwrap());
        assert_eq!(
            world.run_system_once(draw_cycle).unwrap(),
            reference.stream(SimStream::BeamCycleJitter).next_u32()
        );
        assert_eq!(
            world.run_system_once(draw_beam).unwrap(),
            reference.stream(SimStream::BeamDamage).next_u32()
        );
        assert_eq!(world.resource::<SimRng>().state(), reference.state());
    }
}

#[test]
#[should_panic(expected = "incomplete RNG stream installation")]
fn aggregate_without_live_handle_cannot_fall_back_to_entropy() {
    let mut world = World::new();
    world.insert_resource(SimRng::new(1400, SeedSource::Cli));
    world.run_system_once(draw_beam).unwrap();
}

#[test]
#[should_panic(expected = "stale RNG stream handle")]
fn replacing_only_the_aggregate_cannot_draw_the_old_seed() {
    let mut world = World::new();
    install(&mut world, SimRng::new(1, SeedSource::Cli));
    world.insert_resource(SimRng::new(1400, SeedSource::Cli));
    world.run_system_once(draw_beam).unwrap();
}
