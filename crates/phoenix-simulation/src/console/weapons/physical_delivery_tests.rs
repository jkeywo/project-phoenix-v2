use super::*;
use crate::lobby::WorldResource;
use crate::ship::compartment_damage::ShipDamage;
use crate::sim_rng::InstallSimRng;

fn authored() -> crate::entities::config::EntityConfig {
    crate::entities::include_resolve::load_entity_config("assets/entities/alliance_cruiser.toml")
        .unwrap()
}
fn app() -> App {
    let mut app = App::new();
    app.insert_resource(WorldResource(Default::default()))
        .init_resource::<SimOutbox>()
        .insert_sim_rng(crate::sim_rng::SimRng::new(
            971,
            crate::sim_rng::SeedSource::Cli,
        ))
        .add_message::<crate::ai::server::AiEntityDestroyed>()
        .add_message::<AsteroidDestroyedVfx>()
        .add_message::<ShipDestroyedVfx>();
    app
}

fn slot(app: &App, shooter: Entity, bank: &str) -> ActiveBeamSlot {
    app.world()
        .get::<ActiveBeam>(shooter)
        .unwrap()
        .live_banks()
        .find(|(id, _)| id.as_str() == bank)
        .unwrap()
        .1
        .clone()
}

#[test]
fn continuous_beam_retains_hit_sample_until_effective_target_changes() {
    let config = authored();
    let mut app = app();
    app.init_resource::<BeamContext>()
        .add_systems(Update, tick_beams_apply_damage);
    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("target".into()),
            Transform::default(),
            ShipPhysics::default(),
            ShipDamage::from_config(&config).unwrap().unwrap(),
        ))
        .id();
    let blocker = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("blocker".into()),
            Transform::default(),
            ShipPhysics::default(),
            ShipDamage::from_config(&config).unwrap().unwrap(),
        ))
        .id();
    let mut combat = crate::entities::config::PhaserCombatConfig::from_weapons_console(
        config.weapons_console.as_ref().unwrap(),
    );
    combat.banks[0].impact.penetration = 0.0;
    let bank = combat.banks[0].id.clone();
    let mut active = ActiveBeam::default();
    active.start(bank.clone(), "target", 8.0, 1.0);
    let shooter = app
        .world_mut()
        .spawn((PhaserCombatConfigResource(combat), active))
        .id();
    app.world_mut()
        .resource_mut::<BeamContext>()
        .0
        .push(ShooterState {
            shooter_entity: shooter,
            shooter_uuid: "source".into(),
            shooter_x: -100.0,
            shooter_z: 0.0,
            target_uuid: "target".into(),
            active_bank: bank.clone(),
            cooldown_secs: 1.0,
            damage_to_apply: 5,
            shield_pierce: 1.0,
            end_beam_early: false,
            is_local_shooter: false,
            shooter_phaser_freq: 0.5,
            effective_target_uuid: "target".into(),
            effective_target_x: 0.0,
            effective_target_z: 0.0,
            zero_damage: false,
        });
    app.world_mut()
        .entity_mut(target)
        .insert(crate::ship::shields::ShipShields(
            crate::weapons::shield::ShieldSystem::new(&crate::weapons::shield::ShieldConfig {
                max_hp: 100,
                num_facings: 4,
                ..Default::default()
            }),
            0.5,
        ));
    app.world_mut().resource_mut::<BeamContext>().0[0].shield_pierce = 0.0;
    let rng_before_shield = app.world().resource::<crate::sim_rng::SimRng>().state();
    app.update();
    let shield_only = slot(&app, shooter, &bank);
    assert_eq!(shield_only.impact_sample_target, None);
    assert_eq!(shield_only.impact_sample, None);
    assert_eq!(
        app.world().resource::<crate::sim_rng::SimRng>().state(),
        rng_before_shield,
        "shield-only contact must not choose a physical deck/profile"
    );
    app.world_mut()
        .entity_mut(target)
        .remove::<crate::ship::shields::ShipShields>();
    let before = app
        .world()
        .get::<ShipDamage>(target)
        .unwrap()
        .structural_totals()
        .0;
    app.update();
    let sample = slot(&app, shooter, &bank).impact_sample.clone();
    assert!(sample.is_some());
    let rng = app.world().resource::<crate::sim_rng::SimRng>().state();
    app.update();
    assert_eq!(
        app.world().resource::<crate::sim_rng::SimRng>().state(),
        rng,
        "a retained beam does not redraw its deck/profile every tick"
    );
    assert_eq!(slot(&app, shooter, &bank).impact_sample, sample);
    assert_eq!(
        before
            - app
                .world()
                .get::<ShipDamage>(target)
                .unwrap()
                .structural_totals()
                .0,
        10.0
    );
    assert!(app.world().get::<EntitySystemHull>(target).is_none());
    app.world_mut().resource_mut::<BeamContext>().0[0].effective_target_uuid = "blocker".into();
    app.update();
    assert_ne!(
        app.world().resource::<crate::sim_rng::SimRng>().state(),
        rng
    );
    assert_eq!(
        slot(&app, shooter, &bank).impact_sample_target.as_deref(),
        Some("blocker")
    );
    assert!(
        app.world()
            .get::<ShipDamage>(blocker)
            .unwrap()
            .structural_totals()
            .0
            < before
    );
}

