use super::*;

#[test]
fn actual_cylinder_and_capsule_shapes_do_not_become_xz_or_enclosing_sphere_hits() {
    let cylinder = Collider::cylinder(1.0, 10.0);
    assert!(collider_fraction(
        [-20.0, 5.0, 0.0],
        [20.0, 5.0, 0.0],
        0.1,
        Vec3::ZERO,
        Quat::IDENTITY,
        &cylinder
    )
    .is_none());
    let hit = collider_fraction(
        [-20.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        0.1,
        Vec3::ZERO,
        Quat::IDENTITY,
        &cylinder,
    )
    .unwrap();
    assert!((hit - 0.2475).abs() < 0.001);
    let capsule = Collider::capsule_y(5.0, 1.0);
    assert!(collider_fraction(
        [-10.0, 5.5, 0.0],
        [10.0, 5.5, 0.0],
        0.1,
        Vec3::ZERO,
        Quat::IDENTITY,
        &capsule
    )
    .is_some());
    assert!(collider_fraction(
        [-10.0, 7.0, 0.0],
        [10.0, 7.0, 0.0],
        0.1,
        Vec3::ZERO,
        Quat::IDENTITY,
        &capsule
    )
    .is_none());
}

fn app() -> App {
    use crate::sim_rng::InstallSimRng;
    let mut app = App::new();
    app.insert_resource(crate::lobby::WorldResource(Default::default()))
        .init_resource::<crate::server_app::SimOutbox>()
        .insert_sim_rng(crate::sim_rng::SimRng::new(
            975,
            crate::sim_rng::SeedSource::Cli,
        ))
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_message::<super::super::AsteroidDestroyedVfx>()
        .add_message::<super::super::ShipDestroyedVfx>()
        .add_systems(Update, super::super::blaster::handle_blaster_hits);
    app
}

fn incoming() -> super::super::TorpedoSystemResource {
    use crate::weapons::torpedo::{Torpedo, TorpedoConfig, TorpedoSystem};
    let config = TorpedoConfig {
        speed: 1.0,
        durability: 24.0,
        detonation_radius: 0.2,
        ..Default::default()
    };
    let profile = config.round_profile(Default::default());
    let mut system = TorpedoSystem::new(config);
    for index in 0..3 {
        system.in_flight.push(Torpedo {
            profile: profile.clone(),
            durability_remaining: profile.durability,
            strike_damage_bonus: 0.0,
            uuid: format!("incoming-{index}"),
            x: index as f32 * 5.0,
            y: 10.0,
            z: -15.0,
            heading: 0.0,
            pitch: 0.0,
            previous_position: None,
            collision_interval_secs: None,
            lifespan_remaining: profile.lifespan,
            target_uuid: None,
            last_guidance: None,
            source_uuid: Some("remote-launcher".into()),
            tube_id: "remote".into(),
        });
    }
    super::super::TorpedoSystemResource(system)
}

#[test]
fn finite_paid_existing_battery_intercepts_two_rounds_but_a_larger_salvo_saturates_it() {
    use super::super::BlasterSystemResource;
    use crate::weapons::blaster::BlasterSystem;
    use phoenix_model::identity::SystemId;
    use phoenix_sim_gameplay::resources::stock::Stock;
    let config =
        crate::entities::include_resolve::load_entity_config("assets/entities/pdc_cruiser.toml")
            .unwrap();
    let bank_config = config
        .weapons_console
        .as_ref()
        .unwrap()
        .blaster_banks
        .iter()
        .find(|bank| bank.id == "side")
        .unwrap()
        .to_runtime();
    let owner = SystemId("blaster-side".into());
    let mut power = crate::modifiers::power_system::PowerSystem::from_energy(
        config.energy.clone().unwrap(),
        config.ship_config.clone().unwrap(),
    )
    .unwrap();
    let row = config
        .infrastructure
        .as_ref()
        .unwrap()
        .capacities
        .iter()
        .find(|row| row.id == "side-kinetic-rounds")
        .unwrap();
    let mut rounds = Stock::new(row.amount, row.ceiling.unwrap_or(row.amount)).unwrap();
    assert_eq!(rounds.level, 4);
    let before_energy = power.network().consumers[&owner].energy;
    let mut app = app();
    // No Ship marker: emitted vectors remain real physical providers after
    // their launcher is removed. No carrier firing/configuration is needed.
    let provider = app.world_mut().spawn(incoming()).id();
    let shooter = app
        .world_mut()
        .spawn(BlasterSystemResource(vec![BlasterSystem::new(bank_config)]))
        .id();
    let mut minted = 0;
    for index in 0..3 {
        for round in &mut app
            .world_mut()
            .get_mut::<super::super::TorpedoSystemResource>(provider)
            .unwrap()
            .0
            .in_flight
        {
            round.tick(0.4, None);
        }
        {
            let mut banks = app
                .world_mut()
                .get_mut::<BlasterSystemResource>(shooter)
                .unwrap();
            let bank = &mut banks.0[0];
            if index > 0 {
                bank.tick_with_origins_3d(
                    5.0,
                    0.0,
                    &[],
                    0.0,
                    [0.0; 3],
                    [0.0; 3],
                    "pdc",
                    &mut |_| false,
                    &mut || unreachable!(),
                );
            }
            assert!(bank.request_fire());
            let events = bank.tick_with_origins_3d(
                0.1,
                0.0,
                &[Some([0.0, 10.0, 0.0]), Some([0.0, 10.0, 0.0])],
                0.0,
                [index as f32 * 5.0, 10.0, -15.0],
                [0.0; 3],
                "pdc",
                &mut |_| {
                    if rounds.available() < 1 {
                        return false;
                    }
                    if !power.try_work_discrete(&owner).unwrap() {
                        return false;
                    }
                    rounds.reserve("actual-shot", 1).unwrap();
                    assert_eq!(rounds.consume("actual-shot"), Some(1));
                    true
                },
                &mut || {
                    minted += 1;
                    format!("paid-bolt-{minted}")
                },
            );
            assert_eq!(events.len(), if index < 2 { 2 } else { 0 });
            bank.advance_flight_only(0.4);
        }
        app.update();
    }
    assert_eq!(minted, 4);
    assert_eq!(rounds.level, 0);
    assert_eq!(
        power.network().consumers[&owner].energy,
        before_energy - 1.0
    );
    let remaining = &app
        .world()
        .get::<super::super::TorpedoSystemResource>(provider)
        .unwrap()
        .0
        .in_flight;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].uuid, "incoming-2");
    assert_eq!(remaining[0].durability_remaining, 24.0);
    assert!(
        app.world().get::<BlasterSystemResource>(shooter).unwrap().0[0]
            .in_flight
            .is_empty()
    );
    app.update();
    assert_eq!(
        app.world()
            .get::<super::super::TorpedoSystemResource>(provider)
            .unwrap()
            .0
            .in_flight
            .len(),
        1
    );
}

