use super::*;
use crate::modifiers::power_system::PowerSystem;
use crate::ship::power::ShipPowerSystem;
use crate::weapons::shield::ShieldSystem;
use bevy::ecs::system::RunSystemOnce;

fn physical_actor() -> (App, Entity, Vec<SystemId>) {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let base = config
        .shields_console
        .as_ref()
        .unwrap()
        .base
        .as_ref()
        .unwrap()
        .to_runtime();
    let arcs: Vec<_> = config
        .shield_arcs
        .iter()
        .map(|arc| arc.to_runtime())
        .collect();
    let shields = ShieldSystem::from_arcs(&arcs, &base);
    let ids: Vec<_> = shields
        .facings
        .iter()
        .map(|facing| crate::ship::system_registry::shield_arc_system_id(&facing.id).unwrap())
        .collect();
    let mut history = ShieldsDamageHistory::default();
    history.ensure_len(shields.facings.len());
    for (index, facing) in shields.facings.iter().enumerate() {
        history.observe_hp(index, facing.hp);
    }
    let power =
        PowerSystem::from_energy(config.energy.unwrap(), config.ship_config.unwrap()).unwrap();
    let mut app = App::new();
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs(1));
    app.insert_resource(time);
    // Deliberately no Ship marker: the same physical owner serves static actors.
    let actor = app
        .world_mut()
        .spawn((ShipShields(shields, 0.5), ShipPowerSystem(power), history))
        .id();
    (app, actor, ids)
}

#[test]
fn finite_upkeep_is_paid_even_at_full_strength_and_can_partly_protect() {
    let (mut app, actor, ids) = physical_actor();
    let upkeep = app.world().get::<ShipShields>(actor).unwrap().0.facings[0]
        .energy_config
        .upkeep_per_sec;
    let before = app.world().get::<ShipShields>(actor).unwrap().0.facings[0].hp;
    app.world_mut()
        .get_mut::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&ids[0])
        .unwrap()
        .energy = upkeep * 0.5;
    app.world_mut().run_system_once(tick_shields).unwrap();
    let mut shields = app.world_mut().get_mut::<ShipShields>(actor).unwrap();
    assert_eq!(shields.0.facings[0].hp, before);
    assert_eq!(shields.0.facings[0].protection_fraction, 0.5);
    assert_eq!(shields.0.facings[0].apply_damage(10), 5);
    assert_eq!(shields.0.facings[0].hp, before - 5);
}

#[test]
fn recharge_thermal_trip_clears_previously_funded_protection_without_lowering_intent() {
    let (mut app, actor, ids) = physical_actor();
    let profile = app
        .world()
        .get::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .config()
        .consumers
        .iter()
        .find(|profile| profile.system == ids[0])
        .unwrap()
        .clone();
    {
        let mut power = app.world_mut().get_mut::<ShipPowerSystem>(actor).unwrap();
        let state = power.0.network_mut().consumers.get_mut(&ids[0]).unwrap();
        state.energy = profile.reserve.capacity;
        state.heat = profile.reserve.trip_heat - 0.01;
    }
    app.world_mut()
        .get_mut::<ShipShields>(actor)
        .unwrap()
        .0
        .facings[0]
        .hp -= 10;
    app.world_mut().run_system_once(tick_shields).unwrap();
    assert!(
        app.world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .network()
            .consumers[&ids[0]]
            .thermal_stopped
    );
    let shields = app.world().get::<ShipShields>(actor).unwrap();
    assert!(shields.0.facings[0].raised);
    assert_eq!(shields.0.facings[0].protection_fraction, 0.0);
}

#[test]
fn recovery_accepts_only_actual_headroom_without_operating_the_recipient() {
    let (mut app, actor, ids) = physical_actor();
    let (before, yield_per_hp) = {
        let mut shields = app.world_mut().get_mut::<ShipShields>(actor).unwrap();
        let before = shields.0.facings[0].stored_strength();
        let yield_per_hp = shields.0.facings[0].energy_config.recovery_energy_per_hp as f64;
        shields.0.facings[0].recovery_target = Some(ids[1].clone());
        shields.0.facings[0].raised = false;
        shields.0.facings[1].raised = false;
        (before, yield_per_hp)
    };
    let capacity = app
        .world()
        .get::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .config()
        .consumers
        .iter()
        .find(|profile| profile.system == ids[1])
        .unwrap()
        .reserve
        .capacity;
    app.world_mut()
        .get_mut::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&ids[1])
        .unwrap()
        .energy = capacity - 0.5;
    app.world_mut().run_system_once(tick_shields).unwrap();
    let shields = app.world().get::<ShipShields>(actor).unwrap();
    assert!(!shields.0.facings[1].raised);
    assert_eq!(
        (before - shields.0.facings[0].stored_strength()) * yield_per_hp,
        0.5
    );
    assert_eq!(
        app.world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .network()
            .consumers[&ids[1]]
            .energy,
        capacity
    );
    let history = app.world().get::<ShieldsDamageHistory>(actor).unwrap();
    assert_eq!(history.last_hp[0], shields.0.facings[0].hp);
    assert!(history.arcs.iter().all(Vec::is_empty));
}

#[test]
fn missing_energy_owner_never_grants_free_static_protection() {
    let (mut app, actor, _) = physical_actor();
    app.world_mut()
        .entity_mut(actor)
        .remove::<ShipPowerSystem>();
    app.world_mut().run_system_once(tick_shields).unwrap();
    let mut shields = app.world_mut().get_mut::<ShipShields>(actor).unwrap();
    assert!(shields
        .0
        .facings
        .iter()
        .all(|facing| facing.protection_fraction == 0.0));
    assert_eq!(shields.0.facings[0].apply_damage(10), 10);
}