#[test]
fn blaster_uses_actual_projectile_azimuth_for_shield_facing() {
    let config = authored();
    let mut app = app();
    app.add_systems(
        Update,
        crate::console::weapons::blaster::handle_blaster_hits,
    );
    let shields =
        crate::weapons::shield::ShieldSystem::new(&crate::weapons::shield::ShieldConfig {
            max_hp: 100,
            num_facings: 4,
            ..Default::default()
        });
    let port = -std::f32::consts::FRAC_PI_2;
    let before_port = shields.hp_facing_bearing(port);
    let before_fore = shields.hp_facing_bearing(0.0);
    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("target".into()),
            Transform::default(),
            ShipPhysics::default(),
            ShipDamage::from_config(&config).unwrap().unwrap(),
            crate::ship::shields::ShipShields(shields, 0.5),
        ))
        .id();
    let mut weapon =
        crate::weapons::blaster::BlasterSystem::new(crate::weapons::blaster::BlasterBankConfig {
            id: "bolt".into(),
            ..Default::default()
        });
    weapon
        .in_flight
        .push(crate::weapons::blaster::BlasterProjectile {
            impact: phoenix_model::damage::DamageProfile {
                damage_type: phoenix_model::damage::DamageType::Physical,
                penetration: 0.0,
            },
            id: "shot".into(),
            x: 0.0,
            z: 0.0,
            heading: std::f32::consts::FRAC_PI_2,
            speed: 1.0,
            lifespan_remaining: 1.0,
            collision_radius: 1.0,
            damage: 10,
            shield_pierce: 0.0,
            source_uuid: "source".into(),
        });
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::console::weapons::blaster::BlasterSystemResource(vec![weapon]),
    ));
    app.update();
    let after = &app
        .world()
        .get::<crate::ship::shields::ShipShields>(target)
        .unwrap()
        .0;
    assert!(after.hp_facing_bearing(port) < before_port);
    assert_eq!(after.hp_facing_bearing(0.0), before_fore);
    assert!(app.world().get::<EntitySystemHull>(target).is_none());
}

#[test]
fn damaged_beam_operation_reduces_output_without_discounting_paid_work() {
    let run = |condition: Option<phoenix_model::damage::ComponentCondition>| {
        let config = authored();
        let mut app = app();
        let mut clock = Time::<()>::default();
        clock.advance_by(std::time::Duration::from_secs(1));
        app.insert_resource(clock)
            .init_resource::<BeamContext>()
            .add_systems(Update, tick_beams_prepare);
        let combat = crate::entities::config::PhaserCombatConfig::from_weapons_console(
            config.weapons_console.as_ref().unwrap(),
        );
        let bank = combat.banks[0].id.clone();
        let system = crate::ship::system_registry::phaser_bank_system_id(&bank).unwrap();
        let power = crate::modifiers::power_system::PowerSystem::from_energy(
            config.energy.clone().unwrap(),
            config.ship_config.clone().unwrap(),
        )
        .unwrap();
        let before = power.network().consumers[&system].energy;
        let mut physical = ShipDamage::from_config(&config).unwrap().unwrap();
        if let Some(condition) = condition {
            let component = physical
                .state
                .components
                .get_mut(&system)
                .unwrap()
                .get_mut("operation")
                .unwrap();
            component.condition = condition;
            component.last_source = Some("fixture-hit".into());
            physical.state.validate(&physical.definition).unwrap();
        }
        let mut active = ActiveBeam::default();
        active.start(bank.clone(), "target", 8.0, 1.0);
        let shooter = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entities::spawner::EntityUuid("source".into()),
                Transform::from_xyz(-10.0, 0.0, 0.0),
                ShipPhysics {
                    x: -10.0,
                    ..Default::default()
                },
                active,
                PhaserCooldown::default(),
                PhaserCombatConfigResource(combat),
                physical,
                crate::ship::power::ShipPowerSystem(power),
            ))
            .id();
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid("target".into()),
            Transform::default(),
        ));
        app.update();
        let state = slot(&app, shooter, &bank);
        let whole = app
            .world()
            .resource::<BeamContext>()
            .0
            .first()
            .map_or(0.0, |state| state.damage_to_apply as f32);
        let after = app
            .world()
            .get::<crate::ship::power::ShipPowerSystem>(shooter)
            .unwrap()
            .0
            .network()
            .consumers[&system]
            .energy;
        (
            whole + state.damage_accumulator,
            before - after,
            state.remaining_secs,
        )
    };
    let normal = run(None);
    let damaged = run(Some(phoenix_model::damage::ComponentCondition::Damaged));
    let destroyed = run(Some(phoenix_model::damage::ComponentCondition::Destroyed));
    assert!(normal.0 > 0.0 && normal.1 > 0.0);
    assert_eq!(damaged.0, normal.0 * 0.5);
    assert_eq!(damaged.1, normal.1);
    assert_eq!(
        destroyed,
        (0.0, 0.0, 8.0),
        "unavailable ongoing machinery preserves its battery and burn cycle"
    );
}