#[test]
fn an_unidentified_physical_body_blocks_a_bolt_before_the_named_missile() {
    use super::super::BlasterSystemResource;
    use crate::weapons::blaster::{BlasterBankConfig, BlasterSystem};
    let mut app = app();
    let provider = app.world_mut().spawn(incoming()).id();
    // This body deliberately has neither a target ID nor a damage receiver.
    app.world_mut().spawn((
        Collider::cylinder(1.0, 2.0),
        Transform::from_xyz(0.0, 10.0, -5.0),
    ));
    let mut bank = BlasterSystem::new(BlasterBankConfig {
        volley_count: 1,
        projectile_speed: 100.0,
        range: 100.0,
        collision_radius: 0.1,
        damage: 100,
        ..Default::default()
    });
    bank.request_fire();
    bank.tick_with_origins_3d(
        0.1,
        0.0,
        &[Some([0.0, 10.0, 0.0])],
        0.0,
        [0.0, 10.0, -15.0],
        [0.0; 3],
        "shooter",
        &mut |_| true,
        &mut || "paid".into(),
    );
    bank.advance_flight_only(0.2);
    let shooter = app
        .world_mut()
        .spawn(BlasterSystemResource(vec![bank]))
        .id();
    app.update();
    assert!(
        app.world().get::<BlasterSystemResource>(shooter).unwrap().0[0]
            .in_flight
            .is_empty()
    );
    assert!(app
        .world()
        .get::<super::super::TorpedoSystemResource>(provider)
        .unwrap()
        .0
        .in_flight
        .iter()
        .all(|round| round.durability_remaining == 24.0));
}

