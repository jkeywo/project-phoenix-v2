use super::*;
use crate::ship::config::{PowerGroupConfig, SystemInstanceConfig};

fn reserve() -> EnergyConfig {
    EnergyConfig {
        capacity: 100.0,
        max_intake_per_sec: 20.0,
        passive_loss_at_full_per_sec: 1.0,
        charge_heat_per_unit: 0.1,
        discharge_heat_per_unit: 0.1,
        passive_cooling_per_sec: 0.2,
        coolant_heat_per_unit: 10.0,
        trip_heat: 50.0,
        recover_heat: 30.0,
    }
}
fn fixture() -> (
    EnergyConfigSection,
    ShipConfig,
    BTreeMap<String, ResourceEndpoint>,
    BTreeSet<String>,
) {
    let group = PowerGroupId("propulsion".into());
    let ship = ShipConfig {
        consoles: vec![],
        stations: vec![],
        systems: vec![SystemInstanceConfig {
            components: vec![],
            channels: vec![],
            id: SystemId("impulse".into()),
            kind: "impulse".into(),
            console: None,
            ai_only: false,
            power_group: Some(group.clone()),
            marker: None,
            config: None,
        }],
        power_groups: std::collections::HashMap::from([(
            group.clone(),
            PowerGroupConfig {
                label: "propulsion".into(),
                default_level: 2,
                min_level: 0,
                max_level: 4,
            },
        )]),
        coordination_lag_secs: 2.0,
    };
    let energy = EnergyConfigSection {
        reactor: EnergyReactorConfig {
            sustainable_output_per_sec: 20.0,
            max_output_per_sec: 40.0,
            ramp_per_sec: 10.0,
            initial_output_per_sec: 20.0,
            initial_energy: 80.0,
            initial_heat: 0.0,
            max_cooling_per_sec: 5.0,
            default_cooling_per_sec: 1.0,
            reserve: reserve(),
            coolant: default_coolant_link(),
        },
        consumers: vec![EnergyConsumerConfig {
            system: SystemId("impulse".into()),
            initial_energy: 50.0,
            initial_heat: 0.0,
            continuous_cost_per_sec: 10.0,
            discrete_cost: 0.0,
            reserve: reserve(),
            coolant: default_coolant_link(),
        }],
        groups: vec![EnergyGroupConfig {
            id: group,
            parent: None,
            max_flow_per_sec: 20.0,
            default_flow_per_sec: 10.0,
            max_cooling_per_sec: 5.0,
            default_cooling_per_sec: 1.0,
            priority: 0,
            restore_delay_secs: 0.5,
        }],
    };
    (
        energy,
        ship,
        BTreeMap::from([(
            "coolant".into(),
            ResourceEndpoint::Capacity {
                id: "coolant-tank".into(),
            },
        )]),
        BTreeSet::from(["coolant-tank".into()]),
    )
}
#[test]
fn authored_energy_round_trips_and_default_link_binds_actual_stock() {
    let (energy, ship, services, capacities) = fixture();
    assert!(energy.validate(&ship, &services, &capacities).is_ok());
    let source = toml::to_string(&energy).unwrap();
    let decoded: EnergyConfigSection = toml::from_str(&source).unwrap();
    assert_eq!(decoded, energy);
    assert!(
        energy
            .validate(&ship, &BTreeMap::new(), &capacities)
            .unwrap_err()
            .contains("not declared")
    );
    assert!(
        energy
            .validate(&ship, &services, &BTreeSet::new())
            .unwrap_err()
            .contains("capacity")
    );
}
#[test]
fn rejects_missing_duplicate_unknown_and_alias_consumers() {
    let (mut energy, mut ship, services, capacities) = fixture();
    energy.consumers.clear();
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("requires an explicit")
    );
    let (good, _, _, _) = fixture();
    energy = good;
    energy.consumers.push(energy.consumers[0].clone());
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("unique")
    );
    energy.consumers.pop();
    energy.consumers[0].system = SystemId("absent".into());
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("not declared")
    );
    energy.consumers[0].system = SystemId("impulse".into());
    ship.systems[0].kind = crate::ship::system_registry::POWER_BATTERY_KIND.into();
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("second reserve")
    );
    energy.consumers.clear();
    assert!(energy.validate(&ship, &services, &capacities).is_ok());
}
#[test]
fn rejects_cycles_missing_groups_and_excess_group_defaults() {
    let (mut energy, ship, services, capacities) = fixture();
    energy.groups[0].parent = Some(energy.groups[0].id.clone());
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("cycle")
    );
    energy.groups[0].parent = Some(PowerGroupId("absent".into()));
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("not declared")
    );
    energy.groups[0].parent = None;
    energy.groups[0].default_flow_per_sec = 21.0;
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("exceeds")
    );
    energy.groups.clear();
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("no energy.group")
    );
}
#[test]
fn rejects_nonfinite_negative_excess_initial_and_invalid_thermal_hardware() {
    let (mut energy, ship, services, capacities) = fixture();
    energy.consumers[0].initial_energy = 101.0;
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("exceeds reserve")
    );
    energy.consumers[0].initial_energy = 50.0;
    energy.consumers[0].discrete_cost = f32::NAN;
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("finite")
    );
    energy.consumers[0].discrete_cost = -1.0;
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("non-negative")
    );
    energy.consumers[0].discrete_cost = 1.0;
    energy.reactor.reserve.recover_heat = energy.reactor.reserve.trip_heat;
    assert!(energy.validate(&ship, &services, &capacities).is_err());
}
#[test]
fn closed_schema_refuses_old_direct_output_and_unknown_fields() {
    let (energy, _, _, _) = fixture();
    let source = toml::to_string(&energy).unwrap();
    for field in ["power_multipliers = [1.0, 2.0]\n", "capacity = 99.0\n"] {
        assert!(toml::from_str::<EnergyConfigSection>(&format!("{field}{source}")).is_err());
    }
}
#[test]
fn rejects_unaffordable_actions_invalid_restoration_delay_and_excess_reactor_cooling() {
    let (mut energy, ship, services, capacities) = fixture();
    energy.consumers[0].discrete_cost = energy.consumers[0].reserve.capacity + 1.0;
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("can never be afforded")
    );
    energy.consumers[0].discrete_cost = 1.0;
    energy.groups[0].restore_delay_secs = f32::INFINITY;
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("finite")
    );
    energy.groups[0].restore_delay_secs = 0.5;
    energy.reactor.default_cooling_per_sec = energy.reactor.max_cooling_per_sec + 1.0;
    assert!(
        energy
            .validate(&ship, &services, &capacities)
            .unwrap_err()
            .contains("cooling")
    );
}
