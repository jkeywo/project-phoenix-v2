use super::*;
use crate::ai::server::AiEntityDisabled;
use crate::entities::spawner::EntityUuid;
use crate::ship::components::ShipConfigComponent;
use crate::ship::disablement::{publish_ship_disablement, DisablementObservation};

#[test]
fn static_equipment_requires_loss_of_every_physical_capability() {
    let (mut damage, mut ship, mut power) = fixture();
    ship.systems.retain(|s| s.id.0 != "engine");
    destroy(&mut damage, "reactor");
    for reserve in power.0.network_mut().consumers.values_mut() {
        reserve.energy = 0.0;
    }
    assert!(
        !damage.disabled(&ship, Some(&power)),
        "a port never gains fake engines"
    );
    assert!(
        !damage.neutralized(&ship, Some(&power)),
        "working radio is still a capability"
    );
    ship.systems
        .iter_mut()
        .find(|s| s.id.0 == "radio")
        .unwrap()
        .kind = crate::ship::system_registry::SHIELD_ARC_KIND.into();
    assert!(
        !damage.neutralized(&ship, Some(&power)),
        "defensive emitters count too"
    );
    destroy(&mut damage, "radio");
    assert!(damage.neutralized(&ship, Some(&power)));
    assert!(!damage.catastrophic());
    assert!(
        !damage.neutralized(&ship, None),
        "missing power is not exhaustion evidence"
    );
}

#[test]
fn actual_owner_publishes_once_per_transition_and_saved_frontier_does_not_repeat() {
    let (mut damage, ship, mut power) = fixture();
    destroy(&mut damage, "engine");
    destroy(&mut damage, "reactor");
    for reserve in power.0.network_mut().consumers.values_mut() {
        reserve.energy = 0.0;
    }
    let mut app = App::new();
    app.add_message::<AiEntityDisabled>();
    app.add_systems(Update, publish_ship_disablement);
    let entity = app
        .world_mut()
        .spawn((
            EntityUuid("actor".into()),
            ShipConfigComponent(ship),
            damage,
            power,
        ))
        .id();
    let drain = |app: &mut App| -> Vec<bool> {
        app.world_mut()
            .resource_mut::<Messages<AiEntityDisabled>>()
            .drain()
            .map(|event| event.disabled)
            .collect()
    };
    app.update();
    assert_eq!(drain(&mut app), vec![true]);
    app.update();
    assert!(drain(&mut app).is_empty());
    let saved =
        serde_json::to_string(app.world().get::<DisablementObservation>(entity).unwrap()).unwrap();
    let frontier: DisablementObservation = serde_json::from_str(&saved).unwrap();
    app.world_mut().entity_mut(entity).insert(frontier);
    app.update();
    assert!(
        drain(&mut app).is_empty(),
        "restored frontier does not replay defeat"
    );
    app.world_mut()
        .get_mut::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .energy = 1.0;
    app.update();
    assert_eq!(drain(&mut app), vec![false]);
    app.world_mut()
        .get_mut::<crate::ship::power::ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .energy = 0.0;
    app.update();
    assert_eq!(drain(&mut app), vec![true]);
    assert!(
        app.world().get_entity(entity).is_ok(),
        "defeat never deletes an actor"
    );
    assert!(!app
        .world()
        .get::<ShipDamage>(entity)
        .unwrap()
        .catastrophic());
}

#[test]
fn unaffordable_discrete_remainder_cannot_indefinitely_hide_power_loss() {
    let (mut damage, ship, power) = fixture();
    let mut hardware = power.0.config().clone();
    let radio = hardware
        .consumers
        .iter_mut()
        .find(|profile| profile.system.0 == "radio")
        .unwrap();
    radio.continuous_cost_per_sec = 0.0;
    radio.discrete_cost = 2.0;
    let mut power = crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship.clone()).unwrap(),
    );
    destroy(&mut damage, "engine");
    destroy(&mut damage, "reactor");
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .energy = 0.5;
    assert!(
        damage.disabled(&ship, Some(&power)),
        "fractional remainder cannot perform the actual discrete work"
    );
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .energy = 2.0;
    assert!(
        !damage.disabled(&ship, Some(&power)),
        "exactly affordable work disproves total power loss"
    );
}

#[test]
fn charged_domain_reserve_with_zero_generic_costs_still_disproves_power_loss() {
    let (mut damage, ship, power) = fixture();
    let mut hardware = power.0.config().clone();
    let profile = hardware
        .consumers
        .iter_mut()
        .find(|profile| profile.system.0 == "radio")
        .unwrap();
    profile.continuous_cost_per_sec = 0.0;
    profile.discrete_cost = 0.0;
    let mut power = crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship.clone()).unwrap(),
    );
    destroy(&mut damage, "engine");
    destroy(&mut damage, "reactor");
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .energy = 0.5;
    assert!(
        !damage.disabled(&ship, Some(&power)),
        "domain-specific costs do not make an actual charged reserve imaginary"
    );
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .thermal_stopped = true;
    assert!(
        damage.disabled(&ship, Some(&power)),
        "a thermally unavailable buffer is not usable"
    );
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .thermal_stopped = false;
    destroy(&mut damage, "radio");
    assert!(
        damage.disabled(&ship, Some(&power)),
        "Operation loss also makes the charged buffer unusable"
    );
}
