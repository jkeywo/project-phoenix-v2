use super::*;
use crate::modifiers::power_system::PowerSystem;
use crate::sim_rng::{SeedSource, SimRng};
use bevy::ecs::system::RunSystemOnce;

fn fixture(reordered: bool) -> (World, Entity) {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    world.insert_resource(time);
    world.init_resource::<crate::effect_queue::EffectQueue<CapacityAdjustment>>();
    crate::sim_rng::install(&mut world, SimRng::new(781, SeedSource::World));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &authored,
        Vec3::ZERO,
        "selection-owner".into(),
        None,
    );
    world.flush();
    let mut ship = authored.ship_config.clone().unwrap();
    if reordered {
        ship.systems.reverse();
        // Accepted Power definitions preserve repeated rows rather than using
        // an identity set. This also exercises multiple reactor declarations.
        for kind in [
            crate::ship::system_registry::POWER_REACTOR_KIND,
            crate::ship::system_registry::NAVIGATION_KIND,
        ] {
            let row = ship
                .systems
                .iter()
                .find(|system| system.kind == kind)
                .unwrap()
                .clone();
            ship.systems.insert(0, row);
        }
    }
    let power = PowerSystem::from_energy(authored.energy.unwrap(), ship).unwrap();
    world
        .entity_mut(entity)
        .insert((ShipPowerSystem(power), PowerBrownoutState::default()));
    (world, entity)
}

#[test]
fn compiled_adapter_selection_matches_original_tick_and_current_services() {
    for reordered in [false, true] {
        let (mut actual, a) = fixture(reordered);
        let (mut oracle, b) = fixture(reordered);
        let original_services = actual.get::<ResourceServices>(a).unwrap().clone();
        for step in 0..5 {
            for (world, entity) in [(&mut actual, a), (&mut oracle, b)] {
                if step == 1 {
                    // Service bindings remain live, not cached with the definition.
                    let power = &world.get::<ShipPowerSystem>(entity).unwrap().0;
                    let link = &power.config().reactor.coolant;
                    assert!(link.provider.is_none());
                    let service = link.service.clone().unwrap_or_else(|| "coolant".into());
                    world.get_mut::<ResourceServices>(entity).unwrap().0.insert(
                        service,
                        ResourceEndpoint::Capacity {
                            id: "absent-current-endpoint".into(),
                        },
                    );
                }
                if step == 2 {
                    world.entity_mut(entity).insert(original_services.clone());
                    let mut power = world.get_mut::<ShipPowerSystem>(entity).unwrap();
                    let mut hardware = power.0.config().clone();
                    for consumer in &mut hardware.consumers {
                        consumer.continuous_cost_per_sec *= 0.75;
                    }
                    power.0.reconfigure_energy(hardware).unwrap();
                    let saved = power.0.capture_continuation();
                    power.0.restore_continuation(&saved).unwrap();
                }
                if step == 3 {
                    let id = world
                        .get::<ShipPowerSystem>(entity)
                        .unwrap()
                        .0
                        .adapter_selection()
                        .passive_work_systems()
                        .next()
                        .unwrap()
                        .clone();
                    world
                        .get_mut::<crate::ship_plugin::ShipSystemControlSources>(entity)
                        .unwrap()
                        .0
                        .set_offline(id, true);
                }
                if step == 4 {
                    *world.resource_mut::<Time>() = Time::default();
                }
            }
            let power = &actual.get::<ShipPowerSystem>(a).unwrap().0;
            let selection = power.adapter_selection();
            assert_eq!(selection.reactor_systems().collect::<Vec<_>>(), power.ship_config().systems.iter()
                .filter(|system| system.kind == crate::ship::system_registry::POWER_REACTOR_KIND)
                .map(|system| &system.id).collect::<Vec<_>>());
            let expected: Vec<_> = [
                crate::ship::system_registry::HELM_RADAR_KIND,
                crate::ship::system_registry::TACTICAL_RADAR_KIND,
                crate::ship::system_registry::SENSOR_RADAR_KIND,
                crate::ship::system_registry::NAVIGATION_KIND,
            ]
            .into_iter()
            .flat_map(|kind| {
                power
                    .ship_config()
                    .systems
                    .iter()
                    .filter(move |system| system.kind == kind)
                    .map(|system| &system.id)
            })
            .collect();
            assert_eq!(
                selection.passive_work_systems().collect::<Vec<_>>(),
                expected
            );
            actual.run_system_once(tick_power_system).unwrap();
            oracle
                .run_system_once(selection_reference::tick_reference)
                .unwrap();
            let actual_power = &actual.get::<ShipPowerSystem>(a).unwrap().0;
            let oracle_power = &oracle.get::<ShipPowerSystem>(b).unwrap().0;
            assert_eq!(
                actual_power, oracle_power,
                "reordered {reordered}, step {step}"
            );
            assert_eq!(
                serde_json::to_string(&actual_power.capture_continuation()).unwrap(),
                serde_json::to_string(&oracle_power.capture_continuation()).unwrap()
            );
            for consumer in &actual_power.config().consumers {
                assert_eq!(
                    actual_power.emitted_work(&consumer.system).to_bits(),
                    oracle_power.emitted_work(&consumer.system).to_bits()
                );
            }
            assert_eq!(
                actual
                    .resource::<crate::effect_queue::EffectQueue<CapacityAdjustment>>()
                    .0,
                oracle
                    .resource::<crate::effect_queue::EffectQueue<CapacityAdjustment>>()
                    .0
            );
            assert_eq!(
                actual.get::<PowerBrownoutState>(a).unwrap().locked_changed,
                oracle.get::<PowerBrownoutState>(b).unwrap().locked_changed
            );
            assert_eq!(
                actual.resource::<SimRng>().state(),
                oracle.resource::<SimRng>().state()
            );
        }
    }
}