#[test]
fn actual_flight_crossing_intercepts_before_endpoint_and_clears_both_interval_frontiers() {
    use super::super::BlasterSystemResource;
    use crate::weapons::blaster::{BlasterBankConfig, BlasterSystem};
    let mut app = app();
    let mut guided = incoming();
    guided.0.in_flight.truncate(1);
    let missile = &mut guided.0.in_flight[0];
    missile.x = -10.0;
    missile.z = -5.0;
    missile.heading = std::f32::consts::FRAC_PI_2;
    missile.profile.speed = 20.0;
    missile.tick(1.0, None);
    assert!(missile.x > 9.9);
    let provider = app.world_mut().spawn(guided).id();
    let mut bank = BlasterSystem::new(BlasterBankConfig {
        volley_count: 1,
        projectile_speed: 10.0,
        range: 100.0,
        collision_radius: 0.1,
        damage: 100,
        ..Default::default()
    });
    bank.request_fire();
    bank.tick_with_origins_3d(
        0.1,
        0.0,
        &[Some([0.0, 10.0, 0.0])],
        0.0,
        [0.0, 10.0, -10.0],
        [0.0; 3],
        "shooter",
        &mut |_| true,
        &mut || "paid".into(),
    );
    bank.advance_flight_only(1.0);
    let shooter = app
        .world_mut()
        .spawn(BlasterSystemResource(vec![bank]))
        .id();
    app.update();
    assert!(app
        .world()
        .get::<super::super::TorpedoSystemResource>(provider)
        .unwrap()
        .0
        .in_flight
        .is_empty());
    assert!(
        app.world().get::<BlasterSystemResource>(shooter).unwrap().0[0]
            .in_flight
            .is_empty()
    );
    app.update();
    assert!(app
        .world()
        .get::<super::super::TorpedoSystemResource>(provider)
        .unwrap()
        .0
        .in_flight
        .is_empty());
}

#[test]
fn an_authored_body_without_its_actual_collider_does_not_gain_a_point_adapter() {
    use super::super::BlasterSystemResource;
    use crate::weapons::blaster::{BlasterBankConfig, BlasterSystem};
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = app();
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&config)
        .unwrap()
        .unwrap();
    let before = damage.structural_totals();
    let body = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("missing-physical-backend".into()),
            crate::entities::spawner::ColliderSection(config.collider.unwrap()),
            Transform::from_xyz(0.0, 10.0, -5.0),
            damage,
        ))
        .id();
    let mut bank = BlasterSystem::new(BlasterBankConfig {
        volley_count: 1,
        projectile_speed: 10.0,
        range: 100.0,
        collision_radius: 0.1,
        ..Default::default()
    });
    bank.request_fire();
    bank.tick_with_origins_3d(
        0.1,
        0.0,
        &[Some([0.0, 10.0, 0.0])],
        0.0,
        [0.0, 10.0, -10.0],
        [0.0; 3],
        "shooter",
        &mut |_| true,
        &mut || "paid".into(),
    );
    bank.advance_flight_only(1.0);
    let shooter = app
        .world_mut()
        .spawn(BlasterSystemResource(vec![bank]))
        .id();
    app.update();
    assert_eq!(
        app.world()
            .get::<crate::ship::compartment_damage::ShipDamage>(body)
            .unwrap()
            .structural_totals(),
        before
    );
    assert_eq!(
        app.world().get::<BlasterSystemResource>(shooter).unwrap().0[0]
            .in_flight
            .len(),
        1
    );
}