#[test]
fn static_equipment_applies_the_same_admitted_arc_commands() {
    let (mut app, actor, ids) = physical_actor();
    app.world_mut()
        .entity_mut(actor)
        .insert(AdmittedCommands(vec![
            crate::core::messages::AdmittedCommand {
                target: ids[0].clone(),
                payload: SystemControlPayload::SetShieldRaised { raised: false },
                response_token: None,
                feedback_correlation: None,
            },
            crate::core::messages::AdmittedCommand {
                target: ids[0].clone(),
                payload: SystemControlPayload::SetShieldRecovery {
                    target: Some(ids[1].clone()),
                },
                response_token: None,
                feedback_correlation: None,
            },
        ]));
    app.world_mut()
        .run_system_once(handle_shields_messages)
        .unwrap();
    let shields = app.world().get::<ShipShields>(actor).unwrap();
    assert!(!shields.0.facings[0].raised);
    assert_eq!(shields.0.facings[0].recovery_target.as_ref(), Some(&ids[1]));
}

#[test]
fn operation_damage_reduces_paid_protection_and_recharge_without_erasing_strength() {
    let run = |condition: Option<phoenix_model::damage::ComponentCondition>| {
        let (mut app, actor, ids) = physical_actor();
        let config =
            crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml")
                .unwrap();
        let mut damage = crate::ship::physical_damage::ShipDamage::from_config(&config)
            .unwrap()
            .unwrap();
        if let Some(condition) = condition {
            let component = damage
                .state
                .components
                .get_mut(&ids[0])
                .unwrap()
                .get_mut("operation")
                .unwrap();
            component.condition = condition;
            component.last_source = Some("fixture-hit".into());
            damage.state.validate(&damage.definition).unwrap();
        }
        app.world_mut().entity_mut(actor).insert(damage);
        {
            let mut shields = app.world_mut().get_mut::<ShipShields>(actor).unwrap();
            shields.0.facings[0].hp -= 20;
        }
        let before_hp =
            app.world().get::<ShipShields>(actor).unwrap().0.facings[0].stored_strength();
        let before_energy = app
            .world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .network()
            .consumers[&ids[0]]
            .energy;
        app.world_mut().run_system_once(tick_shields).unwrap();
        let after = app.world().get::<ShipShields>(actor).unwrap();
        let energy = app
            .world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .network()
            .consumers[&ids[0]]
            .energy;
        (
            after.0.facings[0].stored_strength() - before_hp,
            before_energy - energy,
            after.0.facings[0].protection_fraction,
            after.0.facings[0].raised,
        )
    };
    let intact = run(None);
    let partial = run(Some(phoenix_model::damage::ComponentCondition::Damaged));
    let zero = run(Some(phoenix_model::damage::ComponentCondition::Destroyed));
    assert!(intact.0 > 0.0 && intact.1 > 0.0);
    assert!((partial.0 - intact.0 * 0.5).abs() < 0.0001);
    assert_eq!(partial.1, intact.1);
    assert_eq!(partial.2, intact.2 * 0.5);
    assert_eq!(zero, (0.0, 0.0, 0.0, true));
}

#[test]
fn recovery_respects_recipient_intake_damage_and_donor_conversion_loss() {
    let run = |broken_intake: bool, damaged_donor: bool| {
        let (mut app, actor, ids) = physical_actor();
        let config =
            crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml")
                .unwrap();
        let mut damage = crate::ship::physical_damage::ShipDamage::from_config(&config)
            .unwrap()
            .unwrap();
        for (id, component, condition) in [
            (
                ids[0].clone(),
                "operation",
                damaged_donor.then_some(phoenix_model::damage::ComponentCondition::Damaged),
            ),
            (
                ids[1].clone(),
                "intake",
                broken_intake.then_some(phoenix_model::damage::ComponentCondition::Destroyed),
            ),
        ] {
            if let Some(condition) = condition {
                let part = damage
                    .state
                    .components
                    .get_mut(&id)
                    .unwrap()
                    .get_mut(component)
                    .unwrap();
                part.condition = condition;
                part.last_source = Some("fixture-hit".into());
            }
        }
        damage.state.validate(&damage.definition).unwrap();
        app.world_mut().entity_mut(actor).insert(damage);
        let (before, yield_per_hp) = {
            let mut shields = app.world_mut().get_mut::<ShipShields>(actor).unwrap();
            for facing in &mut shields.0.facings {
                facing.raised = false;
            }
            shields.0.facings[0].recovery_target = Some(ids[1].clone());
            (
                shields.0.facings[0].stored_strength(),
                shields.0.facings[0].energy_config.recovery_energy_per_hp as f64,
            )
        };
        app.world_mut()
            .get_mut::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .network_mut()
            .consumers
            .get_mut(&ids[1])
            .unwrap()
            .energy = 0.0;
        app.world_mut().run_system_once(tick_shields).unwrap();
        let removed =
            before - app.world().get::<ShipShields>(actor).unwrap().0.facings[0].stored_strength();
        let credited = app
            .world()
            .get::<ShipPowerSystem>(actor)
            .unwrap()
            .0
            .network()
            .consumers[&ids[1]]
            .energy as f64;
        (removed, credited, yield_per_hp)
    };
    let intact = run(false, false);
    let partial = run(false, true);
    let blocked = run(true, false);
    assert!(intact.0 > 0.0 && intact.1 > 0.0);
    assert!((intact.0 - partial.0).abs() < 0.0001);
    assert!((partial.1 - intact.1 * 0.5).abs() < 0.0001);
    assert!((partial.1 - partial.0 * partial.2 * 0.5).abs() < 0.0001);
    assert_eq!(blocked.0, 0.0);
    assert_eq!(blocked.1, 0.0);
}
