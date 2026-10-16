use super::*;
use phoenix_model::damage::{ComponentCondition, DamageBinding};

#[test]
fn runtime_derives_independent_actual_link_effects_without_changing_authored_requests() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut damage = crate::ship::compartment_damage::ShipDamage::from_config(&config)
        .unwrap()
        .unwrap();
    let power = crate::modifiers::power_system::PowerSystem::from_energy(
        config.energy.unwrap(),
        config.ship_config.unwrap(),
    )
    .unwrap();
    let consumer = power.config().consumers[0].system.clone();
    let reactor = power
        .ship_config()
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::POWER_REACTOR_KIND)
        .unwrap()
        .id
        .clone();
    for (id, binding, condition) in [
        (
            &consumer,
            DamageBinding::Intake,
            ComponentCondition::Destroyed,
        ),
        (
            &consumer,
            DamageBinding::Cooling,
            ComponentCondition::Damaged,
        ),
        (
            &reactor,
            DamageBinding::Operation,
            ComponentCondition::Damaged,
        ),
    ] {
        let name = damage.definition.components[id]
            .iter()
            .find(|(_, part)| part.binding == binding)
            .unwrap()
            .0
            .clone();
        let state = damage
            .state
            .components
            .get_mut(id)
            .unwrap()
            .get_mut(&name)
            .unwrap();
        state.condition = condition;
        state.last_source = Some("fixture-hit".into());
    }
    damage.state.validate(&damage.definition).unwrap();
    let before = power.capture_continuation();
    let effects = hardware_effects(&power, &power.adapter_selection(), Some(&damage), None);
    assert_eq!(effects.consumers[&consumer].intake, 0.0);
    assert_eq!(effects.consumers[&consumer].cooling, 0.5);
    assert_eq!(effects.reactor_operation, 0.5);
    assert_eq!(effects.reactor.intake, 1.0);
    assert_eq!(power.capture_continuation(), before);
    assert_eq!(
        hardware_effects(&power, &power.adapter_selection(), None, None),
        crate::resources::energy_network::EnergyHardwareEffects {
            consumers: power
                .config()
                .consumers
                .iter()
                .map(|consumer| (consumer.system.clone(), Default::default()))
                .collect(),
            ..Default::default()
        }
    );
}