#[test]
fn newly_emitted_bolt_cannot_retroactively_hit_a_missile_crossing_its_muzzle() {
    use super::super::BlasterSystemResource;
    use crate::weapons::blaster::{BlasterBankConfig, BlasterSystem};
    let mut app = app();
    let mut guided = incoming();
    guided.0.in_flight.truncate(1);
    let missile = &mut guided.0.in_flight[0];
    missile.x = -10.0;
    missile.z = 0.0;
    missile.heading = std::f32::consts::FRAC_PI_2;
    missile.profile.speed = 20.0;
    missile.tick(1.0, None);
    let provider = app.world_mut().spawn(guided).id();
    let mut bank = BlasterSystem::new(BlasterBankConfig {
        volley_count: 1,
        damage: 100,
        ..Default::default()
    });
    bank.request_fire();
    bank.tick_with_origins_3d(
        0.1,
        0.0,
        &[Some([0.0, 10.0, 0.0])],
        0.0,
        [0.0, 10.0, -10.0],
        [0.0; 3],
        "shooter",
        &mut |_| true,
        &mut || "new-at-end".into(),
    );
    let shooter = app
        .world_mut()
        .spawn(BlasterSystemResource(vec![bank]))
        .id();
    app.update();
    assert_eq!(
        app.world()
            .get::<super::super::TorpedoSystemResource>(provider)
            .unwrap()
            .0
            .in_flight
            .len(),
        1
    );
    assert_eq!(
        app.world().get::<BlasterSystemResource>(shooter).unwrap().0[0]
            .in_flight
            .len(),
        1
    );
    assert_eq!(
        app.world().get::<BlasterSystemResource>(shooter).unwrap().0[0].in_flight[0]
            .collision_interval_secs,
        None
    );
}

#[test]
fn an_asymmetric_body_uses_current_canonical_yaw_instead_of_mirrored_or_stale_pose() {
    let body = Collider::cuboid(0.2, 1.0, 4.0);
    let pose = crate::ship::state::ShipPhysics {
        yaw: std::f32::consts::FRAC_PI_4,
        ..Default::default()
    };
    let stale = Transform::from_xyz(900.0, 0.0, 900.0);
    let (position, rotation) = physical_pose(&stale, Some(&pose));
    assert!(collider_fraction(
        [2.0, -2.0, -2.0],
        [2.0, 2.0, -2.0],
        0.01,
        position,
        rotation,
        &body
    )
    .is_some());
    assert!(collider_fraction(
        [2.0, -2.0, -2.0],
        [2.0, 2.0, -2.0],
        0.01,
        position,
        Quat::from_rotation_y(pose.yaw),
        &body
    )
    .is_none());
    let (position, rotation) = physical_pose(&stale, None);
    assert_eq!(position, stale.translation);
    assert_eq!(rotation, stale.rotation);
}

