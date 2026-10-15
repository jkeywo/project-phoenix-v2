//! Actual paid passive consumer; finite bearing range, no synthetic signal rows.
use super::*;
use bevy::ecs::system::RunSystemOnce;
use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
fn id(value: &str) -> crate::core::messages::SystemId {
    crate::core::messages::SystemId(value.into())
}
#[test]
fn paid_sensors_interference_removes_only_current_passive_reception_then_selected_protection_recovers(
) {
    let (mut world, source, receiver, contact) = crate::comms::jamming::tests::fixture_for_effect(
        None,
        phoenix_sim_gameplay::electronic::jamming::JammingEffect::Sensors,
        true,
    );
    world.init_resource::<Time>();
    // The accepted ordinary emitter is25 ahead of the receiver's real100 range.
    // Its actual initial reactor output is the passive producer's physical cause.
    let emitter = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &emitter,
        Vec3::new(0.0, 0.0, -45.0),
        "passive-cause".into(),
        None,
    );
    world.flush();
    crate::comms::jamming::tests::prepare(&mut world, source, &contact);
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    let first = world.resource::<crate::sim_tick::SimTick>().0;
    world.run_system_once(tick).unwrap();
    let initial = world.get::<PassiveState>(receiver).unwrap().0[&id("passive-receiver")].clone();
    assert!(!initial.signals.is_empty());
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 31;
    world.run_system_once(crate::comms::jamming::tick).unwrap();
    world.run_system_once(tick).unwrap();
    let lost = &world.get::<PassiveState>(receiver).unwrap().0[&id("passive-receiver")];
    assert!(
        lost.signals.is_empty(),
        "actual25 exceeds100 scaled by paid exposure3.6"
    );
    assert!(lost.last_sample_tick.unwrap() > first);
    world
        .get_mut::<crate::science::protection::ShipSignalProtection>(receiver)
        .unwrap()
        .0
        .get_mut(&id("signal-protection"))
        .unwrap()
        .set(
            "priorities",
            CommandChannelValue::SystemPriority(vec![id("passive-receiver"), id("comms")]),
        )
        .unwrap();
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 31;
    world
        .run_system_once(crate::science::protection::tick)
        .unwrap();
    world.run_system_once(crate::comms::jamming::tick).unwrap();
    let point = Vec3::new(0.0, 0.0, -20.0);
    let other = world
        .run_system_once(
            move |paths: crate::regions::signal_path::RegionSignalQuery| {
                paths.sensors_receiver_factor(point, "actual-receiver", &id("tactical-radar"))
            },
        )
        .unwrap();
    assert!(other < 1.0, "recovery must not come from a silent jammer");
    world.run_system_once(tick).unwrap();
    let recovered = &world.get::<PassiveState>(receiver).unwrap().0[&id("passive-receiver")];
    assert!(!recovered.signals.is_empty());
    assert_eq!(
        recovered.last_sample_tick,
        Some(world.resource::<crate::sim_tick::SimTick>().0)
    );
    assert!(recovered
        .signals
        .iter()
        .all(|signal| signal.observed_tick > first));
}
