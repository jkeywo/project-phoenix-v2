use super::*;
use bevy::{ecs::system::RunSystemOnce, prelude::World};
fn draw_beam(rng: LiveStream<{ SimStream::BeamDamage as usize }>) -> u32 {
    with_live_stream(rng.as_deref(), |stream| stream.next_u32())
}

fn draw_cycle(rng: LiveStream<{ SimStream::BeamCycleJitter as usize }>) -> u32 {
    with_live_stream(rng.as_deref(), |stream| stream.next_u32())
}
#[test]
fn real_snapshot_restore_and_checkpoint_rollback_rebind_the_next_draw() {
    let mut world = World::new();
    install(&mut world, SimRng::new(1400, SeedSource::Cli));
    world.run_system_once(draw_beam).unwrap();
    let checkpoint = crate::snapshot::capture(&world);
    world.run_system_once(draw_cycle).unwrap();
    world.run_system_once(draw_beam).unwrap();
    let progressed = crate::snapshot::capture(&world);
    install(&mut world, SimRng::new(999, SeedSource::World));
    // The recovery rollback uses this same actual restore entry point;
    // this fixture isolates first-draw rebinding, not network admission.
    for snapshot in [&progressed, &checkpoint] {
        let expected = SimRng::from_state(snapshot.rng.clone().unwrap()).unwrap();
        crate::snapshot::restore(&mut world, snapshot);
        assert_eq!(
            world.run_system_once(draw_beam).unwrap(),
            expected.stream(SimStream::BeamDamage).next_u32()
        );
        assert_eq!(
            world.run_system_once(draw_cycle).unwrap(),
            expected.stream(SimStream::BeamCycleJitter).next_u32()
        );
        assert_eq!(world.resource::<SimRng>().state(), expected.state());
    }
}