#[test]
fn actual_paid_crossing_prevents_same_step_endpoint_payload_but_unopposed_round_damages_hull() {
    use super::super::{BlasterSystemResource, TorpedoSystemResource};
    use crate::ship::compartment_damage::ShipDamage;
    use crate::weapons::blaster::BlasterSystem;
    use bevy::ecs::system::RunSystemOnce;
    use phoenix_model::identity::SystemId;
    for intercept in [false, true] {
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/pdc_cruiser.toml",
        )
        .unwrap();
        let mut app = app();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        app.insert_resource(time)
            .insert_resource(TorpedoSystemResource(
                crate::weapons::torpedo::TorpedoSystem::new(
                    crate::weapons::torpedo::TorpedoConfig::default(),
                ),
            ))
            .init_resource::<super::super::shared::TorpedoTargetSnapshot>();
        let target = app
            .world_mut()
            .spawn((
                crate::entities::spawner::EntityUuid("endpoint-hull".into()),
                Transform::from_xyz(10.0, 10.0, -25.0),
                ShipDamage::from_config(&config).unwrap().unwrap(),
            ))
            .id();
        let before = app
            .world()
            .get::<ShipDamage>(target)
            .unwrap()
            .structural_totals()
            .0;
        let mut incoming = incoming();
        incoming.0.in_flight.truncate(1);
        let missile = &mut incoming.0.in_flight[0];
        missile.x = -10.0;
        missile.z = -25.0;
        missile.heading = std::f32::consts::FRAC_PI_2;
        missile.profile.speed = 20.0;
        let provider = app
            .world_mut()
            .spawn((crate::server_app::Ship, incoming))
            .id();
        let owner = SystemId("blaster-side".into());
        let mut power = crate::ship::power::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::from_energy(
                config.energy.clone().unwrap(),
                config.ship_config.clone().unwrap(),
            )
            .unwrap(),
        );
        let mut inventory = crate::infrastructure::InfrastructureCondition(
            crate::infrastructure::condition::InfrastructureState::from_config(
                config.infrastructure.as_ref().unwrap(),
            ),
        );
        let logistics = crate::resources::logistics_server::CompiledShipLogistics(
            crate::resources::logistics_profile::compile_entity_logistics(&config)
                .unwrap()
                .unwrap(),
        );
        let bank_config = config
            .weapons_console
            .as_ref()
            .unwrap()
            .blaster_banks
            .iter()
            .find(|bank| bank.id == "side")
            .unwrap()
            .to_runtime();
        let cost = power
            .0
            .config()
            .consumers
            .iter()
            .find(|consumer| consumer.system == owner)
            .unwrap()
            .discrete_cost;
        let energy = power.0.network().consumers[&owner].energy;
        let rounds = inventory
            .0
            .capacity_stock("side-kinetic-rounds")
            .unwrap()
            .level;
        if intercept {
            let feed = bank_config.rounds.clone().unwrap();
            let mut bank = BlasterSystem::new(bank_config);
            assert!(bank.request_fire());
            let mut minted = 0;
            let launched = bank.tick_with_origins_3d(
                0.1,
                0.0,
                &[Some([0.0, 10.0, 0.0]); 2],
                0.0,
                [0.0, 10.0, -25.0],
                [0.0; 3],
                "pdc",
                &mut |barrel| {
                    super::super::round_work::pay_barrel(
                        &owner,
                        Some(&feed),
                        barrel,
                        Some(&mut power),
                        Some(&mut inventory),
                        Some(&logistics),
                        &Default::default(),
                    )
                    .is_some()
                },
                &mut || {
                    minted += 1;
                    format!("actual-crossing-{minted}")
                },
            );
            assert_eq!(launched.len(), 2);
            bank.advance_flight_only(1.0);
            app.world_mut().spawn(BlasterSystemResource(vec![bank]));
        }
        // Real production phases: actual guided advance, PDC Damage, then
        // terminal payload settlement. The endpoint alone is a genuine hit.
        app.world_mut()
            .run_system_once(super::super::torpedo::build_torpedo_target_snapshot)
            .unwrap();
        app.world_mut()
            .run_system_once(super::super::advance_torpedo_flights)
            .unwrap();
        let roster = &app
            .world()
            .resource::<super::super::shared::TorpedoTargetSnapshot>()
            .targets;
        assert!(app
            .world()
            .get::<TorpedoSystemResource>(provider)
            .unwrap()
            .0
            .find_detonation_hits(roster)
            .iter()
            .any(|(_, target)| target == "endpoint-hull"));
        app.world_mut()
            .run_system_once(super::super::blaster::handle_blaster_hits)
            .unwrap();
        app.world_mut()
            .run_system_once(super::super::torpedo::tick_torpedo_lifecycle)
            .unwrap();
        let after = app
            .world()
            .get::<ShipDamage>(target)
            .unwrap()
            .structural_totals()
            .0;
        if intercept {
            assert_eq!(
                after, before,
                "earlier actual crossing removes payload before endpoint settlement"
            );
        } else {
            assert!(
                after < before,
                "same actual unopposed endpoint causes physical hull damage"
            );
        }
        assert!(app
            .world()
            .get::<TorpedoSystemResource>(provider)
            .unwrap()
            .0
            .in_flight
            .is_empty());
        assert_eq!(
            inventory
                .0
                .capacity_stock("side-kinetic-rounds")
                .unwrap()
                .level,
            rounds - if intercept { 2 } else { 0 }
        );
        assert_eq!(
            power.0.network().consumers[&owner].energy,
            energy - if intercept { 2.0 * cost } else { 0.0 }
        );
    }
}
