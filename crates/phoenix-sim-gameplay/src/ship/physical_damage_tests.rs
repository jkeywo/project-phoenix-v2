use super::*;
use phoenix_model::damage::{ComponentCondition, DamageConfig};
use phoenix_model::interior::InteriorConfig;
use serde_json::json;

fn fixture() -> (
    ShipDamage,
    crate::ship::config::ShipConfig,
    crate::ship::power::ShipPowerSystem,
) {
    let template = crate::modifiers::power_system::PowerSystem::default();
    let group = template.config().groups[0].id.clone();
    let component = json!({"id":"operation","binding":"operation",
        "susceptibility":{"physical":1.0,"laser":1.0},"fault_dose_threshold":4.0,
        "degraded_factor":0.5,"full_remedy":"full","field_replaceable":true});
    let groups = serde_json::Map::from_iter([(
        group.0.clone(),
        json!({"label":"Fixture","default_level":2,"min_level":0,"max_level":4}),
    )]);
    let ship: crate::ship::config::ShipConfig = serde_json::from_value(json!({
        "power_groups": groups,
        "system":[
            {"id":"engine","kind":"helm_thrust","power_group":group.0,"damage_component":[component.clone()]},
            {"id":"reactor","kind":"power_reactor","damage_component":[component.clone()]},
            {"id":"radio","kind":"comms","power_group":group.0,"damage_component":[component]}
        ]
    })).unwrap();
    let interior: InteriorConfig = serde_json::from_value(json!({
        "deck":[{"id":"main","label":"Main","damage_weight":1.0}],
        "compartment":[{"id":"room","deck":"main","label":"Room","x":0.0,"z":0.0,
            "width":4.0,"depth":4.0,"material":"steel","max_hp":100.0,
            "recovery_fraction":0.25,"structural_remedy":"structure"}],
        "placements":{"engine":"room","reactor":"room","radio":"room"}
    }))
    .unwrap();
    let recipes: DamageConfig = serde_json::from_value(json!({
        "materials":{"steel":{"resistance":{"physical":1.0,"laser":1.0},"residual_fraction":0.5}},
        "remedies":{
            "structure":{"work_secs":1.0,"manual_work_fraction":0.25,"structural_hp":10.0,"outcome":{"kind":"structural"}},
            "full":{"work_secs":1.0,"manual_work_fraction":0.25,"outcome":{"kind":"full"}}
        }
    }))
    .unwrap();
    let definition = DamageDefinition::compile(&interior, &recipes, &ship).unwrap();
    let damage = ShipDamage {
        emp_clock: None,
        state: DamageState::new(&definition),
        definition,
    };
    let mut hardware = template.config().clone();
    hardware.groups.truncate(1);
    let mut profile = hardware.consumers[0].clone();
    profile.system = SystemId("engine".into());
    let mut radio = profile.clone();
    radio.system = SystemId("radio".into());
    hardware.consumers = vec![profile, radio];
    let power = crate::ship::power::ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship.clone()).unwrap(),
    );
    (damage, ship, power)
}

fn destroy(damage: &mut ShipDamage, system: &str) {
    let component = damage
        .state
        .components
        .get_mut(&SystemId(system.into()))
        .unwrap()
        .get_mut("operation")
        .unwrap();
    component.condition = ComponentCondition::Destroyed;
    component.last_source = Some("fixture-hit".into());
}

