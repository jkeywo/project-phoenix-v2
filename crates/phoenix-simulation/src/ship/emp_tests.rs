use super::*;
use crate::entities::spawner::EntityUuid;
use crate::ship::compartment_damage::ShipDamage;
use crate::sim_rng::InstallSimRng;

#[test]
fn actual_runtime_spawn_initializes_existing_causal_clock_before_any_input_pass() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(321));
    world.insert_resource(Time::<Fixed>::from_seconds(0.1));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "runtime-receiver".into(),
        None,
    );
    world.flush();
    assert_eq!(
        world.get::<ShipDamage>(entity).unwrap().emp_clock,
        Some(phoenix_model::emp::EmpClock {
            tick: 321,
            period_nanos: 100_000_000
        })
    );
}

#[test]
fn actual_authored_emp_bolt_collision_uses_frozen_payload_and_temporary_component_failure() {
    for protected in [false, true] {
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/emp_destroyer.toml",
        )
        .unwrap();
        let bank = config
            .weapons_console
            .as_ref()
            .unwrap()
            .blaster_banks
            .iter()
            .find(|bank| bank.id == "port")
            .unwrap()
            .to_runtime();
        let frozen = bank.impact;
        assert_eq!(frozen.emp.unwrap().duration_secs, 6.0);
        let mut app = App::new();
        app.insert_resource(crate::lobby::WorldResource(Default::default()))
            .init_resource::<crate::server_app::SimOutbox>()
            .insert_resource(crate::sim_tick::SimTick(0))
            .insert_resource(Time::<Fixed>::from_seconds(1.0))
            .insert_sim_rng(crate::sim_rng::SimRng::new(
                971,
                crate::sim_rng::SeedSource::Cli,
            ))
            .add_message::<crate::ai::server::AiEntityDestroyed>()
            .add_message::<crate::console::weapons::AsteroidDestroyedVfx>()
            .add_message::<crate::console::weapons::ShipDestroyedVfx>()
            .add_systems(
                Update,
                (
                    stamp_clock,
                    crate::console::weapons::blaster::handle_blaster_hits,
                )
                    .chain(),
            );
        let target = app
            .world_mut()
            .spawn((
                EntityUuid("target".into()),
                Transform::default(),
                crate::ship::state::ShipPhysics::default(),
                ShipDamage::from_config(&config).unwrap().unwrap(),
            ))
            .id();
        if protected {
            app.world_mut()
                .entity_mut(target)
                .insert(crate::ship::shields::ShipShields(
                    crate::weapons::shield::ShieldSystem::new(
                        &crate::weapons::shield::ShieldConfig {
                            max_hp: 100,
                            num_facings: 4,
                            ..Default::default()
                        },
                    ),
                    0.5,
                ));
        }
        let mut weapon = crate::weapons::blaster::BlasterSystem::new(bank);
        weapon
            .in_flight
            .push(crate::weapons::blaster::BlasterProjectile {
                impact: frozen,
                id: "actual-emp-bolt".into(),
                x: 0.0,
                z: 0.0,
                heading: 0.0,
                speed: 1.0,
                lifespan_remaining: 1.0,
                collision_radius: 1.0,
                damage: 16,
                shield_pierce: 0.0,
                source_uuid: "shooter".into(),
            });
        // The in-flight copy is independent of subsequent equipment tuning.
        weapon.config.impact.emp.as_mut().unwrap().duration_secs = 100.0;
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::console::weapons::blaster::BlasterSystemResource(vec![weapon]),
        ));
        app.update();
        let damage = app.world().get::<ShipDamage>(target).unwrap();
        let affected: Vec<_> = damage
            .state
            .components
            .iter()
            .flat_map(|(system, components)| {
                components
                    .iter()
                    .filter(|(_, component)| component.emp.is_some())
                    .map(move |(component, _)| (system.clone(), component.clone()))
            })
            .collect();
        if protected {
            assert!(
                affected.is_empty(),
                "fully intercepted carrier cannot deposit EMP"
            );
            continue;
        }
        assert!(!affected.is_empty());
        for (system, component) in &affected {
            let state = &damage.state.components[system][component];
            assert_eq!(state.emp.as_ref().unwrap().effect_until_tick, 6);
            assert_eq!(
                damage.factor(system, phoenix_model::damage::DamageBinding::Operation),
                0.0
            );
            assert_ne!(
                state.condition,
                phoenix_model::damage::ComponentCondition::Destroyed
            );
        }
        let saved = damage.state.clone();
        let decoded: phoenix_sim_gameplay::ship::compartment_damage::DamageState =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(decoded, saved);
        for tick in 1..=360 {
            app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
            app.update();
            assert_eq!(
                app.world().get::<ShipDamage>(target).unwrap().state,
                saved,
                "expiry changes no HP/condition/dose or stored causal receipt"
            );
        }
        let damage = app.world().get::<ShipDamage>(target).unwrap();
        for (system, _) in &affected {
            assert!(damage.factor(system, phoenix_model::damage::DamageBinding::Operation) > 0.0);
        }
    }
}