#[test]
fn charged_usable_reserves_disprove_power_loss_until_the_last_buffer_is_exhausted() {
    let (mut damage, ship, mut power) = fixture();
    destroy(&mut damage, "engine");
    destroy(&mut damage, "reactor");
    damage.state.validate(&damage.definition).unwrap();
    assert!(
        !damage.disabled(&ship, None),
        "absence of the owner is not confirmed exhaustion"
    );
    assert!(
        !damage.disabled(&ship, Some(&power)),
        "the still-operable radio has a charged local buffer"
    );
    power
        .0
        .network_mut()
        .consumers
        .get_mut(&SystemId("radio".into()))
        .unwrap()
        .energy = 0.0;
    assert!(damage.disabled(&ship, Some(&power)));
    assert!(
        !damage.catastrophic(),
        "recoverable disablement is not destruction"
    );
    damage
        .state
        .remedy_component(
            &damage.definition,
            &phoenix_model::damage::ComponentRef {
                system: SystemId("engine".into()),
                component: "operation".into(),
            },
            "full",
        )
        .unwrap();
    assert!(
        !damage.disabled(&ship, Some(&power)),
        "ordinary restoration changes the assessment"
    );
}

#[test]
fn idle_settings_do_not_prove_disablement_and_engineless_equipment_is_not_disabled() {
    let (damage, mut ship, mut power) = fixture();
    power.0.network_mut().reactor.energy = 0.0;
    for state in power.0.network_mut().consumers.values_mut() {
        state.energy = 0.0;
    }
    power.0.network_mut().output_target_per_sec = 0.0;
    assert!(!damage.disabled(&ship, Some(&power)));
    ship.systems
        .retain(|system| system.kind != crate::ship::system_registry::HELM_THRUST_KIND);
    assert!(!damage.disabled(&ship, Some(&power)));
}

#[test]
fn structure_zero_is_recoverable_and_never_uses_an_old_hull_as_a_death_oracle() {
    let (mut damage, _, _) = fixture();
    let state = damage.state.compartments.get_mut("room").unwrap();
    state.hp = 0.0;
    state.disabled = true;
    assert_eq!(damage.structural_fraction(), 0.0);
    assert!(!is_catastrophic(Some(&damage), None));
    assert!(operation_disabled(
        Some(&damage),
        None,
        &SystemId("radio".into())
    ));
    damage
        .state
        .restore_structure(&damage.definition, "room", 25.0)
        .unwrap();
    assert!(!operation_disabled(
        Some(&damage),
        None,
        &SystemId("radio".into())
    ));
    damage.state.catastrophic = true;
    assert!(is_catastrophic(Some(&damage), None));
}

#[test]
fn incoming_world_travel_uses_inverse_simulation_yaw() {
    for yaw in [
        0.0,
        core::f32::consts::FRAC_PI_2,
        -core::f32::consts::FRAC_PI_2,
        core::f32::consts::PI,
    ] {
        let (sin, cos) = (crate::simmath::sin(yaw), crate::simmath::cos(yaw));
        let local = super::local_hit_direction(yaw, [sin, -cos]);
        assert!(
            local[0].abs() < 0.000001 && (local[1] + 1.0).abs() < 0.000001,
            "world forward travel must remain local forward at yaw {yaw}: {local:?}"
        );
        let local = super::local_hit_direction(yaw, [cos, sin]);
        assert!(
            (local[0] - 1.0).abs() < 0.000001 && local[1].abs() < 0.000001,
            "world starboard travel must remain local starboard at yaw {yaw}: {local:?}"
        );
    }
}

#[path = "disablement_tests.rs"]
mod disablement_tests;

#[test]
fn structural_loss_is_a_reportable_symptom_without_losing_machinery_efficiency() {
    let (mut damage, _, _) = fixture();
    assert_eq!(
        damage.operating_tier(&SystemId("radio".into())),
        crate::ship::damage::DamageTier::Operational
    );
    damage.state.compartments.get_mut("room").unwrap().hp = 99.0;
    assert_eq!(
        damage.factor(
            &SystemId("radio".into()),
            phoenix_model::damage::DamageBinding::Operation
        ),
        1.0
    );
    assert_eq!(
        damage.operating_tier(&SystemId("radio".into())),
        crate::ship::damage::DamageTier::Damaged
    );
    damage
        .state
        .restore_structure(&damage.definition, "room", 1.0)
        .unwrap();
    assert_eq!(
        damage.operating_tier(&SystemId("radio".into())),
        crate::ship::damage::DamageTier::Operational
    );
}
